//! `request_reviews` over the demo seed and a fake Slack that only records
//! what it would post. Nothing here reaches Slack.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use pr_nexus_core::domain::{
    Account, AccountId, AccountKind, AuthMode, ChatKind, PrId, RequestStatus,
};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{
    ChatChannel, ChatProvider, ChatUser, OutgoingMessage, Page, PostedMessage,
};
use serde_json::{Value, json};

use super::fixtures::Fixtures;
use super::reviews::{FAKE, review_tools};
use super::sends;
use super::session::{self, Answer, Ask, Caller};
use crate::error::AppError;
use crate::mcp::McpSettings;
use crate::mcp::config::{self, SlackSendMode};
use crate::services::prs::{self, PrFilter, PrStateFilter};
use crate::services::review::ChatSource;
use crate::state::AppState;

const TEAM: &str = "T0ACME0001";
const CHANNEL: &str = "C0ACME0001";

#[derive(Default)]
struct FakeSlack {
    posts: Mutex<Vec<(String, Option<String>, String)>>,
}

struct FakeChat(Arc<FakeSlack>);

impl ChatProvider for FakeChat {
    fn kind(&self) -> ChatKind {
        ChatKind::Slack
    }
    fn whoami(&self) -> Result<ChatUser, ProviderError> {
        Ok(ChatUser {
            id: "U0ACME0001".into(),
            name: "ada".into(),
        })
    }
    fn list_channels(&self, _: Option<String>) -> Result<Page<ChatChannel>, ProviderError> {
        Err(ProviderError::Unsupported("fake".into()))
    }
    fn channel_info(&self, channel_id: &str) -> Result<ChatChannel, ProviderError> {
        Ok(ChatChannel {
            id: channel_id.into(),
            name: "team-reviews".into(),
        })
    }
    fn post(&self, message: &OutgoingMessage<'_>) -> Result<PostedMessage, ProviderError> {
        let mut posts = self.0.posts.lock().unwrap();
        posts.push((
            message.channel_id.to_owned(),
            message.thread_ts.map(str::to_owned),
            message.text.to_owned(),
        ));
        let ts = format!("1700000000.{:06}", posts.len());
        Ok(PostedMessage {
            channel_id: message.channel_id.to_owned(),
            permalink: Some(format!(
                "https://acme.slack.com/archives/{}/p{}",
                message.channel_id,
                ts.replace('.', "")
            )),
            thread_ts: message.thread_ts.map(str::to_owned),
            workspace: Some("acme".into()),
            ts,
        })
    }
    fn account_id(&self) -> AccountId {
        AccountId(1)
    }
}

struct Source(Arc<FakeSlack>);

impl ChatSource for Source {
    fn chat(&self, _: &AppState, _: &str) -> Result<Arc<dyn ChatProvider>, AppError> {
        Ok(Arc::new(FakeChat(Arc::clone(&self.0))))
    }
}

struct Env {
    state: AppState,
    slack: Arc<FakeSlack>,
    _dir: tempfile::TempDir,
}

impl Drop for Env {
    fn drop(&mut self) {
        FAKE.with(|cell| *cell.borrow_mut() = None);
    }
}

fn env(mode: SlackSendMode) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::for_tests(&dir.path().join("test.db"), true);
    crate::demo::seed_if_empty(&state.store).unwrap();
    state
        .store
        .upsert_account(Account {
            id: AccountId(0),
            kind: AccountKind::Slack,
            display_name: "Acme".into(),
            host: "acme.slack.com".into(),
            external_id: format!("{TEAM}:U0ACME0001"),
            slack_workspace: Some("acme".into()),
            login: "ada".into(),
            auth_mode: AuthMode::SlackMcpOauth,
            keychain_key: Some(format!("slack:{TEAM}:U0ACME0001")),
            created_at: Utc::now(),
        })
        .unwrap();
    config::save(
        &state.store,
        &McpSettings {
            enabled: true,
            slack_reads: true,
            slack_sends: true,
            slack_send_mode: mode,
            slack_workspaces: vec![TEAM.to_owned()],
            ..McpSettings::default()
        },
    )
    .unwrap();
    state.slack_proxy.set_upstream(Arc::new(Fixtures));
    let slack = Arc::new(FakeSlack::default());
    let source: Arc<dyn ChatSource> = Arc::new(Source(Arc::clone(&slack)));
    FAKE.with(|cell| *cell.borrow_mut() = Some(source));
    Env {
        state,
        slack,
        _dir: dir,
    }
}

