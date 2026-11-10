//! Typed access to the `settings` table.
//!
//! Values are JSON. Missing keys return the spec defaults (§7.12).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::SettingEntry;
use crate::error::CoreError;
use crate::providers::RateBudget;
use crate::review::message_template::{
    MessageTemplates, TemplateKind, validate as validate_template,
};
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
/// Incremental sync's high-water mark (§6), see [`SyncMark`].
pub const SYNC_MARK: &str = "sync.mark";
/// The review-request bucket's mark; [`SYNC_MARK`] is the authored one.
pub const SYNC_MARK_REVIEWS: &str = "sync.mark.reviews";
/// Live updates from the notifications inbox (§6), on by default.
pub const LIVE_ENABLED: &str = "live.enabled";
/// Where the notifications poll stopped, see [`LiveMark`].
pub const LIVE_MARK: &str = "live.mark";
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
/// Slack team IDs in routing priority order (§7.11). Unlisted teams follow.
pub const SLACK_WORKSPACE_ORDER: &str = "slack.workspace_order";
/// Slack team IDs switched off: kept signed in, never used (§7.11).
pub const SLACK_DISABLED_WORKSPACES: &str = "slack.disabled_workspaces";
/// Bumps are thread replies; with this on they are also sent to the channel.
pub const BUMP_BROADCAST: &str = "review.bump_broadcast";
/// Hours after the last request or bump, with no review, before a bump is due.
pub const BUMP_DUE_HOURS: &str = "review.bump_due_hours";
/// When true, Saturday and Sunday UTC do not count toward [`BUMP_DUE_HOURS`].
pub const BUMP_DUE_WORKING_DAYS: &str = "review.bump_due_working_days";
/// Snooze and skip for the bumps-due queue. Never a send.
pub const BUMP_DUE_HOLDS: &str = "review.bump_due_holds";
/// Base URL bare JIRA keys link to, e.g. `https://acme-labs.atlassian.net`. Empty: no links.
pub const JIRA_BASE_URL: &str = "jira.base_url";
/// Master switch for desktop notifications (§8.5). On by default.
pub const NOTIFY_ENABLED: &str = "notify.enabled";
pub const NOTIFY_REVIEW_REQUESTED: &str = "notify.review_requested";
pub const NOTIFY_CI_RED: &str = "notify.ci_red";
pub const NOTIFY_APPROVED: &str = "notify.approved";
pub const NOTIFY_CHANGES_REQUESTED: &str = "notify.changes_requested";
/// New review threads. Off by default: they are the noisy kind.
pub const NOTIFY_COMMENT: &str = "notify.comment";
/// Hours before Review now, the Waiting chip, and a follow-up with no Slack
/// request. Scheduled bumps read this same key.
pub const REVIEW_SLA_HOURS: &str = "review_sla_hours";
/// When true, only Monday–Friday hours count toward [`REVIEW_SLA_HOURS`].
pub const REVIEW_SLA_WORKING_DAYS: &str = "review_sla_working_days";
/// Optional cap on open pull requests in My PRs. Missing or 0 means no cap.
pub const MY_PRS_WIP_LIMIT: &str = "my_prs_wip_limit";

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

/// Where incremental sync picks up (§6). Written only at the end of a pass
/// that succeeded, so a failed or partial pass never moves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncMark {
    /// The tracked scope the mark belongs to (viewer, orgs, personal repos).
    /// A different scope means a full reconcile.
    pub scope: String,
    /// Latest GitHub `updatedAt` seen in that scope.
    pub updated_at: DateTime<Utc>,
    /// When the last full reconcile ended.
    pub full_at: DateTime<Utc>,
    /// When the status of every open pull request was last read.
    pub rollup_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
struct StoredMark {
    scope: String,
    updated_at: String,
    full_at: String,
    rollup_at: String,
}

pub fn sync_mark(store: &Store) -> Result<Option<SyncMark>, CoreError> {
    sync_mark_at(store, SYNC_MARK)
}

