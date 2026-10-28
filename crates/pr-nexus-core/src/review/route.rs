//! Which Slack workspace sends to a target (§7.11).
//!
//! Workspaces are identified by team ID. A domain is only a hint: in a grid a
//! channel link carries the org host, which can be another workspace's
//! domain. Before a request, each enabled workspace is probed read-only in
//! priority order, and the first that can see the target sends. A send that
//! failed with an error proving nothing was posted moves to the next
//! workspace; any other failure is surfaced, because a duplicate post is
//! worse than a failed one.

use std::sync::Arc;

use crate::domain::{Account, AccountKind};
use crate::error::{CoreError, ProviderError};
use crate::providers::ChatProvider;
use crate::review::lifecycle::Target;
use crate::settings;
use crate::store::Store;

/// Slack errors that guarantee nothing was posted, so the next workspace
/// may try.
pub const SAFE_CODES: [&str; 3] = ["channel_not_found", "not_in_channel", "user_not_found"];

/// A connected Slack workspace with its routing switches.
#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub account: Account,
    pub team_id: String,
    pub enabled: bool,
}

impl Workspace {
    pub fn domain(&self) -> &str {
        self.account.slack_workspace.as_deref().unwrap_or_default()
    }

    /// The name shown to the user: the team name, else the domain.
    pub fn label(&self) -> &str {
        if self.account.display_name.is_empty() {
            self.domain()
        } else {
            &self.account.display_name
        }
    }

    /// `target` pointed at this workspace.
    pub fn stamp(&self, target: &Target) -> Target {
        Target {
            workspace: self.domain().to_owned(),
            team_id: Some(self.team_id.clone()),
            ..target.clone()
        }
    }
}

/// Every connected workspace in priority order, enabled or not.
pub fn workspaces(store: &Store) -> Result<Vec<Workspace>, CoreError> {
    let accounts = store
        .list_accounts()?
        .into_iter()
        .filter(|account| account.kind == AccountKind::Slack)
        .collect();
    Ok(ordered(
        accounts,
        &settings::slack_workspace_order(store)?,
        &settings::slack_disabled_workspaces(store)?,
    ))
}

/// Listed teams first in list order, then the rest in connection order.
pub fn ordered(accounts: Vec<Account>, order: &[String], disabled: &[String]) -> Vec<Workspace> {
    let mut out: Vec<Workspace> = accounts
        .into_iter()
        .filter_map(|account| {
            let team_id = account.slack_team_id()?.to_owned();
            Some(Workspace {
                enabled: !disabled.contains(&team_id),
                team_id,
                account,
            })
        })
        .collect();
    out.sort_by_key(|ws| {
        (
            order
                .iter()
                .position(|team| *team == ws.team_id)
                .unwrap_or(usize::MAX),
            ws.account.id.0,
        )
    });
    out
}

/// Enabled workspaces to try for `target`: its team first when known, else
/// the one whose domain matches, then the rest in priority order.
pub fn candidates<'a>(workspaces: &'a [Workspace], target: &Target) -> Vec<&'a Workspace> {
    let preferred = |ws: &Workspace| match &target.team_id {
        Some(team) => ws.team_id == *team,
        None => ws.domain().eq_ignore_ascii_case(&target.workspace),
    };
    let enabled = workspaces.iter().filter(|ws| ws.enabled);
    enabled
        .clone()
        .filter(|ws| preferred(ws))
        .chain(enabled.filter(|ws| !preferred(ws)))
        .collect()
}

/// The workspace a stored target names: its team, else the one workspace
/// with that domain. Two workspaces sharing a domain match neither.
pub fn find<'a>(
    workspaces: &'a [Workspace],
    team_id: Option<&str>,
    domain: &str,
) -> Option<&'a Workspace> {
    if let Some(team) = team_id {
        return workspaces.iter().find(|ws| ws.team_id == team);
    }
    let mut matches = workspaces
        .iter()
        .filter(|ws| ws.domain().eq_ignore_ascii_case(domain));
    match (matches.next(), matches.next()) {
        (Some(ws), None) => Some(ws),
        _ => None,
    }
}

/// Probe only: the first enabled workspace that can see `target`, for
/// batches that send later through a fixed workspace.
pub fn resolve(
    workspaces: &[Workspace],
    target: &Target,
    chat_for: &mut ChatForWorkspace<'_>,
) -> Result<Workspace, CoreError> {
    send_routed(workspaces, target, chat_for, &mut |_, _| Ok(())).map(|routed| routed.workspace)
}

/// Whether `err` proves nothing was posted, so another workspace may try.
/// Timeouts, 5xx, rate limits, unconfirmed sends and unknown errors are not.
pub fn fallback_safe(err: &CoreError) -> bool {
    match err {
        CoreError::Provider(ProviderError::Unauthorized(_)) => true,
        CoreError::Provider(ProviderError::Api { code, .. }) => SAFE_CODES.contains(&code.as_str()),
        _ => false,
    }
}

/// The result of a routed send.
#[derive(Debug)]
pub struct Routed<T> {
    pub value: T,
    /// The workspace that sent; persist its team on the target.
    pub workspace: Workspace,
    /// Workspaces skipped before it, with why.
    pub skipped: Vec<(String, String)>,
}

pub type ChatForWorkspace<'a> =
    dyn FnMut(&Workspace) -> Result<Arc<dyn ChatProvider>, CoreError> + 'a;

/// Probe, then send through the first enabled workspace that can see the
/// target; fall back only on [`fallback_safe`] errors.
pub fn send_routed<T>(
    workspaces: &[Workspace],
    target: &Target,
    chat_for: &mut ChatForWorkspace<'_>,
    send: &mut dyn FnMut(&dyn ChatProvider, &Target) -> Result<T, CoreError>,
) -> Result<Routed<T>, CoreError> {
    let candidates = candidates(workspaces, target);
    if candidates.is_empty() {
        return Err(CoreError::Validation(
            "No enabled Slack workspace. Turn one on in Settings → Slack.".into(),
        ));
    }
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut last: Option<CoreError> = None;
    for ws in candidates {
        let chat = match chat_for(ws) {
            Ok(chat) => chat,
            Err(err) => {
                skipped.push((ws.label().to_owned(), err.to_string()));
                last = Some(err);
                continue;
            }
        };
        match chat.channel_info(&target.channel_id) {
            Ok(_) | Err(ProviderError::Unsupported(_)) => {}
            Err(err) => {
                let err = CoreError::Provider(err);
                if !fallback_safe(&err) {
                    return Err(err);
                }
                skipped.push((ws.label().to_owned(), err.to_string()));
                last = Some(err);
                continue;
            }
        }
        match send(chat.as_ref(), &ws.stamp(target)) {
            Ok(value) => {
                return Ok(Routed {
                    value,
                    workspace: ws.clone(),
                    skipped,
                });
            }
            Err(err) if fallback_safe(&err) => {
                skipped.push((ws.label().to_owned(), err.to_string()));
                last = Some(err);
            }
            Err(err) => return Err(err),
        }
    }
    let tried = skipped
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(match last {
        Some(CoreError::Provider(ProviderError::Api { code, message })) => {
            CoreError::Provider(ProviderError::Api {
                code,
                message: format!("{message} (tried {tried})"),
            })
        }
        Some(err) => err,
        None => CoreError::Validation("No Slack workspace could send".into()),
    })
}

#[cfg(test)]
mod tests;
