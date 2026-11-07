//! v5: each host's last "test connection" run: negotiated algorithms and what was detected.
use anyhow::Result;
use rusqlite::Connection;

pub(super) fn upgrade(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
CREATE TABLE connection_tests (
    host_id TEXT PRIMARY KEY NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    tested_at TEXT NOT NULL,
    status TEXT NOT NULL,
    error TEXT NOT NULL,
    server_version TEXT NOT NULL,
    kex TEXT NOT NULL,
    host_key_algorithm TEXT NOT NULL,
    host_key_fingerprint TEXT NOT NULL,
    cipher TEXT NOT NULL,
    client_mac TEXT NOT NULL,
    server_mac TEXT NOT NULL,
    strict_kex INTEGER NOT NULL,
    auth_method TEXT NOT NULL,
    client_key_algorithm TEXT NOT NULL,
    client_key_fingerprint TEXT NOT NULL,
    kernel TEXT NOT NULL,
    distro TEXT NOT NULL,
    package_managers TEXT NOT NULL
) STRICT;
",
    )?;
    Ok(())
}
