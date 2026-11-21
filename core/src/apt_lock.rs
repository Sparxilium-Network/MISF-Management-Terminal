//! Who holds APT on a host, why it may be stuck, and the few fixes MISF offers.
//!
//! A diagnosis only reads: one unprivileged command lists processes, systemd jobs and
//! `dpkg --audit`. A fix runs only after the user confirms it in the UI, and only when a
//! fresh diagnosis on the host still finds exactly that fix, so a stale PID or job number
//! is never acted on.

use crate::console::Echo;
use crate::model::Host;
use crate::package_manager::{SUDO_PROMPT, apt};
use crate::ssh;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

/// Lists processes, systemd jobs and half-configured packages; changes nothing and needs
/// no root. `ps -w -w` keeps whole command lines.
pub const SNAPSHOT: &str = "sh -c 'echo __MISF_PS__; LC_ALL=C ps -e -w -w -o pid=,ppid=,etimes=,stat=,args=; echo __MISF_JOBS__; systemctl list-jobs --no-legend --no-pager 2>/dev/null; echo __MISF_AUDIT__; LC_ALL=C dpkg --audit 2>/dev/null; true'";

const PS: &str = "__MISF_PS__";
const JOBS: &str = "__MISF_JOBS__";
const AUDIT: &str = "__MISF_AUDIT__";

/// How long a snapshot may take.
const SNAPSHOT_SECONDS: u64 = 30;
/// How long cancelling a job or ending processes may take; configuring has no limit.
const FIX_SECONDS: u64 = 60;
/// Longest command line kept for display.
const ARGS_SHOWN: usize = 300;

#[derive(Clone, Debug, Serialize)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    /// Seconds since it started.
    pub elapsed: u64,
    pub state: String,
    pub args: String,
}

impl Process {
    /// The program's file name: `apt-get`, `dpkg`, `sudo`…
    fn program(&self) -> &str {
        let first = self.args.split_whitespace().next().unwrap_or_default();
        first.rsplit('/').next().unwrap_or(first)
    }

    /// In uninterruptible sleep: not even SIGKILL ends it; usually only a reboot does.
    fn stuck(&self) -> bool {
        self.state.starts_with('D')
    }

    fn shown(&self) -> Process {
        let mut copy = self.clone();
        if copy.args.chars().count() > ARGS_SHOWN {
            copy.args = copy.args.chars().take(ARGS_SHOWN).collect::<String>() + "…";
        }
        copy
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: u64,
    pub unit: String,
    pub kind: String,
    pub state: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HolderKind {
    /// Left by an earlier MISF run whose connection is gone: nobody reads its output.
    MisfOrphan,
    /// A MISF run still connected: another window, or this one's batch.
    MisfRunning,
    /// unattended-upgrades or apt.systemd.daily.
    Automatic,
    Other,
}

/// One apt/dpkg process tree.
#[derive(Clone, Debug, Serialize)]
pub struct Holder {
    pub kind: HolderKind,
    /// The tree's first apt/dpkg process.
    pub root: Process,
    /// Its descendants, in PID order.
    pub tree: Vec<Process>,
    /// dpkg is installing or configuring packages in this tree.
    pub dpkg_running: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fix {
    /// A package script waits on this systemd job, which does not finish.
    CancelJob { job: u64, unit: String },
    /// A MISF leftover whose dpkg is done, stuck writing output nobody reads.
    EndLeftover { pids: Vec<u32> },
    /// Nothing runs, but packages were left half-configured.
    Configure,
    /// A process is in uninterruptible sleep; only a reboot clears it.
    Reboot,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Diagnosis {
    pub holders: Vec<Holder>,
    /// systemd jobs a holder waits on.
    pub jobs: Vec<Job>,
    /// Processes in uninterruptible sleep that hold the upgrade up.
    pub stuck: Vec<Process>,
    /// `dpkg --audit`'s report; empty when every package is configured.
    pub audit: String,
    pub fixes: Vec<Fix>,
}

impl Diagnosis {
    /// Whether an upgrade or check should not start now.
    pub fn busy(&self) -> bool {
        !self.holders.is_empty() || !self.audit.is_empty()
    }
}

/// Splits `text` at the first line that is exactly `marker`. The snapshot's own `sh -c`
/// command line also contains every marker and shows up in the `ps` list, so a marker
/// counts only on a line of its own.
fn split_at_line<'a>(text: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    text.match_indices(marker).find_map(|(at, _)| {
        let before = &text[..at];
        let after = &text[at + marker.len()..];
        let line_start = before.is_empty() || before.ends_with('\n');
        let rest = after
            .strip_prefix("\r\n")
            .or_else(|| after.strip_prefix('\n'))
            .or_else(|| after.is_empty().then_some(after))?;
        line_start.then_some((before, rest))
    })
}

/// Splits [`SNAPSHOT`]'s output into its three parts.
fn sections(output: &str) -> Option<(&str, &str, &str)> {
    let (_, rest) = split_at_line(output, PS)?;
    let (ps, rest) = split_at_line(rest, JOBS)?;
    let (jobs, audit) = split_at_line(rest, AUDIT)?;
    Some((ps, jobs, audit))
}

fn parse_processes(text: &str) -> Vec<Process> {
    text.lines()
        .filter_map(|line| {
            let mut rest = line.trim_start();
            let mut field = || {
                let end = rest.find(char::is_whitespace)?;
                let value = &rest[..end];
                rest = rest[end..].trim_start();
                Some(value)
            };
            let pid = field()?.parse().ok()?;
            let ppid = field()?.parse().ok()?;
            let elapsed = field()?.parse().ok()?;
            let state = field()?.to_owned();
            let args = rest.trim_end().to_owned();
            (!args.is_empty()).then_some(Process {
                pid,
                ppid,
                elapsed,
                state,
                args,
            })
        })
        .collect()
}

fn parse_jobs(text: &str) -> Vec<Job> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some(Job {
                id: fields.next()?.parse().ok()?,
                unit: fields.next()?.to_owned(),
                kind: fields.next()?.to_owned(),
                state: fields.next()?.to_owned(),
            })
        })
        .collect()
}

