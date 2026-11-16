//! What the APT tests share: starting an APT host, reading dpkg's state, common checks.

use misf_core::model::{CheckResult, Package};
use misf_sshtest::{APT, Account, Container, Lab};

/// Starts a Debian/Ubuntu host offering version 2.0 of `packages`, registered for `account`.
pub async fn start(account: Account, use_pty: bool, packages: &[&str]) -> Lab {
    Lab::start(&APT, account, use_pty, packages).await
}

/// dpkg and APT as seen from inside the host.
pub trait Dpkg {
    /// dpkg's view of a package: its state abbreviation and version, e.g. `ii 2.0`
    /// (installed), `iF 2.0` (failed to configure) or `iU 2.0` (unpacked only).
    async fn dpkg(&self, package: &str) -> String;
    /// What `dpkg --audit` reports: empty when every package is fully installed.
    async fn audit(&self) -> String;
    /// Whether an apt or dpkg process is still running in the host.
    async fn apt_running(&self) -> bool;
    /// Waits until no apt or dpkg process is left in the host.
    async fn wait_until_apt_idle(&self, seconds: u64);
}

impl Dpkg for Container {
    async fn dpkg(&self, package: &str) -> String {
        self.sh(&format!(
            "dpkg-query -W -f='${{db:Status-Abbrev}}${{Version}}' {package}"
        ))
        .await
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
    }

    async fn audit(&self) -> String {
        self.sh("dpkg --audit").await.trim().to_owned()
    }

    async fn apt_running(&self) -> bool {
        self.try_sh("pgrep -x 'apt-get|dpkg' >/dev/null").await.0
    }

    async fn wait_until_apt_idle(&self, seconds: u64) {
        self.wait_for("! pgrep -x 'apt-get|dpkg' >/dev/null", seconds)
            .await;
    }
}

pub fn names(packages: &[Package]) -> Vec<&str> {
    packages
        .iter()
        .map(|package| package.name.as_str())
        .collect()
}

/// Saved output and live lines are plain text: no control characters but the newline,
/// and none of the host's secrets.
pub fn assert_plain(lab: &Lab, text: &str) {
    let host = lab.host();
    assert!(
        !text.chars().any(|c| c.is_control() && c != '\n'),
        "control characters left in:\n{text:?}"
    );
    assert!(!text.contains(&host.password), "the password leaked");
    // The live view also shows each command (`$ …`), and sudo commands name the marker.
    assert!(
        !(text.lines())
            .any(|line| !line.starts_with("$ ") && line.contains("__MISF_SUDO_PROMPT__")),
        "the sudo prompt marker was left in the output"
    );
}

/// A failed run names the step and never claims packages.
pub fn assert_failed(result: &CheckResult, error: &str) {
    assert_eq!(result.status, "failed", "{}", result.output);
    assert!(result.error.contains(error), "{}", result.error);
    assert!(result.packages.is_empty() && result.upgraded.is_empty());
    assert!(result.restart.is_none());
}
