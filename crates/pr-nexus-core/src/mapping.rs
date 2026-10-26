//! Team-to-channel CSV import and export.
//!
//! The mapping starts empty. A file uses the fixed header
//! `github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id`
//! and may add `slack_workspace`. Rows are quoted with RFC 4180 rules and
//! exports are sorted by `(org, slug)`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::domain::{MappingLayer, TeamChannelMapping};
use crate::error::CoreError;
use crate::store::Store;

const HEADER: [&str; 5] = [
    "github_org",
    "github_team_slug",
    "github_team_name",
    "slack_channel_name",
    "slack_channel_id",
];
const WORKSPACE: &str = "slack_workspace";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Replace,
    Merge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportMode {
    Effective,
    User,
    Imported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvRow {
    pub github_org: String,
    pub team_slug: String,
    pub team_name: String,
    pub slack_channel_name: String,
    pub slack_channel_id: String,
    pub slack_workspace: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange {
    pub field: &'static str,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedMapping {
    pub row: CsvRow,
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDiff {
    pub added: Vec<CsvRow>,
    pub changed: Vec<ChangedMapping>,
    pub unchanged: Vec<CsvRow>,
    pub user_shadowed: Vec<CsvRow>,
}

/// Parse a mapping CSV.
///
/// `default_workspace` fills rows that omit `slack_workspace`. It is required
/// when any row lacks one. Channel names lose a leading `#`. A non-empty
/// channel id must look like `C…` or `G…`.
pub fn parse_csv(text: &str, default_workspace: Option<&str>) -> Result<Vec<CsvRow>, CoreError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|err| CoreError::Validation(format!("mapping csv: {err}")))?
        .clone();
    validate_headers(&headers)?;
    let mut rows = Vec::new();
    let mut seen = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|err| CoreError::Validation(format!("mapping csv: {err}")))?;
        let row = row_from_record(&headers, &record, index + 2)?;
        let key = (row.github_org.clone(), row.team_slug.clone());
        if seen.contains(&key) {
            return Err(CoreError::Validation(format!(
                "duplicate mapping for {}/{}",
                key.0, key.1
            )));
        }
        seen.push(key);
        rows.push(row);
    }
    if rows.iter().any(|row| row.slack_workspace.is_empty()) {
        let Some(workspace) = default_workspace.filter(|workspace| !workspace.is_empty()) else {
            return Err(CoreError::Validation(
                "target workspace is required when a row omits slack_workspace".into(),
            ));
        };
        for row in &mut rows {
            if row.slack_workspace.is_empty() {
                row.slack_workspace = workspace.to_string();
            }
        }
    }
    Ok(rows)
}

pub fn plan_import(
    existing: &[TeamChannelMapping],
    incoming: &[CsvRow],
    mode: ImportMode,
) -> ImportDiff {
    let mut diff = ImportDiff {
        added: Vec::new(),
        changed: Vec::new(),
        unchanged: Vec::new(),
        user_shadowed: Vec::new(),
    };
    for row in incoming {
        let current = existing.iter().find(|stored| {
            stored.layer == target_layer(mode)
                && stored.github_org == row.github_org
                && stored.team_slug == row.team_slug
        });
        match current {
            None => diff.added.push(row.clone()),
            Some(current) => {
                let changes = field_changes(current, row, mode);
                if changes.is_empty() {
                    diff.unchanged.push(row.clone());
                } else {
                    diff.changed.push(ChangedMapping {
                        row: row.clone(),
                        changes,
                    });
                }
            }
        }
        if mode == ImportMode::Replace
            && existing.iter().any(|stored| {
                stored.layer == MappingLayer::User
                    && stored.github_org == row.github_org
                    && stored.team_slug == row.team_slug
            })
        {
            diff.user_shadowed.push(row.clone());
        }
    }
    diff
}

