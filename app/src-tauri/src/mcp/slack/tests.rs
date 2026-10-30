//! The Slack tools over the `acme` fixtures: two workspaces, `acme` and
//! the Grid one `acme-labs`. Nothing here reaches Slack.

use std::sync::Arc;

use chrono::Utc;
use pr_nexus_core::domain::{Account, AccountId, AccountKind, AuthMode};
use pr_nexus_core::error::ProviderError;
use pr_nexus_providers::slack_mcp::ToolCaller;
use pr_nexus_providers::slack_mcp::schema::ToolOutput;
use rmcp::model::JsonObject;
use serde_json::{Value, json};

use super::cursor::{self, Composite};
use super::elicit::{self, Outcome};
use super::fixtures::{Canned, Fixtures, GRID_ORG, GRID_TEAM};
use super::ids;
use super::sends::{Held, hold};
use super::session::{Answer, Ask, Caller};
use super::tools::unwrap_reply;
use super::upstream::{Exposed, Upstream};
use crate::error::AppError;
use crate::mcp::config::SlackSendMode;
use crate::mcp::{McpSettings, config};
use crate::state::AppState;

const ACME: &str = "T0ACME0001";

fn account(team: &str, domain: &str, name: &str) -> Account {
    Account {
        id: AccountId(0),
        kind: AccountKind::Slack,
        display_name: name.into(),
        host: format!("{domain}.slack.com"),
        external_id: format!("{team}:U0ALICE01"),
        slack_workspace: Some(domain.into()),
        login: "alice".into(),
        auth_mode: AuthMode::SlackMcpOauth,
        keychain_key: Some(format!("slack-{domain}")),
        created_at: Utc::now(),
    }
}

struct Env {
    state: Arc<AppState>,
    _dir: tempfile::TempDir,
}

/// Both workspaces signed in; `allowed` exposed, Slack reads on.
fn env(allowed: &[&str], upstream: Arc<dyn Upstream>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::for_tests(&dir.path().join("test.db"), true);
    state
        .store
        .insert_account(account(ACME, "acme", "Acme"))
        .unwrap();
    state
        .store
        .insert_account(account(GRID_TEAM, "acme-labs", "Acme Labs"))
        .unwrap();
    config::save(
        &state.store,
        &McpSettings {
            enabled: true,
            slack_reads: true,
            slack_workspaces: allowed.iter().map(|s| (*s).to_owned()).collect(),
            ..McpSettings::default()
        },
    )
    .unwrap();
    state.slack_proxy.set_upstream(upstream);
    Env {
        state: Arc::new(state),
        _dir: dir,
    }
}

fn both() -> Env {
    env(&[ACME, GRID_TEAM], Arc::new(Fixtures))
}

fn call(env: &Env, tool: &str, arguments: Value) -> Result<Value, AppError> {
    let spec = super::slack_tools()
        .into_iter()
        .chain(super::sends::send_tools())
        .find(|spec| spec.name == tool)
        .unwrap_or_else(|| panic!("no tool {tool}"));
    let Value::Object(arguments) = arguments else {
        panic!("arguments must be an object")
    };
    (spec.handler)(&env.state, arguments)
}

/// Fixtures, except `acme-labs` fails with a rate limit.
struct LabsDown;

impl Upstream for LabsDown {
    fn caller(&self, state: &AppState, ws: &Exposed) -> Result<Arc<dyn ToolCaller>, AppError> {
        if ws.domain == "acme-labs" {
            return Ok(Arc::new(Failing));
        }
        Fixtures.caller(state, ws)
    }
    fn enterprise(&self, state: &AppState, account: &Account) -> Option<String> {
        Fixtures.enterprise(state, account)
    }
    fn granted_scopes(&self, _state: &AppState, _account: &Account) -> Option<Vec<String>> {
        None
    }
}

/// Fixtures, but `acme-labs` signed in before the agent-read scopes.
struct LabsOldGrant;

/// Slack's own answer when a grant lacks `canvases:read`.
struct NoCanvasScope(Arc<dyn ToolCaller>);

impl ToolCaller for NoCanvasScope {
    fn call(&self, tool: &str, arguments: JsonObject) -> Result<ToolOutput, ProviderError> {
        if tool == "slack_read_canvas" {
            return Err(ProviderError::Forbidden("missing_scope".into()));
        }
        self.0.call(tool, arguments)
    }
}

impl Upstream for LabsOldGrant {
    fn caller(&self, state: &AppState, ws: &Exposed) -> Result<Arc<dyn ToolCaller>, AppError> {
        let inner = Fixtures.caller(state, ws)?;
        Ok(if ws.account.slack_team_id() == Some(GRID_TEAM) {
            Arc::new(NoCanvasScope(inner))
        } else {
            inner
        })
    }
    fn enterprise(&self, state: &AppState, account: &Account) -> Option<String> {
        Fixtures.enterprise(state, account)
    }
    fn granted_scopes(&self, _state: &AppState, account: &Account) -> Option<Vec<String>> {
        let all = pr_nexus_providers::slack_mcp::USER_SCOPES.iter();
        Some(if account.slack_team_id() == Some(GRID_TEAM) {
            all.take(9).map(|s| (*s).to_owned()).collect()
        } else {
            all.map(|s| (*s).to_owned()).collect()
        })
    }
}

struct Failing;

impl ToolCaller for Failing {
    fn call(&self, _tool: &str, _arguments: JsonObject) -> Result<ToolOutput, ProviderError> {
        Err(ProviderError::Unsupported("acme-labs is down".into()))
    }
}

