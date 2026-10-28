//! Detail-panel overview and auto-merge against a mock GitHub (§7.2).

use chrono::{TimeZone, Utc};
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
      "mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","reviewDecision":"REVIEW_REQUIRED",
      "viewerCanMergeAsAdmin":false,"viewerCanEnableAutoMerge":true,"viewerCanDisableAutoMerge":false,
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
