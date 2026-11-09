//! The full log of this run, kept only in memory and gone when MISF closes.
//!
//! It is deliberately complete: requests and responses in full, host IDs, addresses,
//! accounts, fingerprints and each host's remote output. Only secrets are masked —
//! passwords, sudo passwords, passphrases, PINs and private keys — wherever they appear
//! as fields. The core already replaces a host's secrets inside its remote output.
//! Nothing reaches the disk unless the user exports it.

use crate::i18n::Translator;
use chrono::Local;
use misf_core::model::{CheckResult, Host, Package};
use serde_json::Value;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::Path;
use zeroize::Zeroizing;

/// Oldest entries are dropped past this, so a long session cannot exhaust memory.
const LIMIT: usize = 16 * 1024 * 1024;

/// Fields that hold secrets, at any depth of a request or response.
const SECRET_FIELDS: [&str; 6] = [
    "password",
    "sudo_password",
    "key_passphrase",
    "passphrase",
    "private_key",
    "pin",
];
const MASK: &str = "[已遮蔽]";
const RULE: &str = "********";

#[derive(Default)]
pub struct SessionLog {
    entries: VecDeque<Zeroizing<String>>,
    bytes: usize,
    dropped: usize,
    started: String,
}

fn now() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

/// A copy with every secret field masked; an empty secret stays empty so it reads as unset.
fn mask(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let hidden = SECRET_FIELDS.contains(&key.as_str())
                        && value.as_str().is_none_or(|text| !text.is_empty());
                    let value = if hidden {
                        Value::String(MASK.into())
                    } else if key == "output" && value.is_string() {
                        // Remote output is logged once, in its host's block.
                        Value::String(format!(
                            "[{} bytes]",
                            value.as_str().unwrap_or_default().len()
                        ))
                    } else {
                        mask(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(mask).collect()),
        other => other.clone(),
    }
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

    fn push(&mut self, entry: Zeroizing<String>) {
        self.bytes += entry.len();
        self.entries.push_back(entry);
        while self.bytes > LIMIT && self.entries.len() > 1 {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.len();
                self.dropped += 1;
            }
        }
    }

    /// Adds one line; `detail` lines are indented under it.
    pub fn add(&mut self, kind: &str, message: &str, detail: &str) {
        let mut entry = Zeroizing::new(String::new());
        let _ = write!(entry, "{} [{kind}] {}", now(), message.trim_end());
        for line in detail.lines() {
            entry.push_str("\n    ");
            entry.push_str(line);
        }
        entry.push('\n');
        self.push(entry);
    }

    /// Marks the start of something the user did, such as a batch upgrade.
    pub fn banner(&mut self, title: &str) {
        let mut entry = Zeroizing::new(String::new());
        let _ = write!(entry, "\n{RULE}\n{title}  ({})\n{RULE}\n\n", now());
        self.push(entry);
    }

    /// A request in full, secrets masked.
    pub fn request(&mut self, request: &Value) {
        let action = request["action"].as_str().unwrap_or("?").to_owned();
        self.add("REQUEST", &format!("{action} {}", mask(request)), "");
    }

    /// A response in full, secrets masked and remote output left to the host blocks.
    pub fn response(&mut self, action: &str, data: &Value) {
        self.add("RESULT", &format!("{action} {}", mask(data)), "");
    }

    /// One host's check or upgrade: who it is, what changed, and its remote output.
    pub fn host(&mut self, t: Translator, name: &str, host: Option<&Host>, result: &CheckResult) {
        let mut entry = Zeroizing::new(String::new());
        let _ = writeln!(entry, "\n{RULE}\n{name}\n{RULE}");
        let line = |entry: &mut String, label: &str, value: &str| {
            let _ = writeln!(entry, "{label}: {value}");
        };
        if let Some(host) = host {
            line(&mut entry, t.get("log.host_id"), &host.id);
            line(
                &mut entry,
                t.get("log.connection"),
                &format!("{}@{}:{}", host.username, host.address, host.port),
            );
            line(&mut entry, t.get("log.fingerprint"), &host.fingerprint);
        }
        line(
            &mut entry,
            t.get("log.system"),
            &format!("{} / {}", result.distro, result.package_manager),
        );
        line(
            &mut entry,
            t.get("log.operation"),
            &format!(
                "{} / {} / {}",
                result.operation, result.status, result.checked_at
            ),
        );
        let packages = |entry: &mut String, label: String, list: &[Package]| {
            let _ = writeln!(entry, "{label}");
            for p in list {
                let _ = writeln!(
                    entry,
                    "    {} {} -> {} [{}] {}",
                    p.name, p.current_version, p.candidate_version, p.architecture, p.repository
                );
            }
        };
        let count = |list: &[Package]| [("count", list.len().to_string())];
        packages(
            &mut entry,
            t.format("log.pending", &count(&result.packages)),
            &result.packages,
        );
        if result.operation == "upgrade" {
            packages(
                &mut entry,
                t.format("log.upgraded", &count(&result.upgraded)),
                &result.upgraded,
            );
        }
        if !result.phased.is_empty() {
            line(&mut entry, t.get("log.phased"), &result.phased.join(", "));
        }
        if let Some(restart) = &result.restart {
            let _ = writeln!(
                entry,
                "{}: reboot_required={} needrestart={} kernel={} -> {}",
                t.get("log.restart"),
                restart.reboot_required,
                restart.needrestart,
                restart.running_kernel,
                restart.expected_kernel
            );
            for (label, list) in [
                ("packages", &restart.reboot_packages),
                ("services", &restart.services),
                ("containers", &restart.containers),
                ("sessions", &restart.sessions),
            ] {
                if !list.is_empty() {
                    let _ = writeln!(entry, "    {label}: {}", list.join(", "));
                }
            }
        }
        if !result.error.is_empty() {
            line(&mut entry, t.get("log.error"), &result.error);
        }
        let _ = writeln!(entry, "{}:", t.get("log.output"));
        entry.push_str(&result.output);
        if !result.output.ends_with('\n') {
            entry.push('\n');
        }
        let _ = writeln!(entry, "\n{RULE}\nEND OF {name}\n{RULE}\n");
        self.push(entry);
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
    use crate::i18n::Language;
    use serde_json::json;

    const T: Translator = Translator(Language::ZhTw);

    #[test]
    fn requests_and_responses_are_complete_except_for_secrets() {
        let mut log = SessionLog::new();
        log.request(&json!({"action":"vault_unlock","passphrase":"secret-passphrase"}));
        log.request(&json!({"action":"vault_unlock_passkey","pin":"1234","hello":false}));
        log.request(&json!({"action":"save","host":{"id":"h1","name":"web","address":"10.0.0.5",
            "username":"deploy","password":"ssh-secret","private_key":"-----BEGIN OPENSSH PRIVATE KEY-----",
            "key_passphrase":"","sudo_password":"sudo-secret"}}));
        log.request(
            &json!({"action":"key_add","name":"deploy","private_key":"key-body","passphrase":"pp"}),
        );
        log.response(
            "list",
            &json!({"hosts":[{"id":"h1","password":"ssh-secret","last_check":{"output":"long output"}}]}),
        );
        let text = log.render();
        for secret in [
            "secret-passphrase",
            "1234",
            "ssh-secret",
            "sudo-secret",
            "PRIVATE KEY",
            "key-body",
            "\"pp\"",
            "long output",
        ] {
            assert!(!text.contains(secret), "{secret}");
        }
        // Everything else stays: IDs, names, addresses, accounts.
        for kept in [
            r#""id":"h1""#,
            r#""name":"web""#,
            r#""address":"10.0.0.5""#,
            r#""username":"deploy""#,
            r#""key_passphrase":"""#,
            r#""output":"[11 bytes]""#,
        ] {
            assert!(text.contains(kept), "{kept}");
        }
    }

    #[test]
    fn hosts_are_framed_with_their_full_output() {
        let mut log = SessionLog::new();
        log.banner("使用者 進行批次 APT 升級（1 台）");
        let host = Host {
            id: "h1".into(),
            address: "10.0.0.5".into(),
            port: 22,
            username: "deploy".into(),
            fingerprint: "SHA256:abc".into(),
            ..Default::default()
        };
        let result = CheckResult {
            operation: "upgrade".into(),
            status: "success".into(),
            output: "Setting up openssl (3.0.17) ...\n".into(),
            upgraded: vec![Package {
                name: "openssl".into(),
                current_version: "3.0.16".into(),
                candidate_version: "3.0.17".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        log.host(T, "UBUNTU1", Some(&host), &result);
        let text = log.render();
        assert!(text.contains("\n********\n使用者 進行批次 APT 升級（1 台）  ("));
        let block = &text[text.find("\n********\nUBUNTU1\n********\n").unwrap()..];
        assert!(block.contains("h1"));
        assert!(block.contains("deploy@10.0.0.5:22"));
        assert!(block.contains("SHA256:abc"));
        assert!(block.contains("    openssl 3.0.16 -> 3.0.17"));
        assert!(block.contains("Setting up openssl (3.0.17) ...\n"));
        assert!(block.ends_with("\n********\nEND OF UBUNTU1\n********\n\n"));
    }

    #[test]
    fn the_oldest_entries_go_past_the_limit_and_export_writes_the_log() {
        let mut log = SessionLog::new();
        log.add("UI", "first", "");
        let chunk = "x".repeat(1024 * 1024);
        for _ in 0..20 {
            log.add("PROGRESS", &chunk, "");
        }
        assert!(log.bytes() <= LIMIT);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("misf.log");
        log.export(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("MISF Management Terminal "));
        assert!(text.contains("dropped: "));
        assert!(!text.contains("[UI] first"));
    }
}
