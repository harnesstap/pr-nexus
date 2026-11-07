//! Service tests over the demo seed. Slack is always a fake that records
//! what it would post; nothing here talks to a network.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use pr_nexus_core::domain::{Account, AccountId, AccountKind, AuthMode, ChatKind};
use pr_nexus_core::domain::{ProviderRef, PullRequest, TeamRef, VcsKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{
    ChatChannel, ChatProvider, ChatUser, OutgoingMessage, OwnershipInputs, OwnershipProbe, Page,
    PostedMessage, PrQuery, RateBudget, SyncedPullRequest, TeamMembers, VcsProvider, VcsUser,
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
    /// Channel ids read by id, in order.
    lookups: Mutex<Vec<String>>,
    /// How long each read takes.
    delay: Duration,
    calls: Concurrency,
}

/// Calls in flight now, and the most seen at once.
#[derive(Default)]
struct Concurrency {
    now: AtomicUsize,
    peak: AtomicUsize,
}

impl Concurrency {
    fn run<T>(&self, delay: Duration, work: impl FnOnce() -> T) -> T {
        let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(delay);
        let out = work();
        self.now.fetch_sub(1, Ordering::SeqCst);
        out
    }
    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
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
        self.slack.lookups.lock().unwrap().push(channel_id.into());
        // Slack knows two channels' names; any other read gives only the
        // id, like a probe that can't name the channel.
        let name = match channel_id {
            "C0ACME0002" => "new-channel",
            "C0ACME0003" => "docs-reviews",
            other => other,
        };
        Ok(self.slack.calls.run(self.slack.delay, || ChatChannel {
            id: channel_id.into(),
            name: name.into(),
        }))
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

fn in_message(preview: &bulk::BulkPreview, id: i64) -> bool {
    preview.cards.iter().any(|card| {
        card.messages
            .iter()
            .any(|message| message.prs.iter().any(|item| item.pr_id == id))
    })
}

fn checks_warning(preview: &bulk::BulkPreview, id: i64) -> bool {
    preview
        .pr_warnings
        .iter()
        .any(|warning| warning.kind == "checks" && warning.pr_id == id)
}

/// Failing checks stay in the request. A draft flip still refreshes the
/// preview. Conflicts, pending checks, and requested changes still skip.
#[test]
fn failing_checks_stay_in_the_preview_and_mark_ready_still_refreshes() {
    use pr_nexus_core::domain::{ChecksState, MergeStateStatus, PrId, ReviewDecision};

    let fx = fixture("ready-preview");
    let state = &fx.state;
    connect_fake_workspace(state);
    let failing = pr_id(state, 1);
    let pending = pr_id(state, 2);
    let changes = pr_id(state, 3);
    let bulk_of = |ids: Vec<i64>, one_channel: bool| -> BulkRequest {
        let mut body = serde_json::json!({ "pr_ids": ids });
        if one_channel {
            body["target"] = channel_json();
        }
        serde_json::from_value(body).unwrap()
    };

    let passing = bulk::preview(state, &bulk_of(vec![failing], true)).unwrap();
    assert!(in_message(&passing, failing), "{:?}", passing.skipped);
    assert!(
        !checks_warning(&passing, failing),
        "{:?}",
        passing.pr_warnings
    );

    let mut row = state
        .store
        .get_pull_request(PrId(failing))
        .unwrap()
        .unwrap();
    row.checks = ChecksState::Failure;
    state.store.update_pull_request_gate(&row).unwrap();

    let red = bulk::preview(state, &bulk_of(vec![failing, pending, changes], true)).unwrap();
    assert!(
        in_message(&red, failing),
        "failing checks stay in the message: {:?}",
        red.skipped
    );
    assert!(checks_warning(&red, failing), "{:?}", red.pr_warnings);
    assert!(
        red.skipped
            .iter()
            .any(|skip| skip.pr.pr_id == pending && skip.detail.contains("pending")),
        "a pending check still blocks: {:?}",
        red.skipped
    );
    assert!(
        red.skipped
            .iter()
            .any(|skip| { skip.pr.pr_id == changes && skip.detail.contains("requested changes") }),
        "requested changes still block: {:?}",
        red.skipped
    );
    assert!(!in_message(&red, pending) && !in_message(&red, changes));
    assert!(
        !checks_warning(&red, changes),
        "a skipped PR is not noted as included"
    );

    state
        .store
        .set_pull_request_draft(PrId(failing), true)
        .unwrap();
    let drafted = bulk::preview(state, &bulk_of(vec![failing], true)).unwrap();
    assert!(
        drafted
            .pr_warnings
            .iter()
            .any(|warning| warning.kind == "draft" && warning.pr_id == failing)
    );
    assert!(in_message(&drafted, failing));
    assert!(checks_warning(&drafted, failing));

    state
        .store
        .set_pull_request_draft(PrId(failing), false)
        .unwrap();
    let after = bulk::preview(state, &bulk_of(vec![failing], true)).unwrap();
    assert!(
        after
            .pr_warnings
            .iter()
            .all(|warning| warning.kind != "draft"),
        "mark ready drops the draft warning: {:?}",
        after.pr_warnings
    );
    assert!(
        checks_warning(&after, failing),
        "mark ready does not clear a failing check"
    );
    assert!(in_message(&after, failing));

    row.checks = ChecksState::Success;
    row.is_draft = false;
    row.review_decision = Some(ReviewDecision::Approved);
    row.merge_state = MergeStateStatus::Clean;
    state.store.update_pull_request_gate(&row).unwrap();
    let green = bulk::preview(state, &bulk_of(vec![failing], true)).unwrap();
    assert!(!checks_warning(&green, failing));
    assert!(in_message(&green, failing));

    row.checks = ChecksState::Failure;
    row.merge_state = MergeStateStatus::Dirty;
    state.store.update_pull_request_gate(&row).unwrap();
    let conflicted = bulk::preview(state, &bulk_of(vec![failing], true)).unwrap();
    assert!(
        conflicted
            .skipped
            .iter()
            .any(|skip| { skip.pr.pr_id == failing && skip.detail.contains("merge conflicts") }),
        "conflicts still block when checks are also failing: {:?}",
        conflicted.skipped
    );
    assert!(!in_message(&conflicted, failing));

    row.merge_state = MergeStateStatus::Clean;
    state.store.update_pull_request_gate(&row).unwrap();
    let passing_row = state
        .store
        .get_pull_request(PrId(failing))
        .unwrap()
        .unwrap();
    crate::slack::lock(&state.owners).insert(
        failing,
        crate::state::OwnersEntry {
            head_sha: passing_row.head_sha.clone(),
            base_ref: passing_row.base_ref.clone(),
            is_draft: passing_row.is_draft,
            checks: ChecksState::Success,
            at: std::time::Instant::now(),
            blocked: Some("check ci is fail".into()),
            teams: Vec::new(),
        },
    );
    let stale = bulk::preview(state, &bulk_of(vec![failing], false)).unwrap();
    assert!(
        stale
            .skipped
            .iter()
            .all(|skip| !skip.detail.contains("check ci is fail")),
        "an owner decision recorded for a different checks rollup is not reused: {:?}",
        stale.skipped
    );

    crate::slack::lock(&state.owners).insert(
        failing,
        crate::state::OwnersEntry {
            head_sha: passing_row.head_sha.clone(),
            base_ref: passing_row.base_ref.clone(),
            is_draft: false,
            checks: ChecksState::Failure,
            at: std::time::Instant::now(),
            blocked: Some("check ci is fail".into()),
            teams: Vec::new(),
        },
    );
    let cached = bulk::preview(state, &bulk_of(vec![failing], false)).unwrap();
    assert!(
        cached
            .skipped
            .iter()
            .all(|skip| !skip.detail.contains("check ci is fail")),
        "a cached failing-check decision does not leave the PR out: {:?}",
        cached.skipped
    );
    crate::slack::lock(&state.owners).insert(
        failing,
        crate::state::OwnersEntry {
            head_sha: passing_row.head_sha.clone(),
            base_ref: passing_row.base_ref.clone(),
            is_draft: false,
            checks: ChecksState::Failure,
            at: std::time::Instant::now(),
            blocked: Some("check ci is fail; PR has merge conflicts".into()),
            teams: Vec::new(),
        },
    );
    let cached_conflict = bulk::preview(state, &bulk_of(vec![failing], false)).unwrap();
    assert!(
        cached_conflict
            .skipped
            .iter()
            .any(|skip| { skip.pr.pr_id == failing && skip.detail.contains("merge conflicts") }),
        "a cached conflict still blocks: {:?}",
        cached_conflict.skipped
    );
}

#[test]
fn failing_checks_are_accepted_by_single_and_bulk_send() {
    use pr_nexus_core::domain::{ChecksState, PrId};

    let fx = fixture("ci-send");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack::default());
    let failing = pr_id(state, 1);
    let mut row = state
        .store
        .get_pull_request(PrId(failing))
        .unwrap()
        .unwrap();
    row.checks = ChecksState::Failure;
    state.store.update_pull_request_gate(&row).unwrap();

    let single = review::preview(state, failing, channel()).unwrap();
    assert!(
        slack.posts.lock().unwrap().is_empty(),
        "preview posts nothing"
    );
    assert!(
        single
            .pr_warnings
            .iter()
            .any(|warning| warning.kind == "checks")
    );
    let sent = review::send(state, &slack, failing, channel(), &[], None).unwrap();
    assert_eq!(sent.channel_id, "C0ACME0001");
    assert_eq!(slack.posts.lock().unwrap().len(), 1);

    let request: BulkRequest = serde_json::from_value(serde_json::json!({
        "pr_ids": [failing, pr_id(state, 2)],
        "target": channel_json(),
    }))
    .unwrap();
    let preview = bulk::preview(state, &request).unwrap();
    assert!(in_message(&preview, failing));
    assert!(
        preview
            .skipped
            .iter()
            .any(|skip| skip.detail.contains("pending"))
    );
    let fingerprint = preview.cards[0].fingerprint.clone();
    let result = bulk::send_card(state, &slack, &request, &fingerprint).unwrap();
    let posted: Vec<(i64, String)> = result
        .results
        .iter()
        .map(|outcome| (outcome.pr.pr_id, outcome.outcome.clone()))
        .collect();
    assert!(
        posted
            .iter()
            .any(|(id, outcome)| *id == failing && outcome == "sent"),
        "{posted:?}"
    );
    let pending = pr_id(state, 2);
    assert!(
        posted.iter().all(|(id, _)| *id != pending),
        "a pending PR is not posted: {posted:?}"
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

/// An open PR of the seed whose owners resolve to at least one team, and
/// that team (`org/slug`).
fn pr_with_team(state: &AppState) -> (i64, String) {
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
    .find_map(|item| {
        let stored = crate::slack::load_pr(&state.store, item.row.id).ok()?;
        if stored.state != pr_nexus_core::domain::PrState::Open
            || !stored
                .involvement
                .contains(pr_nexus_core::domain::Involvement::AUTHOR)
        {
            return None;
        }
        let detail = prs::pr_detail(state, item.row.id).ok()?;
        if detail.readiness == "blocked" {
            return None;
        }
        let teams = if detail.mandatory_teams.is_empty() {
            detail.candidate_teams
        } else {
            detail.mandatory_teams
        };
        teams.first().cloned().map(|team| (item.row.id, team))
    })
    .map(|(pr, team)| {
        // Start from no tracked thread: dismiss the demo's seeded request.
        let requests = state
            .store
            .list_review_requests(pr_nexus_core::domain::PrId(pr))
            .unwrap();
        for request in requests
            .iter()
            .filter(|r| r.status == pr_nexus_core::domain::RequestStatus::Active)
        {
            state
                .store
                .close_review_request(
                    request.id,
                    pr_nexus_core::domain::CloseReason::Dismissed,
                    Utc::now(),
                )
                .unwrap();
        }
        (pr, team)
    })
    .expect("a seeded PR with an owning team")
}

fn map_team(state: &AppState, team: &str, channel_id: &str, channel_name: &str) {
    let (org, slug) = team.trim_start_matches('@').split_once('/').unwrap();
    pr_nexus_core::mapping::save_user_mapping(
        &state.store,
        pr_nexus_core::mapping::CsvRow {
            github_org: org.into(),
            team_slug: slug.into(),
            team_name: String::new(),
            slack_channel_name: channel_name.into(),
            slack_channel_id: channel_id.into(),
            slack_workspace: "acme".into(),
            slack_team_id: "T0ACME0001".into(),
        },
        false,
        Utc::now(),
    )
    .unwrap();
}

/// The channel id is the truth: a mapping saved with a stale name still
/// previews (bulk and single) under the name Slack gives for the id, and a
/// channel Slack can't name keeps its stored name.
#[test]
fn preview_names_the_channel_from_its_id() {
    let fx = fixture("remap-name");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack::default());
    let (pr, team) = pr_with_team(state);
    map_team(state, &team, "C0ACME0002", "stale-name");

    let request: BulkRequest =
        serde_json::from_value(serde_json::json!({ "pr_ids": [pr] })).unwrap();
    let preview = bulk::preview_with(state, &slack, &bulk::LiveVcs, &request).unwrap();
    assert_eq!(preview.cards.len(), 1, "{:?}", preview.cards);
    assert_eq!(preview.cards[0].target.channel_id, "C0ACME0002");
    assert_eq!(preview.cards[0].target.channel_name, "new-channel");

    let single = review::preview_with(
        state,
        &slack,
        pr,
        ReviewTarget {
            channel_id: "C0ACME0002".into(),
            channel_name: "stale-name".into(),
            ..channel()
        },
    )
    .unwrap();
    assert_eq!(single.target.channel_name, "new-channel");

    let unnamed = review::preview_with(state, &slack, pr, channel()).unwrap();
    assert_eq!(
        unnamed.target.channel_name, "team-reviews",
        "stored fallback"
    );
}

/// Bug: after a team was remapped, the bulk card named the new channel and
/// the send bumped the tracked thread in the old one. Now the preview and
/// the send both target the newly mapped channel, as a new request.
#[test]
fn remapped_team_gets_a_new_request_in_the_mapped_channel() {
    let fx = fixture("remap-bulk");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack::default());
    let (pr, team) = pr_with_team(state);
    map_team(state, &team, "C0ACME0001", "old-channel");
    let old = ReviewTarget {
        channel_id: "C0ACME0001".into(),
        channel_name: "old-channel".into(),
        ..channel()
    };
    review::send(
        state,
        &slack,
        pr,
        old.clone(),
        std::slice::from_ref(&team),
        None,
    )
    .unwrap();
    map_team(state, &team, "C0ACME0002", "new-channel");
    let before = slack.posts.lock().unwrap().len();

    let request: BulkRequest =
        serde_json::from_value(serde_json::json!({ "pr_ids": [pr] })).unwrap();
    let preview = bulk::preview(state, &request).unwrap();
    assert_eq!(
        preview.cards.len(),
        1,
        "{:?} {:?}",
        preview.cards,
        serde_json::to_string(&preview.skipped).unwrap()
    );
    let card = &preview.cards[0];
    assert_eq!(card.target.channel_id, "C0ACME0002");
    assert_eq!(card.target.channel_name, "new-channel");
    assert_eq!(card.messages.len(), 1);
    assert_eq!(card.messages[0].action, "request");
    assert!(card.warnings.is_empty(), "{:?}", card.warnings);
    assert_eq!(card.notes.len(), 1);
    assert!(card.notes[0].contains("#new-channel"), "{}", card.notes[0]);
    assert!(card.notes[0].contains("#old-channel"), "{}", card.notes[0]);

    let result = bulk::send(
        state,
        &slack,
        &request,
        std::slice::from_ref(&card.fingerprint),
    )
    .unwrap();
    assert!(result.results.iter().all(|r| r.outcome == "sent"));
    let posts = slack.posts.lock().unwrap();
    assert_eq!(posts.len(), before + 1);
    assert_eq!(posts[before].0, "C0ACME0002", "posted where the card said");
    assert!(
        posts[before].1.is_none(),
        "a new top-level request, not a bump"
    );
}

/// The single-PR dialog opens on the mapped channel (a new request); the old
/// thread stays an explicit pick, and picking it previews a bump that names
/// the old channel, where the send then goes.
#[test]
fn remapped_team_keeps_the_old_thread_as_an_explicit_pick() {
    let fx = fixture("remap-single");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack::default());
    let (pr, team) = pr_with_team(state);
    map_team(state, &team, "C0ACME0001", "old-channel");
    let old = ReviewTarget {
        channel_id: "C0ACME0001".into(),
        channel_name: "old-channel".into(),
        ..channel()
    };
    review::send(
        state,
        &slack,
        pr,
        old.clone(),
        std::slice::from_ref(&team),
        None,
    )
    .unwrap();
    map_team(state, &team, "C0ACME0002", "new-channel");
    let before = slack.posts.lock().unwrap().len();

    let stored = crate::slack::load_pr(&state.store, pr).unwrap();
    let choices =
        crate::slack::targets_for(&state.store, &stored, std::slice::from_ref(&team)).unwrap();
    let sources: Vec<(&str, &str)> = choices
        .iter()
        .map(|c| (c.target.channel_id.as_str(), c.source.as_str()))
        .collect();
    assert_eq!(
        sources,
        [("C0ACME0002", "mapping"), ("C0ACME0001", "previous")]
    );

    let fresh = review::preview(state, pr, choices[0].target.clone()).unwrap();
    assert_eq!(fresh.action, "request");
    assert_eq!(fresh.target.channel_name, "new-channel");

    let bump = review::preview(state, pr, choices[1].target.clone()).unwrap();
    assert_eq!(bump.action, "bump");
    assert_eq!(bump.target.channel_name, "old-channel");
    review::send(state, &slack, pr, choices[1].target.clone(), &[team], None).unwrap();
    let posts = slack.posts.lock().unwrap();
    assert_eq!(posts[before].0, "C0ACME0001");
    assert!(posts[before].1.is_some(), "a reply in the old thread");
}

