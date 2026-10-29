//! Shared GUI/CLI audit and debug policy, outside the storage protocol.
use crate::{
    audit::{Action, AuditLog, Outcome},
    model::Host,
    runtime::RuntimePaths,
    vault::StorageSession,
};
use anyhow::{Result, ensure};
use serde_json::Value;

pub struct Session {
    storage: StorageSession,
    pub audit: AuditLog,
    debug: bool,
    directory: std::path::PathBuf,
}
impl Session {
    pub fn new(paths: &RuntimePaths, audit: AuditLog) -> Self {
        Self {
            storage: StorageSession::new(paths.store.clone()),
            audit,
            debug: paths.debug,
            directory: paths
                .store
                .parent()
                .unwrap()
                .canonicalize()
                .unwrap_or_else(|_| paths.store.parent().unwrap().to_path_buf()),
        }
    }
    pub async fn handle(&mut self, request: Value, progress: &dyn Fn(&str)) -> Result<Value> {
        let name = request["action"].as_str().unwrap_or_default().to_owned();
        let Some(action) = Action::request(&name) else {
            self.audit
                .record(Action::Unsupported, Outcome::Blocked, None, None)?;
            anyhow::bail!("Unsupported action");
        };
        let host = if name == "save" {
            serde_json::from_value::<Host>(request["host"].clone()).ok()
        } else {
            request["id"]
                .as_str()
                .and_then(|id| self.storage.host_for_audit(id).ok())
        };
        let slot = request["slot"]
            .as_u64()
            .and_then(|s| usize::try_from(s).ok());
        if self.debug {
            let blocked = name == "vault_add_passkey"
                || name == "vault_unlock_passkey"
                || (matches!(name.as_str(), "probe" | "test" | "check")
                    && !host.as_ref().is_some_and(|h| {
                        h.address
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                    }))
                || (name == "vault_backup"
                    && !request["path"].as_str().is_some_and(|path| {
                        std::path::Path::new(path)
                            .parent()
                            .and_then(|p| p.canonicalize().ok())
                            .is_some_and(|parent| parent.starts_with(&self.directory))
                    }));
            if blocked {
                self.audit
                    .record(action, Outcome::Blocked, host.as_ref(), slot)?;
                anyhow::bail!(
                    "Debug mode blocks real Passkey operations, SSH outside loopback IPs and backup exports outside the debug directory"
                );
            }
        }
        self.audit
            .record(action, Outcome::Started, host.as_ref(), slot)?;
        let result = self.storage.handle(request, progress).await;
        let succeeded = result
            .as_ref()
            .is_ok_and(|value| name != "check" || value["status"] == "success");
        // Failure to persist the outcome is visible; do not silently claim complete auditing.
        let logged = self.audit.record(
            action,
            if succeeded {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            },
            host.as_ref(),
            slot.or_else(|| {
                result
                    .as_ref()
                    .ok()
                    .and_then(|value| value["slot"].as_u64())
                    .and_then(|index| usize::try_from(index).ok())
            }),
        );
        ensure!(
            logged.is_ok(),
            "Action finished, but audit outcome could not be saved; inspect state before retrying"
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn debug_blocks_remote_network_and_authenticator_and_logs_only_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: dir.path().join("hosts.json"),
            debug: true,
        };
        let log = AuditLog::new(dir.path()).unwrap();
        let mut session = Session::new(&paths, log);
        let host = Host {
            id: "test".into(),
            name: "example".into(),
            address: "192.0.2.10".into(),
            port: 22,
            username: "root".into(),
            password: "secret-sentinel".into(),
            auth_type: "password".into(),
            os: "Linux".into(),
            ..Default::default()
        };
        session
            .handle(json!({"action":"save","host":host}), &|_| {})
            .await
            .unwrap();
        for request in [
            json!({"action":"test","id":"test"}),
            json!({"action":"vault_add_passkey"}),
            json!({"action":"vault_unlock_passkey"}),
        ] {
            assert!(
                session
                    .handle(request, &|_| {})
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("Debug mode")
            );
        }
        let path = std::fs::read_dir(dir.path().join("logs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(path).unwrap();
        for secret in ["192.0.2.10", "secret-sentinel", "root"] {
            assert!(!text.contains(secret));
        }
        assert_eq!(text.lines().count(), 5);
        assert!(text.contains("blocked"));
    }

    #[tokio::test]
    async fn unavailable_log_blocks_mutations_before_they_start() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: dir.path().join("hosts.json"),
            debug: true,
        };
        let log = AuditLog::new(dir.path()).unwrap();
        std::fs::remove_dir(dir.path().join("logs")).unwrap();
        std::fs::write(dir.path().join("logs"), b"unavailable").unwrap();
        let mut session = Session::new(&paths, log);
        assert!(
            session
                .handle(json!({"action":"save","host":{}}), &|_| {})
                .await
                .is_err()
        );
        assert!(!paths.store.exists());
    }
}
