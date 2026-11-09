//! v6: what still needs a restart after a check or upgrade (APT as root only).
use anyhow::Result;
use rusqlite::Connection;

pub(super) fn upgrade(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
CREATE TABLE restart_status (
    host_id TEXT PRIMARY KEY NOT NULL REFERENCES check_results(host_id) ON DELETE CASCADE,
    reboot_required INTEGER NOT NULL,
    needrestart INTEGER NOT NULL,
    running_kernel TEXT NOT NULL,
    expected_kernel TEXT NOT NULL
) STRICT;
CREATE TABLE restart_items (
    host_id TEXT NOT NULL REFERENCES restart_status(host_id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (host_id, kind, position)
) STRICT;
",
    )?;
    Ok(())
}
