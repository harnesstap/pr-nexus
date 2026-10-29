//! Who answers a Slack tool for one workspace: the official Slack MCP
//! server through that workspace's own session, or demo fixtures.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use pr_nexus_core::domain::Account;
use pr_nexus_providers::slack_mcp::{self, RmcpSession, ToolCaller};
use rmcp::model::JsonObject;

use crate::error::AppError;
use crate::slack::lock;
use crate::state::AppState;

/// A workspace the user exposed to agents in Settings → Agents.
#[derive(Debug, Clone)]
pub struct Exposed {
    pub account: Account,
    pub team_id: String,
    pub domain: String,
    pub name: String,
    pub user_name: String,
    pub signed_in: bool,
    /// The Grid org, when the session belongs to one.
    pub enterprise_id: Option<String>,
    /// Requested scopes the grant lacks; signing in again adds them.
    pub scopes_missing: Vec<String>,
}

impl Exposed {
    /// What qualifies this workspace's IDs (see [`super::ids`]).
    pub fn qualifier(&self) -> &str {
        self.enterprise_id.as_deref().unwrap_or(&self.team_id)
    }
}

/// The session behind one workspace. Tests and the demo swap it.
pub trait Upstream: Send + Sync {
    fn caller(&self, state: &AppState, ws: &Exposed) -> Result<Arc<dyn ToolCaller>, AppError>;
    /// The Grid org of the workspace's token, if any.
    fn enterprise(&self, state: &AppState, account: &Account) -> Option<String>;
    /// The scopes the grant lists; `None` when unknown.
    fn granted_scopes(&self, state: &AppState, account: &Account) -> Option<Vec<String>>;
}

/// The official Slack MCP server, one session per account.
#[derive(Default)]
pub struct Official {
    sessions: Mutex<HashMap<i64, Arc<dyn ToolCaller>>>,
}

impl Upstream for Official {
    fn caller(&self, state: &AppState, ws: &Exposed) -> Result<Arc<dyn ToolCaller>, AppError> {
        if let Some(caller) = lock(&self.sessions).get(&ws.account.id.0) {
            return Ok(caller.clone());
        }
        let config = crate::slack::session_config(state, &ws.account)?;
        let caller: Arc<dyn ToolCaller> = Arc::new(RmcpSession::new(config, state.vault.clone())?);
        lock(&self.sessions).insert(ws.account.id.0, caller.clone());
        Ok(caller)
    }

    fn enterprise(&self, state: &AppState, account: &Account) -> Option<String> {
        let key = account.keychain_key.clone()?;
        let handle = tokio::runtime::Handle::try_current().ok()?;
        handle.block_on(slack_mcp::enterprise_id(
            slack_mcp::SLACK_WEB_API,
            state.vault.clone(),
            &key,
        ))
    }

    fn granted_scopes(&self, state: &AppState, account: &Account) -> Option<Vec<String>> {
        crate::slack::granted_scopes(state, account)
    }
}

/// The scope a tool cannot run without, when the grant may lack it.
pub fn required_scope(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "slack_search_public_and_private" => "search:read.private",
        "slack_get_reactions" => "reactions:read",
        "slack_search_emojis" => "emoji:read",
        "slack_read_file" => "files:read",
        "slack_read_canvas" => "canvases:read",
        "slack_read_list" => "lists:read",
        _ => return None,
    })
}

/// Requests per minute Slack allows a tool, per user and workspace.
pub fn per_minute(tool: &str) -> usize {
    match tool {
        "slack_search_users" | "slack_search_channels" | "slack_list_user_channels" => 20,
        // Slack documents "special" limits for message search; stay well under.
        "slack_search_public" | "slack_search_public_and_private" => 20,
        // Tier 2.
        "slack_search_emojis" | "slack_read_list" => 20,
        // Tier 3.
        "slack_get_reactions" | "slack_read_canvas" => 50,
        "slack_read_channel" | "slack_read_thread" => 50,
        _ => 100,
    }
}

