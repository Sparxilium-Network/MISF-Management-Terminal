//! Live remote output for a terminal view. Each line is cleaned and redacted before it
//! leaves, the same way the saved transcript is.

use crate::model::Host;
use crate::package_manager::SUDO_PROMPT;
use crate::transcript::readable_output;
use std::sync::Arc;

/// Receives what a host's commands print, as it arrives. `host` is the host ID.
pub trait Console: Send + Sync {
    /// A command about to run.
    fn command(&self, host: &str, command: &str);
    /// Complete lines of its output, each ending with a newline.
    fn output(&self, host: &str, text: &str);
}

/// Shows nothing: the CLI and tests.
pub struct Silent;
impl Console for Silent {
    fn command(&self, _: &str, _: &str) {}
    fn output(&self, _: &str, _: &str) {}
}

/// One host's side of a console, holding that host's secrets to hide.
#[derive(Clone)]
pub struct Echo {
    console: Arc<dyn Console>,
    host: String,
    secrets: Vec<String>,
    active: bool,
}

/// Lines of a private key shorter than this are not hidden on their own: too likely to
/// match ordinary output, and too short to give the key away.
const KEY_LINE_MIN: usize = 16;

/// The most of one line shown live, in characters; longer lines are cut, marked, and
/// the rest of them dropped. The saved output keeps every line whole.
pub(crate) const LINE_LIMIT: usize = 8 * 1024;

/// Ends a line cut at [`LINE_LIMIT`].
const CUT: &str = "（此行過長，已截斷）";

impl Echo {
    pub fn new(console: Arc<dyn Console>, host: &Host) -> Self {
        let mut secrets: Vec<String> = [&host.password, &host.sudo_password, &host.key_passphrase]
            .into_iter()
            .filter(|secret| !secret.is_empty())
            .cloned()
            .collect();
        // Output arrives line by line, so a key is hidden one line at a time.
        secrets.extend(
            (host.private_key.lines())
                .map(str::trim)
                .filter(|line| line.len() >= KEY_LINE_MIN)
                .map(str::to_owned),
        );
        // Longest first, so a secret containing another is hidden whole.
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        Self {
            console,
            host: host.id.clone(),
            secrets,
            active: true,
        }
    }

    /// The longest secret in bytes, never fewer than its characters.
    fn longest(&self) -> usize {
        self.secrets.first().map_or(0, String::len)
    }

    /// Shows nothing and does no work.
    pub fn silent() -> Self {
        Self {
            console: Arc::new(Silent),
            host: String::new(),
            secrets: vec![],
            active: false,
        }
    }

    fn redact(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for secret in &self.secrets {
            text = text.replace(secret.as_str(), "[已隱藏]");
        }
        text
    }

    pub(crate) fn command(&self, command: &str) {
        if self.active {
            self.console.command(&self.host, &self.redact(command));
        }
    }

    /// Collects one command's raw output.
    pub(crate) fn lines(&self) -> Lines<'_> {
        Lines {
            echo: self,
            pending: Vec::new(),
            cut: false,
            after_return: false,
        }
    }
}

/// Splits raw output into lines as it arrives. A line is passed on once its end (`\n`,
/// or `\r` as progress bars use) arrives, so escape sequences, UTF-8 characters and
/// secrets inside it are never split; the last unfinished line is passed on when this
/// is dropped. A line longer than [`LINE_LIMIT`] is cut: see [`Lines::cut_line`].
pub(crate) struct Lines<'a> {
    echo: &'a Echo,
    /// The current line's bytes; at most [`LINE_LIMIT`] plus the longest secret.
    pending: Vec<u8>,
    /// The current line was too long: the rest of it is dropped.
    cut: bool,
    /// The last line ended with `\r`, so a `\n` right after it ends nothing more.
    after_return: bool,
}

