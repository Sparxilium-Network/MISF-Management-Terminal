//! The package managers MISF drives over SSH.
//!
//! Each manager's module holds its fixed command strings, how to tell a failed index
//! update, and how to read its pending-package listing. Commands are `&'static str`
//! constants: nothing from a host record is ever interpolated into them, and the
//! confirmation dialog shows exactly the strings that run.

pub mod apk;
pub mod apt;
pub mod detect;
pub mod dnf;
mod operation;
pub mod pacman;
mod rpm;

pub use detect::{DETECT, Platform, parse_platform};
pub(crate) use operation::run;
pub use operation::{check, dist_upgrade, upgrade};

use crate::model::{Host, Package};
use anyhow::Result;

/// The marker every sudo command prints as its password prompt; the SSH layer answers it.
pub const SUDO_PROMPT: &str = "__MISF_SUDO_PROMPT__";

/// What an operation does after listing pending packages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Lists only.
    Check,
    /// Upgrades installed packages without installing or removing any.
    Upgrade,
    /// APT only: may also install new packages and remove installed ones.
    DistUpgrade,
}

impl Mode {
    /// The request and stored operation name.
    pub fn id(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Upgrade => "upgrade",
            Self::DistUpgrade => "dist_upgrade",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Check, Self::Upgrade, Self::DistUpgrade]
            .into_iter()
            .find(|mode| mode.id() == id)
    }

    /// The package manager subcommand, as shown in the transcript.
    fn command_name(self) -> &'static str {
        match self {
            Self::Check => "list",
            Self::Upgrade => "upgrade",
            Self::DistUpgrade => "dist-upgrade",
        }
    }
}

/// The commands one manager runs, for root or through sudo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Commands {
    /// Refreshes the package index.
    pub update: &'static str,
    /// Lists packages with a newer version available.
    pub list: &'static str,
    /// Upgrades installed packages.
    pub upgrade: &'static str,
    /// Upgrades and may install or remove packages; only APT has one.
    pub dist_upgrade: Option<&'static str>,
    /// Read-only simulation of `dist_upgrade`, run when an upgrade kept packages back.
    pub simulate: Option<&'static str>,
    /// `simulate` with phased updates included, run when `simulate` still keeps packages
    /// back, to tell which only wait on a phased rollout; only APT has one.
    pub simulate_phased: Option<&'static str>,
    /// Read-only simulation of `upgrade` run by a check, for what a phased rollout defers;
    /// only APT has one.
    pub preview: Option<&'static str>,
    /// Reads what needs a restart afterwards; only APT has one.
    pub status: Option<&'static str>,
}

/// A supported package manager; one batch only ever runs one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Apt,
    Dnf,
    Pacman,
    Apk,
}

impl Manager {
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "apt" => Self::Apt,
            "dnf" => Self::Dnf,
            "pacman" => Self::Pacman,
            "apk" => Self::Apk,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Pacman => "pacman",
            Self::Apk => "apk",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Apt => "APT",
            Self::Dnf => "DNF",
            Self::Pacman => "pacman",
            Self::Apk => "apk",
        }
    }

    /// The commands for this account: root runs them directly, others through sudo.
    pub fn commands(self, root: bool) -> Commands {
        match (self, root) {
            (Self::Apt, true) => apt::ROOT,
            (Self::Apt, false) => apt::SUDO,
            (Self::Dnf, true) => dnf::ROOT,
            (Self::Dnf, false) => dnf::SUDO,
            (Self::Pacman, true) => pacman::ROOT,
            (Self::Pacman, false) => pacman::SUDO,
            (Self::Apk, true) => apk::ROOT,
            (Self::Apk, false) => apk::SUDO,
        }
    }

    /// Whether an index-update output line reports a failure the exit code may hide.
    fn update_failed(self, line: &str) -> bool {
        match self {
            Self::Apt => apt::update_failed(line),
            Self::Dnf => rpm::update_failed(line),
            Self::Pacman => pacman::update_failed(line),
            Self::Apk => apk::update_failed(line),
        }
    }

    /// Why a failed index refresh failed, when the manager needs something installed.
    fn update_needs(self) -> Option<&'static str> {
        match self {
            Self::Pacman => Some(pacman::NEEDS),
            Self::Apt | Self::Dnf | Self::Apk => None,
        }
    }

    /// Whether an upgrade output line reports an error the exit code may hide.
    fn upgrade_failed(self, line: &str) -> bool {
        match self {
            Self::Apt => line.starts_with("E:"),
            Self::Dnf => line.starts_with("Error:"),
            Self::Pacman => pacman::upgrade_failed(line),
            Self::Apk => apk::upgrade_failed(line),
        }
    }

    /// Whether the listing command's exit code means it listed successfully.
    fn listed(self, code: u32) -> bool {
        match self {
            Self::Dnf => rpm::listed(code),
            Self::Pacman => pacman::listed(code),
            Self::Apt | Self::Apk => code == 0,
        }
    }

    fn parse_pending(self, output: &str) -> Result<Vec<Package>> {
        match self {
            Self::Apt => apt::parse_pending(output),
            Self::Dnf => rpm::parse_pending(output),
            Self::Pacman => pacman::parse_pending(output),
            Self::Apk => apk::parse_pending(output),
        }
    }

    /// Packages the upgrade deliberately held back; only APT reports any.
    fn held_back(self, output: &str) -> Vec<String> {
        match self {
            Self::Apt => apt::parse_phased(output),
            Self::Dnf | Self::Pacman | Self::Apk => vec![],
        }
    }

    /// Packages the upgrade could not take without installing or removing others.
    fn kept_back(self, output: &str) -> Vec<String> {
        match self {
            Self::Apt => apt::parse_kept_back(output),
            Self::Dnf | Self::Pacman | Self::Apk => vec![],
        }
    }
}

