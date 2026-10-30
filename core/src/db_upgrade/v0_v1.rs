//! v1: hosts, their last check result and its package lists.
use anyhow::Result;
use rusqlite::Connection;

pub(super) fn upgrade(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
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
",
    )?;
    Ok(())
}