/// The mark stored under `key` ([`SYNC_MARK`] or [`SYNC_MARK_REVIEWS`]).
pub fn sync_mark_at(store: &Store, key: &str) -> Result<Option<SyncMark>, CoreError> {
    // A mark that no longer decodes is treated as missing: a full sync.
    let Ok(Some(stored)) = get_json::<StoredMark>(store, key) else {
        return Ok(None);
    };
    let (Ok(updated_at), Ok(full_at), Ok(rollup_at)) = (
        parse_time(&stored.updated_at),
        parse_time(&stored.full_at),
        parse_time(&stored.rollup_at),
    ) else {
        return Ok(None);
    };
    Ok(Some(SyncMark {
        scope: stored.scope,
        updated_at,
        full_at,
        rollup_at,
    }))
}

pub fn set_sync_mark(store: &Store, mark: &SyncMark) -> Result<(), CoreError> {
    set_sync_mark_at(store, SYNC_MARK, mark)
}

pub fn set_sync_mark_at(store: &Store, key: &str, mark: &SyncMark) -> Result<(), CoreError> {
    put_json(
        store,
        key,
        &StoredMark {
            scope: mark.scope.clone(),
            updated_at: mark.updated_at.to_rfc3339(),
            full_at: mark.full_at.to_rfc3339(),
            rollup_at: mark.rollup_at.to_rfc3339(),
        },
    )
}

/// The notifications poll's position: the inbox `Last-Modified` (sent back
/// as `If-Modified-Since`) and the newest thread `updated_at` handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveMark {
    pub last_modified: Option<String>,
    pub watermark: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
struct StoredLiveMark {
    last_modified: Option<String>,
    watermark: String,
}

pub fn live_mark(store: &Store) -> Result<Option<LiveMark>, CoreError> {
    let Ok(Some(stored)) = get_json::<StoredLiveMark>(store, LIVE_MARK) else {
        return Ok(None);
    };
    let Ok(watermark) = parse_time(&stored.watermark) else {
        return Ok(None);
    };
    Ok(Some(LiveMark {
        last_modified: stored.last_modified,
        watermark,
    }))
}

pub fn set_live_mark(store: &Store, mark: &LiveMark) -> Result<(), CoreError> {
    put_json(
        store,
        LIVE_MARK,
        &StoredLiveMark {
            last_modified: mark.last_modified.clone(),
            watermark: mark.watermark.to_rfc3339(),
        },
    )
}

pub fn live_updates(store: &Store) -> Result<bool, CoreError> {
    json_or(store, LIVE_ENABLED, true)
}

