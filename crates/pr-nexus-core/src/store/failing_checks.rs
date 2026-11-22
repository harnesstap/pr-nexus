//! Cached failing check names, one JSON list per pull request.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::checks::FailingCheck;
use crate::domain::{AccountId, PrId};
use crate::error::CoreError;

use super::{Store, db};

impl Store {
    /// Replaces the cached failing checks. An empty list deletes the row:
    /// the rollup is not failing, or it failed without a named check.
    pub fn set_failing_checks(&self, id: PrId, checks: &[FailingCheck]) -> Result<(), CoreError> {
        let checks = checks.to_vec();
        self.write(move |conn| write_failing_checks(conn, id, &checks))
    }

    pub fn failing_checks(&self, id: PrId) -> Result<Option<Vec<FailingCheck>>, CoreError> {
        self.read(move |conn| read_failing_checks(conn, id))
    }
}

pub(super) fn failing_checks_for(
    conn: &Connection,
    account_id: AccountId,
) -> Result<HashMap<PrId, Vec<FailingCheck>>, CoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT q.pr_id, q.checks
             FROM pr_failing_checks q
             JOIN pull_requests pr ON pr.id = q.pr_id
             JOIN repositories r ON r.id = pr.repo_id
             WHERE r.account_id = ?1",
        )
        .map_err(db)?;
    let rows = stmt
        .query_map([account_id.0], |row| {
            let id = PrId(row.get(0)?);
            let text: String = row.get(1)?;
            Ok((id, text))
        })
        .map_err(db)?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, text) = row.map_err(db)?;
        out.insert(id, decode(&text)?);
    }
    Ok(out)
}

fn write_failing_checks(
    conn: &Connection,
    id: PrId,
    checks: &[FailingCheck],
) -> Result<(), CoreError> {
    if checks.is_empty() {
        conn.execute("DELETE FROM pr_failing_checks WHERE pr_id = ?1", [id.0])
            .map_err(db)?;
        return Ok(());
    }
    let text = serde_json::to_string(checks)
        .map_err(|err| CoreError::Store(format!("failing checks: {err}")))?;
    conn.execute(
        "INSERT INTO pr_failing_checks (pr_id, checks) VALUES (?1, ?2)
         ON CONFLICT(pr_id) DO UPDATE SET checks = excluded.checks",
        params![id.0, text],
    )
    .map_err(db)?;
    Ok(())
}

fn read_failing_checks(
    conn: &Connection,
    id: PrId,
) -> Result<Option<Vec<FailingCheck>>, CoreError> {
    let text: Option<String> = conn
        .query_row(
            "SELECT checks FROM pr_failing_checks WHERE pr_id = ?1",
            [id.0],
            |row| row.get(0),
        )
        .optional()
        .map_err(db)?;
    text.map(|text| decode(&text)).transpose()
}

fn decode(text: &str) -> Result<Vec<FailingCheck>, CoreError> {
    serde_json::from_str(text).map_err(|err| CoreError::Store(format!("failing checks: {err}")))
}
