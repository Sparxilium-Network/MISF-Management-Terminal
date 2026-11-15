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
        }
    }
}

/// Splits raw output into complete lines as it arrives. A line is passed on only once
/// its newline arrives, so escape sequences, UTF-8 characters and secrets are never
/// split; the last unfinished line is passed on when this is dropped.
pub(crate) struct Lines<'a> {
    echo: &'a Echo,
    pending: Vec<u8>,
}

impl Lines<'_> {
    pub(crate) fn push(&mut self, data: &[u8]) {
        if !self.echo.active {
            return;
        }
        self.pending.extend_from_slice(data);
        if let Some(end) = self.pending.iter().rposition(|&byte| byte == b'\n') {
            let rest = self.pending.split_off(end + 1);
            let done = std::mem::replace(&mut self.pending, rest);
            self.emit(&done);
        }
    }

    fn emit(&self, bytes: &[u8]) {
        let text = readable_output(&String::from_utf8_lossy(bytes).replace(SUDO_PROMPT, ""));
        if !text.is_empty() {
            self.echo
                .console
                .output(&self.echo.host, &self.echo.redact(&text));
        }
    }
}

impl Drop for Lines<'_> {
    fn drop(&mut self) {
        if !self.pending.is_empty() {
            let mut last = std::mem::take(&mut self.pending);
            last.push(b'\n');
            self.emit(&last);
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

    #[test]
    fn a_silent_echo_does_nothing() {
        let echo = Echo::silent();
        echo.command("anything");
        let mut lines = echo.lines();
        lines.push(b"text\n");
        assert!(lines.pending.is_empty());
    }
}
