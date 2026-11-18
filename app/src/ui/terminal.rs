//! The terminal tool window: each host's remote commands and output as an operation
//! runs, one read-only tab per host (the text is already redacted by the core), then
//! any interactive shells the user opened, see [`super::remote_shell`].

use super::{
    App, AppWindow, Clip, Ctx, ShellRow, TerminalLine, TerminalState, TerminalTab, ToolWindow,
    app::Tool,
    remote_shell::{self, ShellState, ShellTab},
};
use misf_core::{
    audit::{Action, Outcome},
    shell::Input,
};
use serde_json::json;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    time::{Duration, Instant},
};

/// A shell's size before the panel has reported one.
const SHELL_SIZE: (u16, u16) = (100, 24);

/// Lines kept per host; older ones are dropped in steps of [`TRIM`].
pub const MAX_LINES: usize = 10_000;
const TRIM: usize = 2_000;

/// Presses this close together, on the same cell, count as a double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabState {
    Waiting,
    Running,
    Succeeded,
    Failed,
    /// A batch stopped before this host started.
    Skipped,
}

impl TabState {
    fn index(self) -> i32 {
        match self {
            Self::Waiting => 0,
            Self::Running => 1,
            Self::Succeeded => 2,
            Self::Failed => 3,
            Self::Skipped => 4,
        }
    }
}

pub struct Line {
    pub text: String,
    pub command: bool,
}

/// A place on the shown text: a screen row and cell in a shell, a line in a run's tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub row: usize,
    pub col: usize,
}

/// What one, two or three clicks select: from a cell, by words, or by whole lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Cell,
    Word,
    Line,
}

/// Text selected with the mouse, as a terminal does: dragging from the unit first
/// pressed to the one under the pointer.
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    unit: Unit,
    /// The first and last cell of the unit first pressed, and of the one under the pointer.
    anchor: (Point, Point),
    head: (Point, Point),
    /// A single click that has not moved: selects nothing.
    click: bool,
    /// The tab and generation of its lines; once either changes it is gone.
    view: (usize, u64),
}

impl Selection {
    /// The first and last cell selected (the last included).
    pub fn span(&self) -> (Point, Point) {
        (
            self.anchor.0.min(self.head.0),
            self.anchor.1.max(self.head.1),
        )
    }
}

pub struct Tab {
    pub host: String,
    pub name: String,
    pub state: TabState,
    pub lines: Vec<Line>,
}

#[derive(Default)]
pub struct Terminal {
    pub tabs: Vec<Tab>,
    /// Interactive shells, shown after the run's tabs.
    pub shells: Vec<ShellTab>,
    /// Into the run's tabs, then the shells.
    pub current: usize,
    /// The shell area's size in cells, as the panel last reported it.
    pub size: Option<(u16, u16)>,
    /// A shell's screen changed since the view last drew it.
    screen_dirty: bool,
    /// The shell tab the view's screen was drawn from.
    screen_synced: Option<usize>,
    /// The shell whose `shell_open` request is out.
    pending_open: Option<u64>,
    /// The request whose hosts the tabs show; its `Done` settles them.
    pub action: String,
    /// Bumped whenever shown lines change other than at the end, so the view rebuilds.
    generation: u64,
    /// The tab and generation the view's lines were built from.
    synced: Option<(usize, u64)>,
    /// Text selected with the mouse in the current tab.
    selection: Option<Selection>,
    /// The last press and how many came in a row, for double and triple clicks.
    last_press: Option<(Instant, Point, u8)>,
}

impl Terminal {
    /// Starts a new run on `hosts` (ID and name), replacing the previous one.
    /// Shells stay, after the new run's tabs.
    pub fn start(&mut self, action: &str, hosts: Vec<(String, String)>) {
        self.tabs = (hosts.into_iter())
            .map(|(host, name)| Tab {
                host,
                name,
                state: TabState::Waiting,
                lines: Vec::new(),
            })
            .collect();
        self.action = action.into();
        self.current = 0;
        self.generation += 1;
        self.screen_dirty = true;
    }