/// Teams still mapped where the request lives keep bumping it.
#[test]
fn unchanged_mapping_still_bumps_the_tracked_thread() {
    let fx = fixture("remap-none");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack::default());
    let (pr, team) = pr_with_team(state);
    map_team(state, &team, "C0ACME0001", "team-reviews");
    review::send(
        state,
        &slack,
        pr,
        channel(),
        std::slice::from_ref(&team),
        None,
    )
    .unwrap();
    let request: BulkRequest =
        serde_json::from_value(serde_json::json!({ "pr_ids": [pr] })).unwrap();
    let preview = bulk::preview(state, &request).unwrap();
    assert_eq!(
        preview.cards.len(),
        1,
        "{}",
        serde_json::to_string(&preview.skipped).unwrap()
    );
    assert_eq!(preview.cards[0].target.channel_id, "C0ACME0001");
    assert_eq!(preview.cards[0].messages[0].action, "bump");
    assert!(preview.cards[0].notes.is_empty());
}

/// GitHub as the demo provider answers it, counting ownership probes and
/// how many run at once.
#[derive(Default)]
struct CountingGithub {
    probes: AtomicUsize,
    clients: AtomicUsize,
    delay: Duration,
    calls: Concurrency,
}

struct CountingClient(Arc<CountingGithub>);

