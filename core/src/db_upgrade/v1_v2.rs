//! v2: a package manager chosen in the editor ("" = detect), and LUKSbox key slot names.
use anyhow::Result;
use rusqlite::Connection;

pub(super) fn upgrade(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
ALTER TABLE hosts ADD COLUMN manager_choice TEXT NOT NULL DEFAULT '';
CREATE TABLE key_slots (
    slot INTEGER PRIMARY KEY NOT NULL,
    name TEXT NOT NULL
) STRICT;
",
    )?;
    Ok(())
}
