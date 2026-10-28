//! Mock-only routing tests. `acme-labs` signed in with the org host as its
//! domain (`acme`), the way a grid workspace can; `acme` has `acme-xyz`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};

use super::*;
use crate::domain::{AccountId, AuthMode, ChatKind, RequestStatus};
use crate::providers::{ChatChannel, ChatUser, OutgoingMessage, Page, PostedMessage};
use crate::review::lifecycle::{self, tests as life};

const LABS: &str = "T0LABS0001";
const MAIN: &str = "T0ACME0001";
const OTHER: &str = "T0OTHR0001";

fn account(id: i64, team: &str, domain: &str, name: &str) -> Account {
    Account {
        id: AccountId(id),
        kind: AccountKind::Slack,
        display_name: name.into(),
        host: format!("{domain}.slack.com"),
        external_id: format!("{team}:U0ALICE"),
        slack_workspace: Some(domain.into()),
        login: "U0ALICE".into(),
        auth_mode: AuthMode::SlackMcpOauth,
        keychain_key: None,
        created_at: Utc.timestamp_opt(0, 0).unwrap(),
    }
}

fn grid() -> Vec<Workspace> {
    ordered(
        vec![
            account(1, LABS, "acme", "acme-labs"),
            account(2, MAIN, "acme-xyz", "acme"),
            account(3, OTHER, "acme-other", "acme-other"),
        ],
        &[],
        &[],
    )
}

/// A channel link pasted from Slack: the org host, no team.
fn linked_target() -> Target {
    Target {
        workspace: "acme".into(),
        team_id: None,
        channel_id: "C0TEAM0001".into(),
        channel_name: "team-web".into(),
    }
}

fn api(code: &str) -> ProviderError {
    ProviderError::Api {
        code: code.into(),
        message: code.into(),
    }
}

/// Per-workspace probe and post results; records calls.
#[derive(Default)]
struct Scripted {
    account: i64,
    probe: Option<fn() -> ProviderError>,
    post: Option<fn() -> ProviderError>,
    log: Arc<Mutex<Vec<String>>>,
}

impl ChatProvider for Scripted {
    fn kind(&self) -> ChatKind {
        ChatKind::Slack
    }
    fn whoami(&self) -> Result<ChatUser, ProviderError> {
        unimplemented!()
    }
    fn list_channels(&self, _: Option<String>) -> Result<Page<ChatChannel>, ProviderError> {
        unimplemented!()
    }
    fn channel_info(&self, id: &str) -> Result<ChatChannel, ProviderError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("probe {}", self.account));
        match self.probe {
            Some(err) => Err(err()),
            None => Ok(ChatChannel {
                id: id.into(),
                name: id.into(),
            }),
        }
    }
    fn post(&self, message: &OutgoingMessage<'_>) -> Result<PostedMessage, ProviderError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("post {}", self.account));
        if let Some(err) = self.post {
            return Err(err());
        }
        Ok(PostedMessage {
            channel_id: message.channel_id.into(),
            ts: "1700000001.000100".into(),
            thread_ts: None,
            permalink: None,
            workspace: None,
        })
    }
    fn account_id(&self) -> AccountId {
        AccountId(self.account)
    }
}

struct Harness {
    chats: HashMap<i64, Arc<Scripted>>,
    log: Arc<Mutex<Vec<String>>>,
}

/// An error a scripted call fails with; `None` succeeds.
type Fails = Option<fn() -> ProviderError>;

/// `(account, probe outcome, post outcome)` per workspace.
fn harness(script: &[(i64, Fails, Fails)]) -> Harness {
    let log = Arc::new(Mutex::new(Vec::new()));
    let chats = script
        .iter()
        .map(|(id, probe, post)| {
            (
                *id,
                Arc::new(Scripted {
                    account: *id,
                    probe: *probe,
                    post: *post,
                    log: log.clone(),
                }),
            )
        })
        .collect();
    Harness { chats, log }
}