pub fn set_live_updates(store: &Store, on: bool) -> Result<(), CoreError> {
    put_json(store, LIVE_ENABLED, &on)
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

/// The review-request templates in effect (§7.6). Absent keys are the
/// script's defaults.
pub fn message_templates(store: &Store) -> Result<MessageTemplates, CoreError> {
    let mut templates = MessageTemplates::default();
    for kind in TemplateKind::ALL {
        if let Some(text) = get_json::<Option<String>>(store, kind.setting_key())?.flatten() {
            templates.set(kind, text);
        }
    }
    Ok(templates)
}

/// Save one template after validating it; `None` resets it to the default.
pub fn set_message_template(
    store: &Store,
    kind: TemplateKind,
    text: Option<&str>,
) -> Result<(), CoreError> {
    match text {
        Some(text) if text != kind.default_text() => {
            validate_template(kind, text)?;
            put_json(store, kind.setting_key(), &Some(text))
        }
        _ => store.delete_setting(kind.setting_key().to_owned()),
    }
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

pub fn slack_workspace_order(store: &Store) -> Result<Vec<String>, CoreError> {
    json_or(store, SLACK_WORKSPACE_ORDER, Vec::new())
}

pub fn set_slack_workspace_order(store: &Store, teams: &[String]) -> Result<(), CoreError> {
    put_json(store, SLACK_WORKSPACE_ORDER, teams)
}

pub fn slack_disabled_workspaces(store: &Store) -> Result<Vec<String>, CoreError> {
    json_or(store, SLACK_DISABLED_WORKSPACES, Vec::new())
}

pub fn set_slack_workspace_enabled(
    store: &Store,
    team_id: &str,
    enabled: bool,
) -> Result<(), CoreError> {
    let mut disabled = slack_disabled_workspaces(store)?;
    disabled.retain(|team| team != team_id);
    if !enabled {
        disabled.push(team_id.to_owned());
    }
    put_json(store, SLACK_DISABLED_WORKSPACES, &disabled)
}

/// `review.bump_broadcast`: whether a bump reply is also sent to the
/// channel (Slack's `reply_broadcast`). Defaults to on.
pub fn bump_broadcast(store: &Store) -> Result<bool, CoreError> {
    json_or(store, BUMP_BROADCAST, true)
}

pub fn set_bump_broadcast(store: &Store, on: bool) -> Result<(), CoreError> {
    put_json(store, BUMP_BROADCAST, &on)
}

pub const DEFAULT_BUMP_DUE_HOURS: i64 = 48;

pub fn bump_due_hours(store: &Store) -> Result<i64, CoreError> {
    json_or(store, BUMP_DUE_HOURS, DEFAULT_BUMP_DUE_HOURS)
}

pub fn bump_due_working_days(store: &Store) -> Result<bool, CoreError> {
    json_or(store, BUMP_DUE_WORKING_DAYS, true)
}

/// The scheduled-bump rule in effect (§7.14).
pub fn bump_due_rule(store: &Store) -> Result<crate::review::due::BumpDueRule, CoreError> {
    Ok(crate::review::due::BumpDueRule {
        after_hours: bump_due_hours(store)?,
        working_days: bump_due_working_days(store)?,
    })
}

/// One snooze: hidden until `until`, and only for the ask it was set on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnoozeHold {
    pub until: String,
    pub asked_at: String,
}

/// Per review-request holds for the bumps-due queue. Keys are request ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BumpHolds {
    #[serde(default)]
    pub snooze: std::collections::BTreeMap<String, SnoozeHold>,
    /// Request id → the ask instant (`rfc3339`) the user skipped.
    #[serde(default)]
    pub skip: std::collections::BTreeMap<String, String>,
}

pub fn bump_holds(store: &Store) -> Result<BumpHolds, CoreError> {
    json_or(store, BUMP_DUE_HOLDS, BumpHolds::default())
}

pub fn set_bump_holds(store: &Store, holds: &BumpHolds) -> Result<(), CoreError> {
    put_json(store, BUMP_DUE_HOLDS, holds)
}

/// `jira.base_url`, without a trailing slash; empty when unset.
pub fn jira_base_url(store: &Store) -> Result<String, CoreError> {
    json_or(store, JIRA_BASE_URL, String::new())
}

/// `review.bulk_bump_max_threads`: above this many distinct roots in one
/// channel, a bulk bump becomes one top-level remind (§7.9). At least 1.
pub fn bulk_bump_max_threads(store: &Store) -> Result<usize, CoreError> {
    Ok(json_or::<usize>(store, BULK_BUMP_MAX_THREADS, 3)?.max(1))
}

/// `review_sla_hours`. Missing or unreadable values use the 48 hour default.
pub fn review_sla_hours(store: &Store) -> Result<i64, CoreError> {
    let hours = json_or(
        store,
        REVIEW_SLA_HOURS,
        crate::sla::DEFAULT_REVIEW_SLA_HOURS,
    )?;
    Ok(if REVIEW_SLA_RANGE.contains(&hours) {
        hours
    } else {
        crate::sla::DEFAULT_REVIEW_SLA_HOURS
    })
}

/// `review_sla_working_days`. Off unless the user turns it on.
pub fn review_sla_working_days(store: &Store) -> Result<bool, CoreError> {
    json_or(store, REVIEW_SLA_WORKING_DAYS, false)
}

/// The list's review wait: hours plus whether weekends count.
pub fn review_sla(store: &Store) -> Result<crate::sla::ReviewSla, CoreError> {
    Ok(crate::sla::ReviewSla {
        hours: review_sla_hours(store)?,
        working_days: review_sla_working_days(store)?,
    })
}

