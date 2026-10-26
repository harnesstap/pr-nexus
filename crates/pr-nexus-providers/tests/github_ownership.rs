//! Ownership inputs against a mock GitHub (§6).

use chrono::{TimeZone, Utc};
use pr_nexus_core::domain::{
    AccountId, ChecksState, Involvement, MergeStateStatus, Mergeable, PrId, PrState, ProviderRef,
    PullRequest, RepoRef, TeamRef,
};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{ClassicProtection, VcsProvider};
use pr_nexus_providers::github::GithubClient;
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn pr() -> PullRequest {
    let at = Utc.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap();
    PullRequest {
        id: PrId(1),
        remote: ProviderRef {
            account: AccountId(1),
            id: "PR_1".into(),
        },
        repo: RepoRef {
            host: "github.com".into(),
            owner: "acme".into(),
            name: "widgets".into(),
        },
        number: 7,
        title: "Tighten the latch".into(),
        url: "https://github.com/acme/widgets/pull/7".into(),
        author_login: "alice".into(),
        body: String::new(),
        state: PrState::Open,
        is_draft: false,
        head_ref: "latch".into(),
        base_ref: "main".into(),
        head_sha: "h1".into(),
        labels: Vec::new(),
        review_decision: None,
        checks: ChecksState::Success,
        mergeable: Mergeable::Mergeable,
        merge_state: MergeStateStatus::Blocked,
        requested_reviewers: Vec::new(),
        unresolved_threads: 0,
        total_threads: 0,
        involvement: Involvement::AUTHOR,
        created_at: at,
        updated_at: at,
        last_activity_at: at,
        synced_at: at,
        hidden: false,
    }
}

const PROBE: &str = r#"{"data":{
  "rateLimit":{"limit":5000,"remaining":4990,"resetAt":"2026-10-26T13:00:00Z","cost":1},
  "repository":{
    "viewerPermission":"WRITE",
    "githubDir":{"oid":"blob1","text":"* @acme/core\n","isTruncated":false},
    "root":null,
    "docs":null,
    "pullRequest":{
      "headRefOid":"h1",
      "baseRefName":"main",
      "latestOpinionatedReviews":{
        "pageInfo":{"hasNextPage":false},
        "nodes":[{"id":"R1","state":"APPROVED","commit":{"oid":"h1"},
                  "author":{"__typename":"User","login":"bob"},
                  "onBehalfOf":{"nodes":[]}}]
      }
    }
  }
}}"#;

fn files_page(paths: &[&str], next: Option<&str>, changed: u64) -> String {
    let nodes: Vec<_> = paths
        .iter()
        .map(|path| serde_json::json!({ "path": path }))
        .collect();
    serde_json::json!({"data": {"repository": {"pullRequest": {
        "changedFiles": changed,
        "files": {
            "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next},
            "nodes": nodes
        }
    }}}})
    .to_string()
}

fn mock_github(protection: ResponseTemplate, changed: u64) -> MockServer {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query OwnershipProbe"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PROBE))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query OwnershipFiles"))
            .and(body_string_contains(r#""cursor":null"#))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(files_page(&["src/lib.rs"], Some("c1"), changed)),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query OwnershipFiles"))
            .and(body_string_contains(r#""cursor":"c1""#))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(files_page(&["docs/readme.md"], None, changed)),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/rules/branches/main"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"type":"deletion"},{"type":"pull_request","parameters":{"require_code_owner_review":true}}]"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/repos/acme/widgets/branches/main/protection/required_pull_request_reviews",
            ))
            .respond_with(protection)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/orgs/acme/teams/core/members"))
            .and(query_param("page", "1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"[{"login":"bob"},{"login":"carol"}]"#),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/orgs/acme/teams/secret/members"))
            .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"message":"Not Found"}"#))
            .mount(&server)
            .await;
        server
    })
}

#[test]
fn probe_and_inputs_read_codeowners_files_rules_and_protection() {
    let server = mock_github(
        ResponseTemplate::new(200).set_body_string(r#"{"require_code_owner_reviews":false}"#),
        2,
    );
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let pr = pr();
    let probe = client.ownership_probe(&pr).unwrap();
    assert_eq!(probe.head_sha, "h1");
    assert_eq!(
        probe.codeowners.as_ref().unwrap().path,
        ".github/CODEOWNERS"
    );
    assert_eq!(probe.reviews.reviews[0].author_login, "bob");
    assert!(!probe.viewer_is_admin);

    let inputs = client.ownership_inputs(&pr, &probe).unwrap();
    assert_eq!(inputs.files, vec!["src/lib.rs", "docs/readme.md"]);
    assert!(!inputs.files_truncated);
    assert_eq!(inputs.rules_require_code_owner, Some(true));
    assert_eq!(
        inputs.protection,
        ClassicProtection::Present {
            require_code_owner_reviews: false
        }
    );
}

#[test]
fn hidden_protection_is_unknown_and_missing_files_are_truncation() {
    let server = mock_github(
        ResponseTemplate::new(404).set_body_string(r#"{"message":"Not Found"}"#),
        5,
    );
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let pr = pr();
    let probe = client.ownership_probe(&pr).unwrap();
    let inputs = client.ownership_inputs(&pr, &probe).unwrap();
    // A non-admin cannot tell "no protection" from "hidden": §6 says Unknown.
    assert!(matches!(
        inputs.protection,
        ClassicProtection::Unknown { .. }
    ));
    // GitHub reports 5 changed files but only 2 came back.
    assert!(inputs.files_truncated);
}

#[test]
fn team_members_read_pages_and_unreadable_teams_are_incomplete() {
    let server = mock_github(ResponseTemplate::new(403), 1);
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let core = client
        .team_members(&TeamRef {
            org: "acme".into(),
            slug: "core".into(),
        })
        .unwrap();
    assert!(core.complete);
    assert_eq!(core.logins, vec!["bob", "carol"]);
    let secret = client
        .team_members(&TeamRef {
            org: "acme".into(),
            slug: "secret".into(),
        })
        .unwrap();
    assert!(!secret.complete);
}

#[test]
fn a_rate_limited_rules_call_stops_inputs() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(files_page(&["a"], None, 1)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/rules/branches/main"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "60")
                    .set_body_string(r#"{"message":"API rate limit exceeded"}"#),
            )
            .mount(&server)
            .await;
        server
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let probe = pr_nexus_core::providers::OwnershipProbe {
        head_sha: "h1".into(),
        base_ref: "main".into(),
        codeowners: None,
        reviews: pr_nexus_core::review::owners::ReviewBundle {
            fetched: true,
            complete: true,
            reviews: Vec::new(),
        },
        viewer_is_admin: false,
    };
    assert!(matches!(
        client.ownership_inputs(&pr(), &probe),
        Err(ProviderError::RateLimited { .. })
    ));
}
