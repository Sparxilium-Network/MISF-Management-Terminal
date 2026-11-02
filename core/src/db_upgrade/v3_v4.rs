//! v4: named host groups in the sidebar; hosts name one in `group_id` ("" = ungrouped).
use anyhow::Result;
use rusqlite::Connection;

pub(super) fn upgrade(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
CREATE TABLE host_groups (
    id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    collapsed INTEGER NOT NULL
) STRICT;
ALTER TABLE hosts ADD COLUMN group_id TEXT NOT NULL DEFAULT '';
",
    )?;
    Ok(())
}