    fn tab(&mut self, host: &str) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.host == host)
    }

    /// Adds a command, or lines of output, to its host's tab.
    pub fn write(&mut self, host: &str, text: &str, command: bool) {
        let Some(tab) = self.tab(host) else {
            return;
        };
        if tab.state == TabState::Waiting {
            tab.state = TabState::Running;
        }
        for (index, line) in text.lines().enumerate() {
            // A command spanning lines continues like a shell's `>` prompt.
            let text = match (command, index) {
                (false, _) => line.to_owned(),
                (true, 0) => format!("$ {line}"),
                (true, _) => format!("> {line}"),
            };
            tab.lines.push(Line { text, command });
        }
        if tab.lines.len() > MAX_LINES {
            tab.lines.drain(..TRIM);
            self.generation += 1;
        }
    }

    /// Marks a host finished, with a closing note.
    pub fn finish(&mut self, host: &str, succeeded: bool, note: String) {
        if let Some(tab) = self.tab(host) {
            tab.state = if succeeded {
                TabState::Succeeded
            } else {
                TabState::Failed
            };
            tab.lines.push(Line {
                text: note,
                command: false,
            });
        }
    }

    /// Settles the tabs when the request ends: hosts never started were skipped, and a
    /// host still running ended with `note`, e.g. after a single-host result.
    pub fn end(&mut self, succeeded: bool, note: &str) {
        for tab in &mut self.tabs {
            match tab.state {
                TabState::Waiting if self.action == "batch" => tab.state = TabState::Skipped,
                TabState::Waiting | TabState::Running => {
                    tab.state = if succeeded {
                        TabState::Succeeded
                    } else {
                        TabState::Failed
                    };
                    tab.lines.push(Line {
                        text: note.into(),
                        command: false,
                    });
                }
                _ => {}
            }
        }
        self.action.clear();
    }

    /// The shell the current tab shows, if it is one.
    pub fn shell(&self) -> Option<&ShellTab> {
        self.shells.get(self.current.checked_sub(self.tabs.len())?)
    }

    fn shell_mut(&mut self) -> Option<&mut ShellTab> {
        let index = self.current.checked_sub(self.tabs.len())?;
        self.shells.get_mut(index)
    }

    fn shell_by_id(&mut self, id: u64) -> Option<&mut ShellTab> {
        self.shells.iter_mut().find(|shell| shell.id == id)
    }

    /// The shells' size in cells.
    pub fn size(&self) -> (u16, u16) {
        self.size.unwrap_or(SHELL_SIZE)
    }

    /// Adds a shell tab and shows it.
    pub fn add_shell(&mut self, shell: ShellTab) {
        self.shells.push(shell);
        self.current = self.tabs.len() + self.shells.len() - 1;
        self.screen_dirty = true;
    }

    /// Output for a shell's screen; true when the emulator failed on it, which leaves
    /// the shell failed with a blank screen.
    pub fn shell_output(&mut self, id: u64, data: &[u8]) -> bool {
        let Some(shell) = self.shell_by_id(id) else {
            return false;
        };
        if shell.state == ShellState::Failed {
            return false;
        }
        let parser = &mut shell.parser;
        let failed = catch_unwind(AssertUnwindSafe(|| parser.process(data))).is_err();
        if failed {
            shell.break_down();
        }
        // New output may scroll the text from under a selection; full-screen programs
        // redraw in place, so theirs stays.
        let alternate = !failed && shell.parser.screen().alternate_screen();
        if !alternate && self.shell().is_some_and(|shell| shell.id == id) {
            self.selection = None;
        }
        self.screen_dirty = true;
        failed
    }

    /// Forgets every run and shell, e.g. when the vault locks.
    pub fn clear(&mut self) {
        self.tabs.clear();
        self.shells.clear();
        self.action.clear();
        self.current = 0;
        self.pending_open = None;
        self.generation += 1;
        self.screen_dirty = true;
    }

    /// Marks every shell not yet ended as failed with `note`, e.g. when the worker that
    /// ran them is gone.
    pub fn fail_shells(&mut self, note: &str) {
        for shell in &mut self.shells {
            if matches!(shell.state, ShellState::Opening | ShellState::Open) {
                shell.state = ShellState::Failed;
                shell.note = note.into();
            }
            shell.connected = false;
        }
        self.pending_open = None;
        self.screen_dirty = true;
    }

    /// A shell ended; returns its host when it was open, for the audit log.
    pub fn shell_ended(&mut self, id: u64, failed: bool, note: String) -> Option<String> {
        let shell = self.shell_by_id(id)?;
        let was_open = std::mem::replace(&mut shell.connected, false);
        // Ended after its emulator failed: that stays the reason shown.
        let emulator_failed = shell.state == ShellState::Failed && shell.note.is_empty();
        if !emulator_failed {
            shell.state = if failed {
                ShellState::Failed
            } else {
                ShellState::Closed
            };
            shell.note = note;
        }
        let host = shell.host.clone();
        self.screen_dirty = true;
        was_open.then_some(host)
    }

    /// Drops an ended shell's tab.
    pub fn remove_shell(&mut self, index: usize) {
        if index < self.shells.len() {
            self.shells.remove(index);
            let last = (self.tabs.len() + self.shells.len()).saturating_sub(1);
            self.current = self.current.min(last);
            self.screen_dirty = true;
        }
    }

    /// The current tab's selection, while its lines are the ones it was made on.
    pub fn selection(&self) -> Option<Selection> {
        let view = (self.current, self.generation);
        self.selection.filter(|s| s.view == view && !s.click)
    }

    /// The rows the current tab shows and a shell's width in cells.
    fn extent(&self) -> Option<(usize, usize)> {
        if let Some(shell) = self.shell() {
            let (rows, cols) = shell.parser.screen().size();
            return Some((usize::from(rows.get()), usize::from(cols.get())));
        }
        let lines = self.tabs.get(self.current)?.lines.len();
        (lines > 0).then_some((lines, 1))
    }

    /// The first and last cell of the unit at `point`. A run's tab selects whole lines.
    fn unit_at(&self, unit: Unit, point: Point) -> (Point, Point) {
        let Some(shell) = self.shell() else {
            return (Point { col: 0, ..point }, Point { col: 0, ..point });
        };
        let screen = shell.parser.screen();
        let cols = usize::from(screen.size().1.get());
        let word_at = |col: usize| {
            u16::try_from(point.row)
                .ok()
                .zip(u16::try_from(col).ok())
                .and_then(|(row, col)| screen.cell(row, col))
                .is_some_and(|cell| {
                    cell.is_wide_continuation()
                        || (cell.has_contents() && !cell.contents().trim().is_empty())
                })
        };
        match unit {
            Unit::Line => (
                Point { col: 0, ..point },
                Point {
                    col: cols - 1,
                    ..point
                },
            ),
            Unit::Word if word_at(point.col) => {
                let mut first = point.col;
                while first > 0 && word_at(first - 1) {
                    first -= 1;
                }
                let mut last = point.col;
                while last + 1 < cols && word_at(last + 1) {
                    last += 1;
                }
                (
                    Point {
                        col: first,
                        ..point
                    },
                    Point { col: last, ..point },
                )
            }
            Unit::Word | Unit::Cell => (point, point),
        }
    }

    /// `row` and `col` as the view reported them, kept inside the shown text.
    fn clamp(&self, row: i32, col: i32) -> Option<Point> {
        let (rows, cols) = self.extent()?;
        let clamp = |n: i32, size: usize| usize::try_from(n.max(0)).unwrap_or(0).min(size - 1);
        Some(Point {
            row: clamp(row, rows),
            col: clamp(col, cols),
        })
    }

    /// The left button went down on `row`, `col`: starts a selection there, by
    /// cells, words or lines after one, two or three clicks.
    pub fn press(&mut self, row: i32, col: i32, now: Instant) {
        let Some(point) = self.clamp(row, col) else {
            self.selection = None;
            return;
        };
        let count = match self.last_press {
            Some((at, last, count))
                if last == point && now.saturating_duration_since(at) < MULTI_CLICK =>
            {
                count % 3 + 1
            }
            _ => 1,
        };
        self.last_press = Some((now, point, count));
        let unit = match (count, self.shell().is_some()) {
            (_, false) | (3, _) => Unit::Line,
            (2, _) => Unit::Word,
            _ => Unit::Cell,
        };
        let range = self.unit_at(unit, point);
        self.selection = Some(Selection {
            unit,
            anchor: range,
            head: range,
            click: count == 1,
            view: (self.current, self.generation),
        });
    }

    /// The pointer moved to `row`, `col` with the button down; true when the
    /// selection changed.
    pub fn drag(&mut self, row: i32, col: i32) -> bool {
        let Some(point) = self.clamp(row, col) else {
            return false;
        };
        let Some(selection) = self.selection else {
            return false;
        };
        let head = self.unit_at(selection.unit, point);
        let click = selection.click && head == selection.anchor;
        if head == selection.head && click == selection.click {
            return false;
        }
        self.selection = Some(Selection {
            head,
            click,
            ..selection
        });
        true
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        let (first, last) = self.selection()?.span();
        if let Some(shell) = self.shell() {
            let to_u16 = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
            let text = shell.parser.screen().contents_between(
                to_u16(first.row),
                to_u16(first.col),
                to_u16(last.row),
                to_u16(last.col + 1),
            );
            return Some(text);
        }
        let lines = &self.tabs.get(self.current)?.lines;
        let picked: Vec<&str> = (lines.iter().skip(first.row))
            .take(last.row - first.row + 1)
            .map(|line| line.text.as_str())
            .collect();
        Some(picked.join("\n"))
    }

    /// Forgets the selection, e.g. once it is copied.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// The current tab's text, for copying.
    pub fn text(&self) -> String {
        if let Some(shell) = self.shell() {
            return shell.text();
        }
        (self.tabs.get(self.current).into_iter())
            .flat_map(|tab| &tab.lines)
            .map(|line| format!("{}\n", line.text))
            .collect()
    }
}

