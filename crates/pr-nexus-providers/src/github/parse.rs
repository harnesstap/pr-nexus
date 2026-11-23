//! GraphQL JSON to domain pull requests.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use pr_nexus_core::checks::{FailingCheck, FailingConclusion};
use pr_nexus_core::domain::{
    AccountId, Assignee, ChecksState, Involvement, MergeStateStatus, Mergeable, PrId, PrState,
    ProviderRef, PullRequest, RepoRef, RequestedReviewer, ReviewDecision, Reviewer, TeamRef,
};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{PrStatus, RateBudget, SyncedPullRequest, VcsUser};
use pr_nexus_core::review_clock::{
    RequestTarget, ReviewClock, ReviewRequestEvent, SubmittedReview,
};
use serde_json::Value;

pub(crate) struct SearchPage {
    pub(crate) items: Vec<SyncedPullRequest>,
    pub(crate) has_next: bool,
    pub(crate) cursor: Option<String>,
    /// GitHub's count of every match, not just the ones it lets a search page through.
    pub(crate) total: u64,
}

pub(crate) fn parse_search(
    body: &Value,
    account: AccountId,
    involvement: Involvement,
) -> Result<SearchPage, ProviderError> {
    let search = data(body)?.get("search").cloned().unwrap_or(Value::Null);
    let page = search.get("pageInfo").cloned().unwrap_or(Value::Null);
    let nodes = search.get("nodes").and_then(Value::as_array);
    let mut items = Vec::new();
    if let Some(nodes) = nodes {
        for node in nodes {
            if node.is_null() {
                continue;
            }
            if node.get("id").and_then(Value::as_str).is_none() {
                continue;
            }
            items.push(parse_pr(node, account, involvement)?);
        }
    }
    Ok(SearchPage {
        items,
        has_next: page
            .get("hasNextPage")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        cursor: page
            .get("endCursor")
            .and_then(Value::as_str)
            .map(str::to_owned),
        total: search
            .get("issueCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

pub(crate) fn parse_nodes(
    body: &Value,
    account: AccountId,
) -> Result<Vec<SyncedPullRequest>, ProviderError> {
    let nodes = data(body)?
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut items = Vec::new();
    for node in nodes {
        if node.is_null() || node.get("id").and_then(Value::as_str).is_none() {
            continue;
        }
        items.push(parse_pr(&node, account, Involvement::empty())?);
    }
    Ok(items)
}

pub(crate) fn parse_viewer(body: &Value) -> Result<VcsUser, ProviderError> {
    let viewer = data(body)?
        .get("viewer")
        .cloned()
        .ok_or_else(|| ProviderError::Decode("viewer missing".into()))?;
    let login = text(&viewer, "login")?;
    let id = text(&viewer, "id")?;
    let mut organizations = Vec::new();
    if let Some(nodes) = viewer
        .pointer("/organizations/nodes")
        .and_then(Value::as_array)
    {
        for node in nodes {
            if let Some(login) = node.get("login").and_then(Value::as_str) {
                organizations.push(login.to_owned());
            }
        }
    }
    Ok(VcsUser {
        id,
        login,
        organizations,
        organizations_needing_sso: Vec::new(),
    })
}

pub(crate) fn parse_budget(body: &Value) -> Option<RateBudget> {
    let rate = body.pointer("/data/rateLimit")?;
    Some(RateBudget {
        limit: rate.get("limit").and_then(Value::as_u64).unwrap_or(0) as u32,
        remaining: rate.get("remaining").and_then(Value::as_u64).unwrap_or(0) as u32,
        reset_at: rate
            .get("resetAt")
            .and_then(Value::as_str)
            .and_then(parse_time)
            .unwrap_or_else(Utc::now),
        cost: rate.get("cost").and_then(Value::as_u64).unwrap_or(1) as u32,
        secondary_until: None,
    })
}

/// `reset_at` is when a RATE_LIMITED answer may be retried.
pub(crate) fn graphql_failure(body: &Value, reset_at: DateTime<Utc>) -> Option<ProviderError> {
    let errors = body.get("errors")?.as_array()?;
    if errors.is_empty() || only_missing_nodes(body, errors) {
        return None;
    }
    let message = errors
        .iter()
        .filter_map(|error| error.get("message").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("; ");
    let rate = errors.iter().any(|error| {
        error
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("RATE_LIMITED"))
            || error
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|text| text.to_ascii_lowercase().contains("rate limit"))
    });
    if rate {
        return Some(ProviderError::RateLimited {
            reset_at,
            secondary: false,
        });
    }
    Some(ProviderError::Api {
        code: "graphql".into(),
        message,
    })
}

/// A `nodes(ids:)` answer where some ids are deleted, transferred, or behind
/// SAML: those entries are null with a NOT_FOUND or FORBIDDEN error at
/// `["nodes", i, ..]`, and the rest of the data is good. Mutation errors sit
/// at the mutation's own path, so they still fail.
fn only_missing_nodes(body: &Value, errors: &[Value]) -> bool {
    body.get("data").is_some_and(|data| !data.is_null())
        && errors.iter().all(|error| {
            let kind = error.get("type").and_then(Value::as_str).unwrap_or("");
            let path = error.get("path").and_then(Value::as_array);
            matches!(kind, "NOT_FOUND" | "FORBIDDEN")
                && path
                    .is_some_and(|path| path.len() >= 2 && path[0] == "nodes" && path[1].is_u64())
        })
}

fn parse_pr(
    node: &Value,
    account: AccountId,
    involvement: Involvement,
) -> Result<SyncedPullRequest, ProviderError> {
    let owner = node
        .pointer("/repository/owner/login")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let name = node
        .pointer("/repository/name")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let updated_at = time_field(node, "updatedAt")?;
    let created_at = time_field(node, "createdAt")?;
    let status = parse_status_node(node)?;
    let labels = node
        .pointer("/labels/nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|label| label.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let number = node.get("number").and_then(Value::as_u64).unwrap_or(0);
    let mut synced = SyncedPullRequest {
        pull_request: PullRequest {
            id: PrId(0),
            remote: ProviderRef {
                account,
                id: text(node, "id")?,
            },
            repo: RepoRef {
                host: "github.com".into(),
                owner,
                name,
            },
            number,
            title: text(node, "title").unwrap_or_else(|_| "Untitled".into()),
            url: text(node, "url").unwrap_or_default(),
            author_login: node
                .pointer("/author/login")
                .and_then(Value::as_str)
                .unwrap_or("ghost")
                .to_owned(),
            body: node
                .get("bodyText")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            state: pr_state(node.get("state").and_then(Value::as_str).unwrap_or("OPEN"))?,
            is_draft: node
                .get("isDraft")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            head_ref: text(node, "headRefName").unwrap_or_default(),
            base_ref: text(node, "baseRefName").unwrap_or_default(),
            head_sha: text(node, "headRefOid").unwrap_or_default(),
            labels,
            review_decision: None,
            checks: ChecksState::None,
            mergeable: Mergeable::Unknown,
            merge_state: MergeStateStatus::Unknown,
            requested_reviewers: reviewers(node),
            assignees: assignees(node),
            unresolved_threads: 0,
            total_threads: 0,
            involvement,
            created_at,
            updated_at,
            last_activity_at: updated_at,
            synced_at: updated_at,
            hidden: false,
        },
        repo_node_id: node
            .pointer("/repository/id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        default_branch: node
            .pointer("/repository/defaultBranchRef/name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        repo_archived: node
            .pointer("/repository/isArchived")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        merge_queue: status.merge_queue.clone(),
        failing_checks: status.failing_checks.clone(),
        review_clock: status.review_clock.clone(),
    };
    status.apply_to(&mut synced.pull_request);
    Ok(synced)
}

/// Status fields from a `nodes` answer to status.graphql. Null nodes (a pull
/// request that is gone or no longer visible) are skipped.
pub(crate) fn parse_status(body: &Value) -> Result<Vec<PrStatus>, ProviderError> {
    let nodes = data(body)?
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    nodes
        .iter()
        .filter(|node| node.get("id").and_then(Value::as_str).is_some())
        .map(parse_status_node)
        .collect()
}

/// Missing fields read as unknown, so the slim search node gives a blank status.
fn parse_status_node(node: &Value) -> Result<PrStatus, ProviderError> {
    let (unresolved_threads, total_threads) = threads(node);
    Ok(PrStatus {
        remote_id: text(node, "id")?,
        state: node
            .get("state")
            .and_then(Value::as_str)
            .map(pr_state)
            .transpose()?,
        updated_at: node
            .get("updatedAt")
            .and_then(Value::as_str)
            .and_then(parse_time),
        review_decision: node
            .get("reviewDecision")
            .and_then(Value::as_str)
            .map(review_decision)
            .transpose()?,
        checks: node
            .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
            .and_then(Value::as_str)
            .map(checks_state)
            .unwrap_or(ChecksState::None),
        mergeable: mergeable(
            node.get("mergeable")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN"),
        )?,
        merge_state: merge_state(
            node.get("mergeStateStatus")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN"),
        )?,
        unresolved_threads,
        total_threads,
        last_commit_at: node
            .pointer("/commits/nodes/0/commit/committedDate")
            .and_then(Value::as_str)
            .and_then(parse_time),
        merge_queue: super::merge_queue::snapshot_from_node(node),
        failing_checks: failing_checks(node),
        review_clock: review_clock(node),
    })
}

/// `None` when this payload has no `reviewClock` field, so the cache stays.
/// Matches `last: 40` in `status.graphql`.
const REVIEW_CLOCK_PAGE: usize = 40;

fn review_clock(node: &Value) -> Option<ReviewClock> {
    let connection = node.get("reviewClock")?;
    let nodes = connection
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut clock = ReviewClock {
        truncated: nodes.len() >= REVIEW_CLOCK_PAGE,
        ..ReviewClock::default()
    };
    for item in &nodes {
        match item.get("__typename").and_then(Value::as_str) {
            Some("ReadyForReviewEvent") => {
                if let Some(at) = item
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .and_then(parse_time)
                    && clock.ready_at.is_none_or(|kept| at > kept)
                {
                    clock.ready_at = Some(at);
                }
            }
            Some("ReviewRequestedEvent") => {
                if let Some(event) = request_event(item) {
                    clock.requests.push(event);
                }
            }
            Some("PullRequestReview") => {
                if let Some(review) = submitted_review(item) {
                    clock.reviews.push(review);
                }
            }
            _ => {}
        }
    }
    Some(clock)
}

fn request_event(item: &Value) -> Option<ReviewRequestEvent> {
    let at = item
        .get("createdAt")
        .and_then(Value::as_str)
        .and_then(parse_time)?;
    let reviewer = item.get("requestedReviewer")?;
    let target = match reviewer.get("__typename").and_then(Value::as_str) {
        Some("Team") => {
            let team = team_ref(reviewer)?;
            RequestTarget::Team {
                org: team.org,
                slug: team.slug,
            }
        }
        Some("Bot" | "Mannequin") => return None,
        _ => {
            let login = reviewer.get("login").and_then(Value::as_str)?;
            if login.is_empty() {
                return None;
            }
            RequestTarget::User {
                login: login.to_owned(),
            }
        }
    };
    Some(ReviewRequestEvent { at, target })
}

fn submitted_review(item: &Value) -> Option<SubmittedReview> {
    let author = item.get("author")?;
    if matches!(
        author.get("__typename").and_then(Value::as_str),
        Some("Bot" | "Mannequin")
    ) {
        return None;
    }
    let login = author.get("login").and_then(Value::as_str)?;
    if login.is_empty() {
        return None;
    }
    let at = item
        .get("submittedAt")
        .and_then(Value::as_str)
        .and_then(parse_time)?;
    Some(SubmittedReview {
        at,
        login: login.to_owned(),
    })
}

pub(crate) fn parse_viewer_teams(body: &Value) -> Result<Vec<TeamRef>, ProviderError> {
    let viewer = data(body)?.get("viewer").cloned().unwrap_or(Value::Null);
    let mut teams = Vec::new();
    let Some(orgs) = viewer
        .pointer("/organizations/nodes")
        .and_then(Value::as_array)
    else {
        return Ok(teams);
    };
    for org in orgs {
        let Some(login) = org
            .get("login")
            .and_then(Value::as_str)
            .filter(|login| !login.is_empty())
        else {
            continue;
        };
        let Some(nodes) = org.pointer("/teams/nodes").and_then(Value::as_array) else {
            continue;
        };
        for team in nodes {
            let Some(slug) = team
                .get("slug")
                .and_then(Value::as_str)
                .filter(|slug| !slug.is_empty())
            else {
                continue;
            };
            teams.push(TeamRef {
                org: login.to_owned(),
                slug: slug.to_owned(),
            });
        }
    }
    Ok(teams)
}

/// Failing checks on the head commit. `None` when this payload has no
/// `contexts` field, so a caller leaves the cache alone. An empty list means
/// the rollup is not failing, or it failed without a named check.
fn failing_checks(node: &Value) -> Option<Vec<FailingCheck>> {
    let rollup = node.pointer("/commits/nodes/0/commit/statusCheckRollup")?;
    let contexts = rollup.get("contexts")?;
    let state = rollup
        .get("state")
        .and_then(Value::as_str)
        .map(checks_state)
        .unwrap_or(ChecksState::None);
    if !matches!(state, ChecksState::Failure | ChecksState::Error) {
        return Some(Vec::new());
    }
    let Some(nodes) = contexts.get("nodes").and_then(Value::as_array) else {
        return Some(Vec::new());
    };
    let mut out = Vec::new();
    for node in nodes {
        if let Some(check) = one_failing_check(node) {
            out.push(check);
            if out.len() == 100 {
                break;
            }
        }
    }
    Some(out)
}

fn one_failing_check(node: &Value) -> Option<FailingCheck> {
    let (name, conclusion, url) = match node.get("__typename").and_then(Value::as_str)? {
        "CheckRun" => {
            if node.get("status").and_then(Value::as_str) != Some("COMPLETED") {
                return None;
            }
            let conclusion = match node.get("conclusion").and_then(Value::as_str)? {
                "FAILURE" | "STARTUP_FAILURE" => FailingConclusion::Failure,
                "TIMED_OUT" => FailingConclusion::TimedOut,
                "CANCELLED" => FailingConclusion::Cancelled,
                "ACTION_REQUIRED" => FailingConclusion::ActionRequired,
                _ => return None,
            };
            (node.get("name"), conclusion, node.get("detailsUrl"))
        }
        "StatusContext" => {
            let conclusion = match node.get("state").and_then(Value::as_str)? {
                "FAILURE" | "ERROR" => FailingConclusion::Failure,
                _ => return None,
            };
            (node.get("context"), conclusion, node.get("targetUrl"))
        }
        _ => return None,
    };
    let name = name
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    let url = url
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned);
    Some(FailingCheck {
        name: name.chars().take(200).collect(),
        conclusion,
        url,
    })
}

fn assignees(node: &Value) -> Vec<Assignee> {
    let Some(nodes) = node.pointer("/assignees/nodes").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for node in nodes {
        let Some(login) = node
            .get("login")
            .and_then(Value::as_str)
            .filter(|login| !login.is_empty())
        else {
            continue;
        };
        if !seen.insert(login.to_ascii_lowercase()) {
            continue;
        }
        let name = node
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        out.push(Assignee {
            login: login.to_owned(),
            name,
        });
    }
    out
}

/// A requested team from a GraphQL `requestedReviewer` node.
///
/// List sync used to take `slug` and `organization.login`, and fall back to an
/// empty org when GitHub omitted the organization (a field error, or a null
/// org). Two orgs that share a slug then stored as the same `/slug` key, so
/// the reviewer-team filter showed only one option. `combinedSlug` (`org/slug`)
/// is a scalar on the team and still identifies it when `organization` is missing.
pub(crate) fn team_ref(reviewer: &Value) -> Option<TeamRef> {
    if let Some(team) = reviewer
        .get("combinedSlug")
        .and_then(Value::as_str)
        .and_then(team_ref_from_combined)
    {
        return Some(team);
    }
    let slug = reviewer
        .get("slug")
        .and_then(Value::as_str)
        .filter(|slug| !slug.is_empty())?;
    let org = reviewer
        .pointer("/organization/login")
        .and_then(Value::as_str)
        .filter(|org| !org.is_empty())?;
    Some(TeamRef {
        org: org.to_owned(),
        slug: slug.to_owned(),
    })
}

fn team_ref_from_combined(combined: &str) -> Option<TeamRef> {
    let (org, slug) = combined.split_once('/')?;
    if org.is_empty() || slug.is_empty() || slug.contains('/') {
        return None;
    }
    Some(TeamRef {
        org: org.to_owned(),
        slug: slug.to_owned(),
    })
}

fn reviewers(node: &Value) -> Vec<RequestedReviewer> {
    let Some(nodes) = node
        .pointer("/reviewRequests/nodes")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for node in nodes {
        let Some(reviewer) = node.get("requestedReviewer") else {
            continue;
        };
        let text = |key: &str| {
            reviewer
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let identity = if let Some(team) = team_ref(reviewer) {
            Reviewer::Team(team)
        } else if let Some(login) = reviewer.get("login").and_then(Value::as_str) {
            Reviewer::User {
                login: login.to_owned(),
            }
        } else {
            continue;
        };
        out.push(RequestedReviewer {
            reviewer: identity,
            name: text("name"),
            avatar_url: text("avatarUrl"),
        });
    }
    out
}

fn threads(node: &Value) -> (u32, u32) {
    let total = node
        .pointer("/reviewThreads/totalCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let unresolved = node
        .pointer("/reviewThreads/nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter(|thread| thread.get("isResolved").and_then(Value::as_bool) == Some(false))
                .count() as u32
        })
        .unwrap_or(0);
    (unresolved, total)
}

pub(crate) fn data(body: &Value) -> Result<&Value, ProviderError> {
    body.get("data")
        .ok_or_else(|| ProviderError::Decode("graphql response has no data".into()))
}

fn text(node: &Value, key: &str) -> Result<String, ProviderError> {
    node.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ProviderError::Decode(format!("missing {key}")))
}

fn time_field(node: &Value, key: &str) -> Result<DateTime<Utc>, ProviderError> {
    let text = text(node, key)?;
    parse_time(&text).ok_or_else(|| ProviderError::Decode(format!("bad timestamp {text}")))
}

fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn pr_state(value: &str) -> Result<PrState, ProviderError> {
    match value {
        "OPEN" => Ok(PrState::Open),
        "CLOSED" => Ok(PrState::Closed),
        "MERGED" => Ok(PrState::Merged),
        other => Err(ProviderError::Decode(format!("unknown pr state {other}"))),
    }
}

fn review_decision(value: &str) -> Result<ReviewDecision, ProviderError> {
    match value {
        "APPROVED" => Ok(ReviewDecision::Approved),
        "CHANGES_REQUESTED" => Ok(ReviewDecision::ChangesRequested),
        "REVIEW_REQUIRED" => Ok(ReviewDecision::ReviewRequired),
        other => Err(ProviderError::Decode(format!(
            "unknown review decision {other}"
        ))),
    }
}

fn mergeable(value: &str) -> Result<Mergeable, ProviderError> {
    match value {
        "MERGEABLE" => Ok(Mergeable::Mergeable),
        "CONFLICTING" => Ok(Mergeable::Conflicting),
        "UNKNOWN" => Ok(Mergeable::Unknown),
        other => Err(ProviderError::Decode(format!("unknown mergeable {other}"))),
    }
}

fn merge_state(value: &str) -> Result<MergeStateStatus, ProviderError> {
    match value {
        "CLEAN" => Ok(MergeStateStatus::Clean),
        "BEHIND" => Ok(MergeStateStatus::Behind),
        "BLOCKED" => Ok(MergeStateStatus::Blocked),
        "DIRTY" => Ok(MergeStateStatus::Dirty),
        "DRAFT" => Ok(MergeStateStatus::Draft),
        "HAS_HOOKS" => Ok(MergeStateStatus::HasHooks),
        "UNSTABLE" => Ok(MergeStateStatus::Unstable),
        "UNKNOWN" => Ok(MergeStateStatus::Unknown),
        other => Err(ProviderError::Decode(format!(
            "unknown merge state {other}"
        ))),
    }
}

fn checks_state(value: &str) -> ChecksState {
    match value {
        "SUCCESS" => ChecksState::Success,
        "FAILURE" => ChecksState::Failure,
        "PENDING" => ChecksState::Pending,
        "ERROR" => ChecksState::Error,
        "EXPECTED" => ChecksState::Expected,
        _ => ChecksState::None,
    }
}

/// Logins from a `GET /user/orgs` page.
pub(crate) fn parse_rest_orgs(body: &str) -> Result<Vec<String>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|err| ProviderError::Decode(format!("user orgs json: {err}")))?;
    let items = value
        .as_array()
        .ok_or_else(|| ProviderError::Decode("user orgs is not a list".into()))?;
    Ok(items
        .iter()
        .filter_map(|item| item.get("login").and_then(Value::as_str))
        .map(str::to_owned)
        .collect())
}

/// Adds orgs that only REST lists. GraphQL `viewer.organizations` silently
/// drops orgs whose SAML SSO the token is not authorized for.
pub(crate) fn merge_orgs(user: &mut VcsUser, rest: Vec<String>) {
    for org in rest {
        if !user
            .organizations
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&org))
        {
            user.organizations.push(org.clone());
            user.organizations_needing_sso.push(org);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_null_nodes_errors_pass() {
        let gone = serde_json::json!({
            "data": { "nodes": [{ "id": "PR_1" }, null, null] },
            "errors": [
                { "type": "NOT_FOUND", "path": ["nodes", 1], "message": "gone" },
                { "type": "FORBIDDEN", "path": ["nodes", 2], "message": "saml" }
            ]
        });
        assert!(graphql_failure(&gone, Utc::now()).is_none());
        let mutation = serde_json::json!({
            "data": { "mergePullRequest": null },
            "errors": [{ "type": "FORBIDDEN", "path": ["mergePullRequest"], "message": "no" }]
        });
        assert!(graphql_failure(&mutation, Utc::now()).is_some());
        let null_data = serde_json::json!({
            "data": null,
            "errors": [{ "type": "NOT_FOUND", "path": ["nodes", 0], "message": "gone" }]
        });
        assert!(graphql_failure(&null_data, Utc::now()).is_some());
        let top_level = serde_json::json!({
            "data": { "nodes": [null] },
            "errors": [{ "type": "NOT_FOUND", "message": "gone" }]
        });
        assert!(graphql_failure(&top_level, Utc::now()).is_some());
    }

    #[test]
    fn assignees_keep_the_first_login_and_skip_a_missing_list() {
        let body = serde_json::json!({
            "data": { "search": { "nodes": [{
                "id": "PR_1",
                "number": 7,
                "title": "Latch",
                "state": "OPEN",
                "createdAt": "2026-11-01T00:00:00Z",
                "updatedAt": "2026-11-02T00:00:00Z",
                "repository": { "owner": { "login": "acme" }, "name": "widgets" },
                "assignees": { "nodes": [
                    { "login": "Ada", "name": "Ada" },
                    { "login": "ada" },
                    { "login": "sam", "name": "Sam" },
                    { "login": "" }
                ]}
            }]}}
        });
        let page = parse_search(&body, AccountId(1), Involvement::empty()).unwrap();
        let people = &page.items[0].pull_request.assignees;
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].login, "Ada");
        assert_eq!(people[0].name.as_deref(), Some("Ada"));
        assert_eq!(people[1].login, "sam");

        let bare = serde_json::json!({
            "data": { "search": { "nodes": [{
                "id": "PR_2",
                "state": "OPEN",
                "createdAt": "2026-11-01T00:00:00Z",
                "updatedAt": "2026-11-02T00:00:00Z"
            }]}}
        });
        let page = parse_search(&bare, AccountId(1), Involvement::empty()).unwrap();
        assert!(page.items[0].pull_request.assignees.is_empty());
    }

    #[test]
    fn status_keeps_named_failing_checks_and_skips_the_rest() {
        let body = serde_json::json!({
            "data": { "nodes": [{
                "id": "PR_1",
                "commits": { "nodes": [{ "commit": { "statusCheckRollup": {
                    "state": "FAILURE",
                    "contexts": { "nodes": [
                        {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://github.com/acme/widgets/actions/runs/1"},
                        {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://github.com/acme/widgets/actions/runs/2"},
                        {"__typename":"CheckRun","name":"integration","status":"COMPLETED","conclusion":"TIMED_OUT","detailsUrl":"https://github.com/acme/widgets/actions/runs/3"},
                        {"__typename":"CheckRun","name":"nightly","status":"COMPLETED","conclusion":"CANCELLED","detailsUrl":null},
                        {"__typename":"CheckRun","name":"deploy","status":"COMPLETED","conclusion":"ACTION_REQUIRED","detailsUrl":"https://github.com/acme/widgets/actions/runs/4"},
                        {"__typename":"CheckRun","name":"boot","status":"COMPLETED","conclusion":"STARTUP_FAILURE","detailsUrl":"https://github.com/acme/widgets/actions/runs/5"},
                        {"__typename":"CheckRun","name":"still-running","status":"IN_PROGRESS","conclusion":null,"detailsUrl":null},
                        {"__typename":"StatusContext","context":"ci/legacy","state":"ERROR","targetUrl":"https://github.com/acme/widgets/commit/abc"}
                    ]}
                }}}]}
            }, {
                "id": "PR_green",
                "commits": { "nodes": [{ "commit": { "statusCheckRollup": {
                    "state": "SUCCESS",
                    "contexts": { "nodes": [
                        {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":null}
                    ]}
                }}}]}
            }, {
                "id": "PR_bare",
                "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "FAILURE" } } }] }
            }] }
        });
        let status = parse_status(&body).unwrap();
        assert_eq!(status[0].checks, ChecksState::Failure);
        let checks = status[0].failing_checks.as_ref().unwrap();
        assert_eq!(
            checks
                .iter()
                .map(|check| (check.name.as_str(), check.conclusion, check.url.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                (
                    "lint",
                    FailingConclusion::Failure,
                    Some("https://github.com/acme/widgets/actions/runs/1")
                ),
                (
                    "integration",
                    FailingConclusion::TimedOut,
                    Some("https://github.com/acme/widgets/actions/runs/3")
                ),
                ("nightly", FailingConclusion::Cancelled, None),
                (
                    "deploy",
                    FailingConclusion::ActionRequired,
                    Some("https://github.com/acme/widgets/actions/runs/4")
                ),
                (
                    "boot",
                    FailingConclusion::Failure,
                    Some("https://github.com/acme/widgets/actions/runs/5")
                ),
                (
                    "ci/legacy",
                    FailingConclusion::Failure,
                    Some("https://github.com/acme/widgets/commit/abc")
                ),
            ]
        );
        assert_eq!(status[1].failing_checks.as_deref(), Some(&[][..]));
        assert!(status[2].failing_checks.is_none());
    }

    #[test]
    fn a_missing_organization_does_not_collapse_the_same_slug_in_two_orgs() {
        let request = |combined: &str| {
            serde_json::json!({
                "reviewRequests": { "nodes": [{
                    "requestedReviewer": {
                        "slug": "platform",
                        "name": "Platform",
                        "combinedSlug": combined,
                        "organization": null
                    }
                }]}
            })
        };
        let acme = reviewers(&request("acme/platform"));
        let labs = reviewers(&request("acme-labs/platform"));
        let team = |items: &[RequestedReviewer]| match &items[0].reviewer {
            Reviewer::Team(team) => team.to_string(),
            Reviewer::User { .. } => panic!("expected a team"),
        };
        assert_eq!(team(&acme), "acme/platform");
        assert_eq!(team(&labs), "acme-labs/platform");
        assert_eq!(acme[0].name.as_deref(), Some("Platform"));
    }

    #[test]
    fn a_team_without_an_org_or_combined_slug_is_dropped() {
        let node = serde_json::json!({
            "reviewRequests": { "nodes": [{
                "requestedReviewer": { "slug": "platform", "name": "Platform", "organization": null }
            }]}
        });
        assert!(reviewers(&node).is_empty());
    }

    #[test]
    fn a_team_still_parses_from_organization_login_without_combined_slug() {
        let node = serde_json::json!({
            "reviewRequests": { "nodes": [{
                "requestedReviewer": {
                    "slug": "web",
                    "name": "Web",
                    "organization": { "login": "acme" }
                }
            }]}
        });
        let items = reviewers(&node);
        match &items[0].reviewer {
            Reviewer::Team(team) => {
                assert_eq!(team.org, "acme");
                assert_eq!(team.slug, "web");
            }
            Reviewer::User { .. } => panic!("expected a team"),
        }
    }

    #[test]
    fn status_keeps_the_latest_review_request_for_a_user_or_a_team() {
        let body = serde_json::json!({
            "data": { "nodes": [{
                "id": "PR_1",
                "reviewClock": { "nodes": [
                    {"__typename":"ReviewRequestedEvent","createdAt":"2026-11-16T12:00:00Z",
                     "requestedReviewer":{"__typename":"User","login":"ada"}},
                    {"__typename":"ReviewRequestedEvent","createdAt":"2026-11-16T13:00:00Z",
                     "requestedReviewer":{"__typename":"Team","slug":"web","organization":{"login":"acme"}}},
                    {"__typename":"ReviewRequestedEvent","createdAt":"2026-11-16T14:00:00Z",
                     "requestedReviewer":{"__typename":"Bot","login":"helper"}},
                    {"__typename":"ReadyForReviewEvent","createdAt":"2026-11-16T11:00:00Z"},
                    {"__typename":"PullRequestReview","submittedAt":"2026-11-16T15:00:00Z",
                     "author":{"__typename":"User","login":"ada"}},
                    {"__typename":"PullRequestReview","submittedAt":"2026-11-16T15:30:00Z",
                     "author":{"__typename":"Bot","login":"helper"}}
                ]}
            }, {
                "id": "PR_bare"
            }] }
        });
        let status = parse_status(&body).unwrap();
        let clock = status[0].review_clock.as_ref().unwrap();
        assert_eq!(clock.requests.len(), 2);
        assert_eq!(clock.reviews.len(), 1);
        assert!(clock.ready_at.is_some());
        assert!(status[1].review_clock.is_none());

        let teams = parse_viewer_teams(&serde_json::json!({
            "data": { "viewer": { "organizations": { "nodes": [
                {"login":"acme","teams":{"nodes":[{"slug":"web"},{"slug":""}]}},
                {"login":"","teams":{"nodes":[{"slug":"other"}]}}
            ]}}}
        }))
        .unwrap();
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0].org, "acme");
        assert_eq!(teams[0].slug, "web");
    }
}