fn open_prs(state: &AppState) -> Vec<i64> {
    prs::list_prs(
        state,
        &PrFilter {
            state: Some(PrStateFilter::Open),
            ..PrFilter::default()
        },
    )
    .unwrap()
    .items
    .iter()
    .map(|item| item.row.id)
    .collect()
}

struct FakeAsk {
    answer: Answer,
    shown: Mutex<Vec<String>>,
}

impl Ask for FakeAsk {
    fn ask(&self, message: &str) -> Answer {
        self.shown.lock().unwrap().push(message.to_owned());
        self.answer.clone()
    }
}

fn call(env: &Env, ask: Option<Arc<FakeAsk>>, arguments: Value) -> Value {
    let Value::Object(arguments) = arguments else {
        panic!("arguments")
    };
    let handler = review_tools()[0].handler;
    session::with(
        Caller {
            session: Some("s1".into()),
            client: Some("acme-agent".into()),
            ask: ask.map(|a| a as Arc<dyn Ask>),
        },
        || handler(&env.state, arguments),
    )
    .unwrap()
}

fn to_channel(env: &Env, extra: Value) -> Value {
    let mut args = json!({
        "prs": open_prs(&env.state),
        "destination": format!("{CHANNEL}@acme"),
    });
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    args
}

fn posts(env: &Env) -> usize {
    env.slack.posts.lock().unwrap().len()
}

