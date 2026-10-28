pub mod model;
pub mod ssh;
pub mod store;

use anyhow::{Context, Result, ensure};
use model::Host;
use serde_json::{Value, json};
use store::Store;

pub async fn dispatch(request: Value, store: &Store, progress: &dyn Fn(&str)) -> Result<Value> {
    let action = request["action"].as_str().context("缺少 action")?;
    let id = request["id"].as_str().unwrap_or_default();
    match action {
        "list" => Ok(json!({"hosts": store.read()?.hosts, "path": store.path, "encrypted": false})),
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
                        host.last_check = None;
                    } else {
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
        "test" => Ok(json!({"os": ssh::test_connection(&store.host(id)?).await?})),
        "check" => {
            let host = store.host(id)?;
            let result = ssh::check(&host, progress).await;
            store.update(|db| {
                if let Some(current) = db.hosts.iter_mut().find(|h| h.id == id) {
                    current.last_check = Some(result.clone());
                }
                Ok(())
            })?;
            Ok(json!(result))
        }
        _ => anyhow::bail!("不支援的操作"),
    }
}