#[test]
fn ids_are_qualified_only_where_slack_prints_them() {
    let text =
        "Channel: #x (ID: C0ACME0001)\nThanks <@U0ACME0001> in <#C0ACME0002>, see C0ACME0009";
    let out = ids::qualify_text(ACME, text);
    assert!(out.contains("(ID: T0ACME0001/C0ACME0001)"));
    assert!(out.contains("<@T0ACME0001/U0ACME0001>"));
    assert!(out.contains("<#T0ACME0001/C0ACME0002>"));
    assert!(out.ends_with("see C0ACME0009"), "{out}");
}

#[test]
fn ids_parse_qualified_bare_and_reject_the_rest() {
    let q = ids::parse("E0ACMELAB1/C0ACMELAB01").unwrap();
    assert_eq!(q.qualifier.as_deref(), Some(GRID_ORG));
    assert_eq!(q.id, "C0ACMELAB01");
    assert_eq!(ids::parse(" C0ACME0001 ").unwrap().qualifier, None);
    assert!(ids::parse("general").is_none());
    assert!(ids::parse("T0ACME0001/").is_none());
}

#[test]
fn permalinks_give_channel_message_and_thread() {
    let link = ids::parse_permalink(
        "https://acme.slack.com/archives/C0ACME0001/p1761055380000100?thread_ts=1761055000.000050&cid=C0ACME0001",
    )
    .unwrap();
    assert_eq!(link.label, "acme");
    assert!(!link.enterprise);
    assert_eq!(link.message_ts.as_deref(), Some("1761055380.000100"));
    assert_eq!(link.thread_ts.as_deref(), Some("1761055000.000050"));
    let grid =
        ids::parse_permalink("https://acme.enterprise.slack.com/archives/C0ACMELAB01").unwrap();
    assert!(grid.enterprise);
    assert_eq!(grid.message_ts, None);
    assert!(ids::parse_permalink("https://acme.example/archives/C0ACME0001").is_none());
}

#[test]
fn cursors_are_signed_and_bound_to_their_tool() {
    let key = [7u8; 32];
    let composite = Composite {
        tool: "slack_search_public".into(),
        pages: [(ACME.to_owned(), Some("page-2".to_owned()))].into(),
    };
    let text = cursor::encode(&key, &composite);
    let back = cursor::decode(&key, "slack_search_public", &text).unwrap();
    assert_eq!(back.pages, composite.pages);
    assert!(cursor::decode(&key, "slack_search_users", &text).is_err());
    assert!(cursor::decode(&[8u8; 32], "slack_search_public", &text).is_err());
    let (body, sig) = text.split_once('.').unwrap();
    let mut forged = body.to_owned();
    forged.push('A');
    assert!(cursor::decode(&key, "slack_search_public", &format!("{forged}.{sig}")).is_err());
}

#[test]
fn replies_unwrap_to_content_and_cursor() {
    let text = super::fixtures::reply("acme", "slack_search_public", &JsonObject::new()).unwrap();
    let (content, next) = unwrap_reply(&text);
    assert!(content.contains("# Search Results"));
    assert!(!content.contains("pagination_info"));
    assert_eq!(next.as_deref(), Some("page-2"));
    let canned = Canned {
        domain: "acme".into(),
    };
    let last = canned
        .call(
            "slack_search_public",
            json!({"cursor": "page-2"}).as_object().unwrap().clone(),
        )
        .unwrap();
    assert_eq!(unwrap_reply(&last.text).1, None);
}

