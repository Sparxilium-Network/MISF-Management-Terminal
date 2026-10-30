use crate::model::Host;
use crate::vault::VaultFile;
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// The in-memory host list, also the shape of the legacy `hosts.json` format (version 1).
#[derive(Default, Deserialize, Serialize)]
pub struct Database {
    pub version: u32,
    pub hosts: Vec<Host>,
}

pub struct Store {
    pub path: PathBuf,
    pub(crate) vault: Option<Arc<Mutex<VaultFile>>>,
}

impl Store {
    pub fn default_path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("MISF_DATA_DIR") {
            return Ok(PathBuf::from(path).join("hosts.db"));
        }
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .context("找不到使用者目錄；請設定 MISF_DATA_DIR")?;
        profile_path(Path::new(&home))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path, vault: None }
    }

    pub fn is_encrypted(&self) -> bool {
        self.vault.is_some()
    }

    /// Opens the SQLite file, creating it (from a legacy `hosts.json` when present) on first use.
    fn open(&self, dir: &Path) -> Result<Connection> {
        if !self.path.exists() {
            self.create(dir)?;
        }
        // SQLite would apply these before any check could refuse the file.
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = self.path.clone().into_os_string();
            sidecar.push(suffix);
            ensure!(
                !Path::new(&sidecar).exists(),
                "主機資料庫旁有 hosts.db{suffix} 檔案，MISF 不使用 WAL；為避免改寫原始檔案，已拒絕開啟"
            );
        }
        crate::sqlite::ensure_rollback_header(&read_header(&self.path)?)?;
        let mut conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .context("無法開啟主機資料庫")?;
        // Another tool may briefly hold a SQLite lock; wait instead of failing at once.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        crate::sqlite::prepare(&mut conn, |version| {
            // The app lock is held and no transaction is open, so the file is consistent.
            let backup = self.path.with_extension(format!("v{version}.bak.db"));
            if !backup.exists() {
                fs::copy(&self.path, &backup).context("升級前無法備份主機資料庫")?;
                set_private_permissions(&backup)?;
            }
            Ok(())
        })?;
        Ok(conn)
    }

    /// Builds the new database beside the target, then moves it into place only if
    /// nothing else created one meanwhile. The legacy JSON is left untouched.
    fn create(&self, dir: &Path) -> Result<()> {
        let legacy = self.path.with_extension("json");
        let imported = if legacy.is_file() {
            Some(parse_legacy(
                &fs::read(&legacy).context("無法讀取舊版 hosts.json")?,
            )?)
        } else {
            None
        };
        let temp = tempfile::Builder::new()
            .prefix(".misf-hosts-")
            .suffix(".db")
            .tempfile_in(dir)?
            .into_temp_path();
        set_private_permissions(&temp)?;
        {
            let mut conn = Connection::open(&temp)?;
            crate::sqlite::prepare(&mut conn, |_| Ok(()))?;
            if let Some(db) = &imported {
                crate::sqlite::save(&mut conn, db).context("無法匯入舊版 hosts.json")?;
            }
        }
        if let Err(error) = temp.persist_noclobber(&self.path)
            && error.error.kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(error.error).context("無法建立主機資料庫");
        }
        Ok(())
    }

    pub fn read(&self) -> Result<Database> {
        self.with_db(
            |db| {
                Ok(Database {
                    version: db.version,
                    hosts: db.hosts.clone(),
                })
            },
            false,
        )
    }

    pub fn update<T>(&self, op: impl FnOnce(&mut Database) -> Result<T>) -> Result<T> {
        self.with_db(op, true)
    }

    pub(crate) fn with_db<T>(
        &self,
        op: impl FnOnce(&mut Database) -> Result<T>,
        write: bool,
    ) -> Result<T> {
        if let Some(vault) = &self.vault {
            let mut vault = vault
                .lock()
                .map_err(|_| anyhow::anyhow!("加密工作階段發生錯誤，請重新啟動"))?;
            let mut db = vault.read()?;
            let result = op(&mut db)?;
            if write {
                vault.write(&db)?;
            }
            return Ok(result);
        }
        let dir = self.path.parent().context("資料路徑無效")?;
        fs::create_dir_all(dir).context("無法建立資料目錄")?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.path.with_extension("lock"))?;
        lock.lock_exclusive().context("無法鎖定資料檔案")?;
        ensure!(
            !crate::vault::encrypted_profile_exists(&self.path),
            "此主機庫已啟用加密；請重新啟動並解鎖，禁止回退明文儲存"
        );
        let mut conn = self.open(dir)?;
        let mut db = crate::sqlite::load(&conn)?;
        let result = op(&mut db)?;
        if write {
            crate::sqlite::save(&mut conn, &db)?;
        }
        Ok(result)
    }

    /// "sqlite" with the file's schema version, or "json" for a not yet converted store.
    pub fn format(&self) -> Result<(&'static str, Option<u32>)> {
        if let Some(vault) = &self.vault {
            return vault
                .lock()
                .map_err(|_| anyhow::anyhow!("加密工作階段發生錯誤，請重新啟動"))?
                .format();
        }
        if !self.path.exists() {
            let legacy = self.path.with_extension("json").exists();
            return Ok((if legacy { "json" } else { "sqlite" }, None));
        }
        Ok((
            "sqlite",
            crate::sqlite::header_version(&read_header(&self.path)?),
        ))
    }

    pub fn host(&self, id: &str) -> Result<Host> {
        self.read()?
            .hosts
            .into_iter()
            .find(|h| h.id == id)
            .context("找不到主機；可能已被刪除")
    }
}

