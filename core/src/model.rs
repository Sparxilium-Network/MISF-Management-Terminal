use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const RSA_UNSUPPORTED: &str =
    "不支援 RSA 金鑰；請改用 Ed25519 或 ECDSA（nistp256／nistp384／nistp521）金鑰";

/// Decodes a private key and rejects RSA, which this build cannot sign with.
pub fn decode_private_key(text: &str, passphrase: Option<&str>) -> Result<russh::keys::PrivateKey> {
    ensure!(!text.contains("BEGIN RSA PRIVATE KEY"), RSA_UNSUPPORTED);
    let key = russh::keys::decode_secret_key(text, passphrase)
        .map_err(|_| anyhow::anyhow!("SSH 私鑰格式或私鑰密語不正確"))?;
    ensure!(
        !matches!(key.algorithm(), russh::keys::Algorithm::Rsa { .. }),
        RSA_UNSUPPORTED
    );
    Ok(key)
}

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
    /// A stored [`SshKey`] used instead of `private_key`; empty for an inline key.
    pub key_id: String,
    /// The [`HostGroup`] the sidebar lists this host under; empty for ungrouped.
    pub group_id: String,
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
    /// What the last "test connection" negotiated and found, successful or not.
    pub last_test: Option<ConnectionTest>,
}

/// One "test connection" run: the negotiated SSH algorithms, how this client signed in
/// and what the host reported. Holds no secrets; failed runs keep what was learned first.
#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ConnectionTest {
    pub tested_at: String,
    /// "success" or "failed".
    pub status: String,
    pub error: String,
    /// The server's identification line, e.g. "SSH-2.0-OpenSSH_9.6p1"; control characters removed.
    pub server_version: String,
    pub kex: String,
    pub host_key_algorithm: String,
    pub host_key_fingerprint: String,
    pub cipher: String,
    /// "none" when the cipher is an AEAD that authenticates by itself.
    pub client_mac: String,
    pub server_mac: String,
    /// OpenSSH strict key exchange, which closes the Terrapin prefix-truncation attack.
    pub strict_kex: bool,
    /// "password" or "publickey"; empty when the handshake failed first.
    pub auth_method: String,
    pub client_key_algorithm: String,
    pub client_key_fingerprint: String,
    pub kernel: String,
    pub distro: String,
    /// Supported package managers on the host, preferred first.
    pub package_managers: Vec<String>,
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
        if self.auth_type == "key" && !self.key_id.is_empty() {
            // The stored key is filled in when connecting; keep no second copy here.
            self.private_key.clear();
            self.key_passphrase.clear();
            self.password.clear();
        } else if self.auth_type == "key" {
            ensure!(!self.private_key.trim().is_empty(), "請匯入 SSH 私鑰");
            decode_private_key(&self.private_key, self.passphrase())?;
            self.password.clear();
        } else {
            ensure!(!self.password.is_empty(), "請填寫 SSH 密碼");
            self.private_key.clear();
            self.key_passphrase.clear();
            self.key_id.clear();
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

/// A private key saved once and shared by every host whose `key_id` names it.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SshKey {
    pub id: String,
    pub name: String,
    /// e.g. "ssh-ed25519", read from the key when it was added.
    pub algorithm: String,
    /// "SHA256:…" of the public key, for telling keys apart without the secret.
    pub fingerprint: String,
    pub private_key: String,
    pub passphrase: String,
}

/// A named, collapsible section of the sidebar's host list.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct HostGroup {
    pub id: String,
    pub name: String,
    pub collapsed: bool,
}

impl HostGroup {
    pub fn new(name: &str) -> Result<Self> {
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: Self::check_name(name)?,
            collapsed: false,
        })
    }

    pub fn check_name(name: &str) -> Result<String> {
        let name = name.trim();
        ensure!(!name.is_empty(), "請填寫分類名稱");
        ensure!(name.chars().count() <= 64, "分類名稱最多 64 個字");
        ensure!(
            !name.chars().any(char::is_control),
            "分類名稱不可包含控制字元"
        );
        Ok(name.into())
    }
}

impl SshKey {
    /// Checks that the key decodes with this passphrase and records what it is.
    pub fn new(name: &str, private_key: String, passphrase: String) -> Result<Self> {
        let name = name.trim();
        ensure!(!name.is_empty(), "請填寫 SSH 金鑰名稱");
        ensure!(name.chars().count() <= 64, "SSH 金鑰名稱最多 64 個字");
        ensure!(
            !name.chars().any(char::is_control),
            "SSH 金鑰名稱不可包含控制字元"
        );
        ensure!(!private_key.trim().is_empty(), "請匯入 SSH 私鑰");
        let key = decode_private_key(
            &private_key,
            (!passphrase.is_empty()).then_some(passphrase.as_str()),
        )?;
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            algorithm: key.algorithm().as_str().into(),
            fingerprint: key
                .public_key()
                .fingerprint(russh::keys::HashAlg::Sha256)
                .to_string(),
            private_key,
            passphrase,
        })
    }

    /// Everything the UI may show; the key material stays in the worker.
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "algorithm": self.algorithm,
            "fingerprint": self.fingerprint,
        })
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
    /// Names apt held back on this upgrade for a phased rollout; they stay in `packages`.
    pub phased: Vec<String>,
    pub output: String,
    pub error: String,
    /// What still needs a restart after this run; only collected for APT as root.
    pub restart: Option<RestartStatus>,
}

/// Reboot and restart needs read by a fixed status command after a check or upgrade.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct RestartStatus {
    /// `/var/run/reboot-required` exists, or needrestart expects a newer kernel.
    pub reboot_required: bool,
    /// Packages listed in `/var/run/reboot-required.pkgs`.
    pub reboot_packages: Vec<String>,
    /// Whether needrestart ran; without it the lists below are unknown, not empty.
    pub needrestart: bool,
    pub running_kernel: String,
    /// The kernel needrestart expects to be running; empty when it matches or is unknown.
    pub expected_kernel: String,
    /// Services running outdated libraries; MISF never restarts them.
    pub services: Vec<String>,
    pub containers: Vec<String>,
    /// User sessions running outdated binaries.
    pub sessions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsa_private_keys_are_rejected_in_both_formats() {
        use russh::keys::ssh_key::{
            LineEnding, Mpint, PrivateKey,
            private::{KeypairData, RsaKeypair, RsaPrivateKey},
            public::RsaPublicKey,
        };
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----\n";
        let error = decode_private_key(pem, None).err().unwrap().to_string();
        assert_eq!(error, RSA_UNSUPPORTED);

        // Fabricated numbers: only the key type matters, the key never signs.
        let int = |byte: u8| Mpint::from_positive_bytes(&[byte; 256]);
        let keypair = RsaKeypair::new(
            RsaPublicKey::new(Mpint::from_positive_bytes(&[1, 0, 1]), int(0xC1)).unwrap(),
            RsaPrivateKey::new(int(0x51), int(0x31), int(0xA1), int(0xB1)).unwrap(),
        )
        .unwrap();
        let openssh = PrivateKey::new(KeypairData::Rsa(keypair), "test")
            .unwrap()
            .to_openssh(LineEnding::LF)
            .unwrap();
        let error = decode_private_key(&openssh, None)
            .err()
            .unwrap()
            .to_string();
        assert_eq!(error, RSA_UNSUPPORTED);

        let ed25519 = PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
            .unwrap()
            .to_openssh(LineEnding::LF)
            .unwrap();
        assert!(decode_private_key(&ed25519, None).is_ok());
    }
}
