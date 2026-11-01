//! v3: SSH keys saved once and shared by hosts, which name one in `key_id` ("" = inline key).
use anyhow::Result;
use rusqlite::Connection;

pub(super) fn upgrade(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
CREATE TABLE ssh_keys (
    id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    algorithm TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    private_key TEXT NOT NULL,
    passphrase TEXT NOT NULL
) STRICT;
ALTER TABLE hosts ADD COLUMN key_id TEXT NOT NULL DEFAULT '';
",
    )?;
    Ok(())
}
