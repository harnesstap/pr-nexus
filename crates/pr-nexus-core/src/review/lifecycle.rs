//! Single-PR request and bump (§7.4, §7.7).
//!
//! [`plan`] decides Request or Bump for one `(workspace, channel)` and builds
//! the text with the byte-exact builder. [`send`] posts through a
//! [`ChatProvider`] and records the result. The provider is the only network
//! dependency, so tests use a fake.

use chrono::{DateTime, Duration, Utc};

use crate::domain::{
    ProviderRef, PullRequest, RequestSource, RequestStatus, ReviewMessage, ReviewMessageKind,
    ReviewMessageOrigin, ReviewMessagePr, ReviewRequest, ReviewRequestId,
};
use crate::error::CoreError;
use crate::providers::{ChatProvider, OutgoingMessage, PostedMessage};
use crate::store::Store;

use super::message::{self, MessageMode};

/// Where the message goes. `workspace` is the domain (`acme`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub workspace: String,
    /// Slack team (`T…`) to send through, when known. Wins over `workspace`.
    pub team_id: Option<String>,
    pub channel_id: String,
    pub channel_name: String,
}

impl Target {
    /// What picks the Slack account: the team ID when known, else the domain.
    pub fn route_key(&self) -> &str {
        self.team_id.as_deref().unwrap_or(&self.workspace)
    }

