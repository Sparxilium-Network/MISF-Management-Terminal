//! APT (Debian, Ubuntu).

use super::Commands;
use crate::model::{Package, RestartStatus, UpgradePlan};
use anyhow::{Result, bail, ensure};

pub const UPDATE: &str = "LC_ALL=C apt-get -q=1 -o APT::Update::Error-Mode=any update";
pub const SUDO_UPDATE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- apt-get -q=1 -o APT::Update::Error-Mode=any update";
pub const LIST: &str = "LC_ALL=C apt list --upgradable";
// Never prompt: keep locally modified config files and only list services needing restart.
pub const UPGRADE: &str = "LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade";
// sudo resets the environment, so the variables go through `env` after it.
pub const SUDO_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- env LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade";
// Same safeguards as the upgrade, but may install new packages and remove installed ones.
pub const DIST_UPGRADE: &str = "LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold dist-upgrade";
pub const SUDO_DIST_UPGRADE: &str = "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- env LC_ALL=C DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l apt-get -y -q=1 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold dist-upgrade";
/// `-s` only prints what dist-upgrade would do; it takes no lock and needs no root.
pub const SIMULATE: &str = "LC_ALL=C apt-get -s -q=1 dist-upgrade";
/// Run by a check: what `upgrade` would do, read for the updates a phased rollout
/// defers on this host. Like SIMULATE, it takes no lock, needs no root, changes nothing.
pub const PREVIEW: &str = "LC_ALL=C apt-get -s -q=1 upgrade";

/// Read-only restart status: `reboot-required` plus needrestart in list mode (`-r l`)
/// with batch output (`-b`); it never restarts anything.
macro_rules! status_script {
    () => {
        "sh -c 'if [ -f /var/run/reboot-required ]; then echo __MISF_REBOOT__; if [ -r /var/run/reboot-required.pkgs ]; then sed \"s/^/__MISF_REBOOT_PKG__/\" /var/run/reboot-required.pkgs; fi; fi; if command -v needrestart >/dev/null 2>&1; then LC_ALL=C needrestart -r l -b; else echo __MISF_NO_NEEDRESTART__; fi'"
    };
}
pub const STATUS: &str = status_script!();
/// The whole script runs inside sudo, like the update and upgrade.
pub const SUDO_STATUS: &str = concat!(
    "LC_ALL=C sudo -S -p '__MISF_SUDO_PROMPT__' -- ",
    status_script!()
);