impl Harness {
    fn run(&self, workspaces: &[Workspace], target: &Target) -> Result<Routed<Target>, CoreError> {
        let mut chat_for = |ws: &Workspace| -> Result<Arc<dyn ChatProvider>, CoreError> {
            Ok(self.chats[&ws.account.id.0].clone() as Arc<dyn ChatProvider>)
        };
        let mut send = |chat: &dyn ChatProvider, target: &Target| {
            chat.post(&OutgoingMessage {
                channel_id: &target.channel_id,
                text: "hi",
                thread_ts: None,
                reply_broadcast: false,
                unfurl_app_links: true,
            })?;
            Ok(target.clone())
        };
        send_routed(workspaces, target, &mut chat_for, &mut send)
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

fn not_found() -> ProviderError {
    api("channel_not_found")
}

#[test]
fn org_host_link_routes_to_the_workspace_that_sees_the_channel() {
    // Regression: the link host `acme` is acme-labs' domain, but only acme
    // (acme-xyz) can see the channel. It used to post through acme-labs.
    let h = harness(&[(1, Some(not_found), None), (2, None, None), (3, None, None)]);
    let routed = h.run(&grid(), &linked_target()).unwrap();
    assert_eq!(routed.workspace.team_id, MAIN);
    assert_eq!(
        routed.value.team_id.as_deref(),
        Some(MAIN),
        "team persisted"
    );
    assert_eq!(routed.value.workspace, "acme-xyz");
    assert_eq!(
        h.log(),
        ["probe 1", "probe 2", "post 2"],
        "labs never posts"
    );
}

#[test]
fn known_team_goes_first_then_priority_order() {
    let order = vec![OTHER.to_owned(), MAIN.to_owned(), LABS.to_owned()];
    let workspaces = ordered(
        grid().into_iter().map(|ws| ws.account).collect(),
        &order,
        &[],
    );
    let ids: Vec<&str> = workspaces.iter().map(|ws| ws.team_id.as_str()).collect();
    assert_eq!(ids, [OTHER, MAIN, LABS]);

    let mut target = linked_target();
    target.team_id = Some(LABS.into());
    let picked: Vec<&str> = candidates(&workspaces, &target)
        .iter()
        .map(|ws| ws.team_id.as_str())
        .collect();
    assert_eq!(picked, [LABS, OTHER, MAIN]);

    target.team_id = None;
    target.workspace = "nowhere".into();
    let picked: Vec<&str> = candidates(&workspaces, &target)
        .iter()
        .map(|ws| ws.team_id.as_str())
        .collect();
    assert_eq!(picked, [OTHER, MAIN, LABS]);
}

#[test]
fn disabled_workspaces_are_skipped() {
    let workspaces = ordered(
        grid().into_iter().map(|ws| ws.account).collect(),
        &[],
        &[LABS.to_owned()],
    );
    assert!(!workspaces[0].enabled);
    let h = harness(&[(1, None, None), (2, None, None), (3, None, None)]);
    let routed = h.run(&workspaces, &linked_target()).unwrap();
    assert_eq!(routed.workspace.team_id, MAIN);
    assert_eq!(h.log(), ["probe 2", "post 2"]);

    let all_off = ordered(
        grid().into_iter().map(|ws| ws.account).collect(),
        &[],
        &[LABS.to_owned(), MAIN.to_owned(), OTHER.to_owned()],
    );
    assert!(matches!(
        h.run(&all_off, &linked_target()),
        Err(CoreError::Validation(_))
    ));
}

#[test]
fn falls_back_on_each_safe_send_error() {
    fn not_in_channel() -> ProviderError {
        api("not_in_channel")
    }
    fn user_not_found() -> ProviderError {
        api("user_not_found")
    }
    fn invalid_auth() -> ProviderError {
        ProviderError::Unauthorized("invalid_auth".into())
    }
    fn token_revoked() -> ProviderError {
        ProviderError::Unauthorized("token_revoked".into())
    }
    for err in [
        not_found,
        not_in_channel,
        user_not_found,
        invalid_auth,
        token_revoked,
    ] {
        let h = harness(&[(1, None, Some(err)), (2, None, None), (3, None, None)]);
        let routed = h.run(&grid(), &linked_target()).unwrap();
        assert_eq!(routed.workspace.team_id, MAIN, "{:?}", err());
        assert_eq!(routed.skipped.len(), 1);
        assert_eq!(h.log(), ["probe 1", "post 1", "probe 2", "post 2"]);
    }
}

#[test]
fn never_falls_back_after_an_unsafe_send_error() {
    fn timeout() -> ProviderError {
        ProviderError::Network("timed out".into())
    }
    fn server() -> ProviderError {
        ProviderError::Network("503 Service Unavailable".into())
    }
    fn unknown() -> ProviderError {
        api("tool_error")
    }
    fn limited() -> ProviderError {
        ProviderError::RateLimited {
            reset_at: Utc.timestamp_opt(0, 0).unwrap(),
            secondary: false,
        }
    }
    fn unconfirmed() -> ProviderError {
        ProviderError::SentUnconfirmed("no ts".into())
    }
    for err in [timeout, server, unknown, limited, unconfirmed] {
        let h = harness(&[(1, None, Some(err)), (2, None, None), (3, None, None)]);
        assert!(h.run(&grid(), &linked_target()).is_err(), "{:?}", err());
        assert_eq!(h.log(), ["probe 1", "post 1"], "no second post");
    }
}

#[test]
fn every_workspace_missing_the_channel_names_them() {
    let h = harness(&[
        (1, Some(not_found), None),
        (2, Some(not_found), None),
        (3, Some(not_found), None),
    ]);
    let err = h.run(&grid(), &linked_target()).unwrap_err().to_string();
    assert!(err.contains("channel_not_found"), "{err}");
    assert!(err.contains("acme-labs, acme, acme-other"), "{err}");
    assert!(h.log().iter().all(|call| call.starts_with("probe")));
}

#[test]
fn routed_request_keeps_the_team_that_worked() {
    let (_dir, store, pr, chat) = life::seed("Fix the latch");
    let team = "T0ACME";
    let workspaces = ordered(store.list_accounts().unwrap(), &[], &[]);
    let mut target = linked_target();
    target.channel_id = "C0TEAM".into();
    let plan = lifecycle::plan(&store, &pr, target.clone(), None, 24, life::at(0)).unwrap();
    let chat: Arc<dyn ChatProvider> = Arc::new(chat);
    let mut chat_for = |_: &Workspace| Ok(chat.clone());
    let mut send = |chat: &dyn ChatProvider, target: &Target| {
        let plan = lifecycle::Plan {
            target: target.clone(),
            ..plan.clone()
        };
        lifecycle::send(&store, chat, &pr, &plan, "[]", life::at(0))
    };
    let routed = send_routed(&workspaces, &target, &mut chat_for, &mut send).unwrap();
    assert_eq!(routed.value.team_id.as_deref(), Some(team));
    let stored = store.list_review_requests(pr.id).unwrap();
    assert_eq!(stored[0].team_id.as_deref(), Some(team));
    assert_eq!(stored[0].status, RequestStatus::Active);
}

#[test]
fn resolve_probes_without_posting() {
    let h = harness(&[(1, Some(not_found), None), (2, None, None), (3, None, None)]);
    let mut chat_for = |ws: &Workspace| -> Result<Arc<dyn ChatProvider>, CoreError> {
        Ok(h.chats[&ws.account.id.0].clone() as Arc<dyn ChatProvider>)
    };
    let ws = resolve(&grid(), &linked_target(), &mut chat_for).unwrap();
    assert_eq!(ws.team_id, MAIN);
    assert_eq!(h.log(), ["probe 1", "probe 2"]);
}

#[test]
fn find_prefers_the_team_and_refuses_a_shared_domain() {
    let mut ws = grid();
    assert_eq!(find(&ws, Some(MAIN), "acme").unwrap().team_id, MAIN);
    assert_eq!(find(&ws, None, "ACME").unwrap().team_id, LABS);
    assert!(find(&ws, Some("T0GONE0001"), "acme").is_none());
    ws.push(Workspace {
        account: account(4, "T0DUPE0001", "acme", "acme-dupe"),
        team_id: "T0DUPE0001".into(),
        enabled: true,
    });
    assert!(find(&ws, None, "acme").is_none(), "no first match");
}

#[test]
fn pinning_moves_only_the_mappings_of_the_failed_workspace() {
    use crate::domain::{MappingLayer, TeamChannelMapping};
    let (_dir, store, _pr, _chat) = life::seed("Fix the latch");
    let row = |team: &str, domain: &str, pinned: Option<&str>| TeamChannelMapping {
        id: 0,
        layer: MappingLayer::User,
        github_org: "acme".into(),
        team_name: team.into(),
        team_slug: team.into(),
        slack_workspace: domain.into(),
        slack_team_id: pinned.map(str::to_owned),
        slack_channel_id: "C0TEAM0001".into(),
        slack_channel_name: "team-web".into(),
        disabled: false,
        note: String::new(),
        updated_at: life::at(0),
    };
    store.upsert_mapping(row("web", "acme", None)).unwrap();
    store
        .upsert_mapping(row("ops", "acme-other", Some(OTHER)))
        .unwrap();
    let changed = store
        .pin_mapping_team(
            "C0TEAM0001".into(),
            None,
            "acme".into(),
            MAIN.into(),
            "acme-xyz".into(),
        )
        .unwrap();
    assert_eq!(changed, 1);
    let rows = store.list_mappings().unwrap();
    let web = rows.iter().find(|row| row.team_slug == "web").unwrap();
    assert_eq!(
        (web.slack_team_id.as_deref(), web.slack_workspace.as_str()),
        (Some(MAIN), "acme-xyz")
    );
    let ops = rows.iter().find(|row| row.team_slug == "ops").unwrap();
    assert_eq!(ops.slack_team_id.as_deref(), Some(OTHER));
}