/// The hosts whose remote work a request shows in the terminal, if any.
pub fn hosts(request: &serde_json::Value) -> Vec<String> {
    match request["action"].as_str().unwrap_or_default() {
        "check" | "upgrade" | "dist_upgrade" | "reboot" => request["id"]
            .as_str()
            .map(str::to_owned)
            .into_iter()
            .collect(),
        "batch" => {
            let mut ids: Vec<String> = Vec::new();
            for id in (request["ids"].as_array().into_iter().flatten()).filter_map(|v| v.as_str()) {
                if !ids.iter().any(|known| known == id) {
                    ids.push(id.to_owned());
                }
            }
            ids
        }
        _ => vec![],
    }
}

fn view(line: &Line) -> TerminalLine {
    TerminalLine {
        text: line.text.as_str().into(),
        command: line.command,
    }
}

pub fn push(app: &mut App, w: &AppWindow) {
    w.global::<ToolWindow>().set_open(Tool::index(app.tool));
    let state = w.global::<TerminalState>();
    let t = app.t();
    let typing_allowed = !app.busy && !app.locked;
    let opener = app.current().map(|host| host.name.clone());
    let terminal = &mut app.terminal;
    state.set_shown(!terminal.tabs.is_empty() || !terminal.shells.is_empty());
    state.set_current(terminal.current as i32);
    state.set_can_open_shell(typing_allowed && opener.is_some());
    state.set_open_shell_hint(
        opener
            .map(|name| t.format("shell.open_hint", &[("name", name)]))
            .unwrap_or_else(|| t.get("shell.open_none").into())
            .into(),
    );
    let tabs: Vec<TerminalTab> = (terminal.tabs.iter())
        .map(|tab| TerminalTab {
            name: tab.name.as_str().into(),
            state: tab.state.index(),
            shell: false,
        })
        .chain(terminal.shells.iter().map(|shell| TerminalTab {
            name: shell.name.as_str().into(),
            state: match shell.state {
                ShellState::Opening => 0,
                ShellState::Open => 1,
                ShellState::Failed => 3,
                ShellState::Closed => 4,
            },
            shell: true,
        }))
        .collect();
    let shown = state.get_tabs();
    if shown.row_count() != tabs.len() || shown.iter().zip(&tabs).any(|(a, b)| a != *b) {
        state.set_tabs(ModelRc::new(VecModel::from(tabs)));
    }
    // One model for the panel's life: lines are appended, never the whole list replaced,
    // so the view keeps its scroll position.
    let model = state.get_lines();
    let lines = match model.as_any().downcast_ref::<VecModel<TerminalLine>>() {
        Some(_) => model,
        None => {
            let model = ModelRc::from(Rc::new(VecModel::<TerminalLine>::default()));
            state.set_lines(model.clone());
            model
        }
    };
    let lines = lines
        .as_any()
        .downcast_ref::<VecModel<TerminalLine>>()
        .expect("terminal lines are a VecModel");
    let broken = push_shell(terminal, &state, typing_allowed, t);
    push_selection(terminal, &state);
    let current = terminal.tabs.get(terminal.current);
    let key = (terminal.current, terminal.generation);
    if terminal.synced != Some(key) {
        lines.set_vec(current.map_or_else(Vec::new, |tab| tab.lines.iter().map(view).collect()));
        terminal.synced = Some(key);
    } else if let Some(tab) = current {
        for line in tab.lines.iter().skip(lines.row_count()) {
            lines.push(view(line));
        }
    }
    // Its screen could not be drawn: end it on the remote side too. Its end is heard
    // and audited as usual.
    if let Some(id) = broken {
        app.worker.shells.send(id, Input::Close);
    }
}