pub fn apply_import(
    store: &Store,
    incoming: &[CsvRow],
    mode: ImportMode,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    match mode {
        ImportMode::Replace => {
            let rows = incoming
                .iter()
                .map(|row| to_stored(row, MappingLayer::Imported, false, String::new(), now))
                .collect();
            store.replace_imported_mappings(rows)
        }
        ImportMode::Merge => {
            let existing = store.list_mappings()?;
            for row in incoming {
                let current = existing.iter().find(|stored| {
                    stored.layer == MappingLayer::User
                        && stored.github_org == row.github_org
                        && stored.team_slug == row.team_slug
                });
                let merged = match current {
                    Some(current) => merge_user(current, row, now),
                    None => to_stored(row, MappingLayer::User, false, String::new(), now),
                };
                store.upsert_mapping(merged)?;
            }
            Ok(())
        }
    }
}

pub fn export_csv(rows: &[TeamChannelMapping], mode: ExportMode) -> Result<String, CoreError> {
    let selected = select_rows(rows, mode);
    let include_workspace = selected.iter().any(|row| !row.slack_workspace.is_empty());
    let mut header: Vec<&str> = HEADER.to_vec();
    if include_workspace {
        header.push(WORKSPACE);
    }
    let mut writer = csv::WriterBuilder::new().from_writer(Vec::new());
    writer
        .write_record(&header)
        .map_err(|err| CoreError::Validation(format!("mapping csv: {err}")))?;
    for row in selected {
        let mut record = vec![
            row.github_org,
            row.team_slug,
            row.team_name,
            row.slack_channel_name,
            row.slack_channel_id,
        ];
        if include_workspace {
            record.push(row.slack_workspace);
        }
        writer
            .write_record(&record)
            .map_err(|err| CoreError::Validation(format!("mapping csv: {err}")))?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|err| CoreError::Validation(format!("mapping csv: {err}")))?;
    String::from_utf8(bytes).map_err(|err| CoreError::Validation(format!("mapping csv: {err}")))
}

/// User layer wins for `(org, slug)` regardless of workspace. A disabled user
/// row hides the imported row. Slug is tried first, then an exact team name.
pub fn lookup_mapping<'a>(
    rows: &'a [TeamChannelMapping],
    org: &str,
    slug: &str,
    team_name: &str,
) -> Option<&'a TeamChannelMapping> {
    match pick_layer(rows, |row| row.github_org == org && row.team_slug == slug) {
        LayerPick::Hit(row) => return Some(row),
        LayerPick::Hidden => return None,
        LayerPick::Miss => {}
    }
    if team_name.is_empty() {
        return None;
    }
    match pick_layer(rows, |row| {
        row.github_org == org && !row.team_name.is_empty() && row.team_name == team_name
    }) {
        LayerPick::Hit(row) => Some(row),
        LayerPick::Hidden | LayerPick::Miss => None,
    }
}

enum LayerPick<'a> {
    Hit(&'a TeamChannelMapping),
    /// A disabled user row hides the imported row and blocks the name fallback.
    Hidden,
    Miss,
}

fn pick_layer<'a>(
    rows: &'a [TeamChannelMapping],
    pred: impl Fn(&TeamChannelMapping) -> bool,
) -> LayerPick<'a> {
    if let Some(user) = rows
        .iter()
        .find(|row| row.layer == MappingLayer::User && pred(row))
    {
        return if user.disabled {
            LayerPick::Hidden
        } else {
            LayerPick::Hit(user)
        };
    }
    match rows
        .iter()
        .find(|row| row.layer == MappingLayer::Imported && !row.disabled && pred(row))
    {
        Some(row) => LayerPick::Hit(row),
        None => LayerPick::Miss,
    }
}

