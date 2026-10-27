use std::collections::{HashMap, HashSet};

use chrono::Utc;
use pr_nexus_core::domain::{
    Account, AccountId, AccountKind, AuthMode, LinkSource, PrId, Project, ProjectId, ProjectRule,
    PullRequest, ReplacedLink, RuleKind, ViewState,
};
use pr_nexus_core::linking;
use pr_nexus_core::mapping::{self, ImportMode};
use pr_nexus_core::onboarding::{self, OnboardingState};
use pr_nexus_core::ownership::{self, OwnershipReport};
use pr_nexus_core::providers::{SetupStep, VcsProvider};
use pr_nexus_core::search;
use pr_nexus_core::settings::{self, DEFAULT_REDIRECT};
use pr_nexus_core::store::ListFacts;
use pr_nexus_core::sync::SyncKind;
use pr_nexus_providers::github::{self, GithubClient};
use pr_nexus_providers::vault::KeyringStatus;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::blocking;
use crate::error::AppError;
use crate::listing;
use crate::slack_link;
use crate::state::{self, AppState};

#[derive(Serialize)]
pub struct SyncDto {
    pub phase: String,
    pub message: String,
    pub synced_at: Option<String>,
    pub paused: bool,
}

/// The keychain banner (§9): state, text, and whether Retry is offered.
#[derive(Debug, Serialize)]
pub struct KeyringDto {
    /// `ready`, `waiting`, `timed_out`, `denied`, `no_backend`, `error`, or `disabled`.
    pub state: String,
    pub message: Option<String>,
    pub can_retry: bool,
    /// Some tokens are only in memory, not in the keychain yet.
    pub unsaved: bool,
}

impl From<KeyringStatus> for KeyringDto {
    fn from(status: KeyringStatus) -> Self {
        Self {
            state: status.state.as_str().to_owned(),
            message: status.message,
            can_retry: status.can_retry,
            unsaved: status.unsaved,
        }
    }
}

/// The keychain state, polled by the UI while a prompt is pending.
#[tauri::command]
pub async fn keyring_status(
    app: State<'_, std::sync::Arc<AppState>>,
) -> Result<KeyringDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || Ok(KeyringDto::from(app.vault.status()))).await
}

/// Ask the keychain again and save what is only in memory. Returns at once;
/// poll [`keyring_status`] for the answer.
#[tauri::command]
pub async fn keyring_retry(
    app: State<'_, std::sync::Arc<AppState>>,
) -> Result<KeyringDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || Ok(KeyringDto::from(app.vault.retry()))).await
}

#[derive(Serialize)]
pub struct Bootstrap {
    pub onboarding: OnboardingState,
    pub resume_step: String,
    pub steps: Vec<String>,
    pub keyring: KeyringDto,
    pub github_login: Option<String>,
    pub github_connected: bool,
    pub tracked_orgs: Vec<String>,
    pub include_personal: bool,
    pub org_choices: Vec<String>,
    pub orgs_needing_sso: Vec<String>,
    pub slack_client_id: String,
    pub slack_redirect_uri: String,
    pub slack_secret_saved: bool,
    pub slack_workspaces: Vec<crate::slack::SlackWorkspace>,
    /// `off`, `public`, or `all` (§7.10).
    pub slack_mentions: String,
    /// `review.bump_broadcast`: bumps are also sent to the channel.
    pub bump_broadcast: bool,
    pub mapping_count: i64,
    pub sync: SyncDto,
    pub env_token: bool,
    pub oauth_client_configured: bool,
    pub gh_ready: bool,
    pub gh_detail: String,
    /// False when `gh_ready`/`gh_detail` were not checked; see `probe_gh`.
    pub gh_probed: bool,
}

#[derive(Serialize)]
pub struct GithubSession {
    pub login: String,
    pub organizations: Vec<String>,
    pub organizations_needing_sso: Vec<String>,
}

#[derive(Serialize)]
pub struct DevicePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Serialize)]
pub struct DeviceResult {
    pub status: String,
    pub login: Option<String>,
}

