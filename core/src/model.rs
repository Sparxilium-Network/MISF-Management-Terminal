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
    pub status: String,
    pub packages: Vec<Package>,
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
