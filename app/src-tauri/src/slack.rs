//! Slack workspaces, sign-in, and the single-PR request/bump flow (§7.7, §7.11).
//!
//! Every Slack call goes through the official MCP server. Demo mode signs in
//! and posts nothing: it records the message against a made-up permalink.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use pr_nexus_core::domain::{
    Account, AccountId, AccountKind, AuthMode, ChatKind, PrId, PrState, PullRequest,
};
use pr_nexus_core::error::{CoreError, ProviderError};
use pr_nexus_core::mapping::lookup_mapping;
use pr_nexus_core::providers::{
    ChatChannel, ChatProvider, ChatUser, OutgoingMessage, Page, PostedMessage, SearchHit,
    ThreadMessage, ThreadReplies,
};
use pr_nexus_core::review::lifecycle::Target;
use pr_nexus_core::review::route::{self, Workspace};
use pr_nexus_core::settings;
use pr_nexus_core::store::Store;
use pr_nexus_providers::slack_mcp::{
    self, CancelHandle, ConnectConfig, SessionConfig, SlackAuthError, TokenState,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::blocking;
use crate::commands::SlackIndicator;
use crate::error::AppError;
use crate::services::review;
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
    /// Slack team (`T…`): the routing key (§7.11).
    pub team_id: String,
    /// Off: the grant stays, but nothing searches, sends, bumps or reads here.
    pub enabled: bool,
    /// 0 is tried first.
    pub priority: usize,
    /// Scopes PR Nexus now asks for that this grant lacks: signing in again
    /// adds them. Empty when the grant does not list its scopes.
    #[serde(default)]
    pub scopes_missing: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    /// Domain; a hint only, since a pasted link can carry the org host.
    pub workspace: String,
    /// Slack team (`T…`) when known; routes before the domain.
    #[serde(default)]
    pub team_id: Option<String>,
    /// Team name for display; empty when unknown.
    #[serde(default)]
    pub workspace_name: String,
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

/// One target picker search result (§7.11): a channel, or a member who
/// gets a direct message (the user ID is the send target).
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct TargetMatch {
    pub target: ReviewTarget,
    /// `channel` or `member`.
    pub kind: String,
}

/// Results per kind; the MCP tools return at most 20.
const TARGET_SEARCH_LIMIT: u32 = 10;

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct ReviewPreview {
    pub target: ReviewTarget,
    /// `request` or `bump`.
    pub action: String,
    pub text: String,
    pub warnings: Vec<String>,
    /// Structured warnings about the PR itself (a draft, …), for the dialog
    /// and for MCP tools that relay the same warning to agents.
    pub pr_warnings: Vec<PrWarning>,
    pub signed_in: bool,
    pub bump_count: u32,
    pub permalink: Option<String>,
    /// A bump is also sent to the channel (`review.bump_broadcast`).
    pub broadcast: bool,
}

/// A non-blocking warning about a PR in a review-request preview.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct PrWarning {
    /// `draft`.
    pub kind: String,
    pub pr_id: i64,
    pub number: u64,
    pub message: String,
}

/// What reviewers should know before a request goes out: today, only that
/// the PR is still a draft. Sending stays allowed.
pub fn pr_warnings(pr: &PullRequest) -> Vec<PrWarning> {
    if pr.state == PrState::Open && pr.is_draft {
        vec![PrWarning {
            kind: "draft".into(),
            pr_id: pr.id.0,
            number: pr.number,
            message: format!(
                "#{} is still a draft. Reviewers may skip it until it's marked ready.",
                pr.number
            ),
        }]
    } else {
        Vec::new()
    }
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
    let team_id = account.slack_team_id().unwrap_or_default().to_owned();
    let workspaces = route::workspaces(&state.store).unwrap_or_default();
    let priority = workspaces
        .iter()
        .position(|ws| ws.team_id == team_id)
        .unwrap_or(workspaces.len());
    let enabled = workspaces.get(priority).is_none_or(|ws| ws.enabled);
    let connected = account.keychain_key.is_some();
    let usable = connected && token_usable(state, account, false);
    SlackWorkspace {
        team_id,
        enabled,
        priority,
        id: account.id.0,
        team_name: account.display_name.clone(),
        domain: account.slack_workspace.clone().unwrap_or_default(),
        user_name: account.login.clone(),
        signed_in: usable,
        needs_sign_in: connected && !usable,
        scopes_missing: if usable {
            granted_scopes(state, account)
                .map(|granted| slack_mcp::missing_scopes(&granted))
                .unwrap_or_default()
        } else {
            Vec::new()
        },
    }
}