    /// How the channel reads in text: `#reviews`, `DM` for a direct
    /// message (`D…`), never `#D…`, or `@name` for a member
    /// (`U…`). Mirrors `channelLabel` in the app.
    pub fn label(&self) -> String {
        let name = self.channel_name.trim().trim_start_matches(['#', '@']);
        if self.channel_id.starts_with(['U', 'W']) {
            // A member ID: Slack delivers it as a direct message.
            format!(
                "@{}",
                if name.is_empty() {
                    &self.channel_id
                } else {
                    name
                }
            )
        } else if self.channel_id.starts_with('D') {
            if name.is_empty() || name == self.channel_id {
                "DM".to_owned()
            } else {
                format!("DM · {name}")
            }
        } else if name.is_empty() {
            format!("#{}", self.channel_id)
        } else {
            format!("#{name}")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// New top-level message.
    Request,
    /// Thread reply under the active request's root, broadcast to the channel.
    Bump { request: Box<ReviewRequest> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub target: Target,
    pub action: Action,
    /// Golden text, stored in `review_messages.text` and shown in the preview.
    pub text: String,
    /// Text handed to `slack_send_message` (§7.6).
    pub wire_text: String,
    pub warnings: Vec<String>,
}

/// The active request for `(pr, workspace, channel)`, if any.
pub fn active_request(
    store: &Store,
    pr: &PullRequest,
    workspace: &str,
    channel_id: &str,
) -> Result<Option<ReviewRequest>, CoreError> {
    Ok(store
        .list_review_requests(pr.id)?
        .into_iter()
        .find(|request| {
            request.status == RequestStatus::Active
                && request.channel_id == channel_id
                && same_workspace(request, workspace)
        }))
}

/// `key` is a team ID or a domain. A request with a team matches by team;
/// one saved before team IDs matches by domain.
fn same_workspace(request: &ReviewRequest, key: &str) -> bool {
    if crate::domain::is_slack_team_id(key) {
        request.team_id.as_deref() == Some(key)
    } else {
        request.workspace.eq_ignore_ascii_case(key)
    }
}

/// The active request for a target: same team when both sides know it,
/// else the same domain (rows saved before team IDs).
pub fn active_for_target(
    store: &Store,
    pr: &PullRequest,
    target: &Target,
) -> Result<Option<ReviewRequest>, CoreError> {
    Ok(store
        .list_review_requests(pr.id)?
        .into_iter()
        .find(|request| {
            request.status == RequestStatus::Active
                && request.channel_id == target.channel_id
                && match (&request.team_id, &target.team_id) {
                    (Some(have), Some(want)) => have == want,
                    _ => request.workspace.eq_ignore_ascii_case(&target.workspace),
                }
        }))
}

/// Build the preview for one channel.
///
/// `min_bump_interval_hours` is `review.min_bump_interval_hours` (default 24).
/// A bump sooner than that warns but does not block (§7.7).
pub fn plan(
    store: &Store,
    pr: &PullRequest,
    target: Target,
    requester: Option<&str>,
    min_bump_interval_hours: i64,
    now: DateTime<Utc>,
) -> Result<Plan, CoreError> {
    let mut warnings = Vec::new();
    let active = active_for_target(store, pr, &target)?;
    let (action, text) = match active {
        None => (
            Action::Request,
            message::build(
                MessageMode::Request,
                &[(&pr.title, &pr.url)],
                requester,
                None,
            )?,
        ),
        Some(request) => {
            let last = request.last_bumped_at.unwrap_or(request.requested_at);
            if now - last < Duration::hours(min_bump_interval_hours) {
                warnings.push(format!(
                    "The last message in {} is less than {min_bump_interval_hours} h old.",
                    target.label()
                ));
            }
            (
                Action::Bump {
                    request: Box::new(request),
                },
                message::build(MessageMode::Remind, &[(&pr.title, &pr.url)], None, None)?,
            )
        }
    };
    Ok(Plan {
        target,
        action,
        wire_text: wire_text(&text),
        text,
        warnings,
    })
}

/// Appends the sender's optional note after the message, separated by a
/// blank line, to both the stored and the sent text. Blank notes change
/// nothing. The preview shows the same text before the send.
pub fn with_note(mut plan: Plan, note: Option<&str>) -> Plan {
    let Some(note) = note.map(str::trim).filter(|note| !note.is_empty()) else {
        return plan;
    };
    let append = |text: &str| format!("{}\n\n{note}\n", text.trim_end());
    plan.text = append(&plan.text);
    plan.wire_text = append(&plan.wire_text);
    plan
}

/// §7.6: the text is sent unchanged. Titles are already inline code spans
/// (see [`message::code_span`]), so Slack shows them literally.
pub fn wire_text(text: &str) -> String {
    text.to_owned()
}

/// Fill in the message link from the workspace domain when Slack returned
/// only the `ts` (§7.11).
pub(crate) fn with_link(mut posted: PostedMessage, target: &Target) -> PostedMessage {
    if posted.permalink.is_none() && !target.workspace.is_empty() {
        posted.permalink = Some(super::permalink::build(
            &target.workspace,
            &posted.channel_id,
            &posted.ts,
            posted.thread_ts.as_deref(),
        ));
        posted.workspace = Some(target.workspace.clone());
    }
    posted
}

/// Post the planned message and record it. Returns the active request row.
///
/// Slack is written first. If the store write then fails, the message is
/// already in Slack and the error says so.
pub fn send(
    store: &Store,
    chat: &dyn ChatProvider,
    pr: &PullRequest,
    plan: &Plan,
    teams_json: &str,
    now: DateTime<Utc>,
) -> Result<ReviewRequest, CoreError> {
    let (thread_ts, kind) = match &plan.action {
        Action::Request => (None, ReviewMessageKind::Request),
        Action::Bump { request } => (Some(request.root_ts.as_str()), ReviewMessageKind::Bump),
    };
    let broadcast = thread_ts.is_some() && crate::settings::bump_broadcast(store)?;
    let posted = chat
        .post(&OutgoingMessage {
            channel_id: &plan.target.channel_id,
            text: &plan.wire_text,
            thread_ts,
            reply_broadcast: broadcast,
            unfurl_app_links: thread_ts.is_none(),
        })
        .map_err(CoreError::Provider)?;
    let posted = with_link(posted, &plan.target);
    let recorded = (|| {
        let message = store.insert_review_message(ReviewMessage {
            id: 0,
            slack_account_id: chat.account_id(),
            channel_id: plan.target.channel_id.clone(),
            channel_name: plan.target.channel_name.clone(),
            ts: posted.ts.clone(),
            thread_root_ts: thread_ts.map(str::to_owned),
            kind,
            origin: ReviewMessageOrigin::Posted,
            permalink: posted.permalink.clone(),
            text: plan.text.clone(),
            batch_id: None,
            posted_at: now,
        })?;
        store.insert_review_message_pr(ReviewMessagePr {
            message_id: message.id,
            pr_id: pr.id,
            head_sha: Some(pr.head_sha.clone()),
            teams_json: teams_json.to_owned(),
        })?;
        match &plan.action {
            Action::Request => store.insert_review_request(ReviewRequest {
                id: ReviewRequestId(0),
                pr: pr.id,
                root: ProviderRef {
                    account: chat.account_id(),
                    id: format!("{}:{}", plan.target.channel_id, posted.ts),
                },
                workspace: plan.target.workspace.clone(),
                team_id: plan.target.team_id.clone(),
                channel_id: plan.target.channel_id.clone(),
                channel_name: plan.target.channel_name.clone(),
                root_ts: posted.ts.clone(),
                permalink: posted.permalink.clone(),
                requested_at: now,
                last_bumped_at: None,
                bump_count: 0,
                status: RequestStatus::Active,
                closed_at: None,
                source: RequestSource::App,
            }),
            Action::Bump { request } => store.record_bump(request.id, now),
        }
    })();
    recorded.map_err(|err| {
        CoreError::Store(format!(
            "posted to Slack ({}) but could not save it: {err}",
            posted.permalink.as_deref().unwrap_or(&posted.ts)
        ))
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::domain::{
        Account, AccountId, AccountKind, AuthMode, ChatKind, ChecksState, Involvement,
        MergeStateStatus, Mergeable, PrId, PrState, RepoRef, Repository, ReviewDecision,
    };
    use crate::error::ProviderError;
    use crate::providers::{ChatChannel, ChatUser, Page};

    /// channel, text, thread_ts, reply_broadcast, unfurl_app_links
    type Post = (String, String, Option<String>, bool, bool);

    #[derive(Default)]
    pub(crate) struct FakeChat {
        pub(crate) account: i64,
        pub(crate) posts: Mutex<Vec<Post>>,
        pub(crate) fail: bool,
    }

    impl ChatProvider for FakeChat {
        fn kind(&self) -> ChatKind {
            ChatKind::Slack
        }
        fn whoami(&self) -> Result<ChatUser, ProviderError> {
            unimplemented!()
        }
        fn list_channels(&self, _: Option<String>) -> Result<Page<ChatChannel>, ProviderError> {
            unimplemented!()
        }
        fn channel_info(&self, _: &str) -> Result<ChatChannel, ProviderError> {
            Err(ProviderError::Unsupported("fake".into()))
        }
        fn post(&self, message: &OutgoingMessage<'_>) -> Result<PostedMessage, ProviderError> {
            if self.fail {
                return Err(ProviderError::Api {
                    code: "not_in_channel".into(),
                    message: "not in channel".into(),
                });
            }
            let mut posts = self.posts.lock().unwrap();
            posts.push((
                message.channel_id.to_owned(),
                message.text.to_owned(),
                message.thread_ts.map(str::to_owned),
                message.reply_broadcast,
                message.unfurl_app_links,
            ));
            let ts = format!("1700000{:03}.000100", posts.len());
            Ok(PostedMessage {
                channel_id: message.channel_id.to_owned(),
                permalink: Some(format!(
                    "https://acme.slack.com/archives/{}/p{}",
                    message.channel_id,
                    ts.replace('.', "")
                )),
                thread_ts: message.thread_ts.map(str::to_owned),
                ts,
                workspace: Some("acme".into()),
            })
        }
        fn account_id(&self) -> AccountId {
            AccountId(self.account)
        }
    }

    pub(crate) fn at(hours: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-26T04:40:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::hours(hours)
    }

    pub(crate) fn seed(title: &str) -> (tempfile::TempDir, Store, PullRequest, FakeChat) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let slack = store
            .insert_account(Account {
                id: AccountId(0),
                kind: AccountKind::Slack,
                display_name: "Acme".into(),
                host: "acme.slack.com".into(),
                external_id: "T0ACME:U0ALICE".into(),
                slack_workspace: Some("acme".into()),
                login: "U0ALICE".into(),
                auth_mode: AuthMode::SlackMcpOauth,
                keychain_key: None,
                created_at: at(0),
            })
            .unwrap();
        let chat = FakeChat {
            account: slack.id.0,
            ..FakeChat::default()
        };
        let account = store
            .insert_account(Account {
                id: AccountId(0),
                kind: AccountKind::GitHub,
                display_name: "alice".into(),
                host: "github.com".into(),
                external_id: "U1".into(),
                slack_workspace: None,
                login: "alice".into(),
                auth_mode: AuthMode::GhCli,
                keychain_key: None,
                created_at: at(0),
            })
            .unwrap();
        let repo = store
            .insert_repository(Repository {
                id: 0,
                account_id: account.id,
                owner: "acme".into(),
                name: "web".into(),
                node_id: None,
                default_branch: Some("main".into()),
                archived: false,
            })
            .unwrap();
        let pr = store
            .upsert_pull_request(
                repo.id,
                PullRequest {
                    id: PrId(0),
                    remote: ProviderRef {
                        account: account.id,
                        id: "PR_1".into(),
                    },
                    repo: RepoRef {
                        host: "github.com".into(),
                        owner: "acme".into(),
                        name: "web".into(),
                    },
                    number: 7,
                    title: title.into(),
                    url: "https://github.com/acme/web/pull/7".into(),
                    author_login: "alice".into(),
                    body: String::new(),
                    state: PrState::Open,
                    is_draft: false,
                    head_ref: "f".into(),
                    base_ref: "main".into(),
                    head_sha: "abc".into(),
                    labels: vec![],
                    review_decision: Some(ReviewDecision::ReviewRequired),
                    checks: ChecksState::Success,
                    mergeable: Mergeable::Mergeable,
                    merge_state: MergeStateStatus::Clean,
                    requested_reviewers: vec![],
                    unresolved_threads: 0,
                    total_threads: 0,
                    involvement: Involvement::AUTHOR,
                    created_at: at(0),
                    updated_at: at(0),
                    last_activity_at: at(0),
                    synced_at: at(0),
                    hidden: false,
                },
            )
            .unwrap();
        (dir, store, pr, chat)
    }

    pub(crate) fn target() -> Target {
        Target {
            team_id: None,
            workspace: "acme".into(),
            channel_id: "C0TEAM".into(),
            channel_name: "team-web".into(),
        }
    }

    #[test]
    fn request_then_bump_in_the_thread() {
        let (_dir, store, pr, chat) = seed("Fix the latch");

        let first = plan(&store, &pr, target(), None, 24, at(0)).unwrap();
        assert_eq!(first.action, Action::Request);
        assert_eq!(
            first.text,
            "Hello team\nCould someone please review `Fix the latch`?\nhttps://github.com/acme/web/pull/7\nThanks!\n"
        );
        assert!(first.warnings.is_empty());
        let request = send(&store, &chat, &pr, &first, "[]", at(0)).unwrap();
        assert_eq!(request.status, RequestStatus::Active);
        assert_eq!(request.root_ts, "1700000001.000100");
        assert_eq!(
            request.permalink.as_deref(),
            Some("https://acme.slack.com/archives/C0TEAM/p1700000001000100")
        );
        assert_eq!(request.bump_count, 0);

        let soon = plan(&store, &pr, target(), None, 24, at(2)).unwrap();
        assert!(matches!(soon.action, Action::Bump { .. }));
        assert!(
            soon.text
                .starts_with("Hello, I'm still looking for a review on `Fix the latch`")
        );
        assert_eq!(soon.warnings.len(), 1, "bump within 24 h warns");

        let later = plan(&store, &pr, target(), None, 24, at(30)).unwrap();
        assert!(later.warnings.is_empty());
        let bumped = send(&store, &chat, &pr, &later, "[]", at(30)).unwrap();
        assert_eq!(bumped.id, request.id, "a bump never creates a row");
        assert_eq!(bumped.bump_count, 1);
        assert_eq!(bumped.last_bumped_at, Some(at(30)));
        assert_eq!(store.list_review_requests(pr.id).unwrap().len(), 1);

        let posts = chat.posts.lock().unwrap();
        assert_eq!(posts[0].2, None);
        assert!(!posts[0].3 && posts[0].4, "request unfurls, no broadcast");
        assert_eq!(posts[1].2.as_deref(), Some("1700000001.000100"));
        assert!(posts[1].3, "bump is broadcast to the channel");
    }

    #[test]
    fn bump_stays_in_the_thread_when_broadcast_is_off() {
        let (_dir, store, pr, chat) = seed("Fix");
        crate::settings::set_bump_broadcast(&store, false).unwrap();
        let first = plan(&store, &pr, target(), None, 24, at(0)).unwrap();
        send(&store, &chat, &pr, &first, "[]", at(0)).unwrap();
        let later = plan(&store, &pr, target(), None, 24, at(30)).unwrap();
        assert!(matches!(later.action, Action::Bump { .. }));
        send(&store, &chat, &pr, &later, "[]", at(30)).unwrap();
        let posts = chat.posts.lock().unwrap();
        assert_eq!(posts[1].2.as_deref(), Some("1700000001.000100"));
        assert!(!posts[1].3, "thread reply only");
    }

    #[test]
    fn a_failed_post_records_nothing() {
        let (_dir, store, pr, mut chat) = seed("Fix");
        chat.fail = true;
        let planned = plan(&store, &pr, target(), None, 24, at(0)).unwrap();
        let err = send(&store, &chat, &pr, &planned, "[]", at(0)).unwrap_err();
        assert!(matches!(
            err,
            CoreError::Provider(ProviderError::Api { .. })
        ));
        assert!(store.list_review_requests(pr.id).unwrap().is_empty());
    }

    #[test]
    fn markdown_titles_are_code_spans_without_warnings() {
        let (_dir, store, pr, _chat) = seed("feat: return location_ip_hash *_init_* [WIP]");
        let planned = plan(&store, &pr, target(), None, 24, at(0)).unwrap();
        assert_eq!(planned.wire_text, planned.text);
        assert!(planned.warnings.is_empty(), "{:?}", planned.warnings);
        assert!(
            planned
                .text
                .contains("review `feat: return location_ip_hash *_init_* [WIP]`?")
        );
    }

    #[test]
    fn a_note_is_appended_after_a_blank_line_and_blank_notes_are_ignored() {
        let (_dir, store, pr, _chat) = seed("Tighten the latch");
        let planned = plan(&store, &pr, target(), None, 24, at(0)).unwrap();
        let same = with_note(planned.clone(), Some("   "));
        assert_eq!(same.text, planned.text);
        let noted = with_note(planned.clone(), Some(" Small one, 5 min \n"));
        assert!(
            noted.text.ends_with("Thanks!\n\nSmall one, 5 min\n"),
            "{}",
            noted.text
        );
        assert_eq!(noted.wire_text, noted.text);
    }

    #[test]
    fn record_bump_refuses_a_closed_request() {
        let (_dir, store, pr, chat) = seed("Fix");
        let planned = plan(&store, &pr, target(), None, 24, at(0)).unwrap();
        let request = send(&store, &chat, &pr, &planned, "[]", at(0)).unwrap();
        store
            .close_review_request(request.id, crate::domain::CloseReason::Dismissed, at(1))
            .unwrap();
        assert!(store.record_bump(request.id, at(2)).is_err());
    }

    #[test]
    fn dm_targets_read_as_dm_in_warnings() {
        let target = |id: &str, name: &str| Target {
            team_id: None,
            workspace: "acme".into(),
            channel_id: id.into(),
            channel_name: name.into(),
        };
        assert_eq!(target("C0123456789", "reviews").label(), "#reviews");
        assert_eq!(target("C0123456789", "#reviews").label(), "#reviews");
        assert_eq!(target("D0123456789", "D0123456789").label(), "DM");
        assert_eq!(target("D0123456789", "ada").label(), "DM · ada");
        assert_eq!(
            target("U0123456789", "@Grace Hopper").label(),
            "@Grace Hopper"
        );
        assert_eq!(target("U0123456789", "").label(), "@U0123456789");
    }
}