impl VcsProvider for CountingClient {
    fn kind(&self) -> VcsKind {
        VcsKind::GitHub
    }
    fn viewer(&self) -> Result<VcsUser, ProviderError> {
        crate::demo::DemoProvider.viewer()
    }
    fn search_pull_requests(&self, q: &PrQuery) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        crate::demo::DemoProvider.search_pull_requests(q)
    }
    fn refresh_pull_requests(
        &self,
        ids: &[ProviderRef],
    ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        crate::demo::DemoProvider.refresh_pull_requests(ids)
    }
    fn pull_request_status(
        &self,
        ids: &[ProviderRef],
    ) -> Result<Vec<pr_nexus_core::providers::PrStatus>, ProviderError> {
        crate::demo::DemoProvider.pull_request_status(ids)
    }
    fn ownership_probe(&self, pr: &PullRequest) -> Result<OwnershipProbe, ProviderError> {
        self.0.probes.fetch_add(1, Ordering::SeqCst);
        self.0.calls.run(self.0.delay, || {
            crate::demo::DemoProvider.ownership_probe(pr)
        })
    }
    fn ownership_inputs(
        &self,
        pr: &PullRequest,
        probe: &OwnershipProbe,
    ) -> Result<OwnershipInputs, ProviderError> {
        crate::demo::DemoProvider.ownership_inputs(pr, probe)
    }
    fn team_members(&self, team: &TeamRef) -> Result<TeamMembers, ProviderError> {
        crate::demo::DemoProvider.team_members(team)
    }
    fn rate_limit(&self) -> Result<RateBudget, ProviderError> {
        crate::demo::DemoProvider.rate_limit()
    }
    fn request_team_review(
        &self,
        pr: &PullRequest,
        teams: &[TeamRef],
    ) -> Result<(), ProviderError> {
        crate::demo::DemoProvider.request_team_review(pr, teams)
    }
}