fn select_rows(rows: &[TeamChannelMapping], mode: ExportMode) -> Vec<CsvRow> {
    let mut by_key: BTreeMap<(String, String), CsvRow> = BTreeMap::new();
    match mode {
        ExportMode::Imported => {
            for row in rows
                .iter()
                .filter(|row| row.layer == MappingLayer::Imported && !row.disabled)
            {
                by_key.insert(key(row), from_stored(row));
            }
        }
        ExportMode::User => {
            for row in rows
                .iter()
                .filter(|row| row.layer == MappingLayer::User && !row.disabled)
            {
                by_key.insert(key(row), from_stored(row));
            }
        }
        ExportMode::Effective => {
            let mut keys: Vec<(String, String)> = rows.iter().map(key).collect();
            keys.sort();
            keys.dedup();
            for (org, slug) in keys {
                if let Some(found) = lookup_mapping(rows, &org, &slug, "") {
                    by_key.insert((org, slug), from_stored(found));
                }
            }
        }
    }
    by_key.into_values().collect()
}

fn key(row: &TeamChannelMapping) -> (String, String) {
    (row.github_org.clone(), row.team_slug.clone())
}

fn target_layer(mode: ImportMode) -> MappingLayer {
    match mode {
        ImportMode::Replace => MappingLayer::Imported,
        ImportMode::Merge => MappingLayer::User,
    }
}

fn field_changes(
    current: &TeamChannelMapping,
    incoming: &CsvRow,
    mode: ImportMode,
) -> Vec<FieldChange> {
    let pairs = [
        (
            "github_team_name",
            current.team_name.as_str(),
            incoming.team_name.as_str(),
        ),
        (
            "slack_channel_name",
            current.slack_channel_name.as_str(),
            incoming.slack_channel_name.as_str(),
        ),
        (
            "slack_channel_id",
            current.slack_channel_id.as_str(),
            incoming.slack_channel_id.as_str(),
        ),
        (
            "slack_workspace",
            current.slack_workspace.as_str(),
            incoming.slack_workspace.as_str(),
        ),
    ];
    pairs
        .into_iter()
        .filter(|(_, before, after)| match mode {
            ImportMode::Replace => before != after,
            ImportMode::Merge => !after.is_empty() && before != after,
        })
        .map(|(field, before, after)| FieldChange {
            field,
            before: before.to_string(),
            after: after.to_string(),
        })
        .collect()
}

fn merge_user(
    current: &TeamChannelMapping,
    incoming: &CsvRow,
    now: DateTime<Utc>,
) -> TeamChannelMapping {
    let mut row = current.clone();
    if !incoming.team_name.is_empty() {
        row.team_name = incoming.team_name.clone();
    }
    if !incoming.slack_channel_name.is_empty() {
        row.slack_channel_name = incoming.slack_channel_name.clone();
    }
    if !incoming.slack_channel_id.is_empty() {
        row.slack_channel_id = incoming.slack_channel_id.clone();
    }
    if !incoming.slack_workspace.is_empty() {
        row.slack_workspace = incoming.slack_workspace.clone();
    }
    row.updated_at = now;
    row
}

fn to_stored(
    row: &CsvRow,
    layer: MappingLayer,
    disabled: bool,
    note: String,
    now: DateTime<Utc>,
) -> TeamChannelMapping {
    TeamChannelMapping {
        id: 0,
        layer,
        github_org: row.github_org.clone(),
        team_slug: row.team_slug.clone(),
        team_name: row.team_name.clone(),
        slack_workspace: row.slack_workspace.clone(),
        slack_channel_id: row.slack_channel_id.clone(),
        slack_channel_name: row.slack_channel_name.clone(),
        disabled,
        note,
        updated_at: now,
    }
}

fn from_stored(row: &TeamChannelMapping) -> CsvRow {
    CsvRow {
        github_org: row.github_org.clone(),
        team_slug: row.team_slug.clone(),
        team_name: row.team_name.clone(),
        slack_channel_name: row.slack_channel_name.clone(),
        slack_channel_id: row.slack_channel_id.clone(),
        slack_workspace: row.slack_workspace.clone(),
    }
}

