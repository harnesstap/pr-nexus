//! The MCP audit log (`mcp_audit`): append, read the newest, clear.

use chrono::{DateTime, Duration, Utc};
use rusqlite::params;

use crate::error::CoreError;

use super::time::{format_ts, ts_col};
use super::{Store, db};

/// At most this many rows are kept...
pub const AUDIT_KEEP_ROWS: i64 = 5_000;
/// ...and none older than this.
pub const AUDIT_KEEP_DAYS: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    Allowed,
    Denied,
    Error,
}

impl AuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Error => "error",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "allowed" => Some(Self::Allowed),
            "denied" => Some(Self::Denied),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: i64,
    pub at: DateTime<Utc>,
    pub client_name: String,
    pub tool: String,
    pub capability: String,
    pub outcome: AuditOutcome,
    /// Argument names, comma-separated.
    pub fields: String,
    /// Ids the call named, comma-separated (`project:3`, `pr:482`).
    pub target_ids: String,
    pub summary: String,
    pub error: Option<String>,
    pub duration_ms: i64,
    /// The Slack message a successful write touched; `None` otherwise.
    pub link: Option<String>,
}

impl Store {
    /// Appends `entry` (its `id` is ignored) and trims the log to the
    /// retention bound. Returns the new row id.
    pub fn append_mcp_audit(&self, entry: AuditEntry) -> Result<i64, CoreError> {
        self.write(move |conn| {
            let tx = conn.transaction().map_err(db)?;
            let id = insert(&tx, &entry)?;
            let cutoff = format_ts(entry.at - Duration::days(AUDIT_KEEP_DAYS));
            tx.execute("DELETE FROM mcp_audit WHERE at < ?1", [cutoff])
                .map_err(db)?;
            tx.execute(
                "DELETE FROM mcp_audit WHERE id <= (SELECT id FROM mcp_audit \
                 ORDER BY id DESC LIMIT 1 OFFSET ?1)",
                [AUDIT_KEEP_ROWS],
            )
            .map_err(db)?;
            tx.commit().map_err(db)?;
            Ok(id)
        })
    }

    /// Test fixture: inserts `entries` in one transaction, without trimming,
    /// so a test can fill the log to its bound without one commit per row.
    #[cfg(any(test, feature = "test-support"))]
    pub fn seed_mcp_audit(&self, entries: Vec<AuditEntry>) -> Result<(), CoreError> {
        self.write(move |conn| {
            let tx = conn.transaction().map_err(db)?;
            for entry in &entries {
                insert(&tx, entry)?;
            }
            tx.commit().map_err(db)
        })
    }

    /// Newest first, at most `limit`, optionally one outcome only.
    pub fn mcp_audit(
        &self,
        outcome: Option<AuditOutcome>,
        limit: usize,
    ) -> Result<Vec<AuditEntry>, CoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.read(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, at, client_name, tool, capability, outcome, fields, target_ids, \
                     summary, error, duration_ms, link FROM mcp_audit \
                     WHERE ?1 IS NULL OR outcome = ?1 ORDER BY id DESC LIMIT ?2",
                )
                .map_err(db)?;
            let rows = stmt
                .query_map(params![outcome.map(AuditOutcome::as_str), limit], |row| {
                    let outcome: String = row.get(5)?;
                    Ok(AuditEntry {
                        id: row.get(0)?,
                        at: ts_col(row.get(1)?)?,
                        client_name: row.get(2)?,
                        tool: row.get(3)?,
                        capability: row.get(4)?,
                        outcome: AuditOutcome::parse(&outcome).unwrap_or(AuditOutcome::Error),
                        fields: row.get(6)?,
                        target_ids: row.get(7)?,
                        summary: row.get(8)?,
                        error: row.get(9)?,
                        duration_ms: row.get(10)?,
                        link: row.get(11)?,
                    })
                })
                .map_err(db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db)
        })
    }

    /// Empties the log. Returns how many rows were removed.
    pub fn clear_mcp_audit(&self) -> Result<usize, CoreError> {
        self.write(|conn| conn.execute("DELETE FROM mcp_audit", []).map_err(db))
    }
}

fn insert(conn: &rusqlite::Connection, entry: &AuditEntry) -> Result<i64, CoreError> {
    conn.execute(
        "INSERT INTO mcp_audit (at, client_name, tool, capability, outcome, fields, \
         target_ids, summary, error, duration_ms, link) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            format_ts(entry.at),
            entry.client_name,
            entry.tool,
            entry.capability,
            entry.outcome.as_str(),
            entry.fields,
            entry.target_ids,
            entry.summary,
            entry.error,
            entry.duration_ms,
            entry.link,
        ],
    )
    .map_err(db)?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at: DateTime<Utc>, tool: &str) -> AuditEntry {
        AuditEntry {
            id: 0,
            at,
            client_name: "acme-agent".into(),
            tool: tool.into(),
            capability: "reads".into(),
            outcome: AuditOutcome::Allowed,
            fields: String::new(),
            target_ids: String::new(),
            summary: String::new(),
            error: None,
            duration_ms: 1,
            link: None,
        }
    }

    #[test]
    fn old_rows_are_trimmed_and_the_log_reads_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let now = Utc::now();
        store
            .append_mcp_audit(entry(now - Duration::days(AUDIT_KEEP_DAYS + 1), "old"))
            .unwrap();
        store.append_mcp_audit(entry(now, "a")).unwrap();
        let mut denied = entry(now, "b");
        denied.outcome = AuditOutcome::Denied;
        store.append_mcp_audit(denied).unwrap();
        let all = store.mcp_audit(None, 10).unwrap();
        assert_eq!(
            all.iter().map(|row| row.tool.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        let mut sent = entry(now, "slack_send_message");
        sent.link = Some("https://acme.slack.com/archives/C0123456789/p1700000000000100".into());
        store.append_mcp_audit(sent).unwrap();
        assert_eq!(
            store.mcp_audit(None, 1).unwrap()[0].link.as_deref(),
            Some("https://acme.slack.com/archives/C0123456789/p1700000000000100")
        );
        let only = store.mcp_audit(Some(AuditOutcome::Denied), 10).unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(store.clear_mcp_audit().unwrap(), 3);
        assert!(store.mcp_audit(None, 10).unwrap().is_empty());
    }

    #[test]
    fn the_log_keeps_at_most_the_row_bound() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let now = Utc::now();
        // Fill to the bound in one commit; the next appends go through trimming.
        store
            .seed_mcp_audit(
                (0..AUDIT_KEEP_ROWS)
                    .map(|index| entry(now, &format!("t{index}")))
                    .collect(),
            )
            .unwrap();
        for index in AUDIT_KEEP_ROWS..(AUDIT_KEEP_ROWS + 3) {
            store
                .append_mcp_audit(entry(now, &format!("t{index}")))
                .unwrap();
        }
        let rows = store.mcp_audit(None, 10_000).unwrap();
        assert_eq!(rows.len() as i64, AUDIT_KEEP_ROWS);
        assert_eq!(rows[0].tool, format!("t{}", AUDIT_KEEP_ROWS + 2));
        assert_eq!(rows.last().unwrap().tool, "t3", "the three oldest went");
    }
}
