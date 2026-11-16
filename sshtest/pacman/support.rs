//! What the pacman tests share: starting an Arch host and reading pacman's state.

use misf_sshtest::{Account, Container, Lab, PACMAN};
pub use misf_sshtest::{assert_failed, assert_plain, names};

/// Starts an Arch Linux host offering version 2.0 of `packages`, registered for `account`.
pub async fn start(account: Account, use_pty: bool, packages: &[&str]) -> Lab {
    Lab::start(&PACMAN, account, use_pty, packages).await
}

/// pacman as seen from inside the host.
pub trait Pacman {
    /// The installed version of a package, e.g. `2.0`; `none` when it is not installed.
    async fn pacman(&self, package: &str) -> String;
    /// Whether a pacman process is still running in the host.
    async fn pacman_running(&self) -> bool;
    /// Waits until no pacman process is left in the host.
    async fn wait_until_pacman_idle(&self, seconds: u64);
    /// Whether pacman's database lock file is there.
    async fn locked(&self) -> bool;
}

const RUNNING: &str = "pgrep -x pacman >/dev/null";

impl Pacman for Container {
    async fn pacman(&self, package: &str) -> String {
        let (installed, line) = self.try_sh(&format!("pacman -Q {package}")).await;
        // `name 2.0-1`
        (installed.then(|| line.split_whitespace().nth(1)).flatten())
            .and_then(|version| version.split('-').next())
            .unwrap_or("none")
            .to_owned()
    }

    async fn pacman_running(&self) -> bool {
        self.try_sh(RUNNING).await.0
    }

    async fn wait_until_pacman_idle(&self, seconds: u64) {
        self.wait_for(&format!("! {RUNNING}"), seconds).await;
    }

    async fn locked(&self) -> bool {
        self.try_sh("test -e /var/lib/pacman/db.lck").await.0
    }
}
