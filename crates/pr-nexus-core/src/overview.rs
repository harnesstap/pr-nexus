//! The PR detail panel's at-a-glance facts (§7.2): checks, comments, people,
//! mergeability, the viewer's merge rights, and auto-merge.
//!
//! Fetched when the panel opens, cached per pull request, and refetched on
//! the next open or sync, so the panel draws the last copy right away.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::bots::{self, BotReviews, CommentNote};
use crate::domain::{PrId, PrState, PullRequest};
use crate::error::{CoreError, ProviderError};
use crate::providers::{PrStatus, VcsProvider};
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
    /// A GitHub App or other bot account; kept, but marked.
    Bot,
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
    /// Whether this user can close the pull request. `None` on overviews
    /// stored before the app asked GitHub.
    #[serde(default)]
    pub can_close: Option<bool>,
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
    /// Head commit; empty in overviews cached before it was fetched.
    #[serde(default)]
    pub head_sha: String,
    /// Base branch tip.
    #[serde(default)]
    pub base_sha: String,
    /// Owner of the head repository (differs from the base for a fork).
    #[serde(default)]
    pub head_owner: String,
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
    /// Who opened the pull request; `None` for a deleted account.
    pub author: Option<Person>,
    pub assignees: Vec<Person>,
    /// Authors of PR and review-thread comments, deduped, bots included.
    /// [`for_viewer`] drops the bots before the panel draws this list.
    pub commenters: Vec<Person>,
    /// Comments and reviews fetched with this overview, bots and people.
    /// The panel does not draw this list; [`for_viewer`] turns it into
    /// [`Self::bot_reviews`] using the extra bot logins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feedback: Vec<CommentNote>,
    /// Filled by [`for_viewer`]. Empty in the cache, which keeps [`Self::feedback`].
    #[serde(default, skip_serializing_if = "BotReviews::is_empty")]
    pub bot_reviews: BotReviews,
    pub viewer: ViewerAccess,
    pub merge_settings: MergeSettings,
    pub auto_merge: Option<AutoMergeState>,
    /// RFC 3339; when the provider answered.
    pub fetched_at: String,
}

