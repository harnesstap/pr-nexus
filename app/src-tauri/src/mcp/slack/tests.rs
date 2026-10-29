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
use super::fixtures::{Canned, Fixtures, GRID_ORG, GRID_TEAM};
use super::ids;
use super::tools::unwrap_reply;
use super::upstream::{Exposed, Upstream};
use crate::error::AppError;
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
    state: AppState,
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
    Env { state, _dir: dir }
}

fn both() -> Env {
    env(&[ACME, GRID_TEAM], Arc::new(Fixtures))
}

fn call(env: &Env, tool: &str, arguments: Value) -> Result<Value, AppError> {
    let spec = super::slack_tools()
        .into_iter()
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
fn contacts_are_masked() {
    let text = "From: Ada (ID: U0ACME0001) <ada@acme.example>\nEmail: ada@acme.example\nPhone: +1 555 0100\nTitle: Staff";
    let out = ids::mask_contacts(text);
    assert!(!out.contains("ada@acme.example"), "{out}");
    assert!(!out.contains("555"), "{out}");
    assert!(out.contains("Email: [hidden by PR Nexus]"));
    assert!(out.contains("Title: Staff"));
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
fn search_all_fans_out_marks_untrusted_and_masks() {
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
    assert!(!text.contains("@acme.example"), "{text}");
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
fn profiles_hide_email_and_phone() {
    let env = both();
    let reply = call(
        &env,
        "slack_read_user_profile",
        json!({"user_id": "T0ACME0001/U0ACME0001"}),
    )
    .unwrap();
    let content = reply["results"][0]["content"].as_str().unwrap();
    assert!(content.contains("Email: [hidden by PR Nexus]"));
    assert!(content.contains("Phone: [hidden by PR Nexus]"));
    assert!(!content.contains("555"));
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
