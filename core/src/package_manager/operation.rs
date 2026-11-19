//! One check, upgrade or dist-upgrade on one host: detect, refresh the index, list, and
//! maybe upgrade.

use super::{Commands, DETECT, Manager, Mode, Platform, parse_platform};
use crate::console::Echo;
use crate::model::{CheckResult, Host, Package, RestartStatus, UpgradePlan};
use crate::ssh::{self, Session};
use crate::transcript::heading;
use anyhow::{Context, Result, ensure};
use chrono::Utc;

const DETECT_SECONDS: u64 = 20;
const UPDATE_SECONDS: u64 = 5 * 60;
const LIST_SECONDS: u64 = 2 * 60;
const STATUS_SECONDS: u64 = 2 * 60;

pub async fn check(host: &Host, progress: &dyn Fn(&str)) -> CheckResult {
    run(host, Mode::Check, progress, Echo::silent()).await
}

pub async fn upgrade(host: &Host, progress: &dyn Fn(&str)) -> CheckResult {
    run(host, Mode::Upgrade, progress, Echo::silent()).await
}

pub async fn dist_upgrade(host: &Host, progress: &dyn Fn(&str)) -> CheckResult {
    run(host, Mode::DistUpgrade, progress, Echo::silent()).await
}

/// What a finished operation found; the remote transcript is kept even when it fails.
#[derive(Default)]
struct Outcome {
    pending: Vec<Package>,
    upgraded: Vec<Package>,
    phased: Vec<String>,
    kept_back: Vec<String>,
    plan: Option<UpgradePlan>,
    restart: Option<RestartStatus>,
}

/// One host's session state as the operation goes.
struct Run<'a> {
    host: &'a Host,
    session: &'a Session,
    transcript: String,
    platform: Option<Platform>,
    echo: Echo,
}

// Generic so a batch task's no-op progress keeps the future `Send`.
pub(crate) async fn run<P: Fn(&str) + ?Sized>(
    host: &Host,
    mode: Mode,
    progress: &P,
    echo: Echo,
) -> CheckResult {
    let checked_at = Utc::now().to_rfc3339();
    let mut transcript = String::new();
    let mut platform = None;
    let result = async {
        progress("正在連線並驗證 SSH 主機指紋…");
        let session = ssh::authenticate(host, &Default::default()).await?;
        let mut run = Run {
            host,
            session: &session,
            transcript: String::new(),
            platform: None,
            echo,
        };
        let outcome = run.operate(mode, progress).await;
        (transcript, platform) = (run.transcript, run.platform);
        ssh::disconnect(&session).await;
        outcome
    }
    .await;
    record(host, checked_at, mode, &transcript, platform, result)
}

