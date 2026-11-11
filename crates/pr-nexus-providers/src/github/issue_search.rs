//! REST issue search for a query the user typed.
//!
//! GraphQL `search` has no sort argument. The REST search takes `sort` and
//! `order`, so `sort:updated-desc` and the other sorts are the order GitHub
//! returns, and paging follows that order up to GitHub's 1000-result cap.

use chrono::{DateTime, Utc};
use pr_nexus_core::domain::{
    AccountId, ChecksState, Involvement, MergeStateStatus, Mergeable, PrId, PrState, ProviderRef,
    PullRequest, RepoRef,
};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::SyncedPullRequest;
use serde_json::Value;

use super::GithubClient;
use super::http;

/// One search hit, plus the comment count REST uses for `sort:comments`.
#[derive(Debug)]
pub struct IssueHit {
    pub synced: SyncedPullRequest,
    pub comments: u32,
}

#[derive(Debug)]
pub struct IssueSearch {
    pub total: u64,
    pub hits: Vec<IssueHit>,
}

/// GitHub returns at most this many search hits for one query string.
pub const SEARCH_CAP: usize = 1000;
const PAGE: usize = 100;

impl GithubClient {
    /// The first `limit` pull requests matching `query`, in the API's order.
    /// Issues are dropped even if the query forgot `is:pr`. `host` is stored
    /// on each row (`github.com` for GitHub.com).
    pub fn search_issues(
        &self,
        query: &str,
        sort: Option<&str>,
        descending: bool,
        host: &str,
        limit: usize,
    ) -> Result<IssueSearch, ProviderError> {
        let limit = limit.min(SEARCH_CAP);
        let mut hits = Vec::new();
        let mut total = 0u64;
        let mut page = 1u32;
        while hits.len() < limit && page <= (SEARCH_CAP as u32 / PAGE as u32) {
            let page_text = page.to_string();
            let order = if descending { "desc" } else { "asc" };
            let mut params = vec![
                ("q", query),
                ("per_page", "100"),
                ("page", page_text.as_str()),
            ];
            if let Some(sort) = sort {
                params.push(("sort", sort));
                params.push(("order", order));
            }
            let body = self.rest_search(&params)?;
            let parsed = parse_issue_search(&body, self.account, host)?;
            total = parsed.total;
            let count = parsed.hits.len();
            hits.extend(parsed.hits);
            if count < PAGE || hits.len() as u64 >= total {
                break;
            }
            page += 1;
        }
        hits.truncate(limit);
        Ok(IssueSearch { total, hits })
    }

    /// `GET /search/issues`. A 422 keeps GitHub's own explanation.
    fn rest_search(&self, query: &[(&str, &str)]) -> Result<String, ProviderError> {
        let _permit = http::Permit::acquire(&self.gate);
        let response = self
            .http
            .get(format!("{}/search/issues", self.api))
            .query(query)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        let status = response.status().as_u16();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let sso = response
            .headers()
            .get("x-github-sso")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .text()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        if status == 422 {
            return Err(ProviderError::Api {
                code: "422".into(),
                message: validation_message(&body),
            });
        }
        if !(200..300).contains(&status) {
            return Err(http::classify(
                status,
                retry.as_deref(),
                sso.as_deref(),
                &body,
                Utc::now(),
            ));
        }
        Ok(body)
    }
}

/// The sentence GitHub put on a rejected search, not the generic
/// "Validation Failed" wrapper.
pub fn validation_message(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return "GitHub rejected that query.".to_owned();
    };
    if let Some(message) = value
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| {
            errors.iter().find_map(|error| {
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .filter(|message| !message.is_empty())
            })
        })
    {
        return message.to_owned();
    }
    value
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty() && !message.eq_ignore_ascii_case("Validation Failed"))
        .map(str::to_owned)
        .unwrap_or_else(|| "GitHub rejected that query.".to_owned())
}

pub fn parse_issue_search(
    body: &str,
    account: AccountId,
    host: &str,
) -> Result<IssueSearch, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|err| ProviderError::Decode(format!("search json: {err}")))?;
    let total = value
        .get("total_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Decode("search json: missing items".into()))?;
    let mut hits = Vec::new();
    for item in items {
        if item.get("pull_request").is_none()
            || item.get("pull_request").is_some_and(Value::is_null)
        {
            continue;
        }
        let Some(hit) = hit_from_item(item, account, host) else {
            continue;
        };
        hits.push(hit);
    }
    Ok(IssueSearch { total, hits })
}

