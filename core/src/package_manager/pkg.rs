//! FreeBSD and DragonFly `pkg`.

use super::Commands;
use crate::model::Package;
use anyhow::{Result, bail};

// `env` keeps these valid under a csh login shell.
pub const UPDATE: &str = "env LC_ALL=C pkg update";
pub const SUDO_UPDATE: &str = "env LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- pkg update";
// Comparing against the fetched catalogue needs no privileges.
pub const LIST: &str = "env LC_ALL=C pkg version -URv -l '<'";
pub const UPGRADE: &str = "env LC_ALL=C pkg upgrade -y";
pub const SUDO_UPGRADE: &str = "env LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- pkg upgrade -y";

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

pub(super) fn update_failed(line: &str) -> bool {
    line.contains("Unable to update repository")
}

/// Parses `pkg version -v -l '<'` lines such as `curl-8.9.0  <  needs updating (remote has 8.10.1)`.
pub(super) fn parse_pending(output: &str) -> Result<Vec<Package>> {
    let mut packages = Vec::new();
    for line in output.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let mut fields = line.split_whitespace();
        let (Some(full), Some("<")) = (fields.next(), fields.next()) else {
            bail!("無法解析 pkg 套件清單；請查看原始輸出");
        };
        let (Some((name, current)), Some((_, candidate))) =
            (full.rsplit_once('-'), line.split_once(" has "))
        else {
            bail!("無法解析 pkg 套件清單；請查看原始輸出");
        };
        packages.push(Package {
            name: name.into(),
            current_version: current.into(),
            candidate_version: candidate.trim().trim_end_matches(')').into(),
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
    fn version_lines_split_name_and_versions() {
        let packages = parse_pending(
            "curl-8.9.0                         <   needs updating (remote has 8.10.1)
             py311-cryptography-42.0.8,1        <   needs updating (remote has 43.0.1,1)
",
        )
        .unwrap();
        assert_eq!(packages[0].name, "curl");
        assert_eq!(packages[0].current_version, "8.9.0");
        assert_eq!(packages[0].candidate_version, "8.10.1");
        assert_eq!(packages[1].name, "py311-cryptography");
        assert_eq!(packages[1].candidate_version, "43.0.1,1");
        assert!(parse_pending("pkg: No package database installed").is_err());
    }

    #[test]
    fn repository_failures_are_seen() {
        assert!(update_failed("pkg: Unable to update repository FreeBSD"));
        assert!(!update_failed("All repositories are up to date."));
    }
}
