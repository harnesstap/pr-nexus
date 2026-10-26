use pr_nexus_core::domain::{AccountId, Involvement, VcsKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{PrQuery, SearchBucket, VcsProvider};
use pr_nexus_providers::github::{DevicePoll, GithubClient, poll_device_flow, start_device_flow};
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