/// An apt or dpkg process, or one of the scripts that run them on a schedule.
fn package_tool(process: &Process) -> bool {
    matches!(
        process.program(),
        "apt-get" | "apt" | "aptitude" | "dpkg" | "unattended-upgrade" | "unattended-upgrades"
    ) || process.args.contains("/usr/bin/unattended-upgrade")
        || process.args.contains("apt.systemd.daily")
}

fn automatic(process: &Process) -> bool {
    process.args.contains("unattended-upgrade") || process.args.contains("apt.systemd.daily")
}

/// The APT commands MISF runs, as they appear once the shell and `env` have run them.
fn misf_command(args: &str) -> bool {
    [apt::UPDATE, apt::UPGRADE, apt::DIST_UPGRADE]
        .iter()
        .filter_map(|command| command.find("apt-get").map(|at| &command[at..]))
        .any(|command| args == command)
}

/// Reads [`SNAPSHOT`]'s output.
pub fn diagnose(output: &str) -> Result<Diagnosis> {
    let (ps, jobs, audit) = sections(output).context("無法讀取主機的程序清單")?;
    let processes = parse_processes(ps);
    let jobs = parse_jobs(jobs);
    let by_pid: HashMap<u32, &Process> = processes.iter().map(|p| (p.pid, p)).collect();
    let mut children: HashMap<u32, Vec<&Process>> = HashMap::new();
    for process in &processes {
        children.entry(process.ppid).or_default().push(process);
    }
    let descendants = |pid: u32| {
        let mut found = Vec::new();
        let mut queue = vec![pid];
        while let Some(next) = queue.pop() {
            for child in children.get(&next).into_iter().flatten() {
                found.push((*child).clone());
                queue.push(child.pid);
            }
        }
        found.sort_by_key(|p| p.pid);
        found
    };
    let mut holders = Vec::new();
    for root in processes.iter().filter(|p| package_tool(p)) {
        if by_pid
            .get(&root.ppid)
            .is_some_and(|parent| package_tool(parent))
        {
            continue;
        }
        // The sudo and env wrappers MISF ran it through, up to the SSH session. Found by
        // program, not by the sudo prompt marker: `ssh::execute` takes the marker out of
        // everything it reads, the `ps` list included.
        let mut top = root;
        while let Some(parent) = by_pid.get(&top.ppid)
            && (matches!(parent.program(), "sudo" | "sudo-rs" | "env")
                || parent.args.contains(SUDO_PROMPT))
        {
            top = parent;
        }
        let misf = top.args.contains(SUDO_PROMPT) || misf_command(&root.args);
        let tree = descendants(root.pid);
        let dpkg_running = root.program() == "dpkg" || tree.iter().any(|p| p.program() == "dpkg");
        let kind = if misf && top.ppid == 1 {
            HolderKind::MisfOrphan
        } else if misf {
            HolderKind::MisfRunning
        } else if automatic(root) {
            HolderKind::Automatic
        } else {
            HolderKind::Other
        };
        holders.push(Holder {
            kind,
            root: root.clone(),
            tree,
            dpkg_running,
        });
    }
    // A job a holder's `systemctl` waits on, e.g. `systemctl restart gpu-manager.service`.
    let waited: Vec<Job> = jobs
        .into_iter()
        .filter(|job| job.state == "running")
        .filter(|job| {
            let stem = job.unit.trim_end_matches(".service");
            holders.iter().flat_map(|h| &h.tree).any(|p| {
                p.program() == "systemctl"
                    && p.args
                        .split_whitespace()
                        .any(|word| word == job.unit || word == stem)
            })
        })
        .collect();
    let mut stuck: Vec<Process> = processes
        .iter()
        .filter(|p| p.stuck())
        .filter(|p| {
            holders
                .iter()
                .any(|h| h.root.pid == p.pid || h.tree.iter().any(|q| q.pid == p.pid))
                || waited
                    .iter()
                    .any(|job| p.program() == job.unit.trim_end_matches(".service"))
        })
        .map(Process::shown)
        .collect();
    stuck.sort_by_key(|p| p.pid);
    let audit = audit.trim().to_owned();
    let mut fixes: Vec<Fix> = waited
        .iter()
        .map(|job| Fix::CancelJob {
            job: job.id,
            unit: job.unit.clone(),
        })
        .collect();
    for holder in &holders {
        if holder.kind == HolderKind::MisfOrphan && !holder.dpkg_running {
            let pids: BTreeSet<u32> = std::iter::once(holder.root.pid)
                .chain(holder.tree.iter().map(|p| p.pid))
                .collect();
            fixes.push(Fix::EndLeftover {
                pids: pids.into_iter().collect(),
            });
        }
    }
    if holders.is_empty() && !audit.is_empty() {
        fixes.push(Fix::Configure);
    }
    if !stuck.is_empty() {
        fixes.push(Fix::Reboot);
    }
    for holder in &mut holders {
        holder.root = holder.root.shown();
        holder.tree = holder.tree.iter().map(Process::shown).collect();
    }
    Ok(Diagnosis {
        holders,
        jobs: waited,
        stuck,
        audit,
        fixes,
    })
}

