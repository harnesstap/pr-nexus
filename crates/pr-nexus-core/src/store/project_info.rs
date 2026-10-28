//! Project descriptions, JIRA links, PR notes and stars (migration 0007).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{AccountId, JiraLink, PrId, PrNote, Project, ProjectId};
use crate::error::CoreError;
use crate::jira::ParsedLink;

use super::projects::load_project;
use super::time::{format_ts, ts_col};
use super::{Store, db};

/// Longest description and note kept, in characters.
pub const MAX_TEXT: usize = 10_000;

impl Store {
    pub fn set_project_description(
        &self,
        id: ProjectId,
        description: String,
    ) -> Result<Project, CoreError> {
        let description = valid_text("Description", &description)?;
        self.write(move |conn| {
            let updated = conn
                .execute(
                    "UPDATE projects SET description = ?1 WHERE id = ?2",
                    params![description, id.0],
                )
                .map_err(db)?;
            if updated == 0 {
                return Err(CoreError::Validation("Project not found".into()));
            }
            load_project(conn, id)?
                .ok_or_else(|| CoreError::Store("described project disappeared".into()))
        })
    }

    pub fn list_jira_links(&self, project: ProjectId) -> Result<Vec<JiraLink>, CoreError> {
        self.read(move |conn| jira_links_in(conn, project))
    }

    /// Every project's links, in display order.
    pub fn all_jira_links(&self) -> Result<HashMap<ProjectId, Vec<JiraLink>>, CoreError> {
        self.read(|conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM project_jira_links ORDER BY project_id, position, id")
                .map_err(db)?;
            let mut links: HashMap<ProjectId, Vec<JiraLink>> = HashMap::new();
            for link in stmt.query_map([], map_jira).map_err(db)? {
                let link = link.map_err(db)?;
                links.entry(link.project).or_default().push(link);
            }
            Ok(links)
        })
    }

    /// Appends one link after the others. A duplicate is a validation error.
    pub fn add_jira_link(
        &self,
        project: ProjectId,
        link: ParsedLink,
        now: DateTime<Utc>,
    ) -> Result<JiraLink, CoreError> {
        self.write(move |conn| {
            ensure_project(conn, project)?;
            let position: i32 = conn
                .query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM project_jira_links WHERE project_id = ?1",
                    [project.0],
                    |row| row.get(0),
                )
                .map_err(db)?;
            insert_jira(conn, project, &link, position, now)
        })
    }

    pub fn remove_jira_link(&self, id: i64) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute("DELETE FROM project_jira_links WHERE id = ?1", [id])
                .map_err(db)?;
            Ok(())
        })
    }

    /// Replaces every link of `project` with `links`, in that order, in one
    /// transaction. Links kept keep their creation time. Duplicates collapse.
    pub fn set_jira_links(
        &self,
        project: ProjectId,
        links: Vec<ParsedLink>,
        now: DateTime<Utc>,
    ) -> Result<Vec<JiraLink>, CoreError> {
        self.write(move |conn| {
            let tx = conn.transaction().map_err(db)?;
            ensure_project(&tx, project)?;
            let before: HashMap<(String, String), DateTime<Utc>> = jira_links_in(&tx, project)?
                .into_iter()
                .map(|link| ((link.key, link.url), link.created_at))
                .collect();
            tx.execute(
                "DELETE FROM project_jira_links WHERE project_id = ?1",
                [project.0],
            )
            .map_err(db)?;
            let mut seen = HashSet::new();
            let mut position = 0;
            for link in links {
                if !seen.insert((link.key.clone(), link.url.clone())) {
                    continue;
                }
                let created = before
                    .get(&(link.key.clone(), link.url.clone()))
                    .copied()
                    .unwrap_or(now);
                insert_jira(&tx, project, &link, position, created)?;
                position += 1;
            }
            let saved = jira_links_in(&tx, project)?;
            tx.commit().map_err(db)?;
            Ok(saved)
        })
    }

    pub fn pr_note(&self, pr: PrId) -> Result<Option<PrNote>, CoreError> {
        self.read(move |conn| {
            conn.query_row("SELECT * FROM pr_notes WHERE pr_id = ?1", [pr.0], |row| {
                Ok(PrNote {
                    pr: PrId(row.get("pr_id")?),
                    note: row.get("note")?,
                    updated_at: ts_col(row.get("updated_at")?)?,
                })
            })
            .optional()
            .map_err(db)
        })
    }

    /// Saves the note; a blank note deletes it and returns `None`.
    pub fn set_pr_note(
        &self,
        pr: PrId,
        note: String,
        now: DateTime<Utc>,
    ) -> Result<Option<PrNote>, CoreError> {
        let note = valid_text("Note", &note)?;
        self.write(move |conn| {
            ensure_pr(conn, pr)?;
            if note.is_empty() {
                conn.execute("DELETE FROM pr_notes WHERE pr_id = ?1", [pr.0])
                    .map_err(db)?;
                return Ok(None);
            }
            conn.execute(
                "INSERT INTO pr_notes (pr_id, note, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (pr_id) DO UPDATE SET note = excluded.note, updated_at = excluded.updated_at",
                params![pr.0, note, format_ts(now)],
            )
            .map_err(db)?;
            Ok(Some(PrNote {
                pr,
                note,
                updated_at: now,
            }))
        })
    }

    /// Stars or unstars by the PR's GitHub node id, so the star outlives the
    /// local row. Idempotent.
    pub fn set_pr_starred(
        &self,
        pr: PrId,
        starred: bool,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.write(move |conn| {
            let node: String = conn
                .query_row(
                    "SELECT node_id FROM pull_requests WHERE id = ?1",
                    [pr.0],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db)?
                .ok_or_else(|| CoreError::Validation("Pull request not found".into()))?;
            if starred {
                conn.execute(
                    "INSERT OR IGNORE INTO pr_stars (node_id, starred_at) VALUES (?1, ?2)",
                    params![node, format_ts(now)],
                )
            } else {
                conn.execute("DELETE FROM pr_stars WHERE node_id = ?1", [node])
            }
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn is_pr_starred(&self, pr: PrId) -> Result<bool, CoreError> {
        self.read(move |conn| {
            conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM pull_requests pr JOIN pr_stars s ON s.node_id = pr.node_id WHERE pr.id = ?1)",
                [pr.0],
                |row| row.get(0),
            )
            .map_err(db)
        })
    }
}

