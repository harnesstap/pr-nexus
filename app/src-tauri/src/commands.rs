use chrono::Utc;
use pr_nexus_core::domain::{
    Account, AccountId, AccountKind, AuthMode, ChecksState, ProjectId, PullRequest, RequestStatus,
    ReviewDecision, ViewState,
};
use pr_nexus_core::mapping::{self, ImportMode};
use pr_nexus_core::onboarding::{self, OnboardingState};
use pr_nexus_core::providers::{SetupStep, VcsProvider};
use pr_nexus_core::review::readiness::{
    CheckSnapshot, Evidence, ReadinessSnapshot, ReviewSnapshot, decide_readiness_with,
};
use pr_nexus_core::settings::{self, DEFAULT_REDIRECT};
use pr_nexus_core::sync::SyncKind;
use pr_nexus_providers::github::{self, GithubClient};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::error::AppError;
use crate::slack_link;
use crate::state::{self, AppState};

#[derive(Serialize)]
pub struct SyncDto {
    pub phase: String,
    pub message: String,
    pub synced_at: Option<String>,
    pub paused: bool,
}

#[derive(Serialize)]
pub struct Bootstrap {
    pub onboarding: OnboardingState,
    pub resume_step: String,
    pub steps: Vec<String>,
    pub keyring_warning: Option<String>,
    pub keyring_available: bool,
    pub github_login: Option<String>,
    pub github_connected: bool,
    pub tracked_orgs: Vec<String>,
    pub include_personal: bool,
    pub org_choices: Vec<String>,
    pub orgs_needing_sso: Vec<String>,
    pub slack_client_id: String,
    pub slack_redirect_uri: String,
    pub mapping_count: i64,
    pub sync: SyncDto,
    pub env_token: bool,
    pub oauth_client_configured: bool,
    pub gh_ready: bool,
    pub gh_detail: String,
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

#[derive(Serialize)]
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

#[derive(Serialize)]
pub struct ListRow {
    pub id: i64,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub body: String,
    pub owner: String,
    pub repo: String,
    pub author_login: String,
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
    pub slack: Option<SlackIndicator>,
}

#[derive(Serialize)]
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

#[tauri::command]
pub fn bootstrap(app: State<'_, std::sync::Arc<AppState>>) -> Result<Bootstrap, AppError> {
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
    let (gh_ready, gh_detail) = if github::github_token_from_env().is_some() {
        (true, "Using PR_NEXUS_GITHUB_TOKEN".to_owned())
    } else {
        match github::read_gh_token() {
            github::GhProbe::Ready { .. } => (true, "gh is logged in".to_owned()),
            github::GhProbe::Unavailable { reason } => (false, reason),
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
        keyring_warning: state.vault.warning(),
        keyring_available: state.vault.available(),
        github_login: account.as_ref().map(|account| account.login.clone()),
        github_connected: account.is_some(),
        tracked_orgs: settings::tracked_orgs(&state.store)?,
        include_personal: settings::include_personal_repos(&state.store)?,
        org_choices: orgs,
        orgs_needing_sso: settings::sso_orgs(&state.store)?,
        slack_client_id: settings::slack_client_id(&state.store)?,
        slack_redirect_uri: settings::slack_redirect_uri(&state.store)?,
        mapping_count: state.store.mapping_count()?,
        sync: sync_dto(state),
        env_token: github::github_token_from_env().is_some(),
        oauth_client_configured: oauth.is_some(),
        gh_ready,
        gh_detail,
        onboarding: saved,
    })
}

#[tauri::command]
pub fn save_providers(
    app: State<'_, std::sync::Arc<AppState>>,
    vcs: String,
    chat: String,
) -> Result<(), AppError> {
    let state = app.inner().as_ref();
    if vcs != "github" || chat != "slack" {
        return Err(AppError::new(
            "validation",
            "Only GitHub and Slack can be selected",
        ));
    }
    settings::set_provider_kind(&state.store, &vcs, &chat)?;
    Ok(())
}

#[tauri::command]
pub fn save_onboarding(
    app: State<'_, std::sync::Arc<AppState>>,
    patch: OnboardingPatch,
) -> Result<(), AppError> {
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
}

#[tauri::command]
pub fn connect_github(
    app: State<'_, std::sync::Arc<AppState>>,
    mode: String,
    pat: Option<String>,
) -> Result<GithubSession, AppError> {
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
}

#[tauri::command]
pub fn start_device_flow(
    app: State<'_, std::sync::Arc<AppState>>,
) -> Result<DevicePrompt, AppError> {
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
}

#[tauri::command]
pub fn poll_device_flow(
    app: State<'_, std::sync::Arc<AppState>>,
) -> Result<DeviceResult, AppError> {
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
pub fn save_orgs(
    app: State<'_, std::sync::Arc<AppState>>,
    orgs: Vec<String>,
    include_personal: bool,
) -> Result<(), AppError> {
    let state = app.inner().as_ref();
    settings::set_tracked_orgs(&state.store, &orgs)?;
    settings::set_include_personal_repos(&state.store, include_personal)?;
    Ok(())
}

#[tauri::command]
pub fn save_slack_settings(
    app: State<'_, std::sync::Arc<AppState>>,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
) -> Result<(), AppError> {
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
    Ok(())
}

#[tauri::command]
pub fn slack_manifest_url(redirect_uri: String) -> Result<String, AppError> {
    let redirect = if redirect_uri.trim().is_empty() {
        DEFAULT_REDIRECT
    } else {
        redirect_uri.trim()
    };
    slack_link::create_slack_app_url(redirect).map_err(|err| AppError::new("validation", err))
}

#[tauri::command]
pub fn import_mapping(
    app: State<'_, std::sync::Arc<AppState>>,
    csv: String,
    workspace: String,
) -> Result<ImportSummary, AppError> {
    let state = app.inner().as_ref();
    let rows = mapping::parse_csv(&csv, Some(workspace.trim()))?;
    mapping::apply_import(&state.store, &rows, ImportMode::Merge, Utc::now())?;
    Ok(ImportSummary { rows: rows.len() })
}

#[tauri::command]
pub fn finish_onboarding(app: State<'_, std::sync::Arc<AppState>>) -> Result<SyncDto, AppError> {
    let state = app.inner().as_ref();
    let mut saved = onboarding::load(&state.store)?;
    saved.step = SetupStep::Done.as_str().into();
    if !saved.completed.iter().any(|step| step == "done") {
        saved.completed.push("done".into());
    }
    saved.finished_at = Some(Utc::now().to_rfc3339());
    onboarding::save(&state.store, &saved)?;
    let _ = state::run_sync(state, SyncKind::Manual);
    Ok(sync_dto(state))
}

#[tauri::command]
pub fn sync_now(app: State<'_, std::sync::Arc<AppState>>) -> Result<SyncDto, AppError> {
    let state = app.inner().as_ref();
    let _ = state::run_sync(state, SyncKind::Manual);
    Ok(sync_dto(state))
}

#[tauri::command]
pub fn sync_status(app: State<'_, std::sync::Arc<AppState>>) -> Result<SyncDto, AppError> {
    let state = app.inner().as_ref();
    Ok(sync_dto(state))
}

#[tauri::command]
pub fn set_focus(app: State<'_, std::sync::Arc<AppState>>, focused: bool) {
    let state = app.inner().as_ref();
    state
        .focused
        .store(focused, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
pub fn list_rows(app: State<'_, std::sync::Arc<AppState>>) -> Result<Vec<ListRow>, AppError> {
    let state = app.inner().as_ref();
    let Some(account) = state::github_account(&state.store)? else {
        return Ok(Vec::new());
    };
    let flag = settings::blocked_review_required_is_ready(&state.store)?;
    let prs = state.store.list_pull_requests_for_account(account.id)?;
    let mut rows = Vec::with_capacity(prs.len());
    for pr in prs {
        rows.push(to_row(state, &pr, flag)?);
    }
    Ok(rows)
}

#[tauri::command]
pub fn list_projects(
    app: State<'_, std::sync::Arc<AppState>>,
) -> Result<Vec<ProjectDto>, AppError> {
    let state = app.inner().as_ref();
    Ok(state
        .store
        .list_projects()?
        .into_iter()
        .map(|project| ProjectDto {
            id: project.id.0,
            name: project.name,
            color: project.color,
            archived: project.archived,
            description: project.description,
            sort_order: project.sort_order,
        })
        .collect())
}

#[tauri::command]
pub fn rename_project(
    app: State<'_, std::sync::Arc<AppState>>,
    id: i64,
    name: String,
) -> Result<(), AppError> {
    let state = app.inner().as_ref();
    state.store.rename_project(ProjectId(id), name)?;
    Ok(())
}

#[tauri::command]
pub fn archive_project(
    app: State<'_, std::sync::Arc<AppState>>,
    id: i64,
    archived: bool,
) -> Result<(), AppError> {
    let state = app.inner().as_ref();
    state.store.set_project_archived(ProjectId(id), archived)?;
    Ok(())
}

#[tauri::command]
pub fn delete_project(app: State<'_, std::sync::Arc<AppState>>, id: i64) -> Result<(), AppError> {
    let state = app.inner().as_ref();
    state.store.delete_project(ProjectId(id))?;
    Ok(())
}

#[tauri::command]
pub fn load_view_state(
    app: State<'_, std::sync::Arc<AppState>>,
    view_key: String,
) -> Result<Option<String>, AppError> {
    let state = app.inner().as_ref();
    Ok(state
        .store
        .get_view_state(view_key)?
        .map(|entry| entry.state_json))
}

#[tauri::command]
pub fn save_view_state(
    app: State<'_, std::sync::Arc<AppState>>,
    view_key: String,
    state_json: String,
) -> Result<(), AppError> {
    let state = app.inner().as_ref();
    state.store.put_view_state(ViewState {
        view_key,
        state_json,
        updated_at: Utc::now(),
    })?;
    Ok(())
}

fn to_row(state: &AppState, pr: &PullRequest, flag: bool) -> Result<ListRow, AppError> {
    let links = state.store.list_pr_projects(pr.id)?;
    let requests = state.store.list_review_requests(pr.id)?;
    let slack = requests
        .into_iter()
        .filter(|request| matches!(request.status, RequestStatus::Active))
        .max_by_key(|request| request.requested_at)
        .map(|request| SlackIndicator {
            requested_at: request.requested_at.to_rfc3339(),
            last_bumped_at: request.last_bumped_at.map(|value| value.to_rfc3339()),
            bump_count: request.bump_count,
            workspace: request.workspace,
            channel_id: request.channel_id,
            channel_name: request.channel_name,
            permalink: request.permalink,
            root_ts: request.root_ts,
            source: request.source.as_str().to_owned(),
        });
    let readiness = readiness_of(pr, flag);
    let (label, notes) = match readiness {
        pr_nexus_core::domain::Readiness::Ready { notes } => ("ready", notes),
        pr_nexus_core::domain::Readiness::Blocked { reasons } => ("blocked", reasons),
        pr_nexus_core::domain::Readiness::Unknown { reasons } => ("unknown", reasons),
    };
    Ok(ListRow {
        id: pr.id.0,
        number: pr.number,
        title: pr.title.clone(),
        url: pr.url.clone(),
        body: pr.body.clone(),
        owner: pr.repo.owner.clone(),
        repo: format!("{}/{}", pr.repo.owner, pr.repo.name),
        author_login: pr.author_login.clone(),
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
            .filter(|link| !matches!(link.source, pr_nexus_core::domain::LinkSource::Excluded))
            .map(|link| link.project.0)
            .collect(),
        readiness: label.into(),
        readiness_notes: notes,
        slack,
    })
}

fn readiness_of(pr: &PullRequest, flag: bool) -> pr_nexus_core::domain::Readiness {
    let checks = match pr.checks {
        ChecksState::Success => Some(pass("pass")),
        ChecksState::Failure | ChecksState::Error => Some(pass("fail")),
        ChecksState::Pending | ChecksState::Expected => Some(pass("pending")),
        ChecksState::None => None,
    };
    let reviews = Some(Evidence {
        complete: true,
        error: None,
        items: if pr.review_decision == Some(ReviewDecision::ChangesRequested) {
            vec![ReviewSnapshot {
                state: "CHANGES_REQUESTED".into(),
                author_login: "reviewer".into(),
                author_is_human: true,
            }]
        } else {
            Vec::new()
        },
    });
    decide_readiness_with(
        &ReadinessSnapshot {
            state: Some(pr.state),
            is_draft: Some(pr.is_draft),
            merge_state: Some(pr.merge_state),
            mergeable: Some(pr.mergeable),
            head_sha: Some(pr.head_sha.clone()),
            review_decision: pr.review_decision,
            checks_rollup: Some(pr.checks),
            checks,
            reviews,
            native_error: None,
            snapshot_changed: false,
            files_truncated: false,
        },
        flag,
    )
}

fn pass(bucket: &str) -> Evidence<Vec<CheckSnapshot>> {
    Evidence {
        complete: true,
        error: None,
        items: vec![CheckSnapshot {
            name: "ci".into(),
            bucket: bucket.into(),
        }],
    }
}

fn reqwest_client() -> Result<reqwest::blocking::Client, AppError> {
    reqwest::blocking::Client::builder()
        .user_agent("pr-nexus")
        .build()
        .map_err(|err| AppError::new("network", err.to_string()))
}

#[tauri::command]
pub fn open_external(app: tauri::AppHandle, url: String) -> Result<(), AppError> {
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
}
