//! Apply `migrations/NNNN_*.sql` in one transaction.
//!
//! `PRAGMA user_version` is the schema version. A failed migration rolls the
//! version back with the schema change.

use rusqlite::Connection;

use crate::error::CoreError;

use super::db;

const MIGRATIONS: &[&str] = &[include_str!("../../migrations/0001_initial.sql")];

pub(super) fn migrate(conn: &mut Connection) -> Result<(), CoreError> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(db)?;
    if current < 0 {
        return Err(CoreError::Store(format!(
            "invalid schema version {current}"
        )));
    }
    let current = usize::try_from(current)
        .map_err(|_| CoreError::Store(format!("invalid schema version {current}")))?;
    if current > MIGRATIONS.len() {
        return Err(CoreError::Store(format!(
            "database schema version {current} is newer than this build ({})",
            MIGRATIONS.len()
        )));
    }
    if current == MIGRATIONS.len() {
        return Ok(());
    }

    let tx = conn.transaction().map_err(db)?;
    for (offset, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        tx.execute_batch(sql).map_err(db)?;
        tx.pragma_update(None, "user_version", (offset + 1) as i64)
            .map_err(db)?;
    }
    tx.commit().map_err(db)?;
    Ok(())
}
