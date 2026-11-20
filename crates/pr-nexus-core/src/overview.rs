//! The PR detail panel's at-a-glance facts (§7.2): checks, comments, people,
//! mergeability, the viewer's merge rights, and auto-merge.
//!
//! Fetched when the panel opens, cached per pull request, and refetched on
//! the next open or sync, so the panel draws the last copy right away.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::bots::{self, BotReviews, CommentNote};
use crate::domain::{
    Assignee, ChecksState, MergeStateStatus, Mergeable, PrId, PrState, PullRequest,
    RequestedReviewer, ReviewDecision, TeamRef,
};
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
    /// GitHub's rollup over every check, lower case (`success`, `failure`,
    /// ...). `None` without checks and on overviews cached before it was read.
    #[serde(default)]
    pub checks_state: Option<String>,
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
    /// Merge queue for this pull request's base branch.
    /// `None` on overviews cached before the queue was read.
    #[serde(default)]
    pub merge_queue: Option<crate::merge_queue::MergeQueueSnapshot>,
    /// RFC 3339 `updatedAt`. Empty on overviews cached before it was fetched.
    #[serde(default)]
    pub updated_at: String,
    /// Unresolved review threads. Zero when this overview did not count them.
    #[serde(default)]
    pub unresolved_threads: u32,
    /// Review threads in total.
    #[serde(default)]
    pub total_threads: u32,
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
    if let Some(queue) = &overview.merge_queue {
        store.set_merge_queue(pr.id, queue)?;
    }
    if let Some(mut stored) = store.get_pull_request(pr.id)? {
        let before = stored.clone();
        apply_to_pull_request(&mut stored, &overview);
        if stored != before {
            store.update_pull_request_detail(&stored)?;
        }
        let fetched = chrono::DateTime::parse_from_rfc3339(&overview.fetched_at)
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        store.set_detail_fetched_at(stored.id, fetched)?;
    }
    Ok(overview)
}

/// Copies the fields the list row shows from a live overview onto `pr`.
/// A merged pull request stays merged. Empty checks leave the stored rollup.
pub fn apply_to_pull_request(pr: &mut PullRequest, overview: &PrOverview) {
    if pr.state != PrState::Merged
        && let Ok(state) = overview.state.parse::<PrState>()
    {
        pr.state = state;
    }
    pr.is_draft = overview.is_draft;
    if !overview.head_ref.is_empty() {
        pr.head_ref = overview.head_ref.clone();
    }
    if !overview.base_ref.is_empty() {
        pr.base_ref = overview.base_ref.clone();
    }
    if !overview.head_sha.is_empty() {
        pr.head_sha = overview.head_sha.clone();
    }
    match overview.review_decision.as_deref() {
        None => pr.review_decision = None,
        Some(text) => {
            if let Ok(decision) = text.parse::<ReviewDecision>() {
                pr.review_decision = Some(decision);
            }
        }
    }
    if let Some(checks) = checks_of(overview) {
        pr.checks = checks;
    }
    if let Ok(mergeable) = overview.mergeable.parse::<Mergeable>() {
        pr.mergeable = mergeable;
    }
    if let Ok(merge_state) = overview.merge_state.parse::<MergeStateStatus>() {
        pr.merge_state = merge_state;
    }
    pr.unresolved_threads = overview.unresolved_threads;
    pr.total_threads = overview.total_threads;
    pr.requested_reviewers = pending_of(overview);
    pr.assignees = overview
        .assignees
        .iter()
        .filter(|person| person.kind == PersonKind::User)
        .map(|person| Assignee {
            login: person.login.clone(),
            name: person.name.clone(),
        })
        .collect();
    if let Ok(at) = chrono::DateTime::parse_from_rfc3339(&overview.updated_at) {
        let at = at.with_timezone(&Utc);
        if at > pr.updated_at {
            pr.updated_at = at;
        }
        if at > pr.last_activity_at {
            pr.last_activity_at = at;
        }
    }
}

fn checks_of(overview: &PrOverview) -> Option<ChecksState> {
    if let Some(state) = overview.checks_state.as_deref() {
        return state.parse().ok();
    }
    if overview.checks.is_empty() {
        return None;
    }
    let mut failed = false;
    let mut pending = false;
    let mut passed = false;
    for check in &overview.checks {
        match check.state {
            CheckState::Failed => failed = true,
            CheckState::Pending => pending = true,
            CheckState::Passed => passed = true,
            CheckState::Skipped => {}
        }
    }
    if failed {
        Some(ChecksState::Failure)
    } else if overview.checks_total as usize > overview.checks.len() {
        // The unread checks may fail; keep the stored rollup.
        None
    } else if pending {
        Some(ChecksState::Pending)
    } else if passed {
        Some(ChecksState::Success)
    } else {
        None
    }
}

