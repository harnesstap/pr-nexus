//! The single-PR Slack review flow: preview, then request or bump (§7.7).
//!
//! [`send`] takes a [`ChatSource`] so tests (and any other caller) can hand
//! in a fake Slack. The app passes [`LiveChats`].

use std::sync::Arc;

use chrono::Utc;
use pr_nexus_core::domain::PullRequest;
use pr_nexus_core::error::{CoreError, ProviderError};
use pr_nexus_core::providers::ChatProvider;
use pr_nexus_core::review::lifecycle::{self, Action, Target};
use pr_nexus_core::review::route::{self, Workspace};
use pr_nexus_core::settings;

use crate::commands::{SlackIndicator, indicator};
use crate::error::AppError;
use crate::slack::{
    ReviewPreview, ReviewTarget, any_usable, chat_for, labeled, load_pr, mark_rejected,
    pr_warnings, request_key, signed_in_account, slack_error, to_core, validate_target,
};
use crate::state::AppState;

/// Where Slack sessions come from. `key` is a team ID, else a domain.
pub trait ChatSource {
    fn chat(&self, state: &AppState, key: &str) -> Result<Arc<dyn ChatProvider>, AppError>;
}

/// The signed-in Slack MCP sessions (the demo chat in demo mode).
pub struct LiveChats;

impl ChatSource for LiveChats {
    fn chat(&self, state: &AppState, key: &str) -> Result<Arc<dyn ChatProvider>, AppError> {
        chat_for(state, key)
    }
}

/// What [`send`] would post for `pr_id` to `target`: a new request, or a bump
/// of the active one. Posts nothing.
pub fn preview(
    state: &AppState,
    pr_id: i64,
    target: ReviewTarget,
) -> Result<ReviewPreview, AppError> {
    let pr = load_pr(&state.store, pr_id)?;
    let target = validate_target(target)?;
    let plan = plan(state, &pr, &target)?;
    let workspaces = route::workspaces(&state.store)?;
    let (action, bump_count, permalink, signed_in) = match &plan.action {
        Action::Request => (
            "request",
            0,
            None,
            state.demo || any_usable(state, &workspaces, &to_core(&target))?,
        ),
        Action::Bump { request } => (
            "bump",
            request.bump_count,
            request.permalink.clone(),
            state.demo
                || signed_in_account(state, request_key(request))
                    .ok()
                    .flatten()
                    .is_some(),
        ),
    };
    Ok(ReviewPreview {
        target: labeled(&workspaces, target),
        action: action.into(),
        text: plan.text,
        warnings: plan.warnings,
        pr_warnings: pr_warnings(&pr),
        signed_in,
        bump_count,
        permalink,
        broadcast: settings::bump_broadcast(&state.store)?,
    })
}

/// Plans again (the state may have moved since the preview) and posts: a new
/// request routed to the first workspace that sees the channel, or a bump in
/// the active request's thread.
pub fn send(
    state: &AppState,
    chats: &dyn ChatSource,
    pr_id: i64,
    target: ReviewTarget,
    teams: &[String],
    note: Option<&str>,
) -> Result<SlackIndicator, AppError> {
    let pr = load_pr(&state.store, pr_id)?;
    let target = validate_target(target)?;
    let plan = lifecycle::with_note(plan(state, &pr, &target)?, note);
    let teams_json = serde_json::to_string(teams).unwrap_or_else(|_| "[]".into());
    let mut send = |chat: &dyn ChatProvider, target: &Target| {
        let plan = lifecycle::Plan {
            target: target.clone(),
            ..plan.clone()
        };
        let sent = lifecycle::send(&state.store, chat, &pr, &plan, &teams_json, Utc::now());
        if let Err(CoreError::Provider(ProviderError::Unauthorized(_))) = &sent {
            mark_rejected(state, chat.account_id().0);
        }
        sent
    };
    let request = match &plan.action {
        // A bump replies in the request's thread: that workspace or nothing.
        Action::Bump { request } => {
            let chat = chats.chat(state, request_key(request))?;
            send(chat.as_ref(), &plan.target).map_err(slack_error)?
        }
        Action::Request => {
            let workspaces = route::workspaces(&state.store)?;
            let mut chat_for_ws = |ws: &Workspace| {
                chats
                    .chat(state, &ws.team_id)
                    .map_err(|err| CoreError::Validation(err.message))
            };
            let routed = route::send_routed(&workspaces, &plan.target, &mut chat_for_ws, &mut send)
                .map_err(slack_error)?;
            state.store.pin_mapping_team(
                plan.target.channel_id.clone(),
                plan.target.team_id.clone(),
                plan.target.workspace.clone(),
                routed.workspace.team_id.clone(),
                routed.workspace.domain().to_owned(),
            )?;
            routed.value
        }
    };
    Ok(indicator(request, &route::workspaces(&state.store)?))
}

/// The request or bump `target` would get now.
pub fn plan(
    state: &AppState,
    pr: &PullRequest,
    target: &ReviewTarget,
) -> Result<lifecycle::Plan, AppError> {
    let requester = settings::review_requester(&state.store)?;
    Ok(lifecycle::plan(
        &state.store,
        pr,
        to_core(target),
        requester.as_deref(),
        settings::min_bump_interval_hours(&state.store)?,
        Utc::now(),
    )?)
}