/// `my_prs_wip_limit`. `None` when unset or not a positive cap.
pub fn my_prs_wip_limit(store: &Store) -> Result<Option<u32>, CoreError> {
    Ok(json_or::<Option<u32>>(store, MY_PRS_WIP_LIMIT, None)?.filter(|limit| *limit > 0))
}

/// The user-editable settings behind Settings → Review and Sync (§7.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    pub review_requester: String,
    pub min_bump_interval_hours: i64,
    pub bulk_bump_max_threads: usize,
    pub bump_broadcast: bool,
    /// Hours before a scheduled bump is due. Older clients omit it: 48.
    #[serde(default = "default_bump_due_hours")]
    pub bump_due_hours: i64,
    /// Skip Saturday and Sunday (UTC). Older clients omit it: on.
    #[serde(default = "default_working_days")]
    pub bump_due_working_days: bool,
    pub blocked_review_required_is_ready: bool,
    pub ticket_key_regex: String,
    pub focused_interval_s: u64,
    pub unfocused_interval_s: u64,
    pub keep_closed_days: i64,
    /// Older clients do not send it: keep live updates on.
    #[serde(default = "live_default")]
    pub live_updates: bool,
    /// `jira.base_url`; older clients do not send it.
    #[serde(default)]
    pub jira_base_url: String,
    /// Desktop notifications (§8.5). Older clients omit these; noisy ones stay off.
    #[serde(default = "live_default")]
    pub notify_enabled: bool,
    #[serde(default = "live_default")]
    pub notify_review_requested: bool,
    #[serde(default = "live_default")]
    pub notify_ci_red: bool,
    #[serde(default = "live_default")]
    pub notify_approved: bool,
    #[serde(default = "live_default")]
    pub notify_changes_requested: bool,
    #[serde(default)]
    pub notify_comment: bool,
    /// `review_sla_hours`. Older clients omit it and keep 48 hours.
    #[serde(default = "default_review_sla_hours")]
    pub review_sla_hours: i64,
    /// `review_sla_working_days`. Older clients omit it and count every hour.
    #[serde(default)]
    pub review_sla_working_days: bool,
    /// `my_prs_wip_limit`. `None` means My PRs has no open-PR cap.
    #[serde(default)]
    pub my_prs_wip_limit: Option<u32>,
}

fn default_review_sla_hours() -> i64 {
    crate::sla::DEFAULT_REVIEW_SLA_HOURS
}

fn live_default() -> bool {
    true
}

fn default_bump_due_hours() -> i64 {
    DEFAULT_BUMP_DUE_HOURS
}

fn default_working_days() -> bool {
    true
}

pub const MIN_BUMP_HOURS: std::ops::RangeInclusive<i64> = 1..=168;
/// `review.bump_due_hours`: one hour through two weeks.
pub const DUE_HOURS: std::ops::RangeInclusive<i64> = 1..=336;
pub const BULK_THREADS: std::ops::RangeInclusive<usize> = 1..=20;
pub const FOCUSED_SECS: std::ops::RangeInclusive<u64> = 60..=3600;
pub const UNFOCUSED_SECS: std::ops::RangeInclusive<u64> = 60..=86_400;
pub const KEEP_CLOSED: std::ops::RangeInclusive<i64> = 0..=90;
pub const REVIEW_SLA_RANGE: std::ops::RangeInclusive<i64> = 1..=336;
pub const WIP_LIMIT_RANGE: std::ops::RangeInclusive<u32> = 1..=500;