#[derive(Serialize)]
pub struct ImportSummary {
    pub rows: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlackIndicator {
    pub requested_at: String,
    pub last_bumped_at: Option<String>,
    pub bump_count: u32,
    pub workspace: String,
    pub channel_id: String,
    pub channel_name: String,
    pub permalink: Option<String>,
    pub root_ts: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListRow {
    pub id: i64,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub owner: String,
    pub repo: String,
    pub author_login: String,
    pub head_ref: String,
    pub is_draft: bool,
    pub state: String,
    pub checks: String,
    pub review_decision: Option<String>,
    pub unresolved_threads: u32,
    pub total_threads: u32,
    pub involvement: u8,
    pub updated_at: String,
    pub created_at: String,
    pub hidden: bool,
    pub project_ids: Vec<i64>,
    pub readiness: String,
    pub readiness_notes: Vec<String>,
    /// GitHub reports merge conflicts (dirty merge state or conflicting).
    pub conflicts: bool,
    /// At least one reviewer or team is requested on GitHub.
    pub reviewers_requested: bool,
    pub slack: Option<SlackIndicator>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectDto {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub archived: bool,
    pub description: String,
    pub sort_order: i32,
}

#[derive(Deserialize)]
pub struct OnboardingPatch {
    pub step: String,
    pub completed: Vec<String>,
    pub mapping_skipped: bool,
    pub slack_skipped: bool,
    pub finished_at: Option<String>,
}

fn sync_dto(state: &AppState) -> SyncDto {
    let status = state.sync.lock().expect("sync status");
    SyncDto {
        phase: status.phase.clone(),
        message: status.message.clone(),
        synced_at: status.synced_at.map(|value| value.to_rfc3339()),
        paused: status.paused,
    }
}

#[derive(Serialize)]
pub struct GhStatus {
    pub gh_ready: bool,
    pub gh_detail: String,
}

fn gh_probe() -> GhStatus {
    let (gh_ready, gh_detail) = if github::github_token_from_env().is_some() {
        (true, "Using PR_NEXUS_GITHUB_TOKEN".to_owned())
    } else {
        match github::read_gh_token() {
            github::GhProbe::Ready { .. } => (true, "gh is logged in".to_owned()),
            github::GhProbe::Unavailable { reason } => (false, reason),
        }
    };
    GhStatus {
        gh_ready,
        gh_detail,
    }
}

/// Whether the gh CLI has a token, for the wizard's GitHub step.
#[tauri::command]
pub async fn probe_gh() -> Result<GhStatus, AppError> {
    blocking::run(|| Ok(gh_probe())).await
}

#[tauri::command]
pub async fn bootstrap(app: State<'_, std::sync::Arc<AppState>>) -> Result<Bootstrap, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let saved = onboarding::load(&state.store)?;
        let validity = state::validity(state, &saved)?;
        let resume = onboarding::resume_step(&state::registry_steps(), Some(&saved), validity);
        let account = state::github_account(&state.store)?;
        let mut orgs = settings::viewer_orgs(&state.store)?;
        for org in settings::seen_orgs(&state.store)? {
            if !orgs
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&org))
            {
                orgs.push(org);
            }
        }
        // Running `gh` costs a subprocess or two on every launch. The list
        // screen never needs it, so a finished setup leaves it to the wizard
        // (`probe_gh`) when the user re-runs setup.
        let gh_probed = saved.finished_at.is_none();
        let probe = if gh_probed {
            gh_probe()
        } else {
            GhStatus {
                gh_ready: false,
                gh_detail: String::new(),
            }
        };
        let oauth = github::oauth_client_id_from_env()
            .or(settings::oauth_client_id(&state.store)?.into())
            .filter(|value| !value.is_empty());
        Ok(Bootstrap {
            resume_step: resume.as_str().to_owned(),
            steps: state::registry_steps()
                .into_iter()
                .map(|step| step.as_str().to_owned())
                .collect(),
            keyring: KeyringDto::from(state.vault.status()),
            github_login: account.as_ref().map(|account| account.login.clone()),
            github_connected: account.is_some(),
            tracked_orgs: settings::tracked_orgs(&state.store)?,
            include_personal: settings::include_personal_repos(&state.store)?,
            org_choices: orgs,
            orgs_needing_sso: settings::sso_orgs(&state.store)?,
            slack_client_id: settings::slack_client_id(&state.store)?,
            slack_redirect_uri: settings::slack_redirect_uri(&state.store)?,
            slack_secret_saved: {
                let id = settings::slack_client_id(&state.store)?;
                !id.trim().is_empty()
                    && state
                        .vault
                        .peek(&format!("slack-client:{}", id.trim()))
                        .ok()
                        .flatten()
                        .is_some()
            },
            slack_workspaces: crate::slack::workspace_accounts(&state.store)?
                .iter()
                .map(|account| crate::slack::workspace_dto(state, account))
                .collect(),
            slack_mentions: settings::slack_mentions(&state.store)?.as_str().to_owned(),
            bump_broadcast: settings::bump_broadcast(&state.store)?,
            mapping_count: state.store.mapping_count()?,
            sync: sync_dto(state),
            env_token: github::github_token_from_env().is_some(),
            oauth_client_configured: oauth.is_some(),
            gh_ready: probe.gh_ready,
            gh_detail: probe.gh_detail,
            gh_probed,
            onboarding: saved,
        })
    })
    .await
}

#[tauri::command]
pub async fn save_providers(
    app: State<'_, std::sync::Arc<AppState>>,
    vcs: String,
    chat: String,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        if vcs != "github" || chat != "slack" {
            return Err(AppError::new(
                "validation",
                "Only GitHub and Slack can be selected",
            ));
        }
        settings::set_provider_kind(&state.store, &vcs, &chat)?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn save_onboarding(
    app: State<'_, std::sync::Arc<AppState>>,
    patch: OnboardingPatch,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        onboarding::save(
            &state.store,
            &OnboardingState {
                step: patch.step,
                completed: patch.completed,
                mapping_skipped: patch.mapping_skipped,
                slack_skipped: patch.slack_skipped,
                finished_at: patch.finished_at,
            },
        )?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn connect_github(
    app: State<'_, std::sync::Arc<AppState>>,
    mode: String,
    pat: Option<String>,
) -> Result<GithubSession, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let auth_mode = match mode.as_str() {
            "gh_cli" => AuthMode::GhCli,
            "pat" => AuthMode::Pat,
            other => {
                return Err(AppError::new(
                    "validation",
                    format!("unknown GitHub auth mode {other}"),
                ));
            }
        };
        let token = if let Some(token) = github::github_token_from_env() {
            token
        } else if auth_mode == AuthMode::GhCli {
            match github::read_gh_token() {
                github::GhProbe::Ready { token } => token,
                github::GhProbe::Unavailable { reason } => {
                    return Err(AppError::new("unauthorized", reason));
                }
            }
        } else {
            pat.filter(|value| !value.trim().is_empty())
                .ok_or_else(|| AppError::new("validation", "Paste a personal access token"))?
        };
        store_github(
            state,
            auth_mode,
            token,
            github::github_token_from_env().is_some(),
        )
    })
    .await
}

