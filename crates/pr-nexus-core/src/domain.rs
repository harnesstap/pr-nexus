//! Product types shared by the store, the registry, and later review logic.
//!
//! Enums that are persisted use lowercase snake_case via [`Display`] and [`FromStr`].
//! An unknown database value fails instead of being coerced.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};

use crate::error::CoreError;

/// Version control providers. Only GitHub is available in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VcsKind {
    GitHub,
    GitLab,
    Gitea,
}

/// Chat providers. Only Slack is available in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatKind {
    Slack,
    Discord,
    Mattermost,
}

/// Row id in `accounts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountId(pub i64);

/// Local surrogate key for a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrId(pub i64);

/// Local surrogate key for a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectId(pub i64);

/// Local surrogate key for a review request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReviewRequestId(pub i64);

/// `host` is `github.com` for GitHub.com.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRef {
    pub host: String,
    pub owner: String,
    pub name: String,
}

/// GitHub team. `org` is kept so CODEOWNERS can name another org's team.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TeamRef {
    pub org: String,
    pub slug: String,
}

/// Opaque provider handle. Core never parses `id`.
///
/// GitHub uses the GraphQL node id. A Slack message id is `C123:1700000000.000100`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderRef {
    pub account: AccountId,
    pub id: String,
}