/// The current tab's selection, for the view to highlight.
fn push_selection(terminal: &Terminal, state: &TerminalState) {
    let span = terminal.selection().map(|s| s.span());
    state.set_selection_shown(span.is_some());
    if let Some((first, last)) = span {
        let int = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
        state.set_selection_first_row(int(first.row));
        state.set_selection_first_col(int(first.col));
        state.set_selection_last_row(int(last.row));
        state.set_selection_last_col(int(last.col));
    }
}

/// The current shell's screen, cursor and whether keys reach it. Returns the shell
/// whose screen failed to draw, which must be closed.
fn push_shell(
    terminal: &mut Terminal,
    state: &TerminalState,
    typing_allowed: bool,
    t: crate::i18n::Translator,
) -> Option<u64> {
    let current = terminal.current;
    let Some(shell) = terminal.shell() else {
        state.set_shell_view(false);
        terminal.screen_synced = None;
        return None;
    };
    state.set_shell_view(true);
    let open = shell.state == ShellState::Open;
    state.set_shell_typing(open && typing_allowed);
    state.set_shell_notice(
        match shell.state {
            ShellState::Opening => t.get("shell.opening").into(),
            ShellState::Open if !typing_allowed => t.get("shell.paused").into(),
            ShellState::Open => String::new(),
            ShellState::Failed if shell.note.is_empty() => t.get("shell.emulator_failed").into(),
            ShellState::Closed | ShellState::Failed => shell.note.clone(),
        }
        .into(),
    );
    let mut broken = None;
    if terminal.screen_dirty || terminal.screen_synced != Some(current) {
        let screen = shell.parser.screen();
        let (row, col) = screen.cursor_position();
        // The emulator's state came from the remote side; a panic drawing it fails
        // this shell, not the app.
        let drawn = catch_unwind(AssertUnwindSafe(|| remote_shell::rows(screen)));
        let rows: Vec<ShellRow> = match drawn {
            Ok(rows) => {
                state.set_cursor_row(i32::from(row));
                state.set_cursor_col(i32::from(col));
                state.set_cursor_shown(open && !screen.hide_cursor() && screen.scrollback() == 0);
                rows
            }
            Err(_) => {
                let id = terminal.shell().map(|shell| shell.id);
                if let Some(shell) = id.and_then(|id| terminal.shell_by_id(id)) {
                    shell.break_down();
                    broken = Some(shell.id);
                }
                state.set_cursor_shown(false);
                Vec::new()
            }
        };
        state.set_screen(ModelRc::new(VecModel::from(rows)));
        terminal.screen_dirty = false;
        terminal.screen_synced = Some(current);
    }
    broken
}

