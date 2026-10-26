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
use crate::providers::{ChatProvider, OutgoingMessage};
use crate::store::Store;

use super::message::{self, MessageMode};

/// Where the message goes. `workspace` is the domain (`acme`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub workspace: String,
    pub channel_id: String,
    pub channel_name: String,
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
                && request.workspace == workspace
                && request.channel_id == channel_id
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
    let active = active_request(store, pr, &target.workspace, &target.channel_id)?;
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
                    "The last message in #{} is less than {min_bump_interval_hours} h old.",
                    target.channel_name
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
    if has_markdown_chars(&pr.title) {
        warnings.push(
            "The title has Markdown characters. Slack may format them instead of showing them literally."
                .to_owned(),
        );
    }
    Ok(Plan {
        target,
        action,
        wire_text: wire_text(&text),
        text,
        warnings,
    })
}

/// §7.6 decision: the M10 read-back check has not run against a live
/// workspace, so the transform is the identity (what the skill sends) and the
/// preview warns about Markdown-active titles instead.
pub fn wire_text(text: &str) -> String {
    text.to_owned()
}

/// True when a title would be reformatted by Slack's Markdown conversion.
pub fn has_markdown_chars(title: &str) -> bool {
    let title = title.trim();
    if title
        .chars()
        .any(|c| matches!(c, '\\' | '*' | '_' | '`' | '~' | '[' | ']' | '<' | '>'))
    {
        return true;
    }
    if title.starts_with(['#', '+', '-']) {
        return true;
    }
    let digits = title.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && title.as_bytes().get(digits) == Some(&b'.')
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
    let posted = chat
        .post(&OutgoingMessage {
            channel_id: &plan.target.channel_id,
            text: &plan.wire_text,
            thread_ts,
            reply_broadcast: thread_ts.is_some(),
            unfurl_app_links: thread_ts.is_none(),
        })
        .map_err(CoreError::Provider)?;
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
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::domain::{
        Account, AccountId, AccountKind, AuthMode, ChatKind, ChecksState, Involvement,
        MergeStateStatus, Mergeable, PrId, PrState, RepoRef, Repository, ReviewDecision,
    };
    use crate::error::ProviderError;
    use crate::providers::{ChatChannel, ChatUser, Page, PostedMessage};

    /// channel, text, thread_ts, reply_broadcast, unfurl_app_links
    type Post = (String, String, Option<String>, bool, bool);

    #[derive(Default)]
    struct FakeChat {
        account: i64,
        posts: Mutex<Vec<Post>>,
        fail: bool,
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
            unimplemented!()
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

    fn at(hours: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-26T04:40:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::hours(hours)
    }

    fn seed(title: &str) -> (tempfile::TempDir, Store, PullRequest, FakeChat) {
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

    fn target() -> Target {
        Target {
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
            "Hello team\nCould someone please review Fix the latch?\nhttps://github.com/acme/web/pull/7\nThanks!\n"
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
                .starts_with("Hello, I'm still looking for a review on Fix the latch")
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
    fn markdown_titles_warn_and_wire_text_is_identity() {
        let (_dir, store, pr, _chat) = seed("fix *_init_* [WIP]");
        let planned = plan(&store, &pr, target(), None, 24, at(0)).unwrap();
        assert_eq!(planned.wire_text, planned.text);
        assert_eq!(planned.warnings.len(), 1);
        assert!(has_markdown_chars("1. Title"));
        assert!(has_markdown_chars("- dash"));
        assert!(!has_markdown_chars("Plain title 2.0"));
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
}