/// Shared state of the Slack tools for one app run.
pub struct SlackProxy {
    upstream: RwLock<Option<Arc<dyn Upstream>>>,
    official: Arc<Official>,
    enterprises: Mutex<HashMap<i64, Option<String>>>,
    calls: Mutex<HashMap<(i64, String), VecDeque<Instant>>>,
    /// Signs page cursors; new on every run, so old cursors expire.
    pub(crate) key: [u8; 32],
}

impl Default for SlackProxy {
    fn default() -> Self {
        Self {
            upstream: RwLock::new(None),
            official: Arc::default(),
            enterprises: Mutex::default(),
            calls: Mutex::default(),
            key: rand::random(),
        }
    }
}

impl SlackProxy {
    /// Answer every workspace from `upstream` instead (tests).
    pub fn set_upstream(&self, upstream: Arc<dyn Upstream>) {
        *self.upstream.write().expect("upstream lock") = Some(upstream);
        lock(&self.enterprises).clear();
    }

    fn upstream(&self, state: &AppState) -> Arc<dyn Upstream> {
        if let Some(upstream) = self.upstream.read().expect("upstream lock").clone() {
            return upstream;
        }
        if state.demo {
            return Arc::new(super::fixtures::Fixtures);
        }
        self.official.clone()
    }

    pub fn enterprise(&self, state: &AppState, account: &Account) -> Option<String> {
        if let Some(known) = lock(&self.enterprises).get(&account.id.0) {
            return known.clone();
        }
        let found = self.upstream(state).enterprise(state, account);
        lock(&self.enterprises).insert(account.id.0, found.clone());
        found
    }

    /// Requested scopes `account`'s grant lacks (empty when unknown).
    pub fn scopes_missing(&self, state: &AppState, account: &Account) -> Vec<String> {
        self.upstream(state)
            .granted_scopes(state, account)
            .map(|granted| slack_mcp::missing_scopes(&granted))
            .unwrap_or_default()
    }

    /// Take one call of `tool`'s per-minute budget, or say when one frees up.
    fn take(&self, account: i64, tool: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let mut calls = lock(&self.calls);
        let recent = calls.entry((account, tool.to_owned())).or_default();
        while recent
            .front()
            .is_some_and(|at| now.duration_since(*at) >= window)
        {
            recent.pop_front();
        }
        if recent.len() >= per_minute(tool) {
            let oldest = *recent.front().expect("full window");
            return Err(window.saturating_sub(now.duration_since(oldest)));
        }
        recent.push_back(now);
        Ok(())
    }

    /// Call `tool` on one workspace. Error replies become errors.
    pub fn call(
        &self,
        state: &AppState,
        ws: &Exposed,
        tool: &str,
        arguments: JsonObject,
    ) -> Result<String, AppError> {
        if !ws.signed_in {
            return Err(AppError::new(
                "unauthorized",
                "This workspace needs signing in again in PR Nexus (Settings → Slack).",
            ));
        }
        crate::slack::ensure_enabled(state, &ws.account)?;
        if let Some(scope) = required_scope(tool)
            && ws.scopes_missing.iter().any(|s| s == scope)
        {
            return Err(AppError::new(
                "forbidden",
                format!(
                    "missing_scope: this workspace's Slack sign-in predates the `{scope}` permission. \
                     The user can add it in PR Nexus → Settings → Agents (Sign in again)."
                ),
            ));
        }
        if let Err(wait) = self.take(ws.account.id.0, tool) {
            let mut err = AppError::new(
                "rate_limited",
                "PR Nexus paused this call to stay under Slack's rate limit.",
            );
            err.retry_at = Some((chrono::Utc::now() + wait).to_rfc3339());
            return Err(err);
        }
        let caller = self.upstream(state).caller(state, ws)?;
        let output = caller.call(tool, arguments).map_err(|err| {
            if matches!(err, pr_nexus_core::error::ProviderError::Unauthorized(_)) {
                crate::slack::mark_rejected(state, ws.account.id.0);
                lock(&self.official.sessions).remove(&ws.account.id.0);
            }
            AppError::from(err)
        })?;
        if output.is_error {
            return Err(slack_mcp::schema::map_error(tool, &output.text, None).into());
        }
        Ok(output.text)
    }
}