pub fn preferences(store: &Store) -> Result<Preferences, CoreError> {
    Ok(Preferences {
        review_requester: review_requester(store)?.unwrap_or_default(),
        min_bump_interval_hours: min_bump_interval_hours(store)?,
        bulk_bump_max_threads: bulk_bump_max_threads(store)?,
        bump_broadcast: bump_broadcast(store)?,
        bump_due_hours: bump_due_hours(store)?,
        bump_due_working_days: bump_due_working_days(store)?,
        blocked_review_required_is_ready: blocked_review_required_is_ready(store)?,
        ticket_key_regex: ticket_key_regex(store)?,
        focused_interval_s: focused_interval_s(store)?,
        unfocused_interval_s: unfocused_interval_s(store)?,
        keep_closed_days: keep_closed_days(store)?,
        live_updates: live_updates(store)?,
        jira_base_url: jira_base_url(store)?,
        notify_enabled: json_or(store, NOTIFY_ENABLED, true)?,
        notify_review_requested: json_or(store, NOTIFY_REVIEW_REQUESTED, true)?,
        notify_ci_red: json_or(store, NOTIFY_CI_RED, true)?,
        notify_approved: json_or(store, NOTIFY_APPROVED, true)?,
        notify_changes_requested: json_or(store, NOTIFY_CHANGES_REQUESTED, true)?,
        notify_comment: json_or(store, NOTIFY_COMMENT, false)?,
        review_sla_hours: review_sla_hours(store)?,
        review_sla_working_days: review_sla_working_days(store)?,
        my_prs_wip_limit: my_prs_wip_limit(store)?,
    })
}

/// The notification switches, for the shell that posts them.
pub fn notify_toggles(store: &Store) -> Result<crate::notify::NotifyToggles, CoreError> {
    let prefs = preferences(store)?;
    Ok(crate::notify::NotifyToggles {
        enabled: prefs.notify_enabled,
        review_requested: prefs.notify_review_requested,
        ci_red: prefs.notify_ci_red,
        approved: prefs.notify_approved,
        changes_requested: prefs.notify_changes_requested,
        comment: prefs.notify_comment,
    })
}