impl bulk::VcsSource for Arc<CountingGithub> {
    fn vcs(&self, _: &AppState) -> Result<Box<dyn VcsProvider>, AppError> {
        self.clients.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountingClient(Arc::clone(self))))
    }
}

fn probes(github: &CountingGithub) -> usize {
    github.probes.load(Ordering::SeqCst)
}

fn lookups(slack: &FakeSlack) -> usize {
    slack.lookups.lock().unwrap().len()
}

/// Forget what earlier calls (e.g. the detail panel) left in the session
/// caches.
fn cold_caches(state: &AppState) {
    state.owners.lock().unwrap().clear();
    state.channel_names.lock().unwrap().clear();
}

/// A second preview (a toggle, a re-open) asks neither GitHub nor Slack;
/// Send still resolves owners again but reuses the channel name.
#[test]
fn second_preview_makes_no_network_calls() {
    let fx = fixture("preview-cache");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack::default());
    let github = Arc::new(CountingGithub::default());
    let (pr, team) = pr_with_team(state);
    map_team(state, &team, "C0ACME0002", "stale-name");
    cold_caches(state);

    let request: BulkRequest =
        serde_json::from_value(serde_json::json!({ "pr_ids": [pr] })).unwrap();
    let first = bulk::preview_with(state, &slack, &github, &request).unwrap();
    assert_eq!(first.cards[0].target.channel_name, "new-channel");
    assert_eq!((probes(&github), lookups(&slack)), (1, 1));

    let again = bulk::preview_with(state, &slack, &github, &request).unwrap();
    assert_eq!(again.cards[0].target, first.cards[0].target);
    assert_eq!(
        (probes(&github), lookups(&slack)),
        (1, 1),
        "all from the cache"
    );

    let result = bulk::send_with_vcs(
        state,
        &slack,
        &github,
        &request,
        std::slice::from_ref(&again.cards[0].fingerprint),
    )
    .unwrap();
    assert!(result.results.iter().all(|r| r.outcome == "sent"));
    assert_eq!(probes(&github), 2, "send re-validates owners");
    // The one extra read is the send's own access probe (route.rs), not a
    // name lookup: the name stays cached.
    assert_eq!(lookups(&slack), 2);
    assert_eq!(slack.posts.lock().unwrap()[0].0, "C0ACME0002");
}

