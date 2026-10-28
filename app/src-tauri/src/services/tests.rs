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
use super::project_info::{self, JiraLinkInput};
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
        let both =
            item.statuses.contains(&PrStatus::Waiting) && item.statuses.contains(&PrStatus::Idle);
        assert!(!both, "waiting and idle are exclusive, as on the row");
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
    let starred = PrFilter {
        starred: Some(true),
        ..PrFilter::default()
    };
    assert_eq!(prs::list_prs(state, &starred).unwrap().total, 0);
    let last = all.items.last().unwrap().row.id;
    project_info::set_pr_starred(state, last, true).unwrap();
    let only = prs::list_prs(state, &starred).unwrap();
    assert_eq!(only.total, 1);
    assert_eq!(only.items[0].row.id, last);
    assert!(only.items[0].row.starred);
    let pinned = prs::list_prs(state, &PrFilter::default()).unwrap();
    assert_eq!(pinned.items[0].row.id, last, "starred pins to the top");
    let unstarred = PrFilter {
        starred: Some(false),
        ..PrFilter::default()
    };
    assert_eq!(
        prs::list_prs(state, &unstarred).unwrap().total,
        all.total - 1
    );
}

#[test]
fn project_info_services_round_trip() {
    let fx = fixture("project-info");
    let state = &fx.state;
    let project = projects::create_project(state, "Acme info".into()).unwrap();
    let described =
        project_info::set_project_description(state, project.id, "Line one\nLine two".into())
            .unwrap();
    assert_eq!(described.description, "Line one\nLine two");

    let text = |text: &str| JiraLinkInput {
        text: Some(text.into()),
        ..JiraLinkInput::default()
    };
    let bare = project_info::add_jira_link(state, project.id, text("acme-7")).unwrap();
    assert_eq!(bare.key, "ACME-7");
    assert_eq!(bare.href, None, "a bare key needs jira.base_url");
    let url = "https://acme-labs.atlassian.net/browse/ACME-123";
    let linked = project_info::add_jira_link(state, project.id, text(url)).unwrap();
    assert_eq!(linked.key, "ACME-123");
    assert_eq!(linked.href.as_deref(), Some(url));
    assert!(project_info::add_jira_link(state, project.id, text("not a key")).is_err());

    let mut prefs = pr_nexus_core::settings::preferences(&state.store).unwrap();
    prefs.jira_base_url = "https://acme-labs.atlassian.net/".into();
    pr_nexus_core::settings::save_preferences(&state.store, &prefs).unwrap();
    let listed = projects::list_projects(state).unwrap();
    let shown = listed.iter().find(|p| p.id == project.id).unwrap();
    assert_eq!(shown.jira_links.len(), 2);
    assert_eq!(
        shown.jira_links[0].href.as_deref(),
        Some("https://acme-labs.atlassian.net/browse/ACME-7")
    );

    let reordered =
        project_info::set_jira_links(state, project.id, vec![text(url), text("ACME-7")]).unwrap();
    assert_eq!(reordered[0].key, "ACME-123");
    project_info::remove_jira_link(state, reordered[1].id).unwrap();
    assert_eq!(
        project_info::list_jira_links(state, project.id)
            .unwrap()
            .len(),
        1
    );

    let pr = pr_id(state, 1);
    assert_eq!(project_info::pr_note(state, pr).unwrap().note, "");
    let saved = project_info::set_pr_note(state, pr, "Check the rollout".into()).unwrap();
    assert!(saved.updated_at.is_some());
    assert_eq!(
        project_info::pr_note(state, pr).unwrap().note,
        "Check the rollout"
    );
    project_info::set_pr_note(state, pr, "  ".into()).unwrap();
    assert_eq!(project_info::pr_note(state, pr).unwrap().updated_at, None);

    assert!(project_info::set_pr_starred(state, pr, true).unwrap());
    assert!(project_info::set_pr_starred(state, pr, true).unwrap());
    assert!(project_info::is_pr_starred(state, pr).unwrap());
    assert!(!project_info::set_pr_starred(state, pr, false).unwrap());
    assert!(!project_info::is_pr_starred(state, pr).unwrap());
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
            "\"merge_ready\"",
            "\"waiting\"",
            "\"idle\""
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

#[test]
fn previews_warn_about_drafts_without_blocking() {
    use pr_nexus_core::domain::PrId;
    let fx = fixture("draft-warning");
    let state = &fx.state;
    connect_fake_workspace(state);
    let pr = pr_id(state, 1);

    state.store.set_pull_request_draft(PrId(pr), false).unwrap();
    let ready = review::preview(state, pr, channel()).unwrap();
    assert!(ready.pr_warnings.is_empty(), "{:?}", ready.pr_warnings);

    state.store.set_pull_request_draft(PrId(pr), true).unwrap();
    let draft = review::preview(state, pr, channel()).unwrap();
    assert_eq!(draft.pr_warnings.len(), 1);
    assert_eq!(draft.pr_warnings[0].kind, "draft");
    assert_eq!(draft.pr_warnings[0].pr_id, pr);
    assert_eq!(draft.pr_warnings[0].number, 1);
    let json = serde_json::to_value(&draft).unwrap();
    assert_eq!(json["pr_warnings"][0]["kind"], "draft");

    let request: BulkRequest = serde_json::from_value(serde_json::json!({
        "pr_ids": [pr],
        "target": channel_json(),
    }))
    .unwrap();
    let bulk = bulk::preview(state, &request).unwrap();
    assert_eq!(bulk.pr_warnings.len(), 1);
    assert_eq!(bulk.pr_warnings[0].pr_id, pr);
    assert!(
        bulk.skipped
            .iter()
            .all(|skip| !skip.detail.contains("draft")),
        "a draft warns but is not skipped: {:?}",
        bulk.skipped
    );

    state.store.set_pull_request_draft(PrId(pr), false).unwrap();
    assert!(
        bulk::preview(state, &request)
            .unwrap()
            .pr_warnings
            .is_empty()
    );
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

#[test]
fn branch_status_and_update_go_through_the_demo_provider() {
    use pr_nexus_core::branch::BranchState;
    let fixture = fixture("branch");
    let state = &fixture.state;
    let id = pr_id(
        state,
        prs::list_prs(state, &PrFilter::default()).unwrap().items[0]
            .row
            .number,
    );
    let status = super::branch::branch_status(state, id).unwrap();
    assert!(status.conflicts_url.ends_with("/conflicts"));
    assert_ne!(status.state, BranchState::Checking);
    let cached = super::branch::branch_status_cached(state, id).unwrap();
    assert_eq!(cached, status);
    let updated = super::branch::update_branch(state, id, "merge").unwrap();
    assert_eq!(updated.branch.base_ref, status.base_ref);
    let err = super::branch::update_branch(state, id, "squash").unwrap_err();
    assert_eq!(err.kind, "validation");
}
