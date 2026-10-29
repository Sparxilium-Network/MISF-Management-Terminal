//! Optional LUKSbox storage. No mount driver and no plaintext extraction files.
use crate::{
    dispatch,
    store::{Database, Store},
};
use anyhow::{Context, Result, ensure};
use luksbox_core::{Argon2idParams, CipherSuite, SlotKind};
use luksbox_format::{Container, UnlockMaterial};
use luksbox_vfs::Vfs;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

const MAX_DATABASE_BYTES: u64 = 64 * 1024 * 1024;
const METADATA_BYTES: u64 = 1024 * 1024;
const DATABASE_FILE: &str = "hosts.json";

pub(crate) struct VaultFile {
    vfs: Vfs,
    poisoned: bool,
}

impl VaultFile {
    fn open(path: &Path, passphrase: &[u8]) -> Result<Self> {
        check_environment()?;
        let container = Container::open(path, None, UnlockMaterial::Passphrase(passphrase))
            .context("無法解鎖 LUKSbox；密碼錯誤、檔案損壞或容器正被另一個程序使用")?;
        Self::from_container(container)
    }

    fn from_container(container: Container) -> Result<Self> {
        let vfs = Vfs::open(container).context("無法讀取 LUKSbox 目錄")?;
        let mut vault = Self {
            vfs,
            poisoned: false,
        };
        vault.read()?;
        Ok(vault)
    }

    pub(crate) fn read(&mut self) -> Result<Database> {
        ensure!(
            !self.poisoned,
            "先前儲存失敗；請鎖定後重新解鎖，確認容器狀態"
        );
        let file = self
            .vfs
            .lookup_path("/hosts.json")
            .context("容器內沒有 MISF hosts.json")?;
        let size = self.vfs.stat(file)?.size;
        ensure!(size <= MAX_DATABASE_BYTES, "主機資料超過 64 MiB 上限");
        let mut bytes = Zeroizing::new(vec![0; size as usize]);
        let mut read = 0;
        while read < bytes.len() {
            let count = self.vfs.read(file, read as u64, &mut bytes[read..])?;
            ensure!(count > 0, "LUKSbox 主機資料不完整");
            read += count;
        }
        let db: Database = serde_json::from_slice(&bytes).context("加密主機資料格式錯誤")?;
        ensure!(db.version == 1, "不支援的主機資料版本");
        Ok(db)
    }

    pub(crate) fn write(&mut self, db: &Database) -> Result<()> {
        ensure!(!self.poisoned, "先前儲存失敗；請鎖定後重新解鎖");
        let bytes = Zeroizing::new(serde_json::to_vec(db)?);
        ensure!(
            bytes.len() as u64 <= MAX_DATABASE_BYTES,
            "主機資料超過 64 MiB 上限"
        );
        // Write a new inode then replace its directory entry in one metadata commit.
        // Never overwrite committed chunks or extract plaintext to disk.
        let operation = (|| {
            let root = self.vfs.root_id();
            let name = format!(".misf-pending-{}", uuid::Uuid::new_v4());
            let file = self.vfs.create(root, &name)?;
            let written = self.vfs.write(file, 0, &bytes)?;
            ensure!(written == bytes.len(), "LUKSbox 寫入不完整");
            self.vfs.rename(root, &name, root, DATABASE_FILE)?;
            self.vfs.flush()?;
            Ok(())
        })();
        if operation.is_err() {
            self.poisoned = true;
        }
        operation
    }

    fn finish(&mut self) -> Result<()> {
        if !self.poisoned {
            self.vfs.finish()?;
        }
        Ok(())
    }

    fn slots(&self) -> Vec<SlotInfo> {
        self.vfs
            .container()
            .slot_kinds()
            .iter()
            .enumerate()
            .filter(|(_, kind)| **kind != SlotKind::Empty)
            .map(|(index, kind)| SlotInfo {
                index,
                kind: format!("{kind:?}"),
            })
            .collect()
    }

