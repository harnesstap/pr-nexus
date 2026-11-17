//! Slack sign-in and MCP session against local fakes: discovery, PKCE,
//! client_secret_post, the loopback redirect, `auth.test`, the keychain
//! grant, refresh, and `slack_send_message`. No real Slack.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pr_nexus_core::domain::AccountId;
use pr_nexus_core::providers::{ChatProvider, ChatUser, OutgoingMessage};
use pr_nexus_providers::slack_mcp::loopback::Loopback;
use pr_nexus_providers::slack_mcp::{
    CancelHandle, ConnectConfig, SessionConfig, SlackMcpChatProvider, USER_SCOPES,
    connect_with_listener,
};
use pr_nexus_providers::vault::Vault;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
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

    let listener = Loopback::bind_ephemeral("127.0.0.1", "/custom/callback")
        .await
        .unwrap();
    let config = ConnectConfig {
        mcp_url: format!("{}/mcp", server.uri()),
        web_api_base: format!("{}/api", server.uri()),
        client_id: CLIENT_ID.into(),
        client_secret: Some("shh".into()),
        redirect_uri: listener.redirect_uri().to_owned(),
        scopes: USER_SCOPES.iter().map(|s| (*s).to_owned()).collect(),
        timeout: Duration::from_secs(20),
    };
    let vault = Arc::new(Vault::memory("test"));
    let seen = Arc::new(Mutex::new(None));
    let seen_in = seen.clone();
    let connected = connect_with_listener(
        &config,
        vault.clone(),
        listener,
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

/// `auth.test` can name this team and still return another workspace's host.
/// `team.info` is the pair that is stored.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_test_url_from_another_workspace_does_not_replace_this_domain() {
    let server = oauth_server().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth.v2.user.access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "access_token": "xoxp-acme",
            "token_type": "user",
            "scope": "chat:write",
            "refresh_token": "xoxe-1-refresh",
            "team": {"id": "T0ACME0001", "name": "Acme"},
            "authed_user": {"id": "U0ADA"},
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "url": "https://acme-labs.slack.com/",
            "team": "Acme",
            "user": "ada",
            "team_id": "T0ACME0001",
            "user_id": "U0ADA",
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/team.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "team": {"id": "T0ACME0001", "name": "Acme", "domain": "acme"},
        })))
        .mount(&server)
        .await;

    let listener = Loopback::bind_ephemeral("127.0.0.1", "/cb").await.unwrap();
    let config = ConnectConfig {
        mcp_url: format!("{}/mcp", server.uri()),
        web_api_base: format!("{}/api", server.uri()),
        client_id: CLIENT_ID.into(),
        client_secret: None,
        redirect_uri: listener.redirect_uri().to_owned(),
        scopes: vec!["chat:write".into()],
        timeout: Duration::from_secs(20),
    };
    let seen = Arc::new(Mutex::new(None));
    let connected = connect_with_listener(
        &config,
        Arc::new(Vault::memory("test")),
        listener,
        move |url| approve(url, seen),
        &CancelHandle::new(),
    )
    .await
    .expect("connect");
    assert_eq!(connected.team_id, "T0ACME0001");
    assert_eq!(connected.team_name, "Acme");
    assert_eq!(connected.domain, "acme");
    assert_ne!(connected.domain, "acme-labs");
    assert_eq!(connected.user_name, "ada");
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

    let listener = Loopback::bind_ephemeral("localhost", "/cb").await.unwrap();
    let config = ConnectConfig {
        mcp_url: format!("{}/mcp", server.uri()),
        web_api_base: format!("{}/api", server.uri()),
        client_id: CLIENT_ID.into(),
        client_secret: None,
        redirect_uri: listener.redirect_uri().to_owned(),
        scopes: vec!["chat:write".into()],
        timeout: Duration::from_secs(20),
    };
    let vault = Arc::new(Vault::memory("test"));
    let seen = Arc::new(Mutex::new(None));
    let connected = connect_with_listener(
        &config,
        vault,
        listener,
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
    let listener = Loopback::bind_ephemeral("127.0.0.1", "/cb").await.unwrap();
    let config = ConnectConfig {
        mcp_url: format!("{}/mcp", server.uri()),
        web_api_base: format!("{}/api", server.uri()),
        client_id: CLIENT_ID.into(),
        client_secret: None,
        redirect_uri: listener.redirect_uri().to_owned(),
        scopes: vec!["chat:write".into()],
        timeout: Duration::from_secs(20),
    };
    let seen = Arc::new(Mutex::new(None));
    let err = connect_with_listener(
        &config,
        Arc::new(Vault::memory("t")),
        listener,
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

/// Local MCP server. Successful tool calls wait until `release_at` of them
/// are in flight, and the first session's calls can fail so the client has
/// to open the session again.
struct PaceServer {
    runtime: Option<tokio::runtime::Runtime>,
    url: String,
    stats: Arc<PaceStats>,
    accept: Option<JoinHandle<()>>,
}

struct PaceStats {
    initializes: AtomicUsize,
    failed_calls: AtomicUsize,
    inflight: AtomicUsize,
    peak: AtomicUsize,
    /// Successful `tools/call`s wait until this many are in flight. `0` answers
    /// immediately. Once that many have overlapped, later calls do not wait.
    release_at: AtomicUsize,
    opened: AtomicBool,
    timed_out: AtomicBool,
    fail_first_session: bool,
    fail_sessions: Mutex<std::collections::HashSet<String>>,
    log: Mutex<Vec<String>>,
}

impl Drop for PaceServer {
    fn drop(&mut self) {
        if let Some(accept) = self.accept.take() {
            accept.abort();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

fn fresh_grant() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut token = slack_token("xoxp-acme", 86_400);
    token.as_object_mut().unwrap().remove("ok");
    json!({
        "client_id": CLIENT_ID,
        "token_response": token,
        "granted_scopes": USER_SCOPES,
        "token_received_at": now,
    })
    .to_string()
}

fn pace_server(fail_first_session: bool) -> PaceServer {
    let stats = Arc::new(PaceStats {
        initializes: AtomicUsize::new(0),
        failed_calls: AtomicUsize::new(0),
        inflight: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        release_at: AtomicUsize::new(0),
        opened: AtomicBool::new(false),
        timed_out: AtomicBool::new(false),
        fail_first_session,
        fail_sessions: Mutex::new(std::collections::HashSet::new()),
        log: Mutex::new(Vec::new()),
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let (url, stats, accept) = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let origin = base.clone();
        let stats_task = stats.clone();
        let accept = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let stats = stats_task.clone();
                let base = origin.clone();
                tokio::spawn(async move {
                    let _ = serve_one(socket, stats, base).await;
                });
            }
        });
        (base, stats, accept)
    });
    PaceServer {
        runtime: Some(runtime),
        url,
        stats,
        accept: Some(accept),
    }
}

fn signed_in(server: &PaceServer) -> SlackMcpChatProvider {
    let vault = Arc::new(Vault::memory("test"));
    vault.set("slack:T0ACME:U0JDOE", &fresh_grant()).unwrap();
    SlackMcpChatProvider::connect(
        SessionConfig {
            mcp_url: format!("{}/mcp", server.url),
            client_id: CLIENT_ID.into(),
            client_secret: Some("shh".into()),
            redirect_uri: "http://127.0.0.1:47821/mcp/oauth/callback".into(),
            keychain_key: "slack:T0ACME:U0JDOE".into(),
        },
        vault,
        AccountId(1),
        ChatUser {
            id: "U0JDOE".into(),
            name: "jdoe".into(),
        },
    )
    .unwrap()
}

async fn serve_one(
    mut socket: tokio::net::TcpStream,
    stats: Arc<PaceStats>,
    base: String,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        if buf.len() > 65_536 {
            return Ok(());
        }
        let n = socket.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut body = buf[header_end + 4..].to_vec();
    let length = header_value(&head, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while body.len() < length {
        let n = socket.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(length);

    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    let session = header_value(&head, "mcp-session-id").unwrap_or_default();

    let response = if path.contains("oauth-protected-resource") {
        json_response(
            200,
            "OK",
            "",
            &json!({
                "resource": format!("{base}/mcp"),
                "authorization_servers": [base],
            })
            .to_string(),
        )
    } else if path.contains(".well-known/") {
        json_response(
            200,
            "OK",
            "",
            &json!({
                "issuer": base,
                "authorization_endpoint": format!("{base}/oauth/v2_user/authorize"),
                "token_endpoint": format!("{base}/api/oauth.v2.user.access"),
                "response_types_supported": ["code"],
                "code_challenge_methods_supported": ["S256"],
                "token_endpoint_auth_methods_supported": ["client_secret_post"],
                "grant_types_supported": ["authorization_code", "refresh_token"],
            })
            .to_string(),
        )
    } else if method == "GET" {
        note(&stats, format!("GET {path}"));
        status_response(405, "Method Not Allowed", "")
    } else if method == "DELETE" {
        note(&stats, format!("DELETE {path}"));
        status_response(200, "OK", "")
    } else if method == "POST" && path.starts_with("/mcp") {
        mcp_response(&stats, &session, &body).await
    } else {
        note(&stats, format!("{method} {path}"));
        status_response(404, "Not Found", "")
    };
    socket.write_all(&response).await?;
    let _ = socket.shutdown().await;
    Ok(())
}

async fn mcp_response(stats: &PaceStats, session: &str, body: &[u8]) -> Vec<u8> {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let id = value.get("id").cloned();
    let method = value["method"].as_str().unwrap_or("");
    note(stats, format!("POST /mcp {method} session={session}"));
    if id.is_none() {
        return status_response(202, "Accepted", session);
    }
    let result = match method {
        "initialize" => {
            let n = stats.initializes.fetch_add(1, Ordering::SeqCst) + 1;
            let issued = format!("s{n}");
            if stats.fail_first_session && n == 1 {
                stats.fail_sessions.lock().unwrap().insert(issued.clone());
            }
            let version = value["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2025-03-26");
            return json_response(
                200,
                "OK",
                &issued,
                &json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": version,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "fake-slack", "version": "1"},
                    }
                })
                .to_string(),
            );
        }
        "tools/list" => json!({"tools": [{
            "name": "slack_send_message",
            "inputSchema": {"type": "object", "properties": {
                "channel_id": {"type": "string"},
                "message": {"type": "string"},
                "thread_ts": {"type": "string"},
                "reply_broadcast": {"type": "boolean"},
                "unfurl_app_links": {"type": "boolean"},
            }},
        }, {
            "name": "slack_read_user_profile",
            "inputSchema": {"type": "object", "properties": {
                "user_id": {"type": "string"},
                "response_format": {"type": "string"},
            }},
        }]}),
        "tools/call" => {
            let fail = stats.fail_sessions.lock().unwrap().contains(session);
            if fail {
                stats.failed_calls.fetch_add(1, Ordering::SeqCst);
                return status_response(500, "Internal Server Error", session);
            }
            hold_for_overlap(stats).await;
            let user = value["params"]["arguments"]["user_id"]
                .as_str()
                .unwrap_or("Ada Acme");
            json!({
                "content": [{"type": "text", "text": user}],
                "isError": false,
            })
        }
        _ => json!({}),
    };
    json_response(
        200,
        "OK",
        session,
        &json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string(),
    )
}