/// The commands an operation with this manager runs, in order.
/// The dist-upgrade simulation runs only when packages were kept back (after an upgrade,
/// or in a check's simulated upgrade), and the restart status last.
pub fn manager_commands(manager: Manager, root: bool, mode: Mode) -> Vec<&'static str> {
    let commands = manager.commands(root);
    let mut list = vec![DETECT];
    // APT runs refuse to start while apt or dpkg is busy; see `apt_lock`.
    if manager == Manager::Apt {
        list.push(crate::apt_lock::SNAPSHOT);
    }
    list.extend([commands.update, commands.list]);
    let install = match mode {
        Mode::Check => None,
        Mode::Upgrade => Some(commands.upgrade),
        Mode::DistUpgrade => commands.dist_upgrade,
    };
    if let Some(install) = install {
        list.extend([install, commands.list]);
        list.extend(commands.simulate);
    } else {
        list.extend(commands.preview);
        if commands.preview.is_some() {
            list.extend(commands.simulate);
        }
    }
    list.extend(commands.status);
    list
}

/// The commands an operation runs on this host; only DETECT until its manager is known.
pub fn package_commands(host: &Host, mode: Mode) -> Vec<&'static str> {
    match Manager::from_id(host.manager()) {
        Some(manager) => manager_commands(manager, host.username == "root", mode),
        None => vec![DETECT],
    }
}

#[cfg(test)]
mod tests {
    use super::{Manager, SUDO_PROMPT};

    const ALL: [Manager; 4] = [Manager::Apt, Manager::Dnf, Manager::Pacman, Manager::Apk];

    #[test]
    fn ids_round_trip() {
        for manager in ALL {
            assert_eq!(Manager::from_id(manager.id()), Some(manager));
        }
        assert_eq!(Manager::from_id(""), None);
        assert_eq!(Manager::from_id("APT"), None);
    }

    #[test]
    fn root_never_uses_sudo_and_sudo_always_answers_the_prompt_marker() {
        let sudo = format!("sudo -S -p '{SUDO_PROMPT}' -- ");
        for manager in ALL {
            let root = manager.commands(true);
            for command in [root.update, root.list, root.upgrade]
                .into_iter()
                .chain(root.status)
            {
                assert!(!command.contains("sudo"), "{command}");
            }
            // Restart status, dist-upgrade and its simulation are APT-only.
            let apt = manager == Manager::Apt;
            assert_eq!(root.status.is_some(), apt);
            assert_eq!(root.dist_upgrade.is_some(), apt);
            assert_eq!(root.simulate.is_some(), apt);
            assert_eq!(root.preview.is_some(), apt);
            for command in root
                .dist_upgrade
                .into_iter()
                .chain(root.simulate)
                .chain(root.preview)
            {
                assert!(!command.contains("sudo"), "{command}");
            }
            let user = manager.commands(false);
            // The status script and dist-upgrade go through sudo like the upgrade does.
            for command in user.status.into_iter().chain(user.dist_upgrade) {
                assert!(command.contains(&sudo), "{command}");
            }
            // The simulation changes nothing, so it never needs sudo.
            assert_eq!(user.simulate, root.simulate);
            assert_eq!(user.preview, root.preview);
            assert!(user.upgrade.contains(&sudo), "{}", user.upgrade);
            // The refresh and listing either need no privileges (pacman's checkupdates
            // syncs a private copy) or go through the same sudo form.
            for command in [user.update, user.list] {
                assert!(
                    !command.contains("sudo") || command.contains(&sudo),
                    "{command}"
                );
            }
            assert_eq!(user.update.contains("sudo"), manager != Manager::Pacman);
            for command in [user.update, user.list, user.upgrade] {
                assert!(command.contains("LC_ALL=C"), "{command}");
            }
        }
    }
}
