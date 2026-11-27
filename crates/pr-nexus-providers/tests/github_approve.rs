//! Batch approval reads and the approving review against a mock GitHub (§8.2 Approve pull requests).

use chrono::{TimeZone, Utc};
use pr_nexus_core::domain::{
    AccountId, ChecksState, Involvement, MergeStateStatus, Mergeable, PrId, PrState, ProviderRef,
    PullRequest, RepoRef,
};
use pr_nexus_core::providers::VcsProvider;
use pr_nexus_providers::github::GithubClient;
use wiremock::matchers::{body_string_contains, method, path};
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
        assignees: Vec::new(),
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

#[test]
fn probes_read_state_head_author_and_the_viewers_latest_review_in_one_query() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query ApprovalProbes"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"viewer":{"login":"bob"},"nodes":[
                  {"id":"PR_1","state":"OPEN","isDraft":true,"headRefOid":"h2","author":{"login":"alice"},
                   "viewerLatestReview":{"state":"APPROVED","commit":{"oid":"h1"}}},
                  {"id":"PR_2","state":"MERGED","isDraft":false,"headRefOid":"h9","author":null,"viewerLatestReview":null},
                  null
                ]},"errors":[{"type":"NOT_FOUND","path":["nodes",2],"message":"Could not resolve to a node"}]}"#,
            ))
            .mount(&server)
            .await;
        server
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let mut second = pr();
    second.remote.id = "PR_2".into();
    let mut gone = pr();
    gone.remote.id = "PR_gone".into();
    let probes = client.approval_probes(&[pr(), second, gone]).unwrap();
    assert_eq!(probes.viewer_login, "bob");
    assert_eq!(probes.probes.len(), 2);
    let first = &probes.probes[0];
    assert_eq!(first.node_id, "PR_1");
    assert_eq!(first.state, PrState::Open);
    assert!(first.is_draft);
    assert_eq!(first.head_oid, "h2");
    assert_eq!(first.author_login, "alice");
    assert_eq!(first.viewer_review_state.as_deref(), Some("APPROVED"));
    assert_eq!(first.viewer_review_commit.as_deref(), Some("h1"));
    assert_eq!(probes.probes[1].state, PrState::Merged);
    assert_eq!(probes.probes[1].viewer_review_state, None);

    let requests = runtime.block_on(server.received_requests()).unwrap();
    assert_eq!(requests.len(), 1);
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(body.contains(r#""ids":["PR_1","PR_2","PR_gone"]"#));
    assert!(!body.contains("mutation"));
}

#[test]
fn approve_pins_the_commit_and_sends_no_body_when_blank() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation ApprovePullRequest"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"addPullRequestReview":{"pullRequestReview":{"id":"PRR_1","state":"APPROVED"}}}}"#,
            ))
            .mount(&server)
            .await;
        server
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    client.approve_pull_request(&pr(), "h2", "").unwrap();
    client
        .approve_pull_request(&pr(), "h2", "Looks good")
        .unwrap();
    let requests = runtime.block_on(server.received_requests()).unwrap();
    let bodies: Vec<String> = requests
        .iter()
        .map(|request| String::from_utf8(request.body.clone()).unwrap())
        .collect();
    assert!(bodies[0].contains(r#""commit":"h2""#));
    assert!(bodies[0].contains(r#""id":"PR_1""#));
    assert!(bodies[0].contains(r#""body":null"#));
    assert!(bodies[1].contains(r#""body":"Looks good""#));
    for body in &bodies {
        assert!(body.contains("event: APPROVE"));
        assert!(!body.to_ascii_lowercase().contains("merge"));
    }
}

#[test]
fn a_refused_approval_is_an_error() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"addPullRequestReview":null},"errors":[{"type":"UNPROCESSABLE","path":["addPullRequestReview"],"message":"Can not approve your own pull request"}]}"#,
            ))
            .mount(&server)
            .await;
        server
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let err = client.approve_pull_request(&pr(), "h2", "").unwrap_err();
    assert!(
        err.to_string()
            .contains("Can not approve your own pull request")
    );
}