/// The scopes the saved grant lists, without waiting on a keychain prompt.
/// `None` in demo mode and for grants that do not list them.
pub(crate) fn granted_scopes(state: &AppState, account: &Account) -> Option<Vec<String>> {
    if state.demo {
        return None;
    }
    let raw = state.vault.peek(account.keychain_key.as_ref()?).ok()??;
    slack_mcp::granted_scopes(&raw)
}

/// The grant is in the vault and the last check did not reject it. An
/// in-memory vault loses grants on restart, so the row alone proves nothing.
/// `wait` waits for a pending keychain prompt; display code passes `false`
/// so it never blocks on one.
fn token_usable(state: &AppState, account: &Account, wait: bool) -> bool {
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
    let saved = if wait {
        state.vault.get(key)
    } else {
        state.vault.peek(key)
    };
    !rejected && saved.ok().flatten().is_some()
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
    workspace_list(&state)
}

/// Every workspace, in priority order.
pub(crate) fn workspace_list(state: &AppState) -> Result<Vec<SlackWorkspace>, AppError> {
    let mut list: Vec<SlackWorkspace> = workspace_accounts(&state.store)?
        .iter()
        .map(|account| workspace_dto(state, account))
        .collect();
    list.sort_by_key(|ws| (ws.priority, ws.id));
    Ok(list)
}

/// Turn a workspace on or off for routing (§7.11). The grant is kept.
#[tauri::command]
pub async fn slack_set_workspace_enabled(
    app: Shared<'_>,
    team_id: String,
    enabled: bool,
) -> Result<Vec<SlackWorkspace>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        known_team(state, &team_id)?;
        settings::set_slack_workspace_enabled(&state.store, &team_id, enabled)?;
        workspace_list(state)
    })
    .await
}

/// Save the routing priority: `team_ids` first to last (§7.11).
#[tauri::command]
pub async fn slack_set_workspace_order(
    app: Shared<'_>,
    team_ids: Vec<String>,
) -> Result<Vec<SlackWorkspace>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        for team in &team_ids {
            known_team(state, team)?;
        }
        settings::set_slack_workspace_order(&state.store, &team_ids)?;
        workspace_list(state)
    })
    .await
}

fn known_team(state: &AppState, team_id: &str) -> Result<(), AppError> {
    if workspace_accounts(&state.store)?
        .iter()
        .any(|account| account.slack_team_id() == Some(team_id))
    {
        Ok(())
    } else {
        Err(AppError::new("not_found", "Slack workspace not found"))
    }
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
pub async fn slack_cancel_connect(app: Shared<'_>) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        if let Some(cancel) = lock(&app.slack_cancel).take() {
            cancel.cancel();
        }
        Ok(())
    })
    .await
}

/// Deletes the local grant. Never calls `auth.revoke` (§7.13 R1). The row
/// stays, signed out, when messages reference it, so history keeps its links.
#[tauri::command]
pub async fn slack_remove_workspace(app: Shared<'_>, id: i64) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
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
    })
    .await
}

/// Channels for the PR: active requests first, then the mapping rows of
/// `teams` (`org/slug`).
#[tauri::command]
pub async fn review_targets(
    app: Shared<'_>,
    pr_id: i64,
    teams: Vec<String>,
) -> Result<Vec<TargetChoice>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let pr = load_pr(&state.store, pr_id)?;
        targets_for(&state.store, &pr, &teams)
    })
    .await
}

/// Active requests first, then the mapping rows of `teams` (`org/slug`).
pub(crate) fn targets_for(
    store: &Store,
    pr: &PullRequest,
    teams: &[String],
) -> Result<Vec<TargetChoice>, AppError> {
    let workspaces = route::workspaces(store)?;
    let mut choices: Vec<TargetChoice> = Vec::new();
    for request in store.list_review_requests(pr.id)? {
        if request.status != pr_nexus_core::domain::RequestStatus::Active {
            continue;
        }
        push_choice(
            &mut choices,
            labeled(
                &workspaces,
                ReviewTarget {
                    workspace: request.workspace,
                    team_id: request.team_id,
                    workspace_name: String::new(),
                    channel_id: request.channel_id,
                    channel_name: request.channel_name,
                },
            ),
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
                labeled(
                    &workspaces,
                    ReviewTarget {
                        workspace: row.slack_workspace.clone(),
                        team_id: row.slack_team_id.clone(),
                        workspace_name: String::new(),
                        channel_id: row.slack_channel_id.clone(),
                        channel_name: row.slack_channel_name.clone(),
                    },
                ),
                Some(team),
                "mapping",
            );
        }
    }
    Ok(choices)
}

