//! `unsafe` is denied everywhere except the Windows WebAuthn FFI adapter.
#![deny(unsafe_code)]

pub mod audit;
pub mod batch;
mod db_upgrade;
pub mod model;
pub mod package_manager;
mod passkey;
pub mod reboot;
pub mod runtime;
pub mod session;
pub mod sqlite;
pub mod ssh;
pub mod store;
pub mod transcript;
pub mod vault;
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod windows_passkey;

use anyhow::{Context, Result, ensure};
use model::{Host, HostGroup, SshKey};
use serde_json::{Value, json};
use store::Store;

pub(crate) fn save_results(
    store: &Store,
    results: Vec<(String, model::CheckResult)>,
) -> Result<()> {
    store.update(|db| {
        for (id, result) in results {
            if let Some(current) = db.hosts.iter_mut().find(|h| h.id == id) {
                if !result.distro.is_empty() {
                    current.distro = result.distro.clone();
                    current.package_manager = result.package_manager.clone();
                }
                current.last_check = Some(result);
            }
        }
        Ok(())
    })
}

/// Applies a sidebar drag: `groups` lists every group ID and `hosts` every host as
/// `{id, group_id}`, both in their new order.
fn arrange(db: &mut store::Database, request: &Value) -> Result<()> {
    const STALE: &str = "主機清單已在其他地方變更，請重新整理後再拖曳";
    let groups: Vec<&str> = request["groups"]
        .as_array()
        .context("缺少分類順序")?
        .iter()
        .map(|id| id.as_str().context("分類 ID 格式錯誤"))
        .collect::<Result<_>>()?;
    let hosts: Vec<(&str, &str)> = request["hosts"]
        .as_array()
        .context("缺少主機順序")?
        .iter()
        .map(|entry| {
            Ok((
                entry["id"].as_str().context("主機 ID 格式錯誤")?,
                entry["group_id"].as_str().unwrap_or_default(),
            ))
        })
        .collect::<Result<_>>()?;
    let mut new_groups = Vec::with_capacity(groups.len());
    for id in &groups {
        let index = db.groups.iter().position(|g| g.id == *id).context(STALE)?;
        new_groups.push(db.groups.swap_remove(index));
    }
    ensure!(db.groups.is_empty(), STALE);
    let mut new_hosts = Vec::with_capacity(hosts.len());
    for (id, group_id) in hosts {
        ensure!(
            group_id.is_empty() || groups.contains(&group_id),
            "找不到分類"
        );
        let index = db.hosts.iter().position(|h| h.id == id).context(STALE)?;
        let mut host = db.hosts.swap_remove(index);
        host.group_id = group_id.into();
        new_hosts.push(host);
    }
    ensure!(db.hosts.is_empty(), STALE);
    db.groups = new_groups;
    db.hosts = new_hosts;
    Ok(())
}

pub async fn dispatch(request: Value, store: &Store, progress: &dyn Fn(&str)) -> Result<Value> {
    dispatch_with(request, store, progress, &batch::Unattended).await
}