impl App {
    /// Opens an interactive shell on the chosen host in a new terminal tab.
    pub fn open_shell(&mut self) {
        if self.busy || self.locked {
            return;
        }
        let Some(host) = self.current() else {
            return;
        };
        let (id, name) = (host.id.clone(), host.name.clone());
        let shell = self.worker.shells.reserve();
        let size = self.terminal.size();
        self.terminal
            .add_shell(ShellTab::new(shell, id.clone(), name, size));
        self.terminal.pending_open = Some(shell);
        self.tool = Some(Tool::Terminal);
        self.send(json!({
            "action": "shell_open",
            "id": id,
            "shell": shell,
            "cols": size.0,
            "rows": size.1,
        }));
    }

    /// The answer to a request to open a shell.
    pub fn shell_opened(&mut self, result: &Result<serde_json::Value, String>) {
        let t = self.t();
        let Some(id) = self.terminal.pending_open.take() else {
            return;
        };
        if let Some(shell) = self.terminal.shell_by_id(id)
            && shell.state == ShellState::Opening
        {
            match result {
                Ok(_) => {
                    shell.state = ShellState::Open;
                    shell.connected = true;
                }
                Err(error) => {
                    shell.state = ShellState::Failed;
                    shell.note = t.format("shell.failed", &[("error", error.clone())]);
                }
            }
            self.terminal.screen_dirty = true;
        }
    }

