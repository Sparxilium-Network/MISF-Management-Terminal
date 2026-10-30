//! Allowlisted audit records. Never accept request bodies, output, or error strings.
use crate::model::Host;
use anyhow::{Context, Result};
use chrono::Utc;
use fs2::FileExt;
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    AppStart,
    DebugPasskeyEnabled,
    AppExit,
    SettingsOpen,
    SettingsClose,
    LanguageChange,
    AppearanceChange,
    HostSelect,
    HostSearch,
    PackageSearch,
    ResultTab,
    EditorOpen,
    EditorClose,
    RevealSecrets,
    ImportKey,
    DeletePrompt,
    DeleteCancel,
    TrustCancel,
    ErrorDismiss,
    VaultDialogOpen,
    VaultDialogClose,
    VaultRevokePrompt,
    VaultRevokeCancel,
    BackupDialog,
    StorageStatus,
    List,
    Save,
    Delete,
    Probe,
    Trust,
    Test,
    Check,
    Upgrade,
    BatchCheck,
    BatchUpgrade,
    UpgradePrompt,
    UpgradeCancel,
    MainTab,
    BatchSelect,
    VaultCreate,
    VaultUnlock,
    VaultLock,
    VaultAddKey,
    VaultAddPasskey,
    VaultUnlockPasskey,
    VaultRevokeKey,
    VaultBackup,
    ConvertStorage,
    PlaintextDeletePrompt,
    PlaintextDeleteCancel,
    VaultDeletePlaintext,
    FormEdit,
    ValidationRejected,
    PasskeyMethod,
    Unsupported,
}
impl Action {
    pub(crate) fn request(value: &str, operation: Option<&str>) -> Option<Self> {
        Some(match value {
            "storage_status" => Self::StorageStatus,
            "list" => Self::List,
            "save" => Self::Save,
            "delete" => Self::Delete,
            "probe" => Self::Probe,
            "trust" => Self::Trust,
            "test" => Self::Test,
            "check" => Self::Check,
            "upgrade" => Self::Upgrade,
            "batch" => match operation {
                Some("check") => Self::BatchCheck,
                Some("upgrade") => Self::BatchUpgrade,
                _ => return None,
            },
            "vault_create" => Self::VaultCreate,
            "vault_unlock" => Self::VaultUnlock,
            "vault_lock" => Self::VaultLock,
            "vault_add_key" => Self::VaultAddKey,
            "vault_add_passkey" => Self::VaultAddPasskey,
            "vault_unlock_passkey" => Self::VaultUnlockPasskey,
            "vault_revoke_key" => Self::VaultRevokeKey,
            "vault_backup" => Self::VaultBackup,
            "convert_storage" => Self::ConvertStorage,
            "vault_delete_plaintext" => Self::VaultDeletePlaintext,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Started,
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
}

#[derive(Clone)]
pub struct AuditLog {
    directory: Arc<PathBuf>,
    writer: Arc<Mutex<()>>,
}
impl AuditLog {
    pub fn new(profile: &Path) -> Result<Self> {
        let directory = profile.join("logs");
        fs::create_dir_all(&directory).context("Cannot create audit log directory")?;
        Ok(Self {
            directory: Arc::new(directory),
            writer: Arc::new(Mutex::new(())),
        })
    }
    pub fn record(
        &self,
        action: Action,
        outcome: Outcome,
        host: Option<&Host>,
        slot: Option<usize>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Record {
            time: String,
            pid: u32,
            action: Action,
            outcome: Outcome,
            #[serde(skip_serializing_if = "Option::is_none")]
            host_name: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            slot: Option<usize>,
        }
        let _guard = self
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("Audit log lock failed"))?;
        let now = Utc::now();
        let path = self
            .directory
            .join(format!("actions-{}.jsonl", now.format("%Y-%m-%d")));
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)
            .context("Cannot open audit log")?;
        file.lock_exclusive().context("Cannot lock audit log")?;
        let record = Record {
            time: now.to_rfc3339(),
            pid: std::process::id(),
            action,
            outcome,
            host_name: host.map(safe_name),
            slot,
        };
        let mut bytes = serde_json::to_vec(&record)?;
        bytes.push(b'\n');
        file.write_all(&bytes).context("Cannot write audit log")?;
        file.sync_data().context("Cannot flush audit log")?;
        Ok(())
    }
}

fn safe_name(host: &Host) -> String {
    let mut name = host.name.clone();
    // A display name may itself contain a copied address or credential.
    let mut forbidden = [
        &host.address,
        &host.password,
        &host.private_key,
        &host.key_passphrase,
        &host.sudo_password,
        &host.fingerprint,
    ];
    forbidden.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for value in forbidden {
        if !value.is_empty() {
            name = name.replace(value, "[redacted]");
        }
    }
    for line in host
        .private_key
        .lines()
        .map(str::trim)
        .filter(|line| line.len() >= 8)
    {
        name = name.replace(line, "[redacted]");
    }
    let name: String = name.chars().filter(|c| !c.is_control()).take(128).collect();
    redact_ips(&name)
}

fn redact_ips(name: &str) -> String {
    let mut result = String::new();
    let mut offset = 0;
    while offset < name.len() {
        let remaining = &name[offset..];
        let candidate_len = remaining
            .bytes()
            .take(45)
            .take_while(|b| b.is_ascii_hexdigit() || matches!(b, b'.' | b':'))
            .count();
        let ip_len = (2..=candidate_len)
            .rev()
            .find(|length| remaining[..*length].parse::<std::net::IpAddr>().is_ok());
        if let Some(length) = ip_len {
            result.push_str("[redacted]");
            offset += length;
        } else {
            let character = remaining.chars().next().unwrap();
            result.push(character);
            offset += character.len_utf8();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_has_only_allowlisted_fields_and_no_host_credentials_or_injected_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::new(dir.path()).unwrap();
        let host = Host {
            name: "web\n192.0.2.10 ssh-secret".into(),
            address: "192.0.2.10".into(),
            password: "ssh-secret".into(),
            private_key: "private-key-sentinel".into(),
            ..Default::default()
        };
        log.record(Action::Test, Outcome::Failed, Some(&host), None)
            .unwrap();
        let path = fs::read_dir(dir.path().join("logs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = fs::read_to_string(path).unwrap();
        assert_eq!(text.lines().count(), 1);
        for secret in ["192.0.2.10", "ssh-secret", "private-key-sentinel"] {
            assert!(!text.contains(secret));
        }
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["action"], "test");
        assert_eq!(value["host_name"], "web[redacted] [redacted]");
        assert_eq!(value.as_object().unwrap().len(), 5);
    }

    #[test]
    fn display_names_also_redact_other_literal_ips_and_private_key_lines() {
        let host = Host {
            name: "主機 203.0.113.8 [2001:db8::1234] secret-key-body".into(),
            private_key: "-----BEGIN KEY-----\nsecret-key-body\n-----END KEY-----".into(),
            ..Default::default()
        };
        assert_eq!(safe_name(&host), "主機 [redacted] [[redacted]] [redacted]");
    }
}
