//! The PR detail's Slack panel (§7.10): known threads and the opt-in
//! Mentions search. Read-only: nothing here posts.

use std::sync::Arc;

use chrono::Utc;
use pr_nexus_core::error::CoreError;
use pr_nexus_core::providers::ChatProvider;
use pr_nexus_core::review::context::{self, ContextThread, MentionGroup};
use pr_nexus_core::settings::{self, MentionsScope};
use tauri::State;

use crate::blocking;
use crate::error::AppError;
use crate::slack::{chat_for, load_pr, mark_rejected, signed_in_account, workspace_accounts};
use crate::state::AppState;

type Shared<'a> = State<'a, Arc<AppState>>;

/// Known threads of one PR, cached for 5 minutes unless `refresh`.
#[tauri::command]
pub async fn pr_slack_context(
    app: Shared<'_>,
    pr_id: i64,
    refresh: bool,
) -> Result<Vec<ContextThread>, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || threads_blocking(&state, pr_id, refresh))
        .await
        .map_err(|err| AppError::new("internal", err.to_string()))?
}

fn threads_blocking(
    state: &AppState,
    pr_id: i64,
    refresh: bool,
) -> Result<Vec<ContextThread>, AppError> {
    let pr = load_pr(&state.store, pr_id)?;
    let mut chat = |workspace: &str| {
        chat_for(state, workspace).map_err(|err| CoreError::Validation(err.message))
    };
    let threads = context::context_threads(&state.store, pr.id, &mut chat, Utc::now(), refresh)?;
    for thread in threads.iter().filter(|thread| thread.unauthorized) {
        if let Some(account) = signed_in_account(state, &thread.workspace)? {
            mark_rejected(state, account.id.0);
        }
    }
    Ok(threads)
}

/// One search per signed-in workspace for the PR link. Refused while the
/// setting is off.
#[tauri::command]
pub async fn pr_slack_mentions(app: Shared<'_>, pr_id: i64) -> Result<Vec<MentionGroup>, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || mentions_blocking(&state, pr_id))
        .await
        .map_err(|err| AppError::new("internal", err.to_string()))?
}

fn mentions_blocking(state: &AppState, pr_id: i64) -> Result<Vec<MentionGroup>, AppError> {
    let scope = settings::slack_mentions(&state.store)?;
    if scope == MentionsScope::Off {
        return Err(AppError::new(
            "validation",
            "Mentions search is off. Turn it on in Settings → Slack.",
        ));
    }
    let pr = load_pr(&state.store, pr_id)?;
    let mut workspaces: Vec<(String, Arc<dyn ChatProvider>)> = Vec::new();
    for account in workspace_accounts(&state.store)? {
        let Some(domain) = account.slack_workspace.clone() else {
            continue;
        };
        if workspaces.iter().any(|(seen, _)| *seen == domain) {
            continue;
        }
        if let Ok(chat) = chat_for(state, &domain) {
            workspaces.push((domain, chat));
        }
    }
    if workspaces.is_empty() {
        return Err(AppError::new(
            "unauthorized",
            "No signed-in Slack workspace. Connect one in Settings → Slack.",
        ));
    }
    let groups = context::mentions(
        &state.store,
        pr.id,
        &pr.url,
        &workspaces,
        scope == MentionsScope::All,
    )?;
    for group in groups.iter().filter(|group| group.unauthorized) {
        if let Some(account) = signed_in_account(state, &group.workspace)? {
            mark_rejected(state, account.id.0);
        }
    }
    Ok(groups)
}

#[tauri::command]
pub async fn save_slack_mentions(app: Shared<'_>, scope: String) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let scope = MentionsScope::parse(&scope)
            .ok_or_else(|| AppError::new("validation", "Mentions must be off, public, or all"))?;
        settings::set_slack_mentions(&app.inner().store, scope)?;
        Ok(())
    })
    .await
}
