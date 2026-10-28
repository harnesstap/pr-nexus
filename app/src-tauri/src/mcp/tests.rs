//! The server end to end on an ephemeral port: auth, DNS-rebinding
//! checks, `tools/list` and every read tool over the demo seed. Nothing
//! here leaves 127.0.0.1.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::audit::{AuditEvent, AuditSink, Outcome};
use super::control::{McpControl, ServerState};
use super::{Capability, McpSettings};
use crate::state::AppState;

#[derive(Default)]
struct Recorder(Mutex<Vec<AuditEvent>>);

impl AuditSink for Recorder {
    fn record(&self, event: &AuditEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

struct Harness {
    control: Arc<McpControl>,
    audit: Arc<Recorder>,
    url: String,
    token: String,
    http: reqwest::Client,
    _dir: tempfile::TempDir,
}

async fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::for_tests(&dir.path().join("test.db"), true));
    crate::demo::seed_if_empty(&state.store).unwrap();
    let audit = Arc::new(Recorder::default());
    let control = Arc::new(McpControl::with_audit(state, audit.clone()).unwrap());
    let view = control
        .update(McpSettings {
            enabled: true,
            ..McpSettings::default()
        })
        .await
        .unwrap();
    assert_eq!(
        view.status.state,
        ServerState::Running,
        "{:?}",
        view.status.error
    );
    let url = view.status.url.unwrap();
    assert!(url.starts_with("http://127.0.0.1:"));
    let token = control.token().await.unwrap();
    Harness {
        control,
        audit,
        url,
        token,
        http: reqwest::Client::new(),
        _dir: dir,
    }
}

/// The JSON-RPC message in a JSON or SSE body.
fn message(body: &str) -> Value {
    if let Ok(value) = serde_json::from_str(body) {
        return value;
    }
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .find(|value| value.get("id").is_some())
        .unwrap_or_else(|| panic!("no JSON-RPC response in {body}"))
}

impl Harness {
    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        self.http
            .post(&self.url)
            .header("accept", "application/json, text/event-stream")
            .json(body)
    }

    /// `initialize` plus `notifications/initialized`; returns the session id.
    async fn session(&self) -> String {
        let response = self
            .post(&json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "acme-agent", "version": "1.0"}}
            }))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let session = response.headers()["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let init = message(&response.text().await.unwrap());
        assert_eq!(init["result"]["serverInfo"]["name"], "pr-nexus");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        let ack = self
            .post(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .bearer_auth(&self.token)
            .header("mcp-session-id", &session)
            .send()
            .await
            .unwrap();
        assert!(ack.status().is_success());
        session
    }

    async fn rpc(&self, session: &str, id: u64, method: &str, params: Value) -> Value {
        let response = self
            .post(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .bearer_auth(&self.token)
            .header("mcp-session-id", session)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        message(&response.text().await.unwrap())
    }

    async fn call(&self, session: &str, tool: &str, arguments: Value) -> Value {
        let reply = self
            .rpc(
                session,
                7,
                "tools/call",
                json!({"name": tool, "arguments": arguments}),
            )
            .await;
        reply["result"].clone()
    }
}

fn initialize() -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "acme-agent", "version": "1.0"}}})
}

