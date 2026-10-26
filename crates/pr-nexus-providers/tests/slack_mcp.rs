//! Slack sign-in and MCP session against local fakes: discovery, PKCE,
//! client_secret_post, the loopback redirect, `auth.test`, the keychain
//! grant, refresh, and `slack_send_message`. No real Slack.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pr_nexus_core::domain::AccountId;
use pr_nexus_core::providers::{ChatProvider, ChatUser, OutgoingMessage};
use pr_nexus_providers::slack_mcp::{
    CancelHandle, ConnectConfig, SessionConfig, SlackMcpChatProvider, USER_SCOPES, connect,
};
use pr_nexus_providers::vault::Vault;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, header, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const CLIENT_ID: &str = "1111111111.2222222222";

async fn oauth_server() -> MockServer {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path_regex("^/.well-known/oauth-protected-resource.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{base}/mcp"),
            "authorization_servers": [base],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(
            "^/.well-known/(oauth-authorization-server|openid-configuration).*",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/oauth/v2_user/authorize"),
            "token_endpoint": format!("{base}/api/oauth.v2.user.access"),
            "response_types_supported": ["code"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["client_secret_post"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
        })))
        .mount(&server)
        .await;
    server
}

fn slack_token(access: &str, expires_in: u64) -> Value {
    json!({
        "ok": true,
        "access_token": access,
        "token_type": "user",
        "scope": USER_SCOPES.join(","),
        "expires_in": expires_in,
        "refresh_token": "xoxe-1-refresh",
        "team": {"id": "T0ACME"},
        "authed_user": {"id": "U0JDOE"},
    })
}

/// Follow the authorize URL like a browser that approves: hit the redirect.
fn approve(auth_url: &str, seen: Arc<Mutex<Option<url::Url>>>) -> Result<(), String> {
    let url = url::Url::parse(auth_url).map_err(|err| err.to_string())?;
    let query = |key: &str| {
        url.query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default()
    };
    let redirect = url::Url::parse(&query("redirect_uri")).map_err(|err| err.to_string())?;
    let state = query("state");
    *seen.lock().unwrap() = Some(url.clone());
    std::thread::spawn(move || {
        let mut stream =
            TcpStream::connect(("127.0.0.1", redirect.port().unwrap())).expect("listener");
        write!(
            stream,
            "GET {}?code=code-123&state={state} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            redirect.path()
        )
        .unwrap();
        let mut reply = String::new();
        let _ = stream.read_to_string(&mut reply);
    });
    Ok(())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_runs_pkce_with_secret_and_stores_the_grant() {
    let server = oauth_server().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth.v2.user.access"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=code-123"))
        .and(body_string_contains("code_verifier="))
        .and(body_string_contains("client_secret=shh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_token("xoxp-first", 43200)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/auth.test"))
        .and(header("authorization", "Bearer xoxp-first"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "url": "https://acme.slack.com/", "team": "Acme Inc",
            "user": "jdoe", "team_id": "T0ACME", "user_id": "U0JDOE",
        })))
        .mount(&server)
        .await;

    let port = free_port();
    let config = ConnectConfig {
        mcp_url: format!("{}/mcp", server.uri()),
        web_api_base: format!("{}/api", server.uri()),
        client_id: CLIENT_ID.into(),
        client_secret: Some("shh".into()),
        redirect_uri: format!("http://127.0.0.1:{port}/custom/callback"),
        scopes: USER_SCOPES.iter().map(|s| (*s).to_owned()).collect(),
        timeout: Duration::from_secs(20),
    };
    let vault = Arc::new(Vault::memory("test"));
    let seen = Arc::new(Mutex::new(None));
    let seen_in = seen.clone();
    let connected = connect(
        &config,
        vault.clone(),
        move |url| approve(url, seen_in),
        &CancelHandle::new(),
    )
    .await
    .expect("connect");

    assert_eq!(connected.team_id, "T0ACME");
    assert_eq!(connected.team_name, "Acme Inc");
    assert_eq!(connected.domain, "acme");
    assert_eq!(connected.user_name, "jdoe");
    assert_eq!(connected.keychain_key, "slack:T0ACME:U0JDOE");

    let url = seen.lock().unwrap().clone().unwrap();
    assert_eq!(url.path(), "/oauth/v2_user/authorize");
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], CLIENT_ID);
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["redirect_uri"], config.redirect_uri);
    // On the `v2_user` endpoint the user scopes go in `scope`; `user_scope`
    // belongs to the `v2/authorize` bot-install flow and must not be sent.
    for scope in USER_SCOPES {
        assert!(
            query["scope"].split([' ', ',']).any(|s| s == *scope),
            "missing {scope}"
        );
    }
    assert!(!query.contains_key("user_scope"));
    assert!(
        !query.contains_key("client_secret"),
        "secret never goes to the browser"
    );

    let stored: Value =
        serde_json::from_str(&vault.get("slack:T0ACME:U0JDOE").unwrap().unwrap()).unwrap();
    assert_eq!(stored["client_id"], CLIENT_ID);
    assert_eq!(stored["token_response"]["refresh_token"], "xoxe-1-refresh");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_without_secret_sends_client_id_and_verifier_only() {
    let server = oauth_server().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth.v2.user.access"))
        .and(body_string_contains("code_verifier="))
        .and(body_string_contains(format!("client_id={CLIENT_ID}")))
        .respond_with(|request: &Request| {
            let body = String::from_utf8_lossy(&request.body);
            assert!(
                !body.contains("client_secret"),
                "no secret configured: {body}"
            );
            ResponseTemplate::new(200).set_body_json(slack_token("xoxp-pkce", 43200))
        })
        .expect(1)
        .mount(&server)
        .await;
    // auth.test fails: the IDs from the token response are enough.
    Mock::given(method("POST"))
        .and(path("/api/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "not_allowed_token_type"})),
        )
        .mount(&server)
        .await;

    let config = ConnectConfig {
        mcp_url: format!("{}/mcp", server.uri()),
        web_api_base: format!("{}/api", server.uri()),
        client_id: CLIENT_ID.into(),
        client_secret: None,
        redirect_uri: format!("http://localhost:{}/cb", free_port()),
        scopes: vec!["chat:write".into()],
        timeout: Duration::from_secs(20),
    };
    let vault = Arc::new(Vault::memory("test"));
    let seen = Arc::new(Mutex::new(None));
    let connected = connect(
        &config,
        vault,
        move |url| approve(url, seen),
        &CancelHandle::new(),
    )
    .await
    .expect("connect");
    assert_eq!(connected.team_name, "T0ACME");
    assert_eq!(connected.domain, "t0acme");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slack_errors_on_exchange_are_readable() {
    let server = oauth_server().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth.v2.user.access"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "invalid_client_id"})),
        )
        .mount(&server)
        .await;
    let config = ConnectConfig {
        mcp_url: format!("{}/mcp", server.uri()),
        web_api_base: format!("{}/api", server.uri()),
        client_id: CLIENT_ID.into(),
        client_secret: None,
        redirect_uri: format!("http://127.0.0.1:{}/cb", free_port()),
        scopes: vec!["chat:write".into()],
        timeout: Duration::from_secs(20),
    };
    let seen = Arc::new(Mutex::new(None));
    let err = connect(
        &config,
        Arc::new(Vault::memory("t")),
        move |url| approve(url, seen),
        &CancelHandle::new(),
    )
    .await
    .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("client secret"), "{text}");
}

