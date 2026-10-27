//! A fake GitHub search that behaves like the real one: cursor pages of the
//! requested size, `issueCount`, `created:` ranges, and no results past the
//! 1000th of any one query string.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use pr_nexus_core::domain::AccountId;
use pr_nexus_core::providers::{PrQuery, SearchBucket, SyncedPullRequest, VcsProvider};
use pr_nexus_providers::github::GithubClient;
use pr_nexus_providers::github::search::SEARCH_RESULT_LIMIT;
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const QUERY: &str = "is:pr is:open author:@me archived:false sort:updated-desc org:acme";

struct FakeSearch {
    /// Creation times, one pull request each; number = index + 1.
    created: Vec<DateTime<Utc>>,
    /// Every query string asked, with the `issueCount` it got.
    asked: Arc<Mutex<Vec<(String, usize)>>>,
    /// Requests left to answer with a secondary rate limit.
    throttled: Arc<Mutex<u32>>,
}

fn range_of(query: &str) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let (_, rest) = query.split_once(" created:")?;
    let range = rest.split_whitespace().next()?;
    let (from, to) = range.split_once("..")?;
    Some((
        DateTime::parse_from_rfc3339(from).ok()?.with_timezone(&Utc),
        DateTime::parse_from_rfc3339(to).ok()?.with_timezone(&Utc),
    ))
}

fn node(number: usize, created: DateTime<Utc>) -> Value {
    json!({
        "id": format!("PR_node_{number}"),
        "number": number,
        "title": format!("Change {number}"),
        "url": format!("https://github.com/acme/widgets/pull/{number}"),
        "bodyText": "",
        "isDraft": false,
        "state": "OPEN",
        "createdAt": created.to_rfc3339(),
        "updatedAt": created.to_rfc3339(),
        "author": { "login": "ada" },
        "headRefName": format!("feat/{number}"),
        "baseRefName": "main",
        "headRefOid": "abc123",
        "repository": {
            "id": "R_widgets",
            "owner": { "login": "acme" },
            "name": "widgets",
            "isArchived": false,
            "defaultBranchRef": { "name": "main" }
        },
        "labels": { "nodes": [] },
        "reviewRequests": { "nodes": [] }
    })
}

impl Respond for FakeSearch {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        {
            let mut left = self.throttled.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                return ResponseTemplate::new(403)
                    .insert_header("retry-after", "1")
                    .set_body_string(r#"{"message":"You have exceeded a secondary rate limit."}"#);
            }
        }
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let vars = &body["variables"];
        let query = vars["query"].as_str().unwrap().to_owned();
        let first = vars["first"].as_u64().unwrap() as usize;
        let offset = vars["cursor"]
            .as_str()
            .map_or(0, |cursor| cursor.parse::<usize>().unwrap());
        let matches: Vec<usize> = self
            .created
            .iter()
            .enumerate()
            .filter(|(_, at)| range_of(&query).is_none_or(|(from, to)| **at >= from && **at <= to))
            .map(|(index, _)| index)
            .collect();
        self.asked.lock().unwrap().push((query, matches.len()));
        // Like GitHub: the count is exact, the pages stop at 1000.
        let reachable = matches.len().min(SEARCH_RESULT_LIMIT as usize);
        let end = (offset + first).min(reachable);
        let nodes: Vec<Value> = matches[offset.min(end)..end]
            .iter()
            .map(|&index| node(index + 1, self.created[index]))
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "rateLimit": { "limit": 5000, "remaining": 4000, "resetAt": "2026-10-26T12:00:00Z", "cost": 1 },
                "search": {
                    "issueCount": matches.len(),
                    "pageInfo": { "hasNextPage": end < reachable, "endCursor": end.to_string() },
                    "nodes": nodes
                }
            }
        }))
    }
}

struct Fixture {
    client: GithubClient,
    asked: Arc<Mutex<Vec<(String, usize)>>>,
    _server: MockServer,
    _runtime: tokio::runtime::Runtime,
}