/// "1 小時 54 分", "3 分", "40 秒".
pub fn duration(seconds: u64) -> String {
    let (hours, minutes) = (seconds / 3600, seconds % 3600 / 60);
    match (hours, minutes) {
        (0, 0) => format!("{seconds} 秒"),
        (0, m) => format!("{m} 分"),
        (h, m) => format!("{h} 小時 {m} 分"),
    }
}

fn kind_text(kind: HolderKind) -> &'static str {
    match kind {
        HolderKind::MisfOrphan => "MISF 先前的作業留下的程序，連線已中斷",
        HolderKind::MisfRunning => "另一個 MISF 作業正在執行",
        HolderKind::Automatic => "系統自動更新正在執行",
        HolderKind::Other => "其他 apt／dpkg 程序",
    }
}

/// One line per finding, for error messages and the terminal.
pub fn summary(diagnosis: &Diagnosis) -> Vec<String> {
    let mut lines = Vec::new();
    for holder in &diagnosis.holders {
        lines.push(format!(
            "PID {}（{}，已執行 {}{}）：{}",
            holder.root.pid,
            kind_text(holder.kind),
            duration(holder.root.elapsed),
            if holder.dpkg_running {
                "，dpkg 仍在安裝或設定套件"
            } else {
                ""
            },
            holder.root.args
        ));
    }
    for job in &diagnosis.jobs {
        lines.push(format!(
            "套件腳本在等 systemd 工作 {}（{} {}），它一直沒有完成",
            job.id, job.unit, job.kind
        ));
    }
    for process in &diagnosis.stuck {
        lines.push(format!(
            "PID {}（{}）處於 D 狀態（無法中斷），通常只能重新開機解除",
            process.pid,
            process.program()
        ));
    }
    if diagnosis.holders.is_empty() && !diagnosis.audit.is_empty() {
        lines.push("有套件只設定到一半（dpkg --audit 有回報）".into());
    }
    lines
}

/// Why a check or upgrade did not start, or `None` when APT is free.
pub fn busy_message(diagnosis: &Diagnosis) -> Option<String> {
    if !diagnosis.busy() {
        return None;
    }
    let automatic_only = !diagnosis.holders.is_empty()
        && diagnosis
            .holders
            .iter()
            .all(|h| h.kind == HolderKind::Automatic);
    let advice = if automatic_only {
        "請等系統自動更新完成後再試；不建議中斷它。"
    } else {
        "沒有執行任何套件命令。可在主機頁「進階操作 › 診斷套件鎖…」查看詳情與處理方式。"
    };
    Some(format!(
        "APT 目前無法使用：\n{}\n{advice}",
        summary(diagnosis).join("\n")
    ))
}