#[tauri::command]
pub async fn start_device_flow(
    app: State<'_, std::sync::Arc<AppState>>,
) -> Result<DevicePrompt, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let client_id = github::oauth_client_id_from_env()
            .or(settings::oauth_client_id(&state.store)?.into())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::new(
                    "unsupported",
                    "Device flow needs github.oauth_client_id or PR_NEXUS_GITHUB_OAUTH_CLIENT_ID",
                )
            })?;
        let http = reqwest_client()?;
        let started = github::start_device_flow(&http, "https://github.com", &client_id)?;
        *state.device.lock().expect("device") = Some(state::DeviceSession {
            device_code: started.device_code,
            client_id,
        });
        Ok(DevicePrompt {
            user_code: started.user_code,
            verification_uri: started.verification_uri,
            expires_in: started.expires_in,
            interval: started.interval,
        })
    })
    .await
}

#[tauri::command]
pub async fn poll_device_flow(
    app: State<'_, std::sync::Arc<AppState>>,
) -> Result<DeviceResult, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let session = state
            .device
            .lock()
            .expect("device")
            .as_ref()
            .map(|session| (session.client_id.clone(), session.device_code.clone()))
            .ok_or_else(|| AppError::new("validation", "Device flow has not started"))?;
        let http = reqwest_client()?;
        match github::poll_device_flow(&http, "https://github.com", &session.0, &session.1)? {
            github::DevicePoll::Pending { .. } => Ok(DeviceResult {
                status: "pending".into(),
                login: None,
            }),
            github::DevicePoll::Denied => Err(AppError::new(
                "unauthorized",
                "GitHub denied the device flow",
            )),
            github::DevicePoll::Expired => {
                Err(AppError::new("unauthorized", "The device code expired"))
            }
            github::DevicePoll::Token(token) => {
                let session = store_github(state, AuthMode::OauthDevice, token, false)?;
                *state.device.lock().expect("device") = None;
                Ok(DeviceResult {
                    status: "connected".into(),
                    login: Some(session.login),
                })
            }
        }
    })
    .await
}

fn store_github(
    state: &AppState,
    mode: AuthMode,
    token: String,
    from_env: bool,
) -> Result<GithubSession, AppError> {
    let client = GithubClient::new(AccountId(0), token.clone())?;
    let viewer = client.viewer()?;
    let key = format!("github:{}", viewer.id);
    let stored_mode = if from_env { AuthMode::GhCli } else { mode };
    let keychain_key = if stored_mode == AuthMode::GhCli {
        None
    } else {
        state.vault.set(&key, &token)?;
        Some(key)
    };
    state.store.upsert_account(Account {
        id: AccountId(0),
        kind: AccountKind::GitHub,
        display_name: viewer.login.clone(),
        host: "github.com".into(),
        external_id: viewer.id.clone(),
        slack_workspace: None,
        login: viewer.login.clone(),
        auth_mode: stored_mode,
        keychain_key,
        created_at: Utc::now(),
    })?;
    settings::set_viewer_orgs(&state.store, &viewer.organizations)?;
    settings::set_sso_orgs(&state.store, &viewer.organizations_needing_sso)?;
    Ok(GithubSession {
        login: viewer.login,
        organizations: viewer.organizations,
        organizations_needing_sso: viewer.organizations_needing_sso,
    })
}

#[tauri::command]
pub async fn save_orgs(
    app: State<'_, std::sync::Arc<AppState>>,
    orgs: Vec<String>,
    include_personal: bool,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        settings::set_tracked_orgs(&state.store, &orgs)?;
        settings::set_include_personal_repos(&state.store, include_personal)?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn save_slack_settings(
    app: State<'_, std::sync::Arc<AppState>>,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        settings::set_slack_client_id(&state.store, client_id.trim())?;
        let redirect = if redirect_uri.trim().is_empty() {
            DEFAULT_REDIRECT
        } else {
            redirect_uri.trim()
        };
        settings::set_slack_redirect_uri(&state.store, redirect)?;
        if let Some(secret) = client_secret.filter(|value| !value.is_empty()) {
            let id = client_id.trim();
            if !id.is_empty() {
                state.vault.set(&format!("slack-client:{id}"), &secret)?;
            }
        }
        // Open sessions carry the old client settings.
        state
            .chats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn slack_manifest_url(redirect_uri: String) -> Result<String, AppError> {
    let redirect = if redirect_uri.trim().is_empty() {
        DEFAULT_REDIRECT
    } else {
        redirect_uri.trim()
    };
    slack_link::create_slack_app_url(redirect).map_err(|err| AppError::new("validation", err))
}

#[tauri::command]
pub async fn import_mapping(
    app: State<'_, std::sync::Arc<AppState>>,
    csv: String,
    workspace: String,
) -> Result<ImportSummary, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let rows = mapping::parse_csv(&csv, Some(workspace.trim()))?;
        mapping::apply_import(&state.store, &rows, ImportMode::Merge, Utc::now())?;
        Ok(ImportSummary { rows: rows.len() })
    })
    .await
}

