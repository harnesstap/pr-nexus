//! Typed access to the `settings` table.
//!
//! Values are JSON. Missing keys return the spec defaults (§7.12).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::SettingEntry;
use crate::error::CoreError;
use crate::providers::RateBudget;
use crate::store::Store;

pub const VCS: &str = "providers.vcs";
pub const CHAT: &str = "providers.chat";
pub const ONBOARDING: &str = "onboarding.state";
pub const TRACKED_ORGS: &str = "github.tracked_orgs";
pub const INCLUDE_PERSONAL: &str = "github.include_personal_repos";
pub const VIEWER_ORGS: &str = "github.viewer_orgs";
pub const SEEN_ORGS: &str = "github.seen_orgs";
pub const SSO_ORGS: &str = "github.sso_orgs";
pub const OAUTH_CLIENT_ID: &str = "github.oauth_client_id";
pub const KEEP_CLOSED_DAYS: &str = "sync.keep_closed_days";
pub const FOCUSED_INTERVAL: &str = "sync.focused_interval_s";
pub const UNFOCUSED_INTERVAL: &str = "sync.unfocused_interval_s";
pub const RATE_BUDGET: &str = "github.rate_budget";
pub const SLACK_CLIENT_ID: &str = "slack.mcp.client_id";
pub const SLACK_REDIRECT: &str = "slack.mcp.redirect_uri";
pub const BLOCKED_IS_READY: &str = "review.blocked_review_required_is_ready";
pub const TICKET_KEY_REGEX: &str = "projects.ticket_key_regex";
pub const SLACK_MCP_URL: &str = "slack.mcp.url";
pub const REVIEW_REQUESTER: &str = "review.requester";
pub const MIN_BUMP_INTERVAL: &str = "review.min_bump_interval_hours";
pub const BULK_BUMP_MAX_THREADS: &str = "review.bulk_bump_max_threads";
/// Mentions search in the Slack panel (§7.10): `off`, `public`, or `all`.
pub const SLACK_MENTIONS: &str = "slack.mentions";
/// Bumps are thread replies; with this on they are also sent to the channel.
pub const BUMP_BROADCAST: &str = "review.bump_broadcast";

pub const DEFAULT_REDIRECT: &str = "http://127.0.0.1:47821/mcp/oauth/callback";

pub fn tracked_orgs(store: &Store) -> Result<Vec<String>, CoreError> {
    json_or(store, TRACKED_ORGS, Vec::new())
}

pub fn set_tracked_orgs(store: &Store, orgs: &[String]) -> Result<(), CoreError> {
    put_json(store, TRACKED_ORGS, orgs)
}

pub fn include_personal_repos(store: &Store) -> Result<bool, CoreError> {
    json_or(store, INCLUDE_PERSONAL, false)
}

pub fn set_include_personal_repos(store: &Store, value: bool) -> Result<(), CoreError> {
    put_json(store, INCLUDE_PERSONAL, &value)
}

pub fn viewer_orgs(store: &Store) -> Result<Vec<String>, CoreError> {
    json_or(store, VIEWER_ORGS, Vec::new())
}

pub fn set_viewer_orgs(store: &Store, orgs: &[String]) -> Result<(), CoreError> {
    put_json(store, VIEWER_ORGS, orgs)
}

/// Orgs the user belongs to that the token cannot read until SSO is authorized.
pub fn sso_orgs(store: &Store) -> Result<Vec<String>, CoreError> {
    json_or(store, SSO_ORGS, Vec::new())
}

pub fn set_sso_orgs(store: &Store, orgs: &[String]) -> Result<(), CoreError> {
    put_json(store, SSO_ORGS, orgs)
}

pub fn seen_orgs(store: &Store) -> Result<Vec<String>, CoreError> {
    json_or(store, SEEN_ORGS, Vec::new())
}

pub fn set_seen_orgs(store: &Store, orgs: &[String]) -> Result<(), CoreError> {
    put_json(store, SEEN_ORGS, orgs)
}

pub fn keep_closed_days(store: &Store) -> Result<i64, CoreError> {
    json_or(store, KEEP_CLOSED_DAYS, 7)
}

pub fn focused_interval_s(store: &Store) -> Result<u64, CoreError> {
    json_or(store, FOCUSED_INTERVAL, 180)
}

pub fn unfocused_interval_s(store: &Store) -> Result<u64, CoreError> {
    json_or(store, UNFOCUSED_INTERVAL, 900)
}

pub fn blocked_review_required_is_ready(store: &Store) -> Result<bool, CoreError> {
    json_or(store, BLOCKED_IS_READY, true)
}

pub fn ticket_key_regex(store: &Store) -> Result<String, CoreError> {
    Ok(optional_string(store, TICKET_KEY_REGEX)?
        .unwrap_or_else(|| crate::linking::DEFAULT_TICKET_KEY_REGEX.to_owned()))
}

pub fn slack_client_id(store: &Store) -> Result<String, CoreError> {
    json_or(store, SLACK_CLIENT_ID, String::new())
}

pub fn set_slack_client_id(store: &Store, value: &str) -> Result<(), CoreError> {
    put_json(store, SLACK_CLIENT_ID, &value)
}

pub fn slack_redirect_uri(store: &Store) -> Result<String, CoreError> {
    json_or(store, SLACK_REDIRECT, DEFAULT_REDIRECT.to_owned())
}

pub fn set_slack_redirect_uri(store: &Store, value: &str) -> Result<(), CoreError> {
    put_json(store, SLACK_REDIRECT, &value)
}

pub const DEFAULT_SLACK_MCP_URL: &str = "https://mcp.slack.com/mcp";