/// `target` with the name of the workspace it names, else its domain.
pub(crate) fn labeled(workspaces: &[Workspace], target: ReviewTarget) -> ReviewTarget {
    let ws = route::find(workspaces, target.team_id.as_deref(), &target.workspace);
    ReviewTarget {
        workspace_name: ws.map_or_else(|| target.workspace.clone(), |ws| ws.label().to_owned()),
        ..target
    }
}

pub(crate) fn to_core(target: &ReviewTarget) -> Target {
    Target {
        workspace: target.workspace.clone(),
        team_id: target.team_id.clone(),
        channel_id: target.channel_id.clone(),
        channel_name: target.channel_name.clone(),
    }
}

pub(crate) fn from_core(workspaces: &[Workspace], target: Target) -> ReviewTarget {
    labeled(
        workspaces,
        ReviewTarget {
            workspace: target.workspace,
            team_id: target.team_id,
            workspace_name: String::new(),
            channel_id: target.channel_id,
            channel_name: target.channel_name,
        },
    )
}

fn push_choice(
    choices: &mut Vec<TargetChoice>,
    target: ReviewTarget,
    team: Option<&String>,
    source: &str,
) {
    if let Some(existing) = choices.iter_mut().find(|choice| {
        choice.target.channel_id == target.channel_id
            && match (&choice.target.team_id, &target.team_id) {
                (Some(a), Some(b)) => a == b,
                _ => choice
                    .target
                    .workspace
                    .eq_ignore_ascii_case(&target.workspace),
            }
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

/// Search every enabled workspace for a send target, in priority order
/// (§7.11). `#term` searches channels, `@term` members, a bare term both.
/// Results are merged, a channel seen by several workspaces is kept once
/// (the first), and each is labeled with its workspace. An empty term
/// returns nothing without calling Slack.
#[tauri::command]
pub async fn slack_search_targets(
    app: Shared<'_>,
    query: String,
) -> Result<Vec<TargetMatch>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let state = app.inner().as_ref();
        let (channels, members, term) = target_query(&query);
        if term.is_empty() {
            return Ok(Vec::new());
        }
        let workspaces: Vec<Workspace> = route::workspaces(&state.store)?
            .into_iter()
            .filter(|ws| ws.enabled)
            .collect();
        search_all(&workspaces, term, channels, members, |ws| {
            chat_for(state, &ws.team_id)
        })
    })
    .await
}

/// [`search_targets`] over `workspaces`, merged and deduped. Errors only
/// when no workspace answered.
pub(crate) fn search_all(
    workspaces: &[Workspace],
    term: &str,
    channels: bool,
    members: bool,
    mut chat_for: impl FnMut(&Workspace) -> Result<Arc<dyn ChatProvider>, AppError>,
) -> Result<Vec<TargetMatch>, AppError> {
    if workspaces.is_empty() {
        return Err(AppError::new(
            "validation",
            "No enabled Slack workspace. Turn one on in Settings → Slack.",
        ));
    }
    let mut found: Vec<TargetMatch> = Vec::new();
    let mut failure = None;
    let mut answered = false;
    for ws in workspaces {
        let base = ReviewTarget {
            workspace: ws.domain().to_owned(),
            team_id: Some(ws.team_id.clone()),
            workspace_name: ws.label().to_owned(),
            channel_id: String::new(),
            channel_name: String::new(),
        };
        let hits = chat_for(ws)
            .and_then(|chat| search_targets(chat.as_ref(), &base, term, channels, members));
        match hits {
            Ok(hits) => {
                answered = true;
                for hit in hits {
                    if !found
                        .iter()
                        .any(|seen| seen.target.channel_id == hit.target.channel_id)
                    {
                        found.push(hit);
                    }
                }
            }
            Err(err) => failure = failure.or(Some(err)),
        }
    }
    match failure {
        Some(err) if !answered => Err(err),
        _ => Ok(found),
    }
}