/// Reads the JSON format used before SQLite; the file itself is never modified.
/// The 100-byte SQLite file header, or less for a shorter file.
fn read_header(path: &Path) -> Result<Vec<u8>> {
    let mut header = Vec::with_capacity(100);
    std::io::Read::read_to_end(
        &mut std::io::Read::take(fs::File::open(path)?, 100),
        &mut header,
    )?;
    Ok(header)
}

pub(crate) fn parse_legacy(bytes: &[u8]) -> Result<Database> {
    let db: Database =
        serde_json::from_slice(bytes).context("舊版主機資料格式錯誤；原始檔案已保留")?;
    ensure!(db.version == 1, "不支援的舊版主機資料版本");
    Ok(db)
}

fn profile_path(home: &Path) -> Result<PathBuf> {
    let database = home.join(".misf-management").join("hosts.db");
    let path = database.with_extension("json");
    // Preserve connection profiles created before the project-name correction;
    // the store imports the copied JSON into SQLite on first use.
    let legacy = home.join(".msif-management").join("hosts.json");
    if !database.exists()
        && !path.exists()
        && !crate::vault::encrypted_profile_exists(&path)
        && legacy.is_file()
    {
        let directory = path.parent().unwrap();
        fs::create_dir_all(directory).context("無法建立 MISF 資料目錄")?;
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        set_private_permissions(temp.path())?;
        let mut source = fs::File::open(legacy).context("無法讀取舊版主機資料")?;
        std::io::copy(&mut source, &mut temp)?;
        temp.as_file().sync_all()?;
        if let Err(error) = temp.persist_noclobber(&path)
            && error.error.kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(error.error).context("無法遷移舊版主機資料");
        }
    }
    Ok(database)
}

fn set_private_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_profile_is_copied_once_without_overwriting_existing_data() {
        let home = tempfile::tempdir().unwrap();
        let legacy = home.path().join(".msif-management");
        fs::create_dir(&legacy).unwrap();
        let original = br#"{"version":1,"hosts":[]}"#;
        fs::write(legacy.join("hosts.json"), original).unwrap();
        let database = profile_path(home.path()).unwrap();
        assert_eq!(database.file_name().unwrap(), "hosts.db");
        let migrated = database.with_extension("json");
        assert_eq!(fs::read(&migrated).unwrap(), original);
        assert_eq!(fs::read(legacy.join("hosts.json")).unwrap(), original);
        fs::write(&migrated, "existing-new-profile").unwrap();
        profile_path(home.path()).unwrap();
        assert_eq!(fs::read(&migrated).unwrap(), b"existing-new-profile");
    }

    #[test]
    fn legacy_json_is_imported_into_sqlite_and_kept_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("hosts.json");
        let original = br#"{"version":1,"hosts":[{"id":"x","name":"old","port":22,"password":"p","last_check":{"status":"success","packages":[{"name":"curl","current_version":"1","candidate_version":"2","architecture":"amd64","repository":"main"}]}}]}"#;
        fs::write(&json, original).unwrap();
        let store = Store::new(dir.path().join("hosts.db"));
        let host = store.host("x").unwrap();
        assert_eq!(host.last_check.unwrap().packages[0].name, "curl");
        assert_eq!(fs::read(&json).unwrap(), original);
        assert_eq!(store.format().unwrap(), ("sqlite", Some(1)));
        // Later edits go to SQLite only.
        store
            .update(|db| {
                db.hosts.clear();
                Ok(())
            })
            .unwrap();
        assert!(store.read().unwrap().hosts.is_empty());
        assert_eq!(fs::read(&json).unwrap(), original);
    }

    #[test]
    fn damaged_legacy_json_creates_no_database() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hosts.json"), "{broken").unwrap();
        let store = Store::new(dir.path().join("hosts.db"));
        assert!(store.read().is_err());
        assert!(!dir.path().join("hosts.db").exists());
    }

    #[test]
    fn wal_databases_are_refused_before_sqlite_replays_or_rewrites_them() {
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("hosts.db");
        Store::new(path.clone()).read().unwrap();
        // The WAL adds a trigger that the main file lacks.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA wal_autocheckpoint = 0;
             CREATE TABLE loot (secret TEXT);
             CREATE TRIGGER steal AFTER INSERT ON hosts BEGIN
                 INSERT INTO loot VALUES (new.password);
             END;",
        )
        .unwrap();
        let with_sidecar = tempfile::tempdir().unwrap();
        for name in ["hosts.db", "hosts.db-wal"] {
            fs::copy(source.path().join(name), with_sidecar.path().join(name)).unwrap();
        }
        drop(conn);
        // Closing checkpoints the WAL; the main file keeps its WAL-mode header.
        assert!(!source.path().join("hosts.db-wal").exists());
        for (dir, message) in [(&with_sidecar, "-wal"), (&source, "WAL 模式")] {
            let files = |dir: &Path| {
                let mut names: Vec<_> = fs::read_dir(dir)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| path.extension().is_none_or(|ext| ext != "lock"))
                    .map(|path| (path.clone(), fs::read(path).unwrap()))
                    .collect();
                names.sort();
                names
            };
            let before = files(dir.path());
            let error = format!(
                "{:#}",
                Store::new(dir.path().join("hosts.db"))
                    .read()
                    .err()
                    .unwrap()
            );
            assert!(files(dir.path()) == before, "refused files were rewritten");
            assert!(error.contains(message), "{error}");
        }
    }
}
