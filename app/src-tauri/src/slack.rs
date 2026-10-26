//! Slack workspaces, sign-in, and the single-PR request/bump flow (§7.7, §7.11).
//!
//! Every Slack call goes through the official MCP server. Demo mode signs in
//! and posts nothing: it records the message against a made-up permalink.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use pr_nexus_core::domain::{
    Account, AccountId, AccountKind, AuthMode, ChatKind, PrId, PullRequest,
};
use pr_nexus_core::error::{CoreError, ProviderError};
use pr_nexus_core::mapping::lookup_mapping;
use pr_nexus_core::providers::{
    ChatChannel, ChatProvider, ChatUser, OutgoingMessage, Page, PostedMessage,
};
use pr_nexus_core::review::lifecycle::{self, Action, Target};
use pr_nexus_core::settings;
use pr_nexus_core::store::Store;
use pr_nexus_providers::slack_mcp::{
    self, CancelHandle, ConnectConfig, SessionConfig, SlackAuthError, TokenState,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::commands::{SlackIndicator, indicator};
use crate::error::AppError;
use crate::state::AppState;

type Shared<'a> = State<'a, Arc<AppState>>;

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct SlackWorkspace {
    pub id: i64,
    pub team_name: String,
    /// `acme` for `acme.slack.com`.
    pub domain: String,
    pub user_name: String,
    /// The grant is saved and Slack has not rejected it.
    pub signed_in: bool,
    /// Was connected, but the token is gone or rejected: show Reconnect.
    pub needs_sign_in: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    pub workspace: String,
    pub channel_id: String,
    pub channel_name: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct TargetChoice {
    pub target: ReviewTarget,
    /// Teams whose mapping points here, `org/slug`.
    pub teams: Vec<String>,
    /// `request` when an active request lives in this channel, else `mapping`.
    pub source: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct ReviewPreview {
    pub target: ReviewTarget,
    /// `request` or `bump`.
    pub action: String,
    pub text: String,
    pub warnings: Vec<String>,
    pub signed_in: bool,
    pub bump_count: u32,
    pub permalink: Option<String>,
}

/// Slack accounts, newest last.
pub fn workspace_accounts(store: &Store) -> Result<Vec<Account>, CoreError> {
    Ok(store
        .list_accounts()?
        .into_iter()
        .filter(|account| account.kind == AccountKind::Slack)
        .collect())
}

pub fn workspace_dto(state: &AppState, account: &Account) -> SlackWorkspace {
    let connected = account.keychain_key.is_some();
    let usable = connected && token_usable(state, account);
    SlackWorkspace {
        id: account.id.0,
        team_name: account.display_name.clone(),
        domain: account.slack_workspace.clone().unwrap_or_default(),
        user_name: account.login.clone(),
        signed_in: usable,
        needs_sign_in: connected && !usable,
    }
}

/// The grant is in the vault and the last check did not reject it. An
/// in-memory vault loses grants on restart, so the row alone proves nothing.
fn token_usable(state: &AppState, account: &Account) -> bool {
    if state.demo {
        return true;
    }
    let Some(key) = &account.keychain_key else {
        return false;
    };
    let rejected = matches!(
        lock(&state.slack_tokens).get(&account.id.0),
        Some(TokenState::Missing | TokenState::Rejected)
    );
    !rejected && state.vault.get(key).ok().flatten().is_some()
}

/// Check every saved grant not checked yet with `auth.test` (startup and
/// the first workspace list).
pub async fn validate_tokens(state: &AppState) {
    if state.demo {
        return;
    }
    let Ok(accounts) = workspace_accounts(&state.store) else {
        return;
    };
    for account in accounts {
        let Some(key) = account.keychain_key.clone() else {
            continue;
        };
        if lock(&state.slack_tokens).contains_key(&account.id.0) {
            continue;
        }
        let checked =
            slack_mcp::token_state(slack_mcp::SLACK_WEB_API, state.vault.clone(), &key).await;
        lock(&state.slack_tokens).insert(account.id.0, checked);
    }
}

/// Remember that Slack rejected an account's token, so Settings offers Reconnect.
pub(crate) fn mark_rejected(state: &AppState, account: i64) {
    lock(&state.chats).remove(&account);
    lock(&state.slack_tokens).insert(account, TokenState::Rejected);
}

#[tauri::command]
pub async fn slack_workspaces(app: Shared<'_>) -> Result<Vec<SlackWorkspace>, AppError> {
    let state = app.inner().clone();
    validate_tokens(&state).await;
    Ok(workspace_accounts(&state.store)?
        .iter()
        .map(|account| workspace_dto(&state, account))
        .collect())
}

/// Browser sign-in for one workspace. Resolves when the redirect arrives,
/// the 10-minute timeout fires, or [`slack_cancel_connect`] runs.
#[tauri::command]
pub async fn slack_connect(
    app: tauri::AppHandle,
    state: Shared<'_>,
) -> Result<SlackWorkspace, AppError> {
    let state = state.inner().clone();
    if state.demo {
        return demo_connect(&state);
    }
    let client_id = settings::slack_client_id(&state.store)?.trim().to_owned();
    let client_secret = if client_id.is_empty() {
        None
    } else {
        state.vault.get(&slack_mcp::client_secret_key(&client_id))?
    };
    let config = ConnectConfig {
        mcp_url: settings::slack_mcp_url(&state.store)?,
        web_api_base: slack_mcp::SLACK_WEB_API.to_owned(),
        client_id,
        client_secret,
        redirect_uri: settings::slack_redirect_uri(&state.store)?,
        scopes: slack_mcp::USER_SCOPES
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        timeout: slack_mcp::loopback::CALLBACK_TIMEOUT,
    };
    let cancel = CancelHandle::new();
    if let Some(previous) = lock(&state.slack_cancel).replace(cancel.clone()) {
        previous.cancel();
    }
    let opener = app.clone();
    let result = slack_mcp::connect(
        &config,
        state.vault.clone(),
        move |url| open_browser(&opener, url),
        &cancel,
    )
    .await;
    lock(&state.slack_cancel).take();
    let connected = result.map_err(auth_error)?;
    let account = state.store.upsert_account(Account {
        id: AccountId(0),
        kind: AccountKind::Slack,
        display_name: connected.team_name.clone(),
        host: format!("{}.slack.com", connected.domain),
        external_id: format!("{}:{}", connected.team_id, connected.user_id),
        slack_workspace: Some(connected.domain.clone()),
        login: connected.user_name.clone(),
        auth_mode: AuthMode::SlackMcpOauth,
        keychain_key: Some(connected.keychain_key.clone()),
        created_at: Utc::now(),
    })?;
    lock(&state.chats).remove(&account.id.0);
    lock(&state.slack_tokens).insert(account.id.0, TokenState::Valid);
    Ok(workspace_dto(&state, &account))
}

#[tauri::command]
pub fn slack_cancel_connect(app: Shared<'_>) {
    if let Some(cancel) = lock(&app.slack_cancel).take() {
        cancel.cancel();
    }
}

/// Deletes the local grant. Never calls `auth.revoke` (§7.13 R1). The row
/// stays, signed out, when messages reference it, so history keeps its links.
#[tauri::command]
pub fn slack_remove_workspace(app: Shared<'_>, id: i64) -> Result<(), AppError> {
    let state = app.inner().as_ref();
    let account = state
        .store
        .get_account(AccountId(id))?
        .filter(|account| account.kind == AccountKind::Slack)
        .ok_or_else(|| AppError::new("not_found", "Slack workspace not found"))?;
    lock(&state.chats).remove(&id);
    if let Some(key) = &account.keychain_key {
        state.vault.remove(key)?;
    }
    if state.store.delete_account(account.id).is_err() {
        state.store.upsert_account(Account {
            keychain_key: None,
            ..account
        })?;
    }
    Ok(())
}

/// Channels for the PR: active requests first, then the mapping rows of
/// `teams` (`org/slug`).
#[tauri::command]
pub fn review_targets(
    app: Shared<'_>,
    pr_id: i64,
    teams: Vec<String>,
) -> Result<Vec<TargetChoice>, AppError> {
    let state = app.inner().as_ref();
    let pr = load_pr(&state.store, pr_id)?;
    targets_for(&state.store, &pr, &teams)
}

/// Active requests first, then the mapping rows of `teams` (`org/slug`).
pub(crate) fn targets_for(
    store: &Store,
    pr: &PullRequest,
    teams: &[String],
) -> Result<Vec<TargetChoice>, AppError> {
    let mut choices: Vec<TargetChoice> = Vec::new();
    for request in store.list_review_requests(pr.id)? {
        if request.status != pr_nexus_core::domain::RequestStatus::Active {
            continue;
        }
        push_choice(
            &mut choices,
            ReviewTarget {
                workspace: request.workspace,
                channel_id: request.channel_id,
                channel_name: request.channel_name,
            },
            None,
            "request",
        );
    }
    let rows = store.list_mappings()?;
    for team in teams {
        let Some((org, slug)) = team.trim_start_matches('@').split_once('/') else {
            continue;
        };
        if let Some(row) = lookup_mapping(&rows, org, slug, "") {
            push_choice(
                &mut choices,
                ReviewTarget {
                    workspace: row.slack_workspace.clone(),
                    channel_id: row.slack_channel_id.clone(),
                    channel_name: row.slack_channel_name.clone(),
                },
                Some(team),
                "mapping",
            );
        }
    }
    Ok(choices)
}

fn push_choice(
    choices: &mut Vec<TargetChoice>,
    target: ReviewTarget,
    team: Option<&String>,
    source: &str,
) {
    if let Some(existing) = choices.iter_mut().find(|choice| {
        choice.target.workspace == target.workspace && choice.target.channel_id == target.channel_id
    }) {
        if let Some(team) = team
            && !existing.teams.contains(team)
        {
            existing.teams.push(team.clone());
        }
        return;
    }
    choices.push(TargetChoice {
        target,
        teams: team.into_iter().cloned().collect(),
        source: source.to_owned(),
    });
}

#[tauri::command]
pub fn review_preview(
    app: Shared<'_>,
    pr_id: i64,
    target: ReviewTarget,
) -> Result<ReviewPreview, AppError> {
    let state = app.inner().as_ref();
    let pr = load_pr(&state.store, pr_id)?;
    let target = validate_target(target)?;
    let plan = plan(state, &pr, &target)?;
    let signed_in = state.demo || signed_in_account(state, &target.workspace)?.is_some();
    let (action, bump_count, permalink) = match &plan.action {
        Action::Request => ("request", 0, None),
        Action::Bump { request } => ("bump", request.bump_count, request.permalink.clone()),
    };
    Ok(ReviewPreview {
        target,
        action: action.into(),
        text: plan.text,
        warnings: plan.warnings,
        signed_in,
        bump_count,
        permalink,
    })
}

/// Plans again (the state may have moved since the preview) and posts.
#[tauri::command]
pub async fn send_review(
    app: Shared<'_>,
    pr_id: i64,
    target: ReviewTarget,
    teams: Vec<String>,
) -> Result<SlackIndicator, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || send_blocking(&state, pr_id, target, &teams))
        .await
        .map_err(|err| AppError::new("internal", err.to_string()))?
}

fn send_blocking(
    state: &AppState,
    pr_id: i64,
    target: ReviewTarget,
    teams: &[String],
) -> Result<SlackIndicator, AppError> {
    let pr = load_pr(&state.store, pr_id)?;
    let target = validate_target(target)?;
    let plan = plan(state, &pr, &target)?;
    let chat = chat_for(state, &target.workspace)?;
    let teams_json = serde_json::to_string(teams).unwrap_or_else(|_| "[]".into());
    match lifecycle::send(
        &state.store,
        chat.as_ref(),
        &pr,
        &plan,
        &teams_json,
        Utc::now(),
    ) {
        Ok(request) => Ok(indicator(request)),
        Err(err) => {
            if matches!(err, CoreError::Provider(ProviderError::Unauthorized(_))) {
                mark_rejected(state, chat.account_id().0);
            }
            Err(slack_error(err))
        }
    }
}

fn plan(
    state: &AppState,
    pr: &PullRequest,
    target: &ReviewTarget,
) -> Result<lifecycle::Plan, AppError> {
    let requester = settings::review_requester(&state.store)?;
    Ok(lifecycle::plan(
        &state.store,
        pr,
        Target {
            workspace: target.workspace.clone(),
            channel_id: target.channel_id.clone(),
            channel_name: target.channel_name.clone(),
        },
        requester.as_deref(),
        settings::min_bump_interval_hours(&state.store)?,
        Utc::now(),
    )?)
}

pub(crate) fn load_pr(store: &Store, id: i64) -> Result<PullRequest, AppError> {
    store
        .get_pull_request(PrId(id))?
        .ok_or_else(|| AppError::new("not_found", "pull request not found"))
}

/// Trims, checks the channel ID shape, and names the channel after its ID
/// when the name is empty. Accepts channels (`C…`), private channels
/// (`G…`), and direct messages (`D…`, e.g. a DM with yourself for a test).
pub(crate) fn validate_target(target: ReviewTarget) -> Result<ReviewTarget, AppError> {
    let workspace = target.workspace.trim().to_ascii_lowercase();
    let channel_id = target.channel_id.trim().to_ascii_uppercase();
    if workspace.is_empty() {
        return Err(AppError::new("validation", "Choose a Slack workspace"));
    }
    let id_ok = channel_id.len() >= 9
        && channel_id.starts_with(['C', 'G', 'D'])
        && channel_id.chars().all(|c| c.is_ascii_alphanumeric());
    if !id_ok {
        return Err(AppError::new(
            "validation",
            "Enter a Slack channel ID such as C0123456789 (channel details → About), or a DM ID starting with D",
        ));
    }
    let name = target
        .channel_name
        .trim()
        .trim_start_matches('#')
        .to_owned();
    Ok(ReviewTarget {
        workspace,
        channel_name: if name.is_empty() {
            channel_id.clone()
        } else {
            name
        },
        channel_id,
    })
}

/// The workspace's account when its grant is usable.
pub(crate) fn signed_in_account(
    state: &AppState,
    workspace: &str,
) -> Result<Option<Account>, AppError> {
    Ok(workspace_accounts(&state.store)?
        .into_iter()
        .find(|account| {
            token_usable(state, account)
                && account
                    .slack_workspace
                    .as_deref()
                    .is_some_and(|domain| domain.eq_ignore_ascii_case(workspace))
        }))
}

pub(crate) fn chat_for(
    state: &AppState,
    workspace: &str,
) -> Result<Arc<dyn ChatProvider>, AppError> {
    if state.demo {
        let account = signed_in_account(state, workspace)?
            .or(crate::state::github_account(&state.store)?)
            .ok_or_else(|| AppError::new("unauthorized", "No demo account"))?;
        return Ok(Arc::new(DemoChat {
            account: account.id,
            workspace: workspace.to_owned(),
        }));
    }
    let account = signed_in_account(state, workspace)?.ok_or_else(|| {
        AppError::new(
            "unauthorized",
            format!("Slack sign-in expired. Reconnect the {workspace} workspace in Settings."),
        )
    })?;
    let mut chats = lock(&state.chats);
    if let Some(chat) = chats.get(&account.id.0) {
        return Ok(chat.clone());
    }
    let client_id = settings::slack_client_id(&state.store)?.trim().to_owned();
    let client_secret = if client_id.is_empty() {
        None
    } else {
        state.vault.get(&slack_mcp::client_secret_key(&client_id))?
    };
    let user_id = account
        .external_id
        .split_once(':')
        .map_or(account.external_id.as_str(), |(_, user)| user)
        .to_owned();
    let chat: Arc<dyn ChatProvider> = Arc::from(pr_nexus_providers::build_chat(
        ChatKind::Slack,
        account.id,
        ChatUser {
            id: user_id,
            name: account.login.clone(),
        },
        SessionConfig {
            mcp_url: settings::slack_mcp_url(&state.store)?,
            client_id,
            client_secret,
            redirect_uri: settings::slack_redirect_uri(&state.store)?,
            keychain_key: account.keychain_key.clone().unwrap_or_default(),
        },
        state.vault.clone(),
    )?);
    chats.insert(account.id.0, chat.clone());
    Ok(chat)
}

/// Opens the authorize URL in the system browser. On Linux that is
/// `xdg-open`, which uses the desktop's default browser.
fn open_browser(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if let Ok(mut child) = std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        // Reap the launcher so it does not linger as a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|err| err.to_string())
}

fn auth_error(err: SlackAuthError) -> AppError {
    let kind = match err {
        SlackAuthError::Cancelled => "cancelled",
        SlackAuthError::Timeout => "timeout",
        SlackAuthError::MissingClientId | SlackAuthError::InvalidRedirect(_) => "validation",
        SlackAuthError::PortInUse(_) => "port_in_use",
        SlackAuthError::Denied(_) => "denied",
        _ => "slack_auth",
    };
    AppError::new(kind, err.to_string())
}

/// Same as `From<CoreError>`, with Slack wording for rate limits.
fn slack_error(err: CoreError) -> AppError {
    match err {
        CoreError::Provider(ProviderError::RateLimited { reset_at, .. }) => AppError {
            kind: "rate_limited".into(),
            message: "Slack rate limit. Try again in a moment.".into(),
            retry_at: Some(reset_at.to_rfc3339()),
            sso_url: None,
        },
        other => other.into(),
    }
}

fn demo_connect(state: &AppState) -> Result<SlackWorkspace, AppError> {
    let account = state.store.upsert_account(Account {
        id: AccountId(0),
        kind: AccountKind::Slack,
        display_name: "Acme (demo)".into(),
        host: "acme.slack.com".into(),
        external_id: "T0DEMO:U0DEMO".into(),
        slack_workspace: Some("acme".into()),
        login: "ada".into(),
        auth_mode: AuthMode::SlackMcpOauth,
        keychain_key: Some("slack:T0DEMO:U0DEMO".into()),
        created_at: Utc::now(),
    })?;
    Ok(workspace_dto(state, &account))
}

/// Demo mode: posts nothing, returns a made-up `acme` permalink.
struct DemoChat {
    account: AccountId,
    workspace: String,
}

impl ChatProvider for DemoChat {
    fn kind(&self) -> ChatKind {
        ChatKind::Slack
    }
    fn whoami(&self) -> Result<ChatUser, ProviderError> {
        Ok(ChatUser {
            id: "U0DEMO".into(),
            name: "ada".into(),
        })
    }
    fn list_channels(&self, _: Option<String>) -> Result<Page<ChatChannel>, ProviderError> {
        Err(ProviderError::Unsupported("demo".into()))
    }
    fn channel_info(&self, _: &str) -> Result<ChatChannel, ProviderError> {
        Err(ProviderError::Unsupported("demo".into()))
    }
    fn post(&self, message: &OutgoingMessage<'_>) -> Result<PostedMessage, ProviderError> {
        let now = Utc::now();
        let ts = format!("{}.{:06}", now.timestamp(), now.timestamp_subsec_micros());
        Ok(PostedMessage {
            channel_id: message.channel_id.to_owned(),
            permalink: Some(format!(
                "https://{}.slack.com/archives/{}/p{}",
                self.workspace,
                message.channel_id,
                ts.replace('.', "")
            )),
            thread_ts: message.thread_ts.map(str::to_owned),
            workspace: Some(self.workspace.clone()),
            ts,
        })
    }
    fn account_id(&self) -> AccountId {
        self.account
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(workspace: &str, id: &str, name: &str) -> ReviewTarget {
        ReviewTarget {
            workspace: workspace.into(),
            channel_id: id.into(),
            channel_name: name.into(),
        }
    }

    #[test]
    fn manual_targets_are_normalized_and_checked() {
        let ok = validate_target(target(" Acme ", "c0123456789", "#team-reviews")).unwrap();
        assert_eq!(ok, target("acme", "C0123456789", "team-reviews"));
        let unnamed = validate_target(target("acme", "C0123456789", " ")).unwrap();
        assert_eq!(unnamed.channel_name, "C0123456789");
        assert!(validate_target(target("acme", "team-reviews", "")).is_err());
        let dm = validate_target(target("acme", " d0123456789 ", "")).unwrap();
        assert_eq!(dm, target("acme", "D0123456789", "D0123456789"));
        assert!(validate_target(target("acme", "U0123456789", "")).is_err());
        assert!(validate_target(target("acme", "D012", "")).is_err());
        assert!(validate_target(target("", "C0123456789", "")).is_err());
    }

    #[test]
    fn choices_merge_teams_per_channel_and_keep_requests_first() {
        let mut choices = Vec::new();
        push_choice(&mut choices, target("acme", "C1", "a"), None, "request");
        push_choice(
            &mut choices,
            target("acme", "C2", "b"),
            Some(&"acme/web".into()),
            "mapping",
        );
        push_choice(
            &mut choices,
            target("acme", "C1", "a"),
            Some(&"acme/api".into()),
            "mapping",
        );
        push_choice(
            &mut choices,
            target("acme", "C2", "b"),
            Some(&"acme/ui".into()),
            "mapping",
        );
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].source, "request");
        assert_eq!(choices[0].teams, vec!["acme/api".to_owned()]);
        assert_eq!(
            choices[1].teams,
            vec!["acme/web".to_owned(), "acme/ui".to_owned()]
        );
    }

    #[test]
    fn demo_chat_returns_a_parseable_permalink() {
        let chat = DemoChat {
            account: AccountId(1),
            workspace: "acme".into(),
        };
        let posted = chat
            .post(&OutgoingMessage {
                channel_id: "C0123456789",
                text: "hi",
                thread_ts: None,
                reply_broadcast: false,
                unfurl_app_links: true,
            })
            .unwrap();
        let link =
            pr_nexus_core::review::permalink::parse(posted.permalink.as_deref().unwrap()).unwrap();
        assert_eq!(link.ts, posted.ts);
        assert_eq!(link.workspace, "acme");
    }
}