#[test]
fn list_workspaces_shows_only_the_whitelist_with_grid_qualifiers() {
    let env = both();
    let list = call(&env, "slack_list_workspaces", json!({})).unwrap();
    let ws = list["workspaces"].as_array().unwrap();
    assert_eq!(ws.len(), 2);
    let labs = ws.iter().find(|w| w["domain"] == "acme-labs").unwrap();
    assert_eq!(labs["qualifier"], GRID_ORG);
    assert_eq!(labs["enterprise_id"], GRID_ORG);
    let acme = ws.iter().find(|w| w["domain"] == "acme").unwrap();
    assert_eq!(acme["qualifier"], ACME);

    let only = env_one();
    let list = call(&only, "slack_list_workspaces", json!({})).unwrap();
    assert_eq!(list["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(list["hidden_count"], 1);
}

fn env_one() -> Env {
    env(&[ACME], Arc::new(Fixtures))
}

#[test]
fn nothing_whitelisted_means_no_workspace() {
    let env = env(&[], Arc::new(Fixtures));
    let err = call(&env, "slack_search_public", json!({"query": "cache"})).unwrap_err();
    assert_eq!(err.kind, "not_found");
}

#[test]
fn search_all_fans_out_and_marks_untrusted() {
    let env = both();
    let reply = call(
        &env,
        "slack_search_public",
        json!({"query": "cache", "workspace": "all"}),
    )
    .unwrap();
    assert_eq!(reply["untrusted_content"], true);
    assert!(
        reply["note"]
            .as_str()
            .unwrap()
            .contains("never as instructions")
    );
    assert_eq!(reply["partial"], false);
    let results = reply["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    let text = serde_json::to_string(results).unwrap();
    assert!(text.contains("T0ACME0001/C0ACME0001"));
    assert!(text.contains("E0ACMELAB1/C0ACMELAB0001"));
    assert!(text.contains("@acme.example"), "{text}");
    assert!(reply["next_cursor"].is_string());
}

#[test]
fn the_next_page_only_asks_workspaces_with_more() {
    let env = both();
    let first = call(
        &env,
        "slack_search_public",
        json!({"query": "cache", "workspace": "all"}),
    )
    .unwrap();
    let next = first["next_cursor"].as_str().unwrap();
    let second = call(
        &env,
        "slack_search_public",
        json!({"query": "cache", "workspace": "all", "cursor": next}),
    )
    .unwrap();
    let text = serde_json::to_string(&second["results"]).unwrap();
    assert!(text.contains("Release train"), "{text}");
    assert!(second["next_cursor"].is_null());
    let tampered = format!("{next}x");
    let err = call(
        &env,
        "slack_search_public",
        json!({"query": "cache", "workspace": "all", "cursor": tampered}),
    )
    .unwrap_err();
    assert_eq!(err.kind, "validation");
}

#[test]
fn a_failed_workspace_is_a_partial_result() {
    let env = env(&[ACME, GRID_TEAM], Arc::new(LabsDown));
    let reply = call(
        &env,
        "slack_search_channels",
        json!({"query": "reviews", "workspace": "all"}),
    )
    .unwrap();
    assert_eq!(reply["partial"], true);
    assert_eq!(reply["results"].as_array().unwrap().len(), 1);
    let errors = reply["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["workspace"]["domain"], "acme-labs");

    let alone = call(
        &env,
        "slack_search_channels",
        json!({"query": "reviews", "workspace": "acme-labs"}),
    );
    assert!(alone.is_err());
}

#[test]
fn reads_need_a_qualified_id_when_several_are_exposed() {
    let env = both();
    let err = call(
        &env,
        "slack_read_channel",
        json!({"channel_id": "C0ACME0001"}),
    )
    .unwrap_err();
    assert_eq!(err.kind, "validation");
    let ok = call(
        &env,
        "slack_read_channel",
        json!({"channel_id": "T0ACME0001/C0ACME0001"}),
    )
    .unwrap();
    assert_eq!(ok["results"][0]["workspace"]["domain"], "acme");
    let thread = call(
        &env,
        "slack_read_thread",
        json!({"channel_id": "E0ACMELAB1/C0ACMELAB0001", "message_ts": "1761055380.000100"}),
    )
    .unwrap();
    assert_eq!(thread["results"][0]["workspace"]["domain"], "acme-labs");

    let one = env_one();
    assert!(
        call(
            &one,
            "slack_read_channel",
            json!({"channel_id": "C0ACME0001"})
        )
        .is_ok()
    );
    let hidden = call(
        &one,
        "slack_read_channel",
        json!({"channel_id": "E0ACMELAB1/C0ACMELAB0001"}),
    )
    .unwrap_err();
    assert_eq!(hidden.kind, "validation");
}

#[test]
fn profiles_pass_slack_fields_through_unfiltered() {
    let env = both();
    let reply = call(
        &env,
        "slack_read_user_profile",
        json!({"user_id": "T0ACME0001/U0ACME0001"}),
    )
    .unwrap();
    let content = reply["results"][0]["content"].as_str().unwrap();
    assert!(content.contains("Email: ada@acme.example"), "{content}");
    assert!(content.contains("Phone: +1 555 0100"), "{content}");
    assert!(
        content.contains("User ID: T0ACME0001/U0ACME0001"),
        "{content}"
    );
}

#[test]
fn resolve_maps_permalinks_without_calling_slack() {
    let env = env(&[ACME, GRID_TEAM], Arc::new(LabsDown));
    let reply = call(
        &env,
        "slack_resolve",
        json!({"reference": "https://acme.slack.com/archives/C0ACME0001/p1761055380000100"}),
    )
    .unwrap();
    let text = reply.to_string();
    assert!(text.contains("T0ACME0001/C0ACME0001"), "{text}");
    assert!(text.contains("1761055380.000100"), "{text}");
}

#[test]
fn unknown_arguments_and_workspaces_are_refused() {
    let env = both();
    assert!(
        call(
            &env,
            "slack_search_users",
            json!({"query": "ada", "team": "x"})
        )
        .is_err()
    );
    let err = call(
        &env,
        "slack_search_users",
        json!({"query": "ada", "workspace": "acme-elsewhere"}),
    )
    .unwrap_err();
    assert_eq!(err.kind, "validation");
}

#[test]
fn descriptions_name_no_account() {
    for spec in super::slack_tools() {
        let text = format!("{} {}", spec.title, spec.description).to_lowercase();
        assert!(!text.contains("acme"), "{}", spec.name);
        assert!(!text.contains("enterprise.slack.com"), "{}", spec.name);
    }
}

fn content(reply: &Value) -> &str {
    reply["results"][0]["content"].as_str().unwrap()
}

#[test]
fn reactions_members_and_emoji_are_qualified_and_unfiltered() {
    let env = both();
    let reactions = call(
        &env,
        "slack_get_reactions",
        json!({ "channel_id": format!("{ACME}/C0ACME0001"), "message_ts": "1761055380.000100" }),
    )
    .unwrap();
    assert!(content(&reactions).contains(&format!("{ACME}/U0ACME0002")));
    assert_eq!(reactions["untrusted_content"], true);

    let members = call(
        &env,
        "slack_list_channel_members",
        json!({ "channel_id": format!("{GRID_ORG}/C0ACMELAB0001") }),
    )
    .unwrap();
    let text = content(&members);
    assert!(text.contains(&format!("(ID: {GRID_ORG}/U0ACMELAB0001)")));
    assert!(text.contains("@acme-labs.example"), "{text}");
    let next = members["next_cursor"].as_str().unwrap().to_owned();
    let page2 = call(
        &env,
        "slack_list_channel_members",
        json!({ "channel_id": format!("{GRID_ORG}/C0ACMELAB0001"), "cursor": next }),
    )
    .unwrap();
    assert!(content(&page2).contains("U0ACMELAB0003"));

    let emoji = call(
        &env,
        "slack_search_emojis",
        json!({ "query": "ship", "workspace": "all" }),
    )
    .unwrap();
    assert_eq!(emoji["results"].as_array().unwrap().len(), 2);
}

#[test]
fn files_canvases_and_lists_take_qualified_file_ids() {
    let env = both();
    for (tool, field, id) in [
        ("slack_read_file", "file_id", "F0ACME0001"),
        ("slack_read_canvas", "canvas_id", "F0ACME0002"),
        ("slack_read_list", "list_id", "F0ACME0003"),
    ] {
        let reply = call(&env, tool, json!({ field: format!("{ACME}/{id}") })).unwrap();
        assert!(
            content(&reply).contains(&format!("(ID: {ACME}/{id})")),
            "{tool}: {reply}"
        );
        let bare = call(&env, tool, json!({ field: id })).unwrap_err();
        assert_eq!(bare.kind, "validation", "{tool}");
        let channel = call(&env, tool, json!({ field: format!("{ACME}/C0ACME0001") })).unwrap_err();
        assert!(
            channel.message.contains("file ID"),
            "{tool}: {}",
            channel.message
        );
    }
}

#[test]
fn private_search_is_its_own_capability() {
    let spec = super::slack_tools()
        .into_iter()
        .chain(super::sends::send_tools())
        .find(|s| s.name == "slack_search_public_and_private")
        .unwrap();
    assert_eq!(spec.capability, crate::mcp::Capability::SlackPrivateSearch);
    assert!(crate::mcp::Capability::SlackPrivateSearch.available());
    let reply = call(
        &both(),
        "slack_search_public_and_private",
        json!({ "query": "agenda" }),
    )
    .unwrap();
    assert!(content(&reply).contains(&format!("{ACME}/G0ACME0001")));
}

#[test]
fn an_old_grant_lists_missing_scopes_and_slack_answers_for_itself() {
    let env = env(&[ACME, GRID_TEAM], Arc::new(LabsOldGrant));
    let list = call(&env, "slack_list_workspaces", json!({})).unwrap();
    let rows = list["workspaces"].as_array().unwrap();
    let labs = rows.iter().find(|w| w["domain"] == "acme-labs").unwrap();
    let missing: Vec<&str> = labs["scopes_missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(missing.contains(&"reactions:read") && missing.contains(&"canvases:read"));
    assert!(
        labs["reauth_hint"]
            .as_str()
            .unwrap()
            .contains("Sign in again")
    );
    let acme = rows.iter().find(|w| w["domain"] == "acme").unwrap();
    assert_eq!(acme["scopes_missing"], json!([]));
    assert_eq!(acme["reauth_hint"], Value::Null);

    let err = call(
        &env,
        "slack_read_canvas",
        json!({ "canvas_id": format!("{GRID_ORG}/F0ACMELAB0002") }),
    )
    .unwrap_err();
    // The call reached Slack; its error comes back as Slack sent it.
    assert_eq!(err.kind, "forbidden");
    assert!(err.message.contains("missing_scope"), "{}", err.message);
    assert!(!err.message.contains("Sign in again"), "{}", err.message);
    call(
        &env,
        "slack_read_channel",
        json!({ "channel_id": format!("{GRID_ORG}/C0ACMELAB0001") }),
    )
    .unwrap();
}

// Slack sends: proposals, the auto-send list and the leak guard. The
// recorder stands in for Slack; nothing here reaches a real workspace.

type Calls = Arc<std::sync::Mutex<Vec<(String, String, JsonObject)>>>;

/// Fixtures that remember every call: (domain, tool, arguments).
struct Recording {
    calls: Calls,
    refuse_sends: bool,
}

struct RecordingCaller {
    domain: String,
    inner: Arc<dyn ToolCaller>,
    calls: Calls,
    refuse_sends: bool,
}

impl ToolCaller for RecordingCaller {
    fn call(&self, tool: &str, arguments: JsonObject) -> Result<ToolOutput, ProviderError> {
        self.calls
            .lock()
            .unwrap()
            .push((self.domain.clone(), tool.to_owned(), arguments.clone()));
        if self.refuse_sends && tool == "slack_send_message" {
            return Ok(ToolOutput {
                text: "not_in_channel".into(),
                structured: None,
                is_error: true,
            });
        }
        self.inner.call(tool, arguments)
    }
}

impl Upstream for Recording {
    fn caller(&self, state: &AppState, ws: &Exposed) -> Result<Arc<dyn ToolCaller>, AppError> {
        Ok(Arc::new(RecordingCaller {
            domain: ws.domain.clone(),
            inner: Fixtures.caller(state, ws)?,
            calls: self.calls.clone(),
            refuse_sends: self.refuse_sends,
        }))
    }
    fn enterprise(&self, state: &AppState, account: &Account) -> Option<String> {
        Fixtures.enterprise(state, account)
    }
    fn granted_scopes(&self, _state: &AppState, _account: &Account) -> Option<Vec<String>> {
        None
    }
}

const ACME_CHANNEL: &str = "T0ACME0001/C0ACME0001";
const LABS_CHANNEL: &str = "E0ACMELAB1/C0ACMELAB0001";

/// Both workspaces, Slack sends on, `auto` on the auto-send list.
fn sending(auto: &[&str], refuse_sends: bool) -> (Env, Calls) {
    let calls = Calls::default();
    let env = env(
        &[ACME, GRID_TEAM],
        Arc::new(Recording {
            calls: calls.clone(),
            refuse_sends,
        }),
    );
    let mut settings = config::load(&env.state.store).unwrap();
    settings.slack_sends = true;
    settings.slack_auto_send_channels = auto.iter().map(|s| (*s).to_owned()).collect();
    config::save(&env.state.store, &settings).unwrap();
    (env, calls)
}

fn writes(calls: &Calls) -> Vec<(String, String, JsonObject)> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, tool, _)| !tool.contains("read") && !tool.contains("search"))
        .cloned()
        .collect()
}

fn as_session<T>(id: &str, f: impl FnOnce() -> T) -> T {
    super::session::with(
        super::session::Caller {
            session: Some(id.into()),
            client: Some("test-agent".into()),
            ask: None,
        },
        f,
    )
}

#[test]
fn a_send_waits_for_approval_then_slack_answers_as_is() {
    let (env, calls) = sending(&[], false);
    let out = call(
        &env,
        "slack_send_message",
        json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "Ship it", "thread_ts": "1761055380.000100" }),
    )
    .unwrap();
    assert_eq!(out["status"], "pending");
    assert_eq!(out["held"]["reason"], "not_listed");
    assert!(
        writes(&calls).is_empty(),
        "nothing reaches Slack before approval"
    );

    let id = out["proposal_id"].as_u64().unwrap();
    let listed = super::sends::list(&env.state);
    assert_eq!(listed[0].channel_id, ACME_CHANNEL);
    assert_eq!(listed[0].arguments["message"], "Ship it");

    let sent = super::sends::decide(&env.state, id, true).unwrap();
    assert_eq!(sent.status, super::sends::Status::Sent);
    let made = writes(&calls);
    assert_eq!(made.len(), 1);
    let (domain, tool, args) = &made[0];
    assert_eq!(
        (domain.as_str(), tool.as_str()),
        ("acme", "slack_send_message")
    );
    assert_eq!(args["channel_id"], "C0ACME0001");
    assert_eq!(args["message"], "Ship it");
    assert_eq!(args["thread_ts"], "1761055380.000100");
    let link = sent
        .link
        .clone()
        .expect("a sent reply keeps its Slack link");
    assert!(
        link.starts_with("https://acme.slack.com/archives/C0ACME0001/p"),
        "{link}"
    );
    assert!(
        link.ends_with("?thread_ts=1761055380.000100&cid=C0ACME0001"),
        "{link}"
    );
    let content = sent.result.unwrap()["results"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        content.contains("Message sent to <#T0ACME0001/C0ACME0001>"),
        "{content}"
    );

    // A second approval does not send twice.
    assert_eq!(
        super::sends::decide(&env.state, id, true).unwrap_err().kind,
        "conflict"
    );
    assert_eq!(writes(&calls).len(), 1);
}