macro_rules! str_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($variant:ident => $text:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $($variant),+
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = CoreError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($text => Ok(Self::$variant),)+
                    _ => Err(CoreError::Validation(format!(
                        "unknown {}: {value}",
                        stringify!($name)
                    ))),
                }
            }
        }
    };
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum AccountKind {
        GitHub => "github",
        Slack => "slack",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum AuthMode {
        GhCli => "gh_cli",
        Pat => "pat",
        OauthDevice => "oauth_device",
        SlackMcpOauth => "slack_mcp_oauth",
        SlackUserToken => "slack_user_token",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum PrState {
        Open => "open",
        Closed => "closed",
        Merged => "merged",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ReviewDecision {
        Approved => "approved",
        ChangesRequested => "changes_requested",
        ReviewRequired => "review_required",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ChecksState {
        Success => "success",
        Failure => "failure",
        Pending => "pending",
        Error => "error",
        Expected => "expected",
        None => "none",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Mergeable {
        Mergeable => "mergeable",
        Conflicting => "conflicting",
        Unknown => "unknown",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum MergeStateStatus {
        Clean => "clean",
        Behind => "behind",
        Blocked => "blocked",
        Dirty => "dirty",
        Draft => "draft",
        HasHooks => "has_hooks",
        Unstable => "unstable",
        Unknown => "unknown",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum RuleKind {
        TicketKeyInBranch => "ticket_key_in_branch",
        BranchRegex => "branch_regex",
        Label => "label",
        Repo => "repo",
        TitleRegex => "title_regex",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum RequestSource {
        App => "app",
        DiscoveredViaSearch => "discovered_via_search",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum CloseReason {
        PrMerged => "pr_merged",
        PrClosed => "pr_closed",
        Dismissed => "dismissed",
        Superseded => "superseded",
        ChannelArchived => "channel_archived",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum MappingLayer {
        Imported => "imported",
        User => "user",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ReviewMessageKind {
        Request => "request",
        Bump => "bump",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ReviewMessageOrigin {
        Posted => "posted",
        Imported => "imported",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum SendOutcome {
        Sent => "sent",
        Skipped => "skipped",
        Failed => "failed",
    }
}

str_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum DiscoveryOutcome {
        Found => "found",
        None => "none",
        SkippedNoScope => "skipped_no_scope",
        Error => "error",
    }
}

/// Active requests have no close reason. Closed requests always have one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestStatus {
    Active,
    Closed(CloseReason),
}

bitflags::bitflags! {
    /// How the signed-in user relates to a pull request.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Involvement: u8 {
        const AUTHOR = 1;
        const REVIEW_REQUESTED = 2;
    }
}

/// A requested reviewer. Informational only: never used as an owner.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Reviewer {
    User { login: String },
    Team(TeamRef),
}

/// Maximum stored pull-request body, in bytes. Search reads this text.
pub const PR_BODY_MAX_BYTES: usize = 8 * 1024;

/// One open or recently closed pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub id: PrId,
    pub remote: ProviderRef,
    pub repo: RepoRef,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author_login: String,
    pub body: String,
    pub state: PrState,
    pub is_draft: bool,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    pub labels: Vec<String>,
    pub review_decision: Option<ReviewDecision>,
    pub checks: ChecksState,
    pub mergeable: Mergeable,
    pub merge_state: MergeStateStatus,
    pub requested_reviewers: Vec<Reviewer>,
    pub unresolved_threads: u32,
    pub total_threads: u32,
    pub involvement: Involvement,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub synced_at: DateTime<Utc>,
    /// Closed pull requests become hidden after `sync.keep_closed_days`.
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub kind: AccountKind,
    pub display_name: String,
    pub host: String,
    pub external_id: String,
    pub slack_workspace: Option<String>,
    pub login: String,
    pub auth_mode: AuthMode,
    pub keychain_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub id: i64,
    pub account_id: AccountId,
    pub owner: String,
    pub name: String,
    pub node_id: Option<String>,
    pub default_branch: Option<String>,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub description: String,
    pub color: String,
    pub archived: bool,
    pub sort_order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRule {
    pub id: i64,
    pub project: ProjectId,
    pub kind: RuleKind,
    pub pattern: String,
    pub priority: i32,
}

/// Why a pull request is linked to a project.
///
/// `Excluded` means the user removed an auto-link. Rules must not re-add it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSource {
    Manual,
    Rule(i64),
    Excluded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrProjectLink {
    pub pr: PrId,
    pub project: ProjectId,
    pub source: LinkSource,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamChannelMapping {
    pub id: i64,
    pub layer: MappingLayer,
    pub github_org: String,
    pub team_slug: String,
    pub team_name: String,
    pub slack_workspace: String,
    pub slack_channel_id: String,
    pub slack_channel_name: String,
    pub disabled: bool,
    pub note: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackChannel {
    pub slack_workspace: String,
    pub channel_id: String,
    pub name: String,
    pub is_private: bool,
    pub is_archived: bool,
    pub is_member: bool,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipCacheEntry {
    pub pr_id: PrId,
    pub head_sha: String,
    pub base_ref: String,
    pub codeowners_oid: Option<String>,
    pub decision_json: String,
    pub computed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMembersCacheEntry {
    pub github_org: String,
    pub team_slug: String,
    pub members_json: String,
    pub complete: bool,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewMessage {
    pub id: i64,
    pub slack_account_id: AccountId,
    pub channel_id: String,
    pub channel_name: String,
    pub ts: String,
    pub thread_root_ts: Option<String>,
    pub kind: ReviewMessageKind,
    pub origin: ReviewMessageOrigin,
    pub permalink: Option<String>,
    pub text: String,
    pub batch_id: Option<String>,
    pub posted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewMessagePr {
    pub message_id: i64,
    pub pr_id: PrId,
    pub head_sha: Option<String>,
    pub teams_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRequest {
    pub id: ReviewRequestId,
    pub pr: PrId,
    /// Slack account that owns `root`, when the workspace is connected.
    pub root: ProviderRef,
    pub workspace: String,
    pub channel_id: String,
    pub channel_name: String,
    /// Slack `ts` of the top-level request message. Bumps reply in its thread.
    pub root_ts: String,
    pub permalink: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub last_bumped_at: Option<DateTime<Utc>>,
    pub bump_count: u32,
    pub status: RequestStatus,
    pub closed_at: Option<DateTime<Utc>>,
    pub source: RequestSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryRun {
    pub pr_id: PrId,
    pub slack_workspace: String,
    pub ran_at: DateTime<Utc>,
    pub outcome: DiscoveryOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendAttempt {
    pub id: i64,
    pub batch_id: String,
    pub pr_id: Option<PrId>,
    pub channel_id: Option<String>,
    pub origin: String,
    pub outcome: SendOutcome,
    pub detail: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatThreadCacheEntry {
    pub slack_account_id: AccountId,
    pub channel_id: String,
    pub thread_root_ts: String,
    pub messages_json: String,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStateEntry {
    pub key: String,
    pub value_json: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingEntry {
    pub key: String,
    pub value_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewState {
    pub view_key: String,
    pub state_json: String,
    pub updated_at: DateTime<Utc>,
}

impl fmt::Display for VcsKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::Gitea => "Gitea",
        })
    }
}

impl fmt::Display for ChatKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Slack => "Slack",
            Self::Discord => "Discord",
            Self::Mattermost => "Mattermost",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_enum_text_fails() {
        assert!(PrState::from_str("opened").is_err());
        assert!(AuthMode::from_str("oauth").is_err());
        assert_eq!(PrState::from_str("merged").unwrap(), PrState::Merged);
        assert_eq!(CloseReason::ChannelArchived.as_str(), "channel_archived");
    }
}
