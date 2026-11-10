//! Detail-panel overview and auto-merge against a mock GitHub (§7.2).

use chrono::{TimeZone, Utc};
use pr_nexus_core::branch::{BranchRef, UpdateMethod};
use pr_nexus_core::domain::{
    AccountId, ChecksState, Involvement, MergeStateStatus, Mergeable, PrId, PrState, ProviderRef,
    PullRequest, RepoRef,
};
use pr_nexus_core::overview::{CheckState, MergeMethod, PersonKind, ReviewState};
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

const OVERVIEW: &str = r#"{"data":{
  "rateLimit":{"limit":5000,"remaining":4990,"resetAt":"2026-10-26T13:00:00Z","cost":1},
  "viewer":{"login":"alice"},
  "repository":{
    "viewerPermission":"WRITE",
    "autoMergeAllowed":true,
    "squashMergeAllowed":true,
    "mergeCommitAllowed":false,
    "rebaseMergeAllowed":true,
    "pullRequest":{
      "state":"OPEN","isDraft":false,"baseRefName":"main","headRefName":"latch",
      "headRefOid":"h1","baseRefOid":"b1","headRepositoryOwner":{"login":"alice"},
      "mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","reviewDecision":"REVIEW_REQUIRED",
      "viewerCanMergeAsAdmin":false,"viewerCanEnableAutoMerge":true,"viewerCanDisableAutoMerge":false,"viewerCanClose":true,
      "autoMergeRequest":null,
      "comments":{"totalCount":3},
      "reviewThreads":{"nodes":[{"comments":{"totalCount":2}},{"comments":{"totalCount":1}}]},
      "author":{"__typename":"User","login":"alice","name":"Alice","avatarUrl":"https://example.test/a"},
      "assignees":{"nodes":[
        {"login":"bob","name":"","avatarUrl":"https://example.test/b"}
      ]},
      "reviewRequests":{"nodes":[
        {"requestedReviewer":{"__typename":"Team","slug":"core","name":"Core","avatarUrl":null,"organization":{"login":"acme"}}}
      ]},
      "latestReviews":{"nodes":[
        {"state":"APPROVED","author":{"__typename":"User","login":"bob","name":"Bob","avatarUrl":"https://example.test/b"}}
      ]},
      "commits":{"nodes":[{"commit":{"statusCheckRollup":{"contexts":{"totalCount":3,"nodes":[
        {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://example.test/run/1","isRequired":true},
        {"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":null,"detailsUrl":null,"isRequired":false},
        {"__typename":"StatusContext","context":"deploy/preview","state":"FAILURE","targetUrl":"https://example.test/s","isRequired":false}
      ]}}}}]}
    }
  }
}}"#;