impl Run<'_> {
    async fn operate<P: Fn(&str) + ?Sized>(&mut self, mode: Mode, progress: &P) -> Result<Outcome> {
        let manager = self.detect().await?;
        if manager == Manager::Apt {
            progress("正在確認 APT 沒有被其他程序佔用…");
            self.check_lock().await?;
        }
        let commands = manager.commands(self.host.username == "root");
        let install = match mode {
            Mode::Check => None,
            Mode::Upgrade => Some(commands.upgrade),
            Mode::DistUpgrade => Some(commands.dist_upgrade.with_context(|| {
                format!("{} 沒有 dist-upgrade；只有 APT 支援", manager.label())
            })?),
        };
        progress("正在更新套件索引…");
        self.refresh(manager, commands.update).await?;
        progress("正在讀取待更新套件…");
        let pending = self.pending(manager, commands.list).await?;
        let mut outcome = match install {
            Some(command) if !pending.is_empty() => {
                progress("正在升級可升級套件…");
                let output = self.install(manager, mode, command).await?;
                progress("正在重新讀取剩餘待更新套件…");
                let remaining = self.pending(manager, commands.list).await?;
                let upgraded = pending
                    .into_iter()
                    .filter(|package| !remaining.iter().any(|left| left.name == package.name))
                    .collect();
                let phased = manager.held_back(&output);
                let kept_back = manager
                    .kept_back(&output)
                    .into_iter()
                    .filter(|name| !phased.contains(name))
                    .collect();
                Outcome {
                    pending: remaining,
                    upgraded,
                    phased,
                    kept_back,
                    ..Default::default()
                }
            }
            _ => {
                let mut outcome = Outcome {
                    pending,
                    ..Default::default()
                };
                // A check simulates the upgrade to show what a phased rollout defers, so
                // the count is known before anyone upgrades.
                if mode == Mode::Check
                    && let Some(command) = commands.preview
                    && !outcome.pending.is_empty()
                {
                    progress("正在模擬升級，找出分階段延後的套件…");
                    let (phased, kept) = self.preview(manager, command).await;
                    outcome.phased = phased;
                    // A package the upgrade would keep back may only wait on others that
                    // are phased; the simulations tell, as after an upgrade. A check
                    // keeps no kept-back list or plan.
                    if !kept.is_empty() {
                        progress("正在模擬 dist-upgrade，確認被保留的套件…");
                        self.settle_kept(&commands, &mut outcome.phased, kept).await;
                    }
                }
                outcome
            }
        };
        // Kept-back packages need a dist-upgrade; show what it would change, without doing it.
        if !outcome.kept_back.is_empty() {
            progress("正在模擬 dist-upgrade…");
            let kept = std::mem::take(&mut outcome.kept_back);
            (outcome.kept_back, outcome.plan) =
                self.settle_kept(&commands, &mut outcome.phased, kept).await;
        }
        if let Some(command) = commands.status {
            progress("正在讀取重新啟動狀態…");
            outcome.restart = self.restart(command).await;
        }
        Ok(outcome)
    }

    /// Runs a read-only command whose failure never fails the run.
    async fn optional(&mut self, title: &str, command: &str) -> Option<(u32, String)> {
        let read = if command.contains("sudo -S") {
            self.privileged(command, STATUS_SECONDS).await
        } else {
            ssh::execute(self.session, command, false, "", STATUS_SECONDS, &self.echo).await
        };
        self.transcript.push_str(&heading(title));
        match read {
            Ok((code, output)) => {
                self.transcript.push_str(&output);
                Some((code, output))
            }
            Err(error) => {
                self.transcript.push_str(&format!("{error:#}\n"));
                None
            }
        }
    }

    /// Reads what needs a restart; a failure leaves it unknown.
    async fn restart(&mut self, command: &str) -> Option<RestartStatus> {
        let (code, output) = self.optional("重新啟動狀態", command).await?;
        super::apt::parse_restart(code, &output)
    }

    /// What a phased rollout defers, read from a simulated upgrade; a failure finds none.
    /// Also returns what the upgrade would keep back, besides what is phased.
    async fn preview(&mut self, manager: Manager, command: &str) -> (Vec<String>, Vec<String>) {
        match self
            .optional("APT upgrade 模擬（不會變更系統）", command)
            .await
        {
            Some((0, output)) => {
                let phased = manager.held_back(&output);
                let kept = (manager.kept_back(&output).into_iter())
                    .filter(|name| !phased.contains(name))
                    .collect();
                (phased, kept)
            }
            _ => (vec![], vec![]),
        }
    }

    /// Sorts what an upgrade, or a check's simulated upgrade, kept back. `simulate` shows
    /// what a dist-upgrade would change. A package it would not upgrade either may only
    /// wait on phased ones; `simulate_phased` tells, and those join `phased`. Returns what
    /// is still kept back and, when anything is, the dist-upgrade plan. Nothing changes.
    async fn settle_kept(
        &mut self,
        commands: &Commands,
        phased: &mut Vec<String>,
        kept: Vec<String>,
    ) -> (Vec<String>, Option<UpgradePlan>) {
        let Some(command) = commands.simulate else {
            return (kept, None);
        };
        let plain = self.simulation(super::apt::SIMULATION_TITLE, command).await;
        let plan = plain.as_deref().and_then(super::apt::parse_plan);
        phased.extend(
            plain
                .as_deref()
                .map(super::apt::parse_phased)
                .unwrap_or_default(),
        );
        let taken = |name: &String| {
            plan.as_ref()
                .is_some_and(|p| p.upgrade.iter().chain(&p.install).any(|q| q.name == *name))
        };
        if let Some(command) = commands.simulate_phased
            && kept
                .iter()
                .any(|name| !taken(name) && !phased.contains(name))
            && let Some(with_phased) = self
                .simulation(super::apt::SIMULATION_PHASED_TITLE, command)
                .await
        {
            phased.extend(super::apt::phasing_freed(
                plain.as_deref().unwrap_or_default(),
                &with_phased,
            ));
        }
        phased.sort();
        phased.dedup();
        let kept: Vec<String> = kept.into_iter().filter(|n| !phased.contains(n)).collect();
        let plan = plan.filter(|_| !kept.is_empty());
        (kept, plan)
    }

    /// A read-only simulation's output, saved under `title`; `None` when it failed.
    async fn simulation(&mut self, title: &str, command: &str) -> Option<String> {
        match self.optional(title, command).await {
            Some((0, output)) => Some(output),
            _ => None,
        }
    }

    /// Refuses to start while apt or dpkg runs, or packages are half-configured: apt would
    /// wait on the lock, or fail, with nothing telling the user why.
    async fn check_lock(&mut self) -> Result<()> {
        let (code, output) = ssh::execute(
            self.session,
            crate::apt_lock::SNAPSHOT,
            false,
            "",
            STATUS_SECONDS,
            &self.echo,
        )
        .await?;
        self.transcript.push_str(&heading("APT 使用狀態"));
        self.transcript.push_str(&output);
        ensure!(code == 0, "無法讀取主機的程序清單（退出碼 {code}）");
        let diagnosis = crate::apt_lock::diagnose(&output)?;
        if let Some(message) = crate::apt_lock::busy_message(&diagnosis) {
            anyhow::bail!(message);
        }
        Ok(())
    }

    /// Runs DETECT and settles which manager may run; never a different one than confirmed.
    async fn detect(&mut self) -> Result<Manager> {
        let (code, output) =
            ssh::execute(self.session, DETECT, false, "", DETECT_SECONDS, &self.echo).await?;
        ensure!(code == 0, "無法辨識遠端作業系統");
        let platform = parse_platform(&output)?;
        let manager = choose(self.host, &platform);
        self.platform = Some(platform);
        manager
    }

    async fn refresh(&mut self, manager: Manager, command: &str) -> Result<()> {
        let (code, output) = self.privileged(command, UPDATE_SECONDS).await?;
        self.transcript.push_str(&heading("更新套件索引"));
        self.transcript.push_str(&output);
        let failed = output
            .lines()
            .any(|line| manager.update_failed(line.trim_start()));
        if code != 0 || failed {
            let needs = manager
                .update_needs()
                .map(|needs| format!("；{needs}"))
                .unwrap_or_default();
            anyhow::bail!("更新套件索引失敗（退出碼 {code}）；未把舊快取當作最新結果{needs}");
        }
        Ok(())
    }

    async fn pending(&mut self, manager: Manager, command: &str) -> Result<Vec<Package>> {
        let (code, output) = if command.contains("sudo -S") {
            self.privileged(command, LIST_SECONDS).await?
        } else {
            ssh::execute(self.session, command, false, "", LIST_SECONDS, &self.echo).await?
        };
        self.transcript.push_str(&heading("待更新套件"));
        self.transcript.push_str(&output);
        ensure!(manager.listed(code), "無法讀取待更新套件（退出碼 {code}）");
        manager.parse_pending(&output)
    }

    /// Upgrades and returns the manager's output for what it held or kept back.
    async fn install(&mut self, manager: Manager, mode: Mode, command: &str) -> Result<String> {
        // No time limit: cutting the connection would leave apt running with nobody reading
        // its output. A quiet upgrade gets warnings instead, and the user may stop it.
        let (code, output) = self.privileged(command, 0).await?;
        self.transcript.push_str(&heading(&format!(
            "{} {}",
            manager.label(),
            mode.command_name()
        )));
        self.transcript.push_str(&output);
        validate_upgrade(manager, code, &output)?;
        Ok(output)
    }

    /// A command that may hit a sudo prompt: uses the host's PTY setting and sudo password.
    /// root's commands have no sudo, so no password goes with them: output that merely
    /// prints the prompt marker (a package script, say) gets nothing.
    async fn privileged(&self, command: &str, seconds: u64) -> Result<(u32, String)> {
        ssh::execute(
            self.session,
            command,
            self.host.use_pty,
            if self.host.username == "root" {
                ""
            } else {
                self.host.sudo_secret()
            },
            seconds,
            &self.echo,
        )
        .await
    }
}

