//! Interactive shell tabs in the terminal tool window: a terminal emulator (`vt100`) per
//! shell, drawn as rows of styled runs, and the keys typed into it turned into the bytes
//! a terminal sends. Output and keys never reach the logs.

use super::{ShellRow, ShellRun};
use slint::{Color, ModelRc, VecModel, platform::Key};
use std::num::NonZeroU16;

/// Lines of history kept above the screen for scrolling back.
const SCROLLBACK: usize = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellState {
    Opening,
    Open,
    /// Closed by the user or the remote side.
    Closed,
    /// Could not open, or the link broke.
    Failed,
}

pub struct ShellTab {
    pub id: u64,
    pub host: String,
    pub name: String,
    pub state: ShellState,
    /// Shown under the screen once the shell has ended; empty after an emulator failure.
    pub note: String,
    /// Its SSH session is up: from the worker's answer until its end is heard. Its end
    /// is audited only then.
    pub connected: bool,
    pub parser: vt100::Parser,
}

impl ShellTab {
    pub fn new(id: u64, host: String, name: String, (cols, rows): (u16, u16)) -> Self {
        Self {
            id,
            host,
            name,
            state: ShellState::Opening,
            note: String::new(),
            connected: false,
            parser: vt100::Parser::new(nonzero(rows), nonzero(cols), SCROLLBACK),
        }
    }

    pub fn resize(&mut self, (cols, rows): (u16, u16)) {
        self.parser.set_size(nonzero(rows), nonzero(cols));
    }

    /// Moves the view `lines` up (positive) or down into the history; the alternate
    /// screen of full-screen programs has none.
    pub fn scroll(&mut self, lines: i32) {
        let screen = self.parser.screen_mut();
        if screen.alternate_screen() {
            return;
        }
        let position = i64::try_from(screen.scrollback()).unwrap_or(0) + i64::from(lines);
        screen.set_scrollback(usize::try_from(position.max(0)).unwrap_or(0));
    }

    /// The screen's text, for copying.
    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }

    /// The emulator panicked on this shell's output: its state can no longer be
    /// trusted, so the screen starts blank and the shell is marked failed.
    pub fn break_down(&mut self) {
        let (rows, cols) = self.parser.screen().size();
        self.parser = vt100::Parser::new(rows, cols, SCROLLBACK);
        self.state = ShellState::Failed;
        self.note.clear();
    }
}

fn nonzero(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(2)).unwrap_or(NonZeroU16::MIN)
}

/// The colours of the 16 basic terminal colours, xterm's.
const BASIC: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 49, 49),
    (13, 188, 121),
    (229, 229, 16),
    (36, 114, 200),
    (188, 63, 188),
    (17, 168, 205),
    (229, 229, 229),
    (102, 102, 102),
    (241, 76, 76),
    (35, 209, 139),
    (245, 245, 67),
    (59, 142, 234),
    (214, 112, 214),
    (41, 184, 219),
    (255, 255, 255),
];

