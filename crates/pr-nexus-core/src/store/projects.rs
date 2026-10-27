use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{LinkSource, PrId, PrProjectLink, Project, ProjectId, ProjectRule};
use crate::error::CoreError;
use crate::linking::RuleLink;

use super::row::{conv, parse_enum};
use super::time::{format_ts, parse_bool, sql_bool, ts_col};
use super::{Store, db};

impl Store {
    pub fn insert_project(&self, project: Project) -> Result<Project, CoreError> {
        self.write(move |conn| insert_project(conn, &project))
    }

    pub fn get_project(&self, id: ProjectId) -> Result<Option<Project>, CoreError> {
        self.read(move |conn| load_project(conn, id))
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, CoreError> {
        self.read(|conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM projects ORDER BY sort_order, name")
                .map_err(db)?;
            let rows = stmt
                .query_map([], map_project)
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }

    /// Creates an empty project named `name` (trimmed, non-empty, unique), placed last.
    pub fn create_project(&self, name: String) -> Result<Project, CoreError> {
        let name = valid_name(&name)?;
        self.write(move |conn| {
            ensure_name_free(conn, &name, None)?;
            let sort_order: i32 = conn
                .query_row(
                    "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM projects",
                    [],
                    |row| row.get(0),
                )
                .map_err(db)?;
            insert_project(
                conn,
                &Project {
                    id: ProjectId(0),
                    name,
                    description: String::new(),
                    color: DEFAULT_COLOR.to_owned(),
                    archived: false,
                    sort_order,
                },
            )
        })
    }

    pub fn rename_project(&self, id: ProjectId, name: String) -> Result<Project, CoreError> {
        let name = valid_name(&name)?;
        self.write(move |conn| {
            ensure_name_free(conn, &name, Some(id))?;
            let updated = conn
                .execute(
                    "UPDATE projects SET name = ?1 WHERE id = ?2",
                    params![name, id.0],
                )
                .map_err(db)?;
            if updated == 0 {
                return Err(CoreError::Validation("Project not found".into()));
            }
            load_project(conn, id)?
                .ok_or_else(|| CoreError::Store("renamed project disappeared".into()))
        })
    }

    pub fn set_project_archived(&self, id: ProjectId, archived: bool) -> Result<(), CoreError> {
        self.write(move |conn| {
            let updated = conn
                .execute(
                    "UPDATE projects SET archived = ?1 WHERE id = ?2",
                    params![sql_bool(archived), id.0],
                )
                .map_err(db)?;
            if updated == 0 {
                return Err(CoreError::Validation("Project not found".into()));
            }
            Ok(())
        })
    }

    pub fn delete_project(&self, id: ProjectId) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute("DELETE FROM projects WHERE id = ?1", [id.0])
                .map_err(db)?;
            Ok(())
        })
    }

    pub fn insert_project_rule(&self, rule: ProjectRule) -> Result<ProjectRule, CoreError> {
        self.write(move |conn| insert_rule(conn, &rule))
    }

    pub fn list_project_rules(&self, project: ProjectId) -> Result<Vec<ProjectRule>, CoreError> {
        self.read(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM project_rules WHERE project_id = ?1 ORDER BY priority, id")
                .map_err(db)?;
            let rows = stmt
                .query_map([project.0], map_rule)
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }

    /// Every rule of every project, highest priority first.
    pub fn list_all_project_rules(&self) -> Result<Vec<ProjectRule>, CoreError> {
        self.read(|conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM project_rules ORDER BY priority DESC, id")
                .map_err(db)?;
            let rows = stmt
                .query_map([], map_rule)
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }

    pub fn update_project_rule(&self, rule: ProjectRule) -> Result<ProjectRule, CoreError> {
        self.write(move |conn| {
            let updated = conn
                .execute(
                    "UPDATE project_rules SET kind = ?1, pattern = ?2, priority = ?3
                     WHERE id = ?4 AND project_id = ?5",
                    params![
                        rule.kind.as_str(),
                        rule.pattern,
                        rule.priority,
                        rule.id,
                        rule.project.0
                    ],
                )
                .map_err(db)?;
            if updated == 0 {
                return Err(CoreError::Validation("Rule not found".into()));
            }
            conn.query_row(
                "SELECT * FROM project_rules WHERE id = ?1",
                [rule.id],
                map_rule,
            )
            .map_err(db)
        })
    }

    /// Deletes a rule and the `rule` links it created. Callers relink afterwards so
    /// another rule of the same project can take over.
    pub fn delete_project_rule(&self, id: i64) -> Result<(), CoreError> {
        self.write(move |conn| {
            let tx = conn.transaction().map_err(db)?;
            tx.execute(
                "DELETE FROM pr_projects WHERE source = 'rule' AND rule_id = ?1",
                [id],
            )
            .map_err(db)?;
            tx.execute("DELETE FROM project_rules WHERE id = ?1", [id])
                .map_err(db)?;
            tx.commit().map_err(db)
        })
    }

    pub fn set_project_color(&self, id: ProjectId, color: String) -> Result<Project, CoreError> {
        let color = valid_color(&color)?;
        self.write(move |conn| {
            let updated = conn
                .execute(
                    "UPDATE projects SET color = ?1 WHERE id = ?2",
                    params![color, id.0],
                )
                .map_err(db)?;
            if updated == 0 {
                return Err(CoreError::Validation("Project not found".into()));
            }
            load_project(conn, id)?
                .ok_or_else(|| CoreError::Store("recolored project disappeared".into()))
        })
    }

    /// Every link of `project`, excluded rows included.
    pub fn list_project_links(&self, project: ProjectId) -> Result<Vec<PrProjectLink>, CoreError> {
        self.read(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM pr_projects WHERE project_id = ?1 ORDER BY created_at, pr_id",
                )
                .map_err(db)?;
            let rows = stmt
                .query_map([project.0], map_link)
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }

    /// Links `prs` to `project` by hand. A manual link replaces a rule or excluded row.
    pub fn link_manual(
        &self,
        project: ProjectId,
        prs: Vec<PrId>,
        now: DateTime<Utc>,
    ) -> Result<usize, CoreError> {
        self.write(move |conn| {
            let tx = conn.transaction().map_err(db)?;
            let mut changed = 0;
            for pr in prs {
                changed += tx
                    .execute(
                        "INSERT INTO pr_projects (pr_id, project_id, source, rule_id, created_at)
                         VALUES (?1, ?2, 'manual', NULL, ?3)
                         ON CONFLICT(pr_id, project_id) DO UPDATE SET
                            source = 'manual', rule_id = NULL
                         WHERE pr_projects.source <> 'manual'",
                        params![pr.0, project.0, format_ts(now)],
                    )
                    .map_err(db)?;
            }
            tx.commit().map_err(db)?;
            Ok(changed)
        })
    }

    /// Removes `pr` from `project` (§8.1). A rule link, or a manual link that a rule
    /// would re-create (`rule_matches`), becomes `excluded`; a plain manual link is deleted.
    pub fn unlink_pr(
        &self,
        project: ProjectId,
        pr: PrId,
        rule_matches: bool,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        self.write(move |conn| {
            let tx = conn.transaction().map_err(db)?;
            let source: Option<String> = tx
                .query_row(
                    "SELECT source FROM pr_projects WHERE pr_id = ?1 AND project_id = ?2",
                    [pr.0, project.0],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db)?;
            match source.as_deref() {
                Some("rule") => exclude(&tx, pr, project, now)?,
                Some("manual") if rule_matches => exclude(&tx, pr, project, now)?,
                Some("manual") => {
                    tx.execute(
                        "DELETE FROM pr_projects WHERE pr_id = ?1 AND project_id = ?2",
                        [pr.0, project.0],
                    )
                    .map_err(db)?;
                }
                _ => {}
            }
            tx.commit().map_err(db)
        })
    }

    /// Drops an `excluded` row so rules may link the PR again.
    pub fn restore_pr(&self, project: ProjectId, pr: PrId) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "DELETE FROM pr_projects
                 WHERE pr_id = ?1 AND project_id = ?2 AND source = 'excluded'",
                [pr.0, project.0],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    /// Makes the `rule` links of the PRs in `scope` equal `wanted`, in one
    /// transaction. Manual and excluded rows are never touched. Returns (added, removed).
    pub fn apply_rule_links(
        &self,
        scope: Vec<PrId>,
        wanted: Vec<RuleLink>,
    ) -> Result<(usize, usize), CoreError> {
        let now = Utc::now();
        self.write(move |conn| {
            let tx = conn.transaction().map_err(db)?;
            let mut removed = 0;
            {
                let mut existing = tx
                    .prepare(
                        "SELECT project_id, rule_id FROM pr_projects
                         WHERE pr_id = ?1 AND source = 'rule'",
                    )
                    .map_err(db)?;
                let mut delete = tx
                    .prepare(
                        "DELETE FROM pr_projects
                         WHERE pr_id = ?1 AND project_id = ?2 AND source = 'rule'",
                    )
                    .map_err(db)?;
                for pr in &scope {
                    let rows = existing
                        .query_map([pr.0], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?))
                        })
                        .map_err(db)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(db)?;
                    for (project, _) in rows {
                        let keep = wanted
                            .iter()
                            .any(|link| link.pr == *pr && link.project.0 == project);
                        if !keep {
                            removed += delete.execute([pr.0, project]).map_err(db)?;
                        }
                    }
                }
            }
            let mut added = 0;
            {
                let mut upsert = tx
                    .prepare(
                        "INSERT INTO pr_projects (pr_id, project_id, source, rule_id, created_at)
                         VALUES (?1, ?2, 'rule', ?3, ?4)
                         ON CONFLICT(pr_id, project_id) DO UPDATE SET rule_id = excluded.rule_id
                         WHERE pr_projects.source = 'rule'
                           AND pr_projects.rule_id IS NOT excluded.rule_id",
                    )
                    .map_err(db)?;
                let mut present = tx
                    .prepare("SELECT 1 FROM pr_projects WHERE pr_id = ?1 AND project_id = ?2")
                    .map_err(db)?;
                for link in &wanted {
                    let existed = present.exists([link.pr.0, link.project.0]).map_err(db)?;
                    upsert
                        .execute(params![
                            link.pr.0,
                            link.project.0,
                            link.rule_id,
                            format_ts(now)
                        ])
                        .map_err(db)?;
                    if !existed {
                        added += 1;
                    }
                }
            }
            tx.commit().map_err(db)?;
            Ok((added, removed))
        })
    }

    pub fn upsert_pr_project(&self, link: PrProjectLink) -> Result<PrProjectLink, CoreError> {
        self.write(move |conn| upsert_link(conn, &link))
    }

    pub fn list_pr_projects(&self, pr: PrId) -> Result<Vec<PrProjectLink>, CoreError> {
        self.read(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM pr_projects WHERE pr_id = ?1 ORDER BY project_id")
                .map_err(db)?;
            let rows = stmt
                .query_map([pr.0], map_link)
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }

    pub fn delete_pr_project(&self, pr: PrId, project: ProjectId) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "DELETE FROM pr_projects WHERE pr_id = ?1 AND project_id = ?2",
                [pr.0, project.0],
            )
            .map_err(db)?;
            Ok(())
        })
    }
}

