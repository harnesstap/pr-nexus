//! The server end to end on an ephemeral port: auth, DNS-rebinding
//! checks, `tools/list` and every read tool over the demo seed. Nothing
//! here leaves 127.0.0.1.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::audit::{AuditEvent, AuditSink, Outcome};
use super::control::{McpControl, ServerState};
use pr_nexus_core::store::AuditOutcome;

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
    let control = Arc::new(McpControl::with_audit(state, audit.clone(), None).unwrap());
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

    async fn writes(&self, on: bool) {
        self.control
            .update(McpSettings {
                enabled: true,
                local_writes: on,
                ..McpSettings::default()
            })
            .await
            .unwrap();
    }

    fn store(&self) -> &pr_nexus_core::store::Store {
        &self.control.app_state().store
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

/// Every `description` string anywhere in `value`.
fn descriptions<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                match (key.as_str(), child) {
                    ("description", Value::String(text)) => out.push(text),
                    _ => descriptions(child, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| descriptions(item, out)),
        _ => {}
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_facing_text_has_no_rust_doc_syntax() {
    let h = harness().await;
    h.writes(true).await;
    let session = h.session().await;
    let reply = h.rpc(&session, 2, "tools/list", json!({})).await;
    let mut texts = Vec::new();
    descriptions(&reply["result"]["tools"], &mut texts);
    assert!(texts.len() > 20, "{texts:?}");
    for text in texts {
        for bad in ["Some(", "[`", "::", "\n"] {
            assert!(!text.contains(bad), "{bad:?} in description {text:?}");
        }
    }
    let tools = reply["result"]["tools"].as_array().unwrap();
    assert!(tools.len() > 4, "write tools missing from the check");
    let starred = &tools[2]["inputSchema"]["properties"]["starred"]["description"];
    assert_eq!(
        starred,
        "true: starred only; false: not starred; omit: any."
    );
    for tool in [&tools[1], &tools[2]] {
        let limit = &tool["inputSchema"]["properties"]["limit"];
        assert_eq!(limit["minimum"], 1, "{limit}");
        assert_eq!(limit["maximum"], 200, "{limit}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn out_of_range_limits_are_refused() {
    let h = harness().await;
    let session = h.session().await;
    for limit in [0, 201] {
        let reply = h.call(&session, "list_prs", json!({"limit": limit})).await;
        assert_eq!(reply["isError"], true, "{reply}");
        assert_eq!(reply["structuredContent"]["kind"], "validation");
    }
    let ok = h.call(&session, "list_prs", json!({"limit": 200})).await;
    assert_ne!(ok["isError"], true, "{ok}");
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
    let second = McpControl::new(other, None).unwrap();
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

fn ok(reply: &Value) -> &Value {
    assert_eq!(reply["isError"], false, "{reply}");
    &reply["structuredContent"]
}

fn kind(reply: &Value) -> &str {
    assert_eq!(reply["isError"], true, "{reply}");
    reply["structuredContent"]["kind"].as_str().unwrap()
}

const WRITE_TOOLS: [&str; 11] = [
    "create_project",
    "update_project",
    "archive_project",
    "unarchive_project",
    "add_prs_to_project",
    "remove_prs_from_project",
    "set_project_jira_links",
    "add_jira_link",
    "remove_jira_link",
    "set_pr_note",
    "set_pr_starred",
];

/// Two PR ids from the demo seed.
async fn two_prs(h: &Harness, session: &str) -> (i64, i64) {
    let page = h.call(session, "list_prs", json!({"limit": 2})).await;
    let items = ok(&page)["items"].as_array().unwrap().clone();
    (
        items[0]["id"].as_i64().unwrap(),
        items[1]["id"].as_i64().unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_are_hidden_refused_and_logged_while_off() {
    let h = harness().await;
    let session = h.session().await;
    let reply = h.rpc(&session, 2, "tools/list", json!({})).await;
    let names: Vec<&str> = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(
        WRITE_TOOLS.iter().all(|tool| !names.contains(tool)),
        "{names:?}"
    );

    let (pr, _) = two_prs(&h, &session).await;
    let denied = h
        .call(
            &session,
            "set_pr_starred",
            json!({"pr_id": pr, "starred": true}),
        )
        .await;
    assert_eq!(kind(&denied), "forbidden");
    assert!(
        !h.store()
            .is_pr_starred(pr_nexus_core::domain::PrId(pr))
            .unwrap()
    );

    let rows = h.store().mcp_audit(None, 10).unwrap();
    assert_eq!(rows[0].tool, "set_pr_starred");
    assert_eq!(rows[0].outcome, AuditOutcome::Denied);
    assert_eq!(rows[0].capability, "local_writes");
    assert_eq!(rows[0].client_name, "acme-agent");
    assert_eq!(rows[0].target_ids, format!("pr:{pr}"));
    assert!(rows[0].error.as_deref().unwrap().contains("turned off"));
    assert_eq!(rows[1].tool, "list_prs", "reads are logged too");
    assert_eq!(rows[1].outcome, AuditOutcome::Allowed);
}

#[tokio::test(flavor = "multi_thread")]
async fn write_tools_are_listed_with_honest_hints_and_no_delete() {
    let h = harness().await;
    h.writes(true).await;
    let session = h.session().await;
    let reply = h.rpc(&session, 2, "tools/list", json!({})).await;
    let tools = reply["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    for tool in WRITE_TOOLS {
        assert!(names.contains(&tool), "{tool} missing");
    }
    assert!(
        names.iter().all(|name| !name.contains("delete")),
        "{names:?}"
    );
    let hint = |name: &str, key: &str| {
        tools.iter().find(|tool| tool["name"] == name).unwrap()["annotations"][key].clone()
    };
    for tool in WRITE_TOOLS {
        assert_eq!(hint(tool, "readOnlyHint"), false, "{tool}");
        assert_eq!(hint(tool, "openWorldHint"), false, "{tool}");
    }
    for tool in [
        "set_pr_note",
        "remove_prs_from_project",
        "set_project_jira_links",
        "remove_jira_link",
        "update_project",
    ] {
        assert_eq!(hint(tool, "destructiveHint"), true, "{tool}");
    }
    for tool in [
        "set_pr_starred",
        "archive_project",
        "add_prs_to_project",
        "create_project",
    ] {
        assert_eq!(hint(tool, "destructiveHint"), false, "{tool}");
    }
    assert_eq!(hint("set_pr_starred", "idempotentHint"), true);
    assert_eq!(hint("create_project", "idempotentHint"), false);
    let deleted = h
        .rpc(
            &session,
            3,
            "tools/call",
            json!({"name": "delete_project", "arguments": {"project_id": 1}}),
        )
        .await;
    assert!(deleted["error"].is_object(), "{deleted}");
}

#[tokio::test(flavor = "multi_thread")]
async fn every_write_tool_changes_the_store() {
    use pr_nexus_core::domain::{PrId, ProjectId};
    let h = harness().await;
    h.writes(true).await;
    let session = h.session().await;
    let (a, b) = two_prs(&h, &session).await;
    let revision = h.control.app_state().sync.lock().unwrap().revision;

    let created = h
        .call(
            &session,
            "create_project",
            json!({"name": "Acme rollout", "description": "Q3", "color": "#3b82f6"}),
        )
        .await;
    let project = ok(&created)["project"].clone();
    let id = project["id"].as_i64().unwrap();
    assert_eq!(project["description"], "Q3");
    assert_eq!(project["color"], "#3b82f6");
    assert!(
        h.control.app_state().sync.lock().unwrap().revision > revision,
        "writes bump the revision"
    );

    let updated = h
        .call(
            &session,
            "update_project",
            json!({"project_id": id, "name": "Acme launch", "description": ""}),
        )
        .await;
    assert_eq!(ok(&updated)["project"]["name"], "Acme launch");
    assert_eq!(ok(&updated)["project"]["description"], "");
    assert_eq!(
        kind(
            &h.call(&session, "update_project", json!({"project_id": id}))
                .await
        ),
        "validation"
    );

    let added = h
        .call(
            &session,
            "add_prs_to_project",
            json!({"project_id": id, "pr_ids": [a, b]}),
        )
        .await;
    assert_eq!(ok(&added)["changed"], 2);
    let again = h
        .call(
            &session,
            "add_prs_to_project",
            json!({"project_id": id, "pr_ids": [a, b]}),
        )
        .await;
    assert_eq!(ok(&again)["changed"], 0);
    let removed = h
        .call(
            &session,
            "remove_prs_from_project",
            json!({"project_id": id, "pr_ids": [b]}),
        )
        .await;
    assert_eq!(ok(&removed)["removed"], 1);
    let read = h
        .call(&session, "get_project", json!({"project_id": id}))
        .await;
    let ids: Vec<i64> = ok(&read)["prs"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pr| pr["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [a]);
    let set = h
        .call(&session, "set_project_jira_links", json!({"project_id": id, "links": ["ACME-1", "https://acme.atlassian.net/browse/ACME-2"]}))
        .await;
    assert_eq!(ok(&set)["jira_links"].as_array().unwrap().len(), 2);
    let one = h
        .call(
            &session,
            "add_jira_link",
            json!({"project_id": id, "link": "acme-3"}),
        )
        .await;
    assert_eq!(ok(&one)["jira_link"]["key"], "ACME-3");
    let gone = h
        .call(
            &session,
            "remove_jira_link",
            json!({"project_id": id, "link": "ACME-2"}),
        )
        .await;
    assert_eq!(ok(&gone)["removed"]["key"], "ACME-2");
    assert_eq!(
        kind(
            &h.call(
                &session,
                "remove_jira_link",
                json!({"project_id": id, "link": "ACME-2"})
            )
            .await
        ),
        "not_found"
    );
    let keys: Vec<String> = h
        .store()
        .list_jira_links(ProjectId(id))
        .unwrap()
        .into_iter()
        .map(|link| link.key)
        .collect();
    assert_eq!(keys, ["ACME-1", "ACME-3"]);
    assert_eq!(
        kind(
            &h.call(
                &session,
                "add_jira_link",
                json!({"project_id": id, "link": "not a key"})
            )
            .await
        ),
        "validation"
    );

    let note = h
        .call(
            &session,
            "set_pr_note",
            json!({"pr_id": a, "note": "secret plan"}),
        )
        .await;
    assert_eq!(ok(&note)["note"], "secret plan");
    assert_eq!(
        h.store().pr_note(PrId(a)).unwrap().unwrap().note,
        "secret plan"
    );
    ok(&h
        .call(&session, "set_pr_note", json!({"pr_id": a, "note": ""}))
        .await);
    assert!(h.store().pr_note(PrId(a)).unwrap().is_none());

    let star = h
        .call(
            &session,
            "set_pr_starred",
            json!({"pr_id": a, "starred": true}),
        )
        .await;
    assert_eq!(ok(&star)["starred"], true);
    assert!(h.store().is_pr_starred(PrId(a)).unwrap());
    ok(&h
        .call(
            &session,
            "set_pr_starred",
            json!({"pr_id": a, "starred": false}),
        )
        .await);
    assert!(!h.store().is_pr_starred(PrId(a)).unwrap());

    let archived = h
        .call(&session, "archive_project", json!({"project_id": id}))
        .await;
    assert_eq!(ok(&archived)["project"]["archived"], true);
    let visible = h.call(&session, "list_projects", json!({})).await;
    assert!(
        ok(&visible)["projects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["id"] != id)
    );
    let back = h
        .call(&session, "unarchive_project", json!({"project_id": id}))
        .await;
    assert_eq!(ok(&back)["project"]["archived"], false);
    assert!(
        h.store().get_project(ProjectId(id)).unwrap().is_some(),
        "archive never deletes"
    );

    assert_eq!(
        kind(
            &h.call(&session, "archive_project", json!({"project_id": 99999}))
                .await
        ),
        "not_found"
    );
    assert_eq!(
        kind(
            &h.call(
                &session,
                "set_pr_starred",
                json!({"pr_id": 99999, "starred": true})
            )
            .await
        ),
        "not_found"
    );
    assert_eq!(
        kind(
            &h.call(
                &session,
                "add_prs_to_project",
                json!({"project_id": id, "pr_ids": [a, 99999]})
            )
            .await
        ),
        "not_found"
    );
    assert_eq!(
        kind(
            &h.call(
                &session,
                "add_prs_to_project",
                json!({"project_id": id, "pr_ids": []})
            )
            .await
        ),
        "validation"
    );
    assert_eq!(
        kind(
            &h.call(
                &session,
                "set_pr_starred",
                json!({"pr_id": a, "starred": true, "extra": 1})
            )
            .await
        ),
        "validation"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_audit_keeps_ids_and_summaries_never_note_text() {
    let h = harness().await;
    h.writes(true).await;
    let session = h.session().await;
    let (a, b) = two_prs(&h, &session).await;
    h.call(
        &session,
        "set_pr_note",
        json!({"pr_id": a, "note": "TOP-SECRET acme merger"}),
    )
    .await;
    h.call(
        &session,
        "set_pr_starred",
        json!({"pr_id": a, "starred": true}),
    )
    .await;
    h.call(
        &session,
        "add_prs_to_project",
        json!({"project_id": 1, "pr_ids": [a, b]}),
    )
    .await;
    h.call(
        &session,
        "set_pr_starred",
        json!({"pr_id": 99999, "starred": true}),
    )
    .await;

    let rows = h.store().mcp_audit(None, 50).unwrap();
    let dump = format!("{rows:?}");
    assert!(!dump.contains("TOP-SECRET"), "{dump}");
    assert!(!dump.contains("merger"), "{dump}");
    let note = rows.iter().find(|row| row.tool == "set_pr_note").unwrap();
    assert_eq!(note.fields, "note,pr_id");
    assert!(note.summary.starts_with("set note on "), "{}", note.summary);
    let star = rows
        .iter()
        .find(|row| row.tool == "set_pr_starred" && row.outcome == AuditOutcome::Allowed)
        .unwrap();
    assert!(
        star.summary.starts_with("starred ") && star.summary.contains('#'),
        "{}",
        star.summary
    );
    let added = rows
        .iter()
        .find(|row| row.tool == "add_prs_to_project")
        .unwrap();
    assert_eq!(added.summary, "added 2 PRs to project 1");
    assert_eq!(added.target_ids, format!("project:1,pr:{a},pr:{b}"));
    let failed = &rows[0];
    assert_eq!(failed.outcome, AuditOutcome::Error);
    assert_eq!(
        failed.error.as_deref(),
        Some("No pull request with id 99999")
    );

    let filtered = h.store().mcp_audit(Some(AuditOutcome::Error), 50).unwrap();
    assert_eq!(filtered.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_audit_is_bounded_through_the_server() {
    use pr_nexus_core::store::{AUDIT_KEEP_ROWS, AuditEntry};
    let h = harness().await;
    let session = h.session().await;
    let old = chrono::Utc::now() - chrono::Duration::days(91);
    for index in 0..AUDIT_KEEP_ROWS {
        h.store()
            .append_mcp_audit(AuditEntry {
                id: 0,
                at: if index == 0 { old } else { chrono::Utc::now() },
                client_name: "acme-agent".into(),
                tool: format!("filler{index}"),
                capability: "reads".into(),
                outcome: AuditOutcome::Allowed,
                fields: String::new(),
                target_ids: String::new(),
                summary: String::new(),
                error: None,
                duration_ms: 0,
            })
            .unwrap();
    }
    h.call(&session, "list_projects", json!({})).await;
    let rows = h.store().mcp_audit(None, 10_000).unwrap();
    assert_eq!(rows.len() as i64, AUDIT_KEEP_ROWS);
    assert_eq!(rows[0].tool, "list_projects");
    assert!(
        rows.iter().all(|row| row.tool != "filler0"),
        "older than 90 days"
    );
    assert_eq!(rows.last().unwrap().tool, "filler1");
    h.call(&session, "list_prs", json!({})).await;
    let rows = h.store().mcp_audit(None, 10_000).unwrap();
    assert_eq!(rows.len() as i64, AUDIT_KEEP_ROWS);
    assert_eq!(
        rows.last().unwrap().tool,
        "filler2",
        "the oldest row makes room"
    );
}