#[test]
fn every_write_needs_one_named_workspace_that_owns_the_channel() {
    let (env, calls) = sending(&[ACME_CHANNEL], false);
    let missing = call(
        &env,
        "slack_add_reaction",
        json!({ "channel_id": ACME_CHANNEL, "message_ts": "1.2", "emoji": "eyes" }),
    )
    .unwrap_err();
    assert_eq!(missing.kind, "validation");
    let all = call(
        &env,
        "slack_send_message",
        json!({ "workspace": "all", "channel_id": ACME_CHANNEL, "message": "x" }),
    )
    .unwrap_err();
    assert_eq!(all.kind, "validation");
    let crossed = call(
        &env,
        "slack_send_message_draft",
        json!({ "workspace": "acme", "channel_id": LABS_CHANNEL, "message": "x" }),
    )
    .unwrap_err();
    assert!(
        crossed.message.contains("belongs to workspace E0ACMELAB1"),
        "{}",
        crossed.message
    );
    assert!(writes(&calls).is_empty());
}

#[test]
fn a_listed_channel_sends_at_once_with_slacks_arguments() {
    let (env, calls) = sending(&[ACME_CHANNEL], false);
    for (tool, args) in [
        ("slack_send_message", json!({ "message": "Done" })),
        (
            "slack_schedule_message",
            json!({ "message": "Later", "post_at": 1_800_000_000 }),
        ),
        ("slack_send_message_draft", json!({ "message": "Draft" })),
        (
            "slack_add_reaction",
            json!({ "message_ts": "1761055380.000100", "emoji": "eyes" }),
        ),
    ] {
        let mut args = args;
        args["workspace"] = json!("T0ACME0001");
        args["channel_id"] = json!(ACME_CHANNEL);
        let out = call(&env, tool, args).unwrap();
        assert_eq!(out["status"], "sent", "{tool}: {out}");
        assert_eq!(out["auto_sent"], true);
    }
    let made = writes(&calls);
    let tools: Vec<&str> = made.iter().map(|(_, tool, _)| tool.as_str()).collect();
    assert_eq!(
        tools,
        [
            "slack_send_message",
            "slack_schedule_message",
            "slack_send_message_draft",
            "slack_add_reaction"
        ]
    );
    assert_eq!(made[1].2["post_at"], 1_800_000_000);
    assert_eq!(made[3].2["emoji"], "eyes");
    assert!(
        made.iter()
            .all(|(_, _, a)| a["channel_id"] == "C0ACME0001" && !a.contains_key("workspace"))
    );
}