/// Stay inside `tools/call` until `release_at` calls overlap, then answer.
/// A caller that never sees that many sets `timed_out` after 10s so the rest
/// can finish and the test can report it.
async fn hold_for_overlap(stats: &PaceStats) {
    let need = stats.release_at.load(Ordering::SeqCst);
    let now = stats.inflight.fetch_add(1, Ordering::SeqCst) + 1;
    stats.peak.fetch_max(now, Ordering::SeqCst);
    if need > 1 && !stats.opened.load(Ordering::SeqCst) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let current = stats.inflight.load(Ordering::SeqCst);
            if current >= need || stats.opened.load(Ordering::SeqCst) {
                stats.opened.store(true, Ordering::SeqCst);
                break;
            }
            if std::time::Instant::now() >= deadline {
                stats.opened.store(true, Ordering::SeqCst);
                stats.timed_out.store(true, Ordering::SeqCst);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    stats.inflight.fetch_sub(1, Ordering::SeqCst);
}

fn note(stats: &PaceStats, line: String) {
    let mut log = stats.log.lock().unwrap();
    if log.len() < 80 {
        log.push(line);
    }
}

fn header_value(head: &str, name: &str) -> Option<String> {
    head.split("\r\n").skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn session_header(session: &str) -> String {
    if session.is_empty() {
        String::new()
    } else {
        format!("Mcp-Session-Id: {session}\r\n")
    }
}

fn status_response(status: u16, reason: &str, session: &str) -> Vec<u8> {
    let body = reason.as_bytes();
    let session_line = session_header(session);
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n{session_line}Connection: close\r\n\r\n",
        body.len()
    );
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn json_response(status: u16, reason: &str, session: &str, body: &str) -> Vec<u8> {
    let session_line = session_header(session);
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{session_line}Connection: close\r\n\r\n",
        body.len()
    );
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

#[test]
fn concurrent_calls_overlap_on_one_session() {
    let server = pace_server(false);
    let slack = signed_in(&server);
    slack.user_name("U0WARM0001").expect("warmup");
    server.stats.peak.store(0, Ordering::SeqCst);
    server.stats.inflight.store(0, Ordering::SeqCst);
    server.stats.opened.store(false, Ordering::SeqCst);
    server.stats.timed_out.store(false, Ordering::SeqCst);
    server.stats.release_at.store(4, Ordering::SeqCst);

    let ids: Vec<String> = (1..=4).map(|n| format!("U0ACME000{n}")).collect();
    let names = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for id in &ids {
            handles.push(scope.spawn(|| slack.user_name(id)));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("thread"))
            .collect::<Vec<_>>()
    });
    let peak = server.stats.peak.load(Ordering::SeqCst);
    let log = server.stats.log.lock().unwrap().join("\n");
    for (id, name) in ids.iter().zip(names) {
        assert_eq!(name.expect("profile"), *id, "{log}");
    }
    assert!(
        !server.stats.timed_out.load(Ordering::SeqCst),
        "calls never overlapped\n{log}"
    );
    assert!(
        peak >= ids.len(),
        "peak {peak} calls did not overlap\n{log}"
    );
    assert_eq!(
        server.stats.initializes.load(Ordering::SeqCst),
        1,
        "one session\n{log}"
    );
}

#[test]
fn a_dropped_session_is_reopened_once() {
    let server = pace_server(true);
    server.stats.release_at.store(2, Ordering::SeqCst);
    let slack = signed_in(&server);
    let ids: Vec<String> = (1..=6).map(|n| format!("U0ACME000{n}")).collect();
    let names = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for id in &ids {
            handles.push(scope.spawn(|| slack.user_name(id)));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("thread"))
            .collect::<Vec<_>>()
    });
    let log = server.stats.log.lock().unwrap().join("\n");
    for (id, name) in ids.iter().zip(&names) {
        assert_eq!(name.as_deref().ok(), Some(id.as_str()), "{name:?}\n{log}");
    }
    let opens = server.stats.initializes.load(Ordering::SeqCst);
    assert_eq!(opens, 2, "one reopen, not one per call\n{log}");
    assert!(
        server.stats.failed_calls.load(Ordering::SeqCst) >= 1,
        "the first session's call failed\n{log}"
    );
    let peak = server.stats.peak.load(Ordering::SeqCst);
    assert!(
        !server.stats.timed_out.load(Ordering::SeqCst),
        "retries never overlapped\n{log}"
    );
    assert!(peak >= 2, "retries overlapped, peak {peak}\n{log}");
}
