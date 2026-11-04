//! Slack review requests as inputs for the review statistics (§8.2).

use chrono::{DateTime, Utc};

use crate::error::CoreError;
use crate::stats::SlackRequestFact;

use super::time::{format_ts, ts_col};
use super::{Store, db};

impl Store {
    /// Every Slack review request (any status) on a pull request created at
    /// or after `since`, with its bump count.
    pub fn slack_request_facts(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<SlackRequestFact>, CoreError> {
        let since = format_ts(since);
        self.read(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT r.owner || '/' || r.name, p.number, q.channel_name, q.requested_at, q.bump_count
                     FROM review_requests q
                     JOIN pull_requests p ON p.id = q.pr_id
                     JOIN repositories r ON r.id = p.repo_id
                     WHERE p.created_at >= ?1
                     ORDER BY q.requested_at, q.id",
                )
                .map_err(db)?;
            let rows = stmt
                .query_map([since], |row| {
                    Ok(SlackRequestFact {
                        repo: row.get(0)?,
                        number: row.get::<_, i64>(1)? as u64,
                        channel: row.get(2)?,
                        requested_at: ts_col(row.get(3)?)?,
                        bump_count: row.get::<_, i64>(4)?.max(0) as u32,
                    })
                })
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }
}
