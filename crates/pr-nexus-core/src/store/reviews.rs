use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{
    AccountId, CloseReason, DiscoveryOutcome, DiscoveryRun, PrId, ProviderRef, RequestSource,
    RequestStatus, ReviewMessage, ReviewMessageKind, ReviewMessageOrigin, ReviewMessagePr,
    ReviewRequest, ReviewRequestId, SendAttempt, SendOutcome,
};
use crate::error::CoreError;

use super::row::{conv, parse_enum};
use super::time::{format_ts, opt_ts_col, ts_col};
use super::{Store, db};

impl Store {
    pub fn insert_review_message(
        &self,
        message: ReviewMessage,
    ) -> Result<ReviewMessage, CoreError> {
        self.write(move |conn| insert_message(conn, &message))
    }

    pub fn get_review_message(&self, id: i64) -> Result<Option<ReviewMessage>, CoreError> {
        self.read(move |conn| load_message(conn, id))
    }

    pub fn delete_review_message(&self, id: i64) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute("DELETE FROM review_messages WHERE id = ?1", [id])
                .map_err(db)?;
            Ok(())
        })
    }

    pub fn insert_review_message_pr(&self, link: ReviewMessagePr) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO review_message_prs (message_id, pr_id, head_sha, teams_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    link.message_id,
                    link.pr_id.0,
                    link.head_sha,
                    link.teams_json
                ],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn list_review_message_prs(
        &self,
        message_id: i64,
    ) -> Result<Vec<ReviewMessagePr>, CoreError> {
        self.read(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM review_message_prs WHERE message_id = ?1 ORDER BY pr_id")
                .map_err(db)?;
            let rows = stmt
                .query_map([message_id], map_message_pr)
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }

    pub fn insert_review_request(
        &self,
        request: ReviewRequest,
    ) -> Result<ReviewRequest, CoreError> {
        self.write(move |conn| insert_request(conn, &request))
    }

    pub fn get_review_request(
        &self,
        id: ReviewRequestId,
    ) -> Result<Option<ReviewRequest>, CoreError> {
        self.read(move |conn| load_request(conn, id))
    }

    pub fn list_review_requests(&self, pr: PrId) -> Result<Vec<ReviewRequest>, CoreError> {
        self.read(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM review_requests WHERE pr_id = ?1 ORDER BY id")
                .map_err(db)?;
            let ids = stmt
                .query_map([pr.0], |row| row.get::<_, i64>(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            ids.into_iter()
                .map(|id| {
                    load_request(conn, ReviewRequestId(id))?
                        .ok_or_else(|| CoreError::Store(format!("review request {id} disappeared")))
                })
                .collect()
        })
    }

    pub fn close_review_request(
        &self,
        id: ReviewRequestId,
        reason: CloseReason,
        at: DateTime<Utc>,
    ) -> Result<ReviewRequest, CoreError> {
        let at = format_ts(at);
        self.write(move |conn| {
            let changed = conn
                .execute(
                    "UPDATE review_requests
                     SET status = 'closed', close_reason = ?1, closed_at = ?2
                     WHERE id = ?3 AND status = 'active'",
                    params![reason.as_str(), at, id.0],
                )
                .map_err(db)?;
            if changed == 0 {
                return Err(CoreError::Validation(
                    "review request is not active".to_owned(),
                ));
            }
            load_request(conn, id)?
                .ok_or_else(|| CoreError::Store("closed review request disappeared".to_owned()))
        })
    }

    /// §7.4: a bump only moves `last_bumped_at` and `bump_count` on the
    /// active row. It never creates a row.
    pub fn record_bump(
        &self,
        id: ReviewRequestId,
        at: DateTime<Utc>,
    ) -> Result<ReviewRequest, CoreError> {
        let at = format_ts(at);
        self.write(move |conn| {
            let changed = conn
                .execute(
                    "UPDATE review_requests
                     SET last_bumped_at = ?1, bump_count = bump_count + 1
                     WHERE id = ?2 AND status = 'active'",
                    params![at, id.0],
                )
                .map_err(db)?;
            if changed == 0 {
                return Err(CoreError::Validation(
                    "review request is not active".to_owned(),
                ));
            }
            load_request(conn, id)?
                .ok_or_else(|| CoreError::Store("bumped review request disappeared".to_owned()))
        })
    }

    pub fn upsert_discovery_run(&self, run: DiscoveryRun) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO discovery_runs (pr_id, slack_workspace, ran_at, outcome)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(pr_id, slack_workspace) DO UPDATE SET
                    ran_at = excluded.ran_at,
                    outcome = excluded.outcome",
                params![
                    run.pr_id.0,
                    run.slack_workspace,
                    format_ts(run.ran_at),
                    run.outcome.as_str(),
                ],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn get_discovery_run(
        &self,
        pr: PrId,
        workspace: String,
    ) -> Result<Option<DiscoveryRun>, CoreError> {
        self.read(move |conn| {
            conn.query_row(
                "SELECT * FROM discovery_runs WHERE pr_id = ?1 AND slack_workspace = ?2",
                params![pr.0, workspace],
                map_discovery,
            )
            .optional()
            .map_err(db)
        })
    }

    pub fn insert_send_attempt(&self, attempt: SendAttempt) -> Result<SendAttempt, CoreError> {
        self.write(move |conn| {
            let id: i64 = conn
                .query_row(
                    "INSERT INTO send_attempts (
                        batch_id, pr_id, channel_id, origin, outcome, detail, at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     RETURNING id",
                    params![
                        attempt.batch_id,
                        attempt.pr_id.map(|id| id.0),
                        attempt.channel_id,
                        attempt.origin,
                        attempt.outcome.as_str(),
                        attempt.detail,
                        format_ts(attempt.at),
                    ],
                    |row| row.get(0),
                )
                .map_err(db)?;
            conn.query_row(
                "SELECT * FROM send_attempts WHERE id = ?1",
                [id],
                map_attempt,
            )
            .map_err(db)
        })
    }
}

fn insert_message(conn: &Connection, message: &ReviewMessage) -> Result<ReviewMessage, CoreError> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO review_messages (
                slack_account_id, channel_id, channel_name, ts, thread_root_ts,
                kind, origin, permalink, text, batch_id, posted_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             RETURNING id",
            params![
                message.slack_account_id.0,
                message.channel_id,
                message.channel_name,
                message.ts,
                message.thread_root_ts,
                message.kind.as_str(),
                message.origin.as_str(),
                message.permalink,
                message.text,
                message.batch_id,
                format_ts(message.posted_at),
            ],
            |row| row.get(0),
        )
        .map_err(db)?;
    load_message(conn, id)?
        .ok_or_else(|| CoreError::Store("inserted review message disappeared".to_owned()))
}

