//! Service tests over the demo seed. Slack is always a fake that records
//! what it would post; nothing here talks to a network.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use pr_nexus_core::domain::{Account, AccountId, AccountKind, AuthMode, ChatKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{
    ChatChannel, ChatProvider, ChatUser, OutgoingMessage, Page, PostedMessage,
};

use super::bulk::{self, BulkRequest};
use super::projects;
use super::prs::{self, PrFilter, PrStateFilter, PrStatus};
use super::review::{self, ChatSource};
use crate::error::AppError;
use crate::slack::ReviewTarget;
use crate::state::AppState;

struct Fixture {
    state: AppState,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture(name: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pr-nexus-services-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let state = AppState::for_tests(&dir.join("test.db"), true);
    crate::demo::seed_if_empty(&state.store).unwrap();
    Fixture { state, dir }
}

fn pr_id(state: &AppState, number: u64) -> i64 {
    prs::list_prs(state, &PrFilter::default())
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.row.number == number)
        .map(|item| item.row.id)
        .expect("seeded PR")
}

/// Records every post and answers like Slack would. Never sends anything.
#[derive(Default)]
struct FakeSlack {
    posts: Mutex<Vec<(String, Option<String>, String)>>,
}

struct FakeChat {
    slack: Arc<FakeSlack>,
}

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
        let mut posts = self.slack.posts.lock().unwrap();
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

impl ChatSource for Arc<FakeSlack> {
    fn chat(&self, _: &AppState, _: &str) -> Result<Arc<dyn ChatProvider>, AppError> {
        Ok(Arc::new(FakeChat {
            slack: Arc::clone(self),
        }))
    }
}

fn connect_fake_workspace(state: &AppState) {
    state
        .store
        .upsert_account(Account {
            id: AccountId(0),
            kind: AccountKind::Slack,
            display_name: "Acme Labs".into(),
            host: "acme.slack.com".into(),
            external_id: "T0ACME0001:U0ACME0001".into(),
            slack_workspace: Some("acme".into()),
            login: "ada".into(),
            auth_mode: AuthMode::SlackMcpOauth,
            keychain_key: Some("slack:T0ACME0001:U0ACME0001".into()),
            created_at: Utc::now(),
        })
        .unwrap();
}

fn channel() -> ReviewTarget {
    ReviewTarget {
        workspace: "acme".into(),
        team_id: Some("T0ACME0001".into()),
        workspace_name: String::new(),
        channel_id: "C0ACME0001".into(),
        channel_name: "team-reviews".into(),
    }
}

#[test]
fn project_lifecycle_and_links() {
    let fx = fixture("projects");
    let state = &fx.state;
    let created = projects::create_project(state, "Acme launch".into()).unwrap();
    projects::rename_project(state, created.id, "Acme rollout".into()).unwrap();
    let colored = projects::set_project_color(state, created.id, "#336699".into()).unwrap();
    assert_eq!(colored.color, "#336699");
    let pr = pr_id(state, 1);
    let outcome = projects::link_prs(state, created.id, vec![pr]).unwrap();
    assert_eq!(outcome.changed, 1);
    let links = projects::list_project_links(state, created.id).unwrap();
    assert!(
        links
            .iter()
            .any(|link| link.pr_id == pr && link.source == "manual")
    );
    let filter = PrFilter {
        project_id: Some(created.id),
        ..PrFilter::default()
    };
    assert_eq!(prs::list_prs(state, &filter).unwrap().total, 1);
    projects::unlink_pr(state, created.id, pr).unwrap();
    assert_eq!(prs::list_prs(state, &filter).unwrap().total, 0);
    projects::archive_project(state, created.id, true).unwrap();
    let listed = projects::list_projects(state).unwrap();
    let found = listed.iter().find(|p| p.id == created.id).unwrap();
    assert!(found.archived);
    assert_eq!(found.name, "Acme rollout");
    assert!(projects::link_prs(state, 9999, vec![pr]).is_err());
}