#[tauri::command]
pub async fn finish_onboarding(
    app: State<'_, std::sync::Arc<AppState>>,
) -> Result<SyncDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let mut saved = onboarding::load(&state.store)?;
        saved.step = SetupStep::Done.as_str().into();
        if !saved.completed.iter().any(|step| step == "done") {
            saved.completed.push("done".into());
        }
        saved.finished_at = Some(Utc::now().to_rfc3339());
        onboarding::save(&state.store, &saved)?;
        // The list opens on what is already stored; the first sync runs
        // behind it and the sidebar shows its progress (§8.2).
        if !state.demo {
            let mut status = state.sync.lock().expect("sync status");
            status.phase = "syncing".into();
            status.message = "Syncing…".into();
        }
        let background = app.inner().clone();
        std::thread::Builder::new()
            .name("pr-nexus-first-sync".into())
            .spawn(move || {
                let _ = state::run_sync(&background, SyncKind::Manual);
            })
            .map_err(|err| AppError::new("internal", format!("could not start sync: {err}")))?;
        Ok(sync_dto(state))
    })
    .await
}

#[tauri::command]
pub async fn sync_now(app: State<'_, std::sync::Arc<AppState>>) -> Result<SyncDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let _ = state::run_sync(state, SyncKind::Manual);
        Ok(sync_dto(state))
    })
    .await
}

#[tauri::command]
pub async fn sync_status(app: State<'_, std::sync::Arc<AppState>>) -> Result<SyncDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        Ok(sync_dto(state))
    })
    .await
}

#[tauri::command]
pub async fn set_focus(
    app: State<'_, std::sync::Arc<AppState>>,
    focused: bool,
) -> Result<(), AppError> {
    state::set_focused(app.inner(), focused);
    Ok(())
}

/// Every synced row of the GitHub account, built with one query per table.
fn list_items(state: &AppState) -> Result<Vec<listing::Item>, AppError> {
    let Some(account) = state::github_account(&state.store)? else {
        return Ok(Vec::new());
    };
    let flag = settings::blocked_review_required_is_ready(&state.store)?;
    let prs = state
        .store
        .list_pull_requests_for_account_slim(account.id)?;
    let mut facts = state.store.list_facts(account.id)?;
    Ok(prs.iter().map(|pr| to_item(&mut facts, pr, flag)).collect())
}

/// Ids matching `query` in SQLite: title, description, repo, number, author,
/// branch, and linked project names. `None` for a blank query.
fn search_ids(
    state: &AppState,
    projects: &[ProjectDto],
    query: &str,
) -> Result<Option<HashSet<i64>>, AppError> {
    let terms = search::terms(query);
    if terms.is_empty() {
        return Ok(None);
    }
    let Some(account) = state::github_account(&state.store)? else {
        return Ok(Some(HashSet::new()));
    };
    let folded: Vec<(i64, String)> = projects
        .iter()
        .map(|project| (project.id, search::fold(&project.name)))
        .collect();
    let hits = terms
        .iter()
        .map(|term| {
            folded
                .iter()
                .filter(|(_, name)| name.contains(term.as_str()))
                .map(|(id, _)| *id)
                .collect()
        })
        .collect();
    Ok(Some(
        state
            .store
            .search_pull_request_ids(account.id, terms, hits)?
            .into_iter()
            .map(|id| id.0)
            .collect(),
    ))
}

fn all_projects(state: &AppState) -> Result<Vec<ProjectDto>, AppError> {
    Ok(state
        .store
        .list_projects()?
        .into_iter()
        .map(project_dto)
        .collect())
}

/// One slice of the list plus counts over every match (§8.2).
#[tauri::command]
pub async fn list_page(
    app: State<'_, std::sync::Arc<AppState>>,
    request: listing::PageRequest,
) -> Result<listing::PageDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let projects = all_projects(state)?;
        let items = list_items(state)?;
        let found = search_ids(state, &projects, &request.query)?;
        Ok(listing::page(&items, &projects, &request, found.as_ref()))
    })
    .await
}

/// Rows by id, in the order asked, for dialogs that outlive the loaded pages.
#[tauri::command]
pub async fn rows_by_id(
    app: State<'_, std::sync::Arc<AppState>>,
    ids: Vec<i64>,
) -> Result<Vec<ListRow>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let mut by_id: HashMap<i64, ListRow> = list_items(state)?
            .into_iter()
            .map(|item| (item.row.id, item.row))
            .collect();
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    })
    .await
}