const DEFAULT_COLOR: &str = "#7c5cbf";

fn valid_name(name: &str) -> Result<String, CoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Validation("Project name cannot be empty".into()));
    }
    Ok(name.to_owned())
}

fn valid_color(color: &str) -> Result<String, CoreError> {
    let color = color.trim();
    let hex = color.strip_prefix('#').unwrap_or("");
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CoreError::Validation("Color must look like #a1b2c3".into()));
    }
    Ok(color.to_ascii_lowercase())
}

fn exclude(
    conn: &Connection,
    pr: PrId,
    project: ProjectId,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO pr_projects (pr_id, project_id, source, rule_id, created_at)
         VALUES (?1, ?2, 'excluded', NULL, ?3)
         ON CONFLICT(pr_id, project_id) DO UPDATE SET source = 'excluded', rule_id = NULL",
        params![pr.0, project.0, format_ts(now)],
    )
    .map_err(db)?;
    Ok(())
}

fn ensure_name_free(
    conn: &Connection,
    name: &str,
    except: Option<ProjectId>,
) -> Result<(), CoreError> {
    let taken: Option<i64> = conn
        .query_row(
            "SELECT id FROM projects WHERE name = ?1 AND id <> ?2",
            params![name, except.map_or(-1, |id| id.0)],
            |row| row.get(0),
        )
        .optional()
        .map_err(db)?;
    if taken.is_some() {
        return Err(CoreError::Validation(
            "A project with that name already exists".into(),
        ));
    }
    Ok(())
}

