//! The PR detail panel's at-a-glance facts (§7.2): checks, comments, people,
//! mergeability, the viewer's merge rights, and auto-merge.
//!
//! Fetched when the panel opens, cached per pull request, and refetched on
//! the next open or sync, so the panel draws the last copy right away.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::domain::{PrId, PullRequest};
use crate::error::{CoreError, ProviderError};
use crate::providers::VcsProvider;
use crate::store::Store;

/// How a pull request lands on its base branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

impl MergeMethod {
    /// GitHub's `PullRequestMergeMethod` value.
    pub fn graphql(self) -> &'static str {
        match self {
            Self::Squash => "SQUASH",
            Self::Merge => "MERGE",
            Self::Rebase => "REBASE",
        }
    }

    /// Accepts the app's snake_case names and GitHub's upper-case ones.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "squash" => Some(Self::Squash),
            "merge" => Some(Self::Merge),
            "rebase" => Some(Self::Rebase),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Passed,
    Failed,
    Pending,
    Skipped,
}

/// One check run or commit status on the head commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckItem {
    pub name: String,
    pub state: CheckState,
    pub url: Option<String>,
    /// Branch protection or a ruleset requires it.
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonKind {
    User,
    Team,
}

/// A user or team, as drawn in an avatar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub kind: PersonKind,
    /// `login` for a user, `org/slug` for a team.
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Pending,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewer {
    #[serde(flatten)]
    pub person: Person,
    pub state: ReviewState,
}

/// What GitHub says the signed-in user may do on this pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerAccess {
    pub login: String,
    /// `admin`, `maintain`, `write`, `triage`, or `read`; `None` when hidden.
    pub permission: Option<String>,
    pub can_merge_as_admin: bool,
    pub can_enable_auto_merge: bool,
    pub can_disable_auto_merge: bool,
}

/// Repository merge settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeSettings {
    pub auto_merge_allowed: bool,
    pub squash: bool,
    pub merge: bool,
    pub rebase: bool,
}

/// Auto-merge already turned on for the pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoMergeState {
    pub method: MergeMethod,
    pub enabled_by: Option<String>,
    /// RFC 3339.
    pub enabled_at: Option<String>,
}

/// Everything the detail panel's cards need, in one provider call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrOverview {
    /// `open`, `closed`, or `merged`.
    pub state: String,
    pub is_draft: bool,
    pub base_ref: String,
    pub head_ref: String,
    /// `mergeable`, `conflicting`, or `unknown`.
    pub mergeable: String,
    /// GitHub's `mergeStateStatus`, lower case: `clean`, `blocked`, `behind`,
    /// `dirty`, `unstable`, `has_hooks`, `draft`, or `unknown`.
    pub merge_state: String,
    /// `approved`, `changes_requested`, `review_required`, or none.
    pub review_decision: Option<String>,
    pub issue_comments: u32,
    pub review_comments: u32,
    pub checks: Vec<CheckItem>,
    /// All checks, including any past the first page in `checks`.
    pub checks_total: u32,
    pub approvals: u32,
    /// Approvals the base branch requires, when the token can see its rules.
    pub required_approvals: Option<u32>,
    pub reviewers: Vec<Reviewer>,
    pub participants: Vec<Person>,
    pub participants_total: u32,
    pub viewer: ViewerAccess,
    pub merge_settings: MergeSettings,
    pub auto_merge: Option<AutoMergeState>,
    /// RFC 3339; when the provider answered.
    pub fetched_at: String,
}

/// The cached overview for `pr`, if one was stored and still parses.
pub fn cached(store: &Store, pr: PrId) -> Result<Option<PrOverview>, CoreError> {
    Ok(store
        .get_pr_overview(pr)?
        .and_then(|json| serde_json::from_str(&json).ok()))
}

/// Fetches a fresh overview from the provider and caches it.
pub fn refresh(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
) -> Result<PrOverview, CoreError> {
    let overview = provider.pull_request_overview(pr)?;
    let json = serde_json::to_string(&overview)
        .map_err(|err| CoreError::Store(format!("encode overview: {err}")))?;
    store.upsert_pr_overview(pr.id, json, Utc::now())?;
    Ok(overview)
}

/// Turns auto-merge on with `method`, or off with `None`, then refreshes.
pub fn set_auto_merge(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
    method: Option<MergeMethod>,
) -> Result<PrOverview, CoreError> {
    provider.set_auto_merge(pr, method)?;
    refresh(store, provider, pr)
}

/// Default for providers without an overview.
pub(crate) fn unsupported() -> ProviderError {
    ProviderError::Unsupported("pull request overview".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_method_round_trips_both_spellings() {
        assert_eq!(MergeMethod::parse("SQUASH"), Some(MergeMethod::Squash));
        assert_eq!(MergeMethod::parse("rebase"), Some(MergeMethod::Rebase));
        assert_eq!(MergeMethod::parse("fast-forward"), None);
        assert_eq!(MergeMethod::Merge.graphql(), "MERGE");
    }

    #[test]
    fn reviewer_serializes_flat() {
        let reviewer = Reviewer {
            person: Person {
                kind: PersonKind::Team,
                login: "acme/web".into(),
                name: Some("Web".into()),
                avatar_url: None,
            },
            state: ReviewState::Pending,
        };
        let value = serde_json::to_value(&reviewer).unwrap();
        assert_eq!(value["kind"], "team");
        assert_eq!(value["login"], "acme/web");
        assert_eq!(value["state"], "pending");
    }
}