/// The user's choice must exist on the host; otherwise detection must match the record,
/// because the confirmation showed the recorded manager's commands.
fn choose(host: &Host, platform: &Platform) -> Result<Manager> {
    if let Some(chosen) = Manager::from_id(&host.manager_choice) {
        ensure!(
            platform.available.contains(&chosen),
            "主機上找不到所選的套件管理器（{}）；請在編輯主機改回自動偵測或選擇其他項目",
            chosen.label()
        );
        return Ok(chosen);
    }
    let manager = platform
        .manager
        .context("此主機沒有支援的套件管理器（APT、DNF、pacman 或 apk）；不支援此檢查")?;
    let expected = host.manager();
    ensure!(
        expected.is_empty() || expected == manager.id(),
        "偵測到的套件管理器（{}）與紀錄（{expected}）不同；請先按「測試連線」更新主機資訊",
        manager.label()
    );
    Ok(manager)
}

/// Any upgrade error line fails the run even with exit 0; the remote may be half-upgraded.
fn validate_upgrade(manager: Manager, code: u32, output: &str) -> Result<()> {
    ensure!(
        code == 0
            && !output
                .lines()
                .any(|line| manager.upgrade_failed(line.trim_start())),
        "套件升級失敗（退出碼 {code}）；請查看執行輸出，遠端可能已部分升級"
    );
    Ok(())
}

