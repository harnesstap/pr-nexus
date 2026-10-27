//! Apply `migrations/NNNN_*.sql` in one transaction.
//!
//! `PRAGMA user_version` is the schema version. A failed migration rolls the
//! version back with the schema change.

use rusqlite::Connection;

use crate::error::CoreError;

use super::db;

const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/0001_initial.sql"),
    include_str!("../../migrations/0002_list_search.sql"),
    include_str!("../../migrations/0003_reviewer_profiles.sql"),
];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewer_profiles_upgrade_keeps_existing_requests() {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..2] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 2_i64).unwrap();
        // The row stands in for one synced before the upgrade; no PR row is needed.
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn.execute(
            "INSERT INTO pr_requested_reviewers (pr_id, kind, org, login_or_slug)
             VALUES (1, 'user', NULL, 'octo')",
            [],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        let (login, name, avatar): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT login_or_slug, name, avatar_url FROM pr_requested_reviewers",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(login, "octo");
        assert_eq!(name, None);
        assert_eq!(avatar, None);
    }
}
