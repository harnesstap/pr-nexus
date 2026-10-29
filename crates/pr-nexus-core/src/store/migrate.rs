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
    include_str!("../../migrations/0004_search_fold_v2.sql"),
    include_str!("../../migrations/0005_pr_overview_cache.sql"),
    include_str!("../../migrations/0006_slack_team_ids.sql"),
    include_str!("../../migrations/0007_project_info.sql"),
    include_str!("../../migrations/0008_mcp_audit.sql"),
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

    #[test]
    fn team_ids_backfill_unique_domains_only() {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..5] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 5_i64).unwrap();
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts (kind, display_name, host, external_id, slack_workspace, login, auth_mode, created_at) VALUES
               ('slack', 'acme', 'acme-xyz.slack.com', 'T0ACME0001:U0ALICE', 'acme-xyz', 'U0ALICE', 'slack_mcp_oauth', 'x'),
               ('slack', 'acme-labs', 'acme.slack.com', 'T0LABS0001:U0ALICE', 'acme', 'U0ALICE', 'slack_mcp_oauth', 'x'),
               ('slack', 'twin', 'Acme.slack.com', 'T0TWIN0001:U0ALICE', 'Acme', 'U0ALICE', 'slack_mcp_oauth', 'x');
             INSERT INTO review_requests (pr_id, slack_workspace, channel_id, channel_name, root_ts, requested_at, status, source) VALUES
               (1, 'acme-xyz', 'C0TEAM0001', 'team-web', '1.1', 'x', 'active', 'app'),
               (2, 'acme', 'C0TEAM0001', 'team-web', '1.2', 'x', 'active', 'app'),
               (3, 'gone', 'C0TEAM0001', 'team-web', '1.3', 'x', 'active', 'app');
             INSERT INTO team_channel_mappings (layer, github_org, team_slug, slack_workspace, updated_at) VALUES
               ('user', 'acme', 'web', 'acme-xyz', 'x');",
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let teams: Vec<Option<String>> = conn
            .prepare("SELECT slack_team_id FROM review_requests ORDER BY pr_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        // `acme` and `Acme` are two accounts: ambiguous, resolved at send time.
        assert_eq!(teams, [Some("T0ACME0001".to_owned()), None, None]);
        let mapping: Option<String> = conn
            .query_row(
                "SELECT slack_team_id FROM team_channel_mappings",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mapping, None, "mappings resolve on their first send");
        // Two accounts may now share a domain.
        conn.execute(
            "INSERT INTO accounts (kind, display_name, host, external_id, slack_workspace, login, auth_mode, created_at)
             VALUES ('slack', 'other', 'acme.slack.com', 'T0OTHR0001:U0ALICE', 'acme', 'U0ALICE', 'slack_mcp_oauth', 'x')",
            [],
        )
        .unwrap();
    }
}
