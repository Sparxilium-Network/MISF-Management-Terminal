//! One file per schema step: `vN_vM.rs` upgrades version N to M. A file at version 3
//! runs v3_v4, v4_v5, … in order inside one transaction, so any older version reaches
//! the current one. Append new steps; never edit a released one, because opened files
//! are compared against the schema these steps produce.
use anyhow::Result;
use rusqlite::Connection;

mod v0_v1;
mod v1_v2;

pub(crate) type Migration = fn(&Connection) -> Result<()>;

/// `MIGRATIONS[n]` upgrades schema version n to n + 1; version 0 is an empty file.
pub(crate) const MIGRATIONS: &[Migration] = &[v0_v1::upgrade, v1_v2::upgrade];