fn insert_project(conn: &Connection, project: &Project) -> Result<Project, CoreError> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO projects (name, description, color, archived, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5)
             RETURNING id",
            params![
                project.name,
                project.description,
                project.color,
                sql_bool(project.archived),
                project.sort_order,
            ],
            |row| row.get(0),
        )
        .map_err(db)?;
    load_project(conn, ProjectId(id))?
        .ok_or_else(|| CoreError::Store("inserted project disappeared".to_owned()))
}

fn load_project(conn: &Connection, id: ProjectId) -> Result<Option<Project>, CoreError> {
    conn.query_row("SELECT * FROM projects WHERE id = ?1", [id.0], map_project)
        .optional()
        .map_err(db)
}

fn map_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: ProjectId(row.get("id")?),
        name: row.get("name")?,
        description: row.get("description")?,
        color: row.get("color")?,
        archived: parse_bool(row.get("archived")?),
        sort_order: row.get("sort_order")?,
    })
}

fn insert_rule(conn: &Connection, rule: &ProjectRule) -> Result<ProjectRule, CoreError> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO project_rules (project_id, kind, pattern, priority)
             VALUES (?1, ?2, ?3, ?4)
             RETURNING id",
            params![
                rule.project.0,
                rule.kind.as_str(),
                rule.pattern,
                rule.priority,
            ],
            |row| row.get(0),
        )
        .map_err(db)?;
    conn.query_row("SELECT * FROM project_rules WHERE id = ?1", [id], map_rule)
        .map_err(db)
}