/// With `cached_names` the preview never waits on Slack: unknown names
/// keep the stored one and `names_pending` asks for a live pass, which
/// renames the card to what Slack says for the id.
#[test]
fn cached_names_preview_skips_slack_and_flags_pending() {
    let fx = fixture("preview-pending");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack::default());
    let (pr, team) = pr_with_team(state);
    map_team(state, &team, "C0ACME0002", "stale-name");
    cold_caches(state);

    let quick: BulkRequest =
        serde_json::from_value(serde_json::json!({ "pr_ids": [pr], "cached_names": true }))
            .unwrap();
    let draft = bulk::preview_with(state, &slack, &bulk::LiveVcs, &quick).unwrap();
    assert_eq!(lookups(&slack), 0);
    assert!(draft.names_pending);
    assert_eq!(draft.cards[0].target.channel_id, "C0ACME0002");
    assert_eq!(draft.cards[0].target.channel_name, "stale-name");

    let live: BulkRequest = serde_json::from_value(serde_json::json!({ "pr_ids": [pr] })).unwrap();
    let named = bulk::preview_with(state, &slack, &bulk::LiveVcs, &live).unwrap();
    assert!(!named.names_pending);
    assert_eq!(named.cards[0].target.channel_name, "new-channel");
    assert_eq!(
        named.cards[0].fingerprint, draft.cards[0].fingerprint,
        "same id, same plan"
    );

    let reopened = bulk::preview_with(state, &slack, &bulk::LiveVcs, &quick).unwrap();
    assert!(!reopened.names_pending);
    assert_eq!(reopened.cards[0].target.channel_name, "new-channel");
    assert_eq!(lookups(&slack), 1);
}