/// `(search channels, search members, term)` for a picker query.
pub(crate) fn target_query(query: &str) -> (bool, bool, &str) {
    let query = query.trim();
    if let Some(term) = query.strip_prefix('#') {
        (true, false, term.trim())
    } else if let Some(term) = query.strip_prefix('@') {
        (false, true, term.trim())
    } else {
        (true, true, query)
    }
}

/// Channels first, then members. With both kinds asked, one kind failing
/// still returns the other; the error surfaces only when nothing worked.
pub(crate) fn search_targets(
    chat: &dyn ChatProvider,
    base: &ReviewTarget,
    term: &str,
    channels: bool,
    members: bool,
) -> Result<Vec<TargetMatch>, AppError> {
    let mut found = Vec::new();
    let mut failure = None;
    if channels {
        match chat.search_channels(term, TARGET_SEARCH_LIMIT) {
            Ok(items) => found.extend(items.into_iter().map(|channel| TargetMatch {
                target: ReviewTarget {
                    channel_id: channel.id,
                    channel_name: channel.name,
                    ..base.clone()
                },
                kind: "channel".into(),
            })),
            Err(err) => failure = Some(err),
        }
    }
    if members {
        match chat.search_users(term, TARGET_SEARCH_LIMIT) {
            Ok(items) => found.extend(items.into_iter().map(|user| TargetMatch {
                target: ReviewTarget {
                    channel_id: user.id,
                    channel_name: user.name,
                    ..base.clone()
                },
                kind: "member".into(),
            })),
            Err(err) => failure = failure.or(Some(err)),
        }
    }
    match failure {
        Some(err) if found.is_empty() => Err(err.into()),
        _ => Ok(found),
    }
}

#[tauri::command]
pub async fn review_preview(
    app: Shared<'_>,
    pr_id: i64,
    target: ReviewTarget,
) -> Result<ReviewPreview, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || review::preview(app.inner().as_ref(), pr_id, target)).await
}

/// Plans again (the state may have moved since the preview) and posts.
#[tauri::command]
pub async fn send_review(
    app: Shared<'_>,
    pr_id: i64,
    target: ReviewTarget,
    teams: Vec<String>,
    note: Option<String>,
) -> Result<SlackIndicator, AppError> {
    let state = app.inner().clone();
    blocking::run(move || {
        review::send(
            &state,
            &review::LiveChats,
            pr_id,
            target,
            &teams,
            note.as_deref(),
        )
    })
    .await
}

/// The routing key of a stored request: its team, else its domain.
pub(crate) fn request_key(request: &pr_nexus_core::domain::ReviewRequest) -> &str {
    request.team_id.as_deref().unwrap_or(&request.workspace)
}

/// Some enabled workspace is signed in (the one the target names first).
pub(crate) fn any_usable(
    state: &AppState,
    workspaces: &[Workspace],
    target: &Target,
) -> Result<bool, AppError> {
    Ok(route::candidates(workspaces, target)
        .iter()
        .any(|ws| token_usable(state, &ws.account, true)))
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
    let team_id = target
        .team_id
        .as_deref()
        .map(str::trim)
        .filter(|team| !team.is_empty())
        .map(str::to_ascii_uppercase);
    if team_id
        .as_deref()
        .is_some_and(|team| !pr_nexus_core::domain::is_slack_team_id(team))
    {
        return Err(AppError::new("validation", "Not a Slack team ID (T…)"));
    }
    let id_ok = channel_id.len() >= 9
        && channel_id.starts_with(['C', 'G', 'D', 'U', 'W'])
        && channel_id.chars().all(|c| c.is_ascii_alphanumeric());
    if !id_ok {
        return Err(AppError::new(
            "validation",
            "Enter a Slack channel ID such as C0123456789 (channel details → About), a DM ID starting with D, or a member ID starting with U",
        ));
    }
    let name = target
        .channel_name
        .trim()
        .trim_start_matches(['#', '@'])
        .to_owned();
    Ok(ReviewTarget {
        workspace,
        team_id,
        workspace_name: target.workspace_name,
        channel_name: if name.is_empty() {
            channel_id.clone()
        } else {
            name
        },
        channel_id,
    })
}