fn indexed(index: u8) -> Color {
    let (r, g, b) = match index {
        0..=15 => BASIC[usize::from(index)],
        16..=231 => {
            let n = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    };
    Color::from_rgb_u8(r, g, b)
}

/// A colour and how the view draws it: 0 this colour, 1 the theme's text colour,
/// 2 the theme's background (for the defaults, and for them swapped by inverse video).
fn paint(color: vt100::Color, default: i32) -> (Color, i32) {
    match color {
        vt100::Color::Default => (Color::default(), default),
        vt100::Color::Idx(index) => (indexed(index), 0),
        vt100::Color::Rgb(r, g, b) => (Color::from_rgb_u8(r, g, b), 0),
    }
}

#[derive(PartialEq)]
struct Style {
    fg: vt100::Color,
    bg: vt100::Color,
    bold: bool,
    underline: bool,
    inverse: bool,
}

impl Style {
    fn of(cell: &vt100::Cell) -> Self {
        Self {
            fg: cell.fgcolor(),
            bg: cell.bgcolor(),
            bold: cell.bold(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        }
    }

    fn run(&self, col: u16, cells: u16, text: String) -> ShellRun {
        // Inverse video swaps the colours, the defaults included.
        let ((fg, fg_mode), (bg, bg_mode)) = if self.inverse {
            (paint(self.bg, 2), paint(self.fg, 1))
        } else {
            (paint(self.fg, 1), paint(self.bg, 3))
        };
        ShellRun {
            col: i32::from(col),
            cells: i32::from(cells),
            text: text.into(),
            fg,
            fg_mode,
            bg,
            // 3: no background of its own.
            bg_mode,
            bold: self.bold,
            underline: self.underline,
        }
    }
}

/// The screen as rows of runs: cells of one style together, each wide (e.g. CJK)
/// character on its own so it sits exactly over its two cells whatever the font.
pub fn rows(screen: &vt100::Screen) -> Vec<ShellRow> {
    let (rows, cols) = screen.size();
    (0..rows.get())
        .map(|row| {
            let mut runs: Vec<ShellRun> = Vec::new();
            let mut start = 0;
            let mut text = String::new();
            let mut style: Option<Style> = None;
            let flush = |runs: &mut Vec<ShellRun>,
                         style: &Option<Style>,
                         text: &mut String,
                         start: u16,
                         end: u16| {
                if let Some(style) = style
                    && end > start
                {
                    // Plain blank cells draw nothing, so plain runs lose their
                    // trailing ones.
                    let plain =
                        !style.inverse && !style.underline && style.bg == vt100::Color::Default;
                    let mut cells = end - start;
                    if plain {
                        let kept = text.trim_end().len();
                        let dropped = u16::try_from(text[kept..].chars().count()).unwrap_or(0);
                        text.truncate(kept);
                        cells = cells.saturating_sub(dropped);
                    }
                    if cells > 0 && !text.is_empty() {
                        runs.push(style.run(start, cells, std::mem::take(text)));
                    }
                }
                text.clear();
            };
            let mut col = 0;
            while col < cols.get() {
                let Some(cell) = screen.cell(row, col) else {
                    break;
                };
                if cell.is_wide_continuation() {
                    col += 1;
                    continue;
                }
                let this = Style::of(cell);
                let contents = if cell.has_contents() {
                    cell.contents()
                } else {
                    " "
                };
                if cell.is_wide() {
                    flush(&mut runs, &style, &mut text, start, col);
                    runs.push(this.run(col, 2, contents.into()));
                    style = None;
                    col += 2;
                    start = col;
                    continue;
                }
                if style.as_ref() != Some(&this) {
                    flush(&mut runs, &style, &mut text, start, col);
                    style = Some(this);
                    start = col;
                }
                text.push_str(contents);
                col += 1;
            }
            flush(&mut runs, &style, &mut text, start, col);
            ShellRow {
                runs: ModelRc::new(VecModel::from(runs)),
            }
        })
        .collect()
}

/// The bytes a terminal sends for a key, or `None` when it is not for the shell.
/// `text` is Slint's key text: the typed character, or a special key's code.
pub fn key_bytes(
    text: &str,
    control: bool,
    alt: bool,
    shift: bool,
    cursor_app: bool,
) -> Option<Vec<u8>> {
    let mut chars = text.chars();
    let (first, rest) = (chars.next()?, chars.next());
    let special = |key: Key| first == char::from(key);
    let arrow = |letter: char| {
        let lead = if cursor_app { "\x1bO" } else { "\x1b[" };
        format!("{lead}{letter}")
    };
    let sequence: Option<String> = if rest.is_some() {
        None
    } else if special(Key::Return) {
        Some("\r".into())
    } else if special(Key::Backspace) {
        Some("\x7f".into())
    } else if special(Key::Tab) {
        Some(if shift { "\x1b[Z" } else { "\t" }.into())
    } else if special(Key::Backtab) {
        Some("\x1b[Z".into())
    } else if special(Key::Escape) {
        Some("\x1b".into())
    } else if special(Key::Delete) {
        Some("\x1b[3~".into())
    } else if special(Key::UpArrow) {
        Some(arrow('A'))
    } else if special(Key::DownArrow) {
        Some(arrow('B'))
    } else if special(Key::RightArrow) {
        Some(arrow('C'))
    } else if special(Key::LeftArrow) {
        Some(arrow('D'))
    } else if special(Key::Home) {
        Some(arrow('H'))
    } else if special(Key::End) {
        Some(arrow('F'))
    } else if special(Key::PageUp) {
        Some("\x1b[5~".into())
    } else if special(Key::PageDown) {
        Some("\x1b[6~".into())
    } else if special(Key::Insert) {
        Some("\x1b[2~".into())
    } else {
        [
            (Key::F1, "\x1bOP"),
            (Key::F2, "\x1bOQ"),
            (Key::F3, "\x1bOR"),
            (Key::F4, "\x1bOS"),
            (Key::F5, "\x1b[15~"),
            (Key::F6, "\x1b[17~"),
            (Key::F7, "\x1b[18~"),
            (Key::F8, "\x1b[19~"),
            (Key::F9, "\x1b[20~"),
            (Key::F10, "\x1b[21~"),
            (Key::F11, "\x1b[23~"),
            (Key::F12, "\x1b[24~"),
        ]
        .into_iter()
        .find(|(key, _)| special(*key))
        .map(|(_, sequence)| sequence.into())
    };
    if let Some(sequence) = sequence {
        let mut bytes = sequence.into_bytes();
        if alt {
            bytes.insert(0, 0x1b);
        }
        return Some(bytes);
    }
    // Other keys Slint names (modifiers alone, menu keys…) sit in a private-use block.
    if text.chars().any(|c| ('\u{f700}'..='\u{f8ff}').contains(&c)) {
        return None;
    }
    // AltGr arrives as Ctrl+Alt: a character it typed (e.g. @ or { on European layouts)
    // is sent as typed, not as a control code.
    let alt_gr = control && alt && !first.is_ascii_alphabetic() && !first.is_control();
    if alt_gr {
        return Some(text.as_bytes().to_vec());
    }
    if control && rest.is_none() {
        // Ctrl+letter and its neighbours: the C0 control codes.
        let code = match first.to_ascii_lowercase() {
            c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
            ' ' | '@' | '2' => Some(0),
            '[' | '3' => Some(0x1b),
            '\\' | '4' => Some(0x1c),
            ']' | '5' => Some(0x1d),
            '^' | '6' => Some(0x1e),
            '_' | '-' | '7' => Some(0x1f),
            '/' => Some(0x1f),
            _ => None,
        }?;
        let mut bytes = vec![code];
        if alt {
            bytes.insert(0, 0x1b);
        }
        return Some(bytes);
    }
    // Control characters Slint passes through as text (e.g. its Tab) were handled above.
    if first.is_control() {
        return None;
    }
    let mut bytes = text.as_bytes().to_vec();
    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

/// Pasted text as a terminal sends it: line ends as Enter, wrapped for programs that
/// asked for bracketed paste so they do not run it line by line.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    // Nothing inside may end the bracket early.
    let text = text.replace("\x1b[201~", "");
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    #[test]
    fn keys_become_terminal_bytes() {
        let key = |k: Key| String::from(char::from(k));
        assert_eq!(
            key_bytes("a", false, false, false, false),
            Some(b"a".to_vec())
        );
        assert_eq!(
            key_bytes("中", false, false, false, false),
            Some("中".as_bytes().to_vec())
        );
        assert_eq!(key_bytes("x", true, false, false, false), Some(vec![0x18]));
        assert_eq!(key_bytes("O", true, false, true, false), Some(vec![0x0f]));
        assert_eq!(
            key_bytes(&key(Key::Return), false, false, false, false),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            key_bytes(&key(Key::Backspace), false, false, false, false),
            Some(vec![0x7f])
        );
        assert_eq!(
            key_bytes(&key(Key::UpArrow), false, false, false, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_bytes(&key(Key::UpArrow), false, false, false, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes(&key(Key::F5), false, false, false, false),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(
            key_bytes("x", false, true, false, false),
            Some(b"\x1bx".to_vec())
        );
        // AltGr (Ctrl+Alt) on a European layout types the character itself.
        assert_eq!(
            key_bytes("@", true, true, false, false),
            Some(b"@".to_vec())
        );
        assert_eq!(
            key_bytes("{", true, true, false, false),
            Some(b"{".to_vec())
        );
        // A modifier pressed alone sends nothing.
        assert_eq!(key_bytes(&key(Key::Shift), false, false, true, false), None);
    }

    #[test]
    fn pasted_lines_end_with_enter_and_are_bracketed_on_request() {
        assert_eq!(paste_bytes("a\r\nb\n", false), b"a\rb\r");
        assert_eq!(paste_bytes("ls", true), b"\x1b[200~ls\x1b[201~");
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~");
    }

    #[test]
    fn the_screen_is_drawn_as_styled_runs_with_wide_characters_alone() {
        let mut tab = ShellTab::new(1, "h".into(), "h".into(), (20, 3));
        // Plain text, then reverse video, then a wide character.
        tab.parser.process(b"ab\x1b[7mCD\x1b[0m \xe4\xb8\xad!");
        let rows = rows(tab.parser.screen());
        assert_eq!(rows.len(), 3);
        let runs: Vec<ShellRun> = rows[0].runs.iter().collect();
        let texts: Vec<(&str, i32, i32)> = runs
            .iter()
            .map(|run| (run.text.as_str(), run.col, run.cells))
            .collect();
        // The plain blank between them draws nothing.
        assert_eq!(
            texts,
            [("ab", 0, 2), ("CD", 2, 2), ("中", 5, 2), ("!", 7, 1)]
        );
        // Inverse video draws the theme's colours swapped.
        assert_eq!((runs[1].fg_mode, runs[1].bg_mode), (2, 1));
        assert_eq!((runs[0].fg_mode, runs[0].bg_mode), (1, 3));
        assert_eq!(runs[2].cells, 2);
        // Empty rows draw nothing.
        assert_eq!(rows[1].runs.row_count(), 0);
    }

    #[test]
    fn scrolling_back_stays_within_the_history() {
        let mut tab = ShellTab::new(1, "h".into(), "h".into(), (10, 2));
        tab.parser.process(b"1\r\n2\r\n3\r\n4\r\n");
        tab.scroll(10);
        assert_eq!(tab.parser.screen().scrollback(), 3);
        tab.scroll(-10);
        assert_eq!(tab.parser.screen().scrollback(), 0);
    }
}