fn hit_from_item(item: &Value, account: AccountId, host: &str) -> Option<IssueHit> {
    let node = item.get("node_id").and_then(Value::as_str)?;
    let number = item.get("number").and_then(Value::as_u64)?;
    let title = item
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let url = item
        .get("html_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let (owner, name) = repo_of(
        item.get("repository_url")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    let author = item
        .pointer("/user/login")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let body = item
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let created_at = parse_time(item.get("created_at").and_then(Value::as_str)?)?;
    let updated_at = parse_time(item.get("updated_at").and_then(Value::as_str)?)?;
    let merged = item
        .pointer("/pull_request/merged_at")
        .and_then(Value::as_str)
        .is_some();
    let state = match item.get("state").and_then(Value::as_str).unwrap_or("open") {
        _ if merged => PrState::Merged,
        "closed" => PrState::Closed,
        _ => PrState::Open,
    };
    let labels = item
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| label.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let comments = item
        .get("comments")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32;
    Some(IssueHit {
        comments,
        synced: SyncedPullRequest {
            repo_node_id: None,
            default_branch: None,
            repo_archived: false,
            merge_queue: None,
            pull_request: PullRequest {
                id: PrId(0),
                remote: ProviderRef {
                    account,
                    id: node.to_owned(),
                },
                repo: RepoRef {
                    host: host.to_owned(),
                    owner,
                    name,
                },
                number,
                title,
                url,
                author_login: author,
                body,
                state,
                is_draft: item.get("draft").and_then(Value::as_bool).unwrap_or(false),
                head_ref: String::new(),
                base_ref: String::new(),
                head_sha: String::new(),
                labels,
                review_decision: None,
                checks: ChecksState::None,
                mergeable: Mergeable::Unknown,
                merge_state: MergeStateStatus::Unknown,
                requested_reviewers: Vec::new(),
                unresolved_threads: 0,
                total_threads: 0,
                involvement: Involvement::empty(),
                created_at,
                updated_at,
                last_activity_at: updated_at,
                synced_at: Utc::now(),
                hidden: false,
            },
        },
    })
}

fn repo_of(url: &str) -> Option<(String, String)> {
    let marker = "/repos/";
    let at = url.find(marker)?;
    let rest = &url[at + marker.len()..];
    let (owner, name) = rest.split_once('/')?;
    let name = name.split(['/', '?']).next().unwrap_or(name);
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some((owner.to_owned(), name.to_owned()))
}

fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_validation_error_keeps_githubs_sentence() {
        let body = r#"{
            "message": "Validation Failed",
            "errors": [{
                "message": "The listed users and repositories cannot be searched either because the resources do not exist or you do not have permission to view them.",
                "resource": "Search",
                "field": "q",
                "code": "invalid"
            }]
        }"#;
        let message = validation_message(body);
        assert!(message.contains("cannot be searched"));
        assert!(!message.contains("Validation Failed"));
        assert!(!message.contains("422"));
    }

    #[test]
    fn a_search_page_keeps_pull_requests_and_their_state() {
        let body = r#"{
            "total_count": 3,
            "items": [
                {
                    "node_id": "PR_1",
                    "number": 7,
                    "title": "Add a widget",
                    "html_url": "https://github.com/acme/widgets/pull/7",
                    "repository_url": "https://api.github.com/repos/acme/widgets",
                    "user": {"login": "octocat"},
                    "body": "Please review",
                    "state": "open",
                    "draft": true,
                    "comments": 4,
                    "created_at": "2026-11-01T00:00:00Z",
                    "updated_at": "2026-11-08T00:00:00Z",
                    "labels": [{"name": "bug"}],
                    "pull_request": {"merged_at": null}
                },
                {
                    "node_id": "ISSUE_2",
                    "number": 8,
                    "title": "An issue",
                    "state": "open",
                    "user": {"login": "octocat"},
                    "created_at": "2026-11-01T00:00:00Z",
                    "updated_at": "2026-11-08T00:00:00Z"
                },
                {
                    "node_id": "PR_3",
                    "number": 9,
                    "title": "Ship the widget",
                    "html_url": "https://github.com/acme/widgets/pull/9",
                    "repository_url": "https://api.github.com/repos/acme/widgets",
                    "user": {"login": "octocat"},
                    "state": "closed",
                    "comments": 1,
                    "created_at": "2026-10-01T00:00:00Z",
                    "updated_at": "2026-11-02T00:00:00Z",
                    "pull_request": {"merged_at": "2026-11-02T00:00:00Z"}
                }
            ]
        }"#;
        let page = parse_issue_search(body, AccountId(1), "github.com").unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.hits.len(), 2);
        let open = &page.hits[0];
        assert_eq!(open.synced.pull_request.number, 7);
        assert_eq!(open.synced.pull_request.repo.owner, "acme");
        assert_eq!(open.synced.pull_request.repo.name, "widgets");
        assert_eq!(open.synced.pull_request.author_login, "octocat");
        assert!(open.synced.pull_request.is_draft);
        assert_eq!(open.synced.pull_request.state, PrState::Open);
        assert_eq!(open.synced.pull_request.labels, vec!["bug".to_owned()]);
        assert_eq!(open.comments, 4);
        assert_eq!(page.hits[1].synced.pull_request.state, PrState::Merged);
    }
}
