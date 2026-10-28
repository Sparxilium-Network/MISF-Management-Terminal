use crate::model::Host;
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize, Serialize)]
pub struct Database {
    pub version: u32,
    pub hosts: Vec<Host>,
}

pub struct Store {
    pub path: PathBuf,
}

impl Store {
    pub fn default_path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("MISF_DATA_DIR") {
            return Ok(PathBuf::from(path).join("hosts.json"));
        }
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .context("找不到使用者目錄；請設定 MISF_DATA_DIR")?;
        profile_path(Path::new(&home))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> Result<Database> {
        if !self.path.exists() {
            return Ok(Database {
                version: 1,
                hosts: vec![],
            });
        }
        let bytes = fs::read(&self.path).context("無法讀取主機資料")?;
        let db: Database =
            serde_json::from_slice(&bytes).context("主機資料格式錯誤；原始檔案已保留")?;
        ensure!(db.version == 1, "不支援的主機資料版本");
        Ok(db)
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

    fn with_db<T>(&self, op: impl FnOnce(&mut Database) -> Result<T>, write: bool) -> Result<T> {
        let dir = self.path.parent().context("資料路徑無效")?;
        fs::create_dir_all(dir).context("無法建立資料目錄")?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.path.with_extension("lock"))?;
        lock.lock_exclusive().context("無法鎖定資料檔案")?;
        let mut db = self.load()?;
        let result = op(&mut db)?;
        if write {
            let mut temp = tempfile::NamedTempFile::new_in(dir)?;
            set_private_permissions(temp.path())?;
            temp.write_all(&serde_json::to_vec_pretty(&db)?)?;
            temp.as_file().sync_all()?;
            temp.persist(&self.path)
                .map_err(|e| e.error)
                .context("無法儲存主機資料")?;
        }
        Ok(result)
    }

    pub fn host(&self, id: &str) -> Result<Host> {
        self.read()?
            .hosts
            .into_iter()
            .find(|h| h.id == id)
            .context("找不到主機；可能已被刪除")
    }
}

fn profile_path(home: &Path) -> Result<PathBuf> {
    let path = home.join(".misf-management").join("hosts.json");
    // Preserve connection profiles created before the project-name correction.
    let legacy = home.join(".msif-management").join("hosts.json");
    if !path.exists() && legacy.is_file() {
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
    Ok(path)
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
        let migrated = profile_path(home.path()).unwrap();
        assert_eq!(fs::read(&migrated).unwrap(), original);
        assert_eq!(fs::read(legacy.join("hosts.json")).unwrap(), original);
        fs::write(&migrated, "existing-new-profile").unwrap();
        profile_path(home.path()).unwrap();
        assert_eq!(fs::read(&migrated).unwrap(), b"existing-new-profile");
    }
}
