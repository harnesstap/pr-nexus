//! The notifications inbox and the targeted pull request read (§6 live
//! updates), against a mock server. Nothing here may write to GitHub.

use pr_nexus_core::domain::AccountId;
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{NotificationsPage, NotificationsRequest, VcsProvider};
use pr_nexus_providers::github::{GithubClient, SCOPE_UNAVAILABLE, TOKEN_TYPE_UNAVAILABLE};
use serde_json::{Value, json};
use wiremock::matchers::{header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("tokio")
}

fn server() -> MockServer {
    runtime().block_on(MockServer::start())
}

fn mount(server: &MockServer, mock: Mock) {
    runtime().block_on(mock.mount(server));
}

fn requests(server: &MockServer) -> Vec<wiremock::Request> {
    runtime()
        .block_on(server.received_requests())
        .unwrap_or_default()
}

fn client(server: &MockServer, token: &str) -> GithubClient {
    GithubClient::with_base(server.uri(), AccountId(1), token).unwrap()
}

fn threads() -> Value {
    json!([
        {
            "id": "1",
            "reason": "review_requested",
            "unread": true,
            "updated_at": "2024-01-02T03:04:05Z",
            "subject": {
                "title": "Add widgets",
                "url": "https://api.github.com/repos/acme/widgets/pulls/12",
                "type": "PullRequest"
            },
            "repository": { "full_name": "acme/widgets" }
        },
        {
            "id": "2",
            "reason": "mention",
            "unread": false,
            "updated_at": "2024-01-02T03:00:00Z",
            "subject": {
                "title": "A question",
                "url": "https://api.github.com/repos/acme/widgets/issues/3",
                "type": "Issue"
            }
        }
    ])
}

fn first_page() -> NotificationsRequest {
    NotificationsRequest {
        if_modified_since: None,
        page: 1,
    }
}

fn assert_read_only(server: &MockServer) {
    for request in requests(server) {
        let verb = request.method.to_string();
        assert!(
            verb == "GET" || (verb == "POST" && request.url.path() == "/graphql"),
            "unexpected {verb} {}",
            request.url
        );
    }
}

#[test]
fn a_changed_inbox_reads_headers_and_threads() {
    let server = server();
    mount(
        &server,
        Mock::given(method("GET"))
            .and(path("/notifications"))
            .and(query_param("all", "true"))
            .and(query_param("participating", "false"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Last-Modified", "Tue, 02 Jan 2024 03:04:05 GMT")
                    .insert_header("X-Poll-Interval", "90")
                    .insert_header("X-OAuth-Scopes", "repo, read:org")
                    .insert_header(
                        "Link",
                        "<https://api.github.com/notifications?page=2>; rel=\"next\"",
                    )
                    .set_body_json(threads()),
            ),
    );
    let page = client(&server, "gho_token")
        .notifications(&first_page())
        .unwrap();
    let NotificationsPage::Changed {
        last_modified,
        poll_interval,
        items,
        has_next,
    } = page
    else {
        panic!("expected a changed inbox");
    };
    assert_eq!(
        last_modified.as_deref(),
        Some("Tue, 02 Jan 2024 03:04:05 GMT")
    );
    assert_eq!(poll_interval, Some(90));
    assert!(has_next);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].subject_type, "PullRequest");
    assert_eq!(items[0].reason, "review_requested");
    assert_eq!(
        items[0].subject_url.as_deref(),
        Some("https://api.github.com/repos/acme/widgets/pulls/12")
    );
    assert_read_only(&server);
}

