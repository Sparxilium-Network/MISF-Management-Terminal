pub mod audit;
pub mod model;
mod passkey;
pub mod runtime;
pub mod session;
pub mod sqlite;
pub mod ssh;
pub mod store;
pub mod transcript;
pub mod vault;
#[cfg(target_os = "windows")]
mod windows_passkey;

use anyhow::{Context, Result, ensure};
use model::Host;
use serde_json::{Value, json};
use std::sync::Arc;
use store::Store;

/// Hosts contacted at once by a batch; the rest wait for a free slot.
const BATCH_PARALLEL: usize = 8;
const BATCH_LIMIT: usize = 256;

/// Resolves a batch's distinct host IDs, in request order.
pub(crate) fn batch_ids(request: &Value) -> Result<Vec<String>> {
    let mut ids: Vec<String> = Vec::new();
    for id in request["ids"].as_array().context("缺少批次主機清單")? {
        let id = id.as_str().context("批次主機 ID 格式錯誤")?;
        if !ids.iter().any(|known| known == id) {
            ids.push(id.to_owned());
        }
    }
    ensure!(!ids.is_empty(), "請至少選擇一台主機");
    ensure!(
        ids.len() <= BATCH_LIMIT,
        "單次批次最多 {BATCH_LIMIT} 台主機"
    );
    Ok(ids)
}

fn batch_hosts(request: &Value, store: &Store) -> Result<Vec<Host>> {
    let database = store.read()?;
    batch_ids(request)?
        .iter()
        .map(|id| {
            database
                .hosts
                .iter()
                .find(|host| host.id == *id)
                .cloned()
                .context("找不到主機；可能已被刪除")
        })
        .collect()
}

fn save_results(store: &Store, results: Vec<(String, model::CheckResult)>) -> Result<()> {
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

pub async fn dispatch(request: Value, store: &Store, progress: &dyn Fn(&str)) -> Result<Value> {
    let action = request["action"].as_str().context("缺少 action")?;
    let id = request["id"].as_str().unwrap_or_default();
    match action {
        "list" => Ok(
            json!({"hosts": store.read()?.hosts, "path": store.path, "encrypted": store.is_encrypted()}),
        ),
        "save" => {
            let mut host: Host =
                serde_json::from_value(request["host"].clone()).context("主機資料格式錯誤")?;
            host.validate()?;
            if host.id.is_empty() {
                host.id = uuid::Uuid::new_v4().to_string();
            }
            store.update(|db| {
                if let Some(old) = db.hosts.iter_mut().find(|h| h.id == host.id) {
                    if old.address != host.address || old.port != host.port {
                        host.fingerprint.clear();
                        host.distro.clear();
                        host.package_manager.clear();
                        host.last_check = None;
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
                    }
                    *old = host.clone();
                } else {
                    host.fingerprint.clear();
                    host.distro.clear();
                    host.package_manager.clear();
                    host.last_check = None;
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
            let platform = ssh::test_connection(&store.host(id)?).await?;
            let manager = platform.manager.map(ssh::Manager::id).unwrap_or_default();
            store.update(|db| {
                if let Some(host) = db.hosts.iter_mut().find(|h| h.id == id) {
                    host.distro = platform.distro.clone();
                    host.package_manager = manager.into();
                }
                Ok(())
            })?;
            Ok(
                json!({"os": platform.kernel, "distro": platform.distro, "package_manager": manager}),
            )
        }
        "check" | "upgrade" => {
            let host = store.host(id)?;
            let result = if action == "upgrade" {
                ssh::upgrade(&host, progress).await
            } else {
                ssh::check(&host, progress).await
            };
            save_results(store, vec![(id.to_owned(), result.clone())])?;
            Ok(json!(result))
        }
        "batch" => {
            let upgrade = match request["operation"].as_str() {
                Some("check") => false,
                Some("upgrade") => true,
                _ => anyhow::bail!("不支援的批次操作"),
            };
            let hosts = batch_hosts(&request, store)?;
            // Mixed managers would mean mixed commands under one confirmation.
            let manager = hosts[0].manager();
            ensure!(
                !manager.is_empty() && hosts.iter().all(|h| h.manager() == manager),
                "批次作業只能包含同一種已偵測套件管理器的主機；請先測試連線偵測各主機"
            );
            let total = hosts.len();
            progress(&format!("批次作業進度 0/{total}"));
            let limit = Arc::new(tokio::sync::Semaphore::new(BATCH_PARALLEL));
            let mut tasks = tokio::task::JoinSet::new();
            for host in hosts {
                let limit = limit.clone();
                tasks.spawn(async move {
                    let _permit = limit.acquire_owned().await;
                    let result = ssh::run_apt(&host, upgrade, &|_: &str| {}).await;
                    (host.id.clone(), result)
                });
            }
            let mut results = Vec::with_capacity(total);
            while let Some(finished) = tasks.join_next().await {
                results.push(finished.context("批次工作異常結束")?);
                progress(&format!("批次作業進度 {}/{total}", results.len()));
            }
            save_results(store, results.clone())?;
            Ok(json!({
                "results": results
                    .into_iter()
                    .map(|(id, result)| json!({"id": id, "result": result}))
                    .collect::<Vec<_>>()
            }))
        }
        _ => anyhow::bail!("不支援的操作"),
    }
}
