//! The package managers MISF drives over SSH.
//!
//! Each manager's module holds its fixed command strings, how to tell a failed index
//! update, and how to read its pending-package listing. Commands are `&'static str`
//! constants: nothing from a host record is ever interpolated into them, and the
//! confirmation dialog shows exactly the strings that run.

pub mod apt;
pub mod detect;
pub mod dnf;
mod operation;
pub mod pkg;
mod rpm;
pub mod yum;

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
    /// Reads what needs a restart afterwards; only APT has one.
    pub status: Option<&'static str>,
}

/// A supported package manager; one batch only ever runs one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Apt,
    Dnf,
    Yum,
    Pkg,
}

impl Manager {
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "apt" => Self::Apt,
            "dnf" => Self::Dnf,
            "yum" => Self::Yum,
            "pkg" => Self::Pkg,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Yum => "yum",
            Self::Pkg => "pkg",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Apt => "APT",
            Self::Dnf => "DNF",
            Self::Yum => "YUM",
            Self::Pkg => "pkg",
        }
    }

    /// The commands for this account: root runs them directly, others through sudo.
    pub fn commands(self, root: bool) -> Commands {
        match (self, root) {
            (Self::Apt, true) => apt::ROOT,
            (Self::Apt, false) => apt::SUDO,
            (Self::Dnf, true) => dnf::ROOT,
            (Self::Dnf, false) => dnf::SUDO,
            (Self::Yum, true) => yum::ROOT,
            (Self::Yum, false) => yum::SUDO,
            (Self::Pkg, true) => pkg::ROOT,
            (Self::Pkg, false) => pkg::SUDO,
        }
    }

    /// Whether an index-update output line reports a failure the exit code may hide.
    fn update_failed(self, line: &str) -> bool {
        match self {
            Self::Apt => apt::update_failed(line),
            Self::Dnf | Self::Yum => rpm::update_failed(line),
            Self::Pkg => pkg::update_failed(line),
        }
    }

    /// Whether the listing command's exit code means it listed successfully.
    fn listed(self, code: u32) -> bool {
        match self {
            Self::Dnf | Self::Yum => rpm::listed(code),
            Self::Apt | Self::Pkg => code == 0,
        }
    }

    fn parse_pending(self, output: &str) -> Result<Vec<Package>> {
        match self {
            Self::Apt => apt::parse_pending(output),
            Self::Dnf | Self::Yum => rpm::parse_pending(output),
            Self::Pkg => pkg::parse_pending(output),
        }
    }

    /// Packages the upgrade deliberately held back; only APT reports any.
    fn held_back(self, output: &str) -> Vec<String> {
        match self {
            Self::Apt => apt::parse_phased(output),
            Self::Dnf | Self::Yum | Self::Pkg => vec![],
        }
    }

    /// Packages the upgrade could not take without installing or removing others.
    fn kept_back(self, output: &str) -> Vec<String> {
        match self {
            Self::Apt => apt::parse_kept_back(output),
            Self::Dnf | Self::Yum | Self::Pkg => vec![],
        }
    }
}

/// The commands an operation with this manager runs, in order.
/// The simulation runs only when packages were kept back, and the restart status last.
pub fn manager_commands(manager: Manager, root: bool, mode: Mode) -> Vec<&'static str> {
    let commands = manager.commands(root);
    let mut list = vec![DETECT, commands.update, commands.list];
    let install = match mode {
        Mode::Check => None,
        Mode::Upgrade => Some(commands.upgrade),
        Mode::DistUpgrade => commands.dist_upgrade,
    };
    if let Some(install) = install {
        list.extend([install, commands.list]);
        list.extend(commands.simulate);
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

    const ALL: [Manager; 4] = [Manager::Apt, Manager::Dnf, Manager::Yum, Manager::Pkg];

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
            for command in root.dist_upgrade.into_iter().chain(root.simulate) {
                assert!(!command.contains("sudo"), "{command}");
            }
            let user = manager.commands(false);
            // The status script and dist-upgrade go through sudo like the upgrade does.
            for command in user.status.into_iter().chain(user.dist_upgrade) {
                assert!(command.contains(&sudo), "{command}");
            }
            // The simulation changes nothing, so it never needs sudo.
            assert_eq!(user.simulate, root.simulate);
            for command in [user.update, user.upgrade] {
                assert!(command.contains(&sudo), "{command}");
            }
            // Listing either needs no privileges or goes through the same sudo form.
            assert!(!user.list.contains("sudo") || user.list.contains(&sudo));
            for command in [user.update, user.list, user.upgrade] {
                assert!(command.contains("LC_ALL=C"), "{command}");
            }
        }
    }
}
