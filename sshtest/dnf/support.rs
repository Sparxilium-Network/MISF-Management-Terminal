//! What the DNF tests share: starting a DNF host and reading rpm's state.

use misf_sshtest::{Account, Container, DNF, Lab};
pub use misf_sshtest::{assert_failed, assert_plain, names};

/// Starts a Fedora/Enterprise Linux host offering version 2.0 of `packages`, registered
/// for `account`.
pub async fn start(account: Account, use_pty: bool, packages: &[&str]) -> Lab {
    Lab::start(&DNF, account, use_pty, packages).await
}

/// rpm and DNF as seen from inside the host.
pub trait Rpm {
    /// The installed version of a package, e.g. `2.0`; two versions when a broken
    /// transaction left both, and `none` when it is not installed.
    async fn rpm(&self, package: &str) -> String;
    /// Whether a dnf process is still running in the host.
    async fn dnf_running(&self) -> bool;
    /// Waits until no dnf process is left in the host.
    async fn wait_until_dnf_idle(&self, seconds: u64);
}

/// dnf is a Python script on dnf 4 and a binary on dnf 5.
const RUNNING: &str = "pgrep -f '(^|/)dnf[0-9-]* ' >/dev/null";

impl Rpm for Container {
    async fn rpm(&self, package: &str) -> String {
        let (installed, versions) = self
            .try_sh(&format!("rpm -q --qf '%{{VERSION}} ' {package}"))
            .await;
        if installed {
            versions.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            "none".into()
        }
    }

    async fn dnf_running(&self) -> bool {
        self.try_sh(RUNNING).await.0
    }

    async fn wait_until_dnf_idle(&self, seconds: u64) {
        self.wait_for(&format!("! {RUNNING}"), seconds).await;
    }
}