    /// A shell ended, by the user, the remote side or a lock; its tab stays to read.
    pub fn shell_ended(&mut self, id: u64, error: Option<String>) {
        let t = self.t();
        let note = match &error {
            Some(error) => t.format("shell.broken", &[("error", error.clone())]),
            None => t.get("shell.ended").into(),
        };
        if let Some(host) = self.terminal.shell_ended(id, error.is_some(), note) {
            let outcome = if error.is_some() {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            };
            let host = self.hosts.iter().find(|h| h.id == host);
            if self
                .audit
                .record(Action::ShellClose, outcome, host, None)
                .is_err()
            {
                self.error = Some(self.t().get("audit.failed").into());
            }
        }
    }

    /// Ends every open shell and logs each end now: the vault is locking or the app is
    /// closing, and their own end may never be heard.
    pub fn end_shells(&mut self) {
        let t = self.t();
        let ended: Vec<String> = (self.terminal.shells.iter_mut())
            .filter(|shell| shell.connected)
            .map(|shell| {
                shell.connected = false;
                if shell.state == ShellState::Open {
                    shell.state = ShellState::Closed;
                    shell.note = t.get("shell.ended").into();
                }
                shell.host.clone()
            })
            .collect();
        self.worker.shells.close_all();
        for id in ended {
            let host = self.hosts.iter().find(|h| h.id == id);
            if (self.audit)
                .record(Action::ShellClose, Outcome::Succeeded, host, None)
                .is_err()
            {
                self.error = Some(t.get("audit.failed").into());
            }
        }
    }

    /// Sends the current shell bytes typed into it.
    fn type_into_shell(&mut self, bytes: Vec<u8>) -> bool {
        if self.busy || self.locked {
            return false;
        }
        self.terminal.selection = None;
        let Some(shell) = self.terminal.shell_mut() else {
            return false;
        };
        if shell.state != ShellState::Open {
            return false;
        }
        let id = shell.id;
        // Typing returns to the live screen.
        if shell.parser.screen().scrollback() > 0 {
            shell.parser.screen_mut().set_scrollback(0);
            self.terminal.screen_dirty = true;
        }
        self.worker.shells.send(id, Input::Data(bytes))
    }
}