#[tokio::test(flavor = "multi_thread")]
async fn requests_without_the_right_token_are_rejected() {
    let h = harness().await;
    let missing = h.post(&initialize()).send().await.unwrap();
    assert_eq!(missing.status(), 401);
    assert_eq!(missing.headers()["www-authenticate"], "Bearer");
    let wrong = h
        .post(&initialize())
        .bearer_auth("pnx_not-the-token")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
    let scheme = h
        .post(&initialize())
        .header("authorization", &h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(scheme.status(), 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn foreign_origins_and_hosts_are_rejected() {
    let h = harness().await;
    let origin = h
        .post(&initialize())
        .bearer_auth(&h.token)
        .header("origin", "https://evil.acme-labs.example")
        .send()
        .await
        .unwrap();
    assert_eq!(origin.status(), 403);
    let rebound = h
        .post(&initialize())
        .bearer_auth(&h.token)
        .header("host", "rebind.acme-labs.example")
        .send()
        .await
        .unwrap();
    assert_eq!(rebound.status(), 403);
    let port = h
        .url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/mcp")
        .to_owned();
    let own = h
        .post(&initialize())
        .bearer_auth(&h.token)
        .header("origin", format!("http://localhost:{port}"))
        .header("host", format!("localhost:{port}"))
        .send()
        .await
        .unwrap();
    assert_eq!(own.status(), 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regenerated_token_replaces_the_old_one_live() {
    let h = harness().await;
    h.control.regenerate().await.unwrap();
    let old = h
        .post(&initialize())
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), 401);
    let fresh = h.control.token().await.unwrap();
    let new = h
        .post(&initialize())
        .bearer_auth(&fresh)
        .send()
        .await
        .unwrap();
    assert_eq!(new.status(), 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_list_has_the_read_tools_with_schemas() {
    let h = harness().await;
    let session = h.session().await;
    let reply = h.rpc(&session, 2, "tools/list", json!({})).await;
    let tools = reply["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["list_projects", "get_project", "list_prs", "get_pr"]
    );
    for tool in tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert!(tool["description"].as_str().unwrap().len() > 80);
    }
    let list_prs = &tools[2]["inputSchema"];
    let schema = list_prs.to_string();
    for status in ["merge_ready", "ci_trouble", "no_review", "idle"] {
        assert!(schema.contains(status), "{status} missing from {schema}");
    }
    assert!(list_prs["properties"]["limit"].is_object());
}

#[tokio::test(flavor = "multi_thread")]
async fn read_tools_answer_from_the_store() {
    let h = harness().await;
    let session = h.session().await;

    let projects = h.call(&session, "list_projects", json!({})).await;
    assert_eq!(projects["isError"], false, "{projects}");
    let list = projects["structuredContent"]["projects"]
        .as_array()
        .unwrap();
    assert!(!list.is_empty());
    assert!(list.iter().all(|project| project["archived"] == false));
    let project = list
        .iter()
        .find(|project| project["pr_count"].as_u64() > Some(0))
        .expect("a project with PRs");
    let project_id = project["id"].as_i64().unwrap();
    assert!(project["jira_links"].is_array());
    assert!(project["open_pr_count"].as_u64() <= project["pr_count"].as_u64());

    let detail = h
        .call(
            &session,
            "get_project",
            json!({"project_id": project_id, "limit": 1}),
        )
        .await;
    let detail = &detail["structuredContent"];
    assert_eq!(detail["project"]["id"], project_id);
    assert_eq!(detail["prs"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(detail["prs"]["total"], project["pr_count"]);

    let all = h.call(&session, "list_prs", json!({})).await["structuredContent"].clone();
    let total = all["total"].as_u64().unwrap();
    assert!(total > 2);
    let page = h.call(&session, "list_prs", json!({"limit": 2})).await["structuredContent"].clone();
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    let cursor = page["next_cursor"].as_str().unwrap().to_owned();
    let next = h
        .call(&session, "list_prs", json!({"limit": 2, "cursor": cursor}))
        .await["structuredContent"]
        .clone();
    assert_ne!(next["items"][0]["id"], page["items"][0]["id"]);

    let status = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|item| {
            item["statuses"]
                .as_array()
                .and_then(|list| list.first())
                .cloned()
        })
        .expect("a seeded PR with a status");
    let matching = h
        .call(&session, "list_prs", json!({"statuses": [status]}))
        .await["structuredContent"]
        .clone();
    let matching = matching["items"].as_array().unwrap();
    assert!(!matching.is_empty());
    assert!(
        matching
            .iter()
            .all(|item| item["statuses"].as_array().unwrap().contains(&status))
    );

    let pr_id = all["items"][0]["id"].as_i64().unwrap();
    let pr = h.call(&session, "get_pr", json!({"pr_id": pr_id})).await;
    assert_eq!(pr["isError"], false, "{pr}");
    let pr = &pr["structuredContent"];
    assert_eq!(pr["id"], pr_id);
    assert!(pr["note"].is_string());
    assert!(pr.get("starred").is_some());
    assert!(pr.get("slack").is_some());
    assert!(pr["branch"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_arguments_and_unknown_ids_are_tool_errors() {
    let h = harness().await;
    let session = h.session().await;
    let bad = h
        .call(&session, "list_prs", json!({"statuses": ["shipped"]}))
        .await;
    assert_eq!(bad["isError"], true);
    assert_eq!(bad["structuredContent"]["kind"], "validation");
    let extra = h
        .call(&session, "list_projects", json!({"archived_too": true}))
        .await;
    assert_eq!(extra["structuredContent"]["kind"], "validation");
    let missing = h.call(&session, "get_pr", json!({"pr_id": 999_999})).await;
    assert_eq!(missing["structuredContent"]["kind"], "not_found");
    let unknown = h
        .rpc(
            &session,
            9,
            "tools/call",
            json!({"name": "delete_everything", "arguments": {}}),
        )
        .await;
    assert!(unknown["error"].is_object(), "{unknown}");
}

#[tokio::test(flavor = "multi_thread")]
async fn turning_reads_off_hides_and_refuses_the_tools() {
    let h = harness().await;
    let session = h.session().await;
    h.call(&session, "list_projects", json!({})).await;
    h.control
        .update(McpSettings {
            enabled: true,
            reads: false,
            ..McpSettings::default()
        })
        .await
        .unwrap();
    let reply = h.rpc(&session, 3, "tools/list", json!({})).await;
    assert!(reply["result"]["tools"].as_array().unwrap().is_empty());
    let denied = h.call(&session, "list_projects", json!({})).await;
    assert_eq!(denied["structuredContent"]["kind"], "forbidden");

    let events = h.audit.0.lock().unwrap().clone();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].outcome, Outcome::Ok);
    assert_eq!(events[0].client.as_deref(), Some("acme-agent"));
    assert_eq!(events[1].outcome, Outcome::Denied);
    assert_eq!(events[1].capability, Capability::Reads);
}

#[tokio::test(flavor = "multi_thread")]
async fn turning_the_server_off_closes_the_port() {
    let h = harness().await;
    let view = h.control.update(McpSettings::default()).await.unwrap();
    assert_eq!(view.status.state, ServerState::Off);
    assert!(
        h.post(&initialize())
            .bearer_auth(&h.token)
            .send()
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_taken_port_is_reported_not_fatal() {
    let h = harness().await;
    let port: u16 = h
        .url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/mcp")
        .parse()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let other = Arc::new(AppState::for_tests(&dir.path().join("other.db"), true));
    let second = McpControl::new(other).unwrap();
    let view = second
        .update(McpSettings {
            enabled: true,
            port: Some(port),
            ..McpSettings::default()
        })
        .await
        .unwrap();
    assert_eq!(view.status.state, ServerState::Error);
    assert!(view.status.error.unwrap().contains("in use"));
}
