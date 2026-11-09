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
pub use operation::{check, upgrade};

use crate::model::{Host, Package};
use anyhow::Result;

/// The marker every sudo command prints as its password prompt; the SSH layer answers it.
pub const SUDO_PROMPT: &str = "__MISF_SUDO_PROMPT__";

/// The three commands one manager runs, for root or through sudo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Commands {
    /// Refreshes the package index.
    pub update: &'static str,
    /// Lists packages with a newer version available.
    pub list: &'static str,
    /// Upgrades installed packages.
    pub upgrade: &'static str,
    /// Reads what needs a restart afterwards; only APT as root has one.
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
}

/// The commands an operation with this manager runs, in order.
pub fn manager_commands(manager: Manager, root: bool, upgrade: bool) -> Vec<&'static str> {
    let commands = manager.commands(root);
    let mut list = vec![DETECT, commands.update, commands.list];
    if upgrade {
        list.extend([commands.upgrade, commands.list]);
    }
    list.extend(commands.status);
    list
}

/// The commands a check or upgrade runs on this host; only DETECT until its manager is known.
pub fn package_commands(host: &Host, upgrade: bool) -> Vec<&'static str> {
    match Manager::from_id(host.manager()) {
        Some(manager) => manager_commands(manager, host.username == "root", upgrade),
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
            // Restart status is APT-only and never offered to a regular account.
            assert_eq!(root.status.is_some(), manager == Manager::Apt);
            let user = manager.commands(false);
            assert_eq!(user.status, None);
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