#[test]
fn list_prs_filters_and_pages() {
    let fx = fixture("list");
    let state = &fx.state;
    let all = prs::list_prs(state, &PrFilter::default()).unwrap();
    assert!(all.total >= 3);
    for item in &all.items {
        assert_eq!(item.statuses, PrStatus::of(&item.row));
    }
    let drafts = prs::list_prs(
        state,
        &PrFilter {
            statuses: vec![PrStatus::Draft],
            ..PrFilter::default()
        },
    )
    .unwrap();
    assert!(drafts.items.iter().all(|item| item.row.is_draft));
    let open = prs::list_prs(
        state,
        &PrFilter {
            state: Some(PrStateFilter::Open),
            author: Some("@NoSuchAuthor".into()),
            ..PrFilter::default()
        },
    )
    .unwrap();
    assert_eq!(open.total, 0);
    let first = prs::list_prs(
        state,
        &PrFilter {
            limit: Some(1),
            ..PrFilter::default()
        },
    )
    .unwrap();
    assert_eq!(first.items.len(), 1);
    let second = prs::list_prs(
        state,
        &PrFilter {
            limit: Some(1),
            cursor: first.next_cursor.clone(),
            ..PrFilter::default()
        },
    )
    .unwrap();
    assert_ne!(first.items[0].row.id, second.items[0].row.id);
    let repo = prs::list_prs(
        state,
        &PrFilter {
            repo: Some("ACME/widgets".into()),
            ..PrFilter::default()
        },
    )
    .unwrap();
    assert_eq!(repo.total, all.total);
    assert!(
        prs::list_prs(
            state,
            &PrFilter {
                starred: Some(true),
                ..PrFilter::default()
            }
        )
        .is_err()
    );
}

#[test]
fn status_names_are_stable() {
    let names: Vec<String> = PrStatus::ALL
        .iter()
        .map(|status| serde_json::to_string(status).unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "\"draft\"",
            "\"ready\"",
            "\"blocked\"",
            "\"unknown\"",
            "\"no_review\"",
            "\"ci_trouble\"",
            "\"declined\"",
            "\"merge_ready\""
        ]
    );
    let filter: PrFilter =
        serde_json::from_str(r#"{"statuses":["merge_ready","ci_trouble"],"state":"open"}"#)
            .unwrap();
    assert_eq!(filter.statuses, [PrStatus::MergeReady, PrStatus::CiTrouble]);
}

#[test]
fn review_request_then_bump_post_through_the_fake() {
    let fx = fixture("review");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack::default());
    let pr = pr_id(state, 1);

    let before = review::preview(state, pr, channel()).unwrap();
    assert_eq!(before.action, "request");
    assert!(
        slack.posts.lock().unwrap().is_empty(),
        "preview posts nothing"
    );

    let sent = review::send(state, &slack, pr, channel(), &[], Some("Thanks!")).unwrap();
    assert_eq!(sent.channel_id, "C0ACME0001");
    {
        let posts = slack.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert!(posts[0].1.is_none(), "a request is a new message");
    }

    let again = review::preview(state, pr, channel()).unwrap();
    assert_eq!(again.action, "bump");
}

#[test]
fn bulk_preview_posts_nothing_and_send_uses_the_fake() {
    let fx = fixture("bulk");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack::default());
    let ids: Vec<i64> = prs::list_prs(
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
    .collect();
    let request: BulkRequest = serde_json::from_value(serde_json::json!({
        "pr_ids": ids,
        "target": channel_json(),
    }))
    .unwrap();
    let preview = bulk::preview(state, &request).unwrap();
    assert!(slack.posts.lock().unwrap().is_empty());
    let fingerprints: Vec<String> = preview
        .cards
        .iter()
        .map(|card| card.fingerprint.clone())
        .collect();
    let result = bulk::send(state, &slack, &request, &fingerprints).unwrap();
    let posted = slack.posts.lock().unwrap().len();
    let sent = result
        .results
        .iter()
        .filter(|outcome| outcome.outcome == "sent")
        .count();
    assert_eq!(posted > 0, sent > 0);
}

fn channel_json() -> serde_json::Value {
    serde_json::json!({
        "workspace": "acme",
        "team_id": "T0ACME0001",
        "workspace_name": "",
        "channel_id": "C0ACME0001",
        "channel_name": "team-reviews",
    })
}
