//! SQLite host database. The schema version lives in the file header's `user_version`
//! slot and `application_id` marks the file as MISF's, so opening or importing an older
//! database detects its version and upgrades it, while foreign or newer files are refused.
use crate::model::{CheckResult, Host, Package};
use crate::store::Database;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, MAIN_DB, config::DbConfig, limits::Limit, params};
use std::collections::HashMap;
use zeroize::Zeroizing;

/// "MISF" in ASCII.
pub const APPLICATION_ID: i32 = 0x4D49_5346;
/// `MIGRATIONS[n]` upgrades schema version n to n + 1; version 0 is an empty file.
/// Append new versions; never edit a released one, because opened files are compared
/// against the schema these statements produce.
const MIGRATIONS: &[&str] = &[V1];
pub const SCHEMA_VERSION: usize = MIGRATIONS.len();

const V1: &str = "
CREATE TABLE hosts (
    id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    address TEXT NOT NULL,
    port INTEGER NOT NULL,
    username TEXT NOT NULL,
    os TEXT NOT NULL,
    auth_type TEXT NOT NULL,
    password TEXT NOT NULL,
    private_key TEXT NOT NULL,
    key_passphrase TEXT NOT NULL,
    sudo_password TEXT NOT NULL,
    use_pty INTEGER NOT NULL,
    fingerprint TEXT NOT NULL,
    notes TEXT NOT NULL,
    distro TEXT NOT NULL,
    package_manager TEXT NOT NULL
) STRICT;
CREATE TABLE check_results (
    host_id TEXT PRIMARY KEY NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    checked_at TEXT NOT NULL,
    operation TEXT NOT NULL,
    status TEXT NOT NULL,
    distro TEXT NOT NULL,
    package_manager TEXT NOT NULL,
    output TEXT NOT NULL,
    error TEXT NOT NULL
) STRICT;
CREATE TABLE packages (
    host_id TEXT NOT NULL REFERENCES check_results(host_id) ON DELETE CASCADE,
    upgraded INTEGER NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    current_version TEXT NOT NULL,
    candidate_version TEXT NOT NULL,
    architecture TEXT NOT NULL,
    repository TEXT NOT NULL,
    PRIMARY KEY (host_id, upgraded, position)
) STRICT;
";

/// Nothing here needs triggers, views, attached files or a trusted schema, so a crafted
/// database cannot run SQL of its own while it is read or written.
fn harden(conn: &Connection) -> Result<()> {
    for (config, enabled) in [
        (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true),
        (DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false),
    ] {
        conn.set_db_config(config, enabled)?;
    }
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    // Rollback journal, not WAL: no long-lived -wal/-shm copies of credential pages.
    conn.pragma_update_and_check(None, "journal_mode", "DELETE", |_| Ok(()))?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA secure_delete = ON;
         PRAGMA temp_store = MEMORY;
         PRAGMA mmap_size = 0;
         PRAGMA cell_size_check = ON;",
    )?;
    Ok(())
}

/// Hardens the connection, refuses foreign, newer, damaged or altered files, then
/// upgrades older ones in one transaction after `before_upgrade(old_version)`.
pub(crate) fn prepare(
    conn: &mut Connection,
    before_upgrade: impl FnOnce(usize) -> Result<()>,
) -> Result<()> {
    prepare_with(conn, MIGRATIONS, before_upgrade)
}

fn prepare_with(
    conn: &mut Connection,
    migrations: &[&str],
    before_upgrade: impl FnOnce(usize) -> Result<()>,
) -> Result<()> {
    harden(conn).context("無法開啟主機資料庫；檔案可能不是 SQLite 資料庫")?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .context("無法讀取主機資料庫；檔案可能損壞")?;
    ensure!(check == "ok", "主機資料庫損壞；原始檔案已保留");
    let application: i32 = conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(
        application == APPLICATION_ID || (application == 0 && version == 0),
        "不是 MISF 主機資料庫"
    );
    let version = usize::try_from(version)
        .ok()
        .filter(|v| *v <= migrations.len())
        .with_context(|| {
            format!(
                "主機資料庫為 v{version}，需要較新版 MISF 才能開啟；此版本支援至 v{}",
                migrations.len()
            )
        })?;
    ensure!(
        schema(conn)? == reference_schema(&migrations[..version])?,
        "主機資料庫結構與 v{version} 不符，可能不是 MISF 資料庫或已遭竄改；原始檔案已保留"
    );
    if version < migrations.len() {
        if version > 0 {
            before_upgrade(version)?;
        }
        let tx = conn.transaction()?;
        for sql in &migrations[version..] {
            tx.execute_batch(sql)?;
        }
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        tx.pragma_update(None, "user_version", migrations.len() as i64)?;
        tx.commit().context("無法升級主機資料庫；原始資料未變更")?;
    }
    Ok(())
}

type SchemaRow = (String, String, String, Option<String>);