#[test]
fn reading_another_workspace_blocks_auto_send_for_that_session() {
    let (env, calls) = sending(&[ACME_CHANNEL], false);
    let held = as_session("s1", || {
        call(
            &env,
            "slack_read_channel",
            json!({ "channel_id": LABS_CHANNEL }),
        )
        .unwrap();
        call(
            &env,
            "slack_send_message",
            json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "Summary" }),
        )
        .unwrap()
    });
    assert_eq!(held["status"], "pending");
    assert_eq!(held["held"]["reason"], "read_other_workspace");
    assert_eq!(held["held"]["workspaces"], json!([GRID_ORG]));
    assert!(writes(&calls).is_empty());

    // Another session that read only acme still auto-sends.
    let fresh = as_session("s2", || {
        call(
            &env,
            "slack_read_channel",
            json!({ "channel_id": ACME_CHANNEL }),
        )
        .unwrap();
        call(
            &env,
            "slack_send_message",
            json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "Hi" }),
        )
        .unwrap()
    });
    assert_eq!(fresh["status"], "sent");

    // The first session only sees its own proposal.
    let id = held["proposal_id"].clone();
    assert!(
        as_session("s2", || call(
            &env,
            "slack_get_proposal",
            json!({ "proposal_id": id })
        ))
        .is_err()
    );
    let mine = as_session("s1", || {
        call(
            &env,
            "slack_get_proposal",
            json!({ "proposal_id": held["proposal_id"] }),
        )
    })
    .unwrap();
    assert_eq!(mine["status"], "pending");
    assert_eq!(
        super::sends::list(&env.state)[1].client.as_deref(),
        Some("test-agent")
    );
}