#[test]
fn if_modified_since_is_sent_and_a_304_is_not_modified() {
    let server = server();
    mount(
        &server,
        Mock::given(method("GET"))
            .and(path("/notifications"))
            .and(header_exists("If-Modified-Since"))
            .respond_with(ResponseTemplate::new(304).insert_header("X-Poll-Interval", "60")),
    );
    let page = client(&server, "ghp_token")
        .notifications(&NotificationsRequest {
            if_modified_since: Some("Tue, 02 Jan 2024 03:04:05 GMT".into()),
            page: 1,
        })
        .unwrap();
    assert_eq!(
        page,
        NotificationsPage::NotModified {
            poll_interval: Some(60)
        }
    );
    let sent = requests(&server);
    assert_eq!(
        sent[0].headers.get("if-modified-since").unwrap(),
        "Tue, 02 Jan 2024 03:04:05 GMT"
    );
    assert_read_only(&server);
}

#[test]
fn fine_grained_and_app_tokens_are_refused_without_a_request() {
    let server = server();
    for token in ["github_pat_abc", "ghs_abc"] {
        let err = client(&server, token)
            .notifications(&first_page())
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Unsupported(ref reason) if reason == TOKEN_TYPE_UNAVAILABLE),
            "{err:?}"
        );
    }
    assert!(requests(&server).is_empty());
}

#[test]
fn forbidden_or_not_found_means_unavailable_with_the_reason() {
    let server = server();
    mount(
        &server,
        Mock::given(method("GET"))
            .and(path("/notifications"))
            .respond_with(
                ResponseTemplate::new(404)
                    .insert_header("X-OAuth-Scopes", "read:org, public_repo")
                    .set_body_json(json!({ "message": "Not Found" })),
            ),
    );
    let err = client(&server, "ghp_token")
        .notifications(&first_page())
        .unwrap_err();
    assert!(
        matches!(err, ProviderError::Unsupported(ref reason) if reason == SCOPE_UNAVAILABLE),
        "{err:?}"
    );

    let server = self::server();
    mount(
        &server,
        Mock::given(method("GET"))
            .and(path("/notifications"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(json!({ "message": "Resource not accessible by integration" })),
            ),
    );
    let err = client(&server, "gho_token")
        .notifications(&first_page())
        .unwrap_err();
    assert!(
        matches!(err, ProviderError::Unsupported(ref reason) if reason == TOKEN_TYPE_UNAVAILABLE),
        "{err:?}"
    );
    // One request, no retry.
    assert_eq!(requests(&server).len(), 1);
}

#[test]
fn a_secondary_rate_limit_is_reported_not_retried() {
    let server = server();
    mount(
        &server,
        Mock::given(method("GET"))
            .and(path("/notifications"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("Retry-After", "120")
                    .set_body_json(
                        json!({ "message": "You have exceeded a secondary rate limit" }),
                    ),
            ),
    );
    let err = client(&server, "gho_token")
        .notifications(&first_page())
        .unwrap_err();
    assert!(
        matches!(
            err,
            ProviderError::RateLimited {
                secondary: true,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(requests(&server).len(), 1);
}

#[test]
fn one_pull_request_is_read_by_number_in_one_query() {
    let server = server();
    let search: Value = serde_json::from_str(include_str!("fixtures/search-page.json")).unwrap();
    let node = search["data"]["search"]["nodes"][0].clone();
    let body = json!({
        "data": {
            "rateLimit": search["data"]["rateLimit"].clone(),
            "repository": { "pullRequest": node }
        }
    });
    mount(
        &server,
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body)),
    );
    let item = client(&server, "gho_token")
        .pull_request_by_number("acme", "widgets", 12)
        .unwrap()
        .unwrap();
    assert_eq!(item.pull_request.number, 12);
    let sent = requests(&server);
    assert_eq!(sent.len(), 1);
    let sent: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(
        sent["variables"],
        json!({ "owner": "acme", "name": "widgets", "number": 12 })
    );
    assert!(
        sent["query"]
            .as_str()
            .unwrap()
            .contains("fragment PrFields")
    );
    assert_read_only(&server);
}

#[test]
fn a_missing_pull_request_is_none() {
    let server = server();
    mount(
        &server,
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "repository": { "pullRequest": null } }
            }))),
    );
    let item = client(&server, "gho_token")
        .pull_request_by_number("acme", "widgets", 404)
        .unwrap();
    assert!(item.is_none());
}
