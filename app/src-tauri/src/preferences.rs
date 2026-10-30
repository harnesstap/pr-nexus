//! Settings → Mappings, Review, Sync and About (§8.2), plus the
//! "update available" check (D10).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pr_nexus_core::domain::MappingLayer;
use pr_nexus_core::mapping::{self, CsvRow, ExportMode, ImportMode};
use pr_nexus_core::settings::{self, Preferences};
use pr_nexus_core::update;
use pr_nexus_providers::vault::KeyringState;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::blocking;
use crate::error::AppError;
use crate::state::{self, AppState};

type Shared<'a> = State<'a, Arc<AppState>>;

/// Where the app keeps its files, for Settings → About.
pub struct AppPaths {
    pub db: PathBuf,
    pub logs: PathBuf,
}

#[tauri::command]
pub async fn load_preferences(app: Shared<'_>) -> Result<Preferences, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || Ok(settings::preferences(&app.inner().store)?)).await
}

#[tauri::command]
pub async fn save_preferences(
    app: Shared<'_>,
    prefs: Preferences,
) -> Result<Preferences, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let saved = settings::save_preferences(&app.inner().store, &prefs)?;
        // The live toggle applies now, not at the next due poll.
        app.inner().live.nudge();
        tracing::info!("settings saved");
        Ok(saved)
    })
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MappingRowDto {
    pub github_org: String,
    pub team_slug: String,
    pub team_name: String,
    pub slack_workspace: String,
    /// Slack team (`T…`), or empty: routes before the domain (§7.11).
    #[serde(default)]
    pub slack_team_id: String,
    pub slack_channel_name: String,
    pub slack_channel_id: String,
}

impl From<CsvRow> for MappingRowDto {
    fn from(row: CsvRow) -> Self {
        Self {
            github_org: row.github_org,
            team_slug: row.team_slug,
            team_name: row.team_name,
            slack_workspace: row.slack_workspace,
            slack_team_id: row.slack_team_id,
            slack_channel_name: row.slack_channel_name,
            slack_channel_id: row.slack_channel_id,
        }
    }
}

impl From<MappingRowDto> for CsvRow {
    fn from(row: MappingRowDto) -> Self {
        Self {
            github_org: row.github_org,
            team_slug: row.team_slug,
            team_name: row.team_name,
            slack_channel_name: row.slack_channel_name,
            slack_channel_id: row.slack_channel_id,
            slack_workspace: row.slack_workspace,
            slack_team_id: row.slack_team_id,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MappingEntryDto {
    pub row: MappingRowDto,
    /// `"user"` (edited in Settings or merged) or `"imported"`.
    pub source: &'static str,
    pub disabled: bool,
    pub has_imported: bool,
}

#[tauri::command]
pub async fn list_mappings(app: Shared<'_>) -> Result<Vec<MappingEntryDto>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let rows = app.inner().store.list_mappings()?;
        Ok(mapping::mapping_table(&rows)
            .into_iter()
            .map(|entry| MappingEntryDto {
                row: entry.row.into(),
                source: match entry.layer {
                    MappingLayer::User => "user",
                    MappingLayer::Imported => "imported",
                },
                disabled: entry.disabled,
                has_imported: entry.has_imported,
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn save_mapping(
    app: Shared<'_>,
    row: MappingRowDto,
    disabled: bool,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        mapping::save_user_mapping(&app.inner().store, row.into(), disabled, Utc::now())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn reset_mapping(app: Shared<'_>, org: String, slug: String) -> Result<bool, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || Ok(mapping::reset_to_imported(&app.inner().store, &org, &slug)?)).await
}

#[derive(Debug, Serialize)]
pub struct FieldChangeDto {
    pub field: &'static str,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Serialize)]
pub struct ChangedDto {
    pub row: MappingRowDto,
    pub changes: Vec<FieldChangeDto>,
}

#[derive(Debug, Serialize)]
pub struct ImportPreviewDto {
    pub added: Vec<MappingRowDto>,
    pub changed: Vec<ChangedDto>,
    pub unchanged: usize,
    /// Replace only: imported rows a user edit still overrides.
    pub user_shadowed: Vec<MappingRowDto>,
}

fn import_mode(mode: &str) -> Result<ImportMode, AppError> {
    match mode {
        "merge" => Ok(ImportMode::Merge),
        "replace" => Ok(ImportMode::Replace),
        other => Err(AppError::new(
            "validation",
            format!("unknown import mode {other}"),
        )),
    }
}

/// Dry run: what an import would add or change. Writes nothing.
#[tauri::command]
pub async fn preview_mapping_import(
    app: Shared<'_>,
    csv: String,
    workspace: String,
    mode: String,
) -> Result<ImportPreviewDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let mode = import_mode(&mode)?;
        let incoming = mapping::parse_csv(&csv, Some(workspace.trim()))?;
        let existing = app.inner().store.list_mappings()?;
        let diff = mapping::plan_import(&existing, &incoming, mode);
        Ok(ImportPreviewDto {
            added: diff.added.into_iter().map(Into::into).collect(),
            changed: diff
                .changed
                .into_iter()
                .map(|changed| ChangedDto {
                    row: changed.row.into(),
                    changes: changed
                        .changes
                        .into_iter()
                        .map(|change| FieldChangeDto {
                            field: change.field,
                            before: change.before,
                            after: change.after,
                        })
                        .collect(),
                })
                .collect(),
            unchanged: diff.unchanged.len(),
            user_shadowed: diff.user_shadowed.into_iter().map(Into::into).collect(),
        })
    })
    .await
}

#[derive(Debug, Serialize)]
pub struct ImportedDto {
    pub rows: usize,
}

#[tauri::command]
pub async fn apply_mapping_import(
    app: Shared<'_>,
    csv: String,
    workspace: String,
    mode: String,
) -> Result<ImportedDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let mode = import_mode(&mode)?;
        let incoming = mapping::parse_csv(&csv, Some(workspace.trim()))?;
        mapping::apply_import(&app.inner().store, &incoming, mode, Utc::now())?;
        tracing::info!(rows = incoming.len(), "mapping imported");
        Ok(ImportedDto {
            rows: incoming.len(),
        })
    })
    .await
}

#[tauri::command]
pub async fn export_mapping(app: Shared<'_>, mode: String) -> Result<String, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let mode = match mode.as_str() {
            "effective" => ExportMode::Effective,
            "user" => ExportMode::User,
            "imported" => ExportMode::Imported,
            other => {
                return Err(AppError::new(
                    "validation",
                    format!("unknown export mode {other}"),
                ));
            }
        };
        let rows = app.inner().store.list_mappings()?;
        Ok(mapping::export_csv(&rows, mode)?)
    })
    .await
}