#[test]
fn rejected_proposals_never_reach_slack_and_slack_errors_come_back_as_is() {
    let (env, calls) = sending(&[], true);
    let first = call(
        &env,
        "slack_send_message",
        json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "No" }),
    )
    .unwrap();
    let id = first["proposal_id"].as_u64().unwrap();
    let rejected = super::sends::decide(&env.state, id, false).unwrap();
    assert_eq!(rejected.status, super::sends::Status::Rejected);
    assert!(super::sends::decide(&env.state, id, true).is_err());
    assert!(writes(&calls).is_empty());

    let second = call(
        &env,
        "slack_send_message",
        json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "Yes" }),
    )
    .unwrap();
    let failed =
        super::sends::decide(&env.state, second["proposal_id"].as_u64().unwrap(), true).unwrap();
    assert_eq!(failed.status, super::sends::Status::Failed);
    assert!(
        failed.error.unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("not_in_channel")
    );
}

#[test]
fn approval_needs_slack_sends_still_on() {
    let (env, calls) = sending(&[], false);
    let out = call(
        &env,
        "slack_send_message",
        json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "x" }),
    )
    .unwrap();
    let mut settings = config::load(&env.state.store).unwrap();
    settings.slack_sends = false;
    config::save(&env.state.store, &settings).unwrap();
    let err =
        super::sends::decide(&env.state, out["proposal_id"].as_u64().unwrap(), true).unwrap_err();
    assert_eq!(err.kind, "forbidden");
    assert!(writes(&calls).is_empty());
}

fn with_mode(env: &Env, mode: SlackSendMode) {
    let mut settings = config::load(&env.state.store).unwrap();
    settings.slack_send_mode = mode;
    config::save(&env.state.store, &settings).unwrap();
}

#[test]
fn hold_follows_each_send_mode() {
    let other = || vec!["E0ACMELAB1".to_owned()];
    // Approve everything: listed or not, read elsewhere or not.
    assert_eq!(
        hold(SlackSendMode::ApproveAll, vec![], || true),
        Some(Held::ApproveAll)
    );
    assert_eq!(
        hold(SlackSendMode::ApproveAll, other(), || false),
        Some(Held::ApproveAll)
    );
    // Automatic: never held, even after reading another workspace.
    assert_eq!(hold(SlackSendMode::Automatic, vec![], || false), None);
    assert_eq!(hold(SlackSendMode::Automatic, other(), || false), None);
    // Allowlist: listed channels only, and the cross-workspace safeguard.
    assert_eq!(hold(SlackSendMode::Allowlist, vec![], || true), None);
    assert_eq!(
        hold(SlackSendMode::Allowlist, vec![], || false),
        Some(Held::NotListed)
    );
    assert_eq!(
        hold(SlackSendMode::Allowlist, other(), || true),
        Some(Held::ReadOtherWorkspace {
            workspaces: other()
        })
    );
}

#[test]
fn approve_all_ignores_the_allowlist() {
    let (env, calls) = sending(&[ACME_CHANNEL], false);
    with_mode(&env, SlackSendMode::ApproveAll);
    let out = call(
        &env,
        "slack_send_message",
        json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "Hi" }),
    )
    .unwrap();
    assert_eq!(out["status"], "pending");
    assert_eq!(out["held"]["reason"], "approve_all");
    assert!(writes(&calls).is_empty());
}

#[test]
fn automatic_sends_everything_at_once_even_after_reading_elsewhere() {
    let (env, calls) = sending(&[], false);
    with_mode(&env, SlackSendMode::Automatic);
    let out = as_session("s1", || {
        call(
            &env,
            "slack_read_channel",
            json!({ "channel_id": LABS_CHANNEL }),
        )
        .unwrap();
        call(
            &env,
            "slack_send_message",
            json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "Hi" }),
        )
        .unwrap()
    });
    assert_eq!(out["status"], "sent");
    assert_eq!(out["auto_sent"], true);
    assert!(out["held"].is_null());
    assert!(
        out["link"]
            .as_str()
            .unwrap()
            .starts_with("https://acme.slack.com/archives/C0ACME0001/p"),
        "{out}"
    );
    assert!(
        out["message"]
            .as_str()
            .unwrap()
            .contains("without approval")
    );
    assert_eq!(writes(&calls).len(), 1);
}

// MCP elicitation: asking in the agent (SPEC §10.1.4).