fn record(
    host: &Host,
    checked_at: String,
    mode: Mode,
    transcript: &str,
    platform: Option<Platform>,
    result: Result<Outcome>,
) -> CheckResult {
    let mut check = CheckResult {
        checked_at,
        operation: mode.id().into(),
        output: host.redact(transcript),
        ..Default::default()
    };
    if let Some(platform) = platform {
        check.distro = platform.distro;
        check.package_manager = platform.manager.map(Manager::id).unwrap_or_default().into();
        if platform.available.is_empty() {
            check.status = "unsupported".into();
        }
    }
    match result {
        Ok(outcome) => {
            check.status = "success".into();
            check.packages = outcome.pending;
            check.upgraded = outcome.upgraded;
            check.phased = outcome.phased;
            check.kept_back = outcome.kept_back;
            check.plan = outcome.plan;
            check.restart = outcome.restart;
        }
        Err(error) => {
            if check.status.is_empty() {
                check.status = "failed".into();
            }
            check.error = host.redact(&format!("{error:#}"));
        }
    }
    check
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform(available: Vec<Manager>) -> Platform {
        Platform {
            kernel: "Linux".into(),
            distro: "Test".into(),
            manager: available.first().copied(),
            available,
        }
    }

    #[test]
    fn upgrade_errors_are_failures_even_with_zero_exit() {
        let apt = Manager::Apt;
        assert!(
            validate_upgrade(
                apt,
                0,
                "E: Sub-process /usr/bin/dpkg returned an error code (1)"
            )
            .is_err()
        );
        assert!(validate_upgrade(apt, 100, "Setting up libc6").is_err());
        assert!(validate_upgrade(apt, 0, "Setting up libc6 (2.36-9) ...").is_ok());
        // Each manager's own error form; another's is only text.
        assert!(validate_upgrade(Manager::Dnf, 0, "Error: Transaction failed").is_err());
        assert!(
            validate_upgrade(
                Manager::Pacman,
                0,
                "error: command failed to execute correctly"
            )
            .is_err()
        );
        assert!(
            validate_upgrade(
                Manager::Apk,
                0,
                "ERROR: misf-fail-2.0-r0.post-upgrade: script exited with error 1"
            )
            .is_err()
        );
        assert!(validate_upgrade(Manager::Apk, 0, "E: printed by a script").is_ok());
    }

    #[test]
    fn only_the_chosen_or_recorded_manager_may_run() {
        let mut host = Host::default();
        let both = platform(vec![Manager::Apt, Manager::Apk]);
        assert_eq!(choose(&host, &both).unwrap(), Manager::Apt);
        host.manager_choice = "apk".into();
        assert_eq!(choose(&host, &both).unwrap(), Manager::Apk);
        host.manager_choice = "dnf".into();
        assert!(choose(&host, &both).is_err());
        host.manager_choice.clear();
        host.package_manager = "dnf".into();
        assert!(choose(&host, &both).is_err());
        assert!(choose(&Host::default(), &platform(vec![])).is_err());
    }
}