fn schema(conn: &Connection) -> Result<Vec<SchemaRow>> {
    let mut statement =
        conn.prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY type, name")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The exact schema a genuine file at this version has; anything extra, such as a
/// trigger or view planted in an imported file, makes the comparison fail.
fn reference_schema(migrations: &[&str]) -> Result<Vec<SchemaRow>> {
    let conn = Connection::open_in_memory()?;
    for sql in migrations {
        conn.execute_batch(sql)?;
    }
    schema(&conn)
}

pub(crate) fn load(conn: &Connection) -> Result<Database> {
    let mut packages: HashMap<(String, bool), Vec<Package>> = HashMap::new();
    let mut statement = conn.prepare(
        "SELECT host_id, upgraded, name, current_version, candidate_version, architecture, repository
         FROM packages ORDER BY host_id, upgraded, position",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        packages
            .entry((row.get(0)?, row.get(1)?))
            .or_default()
            .push(Package {
                name: row.get(2)?,
                current_version: row.get(3)?,
                candidate_version: row.get(4)?,
                architecture: row.get(5)?,
                repository: row.get(6)?,
            });
    }
    let mut results = HashMap::new();
    let mut statement = conn.prepare(
        "SELECT host_id, checked_at, operation, status, distro, package_manager, output, error
         FROM check_results",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let result = CheckResult {
            checked_at: row.get(1)?,
            operation: row.get(2)?,
            status: row.get(3)?,
            distro: row.get(4)?,
            package_manager: row.get(5)?,
            packages: packages.remove(&(id.clone(), false)).unwrap_or_default(),
            upgraded: packages.remove(&(id.clone(), true)).unwrap_or_default(),
            output: row.get(6)?,
            error: row.get(7)?,
        };
        results.insert(id, result);
    }
    let mut statement = conn.prepare(
        "SELECT id, name, address, port, username, os, auth_type, password, private_key,
                key_passphrase, sudo_password, use_pty, fingerprint, notes, distro, package_manager
         FROM hosts ORDER BY position",
    )?;
    let mut rows = statement.query([])?;
    let mut hosts = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        hosts.push(Host {
            last_check: results.remove(&id),
            id,
            name: row.get(1)?,
            address: row.get(2)?,
            port: row.get(3)?,
            username: row.get(4)?,
            os: row.get(5)?,
            auth_type: row.get(6)?,
            password: row.get(7)?,
            private_key: row.get(8)?,
            key_passphrase: row.get(9)?,
            sudo_password: row.get(10)?,
            use_pty: row.get(11)?,
            fingerprint: row.get(12)?,
            notes: row.get(13)?,
            distro: row.get(14)?,
            package_manager: row.get(15)?,
        });
    }
    Ok(Database { version: 1, hosts })
}

/// Replaces the stored hosts in one transaction; secure_delete zeroes the old rows.
pub(crate) fn save(conn: &mut Connection, db: &Database) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch("DELETE FROM packages; DELETE FROM check_results; DELETE FROM hosts;")?;
    {
        let mut host_row = tx.prepare(
            "INSERT INTO hosts (id, position, name, address, port, username, os, auth_type,
                 password, private_key, key_passphrase, sudo_password, use_pty, fingerprint,
                 notes, distro, package_manager)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        )?;
        let mut result_row = tx.prepare(
            "INSERT INTO check_results (host_id, checked_at, operation, status, distro,
                 package_manager, output, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let mut package_row = tx.prepare(
            "INSERT INTO packages (host_id, upgraded, position, name, current_version,
                 candidate_version, architecture, repository)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for (position, host) in db.hosts.iter().enumerate() {
            host_row
                .execute(params![
                    host.id,
                    position as i64,
                    host.name,
                    host.address,
                    host.port,
                    host.username,
                    host.os,
                    host.auth_type,
                    host.password,
                    host.private_key,
                    host.key_passphrase,
                    host.sudo_password,
                    host.use_pty,
                    host.fingerprint,
                    host.notes,
                    host.distro,
                    host.package_manager,
                ])
                .with_context(|| format!("無法寫入主機 {}；主機 ID 可能重複", host.name))?;
            let Some(result) = &host.last_check else {
                continue;
            };
            result_row.execute(params![
                host.id,
                result.checked_at,
                result.operation,
                result.status,
                result.distro,
                result.package_manager,
                result.output,
                result.error,
            ])?;
            for (upgraded, list) in [(false, &result.packages), (true, &result.upgraded)] {
                for (position, package) in list.iter().enumerate() {
                    package_row.execute(params![
                        host.id,
                        upgraded,
                        position as i64,
                        package.name,
                        package.current_version,
                        package.candidate_version,
                        package.architecture,
                        package.repository,
                    ])?;
                }
            }
        }
    }
    tx.commit().context("無法儲存主機資料庫")?;
    Ok(())
}

/// Builds a fresh database image for the vault; deleted rows never carry over into it.
pub(crate) fn to_image(db: &Database) -> Result<Zeroizing<Vec<u8>>> {
    let mut conn = Connection::open_in_memory()?;
    prepare(&mut conn, |_| Ok(()))?;
    save(&mut conn, db)?;
    let image = conn.serialize(MAIN_DB)?;
    Ok(Zeroizing::new(image.to_vec()))
}