/// [`dispatch`] with a batch observer that sees each host as it finishes.
pub async fn dispatch_with(
    request: Value,
    store: &Store,
    progress: &dyn Fn(&str),
    observer: &dyn batch::Observer,
) -> Result<Value> {
    let action = request["action"].as_str().context("缺少 action")?;
    let id = request["id"].as_str().unwrap_or_default();
    match action {
        "list" => {
            let database = store.read()?;
            let (format, schema_version) = store.format()?;
            Ok(json!({
                "hosts": database.hosts,
                "keys": database.keys.iter().map(SshKey::summary).collect::<Vec<_>>(),
                "groups": database.groups,
                "path": store.path,
                "encrypted": store.is_encrypted(),
                "format": format,
                "schema_version": schema_version,
            }))
        }
        "convert_storage" => {
            // Reading already converts a plaintext hosts.json; writing the unchanged
            // hosts converts a vault's legacy /hosts.json in one metadata commit.
            store.update(|_| Ok(()))?;
            let (format, schema_version) = store.format()?;
            Ok(json!({"format": format, "schema_version": schema_version}))
        }
        "save" => {
            let mut host: Host =
                serde_json::from_value(request["host"].clone()).context("主機資料格式錯誤")?;
            host.validate()?;
            if host.id.is_empty() {
                host.id = uuid::Uuid::new_v4().to_string();
            }
            store.update(|db| {
                ensure!(
                    host.key_id.is_empty() || db.keys.iter().any(|k| k.id == host.key_id),
                    "找不到所選的 SSH 金鑰；可能已被刪除"
                );
                if let Some(old) = db.hosts.iter_mut().find(|h| h.id == host.id) {
                    if old.address != host.address || old.port != host.port {
                        host.fingerprint.clear();
                        host.distro.clear();
                        host.package_manager.clear();
                        host.last_check = None;
                        host.last_test = None;
                    } else {
                        // Detection results only ever come from the host itself.
                        host.distro = old.distro.clone();
                        host.package_manager = old.package_manager.clone();
                        host.fingerprint = old.fingerprint.clone();
                        host.last_check = if old.os == host.os {
                            old.last_check.clone()
                        } else {
                            None
                        };
                        host.last_test = old.last_test.clone();
                    }
                    // Only dragging in the sidebar moves a host between groups.
                    host.group_id = old.group_id.clone();
                    *old = host.clone();
                } else {
                    if !db.groups.iter().any(|g| g.id == host.group_id) {
                        host.group_id.clear();
                    }
                    host.fingerprint.clear();
                    host.distro.clear();
                    host.package_manager.clear();
                    host.last_check = None;
                    host.last_test = None;
                    db.hosts.push(host.clone());
                }
                Ok(())
            })?;
            Ok(json!(host))
        }
        "delete" => {
            store.update(|db| {
                let before = db.hosts.len();
                db.hosts.retain(|h| h.id != id);
                ensure!(db.hosts.len() < before, "找不到主機");
                Ok(())
            })?;
            Ok(json!({"deleted": id}))
        }
        "key_add" => {
            let text = |field: &str| request[field].as_str().unwrap_or_default();
            let key = SshKey::new(
                text("name"),
                text("private_key").to_owned(),
                text("passphrase").to_owned(),
            )?;
            let summary = key.summary();
            store.update(|db| {
                if let Some(same) = db.keys.iter().find(|k| k.fingerprint == key.fingerprint) {
                    anyhow::bail!("此 SSH 金鑰已儲存為「{}」", same.name);
                }
                db.keys.push(key);
                Ok(())
            })?;
            Ok(json!({"key": summary}))
        }
        "key_delete" => {
            store.update(|db| {
                let users: Vec<&str> = db
                    .hosts
                    .iter()
                    .filter(|h| h.key_id == id)
                    .map(|h| h.name.as_str())
                    .collect();
                ensure!(
                    users.is_empty(),
                    "仍有主機使用此 SSH 金鑰：{}；請先改用其他認證方式",
                    users.join("、")
                );
                let before = db.keys.len();
                db.keys.retain(|k| k.id != id);
                ensure!(db.keys.len() < before, "找不到 SSH 金鑰");
                Ok(())
            })?;
            Ok(json!({"deleted": id}))
        }
        "group_add" => {
            let group = HostGroup::new(request["name"].as_str().unwrap_or_default())?;
            store.update(|db| {
                db.groups.push(group.clone());
                Ok(())
            })?;
            Ok(json!(group))
        }
        "group_rename" => {
            let name = HostGroup::check_name(request["name"].as_str().unwrap_or_default())?;
            store.update(|db| {
                db.groups
                    .iter_mut()
                    .find(|g| g.id == id)
                    .context("找不到分類")?
                    .name = name;
                Ok(())
            })?;
            Ok(json!({"renamed": id}))
        }
        "group_collapse" => {
            let collapsed = request["collapsed"].as_bool().context("缺少收合狀態")?;
            store.update(|db| {
                db.groups
                    .iter_mut()
                    .find(|g| g.id == id)
                    .context("找不到分類")?
                    .collapsed = collapsed;
                Ok(())
            })?;
            Ok(json!({"collapsed": collapsed}))
        }
        "group_delete" => {
            // The group's hosts stay where they are, ungrouped.
            store.update(|db| {
                let before = db.groups.len();
                db.groups.retain(|g| g.id != id);
                ensure!(db.groups.len() < before, "找不到分類");
                for host in db.hosts.iter_mut().filter(|h| h.group_id == id) {
                    host.group_id.clear();
                }
                Ok(())
            })?;
            Ok(json!({"deleted": id}))
        }
        "arrange" => {
            store.update(|db| arrange(db, &request))?;
            Ok(json!({"arranged": true}))
        }
        "probe" => Ok(json!({"fingerprint": ssh::probe(&store.host(id)?).await?})),
        "trust" => {
            let fingerprint = request["fingerprint"].as_str().context("缺少主機指紋")?;
            ensure!(
                fingerprint.starts_with("SHA256:") && fingerprint.len() > 20,
                "無效的主機指紋"
            );
            store.update(|db| {
                db.hosts
                    .iter_mut()
                    .find(|h| h.id == id)
                    .context("找不到主機")?
                    .fingerprint = fingerprint.into();
                Ok(())
            })?;
            Ok(json!({"trusted": true}))
        }
        "test" => {
            let test = ssh::test_connection(&store.host(id)?).await;
            let succeeded = test.status == "success";
            let manager = test.package_managers.first().cloned().unwrap_or_default();
            store.update(|db| {
                if let Some(host) = db.hosts.iter_mut().find(|h| h.id == id) {
                    if succeeded {
                        host.distro = test.distro.clone();
                        host.package_manager = manager.clone();
                    }
                    host.last_test = Some(test.clone());
                }
                Ok(())
            })?;
            ensure!(succeeded, "{}", test.error);
            Ok(json!({
                "os": test.kernel,
                "distro": test.distro,
                "package_manager": manager,
                "test": test,
            }))
        }
        "check" | "upgrade" | "dist_upgrade" => {
            let host = store.host(id)?;
            let mode = package_manager::Mode::from_id(action).context("不支援的操作")?;
            let result = package_manager::run(&host, mode, progress).await;
            save_results(store, vec![(id.to_owned(), result.clone())])?;
            Ok(json!(result))
        }
        "batch" => batch::run(&request, store, progress, observer).await,
        "reboot" => {
            let host = store.host(id)?;
            let output = reboot::reboot(&host, progress).await?;
            Ok(json!({"output": output}))
        }
        _ => anyhow::bail!("不支援的操作"),
    }
}
