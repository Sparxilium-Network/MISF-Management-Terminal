//! Batch check or upgrade: each host is saved and reported the moment it finishes, and
//! a failed host pauses new starts until the caller decides to continue or stop.

use crate::model::{CheckResult, Host};
use crate::package_manager;
use crate::store::Store;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;

/// Hosts contacted at once by a batch; the rest wait for a free slot.
pub(crate) const PARALLEL: usize = 8;
const LIMIT: usize = 256;

/// What to do with the hosts not yet started after one failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Keep starting the remaining hosts.
    Continue,
    /// Start no more hosts; the ones already running still finish.
    Abort,
}

pub type Asking<'a> = Pin<Box<dyn Future<Output = Decision> + 'a>>;

/// Receives a batch's per-host outcomes while it runs.
pub trait Observer {
    /// A host finished and its result is already saved.
    fn finished(&self, _id: &str, _result: &CheckResult) {}

    /// A host did not succeed. No new host starts until the returned future resolves;
    /// hosts already running keep going and are still saved and reported meanwhile.
    fn failed<'a>(&'a self, _id: &str, _result: &CheckResult) -> Asking<'a> {
        Box::pin(async { Decision::Continue })
    }
}

/// Reports nothing and never stops a batch: the CLI and tests.
pub struct Unattended;
impl Observer for Unattended {}

/// Resolves a batch's distinct host IDs, in request order.
pub(crate) fn ids(request: &Value) -> Result<Vec<String>> {
    let mut ids: Vec<String> = Vec::new();
    for id in request["ids"].as_array().context("缺少批次主機清單")? {
        let id = id.as_str().context("批次主機 ID 格式錯誤")?;
        if !ids.iter().any(|known| known == id) {
            ids.push(id.to_owned());
        }
    }
    ensure!(!ids.is_empty(), "請至少選擇一台主機");
    ensure!(ids.len() <= LIMIT, "單次批次最多 {LIMIT} 台主機");
    Ok(ids)
}

pub(crate) async fn run(
    request: &Value,
    store: &Store,
    progress: &dyn Fn(&str),
    observer: &dyn Observer,
) -> Result<Value> {
    let upgrade = match request["operation"].as_str() {
        Some("check") => false,
        Some("upgrade") => true,
        _ => anyhow::bail!("不支援的批次操作"),
    };
    let database = store.read()?;
    let hosts = ids(request)?
        .iter()
        .map(|id| database.host(id))
        .collect::<Result<Vec<Host>>>()?;
    // Mixed managers would mean mixed commands under one confirmation.
    let manager = hosts[0].manager();
    ensure!(
        !manager.is_empty() && hosts.iter().all(|h| h.manager() == manager),
        "批次作業只能包含同一種已偵測套件管理器的主機；請先測試連線偵測各主機"
    );
    let total = hosts.len();
    progress(&format!("批次作業進度 0/{total}"));
    let mut waiting: VecDeque<Host> = hosts.into();
    let mut tasks = tokio::task::JoinSet::new();
    let mut running: HashMap<tokio::task::Id, String> = HashMap::new();
    let mut results: Vec<(String, CheckResult)> = Vec::with_capacity(total);
    let mut failures: VecDeque<(String, CheckResult)> = VecDeque::new();
    let mut asking: Option<Asking> = None;
    let mut aborted = false;
    let mut save_error = None;
    loop {
        if asking.is_none()
            && !aborted
            && let Some((id, result)) = failures.pop_front()
        {
            asking = Some(observer.failed(&id, &result));
        }
        // A pending question holds back new hosts, never the ones already running.
        while asking.is_none() && !aborted && tasks.len() < PARALLEL {
            let Some(host) = waiting.pop_front() else {
                break;
            };
            let id = host.id.clone();
            let handle = tasks
                .spawn(async move { package_manager::run(&host, upgrade, &|_: &str| {}).await });
            running.insert(handle.id(), id);
        }
        if tasks.is_empty() && asking.is_none() {
            break;
        }
        tokio::select! {
            decision = async { asking.as_mut().unwrap().await }, if asking.is_some() => {
                asking = None;
                if decision == Decision::Abort {
                    aborted = true;
                    failures.clear();
                }
            }
            Some(finished) = tasks.join_next_with_id(), if !tasks.is_empty() => {
                let (id, result) = match finished {
                    Ok((task, result)) => (running.remove(&task).unwrap_or_default(), result),
                    Err(error) => {
                        let id = running.remove(&error.id()).unwrap_or_default();
                        (id, crashed(upgrade))
                    }
                };
                // Saved one by one, so closing the app mid-batch keeps every finished host.
                if let Err(error) = crate::save_results(store, vec![(id.clone(), result.clone())]) {
                    aborted = true;
                    failures.clear();
                    asking = None;
                    save_error.get_or_insert(error);
                } else {
                    observer.finished(&id, &result);
                }
                if result.status != "success" && !aborted {
                    failures.push_back((id.clone(), result.clone()));
                }
                results.push((id, result));
                progress(&format!("批次作業進度 {}/{total}", results.len()));
            }
        }
    }
    if let Some(error) = save_error {
        return Err(error.context(
            "無法儲存批次結果，已停止啟動其他主機；已在執行的主機已完成，請重新整理確認狀態",
        ));
    }
    let skipped: Vec<String> = waiting.into_iter().map(|host| host.id).collect();
    Ok(json!({
        "results": results
            .into_iter()
            .map(|(id, result)| json!({"id": id, "result": result}))
            .collect::<Vec<_>>(),
        "skipped": skipped,
    }))
}

/// A host whose task panicked; recorded as failed so the batch keeps the rest.
fn crashed(upgrade: bool) -> CheckResult {
    CheckResult {
        checked_at: Utc::now().to_rfc3339(),
        operation: if upgrade { "upgrade" } else { "check" }.into(),
        status: "failed".into(),
        error: "批次工作異常結束；遠端可能已部分執行，請查看主機狀態".into(),
        ..Default::default()
    }
}
