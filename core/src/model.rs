use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub os: String,
    pub auth_type: String,
    pub password: String,
    pub private_key: String,
    pub key_passphrase: String,
    pub sudo_password: String,
    pub use_pty: bool,
    pub fingerprint: String,
    pub notes: String,
    /// Detected over SSH, e.g. "Ubuntu 26.04"; never taken from the editor.
    pub distro: String,
    /// Detected package manager ID ("apt", "dnf", "yum" or "pkg").
    pub package_manager: String,
    /// Package manager chosen in the editor; empty means use the detected one.
    pub manager_choice: String,
    pub last_check: Option<CheckResult>,
}

impl Host {
    pub fn validate(&mut self) -> Result<()> {
        self.name = self.name.trim().to_owned();
        self.address = self.address.trim().to_owned();
        self.username = self.username.trim().to_owned();
        ensure!(!self.name.is_empty(), "請填寫主機名稱");
        ensure!(
            !self.address.is_empty() && !self.address.chars().any(char::is_whitespace),
            "主機位址不可為空或包含空白"
        );
        ensure!(self.port > 0, "SSH 埠必須介於 1–65535");
        ensure!(!self.username.is_empty(), "請填寫 SSH 使用者名稱");
        ensure!(
            ["Linux", "BSD"].contains(&self.os.as_str()),
            "僅支援 Linux／BSD"
        );
        ensure!(
            ["password", "key"].contains(&self.auth_type.as_str()),
            "不支援的認證方式"
        );
        ensure!(
            manager_choices(&self.os).contains(&self.manager_choice.as_str()),
            "此目標系統不支援所選的套件管理器"
        );
        if self.auth_type == "key" {
            ensure!(!self.private_key.trim().is_empty(), "請匯入 SSH 私鑰");
            russh::keys::decode_secret_key(&self.private_key, self.passphrase())
                .map_err(|_| anyhow::anyhow!("SSH 私鑰格式或私鑰密語不正確"))?;
            self.password.clear();
        } else {
            ensure!(!self.password.is_empty(), "請填寫 SSH 密碼");
            self.private_key.clear();
            self.key_passphrase.clear();
        }
        Ok(())
    }

    /// The chosen, else the detected package manager; records older than detection only
    /// ever succeeded with APT.
    pub fn manager(&self) -> &str {
        if !self.manager_choice.is_empty() {
            &self.manager_choice
        } else if !self.package_manager.is_empty() {
            &self.package_manager
        } else if self
            .last_check
            .as_ref()
            .is_some_and(|r| r.status == "success")
        {
            "apt"
        } else {
            ""
        }
    }

    pub fn passphrase(&self) -> Option<&str> {
        (!self.key_passphrase.is_empty()).then_some(self.key_passphrase.as_str())
    }

    pub fn sudo_secret(&self) -> &str {
        if !self.sudo_password.is_empty() {
            &self.sudo_password
        } else if self.auth_type == "password" {
            &self.password
        } else {
            ""
        }
    }

    pub fn redact(&self, text: &str) -> String {
        let mut result = text.to_owned();
        for secret in [
            &self.password,
            &self.sudo_password,
            &self.key_passphrase,
            &self.private_key,
        ] {
            if !secret.is_empty() {
                result = result.replace(secret, "[已隱藏]");
            }
        }
        result
    }
}

/// Package managers a host of this OS family may be set to; "" means detect automatically.
pub fn manager_choices(os: &str) -> &'static [&'static str] {
    if os == "BSD" {
        &["", "pkg"]
    } else {
        &["", "apt", "dnf", "yum"]
    }
}

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq)]
pub struct Package {
    pub name: String,
    pub current_version: String,
    pub candidate_version: String,
    pub architecture: String,
    pub repository: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CheckResult {
    pub checked_at: String,
    /// "check" or "upgrade"; empty in records saved before upgrades existed.
    pub operation: String,
    pub status: String,
    /// What detection found on this run; empty when it never got that far.
    pub distro: String,
    pub package_manager: String,
    /// Packages still pending after this operation.
    pub packages: Vec<Package>,
    /// Packages installed by a successful upgrade.
    pub upgraded: Vec<Package>,
    pub output: String,
    pub error: String,
}

pub fn parse_packages(output: &str) -> Result<Vec<Package>> {
    let mut packages = Vec::new();
    for line in output.lines().map(str::trim).filter(|s| !s.is_empty()) {
        if line.starts_with("Listing") || line.starts_with("WARNING:") {
            continue;
        }
        let Some((name, rest)) = line.split_once('/') else {
            anyhow::bail!("無法解析 APT 套件清單；請查看原始輸出");
        };
        let fields: Vec<_> = rest.split_whitespace().collect();
        ensure!(
            fields.len() >= 3 && rest.contains("[upgradable from:"),
            "無法解析 APT 套件清單；請查看原始輸出"
        );
        let current = rest
            .split("[upgradable from:")
            .nth(1)
            .unwrap()
            .trim()
            .trim_end_matches(']')
            .trim();
        packages.push(Package {
            name: name.into(),
            repository: fields[0].into(),
            candidate_version: fields[1].into(),
            architecture: fields[2].into(),
            current_version: current.into(),
        });
    }
    Ok(packages)
}

/// Parses `check-update` rows, then the installed `name.arch evr` lines after the marker.
pub fn parse_rpm_packages(output: &str) -> Result<Vec<Package>> {
    let (updates, installed) = output
        .split_once("__MISF_INSTALLED__")
        .ok_or_else(|| anyhow::anyhow!("無法解析 DNF／YUM 套件清單；請查看原始輸出"))?;
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
            anyhow::bail!("無法解析 DNF／YUM 套件清單；請查看原始輸出");
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

/// Parses FreeBSD `pkg version -v -l '<'` lines such as `curl-8.9.0  <  needs updating (remote has 8.10.1)`.
pub fn parse_pkg_packages(output: &str) -> Result<Vec<Package>> {
    let mut packages = Vec::new();
    for line in output.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let mut fields = line.split_whitespace();
        let (Some(full), Some("<")) = (fields.next(), fields.next()) else {
            anyhow::bail!("無法解析 pkg 套件清單；請查看原始輸出");
        };
        let (Some((name, current)), Some((_, candidate))) =
            (full.rsplit_once('-'), line.split_once(" has "))
        else {
            anyhow::bail!("無法解析 pkg 套件清單；請查看原始輸出");
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
    fn rpm_rows_survive_wrapping_and_obsoletes_and_pick_installed_versions() {
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
        let packages = parse_rpm_packages(output).unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "openssl");
        assert_eq!(packages[0].architecture, "x86_64");
        assert_eq!(packages[0].current_version, "1:3.0.7-27.el9");
        assert_eq!(packages[0].candidate_version, "1:3.0.7-28.el9");
        assert_eq!(packages[1].current_version, "3.9.16-1.el9");
        assert!(
            parse_rpm_packages(
                "__MISF_INSTALLED__
"
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            parse_rpm_packages(
                "openssl.x86_64 1-1 baseos
"
            )
            .is_err()
        );
        assert!(
            parse_rpm_packages(
                "Error: something
__MISF_INSTALLED__
"
            )
            .is_err()
        );
    }

    #[test]
    fn freebsd_pkg_version_lines_split_name_and_versions() {
        let packages = parse_pkg_packages(
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
        assert!(parse_pkg_packages("pkg: No package database installed").is_err());
    }
}