/// Open PRs the project dialog can link by hand: matching, not linked yet.
#[tauri::command]
pub async fn link_candidates(
    app: State<'_, std::sync::Arc<AppState>>,
    project_id: i64,
    query: String,
    limit: usize,
) -> Result<Vec<ListRow>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let projects = all_projects(state)?;
        let Some(found) = search_ids(state, &projects, &query)? else {
            return Ok(Vec::new());
        };
        let linked: HashSet<i64> = state
            .store
            .list_project_links(ProjectId(project_id))?
            .into_iter()
            .filter(|link| !matches!(link.source, LinkSource::Excluded))
            .map(|link| link.pr.0)
            .collect();
        let mut items: Vec<listing::Item> = list_items(state)?
            .into_iter()
            .filter(|item| {
                found.contains(&item.row.id)
                    && !linked.contains(&item.row.id)
                    && !item.row.hidden
                    && item.row.state == "open"
            })
            .collect();
        // `12` or `#12` puts PR number 12 first, then the newest matches.
        let exact: Option<u64> = query.trim().trim_start_matches('#').parse().ok();
        items.sort_by(|left, right| {
            (Some(right.row.number) == exact)
                .cmp(&(Some(left.row.number) == exact))
                .then(right.updated_ms.cmp(&left.updated_ms))
                .then(left.row.id.cmp(&right.row.id))
        });
        Ok(items
            .into_iter()
            .take(limit.min(50))
            .map(|item| item.row)
            .collect())
    })
    .await
}

fn project_dto(project: Project) -> ProjectDto {
    ProjectDto {
        id: project.id.0,
        name: project.name,
        color: project.color,
        archived: project.archived,
        description: project.description,
        sort_order: project.sort_order,
    }
}

#[tauri::command]
pub async fn list_projects(
    app: State<'_, std::sync::Arc<AppState>>,
) -> Result<Vec<ProjectDto>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        Ok(state
            .store
            .list_projects()?
            .into_iter()
            .map(project_dto)
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn create_project(
    app: State<'_, std::sync::Arc<AppState>>,
    name: String,
) -> Result<ProjectDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        Ok(project_dto(state.store.create_project(name)?))
    })
    .await
}

/// "Add to project → New project from selection…": the project and its manual
/// links are created together, or not at all.
#[tauri::command]
pub async fn create_project_with_prs(
    app: State<'_, std::sync::Arc<AppState>>,
    name: String,
    pr_ids: Vec<i64>,
) -> Result<ProjectDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let (project, _) = state.store.create_project_with_prs(
            name,
            pr_ids.into_iter().map(PrId).collect(),
            Utc::now(),
        )?;
        Ok(project_dto(project))
    })
    .await
}

#[tauri::command]
pub async fn rename_project(
    app: State<'_, std::sync::Arc<AppState>>,
    id: i64,
    name: String,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        state.store.rename_project(ProjectId(id), name)?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn archive_project(
    app: State<'_, std::sync::Arc<AppState>>,
    id: i64,
    archived: bool,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        state.store.set_project_archived(ProjectId(id), archived)?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn delete_project(
    app: State<'_, std::sync::Arc<AppState>>,
    id: i64,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        state.store.delete_project(ProjectId(id))?;
        Ok(())
    })
    .await
}

#[derive(Serialize, Deserialize)]
pub struct RuleDto {
    pub id: Option<i64>,
    pub project_id: i64,
    pub kind: String,
    pub pattern: String,
    pub priority: i32,
}

#[derive(Serialize)]
pub struct LinkDto {
    pub pr_id: i64,
    pub source: String,
    pub rule_id: Option<i64>,
}

fn rule_dto(rule: ProjectRule) -> RuleDto {
    RuleDto {
        id: Some(rule.id),
        project_id: rule.project.0,
        kind: rule.kind.as_str().to_owned(),
        pattern: rule.pattern,
        priority: rule.priority,
    }
}

fn parse_kind(kind: &str) -> Result<RuleKind, AppError> {
    kind.parse::<RuleKind>()
        .map_err(|_| AppError::new("validation", format!("Unknown rule kind {kind}")))
}

fn account_prs(state: &AppState) -> Result<Vec<PullRequest>, AppError> {
    Ok(match state::github_account(&state.store)? {
        Some(account) => state.store.list_pull_requests_for_account(account.id)?,
        None => Vec::new(),
    })
}

fn relink_all(state: &AppState) -> Result<(), AppError> {
    linking::relink(&state.store, &account_prs(state)?)?;
    Ok(())
}

#[tauri::command]
pub async fn set_project_color(
    app: State<'_, std::sync::Arc<AppState>>,
    id: i64,
    color: String,
) -> Result<ProjectDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        Ok(project_dto(
            state.store.set_project_color(ProjectId(id), color)?,
        ))
    })
    .await
}

#[tauri::command]
pub async fn list_project_rules(
    app: State<'_, std::sync::Arc<AppState>>,
    project_id: i64,
) -> Result<Vec<RuleDto>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let mut rules = state.store.list_project_rules(ProjectId(project_id))?;
        rules.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
        Ok(rules.into_iter().map(rule_dto).collect())
    })
    .await
}

