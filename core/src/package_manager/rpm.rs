//! What DNF and YUM share: both list through `check-update` and read versions from rpm.

use crate::model::Package;
use anyhow::{Result, bail, ensure};

/// Separates `check-update` rows from the `rpm -qa` installed versions in a listing.
pub(super) const INSTALLED_MARKER: &str = "__MISF_INSTALLED__";

/// Skipped repositories still exit 0 under `skip_if_unavailable`.
pub(super) fn update_failed(line: &str) -> bool {
    line.starts_with("Error:")
        || line.contains("Failed to download metadata")
        || line.starts_with("Ignoring repositories")
}

/// `check-update` exits 100 when updates exist.
pub(super) fn listed(code: u32) -> bool {
    code == 0 || code == 100
}

/// Parses `check-update` rows, then the installed `name.arch evr` lines after the marker.
pub(super) fn parse_pending(output: &str) -> Result<Vec<Package>> {
    let Some((updates, installed)) = output.split_once(INSTALLED_MARKER) else {
        bail!("無法解析 DNF／YUM 套件清單；請查看原始輸出");
    };
    let mut tokens = Vec::new();
    for line in updates.lines().map(str::trim) {
        if line.to_ascii_lowercase().starts_with("obsoleting") {
            break;
        }
        if line.is_empty()
            || line.starts_with("Last metadata expiration check")
            || line.starts_with("Security:")
        {
            continue;
        }
        // yum wraps long names onto their own line, so rows are rebuilt from tokens.
        tokens.extend(line.split_whitespace());
    }
    ensure!(
        tokens.len() % 3 == 0,
        "無法解析 DNF／YUM 套件清單；請查看原始輸出"
    );
    let mut packages = Vec::new();
    for row in tokens.chunks(3) {
        let Some((name, architecture)) = row[0].rsplit_once('.') else {
            bail!("無法解析 DNF／YUM 套件清單；請查看原始輸出");
        };
        let current = installed
            .lines()
            .rev()
            .filter_map(|line| line.trim().split_once(' '))
            .find(|(key, _)| *key == row[0])
            .map(|(_, version)| version.trim())
            .unwrap_or_default();
        packages.push(Package {
            name: name.into(),
            current_version: current.into(),
            candidate_version: row[1].into(),
            architecture: architecture.into(),
            repository: row[2].into(),
        });
    }
    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_survive_wrapping_and_obsoletes_and_pick_installed_versions() {
        let output = "
Last metadata expiration check: 0:01:02 ago.
            openssl.x86_64   1:3.0.7-28.el9   baseos
            python3-a-very-long-package-name.noarch
      3.9.18-1.el9   appstream
            Obsoleting Packages
foo.x86_64 2-1 baseos
    bar.x86_64 1-1 @baseos
            __MISF_INSTALLED__
openssl.x86_64 1:3.0.7-27.el9
            python3-a-very-long-package-name.noarch 3.9.16-1.el9
";
        let packages = parse_pending(output).unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "openssl");
        assert_eq!(packages[0].architecture, "x86_64");
        assert_eq!(packages[0].current_version, "1:3.0.7-27.el9");
        assert_eq!(packages[0].candidate_version, "1:3.0.7-28.el9");
        assert_eq!(packages[1].current_version, "3.9.16-1.el9");
        assert!(parse_pending("__MISF_INSTALLED__\n").unwrap().is_empty());
        assert!(parse_pending("openssl.x86_64 1-1 baseos\n").is_err());
        assert!(parse_pending("Error: something\n__MISF_INSTALLED__\n").is_err());
    }

    #[test]
    fn skipped_repositories_and_exit_codes() {
        assert!(update_failed("Ignoring repositories: epel"));
        assert!(update_failed(
            "Error: Failed to download metadata for repo 'epel'"
        ));
        assert!(!update_failed("Metadata cache created."));
        assert!(listed(0) && listed(100) && !listed(1));
    }
}