/// The command a fix runs on this host; `None` for a reboot, which has its own dialog.
pub fn fix_command(host: &Host, fix: &Fix) -> Option<String> {
    let body = match fix {
        Fix::CancelJob { job, .. } => format!("systemctl cancel {job}"),
        Fix::EndLeftover { pids } => format!(
            "kill -TERM {}",
            pids.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Fix::Configure => "env DEBIAN_FRONTEND=noninteractive dpkg --configure -a".into(),
        Fix::Reboot => return None,
    };
    Some(if host.username == "root" {
        format!("LC_ALL=C {body}")
    } else {
        format!("LC_ALL=C sudo -S -p '{SUDO_PROMPT}' -- {body}")
    })
}

/// A diagnosis with each fix's command, for the UI.
pub fn report(host: &Host, diagnosis: &Diagnosis) -> Value {
    let fixes: Vec<Value> = diagnosis
        .fixes
        .iter()
        .map(|fix| {
            let mut value = json!(fix);
            value["command"] = json!(fix_command(host, fix).unwrap_or_default());
            value
        })
        .collect();
    let mut value = json!(diagnosis);
    value["fixes"] = json!(fixes);
    value["summary"] = json!(summary(diagnosis));
    value
}

async fn snapshot(session: &ssh::Session, echo: &Echo) -> Result<Diagnosis> {
    let (code, output) = ssh::execute(session, SNAPSHOT, false, "", SNAPSHOT_SECONDS, echo).await?;
    ensure!(code == 0, "無法讀取主機的程序清單（退出碼 {code}）");
    diagnose(&output)
}

/// Reads the host's APT state.
pub async fn diagnose_host(host: &Host, progress: &dyn Fn(&str), echo: &Echo) -> Result<Value> {
    ensure!(host.manager() == "apt", "套件鎖診斷只支援 APT 主機");
    progress("正在連線並驗證 SSH 主機指紋…");
    let session = ssh::authenticate(host, &Default::default()).await?;
    progress("正在讀取 apt／dpkg 程序與 systemd 工作…");
    let result = snapshot(&session, echo).await;
    ssh::disconnect(&session).await;
    Ok(report(host, &result?))
}

/// The fix the request names, as the UI showed it.
fn requested(request: &Value) -> Result<Fix> {
    Ok(match request["fix"].as_str().unwrap_or_default() {
        "cancel_job" => Fix::CancelJob {
            job: request["job"].as_u64().context("缺少 systemd 工作編號")?,
            unit: request["unit"].as_str().unwrap_or_default().to_owned(),
        },
        "end_leftover" => {
            let mut pids: Vec<u32> = (request["pids"].as_array().into_iter().flatten())
                .map(|pid| {
                    pid.as_u64()
                        .and_then(|pid| u32::try_from(pid).ok())
                        .context("PID 無效")
                })
                .collect::<Result<_>>()?;
            pids.sort_unstable();
            ensure!(!pids.is_empty(), "沒有要結束的程序");
            Fix::EndLeftover { pids }
        }
        "configure" => Fix::Configure,
        _ => anyhow::bail!("不支援的處理方式"),
    })
}

/// Runs one confirmed fix, after a fresh diagnosis still finds it; returns the command's
/// output and the diagnosis after it.
pub async fn fix_host(
    host: &Host,
    request: &Value,
    progress: &dyn Fn(&str),
    echo: &Echo,
) -> Result<Value> {
    ensure!(host.manager() == "apt", "套件鎖診斷只支援 APT 主機");
    let wanted = requested(request)?;
    progress("正在連線並驗證 SSH 主機指紋…");
    let session = ssh::authenticate(host, &Default::default()).await?;
    let result = async {
        progress("正在重新確認主機狀態…");
        let before = snapshot(&session, echo).await?;
        // The job number is what counts; the unit name only labels it.
        let found = before.fixes.iter().find(|fix| match (fix, &wanted) {
            (Fix::CancelJob { job: a, .. }, Fix::CancelJob { job: b, .. }) => a == b,
            (a, b) => *a == b,
        });
        let fix = found
            .cloned()
            .context("主機狀態已改變，這個處理方式已不適用；請重新診斷")?;
        let command = fix_command(host, &fix).context("這個處理方式沒有命令")?;
        progress("正在執行已確認的處理方式…");
        let root = host.username == "root";
        let (code, output) = ssh::execute(
            &session,
            &command,
            !root && host.use_pty,
            if root { "" } else { host.sudo_secret() },
            // Configuring packages may take long; it warns when quiet instead.
            if fix == Fix::Configure {
                0
            } else {
                FIX_SECONDS
            },
            echo,
        )
        .await?;
        let output = host.redact(&output);
        ensure!(code == 0, "處理失敗（退出碼 {code}）：{}", output.trim());
        progress("正在讀取處理後的狀態…");
        let after = snapshot(&session, echo).await?;
        Ok(json!({"output": output, "diagnosis": report(host, &after)}))
    }
    .await;
    ssh::disconnect(&session).await;
    result
}
