//! What the apk tests share: starting an Alpine host and reading apk's state.

use misf_sshtest::{APK, Account, Container, Lab};
pub use misf_sshtest::{assert_failed, assert_plain, names};

/// Starts an Alpine host offering version 2.0 of `packages`, registered for `account`.
pub async fn start(account: Account, use_pty: bool, packages: &[&str]) -> Lab {
    Lab::start(&APK, account, use_pty, packages).await
}

/// apk as seen from inside the host.
pub trait Apk {
    /// The installed version of a package, e.g. `2.0`; `none` when it is not installed.
    async fn apk(&self, package: &str) -> String;
    /// Waits until no apk process is left in the host.
    async fn wait_until_apk_idle(&self, seconds: u64);
}

const RUNNING: &str = "pgrep -x apk >/dev/null";

impl Apk for Container {
    async fn apk(&self, package: &str) -> String {
        let listed = self
            .sh(&format!("apk list --installed {package} 2>/dev/null"))
            .await;
        listed
            .lines()
            .find_map(|line| {
                let full = line.split_whitespace().next()?;
                let version = full.strip_prefix(&format!("{package}-"))?;
                Some(version.split('-').next()?.to_owned())
            })
            .unwrap_or_else(|| "none".into())
    }

    async fn wait_until_apk_idle(&self, seconds: u64) {
        self.wait_for(&format!("! {RUNNING}"), seconds).await;
    }
}