/// Answers every confirmation with `answer` and keeps what it was shown.
struct FakeAsk {
    answer: Answer,
    shown: std::sync::Mutex<Vec<String>>,
}

fn asking(answer: Answer) -> Arc<FakeAsk> {
    Arc::new(FakeAsk {
        answer,
        shown: Default::default(),
    })
}

impl Ask for FakeAsk {
    fn ask(&self, message: &str) -> Answer {
        self.shown.lock().unwrap().push(message.to_owned());
        self.answer.clone()
    }
}

fn send_as(env: &Env, ask: Option<Arc<FakeAsk>>, arguments: Value) -> Value {
    super::session::with(
        Caller {
            session: Some("s1".into()),
            client: Some("acme-agent".into()),
            ask: ask.map(|a| a as Arc<dyn Ask>),
        },
        || call(env, "slack_send_message", arguments),
    )
    .unwrap()
}

fn hi() -> Value {
    json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "Ship it?" })
}

#[test]
fn asks_only_for_held_writes_when_enabled_and_supported() {
    let held = Some(Held::ApproveAll);
    assert!(elicit::should_ask(held.as_ref(), true, true));
    assert!(
        !elicit::should_ask(held.as_ref(), true, false),
        "no capability"
    );
    assert!(
        !elicit::should_ask(held.as_ref(), false, true),
        "turned off"
    );
    // Automatic mode holds nothing, so it never asks.
    assert!(!elicit::should_ask(None, true, true));
}

#[test]
fn each_answer_has_one_outcome() {
    assert_eq!(elicit::outcome(Answer::Accept), Outcome::Send);
    assert_eq!(elicit::outcome(Answer::Decline), Outcome::Drop);
    assert_eq!(elicit::outcome(Answer::Cancel), Outcome::Drop);
    assert_eq!(
        elicit::outcome(Answer::Unanswered("timed out".into())),
        Outcome::Wait("timed out".into())
    );
}

#[test]
fn accepted_in_the_agent_sends_now_and_says_where() {
    let (env, calls) = sending(&[], false);
    with_mode(&env, SlackSendMode::ApproveAll);
    let ask = asking(Answer::Accept);
    let out = send_as(&env, Some(ask.clone()), hi());
    assert_eq!(out["status"], "sent");
    assert_eq!(out["decided_in"], "acme-agent");
    assert!(
        out["message"]
            .as_str()
            .unwrap()
            .contains("approved it in acme-agent")
    );
    assert_eq!(writes(&calls).len(), 1);
    let shown = ask.shown.lock().unwrap();
    assert_eq!(shown.len(), 1);
    for part in [
        "Send this Slack message",
        "Acme",
        "T0ACME0001",
        "C0ACME0001",
        "Ship it?",
    ] {
        assert!(shown[0].contains(part), "{part} missing from {}", shown[0]);
    }
    let kept = &super::sends::list(&env.state)[0];
    assert_eq!(kept.decided_in.as_deref(), Some("acme-agent"));
    assert!(!kept.auto_sent);
}

#[test]
fn declined_or_cancelled_in_the_agent_is_never_sent() {
    for answer in [Answer::Decline, Answer::Cancel] {
        let (env, calls) = sending(&[], false);
        let out = send_as(&env, Some(asking(answer)), hi());
        assert_eq!(out["status"], "rejected");
        assert_eq!(out["decided_in"], "acme-agent");
        assert!(
            out["message"]
                .as_str()
                .unwrap()
                .contains("declined it in acme-agent")
        );
        assert!(writes(&calls).is_empty());
    }
}

#[test]
fn no_answer_falls_back_to_approval_in_the_app() {
    let (env, calls) = sending(&[], false);
    let out = send_as(
        &env,
        Some(asking(Answer::Unanswered("timed out".into()))),
        hi(),
    );
    assert_eq!(out["status"], "pending");
    assert!(out["decided_in"].is_null());
    assert!(
        out["message"]
            .as_str()
            .unwrap()
            .contains("waits for approval in PR Nexus")
    );
    assert!(writes(&calls).is_empty());
    let id = out["proposal_id"].as_u64().unwrap();
    assert_eq!(
        super::sends::decide(&env.state, id, true).unwrap().status,
        super::sends::Status::Sent
    );
}

#[test]
fn without_the_capability_or_the_setting_nothing_is_asked() {
    let (env, calls) = sending(&[], false);
    let out = send_as(&env, None, hi());
    assert_eq!(out["status"], "pending");

    let mut settings = config::load(&env.state.store).unwrap();
    settings.slack_elicit = false;
    config::save(&env.state.store, &settings).unwrap();
    let ask = asking(Answer::Accept);
    let out = send_as(&env, Some(ask.clone()), hi());
    assert_eq!(out["status"], "pending");
    assert!(ask.shown.lock().unwrap().is_empty());
    assert!(writes(&calls).is_empty());
}

#[test]
fn automatic_mode_never_asks() {
    let (env, calls) = sending(&[], false);
    with_mode(&env, SlackSendMode::Automatic);
    let ask = asking(Answer::Decline);
    let out = send_as(&env, Some(ask.clone()), hi());
    assert_eq!(out["status"], "sent");
    assert!(ask.shown.lock().unwrap().is_empty());
    assert_eq!(writes(&calls).len(), 1);
}