/// The account `key` (a team ID, else a domain) names, when its grant is
/// usable. Never picks the first of several: two workspaces sharing a
/// domain is an error that asks for the team.
pub(crate) fn signed_in_account(state: &AppState, key: &str) -> Result<Option<Account>, AppError> {
    let mut matches: Vec<Account> = workspace_accounts(&state.store)?
        .into_iter()
        .filter(|account| account.is_slack_workspace(key))
        .collect();
    if matches.len() > 1 {
        return Err(AppError::new(
            "validation",
            format!(
                "More than one Slack workspace uses {key}. Pick the channel again so it names its workspace."
            ),
        ));
    }
    Ok(matches
        .pop()
        .filter(|account| token_usable(state, account, true)))
}

/// Refuse a workspace turned off in Settings → Slack.
pub(crate) fn ensure_enabled(state: &AppState, account: &Account) -> Result<(), AppError> {
    let disabled = settings::slack_disabled_workspaces(&state.store)?;
    match account.slack_team_id() {
        Some(team) if disabled.iter().any(|off| off == team) => Err(AppError::new(
            "workspace_disabled",
            format!(
                "The {} Slack workspace is turned off. Turn it on in Settings → Slack.",
                account.display_name
            ),
        )),
        _ => Ok(()),
    }
}

pub(crate) fn chat_for(
    state: &AppState,
    workspace: &str,
) -> Result<Arc<dyn ChatProvider>, AppError> {
    if state.demo {
        let slack = signed_in_account(state, workspace)?;
        if let Some(account) = &slack {
            ensure_enabled(state, account)?;
        }
        let domain = slack
            .as_ref()
            .and_then(|account| account.slack_workspace.clone())
            .unwrap_or_else(|| workspace.to_owned());
        let account = slack
            .or(crate::state::github_account(&state.store)?)
            .ok_or_else(|| AppError::new("unauthorized", "No demo account"))?;
        return Ok(Arc::new(DemoChat {
            account: account.id,
            workspace: domain,
        }));
    }
    let account = signed_in_account(state, workspace)?.ok_or_else(|| {
        let name = workspace_accounts(&state.store)
            .ok()
            .and_then(|accounts| {
                accounts
                    .into_iter()
                    .find(|account| account.is_slack_workspace(workspace))
            })
            .map_or_else(|| workspace.to_owned(), |account| account.display_name);
        AppError::new(
            "unauthorized",
            format!("Slack sign-in expired. Reconnect the {name} workspace in Settings."),
        )
    })?;
    ensure_enabled(state, &account)?;
    let mut chats = lock(&state.chats);
    if let Some(chat) = chats.get(&account.id.0) {
        return Ok(chat.clone());
    }
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
        session_config(state, &account)?,
        state.vault.clone(),
    )?);
    chats.insert(account.id.0, chat.clone());
    Ok(chat)
}

