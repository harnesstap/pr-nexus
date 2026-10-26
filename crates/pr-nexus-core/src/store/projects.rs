use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{LinkSource, PrId, PrProjectLink, Project, ProjectId, ProjectRule};
use crate::error::CoreError;

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

    pub fn delete_project_rule(&self, id: i64) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute("DELETE FROM project_rules WHERE id = ?1", [id])
                .map_err(db)?;
            Ok(())
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

fn map_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<PrProjectLink> {
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
