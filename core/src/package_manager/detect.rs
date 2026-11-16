//! Finds the target's OS and which supported package managers it has.

use super::Manager;
use anyhow::{Result, ensure};

// Wrapped in sh so an account whose login shell is not POSIX parses it too.
pub const DETECT: &str = "sh -c 'uname -s; uname -r; if [ -r /etc/os-release ]; then (. /etc/os-release; printf \"__MISF_NAME__%s\\n__MISF_VERSION__%s\\n__MISF_PRETTY__%s\\n\" \"$NAME\" \"$VERSION_ID\" \"$PRETTY_NAME\"); fi; if command -v apt-get >/dev/null 2>&1 && command -v apt >/dev/null 2>&1; then echo __MISF_PM__apt; fi; for m in dnf pacman apk; do if command -v $m >/dev/null 2>&1; then echo __MISF_PM__$m; fi; done; true'";

/// What DETECT found: kernel name, a display name such as "Debian 13", and the package manager.
#[derive(Debug, PartialEq)]
pub struct Platform {
    pub kernel: String,
    pub distro: String,
    pub manager: Option<Manager>,
    /// Every supported manager on the host, preferred first.
    pub available: Vec<Manager>,
}

pub fn parse_platform(output: &str) -> Result<Platform> {
    let mut lines = output.lines().map(str::trim);
    let kernel = lines.next().unwrap_or_default().to_owned();
    ensure!(kernel == "Linux", "僅支援 Linux 目標主機");
    let release = lines.next().unwrap_or_default().to_owned();
    let (mut name, mut version, mut pretty, mut found) = ("", "", "", vec![]);
    for line in lines {
        if let Some(value) = line.strip_prefix("__MISF_NAME__") {
            name = value;
        } else if let Some(value) = line.strip_prefix("__MISF_VERSION__") {
            version = value;
        } else if let Some(value) = line.strip_prefix("__MISF_PRETTY__") {
            pretty = value;
        } else if let Some(value) = line.strip_prefix("__MISF_PM__") {
            found.extend(Manager::from_id(value));
        }
    }
    let name = name.trim_end_matches(" GNU/Linux");
    let distro = match (name, version, pretty) {
        ("", _, "") => format!("{kernel} {release}").trim().to_owned(),
        (_, "", pretty) if !pretty.is_empty() => pretty.to_owned(),
        (name, version, _) => format!("{name} {version}").trim().to_owned(),
    };
    // A host can carry a second manager's tools (Debian packages dnf and apk, for one);
    // the first in this order is taken as the native one.
    let preferred = [Manager::Apt, Manager::Dnf, Manager::Pacman, Manager::Apk];
    let available: Vec<Manager> = preferred
        .into_iter()
        .filter(|m| found.contains(m))
        .collect();
    Ok(Platform {
        kernel,
        distro,
        manager: available.first().copied(),
        available,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_names_the_distribution_and_prefers_the_native_manager() {
        let linux = |os: &str, managers: &str| {
            parse_platform(&format!("Linux\n6.12.0\n{os}{managers}")).unwrap()
        };
        assert_eq!(
            linux(
                "__MISF_NAME__Debian GNU/Linux\n__MISF_VERSION__13\n__MISF_PRETTY__Debian GNU/Linux 13 (trixie)\n",
                "__MISF_PM__apt\n__MISF_PM__apk\n"
            ),
            Platform {
                kernel: "Linux".into(),
                distro: "Debian 13".into(),
                manager: Some(Manager::Apt),
                available: vec![Manager::Apt, Manager::Apk],
            }
        );
        let ubuntu = linux(
            "__MISF_NAME__Ubuntu\n__MISF_VERSION__26.04\n",
            "__MISF_PM__apt\n",
        );
        assert_eq!(ubuntu.distro, "Ubuntu 26.04");
        let rocky = linux(
            "__MISF_NAME__Rocky Linux\n__MISF_VERSION__9.5\n",
            "__MISF_PM__dnf\n",
        );
        assert_eq!(
            (rocky.distro.as_str(), rocky.manager),
            ("Rocky Linux 9.5", Some(Manager::Dnf))
        );
        // Arch has no VERSION_ID: the pretty name stands in.
        let arch = linux(
            "__MISF_NAME__Arch Linux\n__MISF_VERSION__\n__MISF_PRETTY__Arch Linux\n",
            "__MISF_PM__pacman\n",
        );
        assert_eq!(
            (arch.distro.as_str(), arch.manager),
            ("Arch Linux", Some(Manager::Pacman))
        );
        let alpine = linux(
            "__MISF_NAME__Alpine Linux\n__MISF_VERSION__3.24.4\n",
            "__MISF_PM__apk\n",
        );
        assert_eq!(
            (alpine.distro.as_str(), alpine.manager),
            ("Alpine Linux 3.24.4", Some(Manager::Apk))
        );
        // A manager MISF does not drive, such as yum alone, leaves none.
        let centos = linux(
            "__MISF_NAME__CentOS Linux\n__MISF_VERSION__7\n",
            "__MISF_PM__yum\n",
        );
        assert_eq!((centos.manager, centos.available), (None, vec![]));
        let sid = linux(
            "__MISF_NAME__Debian GNU/Linux\n__MISF_VERSION__\n__MISF_PRETTY__Debian GNU/Linux trixie/sid\n",
            "",
        );
        assert_eq!(
            (sid.distro.as_str(), sid.manager),
            ("Debian GNU/Linux trixie/sid", None)
        );
        for other in [
            "FreeBSD\n14.2-RELEASE\n",
            "OpenBSD\n7.6\n",
            "Darwin\n24.0\n",
        ] {
            assert!(parse_platform(other).is_err(), "{other}");
        }
    }
}