pub(super) const ROOT: Commands = Commands {
    update: UPDATE,
    list: LIST,
    upgrade: UPGRADE,
    dist_upgrade: Some(DIST_UPGRADE),
    simulate: Some(SIMULATE),
    preview: Some(PREVIEW),
    status: Some(STATUS),
};
pub(super) const SUDO: Commands = Commands {
    update: SUDO_UPDATE,
    list: LIST,
    upgrade: SUDO_UPGRADE,
    dist_upgrade: Some(SUDO_DIST_UPGRADE),
    simulate: Some(SIMULATE),
    preview: Some(PREVIEW),
    status: Some(SUDO_STATUS),
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

/// Packages listed under "have been kept back", which need a dist-upgrade.
pub fn parse_kept_back(output: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut inside = false;
    for line in output.lines() {
        if line
            .trim_start()
            .starts_with("The following packages have been kept back")
        {
            inside = true;
        } else if inside && line.starts_with(' ') {
            for name in line.split_whitespace() {
                if !names.iter().any(|known| known == name) {
                    names.push(name.into());
                }
            }
        } else {
            inside = false;
        }
    }
    names
}

/// Reads `apt-get -s dist-upgrade`: `Inst name [old] (new …)`, `Inst name (new …)` and
/// `Remv name [old]` lines; `None` when there are none.
pub fn parse_plan(output: &str) -> Option<UpgradePlan> {
    let mut plan = UpgradePlan::default();
    for line in output.lines().map(str::trim) {
        let (install, rest) = if let Some(rest) = line.strip_prefix("Inst ") {
            (true, rest)
        } else if let Some(rest) = line.strip_prefix("Remv ") {
            (false, rest)
        } else {
            continue;
        };
        let (name, rest) = rest.split_once(' ').unwrap_or((rest, ""));
        let mut package = Package {
            name: name.into(),
            ..Default::default()
        };
        let mut rest = rest.trim();
        if let Some(inner) = rest.strip_prefix('[')
            && let Some((current, after)) = inner.split_once(']')
        {
            package.current_version = current.into();
            rest = after.trim();
        }
        if let Some(inner) = rest.strip_prefix('(') {
            let inner = inner.trim_end_matches(')');
            let mut fields = inner.split_whitespace();
            package.candidate_version = fields.next().unwrap_or_default().into();
            package.repository = fields.next().unwrap_or_default().into();
            if let Some(start) = inner.rfind('[') {
                package.architecture = inner[start + 1..].trim_end_matches(']').into();
            }
        }
        match (install, package.current_version.is_empty()) {
            (false, _) => plan.remove.push(package),
            (true, true) => plan.install.push(package),
            (true, false) => plan.upgrade.push(package),
        }
    }
    (!plan.install.is_empty() || !plan.upgrade.is_empty() || !plan.remove.is_empty())
        .then_some(plan)
}

/// Reads [`STATUS`] output; `None` when neither part of it ran.
pub fn parse_restart(code: u32, output: &str) -> Option<RestartStatus> {
    let mut status = RestartStatus::default();
    let (mut answered, mut kernel_status) = (false, 0);
    let mut expected = String::new();
    for line in output.lines().map(str::trim) {
        if line == "__MISF_REBOOT__" {
            status.reboot_required = true;
        } else if let Some(package) = line.strip_prefix("__MISF_REBOOT_PKG__") {
            if !package.is_empty() && !status.reboot_packages.iter().any(|p| p == package) {
                status.reboot_packages.push(package.into());
            }
        } else if line == "__MISF_NO_NEEDRESTART__" {
            answered = true;
        } else if let Some((key, value)) = line.split_once(": ") {
            let value = value.trim().to_owned();
            match key {
                "NEEDRESTART-VER" => {
                    answered = true;
                    status.needrestart = true;
                }
                "NEEDRESTART-KCUR" => status.running_kernel = value,
                "NEEDRESTART-KEXP" => expected = value,
                // 0 unknown, 1 current, 2 ABI-compatible upgrade, 3 version upgrade.
                "NEEDRESTART-KSTA" => kernel_status = value.parse().unwrap_or(0),
                "NEEDRESTART-SVC" => status.services.push(value),
                "NEEDRESTART-CONT" => status.containers.push(value),
                "NEEDRESTART-SESS" => status.sessions.push(value),
                _ => {}
            }
        }
    }
    if kernel_status >= 2 {
        status.reboot_required = true;
        status.expected_kernel = expected;
    }
    (answered || code == 0).then_some(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_back_packages_and_the_dist_upgrade_plan() {
        let upgrade = "Calculating upgrade...
The following packages have been kept back:
  linux-generic linux-headers-generic
  linux-image-generic
0 upgraded, 0 newly installed, 0 to remove and 3 not upgraded.
";
        assert_eq!(
            parse_kept_back(upgrade),
            [
                "linux-generic",
                "linux-headers-generic",
                "linux-image-generic"
            ]
        );
        assert!(parse_kept_back("Setting up openssl (3.0.17) ...").is_empty());
        let simulation = "NOTE: This is only a simulation!
Inst linux-image-7.0.0-43-generic (7.0.0-43.43 Ubuntu:26.04/resolute-updates [amd64])
Inst linux-generic [7.0.0-42.42] (7.0.0-43.43 Ubuntu:26.04/resolute-updates [amd64])
Remv old-tool [1.2-3]
Conf linux-generic (7.0.0-43.43 Ubuntu:26.04/resolute-updates [amd64])
";
        let plan = parse_plan(simulation).unwrap();
        assert_eq!(plan.install[0].name, "linux-image-7.0.0-43-generic");
        assert_eq!(plan.install[0].candidate_version, "7.0.0-43.43");
        assert_eq!(plan.install[0].architecture, "amd64");
        assert_eq!(plan.upgrade[0].name, "linux-generic");
        assert_eq!(plan.upgrade[0].current_version, "7.0.0-42.42");
        assert_eq!(plan.upgrade[0].repository, "Ubuntu:26.04/resolute-updates");
        assert_eq!(plan.remove[0].name, "old-tool");
        assert_eq!(plan.remove[0].current_version, "1.2-3");
        assert!(parse_plan("0 upgraded, 0 newly installed").is_none());
    }

    #[test]
    fn restart_status_reads_reboot_required_and_needrestart_batch_output() {
        let output = "__MISF_REBOOT__
__MISF_REBOOT_PKG__linux-image-7.0.0-43-generic
__MISF_REBOOT_PKG__linux-base
__MISF_REBOOT_PKG__linux-base
NEEDRESTART-VER: 3.6
NEEDRESTART-KCUR: 7.0.0-42-generic
NEEDRESTART-KEXP: 7.0.0-43-generic
NEEDRESTART-KSTA: 3
NEEDRESTART-SVC: systemd-logind.service
NEEDRESTART-SVC: dbus.service
NEEDRESTART-CONT: web-1
NEEDRESTART-SESS: alice @ session #3
";
        let status = parse_restart(0, output).unwrap();
        assert!(status.reboot_required && status.needrestart);
        assert_eq!(
            status.reboot_packages,
            ["linux-image-7.0.0-43-generic", "linux-base"]
        );
        assert_eq!(status.running_kernel, "7.0.0-42-generic");
        assert_eq!(status.expected_kernel, "7.0.0-43-generic");
        assert_eq!(status.services, ["systemd-logind.service", "dbus.service"]);
        assert_eq!(status.containers, ["web-1"]);
        assert_eq!(status.sessions, ["alice @ session #3"]);

        // A current kernel needs no reboot and keeps no expected version.
        let current = "NEEDRESTART-VER: 3.6
NEEDRESTART-KCUR: 7.0.0-43-generic
NEEDRESTART-KEXP: 7.0.0-43-generic
NEEDRESTART-KSTA: 1
";
        let status = parse_restart(0, current).unwrap();
        assert!(!status.reboot_required && status.expected_kernel.is_empty());

        // Without needrestart only reboot-required is known.
        let status = parse_restart(
            0,
            "__MISF_NO_NEEDRESTART__
",
        )
        .unwrap();
        assert!(!status.needrestart && !status.reboot_required);
        assert!(
            parse_restart(
                1,
                "sh: 1: needrestart: Permission denied
"
            )
            .is_none()
        );
    }

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