fn pending_of(overview: &PrOverview) -> Vec<RequestedReviewer> {
    overview
        .reviewers
        .iter()
        .filter(|seat| seat.state == ReviewState::Pending)
        .map(|seat| {
            let reviewer = match seat.person.kind {
                PersonKind::Team => {
                    let (org, slug) = seat
                        .person
                        .login
                        .split_once('/')
                        .unwrap_or((seat.person.login.as_str(), ""));
                    crate::domain::Reviewer::Team(TeamRef {
                        org: org.to_owned(),
                        slug: slug.to_owned(),
                    })
                }
                PersonKind::User | PersonKind::Bot => crate::domain::Reviewer::User {
                    login: seat.person.login.clone(),
                },
            };
            RequestedReviewer {
                reviewer,
                name: seat.person.name.clone(),
                avatar_url: seat.person.avatar_url.clone(),
            }
        })
        .collect()
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
            assignees: Vec::new(),
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
    fn checks_past_the_first_page_do_not_turn_a_failing_rollup_green() {
        let checks: Vec<String> = (0..100)
            .map(|i| format!(r#"{{"name":"c{i}","state":"passed","url":null,"required":false}}"#))
            .collect();
        let json = format!(
            r#"{{"state":"open","is_draft":false,"base_ref":"main","head_ref":"h","mergeable":"mergeable",
            "merge_state":"unstable","review_decision":"approved","issue_comments":0,"review_comments":0,
            "checks":[{}],"checks_total":150,"approvals":1,"required_approvals":null,"reviewers":[],
            "author":null,"assignees":[],"commenters":[],
            "viewer":{{"login":"a","permission":null,"can_merge_as_admin":false,
            "can_enable_auto_merge":false,"can_disable_auto_merge":false}},
            "merge_settings":{{"auto_merge_allowed":false,"squash":true,"merge":true,"rebase":true}},
            "auto_merge":null,"fetched_at":"2026-11-10T12:00:00Z"}}"#,
            checks.join(",")
        );
        // A cached overview without the rollup state: 50 checks were not
        // read, so the stored state must be kept.
        let mut overview: PrOverview = serde_json::from_str(&json).unwrap();
        assert_eq!(checks_of(&overview), None);

        overview.checks_state = Some("failure".into());
        assert_eq!(checks_of(&overview), Some(ChecksState::Failure));
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

    #[test]
    fn a_live_overview_updates_the_stored_list_row_and_bumps_revision() {
        use crate::domain::Reviewer as DomainReviewer;
        use crate::domain::{
            Account, AccountId, AccountKind, AuthMode, ChecksState, Involvement, MergeStateStatus,
            Mergeable, PrId, ProviderRef, RepoRef, Repository, ReviewDecision, TeamRef,
        };
        use crate::error::ProviderError;
        use crate::providers::VcsProvider;
        use crate::store::Store;
        use chrono::TimeZone;

        let at = Utc.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("detail.db")).unwrap();
        let account = store
            .insert_account(Account {
                id: AccountId(0),
                kind: AccountKind::GitHub,
                display_name: "ada".into(),
                host: "github.com".into(),
                external_id: "U_ada".into(),
                slack_workspace: None,
                login: "ada".into(),
                auth_mode: AuthMode::GhCli,
                keychain_key: None,
                created_at: at,
            })
            .unwrap();
        let repo = store
            .insert_repository(Repository {
                id: 0,
                account_id: account.id,
                owner: "acme".into(),
                name: "widgets".into(),
                node_id: None,
                default_branch: Some("main".into()),
                archived: false,
            })
            .unwrap();
        let saved = store
            .upsert_pull_request(
                repo.id,
                PullRequest {
                    id: PrId(0),
                    remote: ProviderRef {
                        account: account.id,
                        id: "PR_acme_7".into(),
                    },
                    repo: RepoRef {
                        host: "github.com".into(),
                        owner: "acme".into(),
                        name: "widgets".into(),
                    },
                    number: 7,
                    title: "Add address autocomplete".into(),
                    url: "https://github.com/acme/widgets/pull/7".into(),
                    author_login: "ada".into(),
                    body: String::new(),
                    state: PrState::Open,
                    is_draft: true,
                    head_ref: "feature/479".into(),
                    base_ref: "main".into(),
                    head_sha: "old".into(),
                    labels: Vec::new(),
                    review_decision: Some(ReviewDecision::ReviewRequired),
                    checks: ChecksState::Pending,
                    mergeable: Mergeable::Conflicting,
                    merge_state: MergeStateStatus::Dirty,
                    requested_reviewers: vec![RequestedReviewer::bare(DomainReviewer::User {
                        login: "sam".into(),
                    })],
                    assignees: Vec::new(),
                    unresolved_threads: 3,
                    total_threads: 4,
                    involvement: Involvement::AUTHOR,
                    created_at: at,
                    updated_at: at,
                    last_activity_at: at,
                    synced_at: at,
                    hidden: false,
                },
            )
            .unwrap();
        let before = store.revision();
        let overview = PrOverview {
            state: "open".into(),
            is_draft: false,
            base_ref: "main".into(),
            head_ref: "feature/479".into(),
            head_sha: "new".into(),
            base_sha: "base".into(),
            head_owner: "acme".into(),
            mergeable: "mergeable".into(),
            merge_state: "clean".into(),
            review_decision: Some("approved".into()),
            issue_comments: 1,
            review_comments: 1,
            checks: vec![CheckItem {
                name: "build".into(),
                state: CheckState::Passed,
                url: None,
                required: true,
            }],
            checks_total: 1,
            checks_state: Some("success".into()),
            approvals: 1,
            required_approvals: Some(1),
            reviewers: vec![Reviewer {
                person: Person {
                    kind: PersonKind::User,
                    login: "kim".into(),
                    name: Some("Kim Lee".into()),
                    avatar_url: None,
                },
                state: ReviewState::Approved,
            }],
            author: None,
            assignees: vec![Person {
                kind: PersonKind::User,
                login: "ada".into(),
                name: None,
                avatar_url: None,
            }],
            commenters: Vec::new(),
            feedback: Vec::new(),
            bot_reviews: crate::bots::BotReviews::default(),
            viewer: ViewerAccess {
                login: "ada".into(),
                permission: Some("write".into()),
                can_merge_as_admin: false,
                can_enable_auto_merge: true,
                can_disable_auto_merge: false,
                can_close: Some(true),
            },
            merge_settings: MergeSettings {
                auto_merge_allowed: true,
                squash: true,
                merge: true,
                rebase: true,
            },
            auto_merge: None,
            merge_queue: None,
            updated_at: "2026-10-26T16:00:00Z".into(),
            unresolved_threads: 0,
            total_threads: 1,
            fetched_at: "2026-10-26T18:00:00Z".into(),
        };
        struct Only(PrOverview);
        impl VcsProvider for Only {
            fn kind(&self) -> crate::domain::VcsKind {
                crate::domain::VcsKind::GitHub
            }
            fn viewer(&self) -> Result<crate::providers::VcsUser, ProviderError> {
                Err(unsupported())
            }
            fn search_pull_requests(
                &self,
                _: &crate::providers::PrQuery,
            ) -> Result<Vec<crate::providers::SyncedPullRequest>, ProviderError> {
                Err(unsupported())
            }
            fn refresh_pull_requests(
                &self,
                _: &[ProviderRef],
            ) -> Result<Vec<crate::providers::SyncedPullRequest>, ProviderError> {
                Err(unsupported())
            }
            fn pull_request_status(
                &self,
                _: &[ProviderRef],
            ) -> Result<Vec<crate::providers::PrStatus>, ProviderError> {
                Err(unsupported())
            }
            fn ownership_probe(
                &self,
                _: &PullRequest,
            ) -> Result<crate::providers::OwnershipProbe, ProviderError> {
                Err(unsupported())
            }
            fn ownership_inputs(
                &self,
                _: &PullRequest,
                _: &crate::providers::OwnershipProbe,
            ) -> Result<crate::providers::OwnershipInputs, ProviderError> {
                Err(unsupported())
            }
            fn team_members(
                &self,
                _: &TeamRef,
            ) -> Result<crate::providers::TeamMembers, ProviderError> {
                Err(unsupported())
            }
            fn rate_limit(&self) -> Result<crate::providers::RateBudget, ProviderError> {
                Err(unsupported())
            }
            fn request_team_review(
                &self,
                _: &PullRequest,
                _: &[TeamRef],
            ) -> Result<(), ProviderError> {
                Err(unsupported())
            }
            fn pull_request_overview(&self, _: &PullRequest) -> Result<PrOverview, ProviderError> {
                Ok(self.0.clone())
            }
        }
        refresh(&store, &Only(overview), &saved).unwrap();
        let row = store.get_pull_request(saved.id).unwrap().unwrap();
        assert!(store.revision() > before);
        assert!(!row.is_draft);
        assert_eq!(row.review_decision, Some(ReviewDecision::Approved));
        assert_eq!(row.checks, ChecksState::Success);
        assert_eq!(row.mergeable, Mergeable::Mergeable);
        assert_eq!(row.merge_state, MergeStateStatus::Clean);
        assert_eq!(row.head_sha, "new");
        assert!(row.requested_reviewers.is_empty());
        assert_eq!(row.unresolved_threads, 0);
        assert_eq!(row.total_threads, 1);
        assert_eq!(row.assignees.len(), 1);
        assert_eq!(row.assignees[0].login, "ada");
        assert!(row.updated_at > at);
        assert!(store.detail_fetched_at(saved.id).unwrap().is_some());
    }
}