fn fake_github(created: Vec<DateTime<Utc>>, throttled: u32) -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let server = runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(FakeSearch {
                created,
                asked: asked.clone(),
                throttled: Arc::new(Mutex::new(throttled)),
            })
            .mount(&server)
            .await;
        server
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token")
        .unwrap()
        .with_backoff(Duration::ZERO)
        .with_rate_wait_cap(Duration::ZERO);
    Fixture {
        client,
        asked,
        _server: server,
        _runtime: runtime,
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

/// One pull request per hour from 2020-01-01.
fn hourly(count: usize) -> Vec<DateTime<Utc>> {
    (0..count)
        .map(|index| at(1_577_836_800 + index as i64 * 3600))
        .collect()
}

fn search(fixture: &Fixture) -> (Vec<SyncedPullRequest>, Vec<usize>) {
    let mut pages = Vec::new();
    let rows = fixture
        .client
        .search_pull_requests_paged(
            &PrQuery {
                bucket: SearchBucket::Mine,
                queries: vec![QUERY.into()],
            },
            &mut |page| pages.push(page.len()),
        )
        .unwrap();
    (rows, pages)
}

fn numbers(rows: &[SyncedPullRequest]) -> Vec<u64> {
    let mut numbers: Vec<u64> = rows.iter().map(|row| row.pull_request.number).collect();
    numbers.sort_unstable();
    numbers
}

#[test]
fn follows_every_page_past_one_hundred() {
    let fixture = fake_github(hourly(250), 0);
    let (rows, pages) = search(&fixture);
    assert_eq!(numbers(&rows), (1..=250).collect::<Vec<_>>());
    // Pages reach the caller one by one, not as one batch at the end.
    assert_eq!(pages.len(), 25);
    assert_eq!(pages.iter().sum::<usize>(), 250);
    // Under the limit, the query is not split.
    let asked = fixture.asked.lock().unwrap();
    assert!(asked.iter().all(|(query, _)| query == QUERY));
}

#[test]
fn splits_by_creation_date_past_one_thousand() {
    // 2,600 over about three months, plus 1,200 inside one hour so the split
    // has to go below a day.
    let mut created = hourly(2_600);
    created.extend((0..1_200).map(|index| at(1_600_000_000 + index * 2)));
    let total = created.len();
    let fixture = fake_github(created, 0);
    let (rows, pages) = search(&fixture);
    assert_eq!(numbers(&rows), (1..=total as u64).collect::<Vec<_>>());
    assert_eq!(pages.iter().sum::<usize>(), total);
    let asked = fixture.asked.lock().unwrap();
    // The plain query is asked once, sees the count, and is split.
    assert_eq!(asked.iter().filter(|(query, _)| query == QUERY).count(), 1);
    // Every range that was paged past its first page was under the limit.
    let mut paged: Vec<&String> = Vec::new();
    for (query, count) in asked.iter() {
        if paged.contains(&query) {
            assert!(*count as u64 <= SEARCH_RESULT_LIMIT, "{query} has {count}");
        }
        paged.push(query);
    }
    assert!(asked.iter().any(|(query, _)| query.contains(" created:")));
}

#[test]
fn waits_out_a_secondary_rate_limit_and_carries_on() {
    let fixture = fake_github(hourly(120), 2);
    let (rows, _) = search(&fixture);
    assert_eq!(rows.len(), 120);
}

#[test]
fn gives_up_after_repeated_secondary_rate_limits() {
    let fixture = fake_github(hourly(30), 10);
    let err = fixture
        .client
        .search_pull_requests(&PrQuery {
            bucket: SearchBucket::Mine,
            queries: vec![QUERY.into()],
        })
        .unwrap_err();
    assert!(matches!(
        err,
        pr_nexus_core::error::ProviderError::RateLimited {
            secondary: true,
            ..
        }
    ));
}