/// Human comment counts, bot reviews, and commenters with bots removed.
///
/// `extra` is the Settings list of additional bot logins. The stored
/// overview keeps GitHub's totals and [`PrOverview::feedback`]; this copy
/// is what the panel shows and is not written back.
pub fn for_viewer(mut overview: PrOverview, extra: &[String]) -> PrOverview {
    let split = bots::split(
        overview.issue_comments,
        overview.review_comments,
        &overview.feedback,
        extra,
    );
    overview.issue_comments = split.human_issue;
    overview.review_comments = split.human_review;
    overview.bot_reviews = split.bots;
    overview.feedback.clear();
    overview
        .commenters
        .retain(|person| !bots::is_bot(&person.login, person.kind == PersonKind::Bot, extra));
    overview
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

/// Converts the pull request to a draft (`true`) or marks it ready for
/// review (`false`), records the flag, then refreshes the overview.
///
/// Marking ready does not decide that CI passed. When GitHub returns a
/// status, those checks and merge fields replace the stored ones; a failed
/// status read keeps the previous checks and still records the draft flag.
pub fn set_draft(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
    draft: bool,
) -> Result<PrOverview, CoreError> {
    provider.set_draft(pr, draft)?;
    let mut current = pr.clone();
    // The draft change already happened. A failed status read must not
    // turn it into an error or leave the stored flag behind.
    if let Ok(statuses) = provider.pull_request_status(std::slice::from_ref(&pr.remote)) {
        apply_ready_status(&mut current, draft, &statuses);
    } else {
        current.is_draft = draft;
    }
    store.update_pull_request_gate(&current)?;
    refresh(store, provider, &current)
}

/// Writes `draft` onto `pr`, then the matching status when GitHub returned one.
fn apply_ready_status(pr: &mut PullRequest, draft: bool, statuses: &[PrStatus]) {
    pr.is_draft = draft;
    if let Some(status) = statuses
        .iter()
        .find(|status| status.remote_id == pr.remote.id)
    {
        status.apply_to(pr);
        pr.is_draft = draft;
    }
}

/// Merges the pull request now with `method`, then refreshes the overview.
///
/// Once GitHub confirms the merge, the stored row is marked merged right
/// away so it leaves the open list without waiting for the next sync, and
/// its active review requests close. A merged PR never reopens, so the
/// store keeps that state even if a lagging search still reports it open.
/// The refresh after it is best effort: the merge already happened, so a
/// failed or stale read must not turn into a "could not merge" error.
pub fn merge(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
    method: MergeMethod,
) -> Result<PrOverview, CoreError> {
    provider.merge_pull_request(pr, method)?;
    store.set_pull_request_state(pr.id, PrState::Merged)?;
    let merged = PullRequest {
        state: PrState::Merged,
        ..pr.clone()
    };
    crate::sync::close_requests_of_finished(store, &merged, Utc::now())?;
    let mut overview = match refresh(store, provider, pr) {
        Ok(overview) => overview,
        Err(err) => cached(store, pr.id)?.ok_or(err)?,
    };
    if overview.state != PrState::Merged.as_str() {
        overview.state = PrState::Merged.as_str().to_owned();
        overview.auto_merge = None;
        if let Ok(json) = serde_json::to_string(&overview) {
            store.upsert_pr_overview(pr.id, json, Utc::now())?;
        }
    }
    Ok(overview)
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
    fn marking_ready_keeps_a_failing_check_and_takes_a_passing_one() {
        use crate::domain::{
            AccountId, ChecksState, Involvement, MergeStateStatus, Mergeable, PrId, ProviderRef,
            RepoRef, ReviewDecision,
        };

        let now = Utc::now();
        let mut pr = PullRequest {
            id: PrId(7),
            remote: ProviderRef {
                account: AccountId(1),
                id: "PR_acme_7".into(),
            },
            repo: RepoRef {
                host: "github.com".into(),
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 7,
            title: "Fix the latch".into(),
            url: "https://github.com/acme/widgets/pull/7".into(),
            author_login: "ada".into(),
            body: String::new(),
            state: PrState::Open,
            is_draft: true,
            head_ref: "fix-latch".into(),
            base_ref: "main".into(),
            head_sha: "abc7".into(),
            labels: Vec::new(),
            review_decision: None,
            checks: ChecksState::Failure,
            mergeable: Mergeable::Mergeable,
            merge_state: MergeStateStatus::Blocked,
            requested_reviewers: Vec::new(),
            unresolved_threads: 0,
            total_threads: 0,
            involvement: Involvement::AUTHOR,
            created_at: now,
            updated_at: now,
            last_activity_at: now,
            synced_at: now,
            hidden: false,
        };
        let mut still_red = PrStatus::of(&pr);
        still_red.checks = ChecksState::Failure;
        apply_ready_status(&mut pr, false, &[still_red]);
        assert!(!pr.is_draft);
        assert_eq!(pr.checks, ChecksState::Failure);

        let mut green = PrStatus::of(&pr);
        green.checks = ChecksState::Success;
        green.review_decision = Some(ReviewDecision::Approved);
        green.merge_state = MergeStateStatus::Clean;
        apply_ready_status(&mut pr, false, std::slice::from_ref(&green));
        assert!(!pr.is_draft);
        assert_eq!(pr.checks, ChecksState::Success);
        assert_eq!(pr.review_decision, Some(ReviewDecision::Approved));
        assert_eq!(pr.merge_state, MergeStateStatus::Clean);

        let other = PrStatus {
            remote_id: "PR_other".into(),
            checks: ChecksState::Error,
            ..PrStatus::of(&pr)
        };
        apply_ready_status(&mut pr, false, &[other]);
        assert_eq!(pr.checks, ChecksState::Success);
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