/// Validate every field, then write them. Nothing is written when a field
/// is out of range. An empty ticket regex restores the default.
pub fn save_preferences(store: &Store, prefs: &Preferences) -> Result<Preferences, CoreError> {
    fn check<T: PartialOrd + std::fmt::Display>(
        name: &str,
        value: T,
        range: std::ops::RangeInclusive<T>,
    ) -> Result<(), CoreError> {
        if range.contains(&value) {
            Ok(())
        } else {
            Err(CoreError::Validation(format!(
                "{name} must be between {} and {}",
                range.start(),
                range.end()
            )))
        }
    }
    check(
        "Minimum hours between bumps",
        prefs.min_bump_interval_hours,
        MIN_BUMP_HOURS,
    )?;
    check(
        "Hours before a bump is due",
        prefs.bump_due_hours,
        DUE_HOURS,
    )?;
    check(
        "Threads per bulk bump",
        prefs.bulk_bump_max_threads,
        BULK_THREADS,
    )?;
    check(
        "Focused sync interval",
        prefs.focused_interval_s,
        FOCUSED_SECS,
    )?;
    check(
        "Background sync interval",
        prefs.unfocused_interval_s,
        UNFOCUSED_SECS,
    )?;
    check(
        "Days to keep closed PRs",
        prefs.keep_closed_days,
        KEEP_CLOSED,
    )?;
    check("Review now after", prefs.review_sla_hours, REVIEW_SLA_RANGE)?;
    let wip_limit = match prefs.my_prs_wip_limit {
        Some(0) | None => None,
        Some(limit) => {
            check("Open PR limit", limit, WIP_LIMIT_RANGE)?;
            Some(limit)
        }
    };
    let regex = prefs.ticket_key_regex.trim();
    let regex = if regex.is_empty() {
        crate::linking::DEFAULT_TICKET_KEY_REGEX
    } else {
        regex
    };
    crate::linking::validate_ticket_regex(regex)?;
    let jira_base = crate::jira::valid_base_url(&prefs.jira_base_url)?;
    put_json(store, REVIEW_REQUESTER, prefs.review_requester.trim())?;
    put_json(store, MIN_BUMP_INTERVAL, &prefs.min_bump_interval_hours)?;
    put_json(store, BULK_BUMP_MAX_THREADS, &prefs.bulk_bump_max_threads)?;
    put_json(store, BUMP_BROADCAST, &prefs.bump_broadcast)?;
    put_json(store, BUMP_DUE_HOURS, &prefs.bump_due_hours)?;
    put_json(store, BUMP_DUE_WORKING_DAYS, &prefs.bump_due_working_days)?;
    put_json(
        store,
        BLOCKED_IS_READY,
        &prefs.blocked_review_required_is_ready,
    )?;
    put_json(store, TICKET_KEY_REGEX, regex)?;
    put_json(store, FOCUSED_INTERVAL, &prefs.focused_interval_s)?;
    put_json(store, UNFOCUSED_INTERVAL, &prefs.unfocused_interval_s)?;
    put_json(store, KEEP_CLOSED_DAYS, &prefs.keep_closed_days)?;
    put_json(store, LIVE_ENABLED, &prefs.live_updates)?;
    put_json(store, JIRA_BASE_URL, &jira_base)?;
    put_json(store, NOTIFY_ENABLED, &prefs.notify_enabled)?;
    put_json(
        store,
        NOTIFY_REVIEW_REQUESTED,
        &prefs.notify_review_requested,
    )?;
    put_json(store, NOTIFY_CI_RED, &prefs.notify_ci_red)?;
    put_json(store, NOTIFY_APPROVED, &prefs.notify_approved)?;
    put_json(
        store,
        NOTIFY_CHANGES_REQUESTED,
        &prefs.notify_changes_requested,
    )?;
    put_json(store, NOTIFY_COMMENT, &prefs.notify_comment)?;
    put_json(store, REVIEW_SLA_HOURS, &prefs.review_sla_hours)?;
    put_json(
        store,
        REVIEW_SLA_WORKING_DAYS,
        &prefs.review_sla_working_days,
    )?;
    put_json(store, MY_PRS_WIP_LIMIT, &wip_limit)?;
    preferences(store)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("nexus.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn message_templates_save_validate_and_reset() {
        use crate::review::message::{MessageMode, build, build_with};
        let (_dir, store) = store();
        assert_eq!(
            message_templates(&store).unwrap(),
            MessageTemplates::default()
        );
        let url = "https://github.com/acme/widgets/pull/1";
        set_message_template(
            &store,
            TemplateKind::SingleRequest,
            Some("PTAL {title}\n{url}"),
        )
        .unwrap();
        let templates = message_templates(&store).unwrap();
        assert_eq!(
            build_with(
                &templates,
                MessageMode::Request,
                &[("a_b", url)],
                None,
                None
            )
            .unwrap(),
            format!("PTAL `a_b`\n{url}\n")
        );
        let err = set_message_template(&store, TemplateKind::SingleRequest, Some("{oops} {url}"))
            .unwrap_err();
        assert!(err.to_string().contains("unknown placeholder"));
        assert_eq!(message_templates(&store).unwrap(), templates);
        set_message_template(&store, TemplateKind::SingleRequest, None).unwrap();
        assert!(
            store
                .get_setting(TemplateKind::SingleRequest.setting_key().into())
                .unwrap()
                .is_none()
        );
        let defaults = message_templates(&store).unwrap();
        assert_eq!(
            build_with(&defaults, MessageMode::Request, &[("a_b", url)], None, None).unwrap(),
            build(MessageMode::Request, &[("a_b", url)], None, None).unwrap()
        );
    }

    #[test]
    fn preferences_default_to_the_spec_values() {
        let (_dir, store) = store();
        let prefs = preferences(&store).unwrap();
        assert_eq!(prefs.review_requester, "");
        assert_eq!(prefs.min_bump_interval_hours, 24);
        assert_eq!(prefs.bulk_bump_max_threads, 3);
        assert!(prefs.bump_broadcast);
        assert_eq!(prefs.bump_due_hours, 48);
        assert!(prefs.bump_due_working_days);
        assert!(prefs.blocked_review_required_is_ready);
        assert_eq!(
            prefs.ticket_key_regex,
            crate::linking::DEFAULT_TICKET_KEY_REGEX
        );
        assert_eq!(
            (prefs.focused_interval_s, prefs.unfocused_interval_s),
            (180, 900)
        );
        assert_eq!(prefs.keep_closed_days, 7);
        assert!(prefs.notify_enabled);
        assert!(prefs.notify_review_requested);
        assert!(prefs.notify_ci_red);
        assert!(prefs.notify_approved);
        assert!(prefs.notify_changes_requested);
        assert!(!prefs.notify_comment);
        assert_eq!(prefs.review_sla_hours, 48);
        assert!(!prefs.review_sla_working_days);
        assert_eq!(prefs.my_prs_wip_limit, None);
    }

    #[test]
    fn saving_round_trips_and_an_empty_regex_restores_the_default() {
        let (_dir, store) = store();
        let prefs = Preferences {
            review_requester: "  for Ada Acme ".into(),
            min_bump_interval_hours: 4,
            bulk_bump_max_threads: 5,
            bump_broadcast: false,
            bump_due_hours: 72,
            bump_due_working_days: false,
            blocked_review_required_is_ready: false,
            ticket_key_regex: "   ".into(),
            focused_interval_s: 120,
            unfocused_interval_s: 1800,
            keep_closed_days: 14,
            live_updates: false,
            jira_base_url: " https://acme-labs.atlassian.net/ ".into(),
            notify_enabled: false,
            notify_review_requested: false,
            notify_ci_red: true,
            notify_approved: false,
            notify_changes_requested: true,
            notify_comment: true,
            review_sla_hours: 24,
            review_sla_working_days: true,
            my_prs_wip_limit: Some(8),
        };
        let saved = save_preferences(&store, &prefs).unwrap();
        assert_eq!(saved.jira_base_url, "https://acme-labs.atlassian.net");
        assert_eq!(saved.review_requester, "for Ada Acme");
        assert_eq!(saved.review_sla_hours, 24);
        assert!(saved.review_sla_working_days);
        assert_eq!(saved.my_prs_wip_limit, Some(8));
        assert_eq!(review_sla_hours(&store).unwrap(), 24);
        assert_eq!(
            saved.ticket_key_regex,
            crate::linking::DEFAULT_TICKET_KEY_REGEX
        );
        assert!(!bump_broadcast(&store).unwrap());
        assert_eq!(saved.bump_due_hours, 72);
        assert!(!saved.bump_due_working_days);
        assert_eq!(focused_interval_s(&store).unwrap(), 120);
        assert_eq!(saved, preferences(&store).unwrap());
    }

    #[test]
    fn an_invalid_field_writes_nothing() {
        let (_dir, store) = store();
        let mut prefs = preferences(&store).unwrap();
        prefs.bump_broadcast = false;
        prefs.focused_interval_s = 5;
        assert!(save_preferences(&store, &prefs).is_err());
        prefs.focused_interval_s = 180;
        prefs.ticket_key_regex = "([A-Z".into();
        assert!(save_preferences(&store, &prefs).is_err());
        prefs.ticket_key_regex = String::new();
        prefs.jira_base_url = "acme-labs.atlassian.net".into();
        assert!(save_preferences(&store, &prefs).is_err());
        prefs.jira_base_url.clear();
        prefs.review_sla_hours = 0;
        assert!(save_preferences(&store, &prefs).is_err());
        prefs.review_sla_hours = 48;
        prefs.my_prs_wip_limit = Some(9_000);
        assert!(save_preferences(&store, &prefs).is_err());
        assert!(bump_broadcast(&store).unwrap());
        assert_eq!(review_sla_hours(&store).unwrap(), 48);
    }

    #[test]
    fn a_zero_open_pr_limit_clears_the_cap() {
        let (_dir, store) = store();
        let mut prefs = preferences(&store).unwrap();
        prefs.my_prs_wip_limit = Some(0);
        let saved = save_preferences(&store, &prefs).unwrap();
        assert_eq!(saved.my_prs_wip_limit, None);
    }
}
