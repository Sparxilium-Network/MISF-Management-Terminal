//! The full log of this run, kept only in memory and gone when MISF closes.
//!
//! It records UI actions, requests, progress, results (with the core's already-redacted
//! remote output) and errors. Requests are reduced to a fixed set of non-secret fields, so
//! passwords, passphrases, private keys and host records never reach it. Nothing is written
//! to disk unless the user exports it.

use chrono::Local;
use misf_core::model::CheckResult;
use serde_json::Value;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::Path;
use zeroize::Zeroizing;

/// Oldest entries are dropped past this, so a long session cannot exhaust memory.
const LIMIT: usize = 16 * 1024 * 1024;

/// Request fields that never carry secrets; everything else is left out.
const REQUEST_FIELDS: [&str; 4] = ["id", "ids", "operation", "slot"];

#[derive(Default)]
pub struct SessionLog {
    entries: VecDeque<Zeroizing<String>>,
    bytes: usize,
    dropped: usize,
    started: String,
}

impl SessionLog {
    pub fn new() -> Self {
        Self {
            started: Local::now().to_rfc3339(),
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Adds one entry; `detail` lines are indented under it.
    pub fn add(&mut self, kind: &str, message: &str, detail: &str) {
        let mut entry = Zeroizing::new(String::new());
        let _ = write!(
            entry,
            "{} [{kind}] {}",
            Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            message.trim_end()
        );
        for line in detail.lines() {
            entry.push_str("\n    ");
            entry.push_str(line);
        }
        entry.push('\n');
        self.bytes += entry.len();
        self.entries.push_back(entry);
        while self.bytes > LIMIT && self.entries.len() > 1 {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.len();
                self.dropped += 1;
            }
        }
    }

    /// A request by action name and its non-secret fields only.
    pub fn request(&mut self, request: &Value) {
        let action = request["action"].as_str().unwrap_or("?");
        let mut message = action.to_owned();
        for field in REQUEST_FIELDS {
            if !request[field].is_null() {
                let _ = write!(message, " {field}={}", request[field]);
            }
        }
        self.add("REQUEST", &message, "");
    }

    /// One check or upgrade outcome, with its redacted remote output.
    pub fn result(&mut self, kind: &str, host: &str, result: &CheckResult) {
        let mut message = format!(
            "{host}: {} {} distro={:?} manager={:?} pending={} upgraded={} phased={}",
            result.operation,
            result.status,
            result.distro,
            result.package_manager,
            result.packages.len(),
            result.upgraded.len(),
            result.phased.len(),
        );
        if let Some(restart) = &result.restart {
            let _ = write!(
                message,
                " reboot_required={} services={} containers={} sessions={}",
                restart.reboot_required,
                restart.services.len(),
                restart.containers.len(),
                restart.sessions.len()
            );
        }
        let mut detail = String::new();
        if !result.error.is_empty() {
            let _ = writeln!(detail, "error: {}", result.error);
        }
        if !result.output.is_empty() {
            detail.push_str("output:\n");
            detail.push_str(&misf_core::transcript::readable_output(&result.output));
        }
        self.add(kind, &message, &detail);
    }

    /// The whole log as one text file, oldest entry first.
    pub fn render(&self) -> Zeroizing<String> {
        let mut text = Zeroizing::new(String::new());
        let _ = writeln!(
            text,
            "MISF Management Terminal {} session log",
            env!("CARGO_PKG_VERSION")
        );
        let _ = writeln!(text, "started: {}", self.started);
        let _ = writeln!(text, "exported: {}", Local::now().to_rfc3339());
        let _ = writeln!(text, "entries: {}", self.entries.len());
        if self.dropped > 0 {
            let _ = writeln!(
                text,
                "dropped: {} oldest entries (memory limit {} MiB)",
                self.dropped,
                LIMIT / 1024 / 1024
            );
        }
        text.push('\n');
        for entry in &self.entries {
            text.push_str(entry);
        }
        text
    }

    pub fn export(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.render().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_keep_only_non_secret_fields() {
        let mut log = SessionLog::new();
        log.request(&json!({"action":"vault_unlock","passphrase":"secret-passphrase"}));
        log.request(
            &json!({"action":"save","host":{"name":"web","password":"ssh-secret",
            "private_key":"-----BEGIN OPENSSH PRIVATE KEY-----","sudo_password":"sudo-secret"}}),
        );
        log.request(&json!({"action":"key_add","private_key":"key-body","passphrase":"pp"}));
        log.request(&json!({"action":"batch","operation":"upgrade","ids":["a","b"]}));
        let text = log.render();
        for secret in [
            "secret-passphrase",
            "ssh-secret",
            "sudo-secret",
            "PRIVATE KEY",
            "key-body",
            "\"pp\"",
            "web",
        ] {
            assert!(!text.contains(secret), "{secret}");
        }
        assert!(text.contains("[REQUEST] vault_unlock\n"));
        assert!(text.contains(r#"[REQUEST] batch ids=["a","b"] operation="upgrade""#));
    }

    #[test]
    fn results_carry_output_and_the_oldest_entries_go_past_the_limit() {
        let mut log = SessionLog::new();
        let result = CheckResult {
            operation: "check".into(),
            status: "failed".into(),
            error: "索引更新失敗".into(),
            output: "line one\r\nline two\n".into(),
            ..Default::default()
        };
        log.result("RESULT", "web", &result);
        let text = log.render();
        assert!(text.contains("[RESULT] web: check failed"));
        assert!(
            text.contains("\n    error: 索引更新失敗\n    output:\n    line one\n    line two\n")
        );

        let chunk = "x".repeat(1024 * 1024);
        for _ in 0..20 {
            log.add("PROGRESS", &chunk, "");
        }
        assert!(log.bytes() <= LIMIT);
        let text = log.render();
        assert!(text.contains("dropped: "));
        assert!(!text.contains("[RESULT]"));
    }

    #[test]
    fn export_writes_the_rendered_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("misf.log");
        let mut log = SessionLog::new();
        log.add("UI", "settings_open succeeded", "");
        log.export(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("MISF Management Terminal "));
        assert!(text.contains("[UI] settings_open succeeded"));
    }
}