impl Lines<'_> {
    pub(crate) fn push(&mut self, data: &[u8]) {
        if !self.echo.active {
            return;
        }
        let mut shown = String::new();
        let mut start = 0;
        // Only the new bytes are searched for line ends.
        for (index, &byte) in data.iter().enumerate() {
            let after_return = std::mem::replace(&mut self.after_return, false);
            match byte {
                b'\n' if after_return && index == start => start = index + 1,
                b'\n' | b'\r' => {
                    self.take(&data[start..index]);
                    self.end_line(byte == b'\r', &mut shown);
                    self.after_return = byte == b'\r';
                    start = index + 1;
                }
                _ => {}
            }
        }
        self.take(&data[start..]);
        self.show(&shown);
    }

    /// Adds bytes to the current line, keeping only what a cut line can still show.
    fn take(&mut self, bytes: &[u8]) {
        if self.cut {
            return;
        }
        self.pending.extend_from_slice(bytes);
        let keep = LINE_LIMIT + self.echo.longest();
        if self.pending.len() > keep {
            self.pending.truncate(keep);
            self.cut = true;
        }
    }

    /// Ends the current line, adding it to `shown`. A blank line ended by `\r` is a
    /// progress bar redrawing and shows nothing.
    fn end_line(&mut self, by_return: bool, shown: &mut String) {
        let bytes = std::mem::take(&mut self.pending);
        let text = if std::mem::replace(&mut self.cut, false) {
            self.cut_line(&bytes)
        } else {
            self.echo.redact(&self.clean(&bytes))
        };
        if text.is_empty() && by_return {
            return;
        }
        shown.push_str(&text);
        shown.push('\n');
    }

    fn clean(&self, bytes: &[u8]) -> String {
        readable_output(&String::from_utf8_lossy(bytes).replace(SUDO_PROMPT, ""))
            .trim_end_matches('\n')
            .to_owned()
    }

    /// A line too long to show whole: `pending` kept its first [`LINE_LIMIT`] bytes and
    /// as many again as the longest secret is long. A secret that starts before the end
    /// of that is either all inside, and hidden, or cut short at the very end; cleaning
    /// may shorten the text, so the end is always dropped by the longest secret's
    /// length, which takes any such partial secret with it, before cutting to the limit.
    fn cut_line(&self, bytes: &[u8]) -> String {
        let text = self.echo.redact(&self.clean(bytes));
        let drop = self.echo.longest();
        let kept = text.chars().count().saturating_sub(drop);
        let mut text: String = text.chars().take(kept.min(LINE_LIMIT)).collect();
        // A character the cut split in two.
        while text.ends_with('\u{fffd}') {
            text.pop();
        }
        text.push_str(CUT);
        text
    }

    fn show(&self, text: &str) {
        if !text.is_empty() {
            self.echo.console.output(&self.echo.host, text);
        }
    }
}

