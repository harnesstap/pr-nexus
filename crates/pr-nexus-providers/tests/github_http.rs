use std::time::Duration;

use pr_nexus_core::domain::{AccountId, Involvement, ProviderRef, VcsKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{PrQuery, SearchBucket, VcsProvider};
use pr_nexus_providers::github::{
    self, DevicePoll, GithubClient, poll_device_flow, start_device_flow,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn search_body() -> String {
    include_str!("fixtures/search-page.json").to_owned()
}

fn server() -> MockServer {
    let runtime = tokio::runtime::Runtime::new().expect("tokio");
    runtime.block_on(MockServer::start())
}

fn mount(server: &MockServer, template: ResponseTemplate) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio");
    runtime.block_on(async {
        Mock::given(method("POST"))
            .respond_with(template)
            .mount(server)
            .await;
    });
}

#[test]
fn search_page_parses_a_pull_request() {
    let server = server();
    mount(
        &server,
        ResponseTemplate::new(200).set_body_string(search_body()),
    );
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let rows = client
        .search_pull_requests(&PrQuery {
            bucket: SearchBucket::Mine,
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    let pr = &rows[0].pull_request;
    assert_eq!(pr.repo.owner, "acme");
    assert_eq!(pr.number, 12);
    assert_eq!(pr.remote.id, "PR_node_12");
    assert!(pr.involvement.contains(Involvement::AUTHOR));
    assert_eq!(pr.unresolved_threads, 1);
    assert_eq!(pr.total_threads, 2);
    assert_eq!(rows[0].default_branch.as_deref(), Some("main"));
    let budget = client.rate_limit().unwrap();
    assert_eq!(budget.remaining, 4900);
    assert_eq!(client.kind(), VcsKind::GitHub);
}

#[test]
fn maps_401_sso_secondary_and_404() {
    let unauthorized = server();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"message":"Bad credentials"}"#),
            )
            .mount(&unauthorized)
            .await;
    });
    drop(runtime);
    let client = GithubClient::with_base(unauthorized.uri(), AccountId(1), "nope").unwrap();
    assert!(matches!(
        client.viewer().unwrap_err(),
        ProviderError::Unauthorized(_)
    ));

    let sso = server();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header(
                        "x-github-sso",
                        "required; url=https://github.com/orgs/acme/sso?authorization_request=1",
                    )
                    .set_body_string(
                        r#"{"message":"Resource protected by organization SAML enforcement."}"#,
                    ),
            )
            .mount(&sso)
            .await;
    });
    drop(runtime);
    let client = GithubClient::with_base(sso.uri(), AccountId(1), "token").unwrap();
    match client.viewer().unwrap_err() {
        ProviderError::Forbidden(message) => {
            assert!(message.contains("https://github.com/orgs/acme/sso"))
        }
        other => panic!("{other:?}"),
    }

    let secondary = server();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("retry-after", "45")
                    .set_body_string(r#"{"message":"You have exceeded a secondary rate limit."}"#),
            )
            .mount(&secondary)
            .await;
    });
    drop(runtime);
    let client = GithubClient::with_base(secondary.uri(), AccountId(1), "token").unwrap();
    assert!(matches!(
        client.viewer().unwrap_err(),
        ProviderError::RateLimited {
            secondary: true,
            ..
        }
    ));

    let missing = server();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"message":"Not Found"}"#))
            .mount(&missing)
            .await;
    });
    drop(runtime);
    let client = GithubClient::with_base(missing.uri(), AccountId(1), "token").unwrap();
    assert!(matches!(
        client.viewer().unwrap_err(),
        ProviderError::NotFound(_)
    ));
}

#[test]
fn device_flow_start_and_poll() {
    let server = server();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/login/device/code"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"device_code":"dev","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","expires_in":900,"interval":5}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"gho_test","token_type":"bearer","scope":"repo"}"#,
            ))
            .mount(&server)
            .await;
    });
    drop(runtime);
    let http = reqwest::blocking::Client::builder().build().unwrap();
    let start = start_device_flow(&http, &server.uri(), "client").unwrap();
    assert_eq!(start.user_code, "ABCD-1234");
    match poll_device_flow(&http, &server.uri(), "client", &start.device_code).unwrap() {
        DevicePoll::Token(token) => assert_eq!(token, "gho_test"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn viewer_adds_orgs_hidden_from_graphql_as_needing_sso() {
    let server = server();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"rateLimit":{"limit":5000,"remaining":4999,"resetAt":"2026-10-26T01:00:00Z","cost":1},
                   "viewer":{"login":"ada","id":"U_1","organizations":{"nodes":[{"login":"acme"}]}}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/user/orgs"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"login":"acme"},{"login":"acme-sso"},{"login":"other-sso"}]"#,
            ))
            .mount(&server)
            .await;
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let viewer = client.viewer().unwrap();
    assert_eq!(viewer.organizations, vec!["acme", "acme-sso", "other-sso"]);
    assert_eq!(
        viewer.organizations_needing_sso,
        vec!["acme-sso", "other-sso"]
    );
}

#[test]
fn viewer_keeps_graphql_orgs_when_rest_fails() {
    let server = server();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":{"viewer":{"login":"ada","id":"U_1","organizations":{"nodes":[{"login":"acme"}]}}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/user/orgs"))
            .respond_with(ResponseTemplate::new(403).set_body_string(r#"{"message":"forbidden"}"#))
            .mount(&server)
            .await;
    });
    let client = GithubClient::with_base(server.uri(), AccountId(1), "token").unwrap();
    let viewer = client.viewer().unwrap();
    assert_eq!(viewer.organizations, vec!["acme"]);
    assert!(viewer.organizations_needing_sso.is_empty());
}

