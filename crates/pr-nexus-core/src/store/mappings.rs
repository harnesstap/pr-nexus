use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{MappingLayer, TeamChannelMapping};
use crate::error::CoreError;

use super::row::parse_enum;
use super::time::{format_ts, parse_bool, sql_bool, ts_col};
use super::{Store, db};

impl Store {
    pub fn upsert_mapping(&self, row: TeamChannelMapping) -> Result<TeamChannelMapping, CoreError> {
        self.write(move |conn| upsert_mapping(conn, &row))
    }

    /// Upsert every row in one transaction: all of them land, or none do.
    pub fn upsert_mappings(&self, rows: Vec<TeamChannelMapping>) -> Result<(), CoreError> {
        self.write(move |conn| {
            let tx = conn.transaction().map_err(db)?;
            for row in &rows {
                upsert_mapping(&tx, row)?;
            }
            tx.commit().map_err(db)
        })
    }

    pub fn list_mappings(&self) -> Result<Vec<TeamChannelMapping>, CoreError> {
        self.read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM team_channel_mappings
                     ORDER BY github_org, team_slug, layer",
                )
                .map_err(db)?;
            let rows = stmt
                .query_map([], map_mapping)
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }

    /// After a routed send, point the mappings that named this channel in
    /// the `from` workspace (team, or domain when no team was stored) at
    /// the workspace that worked (§7.11). Returns the rows changed.
    pub fn pin_mapping_team(
        &self,
        channel_id: String,
        from_team: Option<String>,
        from_domain: String,
        team_id: String,
        domain: String,
    ) -> Result<usize, CoreError> {
        self.write(move |conn| {
            conn.execute(
                "UPDATE team_channel_mappings SET slack_team_id = ?1, slack_workspace = ?2
                 WHERE slack_channel_id = ?3
                   AND (slack_team_id = ?4
                        OR (slack_team_id IS NULL AND lower(slack_workspace) = lower(?5)))",
                params![team_id, domain, channel_id, from_team, from_domain],
            )
            .map_err(db)
        })
    }

    pub fn delete_mapping(&self, id: i64) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute("DELETE FROM team_channel_mappings WHERE id = ?1", [id])
                .map_err(db)?;
            Ok(())
        })
    }

    /// Delete every imported row and insert `rows`, in one transaction.
    /// User-layer rows are left in place.
    pub fn replace_imported_mappings(
        &self,
        rows: Vec<TeamChannelMapping>,
    ) -> Result<(), CoreError> {
        self.write(move |conn| {
            if rows.iter().any(|row| row.layer != MappingLayer::Imported) {
                return Err(CoreError::Validation(
                    "replace_imported_mappings only inserts imported rows".into(),
                ));
            }
            let tx = conn.transaction().map_err(db)?;
            tx.execute(
                "DELETE FROM team_channel_mappings WHERE layer = 'imported'",
                [],
            )
            .map_err(db)?;
            for row in &rows {
                upsert_mapping(&tx, row)?;
            }
            tx.commit().map_err(db)
        })
    }

    pub fn mapping_count(&self) -> Result<i64, CoreError> {
        self.read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM team_channel_mappings", [], |row| {
                row.get(0)
            })
            .map_err(db)
        })
    }
}

fn upsert_mapping(
    conn: &Connection,
    row: &TeamChannelMapping,
) -> Result<TeamChannelMapping, CoreError> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO team_channel_mappings (
                layer, github_org, team_slug, team_name, slack_workspace,
                slack_channel_id, slack_channel_name, disabled, note, updated_at, slack_team_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(layer, github_org, team_slug) DO UPDATE SET
                team_name = excluded.team_name,
                slack_workspace = excluded.slack_workspace,
                slack_team_id = excluded.slack_team_id,
                slack_channel_id = excluded.slack_channel_id,
                slack_channel_name = excluded.slack_channel_name,
                disabled = excluded.disabled,
                note = excluded.note,
                updated_at = excluded.updated_at
             RETURNING id",
            params![
                row.layer.as_str(),
                row.github_org,
                row.team_slug,
                row.team_name,
                row.slack_workspace,
                row.slack_channel_id,
                row.slack_channel_name,
                sql_bool(row.disabled),
                row.note,
                format_ts(row.updated_at),
                row.slack_team_id,
            ],
            |r| r.get(0),
        )
        .map_err(db)?;
    conn.query_row(
        "SELECT * FROM team_channel_mappings WHERE id = ?1",
        [id],
        map_mapping,
    )
    .optional()
    .map_err(db)?
    .ok_or_else(|| CoreError::Store("upserted mapping disappeared".to_owned()))
}

fn map_mapping(row: &rusqlite::Row<'_>) -> rusqlite::Result<TeamChannelMapping> {
    Ok(TeamChannelMapping {
        id: row.get("id")?,
        layer: parse_enum::<MappingLayer>(row.get("layer")?)?,
        github_org: row.get("github_org")?,
        team_slug: row.get("team_slug")?,
        team_name: row.get("team_name")?,
        slack_workspace: row.get("slack_workspace")?,
        slack_team_id: row.get("slack_team_id")?,
        slack_channel_id: row.get("slack_channel_id")?,
        slack_channel_name: row.get("slack_channel_name")?,
        disabled: parse_bool(row.get("disabled")?),
        note: row.get("note")?,
        updated_at: ts_col(row.get("updated_at")?)?,
    })
}
