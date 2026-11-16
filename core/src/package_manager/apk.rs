//! apk (Alpine Linux), apk-tools 2 and 3.
//!
//! Alpine ships doas rather than sudo; doas cannot read a password from its input, so
//! accounts other than root need sudo installed, like on every other manager.

use super::Commands;
use crate::model::Package;
use anyhow::{Result, bail};

pub const UPDATE: &str = "LC_ALL=C apk update";
pub const SUDO_UPDATE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- apk update";
// Reading the installed database and the fetched indexes needs no privileges.
pub const LIST: &str = "LC_ALL=C apk list --upgradable";
pub const UPGRADE: &str = "LC_ALL=C apk upgrade --no-progress";
pub const SUDO_UPGRADE: &str =
    "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- apk upgrade --no-progress";

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
    update: SUDO_UPDATE,
    list: LIST,
    upgrade: SUDO_UPGRADE,
    dist_upgrade: None,
    simulate: None,
    preview: None,
    status: None,
};

/// An index apk could not open or fetch is only a warning, and `apk update` still exits
/// 0; apk-tools 2 sums it up as `N errors; …`.
pub(super) fn update_failed(line: &str) -> bool {
    line.starts_with("ERROR:")
        || line.starts_with("WARNING: opening")
        || line.starts_with("WARNING: fetching")
        || line.starts_with("WARNING: Ignoring")
        || line.contains(" errors; ")
}

pub(super) fn upgrade_failed(line: &str) -> bool {
    line.starts_with("ERROR:")
}

/// Splits `name-1.2.3-r0` into the name and `1.2.3-r0`; versions always end in `-rN`.
fn split(full: &str) -> Option<(&str, &str)> {
    let (rest, release) = full.rsplit_once('-')?;
    let (name, _) = rest.rsplit_once('-')?;
    (release.starts_with('r') && !name.is_empty()).then(|| (name, &full[name.len() + 1..]))
}

/// Parses `apk list --upgradable` lines such as
/// `zlib-1.4-r0 x86_64 {zlib} (Zlib) [upgradable from: zlib-1.3.2-r1]`; warnings about
/// unreadable indexes are skipped, since the refresh already judged them.
pub(super) fn parse_pending(output: &str) -> Result<Vec<Package>> {
    let mut packages = Vec::new();
    for line in output.lines().map(str::trim) {
        if line.is_empty() || line.starts_with("WARNING:") {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(full), Some(architecture), Some((_, from))) = (
            fields.next(),
            fields.next(),
            line.split_once("[upgradable from: "),
        ) else {
            bail!("無法解析 apk 套件清單；請查看原始輸出");
        };
        let (Some((name, candidate)), Some((_, current))) =
            (split(full), split(from.trim_end_matches(']')))
        else {
            bail!("無法解析 apk 套件清單；請查看原始輸出");
        };
        packages.push(Package {
            name: name.into(),
            current_version: current.into(),
            candidate_version: candidate.into(),
            architecture: architecture.into(),
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
            "WARNING: opening /nonexistent/repo/x86_64/APKINDEX.tar.gz: No such file or directory
zlib-1.4-r0 x86_64 {zlib} (Zlib) [upgradable from: zlib-1.3.2-r1]
py3-cryptography-46.0.1-r0 x86_64 {py3-cryptography} (Apache-2.0 OR BSD-3-Clause) [upgradable from: py3-cryptography-45.0.7-r1]
",
        )
        .unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "zlib");
        assert_eq!(packages[0].current_version, "1.3.2-r1");
        assert_eq!(packages[0].candidate_version, "1.4-r0");
        assert_eq!(packages[0].architecture, "x86_64");
        assert_eq!(packages[1].name, "py3-cryptography");
        assert_eq!(packages[1].current_version, "45.0.7-r1");
        assert!(parse_pending("").unwrap().is_empty());
        assert!(parse_pending("ERROR: unable to lock database").is_err());
        assert!(parse_pending("zlib-1.4-r0 x86_64 {zlib} (Zlib)").is_err());
    }

    #[test]
    fn unreadable_indexes_are_failures_even_with_zero_exit() {
        assert!(update_failed(
            "WARNING: opening /nonexistent/repo/x86_64/APKINDEX.tar.gz: No such file or directory"
        ));
        assert!(update_failed(
            "WARNING: fetching https://dl-cdn.alpinelinux.org/alpine/v3.24/main: temporary error (try again later)"
        ));
        assert!(update_failed("1 errors; 28671 distinct packages available"));
        assert!(!update_failed("OK: 28671 distinct packages available"));
        assert!(upgrade_failed(
            "ERROR: misf-fail-2.0-r0.post-upgrade: script exited with error 1"
        ));
        assert!(!upgrade_failed("(1/1) Upgrading zlib (1.3.2-r1 -> 1.4-r0)"));
    }
}