/// Creates (`id: null`) or updates a rule, then re-evaluates every rule (§8.1).
#[tauri::command]
pub async fn save_project_rule(
    app: State<'_, std::sync::Arc<AppState>>,
    rule: RuleDto,
) -> Result<RuleDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let kind = parse_kind(&rule.kind)?;
        let pattern = linking::validate(kind, &rule.pattern)?;
        let value = ProjectRule {
            id: rule.id.unwrap_or(0),
            project: ProjectId(rule.project_id),
            kind,
            pattern,
            priority: rule.priority,
        };
        let saved = match rule.id {
            Some(_) => state.store.update_project_rule(value)?,
            None => state.store.insert_project_rule(value)?,
        };
        relink_all(state)?;
        Ok(rule_dto(saved))
    })
    .await
}

#[tauri::command]
pub async fn delete_project_rule(
    app: State<'_, std::sync::Arc<AppState>>,
    id: i64,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        state.store.delete_project_rule(id)?;
        relink_all(state)
    })
    .await
}

/// Live preview: how many open PRs `kind`/`pattern` would link.
#[tauri::command]
pub async fn preview_rule(
    app: State<'_, std::sync::Arc<AppState>>,
    kind: String,
    pattern: String,
) -> Result<usize, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let kind = parse_kind(&kind)?;
        Ok(linking::preview_count(
            &state.store,
            &account_prs(state)?,
            kind,
            &pattern,
        )?)
    })
    .await
}

#[tauri::command]
pub async fn list_project_links(
    app: State<'_, std::sync::Arc<AppState>>,
    project_id: i64,
) -> Result<Vec<LinkDto>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        Ok(state
            .store
            .list_project_links(ProjectId(project_id))?
            .into_iter()
            .map(|link| {
                let (source, rule_id) = match link.source {
                    LinkSource::Manual => ("manual", None),
                    LinkSource::Rule(id) => ("rule", Some(id)),
                    LinkSource::Excluded => ("excluded", None),
                };
                LinkDto {
                    pr_id: link.pr.0,
                    source: source.to_owned(),
                    rule_id,
                }
            })
            .collect())
    })
    .await
}

/// What one manual link replaced: `before` is `null` (not linked), `"rule"`
/// (with `rule_id`) or `"excluded"`. The list's Undo hands these back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplacedLinkDto {
    pub pr_id: i64,
    pub before: Option<String>,
    pub rule_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct LinkOutcomeDto {
    /// Rows that became manual links; PRs already linked by hand are not counted.
    pub changed: usize,
    pub undo: Vec<ReplacedLinkDto>,
}

fn replaced_dto(item: ReplacedLink) -> ReplacedLinkDto {
    let (before, rule_id) = match item.before {
        None => (None, None),
        Some(LinkSource::Manual) => (Some("manual"), None),
        Some(LinkSource::Rule(id)) => (Some("rule"), Some(id)),
        Some(LinkSource::Excluded) => (Some("excluded"), None),
    };
    ReplacedLinkDto {
        pr_id: item.pr.0,
        before: before.map(str::to_owned),
        rule_id,
    }
}

fn replaced_from_dto(dto: ReplacedLinkDto) -> Result<ReplacedLink, AppError> {
    let before = match (dto.before.as_deref(), dto.rule_id) {
        (None, _) => None,
        (Some("manual"), _) => Some(LinkSource::Manual),
        (Some("excluded"), _) => Some(LinkSource::Excluded),
        (Some("rule"), Some(id)) => Some(LinkSource::Rule(id)),
        (Some(other), _) => {
            return Err(AppError::new(
                "validation",
                format!("Cannot undo a link that was {other}"),
            ));
        }
    };
    Ok(ReplacedLink {
        pr: PrId(dto.pr_id),
        before,
    })
}

/// Manual links: drag-and-drop onto a project, "Add to project", or "Add pull
/// request" in the project dialog. All or nothing; the outcome carries Undo.
#[tauri::command]
pub async fn link_prs(
    app: State<'_, std::sync::Arc<AppState>>,
    project_id: i64,
    pr_ids: Vec<i64>,
) -> Result<LinkOutcomeDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let project = ProjectId(project_id);
        if state.store.get_project(project)?.is_none() {
            return Err(AppError::new("validation", "Project not found"));
        }
        let replaced =
            state
                .store
                .link_manual(project, pr_ids.into_iter().map(PrId).collect(), Utc::now())?;
        Ok(LinkOutcomeDto {
            changed: replaced.len(),
            undo: replaced.into_iter().map(replaced_dto).collect(),
        })
    })
    .await
}

/// Undo for [`link_prs`]: puts each still-manual row back the way it was.
#[tauri::command]
pub async fn undo_link_prs(
    app: State<'_, std::sync::Arc<AppState>>,
    project_id: i64,
    undo: Vec<ReplacedLinkDto>,
) -> Result<usize, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let replaced = undo
            .into_iter()
            .map(replaced_from_dto)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(state
            .store
            .undo_manual_links(ProjectId(project_id), replaced)?)
    })
    .await
}