/// How to reopen the MCP session of a signed-in account.
pub(crate) fn session_config(
    state: &AppState,
    account: &Account,
) -> Result<SessionConfig, AppError> {
    let client_id = settings::slack_client_id(&state.store)?.trim().to_owned();
    let client_secret = if client_id.is_empty() {
        None
    } else {
        state.vault.get(&slack_mcp::client_secret_key(&client_id))?
    };
    Ok(SessionConfig {
        mcp_url: settings::slack_mcp_url(&state.store)?,
        client_id,
        client_secret,
        redirect_uri: settings::slack_redirect_uri(&state.store)?,
        keychain_key: account.keychain_key.clone().unwrap_or_default(),
    })
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
pub(crate) fn slack_error(err: CoreError) -> AppError {
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
    /// A made-up reply under the root, so the panel has something to show.
    fn read_thread(&self, _: &str, root_ts: &str, _: u32) -> Result<ThreadReplies, ProviderError> {
        let reply_ts = root_ts
            .split_once('.')
            .and_then(|(secs, _)| secs.parse::<i64>().ok())
            .map_or_else(
                || root_ts.to_owned(),
                |secs| format!("{}.000100", secs + 300),
            );
        Ok(ThreadReplies {
            messages: vec![
                ThreadMessage {
                    ts: root_ts.to_owned(),
                    user_id: Some("U0DEMO".into()),
                    author: None,
                    text: "Could someone please review this? (demo)".into(),
                    is_bot: false,
                },
                ThreadMessage {
                    ts: reply_ts,
                    user_id: Some("U0ACME0002".into()),
                    author: None,
                    text: "On it (demo reply)".into(),
                    is_bot: false,
                },
            ],
            has_more: false,
        })
    }
    fn user_name(&self, user_id: &str) -> Result<String, ProviderError> {
        Ok(if user_id == "U0DEMO" {
            "Ada Acme"
        } else {
            "Bob Acme"
        }
        .into())
    }
    fn search_messages(&self, _: &str, _: bool, _: u32) -> Result<Vec<SearchHit>, ProviderError> {
        Ok(Vec::new())
    }
    fn search_channels(&self, query: &str, _: u32) -> Result<Vec<ChatChannel>, ProviderError> {
        Ok(demo_matches(DEMO_CHANNELS, query)
            .map(|(id, name)| ChatChannel {
                id: id.into(),
                name: name.into(),
            })
            .collect())
    }
    fn search_users(&self, query: &str, _: u32) -> Result<Vec<ChatUser>, ProviderError> {
        Ok(demo_matches(DEMO_MEMBERS, query)
            .map(|(id, name)| ChatUser {
                id: id.into(),
                name: name.into(),
            })
            .collect())
    }
}

const DEMO_CHANNELS: &[(&str, &str)] = &[
    ("C0ACME0001", "team-reviews"),
    ("C0ACME0002", "frontend"),
    ("C0ACME0003", "platform"),
    ("C0ACME0004", "releases"),
];
const DEMO_MEMBERS: &[(&str, &str)] = &[
    ("U0ACME0002", "Grace Hopper"),
    ("U0ACME0003", "Linus Acme"),
    ("U0ACME0004", "Margaret Hamilton"),
];

fn demo_matches<'a>(
    items: &'a [(&'a str, &'a str)],
    query: &str,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .copied()
        .filter(move |(_, name)| name.to_lowercase().contains(&query))
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
            workspace_name: String::new(),
            team_id: None,
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
        let member = validate_target(target("acme", "u0123456789", "@Grace Hopper")).unwrap();
        assert_eq!(member, target("acme", "U0123456789", "Grace Hopper"));
        assert!(validate_target(target("acme", "X0123456789", "")).is_err());
        assert!(validate_target(target("acme", "D012", "")).is_err());
        // No workspace: routing picks the first enabled one that sees it.
        assert!(validate_target(target("", "C0123456789", "")).is_ok());
        let mut bad_team = target("acme", "C0123456789", "");
        bad_team.team_id = Some("acme".into());
        assert!(validate_target(bad_team).is_err());
        let mut team = target("acme", "C0123456789", "");
        team.team_id = Some(" t0acme0001 ".into());
        assert_eq!(
            validate_target(team).unwrap().team_id.as_deref(),
            Some("T0ACME0001")
        );
    }

    #[test]
    fn picker_prefix_picks_the_kind() {
        assert_eq!(target_query("#front"), (true, false, "front"));
        assert_eq!(target_query(" @ grace "), (false, true, "grace"));
        assert_eq!(target_query("plat"), (true, true, "plat"));
        assert_eq!(target_query("#"), (true, false, ""));
    }

    /// Channel search works, people search lacks its scope.
    struct HalfChat;

    impl ChatProvider for HalfChat {
        fn kind(&self) -> ChatKind {
            ChatKind::Slack
        }
        fn whoami(&self) -> Result<ChatUser, ProviderError> {
            Err(ProviderError::Unsupported("test".into()))
        }
        fn list_channels(&self, _: Option<String>) -> Result<Page<ChatChannel>, ProviderError> {
            Err(ProviderError::Unsupported("test".into()))
        }
        fn channel_info(&self, _: &str) -> Result<ChatChannel, ProviderError> {
            Err(ProviderError::Unsupported("test".into()))
        }
        fn post(&self, _: &OutgoingMessage<'_>) -> Result<PostedMessage, ProviderError> {
            Err(ProviderError::Unsupported("test".into()))
        }
        fn account_id(&self) -> AccountId {
            AccountId(1)
        }
        fn search_channels(&self, query: &str, _: u32) -> Result<Vec<ChatChannel>, ProviderError> {
            Ok(vec![ChatChannel {
                id: "C0ACME0001".into(),
                name: format!("{query}-reviews"),
            }])
        }
        fn search_users(&self, _: &str, _: u32) -> Result<Vec<ChatUser>, ProviderError> {
            Err(ProviderError::Forbidden("missing_scope".into()))
        }
    }

    #[test]
    fn search_keeps_what_worked_and_reports_a_lone_failure() {
        let both = search_targets(&HalfChat, &target("acme", "", ""), "team", true, true).unwrap();
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].kind, "channel");
        assert_eq!(both[0].target, target("acme", "C0ACME0001", "team-reviews"));
        assert!(search_targets(&HalfChat, &target("acme", "", ""), "grace", false, true).is_err());
    }

    fn ws(id: i64, team: &str, domain: &str, name: &str, enabled: bool) -> Workspace {
        Workspace {
            account: Account {
                id: AccountId(id),
                kind: AccountKind::Slack,
                display_name: name.into(),
                host: format!("{domain}.slack.com"),
                external_id: format!("{team}:U0ALICE"),
                slack_workspace: Some(domain.into()),
                login: "alice".into(),
                auth_mode: AuthMode::SlackMcpOauth,
                keychain_key: None,
                created_at: Utc::now(),
            },
            team_id: team.into(),
            enabled,
        }
    }

    #[test]
    fn search_merges_every_workspace_dedupes_and_labels() {
        let labs = ws(1, "T0LABS0001", "acme", "acme-labs", true);
        let main = ws(2, "T0ACME0001", "acme-xyz", "acme", true);
        let demo: Arc<dyn ChatProvider> = Arc::new(DemoChat {
            account: AccountId(2),
            workspace: "acme-xyz".into(),
        });
        let mut asked = Vec::new();
        let found = search_all(&[labs.clone(), main.clone()], "team", true, false, |ws| {
            asked.push(ws.team_id.clone());
            if ws.team_id == "T0LABS0001" {
                Ok(Arc::new(HalfChat) as Arc<dyn ChatProvider>)
            } else {
                Ok(demo.clone())
            }
        })
        .unwrap();
        assert_eq!(asked, ["T0LABS0001", "T0ACME0001"]);
        let first = &found[0].target;
        assert_eq!(first.channel_id, "C0ACME0001");
        assert_eq!(
            first.workspace_name, "acme-labs",
            "first workspace wins a duplicate"
        );
        assert_eq!(first.team_id.as_deref(), Some("T0LABS0001"));
        let ids: Vec<&str> = found
            .iter()
            .map(|hit| hit.target.channel_id.as_str())
            .collect();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(ids.len(), unique.len());
        assert!(
            found
                .iter()
                .skip(1)
                .all(|hit| hit.target.workspace_name == "acme")
        );
    }

    #[test]
    fn search_fails_only_when_no_workspace_answers() {
        let labs = ws(1, "T0LABS0001", "acme", "acme-labs", true);
        let main = ws(2, "T0ACME0001", "acme-xyz", "acme", true);
        let found = search_all(&[labs.clone(), main.clone()], "team", true, false, |ws| {
            if ws.team_id == "T0LABS0001" {
                Err(AppError::new("unauthorized", "expired"))
            } else {
                Ok(Arc::new(HalfChat) as Arc<dyn ChatProvider>)
            }
        })
        .unwrap();
        assert_eq!(found[0].target.workspace_name, "acme");
        assert!(
            search_all(&[labs], "team", true, false, |_| Err(AppError::new(
                "unauthorized",
                "x"
            )))
            .is_err()
        );
        assert!(search_all(&[], "team", true, false, |_| unreachable!()).is_err());
    }

    #[test]
    fn labels_name_the_team_not_the_link_host() {
        let all = [
            ws(1, "T0LABS0001", "acme", "acme-labs", true),
            ws(2, "T0ACME0001", "acme-xyz", "acme", false),
        ];
        let mut pinned = target("acme", "C1", "a");
        pinned.team_id = Some("T0ACME0001".into());
        assert_eq!(labeled(&all, pinned).workspace_name, "acme");
        assert_eq!(
            labeled(&all, target("acme", "C1", "a")).workspace_name,
            "acme-labs"
        );
        assert_eq!(
            labeled(&all, target("gone", "C1", "a")).workspace_name,
            "gone"
        );
    }

    #[test]
    fn demo_search_matches_by_name() {
        let demo = DemoChat {
            account: AccountId(1),
            workspace: "acme".into(),
        };
        let people = search_targets(&demo, &target("acme", "", ""), "grace", false, true).unwrap();
        assert_eq!(people[0].kind, "member");
        assert_eq!(people[0].target.channel_id, "U0ACME0002");
        assert!(
            search_targets(
                &demo,
                &target("acme", "", ""),
                "nothing-matches",
                true,
                true
            )
            .unwrap()
            .is_empty()
        );
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