const GATEWAY_PAGE: &str = "<html><head><title>502 Bad Gateway</title></head></html>";

fn graphql_variables(request: &wiremock::Request) -> serde_json::Value {
    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("graphql body");
    body["variables"].clone()
}

fn sent_variables(server: &MockServer) -> Vec<serde_json::Value> {
    let runtime = tokio::runtime::Runtime::new().expect("tokio");
    runtime
        .block_on(server.received_requests())
        .expect("recorded requests")
        .iter()
        .map(graphql_variables)
        .collect()
}

/// Answers the gateway page for requests `slow` picks and `ok` for the rest.
fn mount_slow(
    server: &MockServer,
    slow: impl Fn(&serde_json::Value) -> bool + Send + Sync + 'static,
    ok: ResponseTemplate,
) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio");
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(move |request: &wiremock::Request| slow(&graphql_variables(request)))
            .respond_with(ResponseTemplate::new(502).set_body_string(GATEWAY_PAGE))
            .with_priority(1)
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ok)
            .with_priority(2)
            .mount(server)
            .await;
    });
}

fn fast_client(server: &MockServer) -> GithubClient {
    GithubClient::with_base(server.uri(), AccountId(1), "token")
        .unwrap()
        .with_backoff(Duration::ZERO)
}

#[test]
fn search_retries_a_gateway_timeout_with_a_smaller_page() {
    let server = server();
    mount_slow(
        &server,
        |vars| vars["first"] == 20,
        ResponseTemplate::new(200).set_body_string(search_body()),
    );
    let rows = fast_client(&server)
        .search_pull_requests(&PrQuery {
            bucket: SearchBucket::Mine,
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    let firsts: Vec<_> = sent_variables(&server)
        .iter()
        .map(|vars| vars["first"].clone())
        .collect();
    assert_eq!(firsts, vec![json!(20), json!(10)]);
}

#[test]
fn search_gives_up_after_three_timeouts_and_names_the_query() {
    let server = server();
    mount_slow(
        &server,
        |_| true,
        ResponseTemplate::new(200).set_body_string(search_body()),
    );
    let err = fast_client(&server)
        .search_pull_requests(&PrQuery {
            bucket: SearchBucket::ReviewRequested,
        })
        .unwrap_err();
    match err {
        ProviderError::Api { code, message } => {
            assert_eq!(code, "502");
            assert!(message.contains("timed out"), "{message}");
            assert!(message.contains("review requests"), "{message}");
            assert!(message.contains("3 tries"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    let firsts: Vec<_> = sent_variables(&server)
        .iter()
        .map(|vars| vars["first"].clone())
        .collect();
    assert_eq!(firsts, vec![json!(20), json!(10), json!(5)]);
}

#[test]
fn one_page_never_waits_more_than_a_minute_when_github_hangs() {
    assert_eq!(github::REQUEST_TIMEOUT, Duration::from_secs(15));
    assert_eq!(github::worst_case_page_time(), Duration::from_secs(48));
    assert!(github::worst_case_page_time() <= Duration::from_secs(60));
}

#[test]
fn a_graphql_timeout_error_is_retried_like_a_gateway_timeout() {
    let server = server();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":null,"errors":[{"message":"Something went wrong while executing your query. This may be the result of a timeout, or it could be a GitHub bug."}]}"#,
            ))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(search_body()))
            .with_priority(2)
            .mount(&server)
            .await;
    });
    drop(runtime);
    let rows = fast_client(&server)
        .search_pull_requests(&PrQuery {
            bucket: SearchBucket::Mine,
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(sent_variables(&server).len(), 2);
}

#[test]
fn other_server_errors_are_not_retried() {
    let server = server();
    mount(
        &server,
        ResponseTemplate::new(500).set_body_string(r#"{"message":"boom"}"#),
    );
    let err = fast_client(&server)
        .search_pull_requests(&PrQuery {
            bucket: SearchBucket::Mine,
        })
        .unwrap_err();
    assert!(matches!(err, ProviderError::Api { ref code, .. } if code == "500"));
    assert_eq!(sent_variables(&server).len(), 1);
}

#[test]
fn refresh_shrinks_its_chunks_after_a_gateway_timeout() {
    let fixture: serde_json::Value = serde_json::from_str(&search_body()).unwrap();
    let node = fixture["data"]["search"]["nodes"][0].clone();
    let server = server();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(|request: &wiremock::Request| {
                graphql_variables(request)["ids"]
                    .as_array()
                    .is_some_and(|ids| ids.len() > 10)
            })
            .respond_with(ResponseTemplate::new(504).set_body_string(GATEWAY_PAGE))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(move |request: &wiremock::Request| {
                let count = graphql_variables(request)["ids"]
                    .as_array()
                    .map_or(0, Vec::len);
                ResponseTemplate::new(200).set_body_json(json!({
                    "data": {
                        "rateLimit": fixture["data"]["rateLimit"].clone(),
                        "nodes": vec![node.clone(); count],
                    }
                }))
            })
            .with_priority(2)
            .mount(&server)
            .await;
    });
    drop(runtime);
    let ids: Vec<ProviderRef> = (0..25)
        .map(|n| ProviderRef {
            account: AccountId(1),
            id: format!("PR_node_{n}"),
        })
        .collect();
    let rows = fast_client(&server).refresh_pull_requests(&ids).unwrap();
    assert_eq!(rows.len(), 25);
    let sizes: Vec<_> = sent_variables(&server)
        .iter()
        .map(|vars| vars["ids"].as_array().map_or(0, Vec::len))
        .collect();
    assert_eq!(sizes, vec![20, 10, 10, 5]);
}