fn load_message(conn: &Connection, id: i64) -> Result<Option<ReviewMessage>, CoreError> {
    conn.query_row(
        "SELECT * FROM review_messages WHERE id = ?1",
        [id],
        map_message,
    )
    .optional()
    .map_err(db)
}

fn map_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewMessage> {
    Ok(ReviewMessage {
        id: row.get("id")?,
        slack_account_id: AccountId(row.get("slack_account_id")?),
        channel_id: row.get("channel_id")?,
        channel_name: row.get("channel_name")?,
        ts: row.get("ts")?,
        thread_root_ts: row.get("thread_root_ts")?,
        kind: parse_enum::<ReviewMessageKind>(row.get("kind")?)?,
        origin: parse_enum::<ReviewMessageOrigin>(row.get("origin")?)?,
        permalink: row.get("permalink")?,
        text: row.get("text")?,
        batch_id: row.get("batch_id")?,
        posted_at: ts_col(row.get("posted_at")?)?,
    })
}

fn map_message_pr(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewMessagePr> {
    Ok(ReviewMessagePr {
        message_id: row.get("message_id")?,
        pr_id: PrId(row.get("pr_id")?),
        head_sha: row.get("head_sha")?,
        teams_json: row.get("teams_json")?,
    })
}

fn insert_request(conn: &Connection, request: &ReviewRequest) -> Result<ReviewRequest, CoreError> {
    let (status, reason, closed_at) = status_columns(request)?;
    let id: i64 = conn
        .query_row(
            "INSERT INTO review_requests (
                pr_id, slack_workspace, channel_id, channel_name, root_ts, permalink,
                requested_at, last_bumped_at, bump_count, status, close_reason, closed_at, source
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             RETURNING id",
            params![
                request.pr.0,
                request.workspace,
                request.channel_id,
                request.channel_name,
                request.root_ts,
                request.permalink,
                format_ts(request.requested_at),
                request.last_bumped_at.map(format_ts),
                request.bump_count,
                status,
                reason,
                closed_at,
                request.source.as_str(),
            ],
            |row| row.get(0),
        )
        .map_err(db)?;
    load_request(conn, ReviewRequestId(id))?
        .ok_or_else(|| CoreError::Store("inserted review request disappeared".to_owned()))
}

fn status_columns(
    request: &ReviewRequest,
) -> Result<(&'static str, Option<&'static str>, Option<String>), CoreError> {
    match (request.status, request.closed_at) {
        (RequestStatus::Active, None) => Ok(("active", None, None)),
        (RequestStatus::Active, Some(_)) => Err(CoreError::Validation(
            "active review request cannot set closed_at".to_owned(),
        )),
        (RequestStatus::Closed(reason), Some(at)) => {
            Ok(("closed", Some(reason.as_str()), Some(format_ts(at))))
        }
        (RequestStatus::Closed(_), None) => Err(CoreError::Validation(
            "closed review request requires closed_at".to_owned(),
        )),
    }
}

fn load_request(
    conn: &Connection,
    id: ReviewRequestId,
) -> Result<Option<ReviewRequest>, CoreError> {
    let Some(mut request) = conn
        .query_row(
            "SELECT * FROM review_requests WHERE id = ?1",
            [id.0],
            map_request,
        )
        .optional()
        .map_err(db)?
    else {
        return Ok(None);
    };
    request.root.account = slack_account(conn, &request.workspace).map_err(db)?;
    Ok(Some(request))
}

fn map_request(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewRequest> {
    let workspace: String = row.get("slack_workspace")?;
    let channel_id: String = row.get("channel_id")?;
    let root_ts: String = row.get("root_ts")?;
    let status: String = row.get("status")?;
    let close_reason: Option<String> = row.get("close_reason")?;
    let status = match status.as_str() {
        "active" => RequestStatus::Active,
        "closed" => {
            let reason = close_reason.ok_or_else(|| {
                conv(CoreError::Validation(
                    "closed review request is missing close_reason".to_owned(),
                ))
            })?;
            RequestStatus::Closed(parse_enum::<CloseReason>(reason)?)
        }
        other => {
            return Err(conv(CoreError::Validation(format!(
                "unknown review request status {other}"
            ))));
        }
    };
    Ok(ReviewRequest {
        id: ReviewRequestId(row.get("id")?),
        pr: PrId(row.get("pr_id")?),
        root: ProviderRef {
            account: AccountId(0),
            id: format!("{channel_id}:{root_ts}"),
        },
        workspace,
        channel_id,
        channel_name: row.get("channel_name")?,
        root_ts,
        permalink: row.get("permalink")?,
        requested_at: ts_col(row.get("requested_at")?)?,
        last_bumped_at: opt_ts_col(row.get("last_bumped_at")?)?,
        bump_count: row.get::<_, i64>("bump_count")? as u32,
        status,
        closed_at: opt_ts_col(row.get("closed_at")?)?,
        source: parse_enum::<RequestSource>(row.get("source")?)?,
    })
}

fn slack_account(conn: &Connection, workspace: &str) -> rusqlite::Result<AccountId> {
    let id = conn
        .query_row(
            "SELECT id FROM accounts WHERE slack_workspace = ?1",
            [workspace],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(AccountId(id.unwrap_or(0)))
}

fn map_discovery(row: &rusqlite::Row<'_>) -> rusqlite::Result<DiscoveryRun> {
    Ok(DiscoveryRun {
        pr_id: PrId(row.get("pr_id")?),
        slack_workspace: row.get("slack_workspace")?,
        ran_at: ts_col(row.get("ran_at")?)?,
        outcome: parse_enum::<DiscoveryOutcome>(row.get("outcome")?)?,
    })
}

fn map_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<SendAttempt> {
    let pr_id: Option<i64> = row.get("pr_id")?;
    Ok(SendAttempt {
        id: row.get("id")?,
        batch_id: row.get("batch_id")?,
        pr_id: pr_id.map(PrId),
        channel_id: row.get("channel_id")?,
        origin: row.get("origin")?,
        outcome: parse_enum::<SendOutcome>(row.get("outcome")?)?,
        detail: row.get("detail")?,
        at: ts_col(row.get("at")?)?,
    })
}