fn validate_headers(headers: &csv::StringRecord) -> Result<(), CoreError> {
    let names: Vec<&str> = headers.iter().collect();
    if names.is_empty() {
        return Err(CoreError::Validation(
            "mapping csv is missing a header".into(),
        ));
    }
    for name in &names {
        if !HEADER.contains(name) && *name != WORKSPACE {
            return Err(CoreError::Validation(format!(
                "unknown mapping column: {name}"
            )));
        }
    }
    for required in ["github_org", "github_team_slug"] {
        if !names.contains(&required) {
            return Err(CoreError::Validation(format!(
                "mapping csv is missing {required}"
            )));
        }
    }
    Ok(())
}

fn row_from_record(
    headers: &csv::StringRecord,
    record: &csv::StringRecord,
    line: usize,
) -> Result<CsvRow, CoreError> {
    let mut values = BTreeMap::new();
    for (header, value) in headers.iter().zip(record.iter()) {
        values.insert(header, value.trim().to_string());
    }
    let org = values.get("github_org").cloned().unwrap_or_default();
    let slug = values.get("github_team_slug").cloned().unwrap_or_default();
    if org.is_empty() || slug.is_empty() {
        return Err(CoreError::Validation(format!(
            "mapping csv line {line}: github_org and github_team_slug are required"
        )));
    }
    let mut channel_name = values
        .get("slack_channel_name")
        .cloned()
        .unwrap_or_default();
    channel_name = channel_name.trim_start_matches('#').to_string();
    let channel_id = values.get("slack_channel_id").cloned().unwrap_or_default();
    if !channel_id.is_empty() && !valid_channel_id(&channel_id) {
        return Err(CoreError::Validation(format!(
            "mapping csv line {line}: slack_channel_id must look like C… or G…"
        )));
    }
    Ok(CsvRow {
        github_org: org,
        team_slug: slug,
        team_name: values.get("github_team_name").cloned().unwrap_or_default(),
        slack_channel_name: channel_name,
        slack_channel_id: channel_id,
        slack_workspace: values.get(WORKSPACE).cloned().unwrap_or_default(),
    })
}

