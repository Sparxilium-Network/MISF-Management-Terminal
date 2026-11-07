//! Finds the target's OS and which supported package managers it has.

use super::Manager;
use anyhow::{Result, ensure};

// Wrapped in sh so FreeBSD accounts with a csh login shell parse it too.
pub const DETECT: &str = "sh -c 'uname -s; uname -r; if [ -r /etc/os-release ]; then (. /etc/os-release; printf \"__MISF_NAME__%s\\n__MISF_VERSION__%s\\n__MISF_PRETTY__%s\\n\" \"$NAME\" \"$VERSION_ID\" \"$PRETTY_NAME\"); fi; if command -v apt-get >/dev/null 2>&1 && command -v apt >/dev/null 2>&1; then echo __MISF_PM__apt; fi; for m in dnf yum; do if command -v $m >/dev/null 2>&1; then echo __MISF_PM__$m; fi; done; if command -v pkg >/dev/null 2>&1 && pkg -N >/dev/null 2>&1; then echo __MISF_PM__pkg; fi; true'";

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
    ensure!(
        matches!(
            kernel.as_str(),
            "Linux" | "FreeBSD" | "OpenBSD" | "NetBSD" | "DragonFly"
        ),
        "僅支援 Linux／BSD 目標主機"
    );
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
    // Debian derivatives can carry yum, so the native manager wins.
    let preferred: &[Manager] = match kernel.as_str() {
        "Linux" => &[Manager::Apt, Manager::Dnf, Manager::Yum],
        "FreeBSD" | "DragonFly" => &[Manager::Pkg],
        _ => &[],
    };
    let available: Vec<Manager> = preferred
        .iter()
        .copied()
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
                "__MISF_PM__apt\n__MISF_PM__yum\n"
            ),
            Platform {
                kernel: "Linux".into(),
                distro: "Debian 13".into(),
                manager: Some(Manager::Apt),
                available: vec![Manager::Apt, Manager::Yum],
            }
        );
        let ubuntu = linux(
            "__MISF_NAME__Ubuntu\n__MISF_VERSION__26.04\n",
            "__MISF_PM__apt\n",
        );
        assert_eq!(ubuntu.distro, "Ubuntu 26.04");
        let rocky = linux(
            "__MISF_NAME__Rocky Linux\n__MISF_VERSION__9.5\n",
            "__MISF_PM__dnf\n__MISF_PM__yum\n",
        );
        assert_eq!(
            (rocky.distro.as_str(), rocky.manager, rocky.available),
            (
                "Rocky Linux 9.5",
                Some(Manager::Dnf),
                vec![Manager::Dnf, Manager::Yum]
            )
        );
        let sid = linux(
            "__MISF_NAME__Debian GNU/Linux\n__MISF_VERSION__\n__MISF_PRETTY__Debian GNU/Linux trixie/sid\n",
            "",
        );
        assert_eq!(
            (sid.distro.as_str(), sid.manager),
            ("Debian GNU/Linux trixie/sid", None)
        );
        let freebsd = parse_platform("FreeBSD\n14.2-RELEASE\n__MISF_PM__pkg\n").unwrap();
        assert_eq!(
            (freebsd.distro.as_str(), freebsd.manager),
            ("FreeBSD 14.2-RELEASE", Some(Manager::Pkg))
        );
        let openbsd = parse_platform("OpenBSD\n7.6\n__MISF_PM__pkg\n").unwrap();
        assert_eq!((openbsd.manager, openbsd.available), (None, vec![]));
        assert!(parse_platform("Darwin\n24.0\n").is_err());
    }
}