impl Drop for Lines<'_> {
    fn drop(&mut self) {
        if !self.pending.is_empty() || self.cut {
            let mut shown = String::new();
            self.end_line(false, &mut shown);
            self.show(&shown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Record(Mutex<Vec<(String, String)>>);
    impl Console for Record {
        fn command(&self, host: &str, command: &str) {
            self.0
                .lock()
                .unwrap()
                .push((host.into(), format!("$ {command}\n")));
        }
        fn output(&self, host: &str, text: &str) {
            self.0.lock().unwrap().push((host.into(), text.into()));
        }
    }

    #[test]
    fn output_is_passed_on_in_whole_lines_with_secrets_hidden() {
        let record = Arc::new(Record::default());
        let host = Host {
            id: "web".into(),
            password: "hunter2-secret".into(),
            private_key: "-----BEGIN KEY-----\nAAAAB3NzaC1yc2EAAAADAQABAAAB\n-----END KEY-----"
                .into(),
            ..Default::default()
        };
        let echo = Echo::new(record.clone(), &host);
        echo.command("sudo -S apt-get update");
        {
            let mut lines = echo.lines();
            // A secret, a UTF-8 character and an escape split across chunks.
            lines.push(b"Get:1 hunter2-");
            lines.push(b"secret ok\n\xe4\xb8");
            lines.push(b"\xad\x1b[3");
            lines.push(b"1mred\x1b[0m\npartial AAAAB3NzaC1yc2EAAAADAQABAAAB");
            lines.push(SUDO_PROMPT.as_bytes());
        }
        let seen = record.0.lock().unwrap().clone();
        let text: String = seen.iter().map(|(_, text)| text.as_str()).collect();
        assert!(seen.iter().all(|(id, _)| id == "web"));
        assert_eq!(
            text,
            "$ sudo -S apt-get update\nGet:1 [已隱藏] ok\n中red\npartial [已隱藏]\n"
        );
    }

    fn shown(lines: impl FnOnce(&Echo), host: Host) -> String {
        let record = Arc::new(Record::default());
        let echo = Echo::new(record.clone(), &host);
        lines(&echo);
        let seen = record.0.lock().unwrap().clone();
        seen.iter().map(|(_, text)| text.as_str()).collect()
    }

    fn with_password(password: &str) -> Host {
        Host {
            id: "web".into(),
            password: password.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_long_line_is_cut_and_the_next_line_shows_whole() {
        let text = shown(
            |echo| {
                let mut lines = echo.lines();
                lines.push(&vec![b'x'; 20_000]);
                lines.push(&vec![b'x'; 20_000]);
                lines.push(b"\nnext line\n");
            },
            Host::default(),
        );
        let mut rows = text.lines();
        let first = rows.next().unwrap();
        assert_eq!(first, format!("{}{CUT}", "x".repeat(LINE_LIMIT)));
        assert_eq!(rows.next(), Some("next line"));
        assert_eq!(rows.next(), None);
    }

    #[test]
    fn no_part_of_a_secret_across_the_cut_is_shown() {
        let secret = "Qz7-secret-Wv9";
        let longest = secret.len();
        // Plain text up to a few bytes short of what is kept, then the secret.
        let before = LINE_LIMIT + longest - 4;
        let mut line = vec![b'a'; before];
        line.extend_from_slice(secret.as_bytes());
        line.extend_from_slice(&[b'b'; 100]);
        let text = shown(
            |echo| {
                let mut lines = echo.lines();
                lines.push(&line);
                lines.push(b"\n");
            },
            with_password(secret),
        );
        assert!(!text.contains("Qz7"), "{}", &text[text.len() - 60..]);
        assert!(text.ends_with(&format!("{CUT}\n")));
    }

    #[test]
    fn a_line_mostly_of_escapes_shows_no_part_of_a_secret_at_the_cut() {
        let secret = "Qz7-secret-Wv9";
        let longest = secret.len();
        // Escapes clean away to nothing, so the cleaned text is far under the limit.
        let mut line: Vec<u8> = b"\x1b[0m".repeat((LINE_LIMIT + longest) / 4);
        line.truncate(LINE_LIMIT + longest - 10);
        line.extend_from_slice(b"ok ");
        line.extend_from_slice(secret.as_bytes());
        line.extend_from_slice(b" tail");
        let text = shown(
            |echo| {
                let mut lines = echo.lines();
                lines.push(&line);
            },
            with_password(secret),
        );
        for size in 2..=secret.len() {
            assert!(!text.contains(&secret[..size]), "{text}");
        }
        assert!(text.ends_with(&format!("{CUT}\n")), "{text}");
    }

    #[test]
    fn a_wide_character_across_the_cut_leaves_no_replacement_mark() {
        let mut line = vec![b'a'; LINE_LIMIT - 1];
        line.extend_from_slice("中文".repeat(10).as_bytes());
        let text = shown(
            |echo| {
                let mut lines = echo.lines();
                lines.push(&line);
                lines.push(b"\n");
            },
            Host::default(),
        );
        assert!(!text.contains('\u{fffd}'));
        assert!(text.ends_with(&format!("{CUT}\n")));
    }

    #[test]
    fn progress_redraws_show_as_they_come_and_crlf_split_in_two_is_one_end() {
        let record = Arc::new(Record::default());
        let echo = Echo::new(record.clone(), &Host::default());
        let seen = || -> Vec<String> {
            (record.0.lock().unwrap().iter())
                .map(|(_, text)| text.clone())
                .collect()
        };
        let mut lines = echo.lines();
        lines.push(b"10%\r");
        assert_eq!(seen(), ["10%\n"]);
        lines.push(b"\x1b[2K50%\r");
        assert_eq!(seen(), ["10%\n", "50%\n"]);
        // A \r\n split across two chunks ends one line, with no blank line after.
        lines.push(b"done\r");
        lines.push(b"\nnext\n\nlast\n");
        drop(lines);
        assert_eq!(seen(), ["10%\n", "50%\n", "done\n", "next\n\nlast\n"]);
    }

    #[test]
    fn a_silent_echo_does_nothing() {
        let echo = Echo::silent();
        echo.command("anything");
        let mut lines = echo.lines();
        lines.push(b"text\n");
        assert!(lines.pending.is_empty());
    }
}