fn valid_channel_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    matches!(first, 'C' | 'G')
        && chars.clone().next().is_some()
        && chars.all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 26, 0, 0, 0).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn stored(
        layer: MappingLayer,
        org: &str,
        slug: &str,
        name: &str,
        workspace: &str,
        channel_id: &str,
        channel_name: &str,
        disabled: bool,
    ) -> TeamChannelMapping {
        TeamChannelMapping {
            id: 0,
            layer,
            github_org: org.into(),
            team_slug: slug.into(),
            team_name: name.into(),
            slack_workspace: workspace.into(),
            slack_channel_id: channel_id.into(),
            slack_channel_name: channel_name.into(),
            disabled,
            note: String::new(),
            updated_at: now(),
        }
    }

    #[test]
    fn parses_the_five_column_fixture_and_a_workspace_column() {
        let text = std::fs::read_to_string(format!(
            "{}/tests/fixtures/mappings/acme-five-column.csv",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let rows = parse_csv(&text, Some("acme")).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].slack_channel_name, "team-reviews");
        assert_eq!(rows[0].slack_channel_id, "C0123456789");
        assert_eq!(rows[0].slack_workspace, "acme");
        assert_eq!(rows[1].slack_channel_id, "G0123456789");

        let with_workspace = "\
github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id,slack_workspace
acme,team-a,Team A,team-reviews,C0123456789,widgets
";
        let rows = parse_csv(with_workspace, None).unwrap();
        assert_eq!(rows[0].slack_workspace, "widgets");
    }

    #[test]
    fn rejects_unknown_columns_duplicates_bad_ids_and_a_missing_workspace() {
        let err = parse_csv(
            "github_org,github_team_slug,notes\nacme,team-a,x\n",
            Some("acme"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown mapping column"));
        let err = parse_csv(
            "github_org,github_team_slug\nacme,team-a\nacme,team-a\n",
            Some("acme"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate"));
        let err = parse_csv(
            "github_org,github_team_slug,slack_channel_id\nacme,team-a,D123\n",
            Some("acme"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("C…"));
        let err = parse_csv("github_org,github_team_slug\nacme,team-a\n", None).unwrap_err();
        assert!(err.to_string().contains("target workspace"));
    }

    #[test]
    fn quotes_exports_and_sorts_by_org_and_slug() {
        let rows = vec![
            stored(
                MappingLayer::Imported,
                "acme",
                "team-b",
                "Team B",
                "acme",
                "G0123456789",
                "say \"hi\"",
                false,
            ),
            stored(
                MappingLayer::Imported,
                "acme",
                "team-a",
                "Team, A",
                "acme",
                "C0123456789",
                "team-reviews",
                false,
            ),
        ];
        let csv = export_csv(&rows, ExportMode::Imported).unwrap();
        assert!(csv.starts_with(
            "github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id,slack_workspace\n"
        ));
        let team_a = csv.find("team-a").unwrap();
        let team_b = csv.find("team-b").unwrap();
        assert!(team_a < team_b);
        assert!(csv.contains("\"Team, A\""));
        assert!(csv.contains("\"say \"\"hi\"\"\""));
        let parsed = parse_csv(&csv, None).unwrap();
        assert_eq!(parsed[0].team_name, "Team, A");
        assert_eq!(parsed[1].slack_channel_name, "say \"hi\"");
    }

    #[test]
    fn user_row_beats_imported_and_a_tombstone_hides_it() {
        let rows = vec![
            stored(
                MappingLayer::Imported,
                "acme",
                "team-a",
                "Imported",
                "other",
                "C0123456789",
                "old",
                false,
            ),
            stored(
                MappingLayer::User,
                "acme",
                "team-a",
                "Mine",
                "acme",
                "C9990001112",
                "team-reviews",
                false,
            ),
        ];
        let found = lookup_mapping(&rows, "acme", "team-a", "Imported").unwrap();
        assert_eq!(found.layer, MappingLayer::User);
        assert_eq!(found.slack_workspace, "acme");

        let mut tombstone = rows;
        tombstone[1].disabled = true;
        assert!(lookup_mapping(&tombstone, "acme", "team-a", "Imported").is_none());

        let by_name = vec![stored(
            MappingLayer::Imported,
            "acme",
            "other-slug",
            "Team A",
            "acme",
            "C0123456789",
            "team-reviews",
            false,
        )];
        assert_eq!(
            lookup_mapping(&by_name, "acme", "missing", "Team A")
                .unwrap()
                .team_slug,
            "other-slug"
        );
    }

    #[test]
    fn replace_keeps_user_rows_and_merge_does_not_blank_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("nexus.sqlite")).unwrap();
        let imported = parse_csv(
            "github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id\n\
             acme,team-a,Team A,#team-reviews,C0123456789\n\
             acme,team-b,Team B,team-widgets,G0123456789\n",
            Some("acme"),
        )
        .unwrap();
        let diff = plan_import(&[], &imported, ImportMode::Replace);
        assert_eq!(diff.added.len(), 2);
        apply_import(&store, &imported, ImportMode::Replace, now()).unwrap();

        store
            .upsert_mapping(stored(
                MappingLayer::User,
                "acme",
                "team-a",
                "Custom",
                "acme",
                "C9990001112",
                "team-custom",
                false,
            ))
            .unwrap();

        let replacement = parse_csv(
            "github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id\n\
             acme,team-b,Team B renamed,team-widgets,G0123456789\n",
            Some("acme"),
        )
        .unwrap();
        let existing = store.list_mappings().unwrap();
        let diff = plan_import(&existing, &replacement, ImportMode::Replace);
        assert_eq!(diff.changed.len(), 1);
        assert!(
            diff.changed[0]
                .changes
                .iter()
                .any(|change| change.field == "github_team_name")
        );
        apply_import(&store, &replacement, ImportMode::Replace, now()).unwrap();
        let rows = store.list_mappings().unwrap();
        assert!(lookup_mapping(&rows, "acme", "team-a", "").unwrap().layer == MappingLayer::User);
        assert!(
            lookup_mapping(&rows, "acme", "team-a", "")
                .unwrap()
                .slack_channel_name
                == "team-custom"
        );
        assert!(
            rows.iter()
                .all(|row| { !(row.layer == MappingLayer::Imported && row.team_slug == "team-a") })
        );

        let merge = parse_csv(
            "github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id\n\
             acme,team-a,,team-renamed,\n",
            Some("acme"),
        )
        .unwrap();
        let diff = plan_import(&rows, &merge, ImportMode::Merge);
        assert_eq!(diff.changed.len(), 1);
        assert!(diff.user_shadowed.is_empty());
        apply_import(&store, &merge, ImportMode::Merge, now()).unwrap();
        let rows = store.list_mappings().unwrap();
        let user = lookup_mapping(&rows, "acme", "team-a", "").unwrap();
        assert_eq!(user.slack_channel_name, "team-renamed");
        assert_eq!(user.slack_channel_id, "C9990001112");
        assert_eq!(user.team_name, "Custom");

        let exported = export_csv(&rows, ExportMode::Effective).unwrap();
        let parsed = parse_csv(&exported, None).unwrap();
        assert_eq!(parsed[0].team_slug, "team-a");
        assert_eq!(parsed[0].slack_channel_id, "C9990001112");
        let user_only = export_csv(&rows, ExportMode::User).unwrap();
        assert!(!user_only.contains("team-b"));
        let imported_only = export_csv(&rows, ExportMode::Imported).unwrap();
        assert!(!imported_only.contains("team-a"));
        assert!(imported_only.contains("team-b"));
    }

    #[test]
    fn replace_reports_rows_the_user_layer_shadows() {
        let existing = vec![stored(
            MappingLayer::User,
            "acme",
            "team-a",
            "Custom",
            "acme",
            "C9990001112",
            "team-custom",
            false,
        )];
        let incoming = parse_csv(
            "github_org,github_team_slug,slack_channel_id\nacme,team-a,C0123456789\n",
            Some("acme"),
        )
        .unwrap();
        let diff = plan_import(&existing, &incoming, ImportMode::Replace);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.user_shadowed.len(), 1);
    }

    #[test]
    fn tombstones_are_omitted_from_csv_and_do_not_round_trip() {
        let rows = vec![
            stored(
                MappingLayer::User,
                "acme",
                "team-a",
                "Hidden",
                "acme",
                "C9990001112",
                "team-reviews",
                true,
            ),
            stored(
                MappingLayer::Imported,
                "acme",
                "team-b",
                "Team B",
                "acme",
                "C0123456789",
                "team-widgets",
                false,
            ),
            stored(
                MappingLayer::Imported,
                "acme",
                "team-c",
                "Gone",
                "acme",
                "G0123456789",
                "old",
                true,
            ),
        ];
        for mode in [
            ExportMode::Effective,
            ExportMode::User,
            ExportMode::Imported,
        ] {
            let csv = export_csv(&rows, mode).unwrap();
            assert!(
                !csv.contains("team-a") && !csv.contains("team-c"),
                "{mode:?} kept a tombstone:\n{csv}"
            );
        }
        let exported = export_csv(&rows, ExportMode::Effective).unwrap();
        let parsed = parse_csv(&exported, None).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].team_slug, "team-b");
        let user_only = export_csv(&rows, ExportMode::User).unwrap();
        let parsed_user = parse_csv(&user_only, Some("acme")).unwrap();
        assert!(parsed_user.is_empty());
    }
}