#[tauri::command]
pub async fn unlink_pr(
    app: State<'_, std::sync::Arc<AppState>>,
    project_id: i64,
    pr_id: i64,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let project = ProjectId(project_id);
        let pr = PrId(pr_id);
        let rule_matches = match state.store.get_pull_request(pr)? {
            Some(value) => linking::rule_matches(&state.store, &value, project)?,
            None => false,
        };
        state
            .store
            .unlink_pr(project, pr, rule_matches, Utc::now())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn restore_pr(
    app: State<'_, std::sync::Arc<AppState>>,
    project_id: i64,
    pr_id: i64,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        state.store.restore_pr(ProjectId(project_id), PrId(pr_id))?;
        relink_all(state)
    })
    .await
}

#[derive(Serialize)]
pub struct TeamGap {
    pub team: String,
    pub reasons: Vec<String>,
}

#[derive(Serialize)]
pub struct FileOwners {
    pub path: String,
    pub teams: Vec<String>,
}

/// The detail view: readiness, owners, and why (§6, §7.2).
#[derive(Serialize)]
pub struct PrDetail {
    pub id: i64,
    pub readiness: String,
    pub readiness_notes: Vec<String>,
    pub head_sha: String,
    pub base_ref: String,
    pub codeowners_path: Option<String>,
    pub codeowners_error: Option<String>,
    /// `true`, `false`, or `unknown`.
    pub codeowners_required: String,
    pub codeowners_required_reason: String,
    pub code_owner_review_pending: Option<bool>,
    pub mandatory_teams: Vec<String>,
    pub optional_teams: Vec<String>,
    pub approved_teams: Vec<String>,
    pub candidate_teams: Vec<String>,
    pub attribution_unknown: Vec<TeamGap>,
    pub requested_teams: Vec<String>,
    pub users_ignored: Vec<String>,
    pub rationale: Vec<String>,
    pub files: Vec<FileOwners>,
    pub files_truncated: bool,
    pub rules_error: Option<String>,
    pub protection: String,
    pub from_cache: bool,
}

/// Resolves ownership off the UI thread. Demo mode uses the fixture provider.
#[tauri::command]
pub async fn pr_detail(
    app: State<'_, std::sync::Arc<AppState>>,
    id: i64,
) -> Result<PrDetail, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || pr_detail_blocking(&state, PrId(id)))
        .await
        .map_err(|err| AppError::new("internal", err.to_string()))?
}

pub(crate) fn pr_detail_blocking(state: &AppState, id: PrId) -> Result<PrDetail, AppError> {
    let pr = state
        .store
        .get_pull_request(id)?
        .ok_or_else(|| AppError::new("not_found", "pull request not found"))?;
    let flag = settings::blocked_review_required_is_ready(&state.store)?;
    let report = if state.demo {
        ownership::resolve(
            &state.store,
            &crate::demo::DemoProvider,
            &pr,
            flag,
            Utc::now(),
        )?
    } else {
        let account = state::github_account(&state.store)?
            .ok_or_else(|| AppError::new("unauthorized", "Connect GitHub first"))?;
        let token = state::resolve_github_token(state, Some(&account))?;
        let client = GithubClient::new(account.id, token)?;
        ownership::resolve(&state.store, &client, &pr, flag, Utc::now())?
    };
    Ok(detail_dto(pr.id, report))
}

fn detail_dto(id: PrId, report: OwnershipReport) -> PrDetail {
    use pr_nexus_core::domain::{Readiness, TeamRef};
    use pr_nexus_core::providers::ClassicProtection;
    let names = |teams: &[TeamRef]| teams.iter().map(ToString::to_string).collect::<Vec<_>>();
    let (readiness, readiness_notes) = match report.readiness {
        Readiness::Ready { notes } => ("ready", notes),
        Readiness::Blocked { reasons } => ("blocked", reasons),
        Readiness::Unknown { reasons } => ("unknown", reasons),
    };
    let decision = report.decision;
    PrDetail {
        id: id.0,
        readiness: readiness.into(),
        readiness_notes,
        head_sha: report.head_sha,
        base_ref: report.base_ref,
        codeowners_path: report.codeowners_path,
        codeowners_error: report.codeowners_error,
        codeowners_required: match decision.codeowners_required {
            pr_nexus_core::domain::Tri::True => "true",
            pr_nexus_core::domain::Tri::False => "false",
            pr_nexus_core::domain::Tri::Unknown => "unknown",
        }
        .into(),
        codeowners_required_reason: decision.codeowners_required_reason,
        code_owner_review_pending: report.code_owner_review_pending,
        mandatory_teams: names(&decision.mandatory_teams),
        optional_teams: names(&decision.optional_teams),
        approved_teams: names(&decision.approved_teams),
        candidate_teams: names(&decision.candidate_teams),
        attribution_unknown: decision
            .review_attribution_unknown
            .into_iter()
            .map(|gap| TeamGap {
                team: gap.team.to_string(),
                reasons: gap.reasons,
            })
            .collect(),
        requested_teams: decision.requested_teams_informational,
        users_ignored: decision.users_ignored_for_slack,
        rationale: decision.rationale,
        files: report
            .files
            .into_iter()
            .map(|file| FileOwners {
                path: file.path,
                teams: names(&file.teams),
            })
            .collect(),
        files_truncated: report.files_truncated,
        rules_error: report.rules_error,
        protection: match report.protection {
            ClassicProtection::Present {
                require_code_owner_reviews: true,
            } => "Branch protection requires code-owner reviews".into(),
            ClassicProtection::Present {
                require_code_owner_reviews: false,
            } => "Branch protection does not require code-owner reviews".into(),
            ClassicProtection::Absent => "No classic branch protection".into(),
            ClassicProtection::Unknown { reason } => format!("Unknown: {reason}"),
        },
        from_cache: report.from_cache,
    }
}