/// Opens a vault image in memory only, upgrading it like a file on disk.
pub(crate) fn from_image(bytes: &[u8]) -> Result<Database> {
    let mut conn = Connection::open_in_memory()?;
    conn.deserialize_read_exact(MAIN_DB, bytes, bytes.len(), false)
        .context("無法讀取加密主機資料庫")?;
    prepare(&mut conn, |_| Ok(()))?;
    load(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Database {
        Database {
            version: 1,
            hosts: vec![
                Host {
                    id: "b".into(),
                    name: "second-first".into(),
                    port: 2222,
                    use_pty: true,
                    password: "secret".into(),
                    last_check: Some(CheckResult {
                        status: "success".into(),
                        packages: vec![
                            Package {
                                name: "zlib".into(),
                                ..Default::default()
                            },
                            Package {
                                name: "acl".into(),
                                ..Default::default()
                            },
                        ],
                        upgraded: vec![Package {
                            name: "openssl".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Host {
                    id: "a".into(),
                    name: "plain".into(),
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn image_round_trip_keeps_order_results_and_version_slot() {
        let image = to_image(&sample()).unwrap();
        let db = from_image(&image).unwrap();
        assert_eq!(
            serde_json::to_value(&db).unwrap(),
            serde_json::to_value(sample()).unwrap()
        );
        let mut conn = Connection::open_in_memory().unwrap();
        conn.deserialize_read_exact(MAIN_DB, &image[..], image.len(), true)
            .unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let application: i32 = conn
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .unwrap();
        assert_eq!(
            (version, application),
            (SCHEMA_VERSION as i64, APPLICATION_ID)
        );
    }

    fn image_with(sql: &str) -> Zeroizing<Vec<u8>> {
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn, |_| Ok(())).unwrap();
        // A test author, unlike an attacker's file, may still create objects.
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, false)
            .unwrap();
        conn.execute_batch(sql).unwrap();
        Zeroizing::new(conn.serialize(MAIN_DB).unwrap().to_vec())
    }

    #[test]
    fn foreign_newer_damaged_and_altered_databases_are_refused() {
        let foreign = {
            let mut conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE notes (body TEXT)")
                .unwrap();
            conn.serialize(MAIN_DB).unwrap().to_vec()
        };
        let newer = image_with("PRAGMA user_version = 99");
        let other_app = image_with("PRAGMA application_id = 1");
        // Triggers could copy credentials elsewhere whenever MISF writes a host.
        let trigger = image_with(
            "CREATE TABLE loot (secret TEXT);
             CREATE TRIGGER steal AFTER INSERT ON hosts BEGIN
                 INSERT INTO loot VALUES (new.password);
             END;",
        );
        let view = image_with("CREATE VIEW everything AS SELECT * FROM hosts");
        let index = image_with("CREATE INDEX extra ON hosts(name)");
        for (image, message) in [
            (&foreign[..], "結構"),
            (&newer[..], "v99"),
            (&other_app[..], "不是 MISF"),
            (&trigger[..], "結構"),
            (&view[..], "結構"),
            (&index[..], "結構"),
            (&b"damaged-data, not a database"[..], ""),
        ] {
            let error = format!("{:#}", from_image(image).err().unwrap());
            assert!(error.contains(message), "{error}");
        }
    }

    #[test]
    fn older_versions_upgrade_once_after_the_backup_hook() {
        // A stand-in v2 shows the path every future migration takes.
        let v2 = "ALTER TABLE hosts ADD COLUMN tags TEXT NOT NULL DEFAULT ''";
        let image = to_image(&sample()).unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        conn.deserialize_read_exact(MAIN_DB, &image[..], image.len(), false)
            .unwrap();
        let mut seen = None;
        prepare_with(&mut conn, &[V1, v2], |version| {
            seen = Some(version);
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, Some(1));
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        assert_eq!(load(&conn).unwrap().hosts.len(), 2);
        // Already current: no second upgrade, and this build refuses the newer file.
        prepare_with(&mut conn, &[V1, v2], |_| panic!("upgraded twice")).unwrap();
        assert!(prepare(&mut conn, |_| Ok(())).is_err());
        // A failed backup leaves the file at its old version.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.deserialize_read_exact(MAIN_DB, &image[..], image.len(), false)
            .unwrap();
        assert!(prepare_with(&mut conn, &[V1, v2], |_| anyhow::bail!("disk full")).is_err());
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn deleted_hosts_do_not_survive_in_a_rewritten_image() {
        let mut db = sample();
        let before = to_image(&db).unwrap();
        assert!(before.windows(6).any(|w| w == b"secret"));
        db.hosts.remove(0);
        let after = to_image(&db).unwrap();
        assert!(!after.windows(6).any(|w| w == b"secret"));
    }
}
