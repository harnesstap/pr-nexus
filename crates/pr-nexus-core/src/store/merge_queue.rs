//! Stored merge queue facts, one row per pull request.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{AccountId, PrId};
use crate::error::CoreError;
use crate::merge_queue::{MergeQueuePhase, MergeQueueSnapshot};

use super::time::{parse_bool, sql_bool};
use super::{Store, db};

impl Store {
    pub fn set_merge_queue(&self, id: PrId, queue: &MergeQueueSnapshot) -> Result<(), CoreError> {
        let queue = queue.clone();
        self.write(move |conn| {
            write_merge_queue(conn, id, &queue)?;
            Ok(())
        })
    }

    pub fn merge_queue(&self, id: PrId) -> Result<Option<MergeQueueSnapshot>, CoreError> {
        self.read(move |conn| read_merge_queue(conn, id))
    }
}

pub(super) fn merge_queues_for(
    conn: &Connection,
    account_id: AccountId,
) -> Result<HashMap<PrId, MergeQueueSnapshot>, CoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT q.pr_id, q.required, q.phase, q.position, q.eta_seconds, q.reason
             FROM pr_merge_queue q
             JOIN pull_requests pr ON pr.id = q.pr_id
             JOIN repositories r ON r.id = pr.repo_id
             WHERE r.account_id = ?1",
        )
        .map_err(db)?;
    let rows = stmt
        .query_map([account_id.0], |row| {
            let id = PrId(row.get(0)?);
            let queue = snapshot_from_row(row, 1)?;
            Ok((id, queue))
        })
        .map_err(db)?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, queue) = row.map_err(db)?;
        out.insert(id, queue);
    }
    Ok(out)
}

fn write_merge_queue(
    conn: &Connection,
    id: PrId,
    queue: &MergeQueueSnapshot,
) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO pr_merge_queue (pr_id, required, phase, position, eta_seconds, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(pr_id) DO UPDATE SET
            required = excluded.required,
            phase = excluded.phase,
            position = excluded.position,
            eta_seconds = excluded.eta_seconds,
            reason = excluded.reason",
        params![
            id.0,
            sql_bool(queue.required),
            queue.phase.map(|phase| phase.as_str().to_owned()),
            queue.position.map(i64::from),
            queue.eta_seconds.map(i64::from),
            queue.reason,
        ],
    )
    .map_err(db)?;
    Ok(())
}

fn read_merge_queue(conn: &Connection, id: PrId) -> Result<Option<MergeQueueSnapshot>, CoreError> {
    conn.query_row(
        "SELECT required, phase, position, eta_seconds, reason FROM pr_merge_queue WHERE pr_id = ?1",
        [id.0],
        |row| snapshot_from_row(row, 0),
    )
    .optional()
    .map_err(db)
}

fn snapshot_from_row(
    row: &rusqlite::Row<'_>,
    start: usize,
) -> Result<MergeQueueSnapshot, rusqlite::Error> {
    let phase: Option<String> = row.get(start + 1)?;
    let phase = match phase.as_deref() {
        None => None,
        Some(value) => Some(MergeQueuePhase::parse(value).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                start + 1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown merge queue phase {value}"),
                )),
            )
        })?),
    };
    let position: Option<i64> = row.get(start + 2)?;
    let eta: Option<i64> = row.get(start + 3)?;
    Ok(MergeQueueSnapshot {
        required: parse_bool(row.get(start)?),
        phase,
        position: position.and_then(|value| u32::try_from(value).ok()),
        eta_seconds: eta.and_then(|value| u32::try_from(value).ok()),
        reason: row.get(start + 4)?,
    })
}
