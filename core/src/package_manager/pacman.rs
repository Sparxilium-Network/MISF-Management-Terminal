//! pacman (Arch Linux and derivatives).
//!
//! Arch does not support partial upgrades: syncing the system's package databases
//! without upgrading leaves the host one manual install away from a broken library. So a
//! check never runs `pacman -Sy`; `checkupdates` (pacman-contrib, which needs fakeroot)
//! syncs a private copy under /tmp instead, and only the upgrade, `pacman -Syu`, touches
//! the system's databases. Neither the refresh nor the listing needs root.

use super::Commands;
use crate::model::Package;
use anyhow::{Result, bail};

/// Syncs checkupdates' private database; its exit 2 means "nothing to upgrade".
pub const UPDATE: &str =
    "LC_ALL=C sh -c 'checkupdates --nocolor >/dev/null; rc=$?; [ $rc -eq 2 ] && rc=0; exit $rc'";
/// Compares the private database just synced with the installed packages.
pub const LIST: &str = "LC_ALL=C checkupdates --nosync --nocolor";
pub const UPGRADE: &str = "LC_ALL=C pacman -Syu --noconfirm --noprogressbar --color never";
pub const SUDO_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- pacman -Syu --noconfirm --noprogressbar --color never";

pub(super) const ROOT: Commands = Commands {
    update: UPDATE,
    list: LIST,
    upgrade: UPGRADE,
    dist_upgrade: None,
    simulate: None,
    preview: None,
    status: None,
};
pub(super) const SUDO: Commands = Commands {
    update: UPDATE,
    list: LIST,
    upgrade: SUDO_UPGRADE,
    dist_upgrade: None,
    simulate: None,
    preview: None,
    status: None,
};

/// What a failed refresh says when the exit code alone is not explained.
pub(super) const NEEDS: &str =
    "pacman 主機需要安裝 pacman-contrib（checkupdates）與 fakeroot 才能檢查更新";

/// checkupdates reports a failed sync as `==> ERROR: Cannot fetch updates`.
pub(super) fn update_failed(line: &str) -> bool {
    line.starts_with("==> ERROR:")
}

/// checkupdates exits 2 when nothing is pending.
pub(super) fn listed(code: u32) -> bool {
    code == 0 || code == 2
}

/// pacman's own errors; a failed hook or install script prints one and still exits 0.
pub(super) fn upgrade_failed(line: &str) -> bool {
    line.starts_with("error:")
}

/// Parses `name current -> candidate` lines; an IgnorePkg entry ends with `[ignored]`.
pub(super) fn parse_pending(output: &str) -> Result<Vec<Package>> {
    let mut packages = Vec::new();
    for line in output.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let ([name, current, "->", candidate] | [name, current, "->", candidate, "[ignored]"]) =
            fields[..]
        else {
            bail!("無法解析 pacman 套件清單；請查看原始輸出");
        };
        packages.push(Package {
            name: name.into(),
            current_version: current.into(),
            candidate_version: candidate.into(),
            architecture: String::new(),
            repository: String::new(),
        });
    }
    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_rows_split_into_versions_and_unknown_text_is_refused() {
        let packages = parse_pending(
            "coreutils 9.12-2 -> 9.12-3
linux 6.17.1.arch1-1 -> 1:6.17.2.arch1-1 [ignored]

",
        )
        .unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "coreutils");
        assert_eq!(packages[0].current_version, "9.12-2");
        assert_eq!(packages[0].candidate_version, "9.12-3");
        assert_eq!(packages[1].candidate_version, "1:6.17.2.arch1-1");
        assert!(parse_pending("").unwrap().is_empty());
        assert!(parse_pending("==> ERROR: Cannot find the fakeroot binary").is_err());
    }

    #[test]
    fn failures_the_exit_code_hides_are_seen() {
        assert!(update_failed("==> ERROR: Cannot fetch updates"));
        assert!(!update_failed("coreutils 9.12-2 -> 9.12-3"));
        assert!(listed(0) && listed(2) && !listed(1));
        assert!(upgrade_failed("error: command failed to execute correctly"));
        assert!(!upgrade_failed("(1/1) upgrading coreutils"));
    }

    #[test]
    fn checks_never_sync_the_system_databases() {
        for command in [UPDATE, LIST] {
            assert!(!command.contains("pacman -S"), "{command}");
            assert!(!command.contains("sudo"), "{command}");
        }
    }
}
