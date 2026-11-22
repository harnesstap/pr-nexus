//! Cached review-request timeline, and the teams the signed-in user belongs to.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{AccountId, PrId, TeamRef};
use crate::error::CoreError;
use crate::review_clock::ReviewClock;

use super::{Store, db};

/// Teams from the last successful lookup. `known` is false until that lookup.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewerTeams {
    pub teams: Vec<TeamRef>,
    pub known: bool,
}

impl Store {
    pub fn set_review_clock(&self, id: PrId, clock: &ReviewClock) -> Result<(), CoreError> {
        let clock = clock.clone();
        self.write(move |conn| write_clock(conn, id, &clock))
    }

    pub fn review_clock(&self, id: PrId) -> Result<Option<ReviewClock>, CoreError> {
        self.read(move |conn| read_clock(conn, id))
    }

    pub fn viewer_teams(&self, account_id: AccountId) -> Result<ViewerTeams, CoreError> {
        self.read(move |conn| read_teams(conn, account_id))
    }

    /// Replaces the cached teams and marks the lookup as done. A no-op when
    /// the stored set is already this one, so a sync does not bump the list.
    pub fn replace_viewer_teams(
        &self,
        account_id: AccountId,
        teams: &[TeamRef],
    ) -> Result<(), CoreError> {
        let next = normalize_teams(teams);
        let current = self.viewer_teams(account_id)?;
        if current.known && same_teams(&current.teams, &next) {
            return Ok(());
        }
        self.write(move |conn| write_teams(conn, account_id, &next))
    }
}

pub(super) fn clocks_for(
    conn: &Connection,
    account_id: AccountId,
) -> Result<HashMap<PrId, ReviewClock>, CoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT q.pr_id, q.clock
             FROM pr_review_clock q
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

fn write_clock(conn: &Connection, id: PrId, clock: &ReviewClock) -> Result<(), CoreError> {
    let text = serde_json::to_string(clock)
        .map_err(|err| CoreError::Store(format!("review clock: {err}")))?;
    conn.execute(
        "INSERT INTO pr_review_clock (pr_id, clock) VALUES (?1, ?2)
         ON CONFLICT(pr_id) DO UPDATE SET clock = excluded.clock",
        params![id.0, text],
    )
    .map_err(db)?;
    Ok(())
}

fn read_clock(conn: &Connection, id: PrId) -> Result<Option<ReviewClock>, CoreError> {
    let text: Option<String> = conn
        .query_row(
            "SELECT clock FROM pr_review_clock WHERE pr_id = ?1",
            [id.0],
            |row| row.get(0),
        )
        .optional()
        .map_err(db)?;
    text.map(|text| decode(&text)).transpose()
}

fn decode(text: &str) -> Result<ReviewClock, CoreError> {
    serde_json::from_str(text).map_err(|err| CoreError::Store(format!("review clock: {err}")))
}

fn read_teams(conn: &Connection, account_id: AccountId) -> Result<ViewerTeams, CoreError> {
    let known: Option<i64> = conn
        .query_row(
            "SELECT known FROM viewer_team_state WHERE account_id = ?1",
            [account_id.0],
            |row| row.get(0),
        )
        .optional()
        .map_err(db)?;
    if known != Some(1) {
        return Ok(ViewerTeams::default());
    }
    let mut stmt = conn
        .prepare("SELECT org, slug FROM viewer_team WHERE account_id = ?1 ORDER BY org, slug")
        .map_err(db)?;
    let rows = stmt
        .query_map([account_id.0], |row| {
            Ok(TeamRef {
                org: row.get(0)?,
                slug: row.get(1)?,
            })
        })
        .map_err(db)?;
    let mut teams = Vec::new();
    for row in rows {
        teams.push(row.map_err(db)?);
    }
    Ok(ViewerTeams { teams, known: true })
}

fn write_teams(
    conn: &mut Connection,
    account_id: AccountId,
    teams: &[TeamRef],
) -> Result<(), CoreError> {
    let tx = conn.transaction().map_err(db)?;
    tx.execute(
        "DELETE FROM viewer_team WHERE account_id = ?1",
        [account_id.0],
    )
    .map_err(db)?;
    for team in teams {
        tx.execute(
            "INSERT INTO viewer_team (account_id, org, slug) VALUES (?1, ?2, ?3)",
            params![account_id.0, team.org, team.slug],
        )
        .map_err(db)?;
    }
    tx.execute(
        "INSERT INTO viewer_team_state (account_id, known) VALUES (?1, 1)
         ON CONFLICT(account_id) DO UPDATE SET known = 1",
        [account_id.0],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(())
}

fn normalize_teams(teams: &[TeamRef]) -> Vec<TeamRef> {
    let mut out = Vec::new();
    for team in teams {
        let org = team.org.trim();
        let slug = team.slug.trim();
        if org.is_empty() || slug.is_empty() {
            continue;
        }
        if out.iter().any(|kept: &TeamRef| {
            kept.org.eq_ignore_ascii_case(org) && kept.slug.eq_ignore_ascii_case(slug)
        }) {
            continue;
        }
        out.push(TeamRef {
            org: org.to_owned(),
            slug: slug.to_owned(),
        });
    }
    out
}

fn same_teams(left: &[TeamRef], right: &[TeamRef]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    right.iter().all(|team| {
        left.iter().any(|kept| {
            kept.org.eq_ignore_ascii_case(&team.org) && kept.slug.eq_ignore_ascii_case(&team.slug)
        })
    })
}