#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub version: String,
    pub repository: String,
    pub db_path: String,
    pub log_dir: String,
    pub keyring_in_memory: bool,
    pub os: String,
}

#[tauri::command]
pub async fn app_info(app: Shared<'_>, paths: State<'_, AppPaths>) -> Result<AppInfo, AppError> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        repository: update::repo_slug(env!("CARGO_PKG_REPOSITORY")).unwrap_or_default(),
        db_path: paths.db.display().to_string(),
        log_dir: paths.logs.display().to_string(),
        keyring_in_memory: !matches!(
            app.inner().vault.status().state,
            KeyringState::Ready | KeyringState::Waiting
        ),
        os: std::env::consts::OS.to_owned(),
    })
}

/// Release builds compare tags; debug builds compare their commit with `main`.
fn channel() -> update::Channel {
    if cfg!(debug_assertions) {
        let sha = env!("PR_NEXUS_GIT_SHA");
        update::Channel::Dev {
            sha: (!sha.is_empty()).then(|| sha.to_owned()),
        }
    } else {
        update::Channel::Release
    }
}

/// `reqwest` behind the core's `HttpGet`; logs what the core keeps silent.
struct GithubGet {
    client: reqwest::blocking::Client,
    token: Option<String>,
}

impl update::HttpGet for GithubGet {
    fn get(&self, url: &str, accept: &str) -> Result<update::HttpResponse, String> {
        let mut request = self
            .client
            .get(url)
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().map_err(|err| {
            tracing::warn!(error = %err, "update check failed");
            err.to_string()
        })?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            tracing::info!(status, url, "update check: no answer");
        }
        Ok(update::HttpResponse {
            status,
            body: response.text().unwrap_or_default(),
        })
    }
}

/// Latest release (release builds) or `main` (dev builds) on GitHub, read
/// with the user's token when one is set, anonymously otherwise. Any failure
/// means "no update": the check is best effort.
#[tauri::command]
pub async fn check_update(app: Shared<'_>) -> Result<Option<update::UpdateInfo>, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || Ok(check_update_blocking(&state)))
        .await
        .map_err(|err| AppError::new("internal", err.to_string()))?
}

fn check_update_blocking(state: &AppState) -> Option<update::UpdateInfo> {
    if state.demo {
        return None;
    }
    let repo = update::repo_slug(env!("CARGO_PKG_REPOSITORY"))?;
    let account = state::github_account(&state.store).ok().flatten();
    let token = state::resolve_github_token(state, account.as_ref()).ok();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("pr-nexus/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let http = GithubGet { client, token };
    update::check(&http, &repo, env!("CARGO_PKG_VERSION"), &channel())
}