/// A Streamable HTTP MCP server with one tool, answering JSON.
struct FakeMcp {
    calls: Arc<Mutex<Vec<Value>>>,
}

impl Respond for FakeMcp {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let id = body.get("id").cloned();
        let method = body["method"].as_str().unwrap_or_default();
        let result = match method {
            "initialize" => json!({
                "protocolVersion": body["params"]["protocolVersion"],
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "fake-slack", "version": "1"},
            }),
            "tools/list" => json!({"tools": [{
                "name": "slack_send_message",
                "inputSchema": {"type": "object", "properties": {
                    "channel_id": {"type": "string"}, "message": {"type": "string"},
                    "thread_ts": {"type": "string"}, "reply_broadcast": {"type": "boolean"},
                    "unfurl_app_links": {"type": "boolean"},
                }},
            }]}),
            "tools/call" => {
                self.calls.lock().unwrap().push(body["params"].clone());
                json!({"content": [{"type": "text", "text":
                    "Message sent: https://acme.slack.com/archives/C0TEAM/p1700000000000100"}], "isError": false})
            }
            _ => return ResponseTemplate::new(202),
        };
        match id {
            Some(id) => ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({"jsonrpc": "2.0", "id": id, "result": result})),
            None => ResponseTemplate::new(202),
        }
    }
}

#[test]
fn session_refreshes_the_grant_and_posts_through_mcp() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server = runtime.block_on(async {
        let server = oauth_server().await;
        Mock::given(method("POST"))
            .and(path("/api/oauth.v2.user.access"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=xoxe-1-refresh"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(slack_token("xoxp-refreshed", 43200)),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .and(header("authorization", "Bearer xoxp-refreshed"))
            .respond_with(FakeMcp {
                calls: calls.clone(),
            })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        server
    });

    // A grant that is about to expire, as saved by a sign-in days ago.
    let vault = Arc::new(Vault::memory("test"));
    let mut token = slack_token("xoxp-old", 60);
    token.as_object_mut().unwrap().remove("ok");
    let stored = json!({
        "client_id": CLIENT_ID,
        "token_response": token,
        "granted_scopes": USER_SCOPES,
        "token_received_at": 1_700_000_000u64,
    });
    vault
        .set("slack:T0ACME:U0JDOE", &stored.to_string())
        .unwrap();

    let slack = SlackMcpChatProvider::connect(
        SessionConfig {
            mcp_url: format!("{}/mcp", server.uri()),
            client_id: CLIENT_ID.into(),
            client_secret: Some("shh".into()),
            redirect_uri: "http://127.0.0.1:47821/mcp/oauth/callback".into(),
            keychain_key: "slack:T0ACME:U0JDOE".into(),
        },
        vault.clone(),
        AccountId(1),
        ChatUser {
            id: "U0JDOE".into(),
            name: "jdoe".into(),
        },
    )
    .unwrap();
    let posted = slack
        .post(&OutgoingMessage {
            channel_id: "C0TEAM",
            text: "Review please",
            thread_ts: None,
            reply_broadcast: false,
            unfurl_app_links: true,
        })
        .expect("post");
    assert_eq!(posted.ts, "1700000000.000100");
    assert_eq!(
        posted.permalink.as_deref(),
        Some("https://acme.slack.com/archives/C0TEAM/p1700000000000100")
    );
    let calls = calls.lock().unwrap().clone();
    assert_eq!(calls[0]["name"], "slack_send_message");
    assert_eq!(calls[0]["arguments"]["channel_id"], "C0TEAM");

    let saved: Value =
        serde_json::from_str(&vault.get("slack:T0ACME:U0JDOE").unwrap().unwrap()).unwrap();
    assert_eq!(saved["token_response"]["access_token"], "xoxp-refreshed");
    drop(slack);
    drop(server);
}
