//! GraphQL JSON to domain pull requests.

use chrono::{DateTime, Utc};
use pr_nexus_core::domain::{
    AccountId, ChecksState, Involvement, MergeStateStatus, Mergeable, PrId, PrState, ProviderRef,
    PullRequest, RepoRef, ReviewDecision, Reviewer, TeamRef,
};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{RateBudget, SyncedPullRequest, VcsUser};
use serde_json::Value;

pub(crate) struct SearchPage {
    pub(crate) items: Vec<SyncedPullRequest>,
    pub(crate) has_next: bool,
    pub(crate) cursor: Option<String>,
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

pub(crate) fn graphql_failure(body: &Value) -> Option<ProviderError> {
    let errors = body.get("errors")?.as_array()?;
    if errors.is_empty() {
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
            reset_at: Utc::now(),
            secondary: false,
        });
    }
    Some(ProviderError::Api {
        code: "graphql".into(),
        message,
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
    let committed = node
        .pointer("/commits/nodes/0/commit/committedDate")
        .and_then(Value::as_str)
        .and_then(parse_time);
    let last_activity_at = match committed {
        Some(committed) if committed > updated_at => committed,
        _ => updated_at,
    };
    let checks = node
        .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
        .and_then(Value::as_str)
        .map(checks_state)
        .unwrap_or(ChecksState::None);
    let (unresolved, total) = threads(node);
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
    Ok(SyncedPullRequest {
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
            review_decision: node
                .get("reviewDecision")
                .and_then(Value::as_str)
                .map(review_decision)
                .transpose()?,
            checks,
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
            requested_reviewers: reviewers(node),
            unresolved_threads: unresolved,
            total_threads: total,
            involvement,
            created_at,
            updated_at,
            last_activity_at,
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
    })
}

fn reviewers(node: &Value) -> Vec<Reviewer> {
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
        if let Some(slug) = reviewer.get("slug").and_then(Value::as_str) {
            let org = reviewer
                .pointer("/organization/login")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            out.push(Reviewer::Team(TeamRef {
                org,
                slug: slug.to_owned(),
            }));
        } else if let Some(login) = reviewer.get("login").and_then(Value::as_str) {
            out.push(Reviewer::User {
                login: login.to_owned(),
            });
        }
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

fn data(body: &Value) -> Result<&Value, ProviderError> {
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
