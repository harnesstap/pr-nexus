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
pub const OAUTH_CLIENT_ID: &str = "github.oauth_client_id";
pub const KEEP_CLOSED_DAYS: &str = "sync.keep_closed_days";
pub const FOCUSED_INTERVAL: &str = "sync.focused_interval_s";
pub const UNFOCUSED_INTERVAL: &str = "sync.unfocused_interval_s";
pub const RATE_BUDGET: &str = "github.rate_budget";
pub const SLACK_CLIENT_ID: &str = "slack.mcp.client_id";
pub const SLACK_REDIRECT: &str = "slack.mcp.redirect_uri";
pub const BLOCKED_IS_READY: &str = "review.blocked_review_required_is_ready";

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