/// Starred rows of one account, for the list.
pub(super) fn starred_in(
    conn: &Connection,
    account: AccountId,
) -> Result<HashSet<PrId>, CoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT pr.id FROM pull_requests pr
             JOIN repositories r ON r.id = pr.repo_id
             JOIN pr_stars s ON s.node_id = pr.node_id
             WHERE r.account_id = ?1",
        )
        .map_err(db)?;
    let ids = stmt
        .query_map([account.0], |row| row.get(0).map(PrId))
        .map_err(db)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(db)?;
    Ok(ids)
}

fn valid_text(what: &str, text: &str) -> Result<String, CoreError> {
    let text = text.trim();
    if text.chars().count() > MAX_TEXT {
        return Err(CoreError::Validation(format!(
            "{what} is longer than {MAX_TEXT} characters"
        )));
    }
    Ok(text.to_owned())
}

fn ensure_project(conn: &Connection, project: ProjectId) -> Result<(), CoreError> {
    load_project(conn, project)?
        .map(|_| ())
        .ok_or_else(|| CoreError::Validation("Project not found".into()))
}

fn ensure_pr(conn: &Connection, pr: PrId) -> Result<(), CoreError> {
    let found: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM pull_requests WHERE id = ?1)",
            [pr.0],
            |row| row.get(0),
        )
        .map_err(db)?;
    if found {
        Ok(())
    } else {
        Err(CoreError::Validation("Pull request not found".into()))
    }
}

fn jira_links_in(conn: &Connection, project: ProjectId) -> Result<Vec<JiraLink>, CoreError> {
    let mut stmt = conn
        .prepare("SELECT * FROM project_jira_links WHERE project_id = ?1 ORDER BY position, id")
        .map_err(db)?;
    stmt.query_map([project.0], map_jira)
        .map_err(db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db)
}

fn insert_jira(
    conn: &Connection,
    project: ProjectId,
    link: &ParsedLink,
    position: i32,
    created: DateTime<Utc>,
) -> Result<JiraLink, CoreError> {
    let duplicate: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM project_jira_links WHERE project_id = ?1 AND key = ?2 AND url = ?3)",
            params![project.0, link.key, link.url],
            |row| row.get(0),
        )
        .map_err(db)?;
    if duplicate {
        return Err(CoreError::Validation(
            "That JIRA link is already on this project".into(),
        ));
    }
    let id: i64 = conn
        .query_row(
            "INSERT INTO project_jira_links (project_id, key, url, position, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
            params![project.0, link.key, link.url, position, format_ts(created)],
            |row| row.get(0),
        )
        .map_err(db)?;
    Ok(JiraLink {
        id,
        project,
        key: link.key.clone(),
        url: link.url.clone(),
        position,
        created_at: created,
    })
}

fn map_jira(row: &rusqlite::Row<'_>) -> rusqlite::Result<JiraLink> {
    Ok(JiraLink {
        id: row.get("id")?,
        project: ProjectId(row.get("project_id")?),
        key: row.get("key")?,
        url: row.get("url")?,
        position: row.get("position")?,
        created_at: ts_col(row.get("created_at")?)?,
    })
}