#[test]
fn preview_renders_one_card_per_channel_and_sends_nothing() {
    let env = env(SlackSendMode::Automatic);
    let out = call(&env, None, to_channel(&env, json!({ "preview": true })));
    assert_eq!(out["preview"], true);
    let cards = out["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1, "one destination, one card: {out}");
    assert_eq!(
        cards[0]["destination"]["channel_id"],
        format!("{TEAM}/{CHANNEL}")
    );
    let messages = cards[0]["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
    for message in messages {
        assert_eq!(message["action"], "request");
        let text = message["text"].as_str().unwrap();
        for pr in message["prs"].as_array().unwrap() {
            assert!(text.contains(&format!("/pull/{}", pr["number"])), "{text}");
        }
    }
    assert_eq!(posts(&env), 0);
    assert!(
        sends::list(&env.state).is_empty(),
        "a preview proposes nothing"
    );
}

#[test]
fn automatic_sends_one_message_per_channel_and_tracks_the_requests() {
    let env = env(SlackSendMode::Automatic);
    let preview = call(&env, None, to_channel(&env, json!({ "preview": true })));
    let planned = preview["cards"][0]["messages"].as_array().unwrap().len();
    let out = call(&env, None, to_channel(&env, json!({})));
    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{out}");
    assert_eq!(results[0]["status"], "sent");
    assert_eq!(results[0]["auto_sent"], true);
    assert!(
        results[0]["link"]
            .as_str()
            .unwrap()
            .starts_with("https://acme.slack.com/archives/")
    );
    assert_eq!(out["status"], "sent");
    assert_eq!(out["link"], results[0]["link"]);
    assert_eq!(posts(&env), planned);
    // Tracked like the app's batch: each PR now has an active request.
    let sent: Vec<i64> = results[0]["prs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pr| pr["pr_id"].as_i64().unwrap())
        .collect();
    assert!(!sent.is_empty());
    for pr in &sent {
        let requests = env.state.store.list_review_requests(PrId(*pr)).unwrap();
        assert!(
            requests.iter().any(|r| r.status == RequestStatus::Active),
            "PR {pr} has no active request"
        );
    }
    // A new request is no longer due: strict `request` skips them all.
    let again = call(
        &env,
        None,
        to_channel(&env, json!({ "mode": "request", "preview": true })),
    );
    assert!(again["cards"].as_array().unwrap().is_empty(), "{again}");
    let skipped = again["skipped"].as_array().unwrap();
    assert!(
        sent.iter()
            .all(|pr| skipped.iter().any(|s| s["pr_id"] == *pr))
    );
    // And `reminder` keeps them.
    let remind = call(
        &env,
        None,
        to_channel(&env, json!({ "mode": "reminder", "preview": true })),
    );
    let actions: Vec<&str> = remind["cards"][0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["action"].as_str().unwrap())
        .collect();
    assert!(
        !actions.is_empty() && actions.iter().all(|a| *a != "request"),
        "{actions:?}"
    );
}

#[test]
fn reminder_mode_skips_prs_without_a_request() {
    let env = env(SlackSendMode::Automatic);
    let out = call(&env, None, to_channel(&env, json!({ "mode": "reminder" })));
    assert!(out["results"].as_array().unwrap().is_empty(), "{out}");
    assert_eq!(posts(&env), 0);
    assert!(!out["skipped"].as_array().unwrap().is_empty());
}

#[test]
fn approve_all_leaves_it_pending_until_approved_in_the_app() {
    let env = env(SlackSendMode::ApproveAll);
    let out = call(&env, None, to_channel(&env, json!({})));
    let result = &out["results"][0];
    assert_eq!(result["status"], "pending", "{out}");
    assert_eq!(result["held"]["reason"], "approve_all");
    assert_eq!(posts(&env), 0);
    let id = result["proposal_id"].as_u64().unwrap();
    let proposal = sends::list(&env.state)
        .into_iter()
        .find(|p| p.id == id)
        .unwrap();
    assert_eq!(proposal.tool, "request_reviews");
    assert!(
        proposal.arguments["message"]
            .as_str()
            .unwrap()
            .contains("/pull/")
    );

    let approved = sends::decide(&env.state, id, true).unwrap();
    assert_eq!(approved.status, sends::Status::Sent);
    assert!(approved.link.is_some());
    assert!(posts(&env) > 0);
}

#[test]
fn declining_in_the_agent_sends_nothing() {
    let env = env(SlackSendMode::ApproveAll);
    let ask = Arc::new(FakeAsk {
        answer: Answer::Decline,
        shown: Mutex::default(),
    });
    let out = call(&env, Some(Arc::clone(&ask)), to_channel(&env, json!({})));
    assert_eq!(out["results"][0]["status"], "declined", "{out}");
    assert_eq!(posts(&env), 0);
    let shown = ask.shown.lock().unwrap();
    assert_eq!(shown.len(), 1);
    assert!(shown[0].starts_with("Send this Slack review request as you?"));
    assert!(shown[0].contains(&format!("Channel: {TEAM}/{CHANNEL}")));
    assert!(shown[0].contains("/pull/"), "the exact text: {}", shown[0]);
}

#[test]
fn accepting_in_the_agent_sends_it() {
    let env = env(SlackSendMode::ApproveAll);
    let ask = Arc::new(FakeAsk {
        answer: Answer::Accept,
        shown: Mutex::default(),
    });
    let out = call(&env, Some(ask), to_channel(&env, json!({})));
    assert_eq!(out["results"][0]["status"], "sent", "{out}");
    assert_eq!(out["results"][0]["decided_in"], "acme-agent");
    assert!(posts(&env) > 0);
}

#[test]
fn drafts_warn_but_are_sent() {
    let env = env(SlackSendMode::Automatic);
    let pr = open_prs(&env.state)[0];
    env.state
        .store
        .set_pull_request_draft(PrId(pr), true)
        .unwrap();
    let args = json!({ "prs": [pr], "destination": format!("{CHANNEL}@acme") });
    let mut preview_args = args.clone();
    preview_args["preview"] = json!(true);
    let preview = call(&env, None, preview_args);
    let warnings = preview["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{preview}");
    assert_eq!(warnings[0]["kind"], "draft");
    assert_eq!(warnings[0]["pr_id"], pr);
    let out = call(&env, None, args);
    assert_eq!(out["warnings"][0]["kind"], "draft");
    assert_eq!(out["results"][0]["status"], "sent", "{out}");
    assert_eq!(posts(&env), 1);
}

#[test]
fn prs_by_url_or_owner_repo_number() {
    let env = env(SlackSendMode::Automatic);
    let pr = open_prs(&env.state)[0];
    let found = env.state.store.get_pull_request(PrId(pr)).unwrap().unwrap();
    let slug = format!("{}/{}#{}", found.repo.owner, found.repo.name, found.number);
    let url = format!(
        "https://github.com/{}/{}/pull/{}",
        found.repo.owner, found.repo.name, found.number
    );
    for reference in [slug, url] {
        let out = call(
            &env,
            None,
            json!({ "prs": [reference], "destination": format!("{CHANNEL}@acme"), "preview": true }),
        );
        assert_eq!(
            out["cards"][0]["messages"][0]["prs"][0]["pr_id"], pr,
            "{out}"
        );
    }
}