pub fn wire(ctx: &Ctx, w: &AppWindow) {
    let state = w.global::<TerminalState>();
    let c = ctx.clone();
    state.on_select(move |index| {
        c.update(|app| {
            if let Ok(index) = usize::try_from(index)
                && index < app.terminal.tabs.len() + app.terminal.shells.len()
            {
                app.terminal.current = index;
            }
        })
    });
    let c = ctx.clone();
    state.on_open_shell(move || c.update(App::open_shell));
    let c = ctx.clone();
    state.on_close_shell(move |index| {
        c.update(|app| {
            let Some(index) = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(app.terminal.tabs.len()))
            else {
                return;
            };
            let Some(shell) = app.terminal.shells.get(index) else {
                return;
            };
            // Still connected, even if its screen failed: end it first. Its end arrives
            // as an event, is audited and marks the tab; the next click removes it.
            if shell.connected {
                app.worker.shells.send(shell.id, Input::Close);
                return;
            }
            match shell.state {
                // Still waiting for the worker; once nothing runs, the tab is stale.
                ShellState::Opening if app.busy => {}
                _ => app.terminal.remove_shell(index),
            }
        })
    });
    let c = ctx.clone();
    state.on_shell_key(move |text, control, alt, shift| {
        let typed = {
            let Ok(mut app) = c.app.try_borrow_mut() else {
                return false;
            };
            let cursor_app = app
                .terminal
                .shell()
                .is_some_and(|shell| shell.parser.screen().application_cursor());
            let Some(bytes) = remote_shell::key_bytes(&text, control, alt, shift, cursor_app)
            else {
                return false;
            };
            app.type_into_shell(bytes)
        };
        c.update(|_| {});
        typed
    });
    let c = ctx.clone();
    state.on_shell_paste(move |text| {
        c.update(|app| {
            let bracketed = app
                .terminal
                .shell()
                .is_some_and(|shell| shell.parser.screen().bracketed_paste());
            app.type_into_shell(remote_shell::paste_bytes(&text, bracketed));
        })
    });
    let c = ctx.clone();
    state.on_shell_send(move |name| {
        if let Some(bytes) = remote_shell::kept_key_bytes(&name) {
            c.update(|app| {
                app.type_into_shell(bytes.to_vec());
            });
        }
    });
    let c = ctx.clone();
    state.on_shell_resized(move |cols, rows| {
        let (Ok(cols), Ok(rows)) = (u16::try_from(cols), u16::try_from(rows)) else {
            return;
        };
        let size = (cols.clamp(2, 1000), rows.clamp(2, 1000));
        c.update(|app| {
            if app.terminal.size == Some(size) {
                return;
            }
            app.terminal.size = Some(size);
            app.terminal.selection = None;
            for shell in &mut app.terminal.shells {
                shell.resize(size);
                if shell.state == ShellState::Open {
                    app.worker.shells.send(
                        shell.id,
                        Input::Resize {
                            cols: u32::from(size.0),
                            rows: u32::from(size.1),
                        },
                    );
                }
            }
            app.terminal.screen_dirty = true;
        })
    });
    let c = ctx.clone();
    state.on_shell_scrolled(move |lines| {
        c.update(|app| {
            if let Some(shell) = app.terminal.shell_mut() {
                shell.scroll(lines);
                app.terminal.screen_dirty = true;
                app.terminal.selection = None;
            }
        })
    });
    let c = ctx.clone();
    state.on_select_press(move |row, col| {
        c.update(|app| app.terminal.press(row, col, Instant::now()));
    });
    // Called on every pointer move while dragging: only the selection is redrawn.
    let c = ctx.clone();
    let main = w.as_weak();
    state.on_select_drag(move |row, col| {
        let Ok(mut app) = c.app.try_borrow_mut() else {
            return;
        };
        if app.terminal.drag(row, col)
            && let Some(main) = main.upgrade()
        {
            push_selection(&app.terminal, &main.global::<TerminalState>());
        }
    });
    let c = ctx.clone();
    let main = w.as_weak();
    state.on_copy_selection(move || {
        let Some(text) = c.app.borrow().terminal.selected_text() else {
            return false;
        };
        if let Some(main) = main.upgrade() {
            let clip = main.global::<Clip>();
            clip.set_text(SharedString::from(text));
            clip.set_serial(clip.get_serial() + 1);
        }
        // As terminals do, copying ends the selection.
        c.update(|app| app.terminal.clear_selection());
        true
    });
    let c = ctx.clone();
    let main = w.as_weak();
    state.on_copy(move || {
        let text = c.app.borrow().terminal.text();
        if let Some(main) = main.upgrade() {
            let clip = main.global::<Clip>();
            clip.set_text(SharedString::from(text));
            clip.set_serial(clip.get_serial() + 1);
        }
    });

    // The stripe of tool buttons and the shared tool window height.
    let tools = w.global::<ToolWindow>();
    let height = ctx.app.borrow().preferences.tool_height;
    tools.set_body_height(height.unwrap_or(crate::preferences::TOOL_HEIGHT));
    let c = ctx.clone();
    tools.on_chosen(move |index| {
        c.update(|app| {
            if let Some(tool) = Tool::from_index(index) {
                app.toggle_tool(tool);
            } else {
                app.tool = None;
            }
        })
    });
    let c = ctx.clone();
    tools.on_resized(move |height| {
        c.update(|app| {
            app.preferences.tool_height = Some(height);
            let _ = app.save_preferences();
        })
    });
}
