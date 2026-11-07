//! APT (Debian, Ubuntu).

use super::Commands;
use crate::model::Package;
use anyhow::{Result, bail, ensure};

pub const UPDATE: &str = "LC_ALL=C apt-get -q=1 -o APT::Update::Error-Mode=any update";
pub const SUDO_UPDATE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- apt-get -q=1 -o APT::Update::Error-Mode=any update";
pub const LIST: &str = "LC_ALL=C apt list --upgradable";
// Never prompt: keep locally modified config files and only list services needing restart.
pub const UPGRADE: &str = "LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade";
// sudo resets the environment, so the variables go through `env` after it.
pub const SUDO_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- env LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade";

pub(super) const ROOT: Commands = Commands {
    update: UPDATE,
    list: LIST,
    upgrade: UPGRADE,
};
pub(super) const SUDO: Commands = Commands {
    update: SUDO_UPDATE,
    list: LIST,
    upgrade: SUDO_UPGRADE,
};

/// A repository that failed to download still lets `apt-get update` exit 0.
pub(super) fn update_failed(line: &str) -> bool {
    line.starts_with("E:")
        || line.starts_with("Err:")
        || line.contains("Failed to fetch")
        || line.contains("Some index files failed to download")
}

/// Parses `apt list --upgradable` rows such as `curl/stable 8.1 amd64 [upgradable from: 8.0]`.
pub fn parse_pending(output: &str) -> Result<Vec<Package>> {
    let mut packages = Vec::new();
    for line in output.lines().map(str::trim).filter(|s| !s.is_empty()) {
        if line.starts_with("Listing") || line.starts_with("WARNING:") {
            continue;
        }
        let Some((name, rest)) = line.split_once('/') else {
            bail!("無法解析 APT 套件清單；請查看原始輸出");
        };
        let fields: Vec<_> = rest.split_whitespace().collect();
        let Some((_, current)) = rest.split_once("[upgradable from:") else {
            bail!("無法解析 APT 套件清單；請查看原始輸出");
        };
        ensure!(fields.len() >= 3, "無法解析 APT 套件清單；請查看原始輸出");
        packages.push(Package {
            name: name.into(),
            repository: fields[0].into(),
            candidate_version: fields[1].into(),
            architecture: fields[2].into(),
            current_version: current.trim().trim_end_matches(']').trim().into(),
        });
    }
    Ok(packages)
}

/// Packages an upgrade deferred for a phased rollout, under either heading APT has used.
pub fn parse_phased(output: &str) -> Vec<String> {
    let mut phased = Vec::new();
    let mut listing = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed == "The following upgrades have been deferred due to phasing:"
            || trimmed == "Not upgrading yet due to phasing:"
        {
            listing = true;
        } else if listing && line.starts_with(' ') && !trimmed.is_empty() {
            phased.extend(trimmed.split_whitespace().map(str::to_owned));
        } else {
            listing = false;
        }
    }
    phased.sort();
    phased.dedup();
    phased
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_rows_split_into_versions_and_unknown_text_is_refused() {
        let packages = parse_pending(
            "Listing... Done\ncurl/stable-security 8.14.1-2 amd64 [upgradable from: 8.14.1-1]\n",
        )
        .unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(
            (
                packages[0].name.as_str(),
                packages[0].repository.as_str(),
                packages[0].candidate_version.as_str(),
                packages[0].architecture.as_str(),
                packages[0].current_version.as_str()
            ),
            ("curl", "stable-security", "8.14.1-2", "amd64", "8.14.1-1")
        );
        assert!(parse_pending("Listing...\n").unwrap().is_empty());
        assert!(parse_pending("unexpected localized output").is_err());
        assert!(parse_pending("package/repo missing-fields").is_err());
    }

    #[test]
    fn failed_repositories_are_seen_even_with_zero_exit() {
        assert!(update_failed(
            "W: Failed to fetch https://repo.invalid/Packages"
        ));
        assert!(update_failed("W: Some index files failed to download."));
        assert!(update_failed("Err:1 http://deb.example stable InRelease"));
        assert!(!update_failed("Hit:1 http://deb.example stable InRelease"));
    }

    #[test]
    fn phased_upgrades_are_read_under_old_and_new_wording() {
        let old = "Calculating upgrade...\nThe following upgrades have been deferred due to phasing:\n  libfoo1 python3-bar\n  zed\n0 upgraded, 0 newly installed, 0 to remove and 3 not upgraded.\n";
        assert_eq!(parse_phased(old), ["libfoo1", "python3-bar", "zed"]);
        let new = "Not upgrading yet due to phasing:\r\n  libfoo1 zed\r\n\r\nSummary:\r\n  Upgrading: 0\r\n";
        assert_eq!(parse_phased(new), ["libfoo1", "zed"]);
        assert!(parse_phased("Summary:\n  Upgrading: 2\n").is_empty());
    }
}
