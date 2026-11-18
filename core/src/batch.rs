//! Batch check or upgrade: each host is saved and reported the moment it finishes, and
//! a failed host pauses new starts until the caller decides to continue or stop. A stop
//! request ends the batch at once, cutting off the hosts still running.

use crate::cancel::Cancel;
use crate::console::{Console, Echo};
use crate::model::{CheckResult, Host};
use crate::package_manager::{self, Mode};
use crate::store::Store;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Hosts contacted at once by a batch when the request names no number; the rest wait
/// for a free slot.
pub(crate) const PARALLEL: usize = 8;
/// The most hosts a request may ask to contact at once.
pub(crate) const PARALLEL_MAX: usize = 32;
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

/// How many hosts the request contacts at once: `parallel`, or [`PARALLEL`] without it.
fn parallel(request: &Value) -> Result<usize> {
    let Some(value) = request.get("parallel").filter(|value| !value.is_null()) else {
        return Ok(PARALLEL);
    };
    value
        .as_u64()
        .and_then(|count| usize::try_from(count).ok())
        .filter(|count| (1..=PARALLEL_MAX).contains(count))
        .with_context(|| format!("同時處理的主機數必須是 1 到 {PARALLEL_MAX}"))
}

pub(crate) async fn run(
    request: &Value,
    store: &Store,
    progress: &dyn Fn(&str),
    observer: &dyn Observer,
    console: &Arc<dyn Console>,
    cancel: &Cancel,
) -> Result<Value> {
    let Some(mode) = request["operation"].as_str().and_then(Mode::from_id) else {
        anyhow::bail!("不支援的批次操作");
    };
    let parallel = parallel(request)?;
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
    let order: Vec<String> = hosts.iter().map(|host| host.id.clone()).collect();
    progress(&format!("批次作業進度 0/{total}"));
    let mut waiting: VecDeque<Host> = hosts.into();
    let mut tasks = tokio::task::JoinSet::new();
    let mut running: HashMap<tokio::task::Id, String> = HashMap::new();
    let mut results: Vec<(String, CheckResult)> = Vec::with_capacity(total);
    let mut failures: VecDeque<(String, CheckResult)> = VecDeque::new();
    let mut asking: Option<Asking> = None;
    let mut aborted = false;
    let mut cancelled = false;
    let mut save_error = None;
    loop {
        if asking.is_none()
            && !aborted
            && let Some((id, result)) = failures.pop_front()
        {
            asking = Some(observer.failed(&id, &result));
        }
        // A pending question holds back new hosts, never the ones already running.
        while asking.is_none() && !aborted && tasks.len() < parallel {
            let Some(host) = waiting.pop_front() else {
                break;
            };
            let id = host.id.clone();
            let echo = Echo::new(console.clone(), &host);
            let handle = tasks
                .spawn(async move { package_manager::run(&host, mode, &|_: &str| {}, echo).await });
            running.insert(handle.id(), id);
        }
        if tasks.is_empty() && asking.is_none() {
            break;
        }
        tokio::select! {
            () = cancel.requested() => {
                cancelled = true;
                break;
            }
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
                        (id, crashed(mode))
                    }
                };
                // Saved one by one, so closing the app mid-batch keeps every finished host.
                if !keep(store, observer, &id, &result, &mut save_error) {
                    aborted = true;
                    failures.clear();
                    asking = None;
                }
                if result.status != "success" && !aborted {
                    failures.push_back((id.clone(), result.clone()));
                }
                results.push((id, result));
                progress(&format!("批次作業進度 {}/{total}", results.len()));
            }
        }
    }
    // Stopped: cut off the hosts still running. One that finished meanwhile keeps its
    // result; an upgrade cut off is saved as unknown, a check keeps its last result.
    let mut stopped = Vec::new();
    if cancelled {
        tasks.abort_all();
        while let Some(finished) = tasks.join_next_with_id().await {
            match finished {
                Ok((task, result)) => {
                    let id = running.remove(&task).unwrap_or_default();
                    keep(store, observer, &id, &result, &mut save_error);
                    results.push((id, result));
                }
                Err(error) => {
                    let id = running.remove(&error.id()).unwrap_or_default();
                    if mode != Mode::Check {
                        let result = stopped_upgrade(mode.id());
                        keep(store, observer, &id, &result, &mut save_error);
                        results.push((id.clone(), result));
                    }
                    stopped.push(id);
                }
            }
        }
        stopped.sort_by_key(|id| order.iter().position(|known| known == id));
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
        // Cut off by a stop request while running.
        "cancelled": stopped,
    }))
}

/// Saves a host's result and reports it; a failure to save is kept in `error` and
/// returns false, and the host is not reported.
fn keep(
    store: &Store,
    observer: &dyn Observer,
    id: &str,
    result: &CheckResult,
    error: &mut Option<anyhow::Error>,
) -> bool {
    match crate::save_results(store, vec![(id.to_owned(), result.clone())]) {
        Ok(()) => {
            observer.finished(id, result);
            true
        }
        Err(failure) => {
            error.get_or_insert(failure);
            false
        }
    }
}

/// An upgrade the user stopped while it ran: the remote may be half upgraded.
pub fn stopped_upgrade(operation: &str) -> CheckResult {
    CheckResult {
        checked_at: Utc::now().to_rfc3339(),
        operation: operation.into(),
        status: "failed".into(),
        error: "升級已由使用者中斷，結果未知；遠端可能已部分執行。請登入主機確認，必要時執行 sudo dpkg --configure -a，再重新檢查。".into(),
        ..Default::default()
    }
}

/// A host whose upgrade the worker lost track of: it stopped before reporting the outcome,
/// so the remote host may be half upgraded.
pub fn interrupted(operation: &str) -> CheckResult {
    CheckResult {
        checked_at: Utc::now().to_rfc3339(),
        operation: operation.into(),
        status: "failed".into(),
        error: "背景工作意外結束，升級結果未知；遠端可能已部分執行。請登入主機確認，必要時執行 sudo dpkg --configure -a，再重新檢查。".into(),
        ..Default::default()
    }
}

/// A host whose task panicked; recorded as failed so the batch keeps the rest.
fn crashed(mode: Mode) -> CheckResult {
    CheckResult {
        checked_at: Utc::now().to_rfc3339(),
        operation: mode.id().into(),
        status: "failed".into(),
        error: "批次工作異常結束；遠端可能已部分執行，請查看主機狀態".into(),
        ..Default::default()
    }
}
