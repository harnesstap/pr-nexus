//! The Stats view's fetch (§8.2): the viewer's own pull requests in a period
//! with their review timelines, one search page at a time.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::stats::{
    RequestTarget, StatsFetch, StatsPr, StatsPrState, StatsRequest, StatsReview,
};
use serde_json::{Value, json};

use super::GithubClient;
use super::parse::data;

const REVIEW_STATS: &str = include_str!("graphql/review_stats.graphql");

/// Stop early (flagged partial) when this little GraphQL budget is left, so
/// the stats never starve the sync.
pub const BUDGET_FLOOR: u32 = 300;

/// Most pull requests one fetch reads (flagged partial past it).
pub const MAX_PRS: usize = 500;

pub(crate) fn fetch(
    client: &GithubClient,
    queries: &[String],
    on_progress: &mut dyn FnMut(usize, u64),
) -> Result<StatsFetch, ProviderError> {
    let mut seen = HashSet::new();
    let mut prs = Vec::new();
    let mut total = 0u64;
    for query in queries {
        let mut cursor: Option<String> = None;
        let mut level = 0;
        loop {
            let body = client.paged("review stats", &mut level, |size| {
                client.graphql(
                    REVIEW_STATS,
                    json!({ "query": query, "first": size, "cursor": cursor }),
                )
            })?;
            let page = parse(&body)?;
            total = total.max(page.total);
            for (id, pr) in page.items {
                if seen.insert(id) {
                    prs.push(pr);
                }
            }
            on_progress(prs.len(), total);
            let low = client
                .budget
                .lock()
                .expect("rate budget")
                .as_ref()
                .is_some_and(|budget| budget.remaining < BUDGET_FLOOR);
            if low || prs.len() >= MAX_PRS {
                return Ok(StatsFetch { prs, partial: true });
            }
            match (page.has_next, page.cursor) {
                (true, Some(next)) => cursor = Some(next),
                _ => break,
            }
        }
    }
    Ok(StatsFetch {
        prs,
        partial: false,
    })
}

pub(crate) struct Page {
    pub items: Vec<(String, StatsPr)>,
    pub has_next: bool,
    pub cursor: Option<String>,
    pub total: u64,
}

fn time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|at| at.with_timezone(&Utc))
}

fn text<'a>(node: &'a Value, pointer: &str) -> Option<&'a str> {
    node.pointer(pointer).and_then(Value::as_str)
}

pub(crate) fn parse(body: &Value) -> Result<Page, ProviderError> {
    let search = data(body)?.get("search").cloned().unwrap_or(Value::Null);
    let mut items = Vec::new();
    for node in search
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(number), Some(created_at), Some(repo)) = (
            text(node, "/id"),
            node.get("number").and_then(Value::as_u64),
            time(node.get("createdAt")),
            text(node, "/repository/nameWithOwner"),
        ) else {
            continue;
        };
        let state = match text(node, "/state") {
            Some("MERGED") => StatsPrState::Merged,
            Some("CLOSED") => StatsPrState::Closed,
            _ => StatsPrState::Open,
        };
        let mut pr = StatsPr {
            repo: repo.to_owned(),
            number,
            author: text(node, "/author/login").unwrap_or_default().to_owned(),
            created_at,
            is_draft: node
                .get("isDraft")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            state,
            merged_at: time(node.get("mergedAt")),
            ready_events: Vec::new(),
            requests: Vec::new(),
            reviews: Vec::new(),
        };
        for item in node
            .pointer("/timelineItems/nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match text(item, "/__typename") {
                Some("ReadyForReviewEvent") => pr.ready_events.extend(time(item.get("createdAt"))),
                Some("ReviewRequestedEvent") => {
                    let Some(at) = time(item.get("createdAt")) else {
                        continue;
                    };
                    let reviewer = item.get("requestedReviewer").unwrap_or(&Value::Null);
                    let target = match text(reviewer, "/__typename") {
                        Some("Team") => match (
                            text(reviewer, "/organization/login"),
                            text(reviewer, "/slug"),
                        ) {
                            (Some(org), Some(slug)) => RequestTarget::Team(format!("{org}/{slug}")),
                            _ => continue,
                        },
                        Some("Bot") => continue,
                        _ => match text(reviewer, "/login") {
                            Some(login) => RequestTarget::User(login.to_owned()),
                            None => continue,
                        },
                    };
                    pr.requests.push(StatsRequest { target, at });
                }
                Some("PullRequestReview") => {
                    let (Some(author), Some(at)) =
                        (text(item, "/author/login"), time(item.get("submittedAt")))
                    else {
                        continue;
                    };
                    pr.reviews.push(StatsReview {
                        author: author.to_owned(),
                        is_bot: text(item, "/author/__typename") == Some("Bot"),
                        at,
                    });
                }
                _ => {}
            }
        }
        items.push((id.to_owned(), pr));
    }
    Ok(Page {
        items,
        has_next: search
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        cursor: text(&search, "/pageInfo/endCursor").map(str::to_owned),
        total: search
            .get("issueCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ready_events_requests_and_reviews() {
        let body = json!({ "data": { "search": {
            "issueCount": 1,
            "pageInfo": { "hasNextPage": true, "endCursor": "c1" },
            "nodes": [{
                "id": "PR_1", "number": 42, "isDraft": false, "state": "MERGED",
                "createdAt": "2026-03-01T09:00:00Z", "mergedAt": "2026-03-02T09:00:00Z",
                "author": { "login": "ada" },
                "repository": { "nameWithOwner": "acme/api" },
                "timelineItems": { "nodes": [
                    { "__typename": "ReadyForReviewEvent", "createdAt": "2026-03-01T10:00:00Z" },
                    { "__typename": "ReviewRequestedEvent", "createdAt": "2026-03-01T10:00:00Z",
                      "requestedReviewer": { "__typename": "User", "login": "grace" } },
                    { "__typename": "ReviewRequestedEvent", "createdAt": "2026-03-01T10:00:00Z",
                      "requestedReviewer": { "__typename": "Team", "slug": "platform", "organization": { "login": "acme" } } },
                    { "__typename": "ReviewRequestedEvent", "createdAt": "2026-03-01T10:00:00Z",
                      "requestedReviewer": { "__typename": "Bot", "login": "helper" } },
                    { "__typename": "PullRequestReview", "submittedAt": "2026-03-01T12:00:00Z",
                      "author": { "__typename": "User", "login": "grace" } },
                    { "__typename": "PullRequestReview", "submittedAt": null,
                      "author": { "__typename": "User", "login": "linus" } },
                    { "__typename": "PullRequestReview", "submittedAt": "2026-03-01T11:00:00Z",
                      "author": { "__typename": "Bot", "login": "helper" } }
                ]}
            }, null]
        }}});
        let page = parse(&body).unwrap();
        assert!(page.has_next);
        assert_eq!(page.cursor.as_deref(), Some("c1"));
        let (id, pr) = &page.items[0];
        assert_eq!(
            (id.as_str(), pr.repo.as_str(), pr.number),
            ("PR_1", "acme/api", 42)
        );
        assert_eq!(pr.state, StatsPrState::Merged);
        assert_eq!(pr.ready_events.len(), 1);
        assert_eq!(
            pr.requests
                .iter()
                .map(|r| r.target.clone())
                .collect::<Vec<_>>(),
            vec![
                RequestTarget::User("grace".into()),
                RequestTarget::Team("acme/platform".into())
            ]
        );
        assert_eq!(pr.reviews.len(), 2, "a pending review has no submittedAt");
        assert!(pr.reviews[1].is_bot);
    }
}