    fn add_key(&mut self, passphrase: &[u8], params: Argon2idParams) -> Result<usize> {
        ensure!(!self.poisoned, "先前儲存失敗；請鎖定後重新解鎖");
        let result = (|| {
            let index = self
                .vfs
                .container_mut()
                .enroll_passphrase(passphrase, params)?;
            self.vfs.container_mut().persist_header()?;
            Ok(index)
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn revoke_key(&mut self, index: usize) -> Result<()> {
        ensure!(!self.poisoned, "先前儲存失敗；請鎖定後重新解鎖");
        let slots = self.slots();
        ensure!(
            slots.iter().any(|slot| slot.index == index),
            "指定金鑰槽不存在"
        );
        ensure!(
            slots.len() > 1,
            "禁止撤銷最後一把解鎖金鑰；請先新增並驗證備援金鑰"
        );
        let result = (|| {
            self.vfs.container_mut().revoke_slot(index)?;
            self.vfs.container_mut().persist_header()?;
            Ok(())
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn add_passkey(&mut self, hello: bool, pin: Option<&str>) -> Result<usize> {
        ensure!(!self.poisoned, "先前儲存失敗；請鎖定後重新解鎖");
        ensure!(self.slots().len() < 8, "所有金鑰槽均已使用");
        check_environment()?;
        let mut auth = crate::passkey::authenticator(hello)?;
        let index = crate::passkey::enroll(self.vfs.container_mut(), &mut auth, pin)?;
        if let Err(error) = self.vfs.container_mut().persist_header() {
            self.poisoned = true;
            return Err(error.into());
        }
        Ok(index)
    }
}

#[derive(Clone, Serialize)]
pub struct SlotInfo {
    pub index: usize,
    pub kind: String,
}

#[derive(Serialize)]
pub struct StorageStatus {
    pub encrypted: bool,
    pub locked: bool,
    pub path: PathBuf,
    pub plaintext_exists: bool,
    pub slots: Vec<SlotInfo>,
    pub warning: String,
}

/// Lives on the background worker. Decryption keys remain inside LUKSbox.
pub struct StorageSession {
    plain_path: PathBuf,
    store: Option<Store>,
    encrypted: bool,
    warning: String,
}

impl StorageSession {
    pub(crate) fn host_for_audit(&self, id: &str) -> Result<crate::model::Host> {
        self.store.as_ref().context("Vault locked")?.host(id)
    }
    pub fn new(plain_path: PathBuf) -> Self {
        let encrypted = encrypted_profile_exists(&plain_path);
        let store = (!encrypted).then(|| Store::new(plain_path.clone()));
        Self {
            plain_path,
            store,
            encrypted,
            warning: String::new(),
        }
    }

    pub fn status(&mut self) -> Result<StorageStatus> {
        // Another MISF instance may have enabled encryption after this one started.
        if !self.encrypted && encrypted_profile_exists(&self.plain_path) {
            self.encrypted = true;
            self.store = None;
        }
        if self.encrypted && self.plain_path.with_extension("lbx").exists() {
            ensure_encrypted_marker(&self.plain_path)?;
        }
        let slots = match self.store.as_ref().and_then(|store| store.vault.as_ref()) {
            Some(vault) => vault
                .lock()
                .map_err(|_| anyhow::anyhow!("加密工作階段發生錯誤"))?
                .slots(),
            None => vec![],
        };
        Ok(StorageStatus {
            encrypted: self.encrypted,
            locked: self.encrypted && self.store.is_none(),
            path: if self.encrypted {
                self.plain_path.with_extension("lbx")
            } else {
                self.plain_path.clone()
            },
            plaintext_exists: self.plain_path.exists(),
            slots,
            warning: self.warning.clone(),
        })
    }

    pub async fn handle(&mut self, mut request: Value, progress: &dyn Fn(&str)) -> Result<Value> {
        self.status()?;
        let action = request["action"]
            .as_str()
            .context("缺少 action")?
            .to_owned();
        match action.as_str() {
            "storage_status" => Ok(serde_json::to_value(self.status()?)?),
            "vault_create" => {
                let password = take_secret(&mut request)?;
                validate_passphrase(&password)?;
                progress("正在建立 LUKSbox，並驗證資料可重新解鎖…");
                self.enable(&password, Argon2idParams::INTERACTIVE)?;
                Ok(serde_json::to_value(self.status()?)?)
            }
            "vault_unlock" => {
                ensure!(self.encrypted, "尚未啟用 LUKSbox");
                let password = take_secret(&mut request)?;
                progress("正在解鎖 LUKSbox…");
                let path = self.plain_path.with_extension("lbx");
                let vault = VaultFile::open(&path, &password)?;
                self.store = Some(encrypted_store(path, vault));
                self.warning.clear();
                Ok(serde_json::to_value(self.status()?)?)
            }
            "vault_lock" => {
                ensure!(self.encrypted, "目前使用明文儲存");
                let result = self.with_vault(|vault| vault.finish());
                self.store = None;
                if let Err(error) = result {
                    self.warning = format!("已鎖定；最後同步失敗，請重新解鎖確認：{error:#}");
                }
                Ok(serde_json::to_value(self.status()?)?)
            }
            "vault_add_key" => {
                let password = take_secret(&mut request)?;
                validate_passphrase(&password)?;
                progress("正在新增 LUKSbox 解鎖密碼…");
                let index =
                    self.with_vault(|vault| vault.add_key(&password, Argon2idParams::INTERACTIVE))?;
                Ok(json!({"slot":index,"storage":self.status()?}))
            }
            "vault_add_passkey" => {
                let pin = take_pin(&mut request)?;
                require_hardware_pin(&pin)?;
                let hello = request["hello"].as_bool().unwrap_or(false);
                progress("請在系統視窗或安全金鑰上完成 Passkey 註冊與兩次驗證…");
                let index = self.with_vault(|vault| {
                    vault.add_passkey(hello, pin.as_deref().map(String::as_str))
                })?;
                Ok(json!({"slot":index,"storage":self.status()?}))
            }
            "vault_unlock_passkey" => {
                ensure!(self.encrypted && self.store.is_none(), "請先鎖定 LUKSbox");
                check_environment()?;
                let pin = take_pin(&mut request)?;
                require_hardware_pin(&pin)?;
                let mut auth =
                    crate::passkey::authenticator(request["hello"].as_bool().unwrap_or(false))?;
                progress("請在系統視窗或安全金鑰上完成 Passkey 驗證…");
                let path = self.plain_path.with_extension("lbx");
                let vault = VaultFile::from_container(crate::passkey::open(
                    &path,
                    &mut auth,
                    pin.as_deref().map(String::as_str),
                )?)?;
                self.store = Some(encrypted_store(path, vault));
                self.warning.clear();
                Ok(serde_json::to_value(self.status()?)?)
            }
            "vault_revoke_key" => {
                let index = request["slot"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .context("缺少有效的金鑰槽編號")?;
                self.with_vault(|vault| vault.revoke_key(index))?;
                Ok(json!({"slot":index,"storage":self.status()?}))
            }
            "vault_backup" => {
                let destination = request["path"].as_str().context("缺少備份路徑")?;
                self.backup(Path::new(destination))?;
                Ok(json!({"path":destination}))
            }
            _ => {
                let store = self
                    .store
                    .as_ref()
                    .context("主機庫已鎖定，請先輸入解鎖密碼")?;
                dispatch(request, store, progress).await
            }
        }
    }

    fn with_vault<T>(&self, operation: impl FnOnce(&mut VaultFile) -> Result<T>) -> Result<T> {
        let vault = self
            .store
            .as_ref()
            .and_then(|store| store.vault.as_ref())
            .context("請先解鎖 LUKSbox")?;
        let mut vault = vault
            .lock()
            .map_err(|_| anyhow::anyhow!("加密工作階段發生錯誤"))?;
        operation(&mut vault)
    }

    fn enable(&mut self, passphrase: &[u8], params: Argon2idParams) -> Result<()> {
        check_environment()?;
        ensure!(!self.encrypted, "已啟用 LUKSbox，禁止覆寫現有容器");
        let source = self.store.as_ref().context("目前主機庫不可用")?;
        let target = self.plain_path.with_extension("lbx");
        let store = source.with_db(
            |db| {
                ensure!(!target.exists(), "LUKSbox 容器已存在，禁止覆寫");
                for suffix in ["header-bak", "meta-bak"] {
                    ensure!(
                        !sidecar(&target, suffix).exists(),
                        "偵測到先前未完成轉換的容器備份；請先移動 .lbx.*-bak 後重試"
                    );
                }
                let dir = target.parent().context("容器路徑無效")?;
                let temp = tempfile::Builder::new()
                    .prefix(".misf-vault-")
                    .tempdir_in(dir)?;
                let staging = temp.path().join("hosts.lbx");
                let size_guard = luksbox_format::metadata::set_create_metadata_region_size_override(
                    Some(METADATA_BYTES),
                );
                let format_guard = luksbox_vfs::set_format_v3_override(Some(true));
                let container = Container::create_with_passphrase(
                    &staging,
                    None,
                    CipherSuite::Aes256GcmSiv,
                    params,
                    passphrase,
                )?;
                let mut vault = VaultFile {
                    vfs: Vfs::open(container)?,
                    poisoned: false,
                };
                vault.write(db)?;
                vault.finish()?;
                drop(vault);
                drop(format_guard);
                drop(size_guard);
                let mut verified = VaultFile::open(&staging, passphrase)?;
                let actual = Zeroizing::new(serde_json::to_vec(&verified.read()?)?);
                let expected = Zeroizing::new(serde_json::to_vec(db)?);
                ensure!(
                    *actual == *expected,
                    "LUKSbox 轉換驗證失敗；原明文檔案已保留"
                );
                verified.finish()?;
                drop(verified);
                let mut moved = vec![];
                let commit: Result<()> = (|| {
                    for suffix in ["header-bak", "meta-bak"] {
                        let staged = sidecar(&staging, suffix);
                        if staged.exists() {
                            let destination = sidecar(&target, suffix);
                            fs::rename(staged, &destination)?;
                            moved.push(destination);
                        }
                    }
                    fs::rename(&staging, &target)?;
                    Ok(())
                })();
                if commit.is_err() && !target.exists() {
                    for path in moved {
                        let _ = fs::remove_file(path);
                    }
                }
                commit.context("無法提交加密容器；原明文檔案已保留")?;
                let vault = VaultFile::open(&target, passphrase)?;
                Ok(encrypted_store(target.clone(), vault))
            },
            false,
        )?;
        self.store = Some(store);
        self.encrypted = true;
        self.warning.clear();
        ensure_encrypted_marker(&self.plain_path)?;
        Ok(())
    }

    fn backup(&self, destination: &Path) -> Result<()> {
        ensure!(self.encrypted, "加密備份需要先啟用 LUKSbox");
        ensure!(
            !destination.exists(),
            "備份檔案已存在；請使用新的檔名，避免覆寫"
        );
        let parent = destination.parent().context("備份路徑無效")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        self.with_vault(|vault| {
            use std::io::Write;
            ensure!(!vault.poisoned, "先前儲存失敗，請先重新解鎖確認資料");
            vault.finish()?;
            // LockFileEx on Windows rejects a second handle reading the file.
            // Copy ciphertext using the container's existing locked handle.
            let container = vault.vfs.container_mut();
            let length = container.data_file_len()?;
            let mut offset = 0;
            let mut bytes = vec![0; 64 * 1024];
            while offset < length {
                let count = (length - offset).min(bytes.len() as u64) as usize;
                container.read_at(offset, &mut bytes[..count])?;
                temp.write_all(&bytes[..count])?;
                offset += count as u64;
            }
            Ok(())
        })?;
        temp.as_file().sync_all()?;
        temp.persist_noclobber(destination)
            .map_err(|error| error.error)?;
        Ok(())
    }
}

fn encrypted_store(path: PathBuf, vault: VaultFile) -> Store {
    Store {
        path,
        vault: Some(Arc::new(Mutex::new(vault))),
    }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{suffix}"));
    PathBuf::from(name)
}

pub(crate) fn encrypted_profile_exists(plain_path: &Path) -> bool {
    plain_path.with_extension("lbx").exists() || plain_path.with_extension("lbx.enabled").exists()
}

fn ensure_encrypted_marker(plain_path: &Path) -> Result<()> {
    use std::io::Write;
    let path = plain_path.with_extension("lbx.enabled");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(b"MISF-LUKSbox-v1\n")?;
            file.sync_all()?;
            luksbox_core::file_util::sync_parent_dir(&path)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error).context("無法記錄加密模式；禁止回退明文儲存"),
    }
}

fn validate_passphrase(password: &[u8]) -> Result<()> {
    ensure!(
        std::str::from_utf8(password)?.chars().count() >= 12,
        "解鎖密碼至少需要 12 個字元，建議使用長密碼句"
    );
    Ok(())
}

fn take_secret(request: &mut Value) -> Result<Zeroizing<Vec<u8>>> {
    match request["passphrase"].take() {
        Value::String(value) => Ok(Zeroizing::new(value.into_bytes())),
        _ => anyhow::bail!("缺少解鎖密碼"),
    }
}

fn take_pin(request: &mut Value) -> Result<Option<Zeroizing<String>>> {
    match request["pin"].take() {
        Value::Null => Ok(None),
        Value::String(pin) if pin.is_empty() => Ok(None),
        Value::String(pin) => Ok(Some(Zeroizing::new(pin))),
        _ => anyhow::bail!("Invalid authenticator PIN"),
    }
}

fn require_hardware_pin(pin: &Option<Zeroizing<String>>) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    ensure!(
        pin.is_some(),
        "Linux FIDO2 operations require the security key PIN"
    );
    #[cfg(target_os = "windows")]
    let _ = pin; // The Windows WebAuthn dialog always requires user verification.
    Ok(())
}

fn check_environment() -> Result<()> {
    for key in [
        "LUKSBOX_NO_LOCK",
        "LUKSBOX_DEBUG_OPEN",
        "LUKSBOX_TOLERATE_BAD_CHUNK_LISTS",
    ] {
        ensure!(
            std::env::var_os(key).is_none(),
            "請先移除環境變數 {key}；MISF 不允許停用容器鎖、輸出解密資料或容錯覆寫"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Host;

    const FIRST: &[u8] = b"first recovery phrase";
    const SECOND: &[u8] = b"second recovery phrase";
    fn params() -> Argon2idParams {
        Argon2idParams {
            m_cost_kib: 8,
            t_cost: 1,
            p_cost: 1,
        }
    }

    fn fixture() -> (tempfile::TempDir, StorageSession) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        let store = Store::new(path.clone());
        store
            .update(|db| {
                db.hosts.push(Host {
                    id: "sample".into(),
                    name: "encrypted-name-sentinel".into(),
                    address: "192.0.2.10".into(),
                    port: 22,
                    username: "operator".into(),
                    os: "Linux".into(),
                    auth_type: "password".into(),
                    password: "ssh-secret-sentinel".into(),
                    ..Default::default()
                });
                Ok(())
            })
            .unwrap();
        (dir, StorageSession::new(path))
    }

    #[tokio::test]
    async fn migration_preserves_original_and_locked_session_never_falls_back_to_plaintext() {
        let (dir, mut session) = fixture();
        let original = fs::read(dir.path().join("hosts.json")).unwrap();
        session.enable(FIRST, params()).unwrap();
        assert!(session.status().unwrap().encrypted);
        assert!(session.status().unwrap().plaintext_exists);
        assert_eq!(fs::read(dir.path().join("hosts.json")).unwrap(), original);
        let hosts = session
            .handle(json!({"action":"list"}), &|_| {})
            .await
            .unwrap();
        assert_eq!(hosts["hosts"][0]["password"], "ssh-secret-sentinel");
        session
            .handle(json!({"action":"vault_lock"}), &|_| {})
            .await
            .unwrap();
        assert!(session.status().unwrap().locked);
        for action in [
            json!({"action":"list"}),
            json!({"action":"test","id":"sample"}),
            json!({"action":"delete","id":"sample"}),
        ] {
            assert!(session.handle(action, &|_| {}).await.is_err());
        }
        assert!(
            session
                .handle(
                    json!({"action":"vault_unlock","passphrase":"incorrect password"}),
                    &|_| {}
                )
                .await
                .is_err()
        );
        assert!(session.status().unwrap().locked);
        session
            .handle(
                json!({"action":"vault_unlock","passphrase":std::str::from_utf8(FIRST).unwrap()}),
                &|_| {},
            )
            .await
            .unwrap();
        assert_eq!(
            session
                .handle(json!({"action":"list"}), &|_| {})
                .await
                .unwrap()["hosts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn two_independent_password_slots_survive_restart_and_encrypted_backup() {
        let (dir, mut session) = fixture();
        session.enable(FIRST, params()).unwrap();
        let slot = session
            .with_vault(|vault| vault.add_key(SECOND, params()))
            .unwrap();
        assert_eq!(slot, 1);
        assert_eq!(session.status().unwrap().slots.len(), 2);
        let backup = dir.path().join("backup.lbx");
        session.backup(&backup).unwrap();
        assert!(
            session.backup(&backup).is_err(),
            "must not overwrite an existing backup"
        );
        session
            .handle(json!({"action":"vault_lock"}), &|_| {})
            .await
            .unwrap();
        drop(session);
        for passphrase in [FIRST, SECOND] {
            let mut restarted = StorageSession::new(dir.path().join("hosts.json"));
            assert!(restarted.status().unwrap().locked);
            restarted.handle(json!({"action":"vault_unlock","passphrase":std::str::from_utf8(passphrase).unwrap()}), &|_| {}).await.unwrap();
            assert_eq!(restarted.status().unwrap().slots.len(), 2);
            assert_eq!(
                restarted
                    .handle(json!({"action":"list"}), &|_| {})
                    .await
                    .unwrap()["hosts"][0]["id"],
                "sample"
            );
            let mut copied = VaultFile::open(&backup, passphrase).unwrap();
            assert_eq!(
                copied.read().unwrap().hosts[0].name,
                "encrypted-name-sentinel"
            );
        }
    }

    #[test]
    fn encrypted_updates_replace_shorter_snapshots_and_never_write_secrets_to_sidecars() {
        let (dir, mut session) = fixture();
        session.enable(FIRST, params()).unwrap();
        let original = fs::read(dir.path().join("hosts.json")).unwrap();
        let store = session.store.as_ref().unwrap();
        store
            .update(|db| {
                db.hosts[0].notes = "x".repeat(20_000);
                Ok(())
            })
            .unwrap();
        assert_eq!(store.read().unwrap().hosts[0].notes.len(), 20_000);
        store
            .update(|db| {
                db.hosts.clear();
                Ok(())
            })
            .unwrap();
        assert!(store.read().unwrap().hosts.is_empty());
        session.with_vault(|vault| vault.finish()).unwrap();
        assert_eq!(fs::read(dir.path().join("hosts.json")).unwrap(), original);
        drop(session);
        for entry in fs::read_dir(dir.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().unwrap().to_string_lossy().contains(".lbx") {
                let bytes = fs::read(path).unwrap();
                for secret in [
                    b"ssh-secret-sentinel".as_slice(),
                    b"encrypted-name-sentinel".as_slice(),
                ] {
                    assert!(!bytes.windows(secret.len()).any(|window| window == secret));
                }
            }
        }
        let mut reopened = VaultFile::open(&dir.path().join("hosts.lbx"), FIRST).unwrap();
        assert!(reopened.read().unwrap().hosts.is_empty());
    }

    #[test]
    fn tampered_file_chunk_fails_closed_and_does_not_change_original_json() {
        use std::io::{Read, Seek, SeekFrom, Write};
        let (dir, mut session) = fixture();
        let original = fs::read(dir.path().join("hosts.json")).unwrap();
        session.enable(FIRST, params()).unwrap();
        let offset = session
            .with_vault(|vault| Ok(vault.vfs.container().data_offset()))
            .unwrap();
        drop(session);
        let path = dir.path().join("hosts.lbx");
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.seek(SeekFrom::Start(offset + 24)).unwrap();
        let mut byte = [0];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x80;
        file.seek(SeekFrom::Start(offset + 24)).unwrap();
        file.write_all(&byte).unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert!(VaultFile::open(&path, FIRST).is_err());
        assert_eq!(fs::read(dir.path().join("hosts.json")).unwrap(), original);
    }

    #[test]
    fn duplicate_conversion_and_concurrent_open_cannot_overwrite_the_vault() {
        let (dir, mut session) = fixture();
        session.enable(FIRST, params()).unwrap();
        assert!(session.enable(SECOND, params()).is_err());
        assert!(VaultFile::open(&dir.path().join("hosts.lbx"), FIRST).is_err());
        assert!(
            Store::new(dir.path().join("hosts.json"))
                .update(|db| {
                    db.hosts.clear();
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(
            session.store.as_ref().unwrap().read().unwrap().hosts.len(),
            1
        );
    }

    #[test]
    fn passphrase_policy_does_not_accept_short_pin() {
        assert!(validate_passphrase(b"123456").is_err());
        assert!(validate_passphrase(FIRST).is_ok());
    }

    #[tokio::test]
    async fn revoke_password_survives_restart_and_last_slot_cannot_be_removed() {
        let (dir, mut session) = fixture();
        session.enable(FIRST, params()).unwrap();
        assert!(session.with_vault(|vault| vault.revoke_key(0)).is_err());
        session
            .with_vault(|vault| vault.add_key(SECOND, params()))
            .unwrap();
        let backup = dir.path().join("before-revoke.lbx");
        session.backup(&backup).unwrap();
        assert!(
            session
                .handle(json!({"action":"vault_revoke_key","slot":99}), &|_| {})
                .await
                .is_err()
        );
        session
            .handle(json!({"action":"vault_revoke_key","slot":0}), &|_| {})
            .await
            .unwrap();
        assert_eq!(session.status().unwrap().slots[0].index, 1);
        assert!(session.with_vault(|vault| vault.revoke_key(1)).is_err());
        drop(session);
        let path = dir.path().join("hosts.lbx");
        assert!(VaultFile::open(&path, FIRST).is_err());
        assert_eq!(
            VaultFile::open(&path, SECOND)
                .unwrap()
                .read()
                .unwrap()
                .hosts
                .len(),
            1
        );
        // An exported old copy remains usable; slot revocation is not key rotation.
        assert!(VaultFile::open(&backup, FIRST).is_ok());
    }

    #[test]
    fn real_passkey_protocol_with_mock_devices_supports_backup_unlock_and_revocation() {
        use luksbox_fido2::MockAuthenticator;
        let (dir, mut session) = fixture();
        session.enable(FIRST, params()).unwrap();
        let mut primary = MockAuthenticator::new();
        let mut backup = MockAuthenticator::new();
        for auth in [&mut primary, &mut backup] {
            session
                .with_vault(|vault| {
                    crate::passkey::enroll(vault.vfs.container_mut(), auth, None)?;
                    vault.vfs.container_mut().persist_header()?;
                    Ok(())
                })
                .unwrap();
        }
        drop(session);
        let path = dir.path().join("hosts.lbx");
        for auth in [&mut primary, &mut backup] {
            let mut vault =
                VaultFile::from_container(crate::passkey::open(&path, auth, None).unwrap())
                    .unwrap();
            assert_eq!(vault.read().unwrap().hosts[0].id, "sample");
        }
        assert!(crate::passkey::open(&path, &mut MockAuthenticator::new(), None).is_err());
        {
            let mut vault =
                VaultFile::from_container(crate::passkey::open(&path, &mut backup, None).unwrap())
                    .unwrap();
            vault.revoke_key(1).unwrap();
        }
        assert!(crate::passkey::open(&path, &mut primary, None).is_err());
        assert!(crate::passkey::open(&path, &mut backup, None).is_ok());
        assert!(VaultFile::open(&path, FIRST).is_ok());
    }

    #[test]
    fn failed_passkey_enrollment_does_not_add_a_slot() {
        use luksbox_fido2::MockAuthenticator;
        let (_dir, mut session) = fixture();
        session.enable(FIRST, params()).unwrap();
        let mut cancelled = MockAuthenticator::new();
        cancelled.simulate_no_touch();
        let mut zero_secret = MockAuthenticator::new();
        zero_secret.force_hmac_secret([0; 32]);
        let mut oversized_id = MockAuthenticator::new();
        oversized_id.force_cred_id_len(4096);
        for auth in [&mut cancelled, &mut zero_secret, &mut oversized_id] {
            assert!(
                session
                    .with_vault(|vault| crate::passkey::enroll(
                        vault.vfs.container_mut(),
                        auth,
                        None
                    ))
                    .is_err()
            );
            assert_eq!(session.status().unwrap().slots.len(), 1);
        }
    }

    #[tokio::test]
    async fn missing_vault_stays_locked_even_when_original_plaintext_still_exists() {
        let (dir, mut session) = fixture();
        session.enable(FIRST, params()).unwrap();
        session
            .handle(json!({"action":"vault_lock"}), &|_| {})
            .await
            .unwrap();
        drop(session);
        fs::rename(
            dir.path().join("hosts.lbx"),
            dir.path().join("offline-copy.lbx"),
        )
        .unwrap();
        let mut restarted = StorageSession::new(dir.path().join("hosts.json"));
        assert!(restarted.status().unwrap().encrypted);
        assert!(restarted.status().unwrap().locked);
        assert!(
            restarted
                .handle(json!({"action":"list"}), &|_| {})
                .await
                .is_err()
        );
        assert!(restarted.handle(json!({"action":"vault_unlock","passphrase":std::str::from_utf8(FIRST).unwrap()}), &|_| {}).await.is_err());
        assert!(Store::new(dir.path().join("hosts.json")).read().is_err());
    }
}