/// Owners of several PRs resolve through one client, a bounded number at
/// a time; channel names are read once per channel, in parallel.
#[test]
fn many_prs_resolve_in_bounded_parallel() {
    let fx = fixture("preview-parallel");
    let state = &fx.state;
    connect_fake_workspace(state);
    let slack = Arc::new(FakeSlack {
        delay: Duration::from_millis(40),
        ..FakeSlack::default()
    });
    let github = Arc::new(CountingGithub {
        delay: Duration::from_millis(40),
        ..CountingGithub::default()
    });
    let ids: Vec<i64> = prs::list_prs(state, &PrFilter::default())
        .unwrap()
        .items
        .iter()
        .map(|item| item.row.id)
        .collect();
    assert!(ids.len() >= 2, "the seed has several PRs");
    let (_, team) = pr_with_team(state);
    map_team(state, &team, "C0ACME0002", "stale-name");
    cold_caches(state);

    let request: BulkRequest =
        serde_json::from_value(serde_json::json!({ "pr_ids": ids })).unwrap();
    bulk::preview_with(state, &slack, &github, &request).unwrap();
    assert_eq!(
        github.clients.load(Ordering::SeqCst),
        1,
        "one shared client"
    );
    let (probed, peak) = (probes(&github), github.calls.peak());
    assert!(probed >= 2, "{probed}");
    assert!(
        (2.min(probed)..=bulk::OWNER_LOOKUPS).contains(&peak),
        "owner lookups overlap but stay bounded: {peak} of {probed}"
    );

    // Five targets over two channels in two workspaces: two reads, at once.
    let target = |workspace: &str, id: &str| ReviewTarget {
        workspace: workspace.into(),
        team_id: None,
        workspace_name: String::new(),
        channel_id: id.into(),
        channel_name: "stored".into(),
    };
    slack.lookups.lock().unwrap().clear();
    let targets = vec![
        target("acme", "C0ACME0002"),
        target("acme-labs", "C0ACME0003"),
        target("acme", "C0ACME0002"),
        target("acme-labs", "C0ACME0003"),
        target("acme", "C0ACME0002"),
    ];
    let named = crate::slack::resolve_targets(state, &slack, targets, crate::slack::Names::Live);
    let mut read = slack.lookups.lock().unwrap().clone();
    read.sort();
    assert_eq!(read, ["C0ACME0002", "C0ACME0003"]);
    assert_eq!(
        slack.calls.peak(),
        2,
        "the two channels are read in parallel"
    );
    assert_eq!(
        named
            .iter()
            .map(|t| t.channel_name.as_str())
            .collect::<Vec<_>>(),
        [
            "new-channel",
            "docs-reviews",
            "new-channel",
            "docs-reviews",
            "new-channel"
        ]
    );
}

/// A mapping that names a channel but has no ID yet is listed as
/// `unresolved`: never an empty-ID target to preselect or preview (§7.11).
#[test]
fn a_name_only_mapping_is_an_unresolved_suggestion() {
    let fx = fixture("name-only");
    let state = &fx.state;
    connect_fake_workspace(state);
    let (pr, team) = pr_with_team(state);
    map_team(state, &team, "", "acme-data");
    let stored = crate::slack::load_pr(&state.store, pr).unwrap();
    let choices =
        crate::slack::targets_for(&state.store, &stored, std::slice::from_ref(&team)).unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].source, "unresolved");
    assert_eq!(choices[0].target.channel_name, "acme-data");
    assert!(choices[0].target.channel_id.is_empty());

    map_team(state, &team, "C0ACME0001", "acme-data");
    let choices =
        crate::slack::targets_for(&state.store, &stored, std::slice::from_ref(&team)).unwrap();
    assert_eq!(choices[0].source, "mapping");
    assert_eq!(choices[0].target.channel_id, "C0ACME0001");
}