fn map_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRule> {
    Ok(ProjectRule {
        id: row.get("id")?,
        project: ProjectId(row.get("project_id")?),
        kind: parse_enum(row.get("kind")?)?,
        pattern: row.get("pattern")?,
        priority: row.get("priority")?,
    })
}

fn upsert_link(conn: &Connection, link: &PrProjectLink) -> Result<PrProjectLink, CoreError> {
    let (source, rule_id) = match &link.source {
        LinkSource::Manual => ("manual", None),
        LinkSource::Excluded => ("excluded", None),
        LinkSource::Rule(id) => ("rule", Some(*id)),
    };
    conn.execute(
        "INSERT INTO pr_projects (pr_id, project_id, source, rule_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(pr_id, project_id) DO UPDATE SET
            source = excluded.source,
            rule_id = excluded.rule_id",
        params![
            link.pr.0,
            link.project.0,
            source,
            rule_id,
            format_ts(link.created_at),
        ],
    )
    .map_err(db)?;
    conn.query_row(
        "SELECT * FROM pr_projects WHERE pr_id = ?1 AND project_id = ?2",
        [link.pr.0, link.project.0],
        map_link,
    )
    .map_err(db)
}

pub(super) fn map_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<PrProjectLink> {
    let source: String = row.get("source")?;
    let rule_id: Option<i64> = row.get("rule_id")?;
    let source = match source.as_str() {
        "manual" => LinkSource::Manual,
        "excluded" => LinkSource::Excluded,
        "rule" => {
            let id = rule_id.ok_or_else(|| {
                conv(CoreError::Validation(
                    "rule link is missing rule_id".to_owned(),
                ))
            })?;
            LinkSource::Rule(id)
        }
        other => {
            return Err(conv(CoreError::Validation(format!(
                "unknown link source {other}"
            ))));
        }
    };
    Ok(PrProjectLink {
        pr: PrId(row.get("pr_id")?),
        project: ProjectId(row.get("project_id")?),
        source,
        created_at: ts_col(row.get("created_at")?)?,
    })
}