fn mock_github() -> MockServer {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query PullRequestOverview"))
            .respond_with(ResponseTemplate::new(200).set_body_string(OVERVIEW))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/rules/branches/main"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"type":"pull_request","parameters":{"required_approving_review_count":2}}]"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation ConvertToDraft"))
            .and(body_string_contains(r#""id":"PR_1""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"convertPullRequestToDraft":{"pullRequest":{"id":"PR_1","isDraft":true}}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation MarkReady"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":null,"errors":[{"type":"UNPROCESSABLE","message":"Draft pull requests are not supported in this repository."}]}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation EnableAutoMerge"))
            .and(body_string_contains(r#""method":"SQUASH""#))
            .and(body_string_contains(r#""id":"PR_1""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"enablePullRequestAutoMerge":{"pullRequest":{"id":"PR_1"}}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation MergePullRequest"))
            .and(body_string_contains(r#""method":"SQUASH""#))
            .and(body_string_contains(r#""id":"PR_1""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"mergePullRequest":{"pullRequest":{"id":"PR_1","state":"MERGED"}}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation MergePullRequest"))
            .and(body_string_contains(r#""method":"REBASE""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":null,"errors":[{"type":"UNPROCESSABLE","message":"Pull Request is not mergeable"}]}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/compare/acme:main...alice:feat%2Flatch"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"ahead_by":2,"behind_by":4,"commits":[{"author":{"login":"alice"}},{"author":{"login":"bob"}}]}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation UpdatePullRequestBranch"))
            .and(body_string_contains(r#""expectedHeadOid":"h1""#))
            .and(body_string_contains(r#""method":"MERGE""#))
            .and(body_string_contains(r#""id":"PR_1""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"updatePullRequestBranch":{"pullRequest":{"id":"PR_1","headRefOid":"h2"}}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation UpdatePullRequestBranch"))
            .and(body_string_contains(r#""method":"REBASE""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":null,"errors":[{"type":"UNPROCESSABLE","message":"expected head sha didn't match current head ref"}]}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation DisableAutoMerge"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":null,"errors":[{"type":"FORBIDDEN","message":"Viewer is not allowed"}]}"#,
            ))
            .mount(&server)
            .await;
        server
    })
}

#[test]
fn overview_reads_checks_comments_people_and_merge_rights() {
    let server = mock_github();
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let overview = client.pull_request_overview(&pr()).unwrap();
    assert_eq!(overview.merge_state, "blocked");
    assert_eq!(overview.mergeable, "mergeable");
    assert_eq!(overview.review_decision.as_deref(), Some("review_required"));
    assert_eq!((overview.issue_comments, overview.review_comments), (3, 3));
    assert_eq!(overview.checks_total, 3);
    let states: Vec<_> = overview.checks.iter().map(|check| check.state).collect();
    assert_eq!(
        states,
        [CheckState::Passed, CheckState::Pending, CheckState::Failed]
    );
    assert!(overview.checks[0].required);
    assert_eq!(overview.approvals, 1);
    assert_eq!(overview.required_approvals, Some(2));
    assert_eq!(overview.reviewers.len(), 2);
    assert_eq!(overview.reviewers[1].person.kind, PersonKind::Team);
    assert_eq!(overview.reviewers[1].person.login, "acme/core");
    assert_eq!(overview.reviewers[1].state, ReviewState::Pending);
    assert_eq!(
        overview.author.as_ref().map(|a| a.login.as_str()),
        Some("alice")
    );
    assert_eq!(overview.assignees.len(), 1);
    assert_eq!(overview.assignees[0].login, "bob");
    assert_eq!(overview.assignees[0].name, None);
    assert_eq!(overview.viewer.login, "alice");
    assert_eq!(overview.viewer.permission.as_deref(), Some("write"));
    assert!(overview.viewer.can_enable_auto_merge);
    assert_eq!(overview.viewer.can_close, Some(true));
    assert!(overview.merge_settings.auto_merge_allowed);
    assert!(!overview.merge_settings.merge);
    assert!(overview.auto_merge.is_none());
}

#[test]
fn auto_merge_mutations_send_the_node_id_and_surface_errors() {
    let server = mock_github();
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    client
        .set_auto_merge(&pr(), Some(MergeMethod::Squash))
        .unwrap();
    let err = client.set_auto_merge(&pr(), None).unwrap_err();
    assert!(err.to_string().contains("not allowed"), "{err}");
}

#[test]
fn draft_mutations_send_the_node_id_and_explain_plan_limits() {
    let server = mock_github();
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    client.set_draft(&pr(), true).unwrap();
    let err = client.set_draft(&pr(), false).unwrap_err();
    assert!(
        matches!(err, pr_nexus_core::error::ProviderError::Unsupported(_)),
        "{err}"
    );
}

#[test]
fn merge_sends_the_node_id_and_method_and_surfaces_errors() {
    let server = mock_github();
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    client
        .merge_pull_request(&pr(), MergeMethod::Squash)
        .unwrap();
    let err = client
        .merge_pull_request(&pr(), MergeMethod::Rebase)
        .unwrap_err();
    assert!(err.to_string().contains("not mergeable"), "{err}");
}

#[test]
fn compare_qualifies_both_owners_for_forks() {
    let server = mock_github();
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let overview = client.pull_request_overview(&pr()).unwrap();
    assert_eq!(
        (overview.head_sha.as_str(), overview.base_sha.as_str()),
        ("h1", "b1")
    );
    assert_eq!(overview.head_owner, "alice");
    let base = BranchRef {
        owner: "acme".into(),
        name: "main".into(),
    };
    let head = BranchRef {
        owner: "alice".into(),
        name: "feat/latch".into(),
    };
    let found = client.compare_branches(&pr(), &base, &head).unwrap();
    assert_eq!((found.behind_by, found.ahead_by), (4, 2));
    assert_eq!(found.authors, vec!["alice", "bob"]);
}

#[test]
fn close_posts_the_comment_then_closes_without_deleting_the_branch() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation AddPullRequestComment"))
            .and(body_string_contains(r#""body":"Please close this.""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"addComment":{"commentEdge":{"node":{"id":"IC_1"}}}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation ClosePullRequest"))
            .and(body_string_contains(r#""id":"PR_1""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"closePullRequest":{"pullRequest":{"id":"PR_1","state":"CLOSED"}}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation ClosePullRequest"))
            .and(body_string_contains(r#""id":"PR_denied""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":null,"errors":[{"type":"FORBIDDEN","message":"Resource not accessible by integration"}]}"#,
            ))
            .mount(&server)
            .await;
        server
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    client
        .add_pull_request_comment(&pr(), "Please close this.")
        .unwrap();
    client.close_pull_request(&pr()).unwrap();

    let mut denied = pr();
    denied.remote.id = "PR_denied".into();
    let err = client.close_pull_request(&denied).unwrap_err();
    assert_eq!(
        err.to_string(),
        "forbidden: You don't have permission to close this pull request."
    );

    let requests = runtime.block_on(server.received_requests()).unwrap();
    assert!(requests.iter().all(|request| request.method == "POST"));
    assert!(
        requests
            .iter()
            .all(|request| request.url.path() == "/graphql")
    );
    let bodies: Vec<String> = requests
        .iter()
        .map(|request| String::from_utf8(request.body.clone()).unwrap())
        .collect();
    assert!(bodies.iter().all(|body| {
        let lower = body.to_ascii_lowercase();
        !lower.contains("delete") && !lower.contains("merge")
    }));
    let comment_at = bodies
        .iter()
        .position(|body| body.contains("AddPullRequestComment"))
        .unwrap();
    let close_at = bodies
        .iter()
        .position(|body| body.contains("ClosePullRequest"))
        .unwrap();
    assert!(comment_at < close_at);
}

#[test]
fn close_without_a_comment_sends_only_the_close_mutation() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation ClosePullRequest"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"closePullRequest":{"pullRequest":{"id":"PR_1","state":"CLOSED"}}}}"#,
            ))
            .mount(&server)
            .await;
        server
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    client.close_pull_request(&pr()).unwrap();
    let requests = runtime.block_on(server.received_requests()).unwrap();
    assert_eq!(requests.len(), 1);
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(body.contains("ClosePullRequest"));
    assert!(!body.contains("AddPullRequestComment"));
    assert!(!body.to_ascii_lowercase().contains("delete"));
}

#[test]
fn close_graphql_does_not_delete_a_branch_or_merge() {
    let close = include_str!("../src/github/graphql/close_pull_request.graphql");
    let comment = include_str!("../src/github/graphql/add_comment.graphql");
    for source in [close, comment] {
        let lower = source.to_ascii_lowercase();
        assert!(!lower.contains("delete"), "{source}");
        assert!(!lower.contains("merge"), "{source}");
    }
}

#[test]
fn enqueue_sends_the_node_id_and_expected_head() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation EnqueuePullRequest"))
            .and(body_string_contains(r#""id":"PR_1""#))
            .and(body_string_contains(r#""expectedHeadOid":"h1""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"enqueuePullRequest":{"mergeQueueEntry":{"position":2,"state":"QUEUED","estimatedTimeToMerge":300}}}}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation DequeuePullRequest"))
            .and(body_string_contains(r#""id":"PR_1""#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"dequeuePullRequest":{"mergeQueueEntry":null}}}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        server
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let queued = client.enqueue_pull_request(&pr(), "h1").unwrap();
    assert!(queued.required);
    assert_eq!(
        queued.phase,
        Some(pr_nexus_core::merge_queue::MergeQueuePhase::Queued)
    );
    assert_eq!(queued.position, Some(2));
    assert_eq!(queued.eta_seconds, Some(300));
    client.dequeue_pull_request(&pr()).unwrap();
}

#[test]
fn update_branch_sends_expected_head_and_method_and_surfaces_errors() {
    let server = mock_github();
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    client
        .update_branch(&pr(), "h1", UpdateMethod::Merge)
        .unwrap();
    let err = client
        .update_branch(&pr(), "h1", UpdateMethod::Rebase)
        .unwrap_err();
    assert!(err.to_string().contains("expected head"), "{err}");
}
