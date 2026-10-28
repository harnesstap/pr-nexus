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

#[derive(Debug, Serialize, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    /// The download that skips the quarantine flag on macOS (§9).
    pub command: String,
}

fn update_from(current: &str, repo: &str, release: Release) -> Option<UpdateInfo> {
    if release.draft || release.prerelease || !update::is_newer(current, &release.tag_name) {
        return None;
    }
    Some(UpdateInfo {
        command: format!(
            "gh release download {} -R {repo} -p '*.dmg'",
            release.tag_name
        ),
        version: release.tag_name.trim_start_matches('v').to_owned(),
        url: release.html_url,
    })
}

/// Latest release on GitHub, read with the user's own token (the repo can be
/// private). Any failure means "no banner": the check is best effort.
#[tauri::command]
pub async fn check_update(app: Shared<'_>) -> Result<Option<UpdateInfo>, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || Ok(check_update_blocking(&state)))
        .await
        .map_err(|err| AppError::new("internal", err.to_string()))?
}

fn check_update_blocking(state: &AppState) -> Option<UpdateInfo> {
    if state.demo {
        return None;
    }
    let repo = update::repo_slug(env!("CARGO_PKG_REPOSITORY"))?;
    let account = state::github_account(&state.store).ok()?;
    let token = state::resolve_github_token(state, account.as_ref()).ok()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("pr-nexus/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send();
    let response = match response {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            tracing::info!(status = %response.status(), "update check: no release");
            return None;
        }
        Err(err) => {
            tracing::warn!(error = %err, "update check failed");
            return None;
        }
    };
    let release: Release = response.json().ok()?;
    update_from(env!("CARGO_PKG_VERSION"), &repo, release)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: format!("https://github.com/acme/widgets/releases/tag/{tag}"),
            draft: false,
            prerelease: false,
        }
    }

    #[test]
    fn a_newer_final_release_becomes_a_banner() {
        let info = update_from("0.1.0", "acme/widgets", release("v0.2.0")).unwrap();
        assert_eq!(info.version, "0.2.0");
        assert_eq!(
            info.command,
            "gh release download v0.2.0 -R acme/widgets -p '*.dmg'"
        );
        assert!(update_from("0.2.0", "acme/widgets", release("v0.2.0")).is_none());
        let mut pre = release("v0.3.0");
        pre.prerelease = true;
        assert!(update_from("0.2.0", "acme/widgets", pre).is_none());
    }
}