#[test]
fn a_cross_workspace_hold_can_be_approved_in_the_agent() {
    let (env, calls) = sending(&[ACME_CHANNEL], false);
    let out = super::session::with(
        Caller {
            session: Some("s1".into()),
            client: Some("acme-agent".into()),
            ask: Some(asking(Answer::Accept)),
        },
        || {
            call(
                &env,
                "slack_read_channel",
                json!({ "channel_id": LABS_CHANNEL }),
            )
            .unwrap();
            call(&env, "slack_send_message", hi()).unwrap()
        },
    );
    assert_eq!(out["status"], "sent");
    assert_eq!(out["held"]["reason"], "read_other_workspace");
    assert_eq!(writes(&calls).len(), 1);
}

#[test]
fn older_saves_ask_in_the_agent() {
    let saved: McpSettings = serde_json::from_value(json!({ "enabled": true })).unwrap();
    assert!(saved.slack_elicit);
}

/// A real MCP client over HTTP that declares form elicitation and answers
/// every confirmation with `action`.
struct Agent(rmcp::model::ElicitationAction);

impl rmcp::ClientHandler for Agent {
    async fn create_elicitation(
        &self,
        request: rmcp::model::ElicitRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleClient>,
    ) -> Result<rmcp::model::ElicitResult, rmcp::ErrorData> {
        let rmcp::model::ElicitRequestParams::FormElicitationParams { message, .. } = request
        else {
            panic!("expected a form elicitation");
        };
        assert!(message.contains("Ship it?"), "{message}");
        let accept = self.0 == rmcp::model::ElicitationAction::Accept;
        let result = rmcp::model::ElicitResult::new(self.0.clone());
        Ok(if accept {
            result.with_content(json!({}))
        } else {
            result
        })
    }

    fn get_info(&self) -> rmcp::model::ClientConfig {
        rmcp::model::ClientConfig::new(
            rmcp::model::ClientCapabilities::builder()
                .enable_elicitation()
                .build(),
            rmcp::model::Implementation::new("acme-agent", "1.0"),
        )
    }
}

#[derive(Default)]
struct Kept(std::sync::Mutex<Vec<crate::mcp::audit::AuditEvent>>);

impl crate::mcp::audit::AuditSink for Kept {
    fn record(&self, event: &crate::mcp::audit::AuditEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

/// Send `hi()` through the HTTP server as an agent answering `action`.
async fn send_over_http(
    action: rmcp::model::ElicitationAction,
) -> (Value, Calls, Vec<crate::mcp::audit::AuditEvent>) {
    use rmcp::ServiceExt;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let (env, calls) = sending(&[], false);
    let kept = Arc::new(Kept::default());
    let control =
        crate::mcp::control::McpControl::with_audit(env.state.clone(), kept.clone(), None).unwrap();
    let view = control
        .update(config::load(&env.state.store).unwrap())
        .await
        .unwrap();
    let url = view.status.url.unwrap();
    let token = control.token().await.unwrap();
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
    );
    let client = Agent(action).serve(transport).await.unwrap();
    let mut request = rmcp::model::CallToolRequestParams::new("slack_send_message");
    request.arguments = hi().as_object().cloned();
    let reply = client.call_tool(request).await.unwrap();
    let out = reply.structured_content.clone().expect("structured reply");
    client.cancel().await.unwrap();
    let mut off = config::load(&env.state.store).unwrap();
    off.enabled = false;
    control.update(off).await.unwrap();
    drop(env);
    let events = kept.0.lock().unwrap().clone();
    (out, calls, events)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_elicitation_approves_over_http() {
    let (out, calls, events) = send_over_http(rmcp::model::ElicitationAction::Accept).await;
    let event = events
        .iter()
        .find(|event| event.tool == "slack_send_message")
        .unwrap();
    assert_eq!(event.link.as_deref(), out["link"].as_str());
    assert_eq!(out["status"], "sent", "{out}");
    assert_eq!(out["decided_in"], "acme-agent");
    assert!(
        out["link"]
            .as_str()
            .unwrap()
            .starts_with("https://acme.slack.com/archives/"),
        "{out}"
    );
    assert_eq!(writes(&calls).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_elicitation_declines_over_http() {
    let (out, calls, events) = send_over_http(rmcp::model::ElicitationAction::Decline).await;
    assert!(events.iter().all(|event| event.link.is_none()));
    assert_eq!(out["status"], "rejected", "{out}");
    assert!(writes(&calls).is_empty());
}

/// The in-app approval line names the channel, cached name first.
#[tokio::test(flavor = "multi_thread")]
async fn the_in_app_approval_says_where_it_went() {
    let (env, _calls) = sending(&[], false);
    env.state
        .store
        .upsert_slack_channel(pr_nexus_core::domain::SlackChannel {
            slack_workspace: "acme".into(),
            channel_id: "C0ACME0001".into(),
            name: "release".into(),
            is_private: false,
            is_archived: false,
            is_member: true,
            fetched_at: chrono::Utc::now(),
        })
        .unwrap();
    let kept = Arc::new(Kept::default());
    let control =
        crate::mcp::control::McpControl::with_audit(env.state.clone(), kept.clone(), None).unwrap();
    let approve = async |arguments: Value| {
        let out = call(&env, "slack_send_message", arguments).unwrap();
        let id = out["proposal_id"].as_u64().unwrap();
        control.decide(id, true).await.unwrap();
        let summary = kept.0.lock().unwrap().last().unwrap().summary.clone();
        (id, summary)
    };
    let (id, summary) = approve(hi()).await;
    assert_eq!(
        summary,
        format!("user approved Slack proposal {id}, sent in #release")
    );
    let (id, summary) = approve(
        json!({ "workspace": "acme", "channel_id": ACME_CHANNEL, "message": "Done", "thread_ts": "1700000000.000100" }),
    )
    .await;
    assert_eq!(
        summary,
        format!("user approved Slack proposal {id}, replied in a thread in #release")
    );
}
