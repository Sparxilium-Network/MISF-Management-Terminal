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
    allow_passkey: bool,
    directory: std::path::PathBuf,
}
impl Session {
    pub fn new(paths: &RuntimePaths, audit: AuditLog) -> Self {
        Self {
            storage: StorageSession::new(paths.store.clone()),
            audit,
            debug: paths.debug,
            allow_passkey: paths.passkey_allowed(),
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
        let Some(action) = Action::request(&name, request["operation"].as_str()) else {
            self.audit
                .record(Action::Unsupported, Outcome::Blocked, None, None)?;
            anyhow::bail!("Unsupported action");
        };
        let batch = name == "batch";
        let batch_ids = if batch {
            crate::batch_ids(&request).unwrap_or_default()
        } else {
            vec![]
        };
        // Every host the request reaches: audit names each one, debug checks each address.
        let targets: Vec<Host> = if name == "save" {
            serde_json::from_value::<Host>(request["host"].clone())
                .ok()
                .into_iter()
                .collect()
        } else if batch {
            batch_ids
                .iter()
                .filter_map(|id| self.storage.host_for_audit(id).ok())
                .collect()
        } else {
            request["id"]
                .as_str()
                .and_then(|id| self.storage.host_for_audit(id).ok())
                .into_iter()
                .collect()
        };
        let slot = request["slot"]
            .as_u64()
            .and_then(|s| usize::try_from(s).ok());
        if self.debug {
            let remote = matches!(
                name.as_str(),
                "probe" | "test" | "check" | "upgrade" | "batch"
            );
            let blocked = (matches!(name.as_str(), "vault_add_passkey" | "vault_unlock_passkey")
                && !self.allow_passkey)
                || (remote
                    && (targets.is_empty()
                        || targets.len() < batch_ids.len()
                        || !targets.iter().all(|h| {
                            h.address
                                .parse::<std::net::IpAddr>()
                                .is_ok_and(|ip| ip.is_loopback())
                        })))
                || (name == "vault_backup"
                    && !request["path"].as_str().is_some_and(|path| {
                        std::path::Path::new(path)
                            .parent()
                            .and_then(|p| p.canonicalize().ok())
                            .is_some_and(|parent| parent.starts_with(&self.directory))
                    }));
            if blocked {
                self.record_each(action, Outcome::Blocked, &targets, slot)?;
                anyhow::bail!(
                    "Debug mode blocks this operation: remote SSH and external backups are prohibited; Passkey requires --debug.allow-passkey"
                );
            }
        }
        self.record_each(action, Outcome::Started, &targets, slot)?;
        let result = self.storage.handle(request, progress).await;
        // Failure to persist the outcome is visible; do not silently claim complete auditing.
        let logged = if batch {
            targets.iter().try_for_each(|host| {
                let succeeded = result.as_ref().is_ok_and(|value| {
                    value["results"].as_array().is_some_and(|results| {
                        results.iter().any(|entry| {
                            entry["id"] == host.id.as_str()
                                && entry["result"]["status"] == "success"
                        })
                    })
                });
                self.audit
                    .record(action, outcome(succeeded), Some(host), None)
            })
        } else {
            let succeeded = result.as_ref().is_ok_and(|value| {
                !matches!(name.as_str(), "check" | "upgrade") || value["status"] == "success"
            });
            self.audit.record(
                action,
                outcome(succeeded),
                targets.first(),
                slot.or_else(|| {
                    result
                        .as_ref()
                        .ok()
                        .and_then(|value| value["slot"].as_u64())
                        .and_then(|index| usize::try_from(index).ok())
                }),
            )
        };
        ensure!(
            logged.is_ok(),
            "Action finished, but audit outcome could not be saved; inspect state before retrying"
        );
        result
    }

    fn record_each(
        &self,
        action: Action,
        outcome: Outcome,
        targets: &[Host],
        slot: Option<usize>,
    ) -> Result<()> {
        if targets.is_empty() {
            return self.audit.record(action, outcome, None, slot);
        }
        for host in targets {
            self.audit.record(action, outcome, Some(host), slot)?;
        }
        Ok(())
    }
}

fn outcome(succeeded: bool) -> Outcome {
    if succeeded {
        Outcome::Succeeded
    } else {
        Outcome::Failed
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
            store: dir.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: false,
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
            store: dir.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: false,
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

    #[tokio::test]
    async fn hardware_opt_in_does_not_disable_other_debug_restrictions() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: dir.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: true,
        };
        let mut session = Session::new(&paths, AuditLog::new(dir.path()).unwrap());
        // No vault exists: failure happens before the native authenticator is constructed.
        let error = session
            .handle(json!({"action":"vault_add_passkey"}), &|_| {})
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("Debug mode"));
        for request in [
            json!({"action":"test","id":"missing"}),
            json!({"action":"vault_backup","path":dir.path().parent().unwrap().join("outside.lbx")}),
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
    }

    #[tokio::test]
    async fn debug_blocks_upgrades_and_batches_reaching_any_remote_or_unknown_host() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths {
            store: dir.path().join("hosts.db"),
            debug: true,
            debug_allow_passkey: false,
        };
        let mut session = Session::new(&paths, AuditLog::new(dir.path()).unwrap());
        for (id, name, address) in [
            ("local", "loopback", "127.0.0.1"),
            ("remote", "external", "192.0.2.10"),
        ] {
            let host = Host {
                id: id.into(),
                name: name.into(),
                address: address.into(),
                port: 22,
                username: "operator".into(),
                password: "secret-sentinel".into(),
                auth_type: "password".into(),
                os: "Linux".into(),
                ..Default::default()
            };
            session
                .handle(json!({"action":"save","host":host}), &|_| {})
                .await
                .unwrap();
        }
        for request in [
            json!({"action":"upgrade","id":"remote"}),
            json!({"action":"batch","operation":"upgrade","ids":["local","remote"]}),
            json!({"action":"batch","operation":"check","ids":["local","missing"]}),
            json!({"action":"batch","operation":"check","ids":[]}),
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
        // Each reached host is named in its own blocked record.
        assert!(
            text.contains(r#""action":"batch_upgrade","outcome":"blocked","host_name":"loopback""#)
        );
        assert!(
            text.contains(r#""action":"batch_upgrade","outcome":"blocked","host_name":"external""#)
        );
        assert!(!text.contains("192.0.2.10") && !text.contains("secret-sentinel"));
    }
}