pub fn slack_mcp_url(store: &Store) -> Result<String, CoreError> {
    let url = json_or(store, SLACK_MCP_URL, DEFAULT_SLACK_MCP_URL.to_owned())?;
    Ok(if url.trim().is_empty() {
        DEFAULT_SLACK_MCP_URL.to_owned()
    } else {
        url
    })
}

/// `review.requester`: empty means no suffix (D6).
pub fn review_requester(store: &Store) -> Result<Option<String>, CoreError> {
    let value: String = json_or(store, REVIEW_REQUESTER, String::new())?;
    Ok(Some(value.trim().to_owned()).filter(|value| !value.is_empty()))
}

pub fn min_bump_interval_hours(store: &Store) -> Result<i64, CoreError> {
    json_or(store, MIN_BUMP_INTERVAL, 24)
}

/// Who the Slack panel's Mentions button searches (§7.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MentionsScope {
    /// No button. The default.
    Off,
    /// `slack_search_public`: public channels only.
    Public,
    /// `slack_search_public_and_private`: also private channels and DMs the
    /// user is in. An explicit opt-in.
    All,
}

impl MentionsScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Public => "public",
            Self::All => "all",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "public" => Some(Self::Public),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

pub fn slack_mentions(store: &Store) -> Result<MentionsScope, CoreError> {
    let value: String = json_or(store, SLACK_MENTIONS, "off".to_owned())?;
    Ok(MentionsScope::parse(&value).unwrap_or(MentionsScope::Off))
}

pub fn set_slack_mentions(store: &Store, scope: MentionsScope) -> Result<(), CoreError> {
    put_json(store, SLACK_MENTIONS, &scope.as_str())
}

/// `review.bump_broadcast`: whether a bump reply is also sent to the
/// channel (Slack's `reply_broadcast`). Defaults to on.
pub fn bump_broadcast(store: &Store) -> Result<bool, CoreError> {
    json_or(store, BUMP_BROADCAST, true)
}

pub fn set_bump_broadcast(store: &Store, on: bool) -> Result<(), CoreError> {
    put_json(store, BUMP_BROADCAST, &on)
}

/// `review.bulk_bump_max_threads`: above this many distinct roots in one
/// channel, a bulk bump becomes one top-level remind (§7.9). At least 1.
pub fn bulk_bump_max_threads(store: &Store) -> Result<usize, CoreError> {
    Ok(json_or::<usize>(store, BULK_BUMP_MAX_THREADS, 3)?.max(1))
}

pub fn oauth_client_id(store: &Store) -> Result<String, CoreError> {
    json_or(store, OAUTH_CLIENT_ID, String::new())
}

pub fn set_provider_kind(store: &Store, vcs: &str, chat: &str) -> Result<(), CoreError> {
    put_json(store, VCS, &vcs)?;
    put_json(store, CHAT, &chat)
}

pub fn provider_kinds(store: &Store) -> Result<(Option<String>, Option<String>), CoreError> {
    Ok((optional_string(store, VCS)?, optional_string(store, CHAT)?))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredBudget {
    pub limit: u32,
    pub remaining: u32,
    pub reset_at: String,
    pub cost: u32,
    #[serde(default)]
    pub secondary_until: Option<String>,
}

pub fn rate_budget(store: &Store) -> Result<Option<RateBudget>, CoreError> {
    let Some(stored) = get_json::<StoredBudget>(store, RATE_BUDGET)? else {
        return Ok(None);
    };
    Ok(Some(RateBudget {
        limit: stored.limit,
        remaining: stored.remaining,
        reset_at: parse_time(&stored.reset_at)?,
        cost: stored.cost,
        secondary_until: stored
            .secondary_until
            .as_deref()
            .map(parse_time)
            .transpose()?,
    }))
}

pub fn set_rate_budget(store: &Store, budget: &RateBudget) -> Result<(), CoreError> {
    put_json(
        store,
        RATE_BUDGET,
        &StoredBudget {
            limit: budget.limit,
            remaining: budget.remaining,
            reset_at: budget.reset_at.to_rfc3339(),
            cost: budget.cost,
            secondary_until: budget.secondary_until.map(|value| value.to_rfc3339()),
        },
    )
}

fn parse_time(text: &str) -> Result<DateTime<Utc>, CoreError> {
    DateTime::parse_from_rfc3339(text)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| CoreError::Validation(format!("timestamp {text}: {err}")))
}

fn optional_string(store: &Store, key: &str) -> Result<Option<String>, CoreError> {
    match get_json::<String>(store, key)? {
        Some(value) if value.is_empty() => Ok(None),
        other => Ok(other),
    }
}

pub fn put_json<T: Serialize + ?Sized>(
    store: &Store,
    key: &str,
    value: &T,
) -> Result<(), CoreError> {
    let value_json = serde_json::to_string(value)
        .map_err(|err| CoreError::Store(format!("encode {key}: {err}")))?;
    store.put_setting(SettingEntry {
        key: key.to_owned(),
        value_json,
    })
}

fn json_or<T: for<'de> Deserialize<'de>>(
    store: &Store,
    key: &str,
    default: T,
) -> Result<T, CoreError> {
    Ok(get_json(store, key)?.unwrap_or(default))
}

fn get_json<T: for<'de> Deserialize<'de>>(
    store: &Store,
    key: &str,
) -> Result<Option<T>, CoreError> {
    let Some(entry) = store.get_setting(key.to_owned())? else {
        return Ok(None);
    };
    serde_json::from_str(&entry.value_json)
        .map(Some)
        .map_err(|err| CoreError::Store(format!("decode {key}: {err}")))
}