#[tauri::command]
pub async fn load_view_state(
    app: State<'_, std::sync::Arc<AppState>>,
    view_key: String,
) -> Result<Option<String>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        Ok(state
            .store
            .get_view_state(view_key)?
            .map(|entry| entry.state_json))
    })
    .await
}

#[tauri::command]
pub async fn save_view_state(
    app: State<'_, std::sync::Arc<AppState>>,
    view_key: String,
    state_json: String,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        state.store.put_view_state(ViewState {
            view_key,
            state_json,
            updated_at: Utc::now(),
        })?;
        Ok(())
    })
    .await
}

pub(crate) fn indicator(request: pr_nexus_core::domain::ReviewRequest) -> SlackIndicator {
    SlackIndicator {
        requested_at: request.requested_at.to_rfc3339(),
        last_bumped_at: request.last_bumped_at.map(|value| value.to_rfc3339()),
        bump_count: request.bump_count,
        workspace: request.workspace,
        channel_id: request.channel_id,
        channel_name: request.channel_name,
        permalink: request.permalink,
        root_ts: request.root_ts,
        source: request.source.as_str().to_owned(),
    }
}

fn to_item(facts: &mut ListFacts, pr: &PullRequest, flag: bool) -> listing::Item {
    let links = facts.links.remove(&pr.id).unwrap_or_default();
    let request = facts.active_requests.remove(&pr.id);
    let slack_requested_ms = request
        .as_ref()
        .map_or(0, |request| request.requested_at.timestamp_millis());
    let slack_stamp_ms = request.as_ref().map(|request| {
        request
            .last_bumped_at
            .unwrap_or(request.requested_at)
            .timestamp_millis()
    });
    let readiness = ownership::cached_readiness_with(facts.ownership.get(&pr.id), pr, flag);
    let (label, notes) = match readiness {
        pr_nexus_core::domain::Readiness::Ready { notes } => ("ready", notes),
        pr_nexus_core::domain::Readiness::Blocked { reasons } => ("blocked", reasons),
        pr_nexus_core::domain::Readiness::Unknown { reasons } => ("unknown", reasons),
    };
    listing::Item {
        row: ListRow {
            id: pr.id.0,
            number: pr.number,
            title: pr.title.clone(),
            url: pr.url.clone(),
            owner: pr.repo.owner.clone(),
            repo: format!("{}/{}", pr.repo.owner, pr.repo.name),
            author_login: pr.author_login.clone(),
            head_ref: pr.head_ref.clone(),
            is_draft: pr.is_draft,
            state: pr.state.as_str().to_owned(),
            checks: pr.checks.as_str().to_owned(),
            review_decision: pr.review_decision.map(|value| value.as_str().to_owned()),
            unresolved_threads: pr.unresolved_threads,
            total_threads: pr.total_threads,
            involvement: pr.involvement.bits(),
            updated_at: pr.updated_at.to_rfc3339(),
            created_at: pr.created_at.to_rfc3339(),
            hidden: pr.hidden,
            project_ids: links
                .into_iter()
                .filter(|link| !matches!(link.source, LinkSource::Excluded))
                .map(|link| link.project.0)
                .collect(),
            readiness: label.into(),
            readiness_notes: notes,
            conflicts: pr.mergeable == pr_nexus_core::domain::Mergeable::Conflicting
                || pr.merge_state == pr_nexus_core::domain::MergeStateStatus::Dirty,
            reviewers_requested: !pr.requested_reviewers.is_empty(),
            slack: request.map(indicator),
        },
        updated_ms: pr.updated_at.timestamp_millis(),
        created_ms: pr.created_at.timestamp_millis(),
        slack_requested_ms,
        slack_stamp_ms,
    }
}

fn reqwest_client() -> Result<reqwest::blocking::Client, AppError> {
    reqwest::blocking::Client::builder()
        .user_agent("pr-nexus")
        .timeout(github::REQUEST_TIMEOUT)
        .build()
        .map_err(|err| AppError::new("network", err.to_string()))
}

#[tauri::command]
pub async fn open_external(app: tauri::AppHandle, url: String) -> Result<(), AppError> {
    blocking::run(move || {
        let slack_https = url.starts_with("https://") && url.contains(".slack.com/");
        let allowed = url.starts_with("https://github.com/")
            || url.starts_with("https://api.slack.com/")
            || slack_https
            || url.starts_with("slack://");
        if !allowed {
            return Err(AppError::new("validation", "That link cannot be opened"));
        }
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|err| AppError::new("network", err.to_string()))
    })
    .await
}
