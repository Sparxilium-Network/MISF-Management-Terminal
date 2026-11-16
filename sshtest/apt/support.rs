//! What the APT tests share: starting an APT host, reading dpkg's state, common checks.

use misf_sshtest::{APT, Account, Container, Lab};
pub use misf_sshtest::{assert_failed, assert_plain, names};

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
