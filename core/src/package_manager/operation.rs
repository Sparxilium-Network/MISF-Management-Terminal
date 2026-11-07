//! One check or upgrade on one host: detect, refresh the index, list, and maybe upgrade.

use super::{DETECT, Manager, Platform, parse_platform};
use crate::model::{CheckResult, Host, Package};
use crate::ssh::{self, Session};
use anyhow::{Context, Result, ensure};
use chrono::Utc;

const DETECT_SECONDS: u64 = 20;
const UPDATE_SECONDS: u64 = 5 * 60;
const LIST_SECONDS: u64 = 2 * 60;
const UPGRADE_SECONDS: u64 = 30 * 60;

pub async fn check(host: &Host, progress: &dyn Fn(&str)) -> CheckResult {
    run(host, false, progress).await
}

pub async fn upgrade(host: &Host, progress: &dyn Fn(&str)) -> CheckResult {
    run(host, true, progress).await
}

/// What a finished operation found; the remote transcript is kept even when it fails.
#[derive(Default)]
struct Outcome {
    pending: Vec<Package>,
    upgraded: Vec<Package>,
    phased: Vec<String>,
}

/// One host's session state as the operation goes.
struct Run<'a> {
    host: &'a Host,
    session: &'a Session,
    transcript: String,
    platform: Option<Platform>,
}

// Generic so a batch task's no-op progress keeps the future `Send`.
pub(crate) async fn run<P: Fn(&str) + ?Sized>(
    host: &Host,
    upgrade: bool,
    progress: &P,
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
        };
        let outcome = run.operate(upgrade, progress).await;
        (transcript, platform) = (run.transcript, run.platform);
        ssh::disconnect(&session).await;
        outcome
    }
    .await;
    record(host, checked_at, upgrade, &transcript, platform, result)
}

impl Run<'_> {
    async fn operate<P: Fn(&str) + ?Sized>(
        &mut self,
        upgrade: bool,
        progress: &P,
    ) -> Result<Outcome> {
        let manager = self.detect().await?;
        let commands = manager.commands(self.host.username == "root");
        progress("正在更新套件索引…");
        self.refresh(manager, commands.update).await?;
        progress("正在讀取待更新套件…");
        let pending = self.pending(manager, commands.list).await?;
        if !upgrade || pending.is_empty() {
            return Ok(Outcome {
                pending,
                ..Default::default()
            });
        }
        progress("正在升級可升級套件…");
        let phased = self.install(manager, commands.upgrade).await?;
        progress("正在重新讀取剩餘待更新套件…");
        let remaining = self.pending(manager, commands.list).await?;
        let upgraded = pending
            .into_iter()
            .filter(|package| !remaining.iter().any(|left| left.name == package.name))
            .collect();
        Ok(Outcome {
            pending: remaining,
            upgraded,
            phased,
        })
    }

    /// Runs DETECT and settles which manager may run; never a different one than confirmed.
    async fn detect(&mut self) -> Result<Manager> {
        let (code, output) = ssh::execute(self.session, DETECT, false, "", DETECT_SECONDS).await?;
        ensure!(code == 0, "無法辨識遠端作業系統");
        let platform = parse_platform(&output)?;
        let manager = choose(self.host, &platform);
        self.platform = Some(platform);
        manager
    }

    async fn refresh(&mut self, manager: Manager, command: &str) -> Result<()> {
        let (code, output) = self.privileged(command, UPDATE_SECONDS).await?;
        self.transcript.push_str(&output);
        let failed = output
            .lines()
            .any(|line| manager.update_failed(line.trim_start()));
        ensure!(
            code == 0 && !failed,
            "更新套件索引失敗（退出碼 {code}）；未把舊快取當作最新結果"
        );
        Ok(())
    }

    async fn pending(&mut self, manager: Manager, command: &str) -> Result<Vec<Package>> {
        let (code, output) = if command.contains("sudo -S") {
            self.privileged(command, LIST_SECONDS).await?
        } else {
            ssh::execute(self.session, command, false, "", LIST_SECONDS).await?
        };
        self.transcript.push_str("\n--- 待更新套件 ---\n");
        self.transcript.push_str(&output);
        ensure!(manager.listed(code), "無法讀取待更新套件（退出碼 {code}）");
        manager.parse_pending(&output)
    }

    /// Upgrades and returns what the manager held back on purpose.
    async fn install(&mut self, manager: Manager, command: &str) -> Result<Vec<String>> {
        let (code, output) = self.privileged(command, UPGRADE_SECONDS).await?;
        self.transcript
            .push_str(&format!("\n--- {} upgrade ---\n", manager.label()));
        self.transcript.push_str(&output);
        validate_upgrade(code, &output)?;
        Ok(manager.held_back(&output))
    }

    /// A command that may hit a sudo prompt: uses the host's PTY setting and sudo password.
    async fn privileged(&self, command: &str, seconds: u64) -> Result<(u32, String)> {
        ssh::execute(
            self.session,
            command,
            self.host.use_pty,
            self.host.sudo_secret(),
            seconds,
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
        .context("此主機沒有支援的套件管理器（APT、DNF、YUM 或 FreeBSD pkg）；不支援此檢查")?;
    let expected = host.manager();
    ensure!(
        expected.is_empty() || expected == manager.id(),
        "偵測到的套件管理器（{}）與紀錄（{expected}）不同；請先按「測試連線」更新主機資訊",
        manager.label()
    );
    Ok(manager)
}

/// Any upgrade error line fails the run even with exit 0; the remote may be half-upgraded.
fn validate_upgrade(code: u32, output: &str) -> Result<()> {
    ensure!(
        code == 0
            && !output.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("E:") || line.starts_with("Error:")
            }),
        "套件升級失敗（退出碼 {code}）；請查看執行輸出，遠端可能已部分升級"
    );
    Ok(())
}

fn record(
    host: &Host,
    checked_at: String,
    upgrade: bool,
    transcript: &str,
    platform: Option<Platform>,
    result: Result<Outcome>,
) -> CheckResult {
    let mut check = CheckResult {
        checked_at,
        operation: if upgrade { "upgrade" } else { "check" }.into(),
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
        assert!(
            validate_upgrade(0, "E: Sub-process /usr/bin/dpkg returned an error code (1)").is_err()
        );
        assert!(validate_upgrade(100, "Setting up libc6").is_err());
        assert!(validate_upgrade(0, "Setting up libc6 (2.36-9) ...").is_ok());
    }

    #[test]
    fn only_the_chosen_or_recorded_manager_may_run() {
        let mut host = Host::default();
        let both = platform(vec![Manager::Dnf, Manager::Yum]);
        assert_eq!(choose(&host, &both).unwrap(), Manager::Dnf);
        host.manager_choice = "yum".into();
        assert_eq!(choose(&host, &both).unwrap(), Manager::Yum);
        host.manager_choice = "apt".into();
        assert!(choose(&host, &both).is_err());
        host.manager_choice.clear();
        host.package_manager = "apt".into();
        assert!(choose(&host, &both).is_err());
        assert!(choose(&Host::default(), &platform(vec![])).is_err());
    }
}
