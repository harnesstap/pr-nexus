use std::sync::{Arc, Mutex};

use chrono::Duration;

use super::*;
use crate::domain::{AccountId, ChatKind, CloseReason};
use crate::providers::{ChatChannel, ChatUser, OutgoingMessage, Page, PostedMessage};
use crate::review::lifecycle::tests::{at, seed, target};
use crate::review::lifecycle::{plan, send};

/// Answers `read_thread` from a script and counts calls.
#[derive(Default)]
struct ThreadChat {
    account: i64,
    reads: Mutex<Vec<(String, String, u32)>>,
    lookups: Mutex<Vec<String>>,
    searches: Mutex<Vec<(String, bool, u32)>>,
    replies: ThreadReplies,
    unauthorized: bool,
    hits: Vec<SearchHit>,
}

impl ChatProvider for ThreadChat {
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
    fn post(&self, _: &OutgoingMessage<'_>) -> Result<PostedMessage, ProviderError> {
        panic!("the context panel never posts")
    }
    fn account_id(&self) -> AccountId {
        AccountId(self.account)
    }
    fn read_thread(
        &self,
        channel_id: &str,
        root_ts: &str,
        limit: u32,
    ) -> Result<ThreadReplies, ProviderError> {
        self.reads
            .lock()
            .unwrap()
            .push((channel_id.into(), root_ts.into(), limit));
        if self.unauthorized {
            return Err(ProviderError::Unauthorized(
                "slack_read_thread: invalid_auth".into(),
            ));
        }
        Ok(self.replies.clone())
    }
    fn user_name(&self, user_id: &str) -> Result<String, ProviderError> {
        self.lookups.lock().unwrap().push(user_id.into());
        Ok(format!("Name of {user_id}"))
    }
    fn search_messages(
        &self,
        phrase: &str,
        private: bool,
        limit: u32,
    ) -> Result<Vec<SearchHit>, ProviderError> {
        self.searches
            .lock()
            .unwrap()
            .push((phrase.into(), private, limit));
        Ok(self.hits.clone())
    }
}

fn message(ts: &str, user: &str, author: Option<&str>, text: &str) -> ThreadMessage {
    ThreadMessage {
        ts: ts.into(),
        user_id: Some(user.into()),
        author: author.map(str::to_owned),
        text: text.into(),
        is_bot: false,
    }
}

/// A PR with one active request (root `1700000001.000100` in `C0TEAM`).
fn with_request() -> (tempfile::TempDir, Store, crate::domain::PullRequest, i64) {
    let (dir, store, pr, chat) = seed("Fix the latch");
    let planned = plan(&store, &pr, target(), None, 24, at(0)).unwrap();
    send(&store, &chat, &pr, &planned, "[]", at(0)).unwrap();
    (dir, store, pr, chat.account)
}

fn replies() -> ThreadReplies {
    ThreadReplies {
        messages: vec![
            message(
                "1700000001.000100",
                "U0ACME0001",
                Some("Ada Acme"),
                "Could someone review?",
            ),
            message("1700000050.000200", "U0ACME0002", None, "On it"),
        ],
        has_more: false,
    }
}

#[test]
fn reads_known_threads_names_authors_and_caches_for_five_minutes() {
    let (_dir, store, pr, account) = with_request();
    let chat = Arc::new(ThreadChat {
        account,
        replies: replies(),
        ..ThreadChat::default()
    });
    let provider: Arc<dyn ChatProvider> = chat.clone();
    let mut chat_for = |_: &str| Ok(provider.clone());

    let threads = context_threads(&store, pr.id, &mut chat_for, at(1), false).unwrap();
    assert_eq!(threads.len(), 1);
    let thread = &threads[0];
    assert_eq!(thread.status, ThreadStatus::Active);
    assert_eq!(thread.channel_id, "C0TEAM");
    assert!(!thread.from_cache);
    assert_eq!(thread.messages.len(), 2);
    assert_eq!(thread.messages[0].author, "Ada Acme");
    assert_eq!(
        thread.messages[0].permalink.as_deref(),
        Some("https://acme.slack.com/archives/C0TEAM/p1700000001000100")
    );
    assert_eq!(thread.messages[1].author, "Name of U0ACME0002");
    assert_eq!(
        thread.messages[1].permalink.as_deref(),
        Some(
            "https://acme.slack.com/archives/C0TEAM/p1700000050000200?thread_ts=1700000001.000100&cid=C0TEAM"
        )
    );
    assert_eq!(thread.messages[1].at, ts_time("1700000050.000200"));
    assert_eq!(
        chat.reads.lock().unwrap().as_slice(),
        &[(
            "C0TEAM".to_owned(),
            "1700000001.000100".to_owned(),
            THREAD_REPLY_LIMIT
        )]
    );
    assert_eq!(
        chat.lookups.lock().unwrap().as_slice(),
        &["U0ACME0002".to_owned()]
    );

    let again = context_threads(
        &store,
        pr.id,
        &mut chat_for,
        at(1) + Duration::minutes(4),
        false,
    )
    .unwrap();
    assert!(again[0].from_cache);
    assert_eq!(
        again[0].messages[1].author, "Name of U0ACME0002",
        "names are cached with the thread"
    );
    assert_eq!(chat.reads.lock().unwrap().len(), 1, "fresh cache, no call");

    context_threads(
        &store,
        pr.id,
        &mut chat_for,
        at(1) + Duration::minutes(6),
        false,
    )
    .unwrap();
    assert_eq!(chat.reads.lock().unwrap().len(), 2, "stale after 5 min");
    context_threads(
        &store,
        pr.id,
        &mut chat_for,
        at(1) + Duration::minutes(7),
        true,
    )
    .unwrap();
    assert_eq!(
        chat.reads.lock().unwrap().len(),
        3,
        "Refresh skips the cache"
    );
}

#[test]
fn a_failed_read_keeps_the_cached_copy_and_flags_sign_in() {
    let (_dir, store, pr, account) = with_request();
    let good: Arc<dyn ChatProvider> = Arc::new(ThreadChat {
        account,
        replies: replies(),
        ..ThreadChat::default()
    });
    let mut chat_for = |_: &str| Ok(good.clone());
    context_threads(&store, pr.id, &mut chat_for, at(1), false).unwrap();

    let bad: Arc<dyn ChatProvider> = Arc::new(ThreadChat {
        account,
        unauthorized: true,
        ..ThreadChat::default()
    });
    let mut chat_for = |_: &str| Ok(bad.clone());
    let threads = context_threads(&store, pr.id, &mut chat_for, at(2), false).unwrap();
    assert!(threads[0].unauthorized);
    assert!(threads[0].error.is_some());
    assert!(threads[0].from_cache);
    assert_eq!(threads[0].messages.len(), 2);

    let mut signed_out = |_: &str| -> Result<Arc<dyn ChatProvider>, CoreError> {
        Err(CoreError::Validation(
            "Slack sign-in expired. Reconnect the acme workspace in Settings.".into(),
        ))
    };
    let threads = context_threads(&store, pr.id, &mut signed_out, at(2), false).unwrap();
    assert!(threads[0].messages.is_empty());
    assert!(threads[0].error.as_deref().unwrap().contains("Reconnect"));
}

#[test]
fn active_threads_come_before_closed_ones_with_their_reason() {
    let (_dir, store, pr, account) = with_request();
    let first = store.list_review_requests(pr.id).unwrap().remove(0);
    store
        .close_review_request(first.id, CloseReason::Superseded, at(1))
        .unwrap();
    // A second, active request in another channel.
    let mut other = target();
    other.channel_id = "D0123456789".into();
    other.channel_name = "D0123456789".into();
    let poster = crate::review::lifecycle::tests::FakeChat {
        account,
        ..Default::default()
    };
    let planned = plan(&store, &pr, other, None, 24, at(2)).unwrap();
    send(&store, &poster, &pr, &planned, "[]", at(2)).unwrap();

    let reader: Arc<dyn ChatProvider> = Arc::new(ThreadChat {
        account,
        ..ThreadChat::default()
    });
    let mut chat_for = |_: &str| Ok(reader.clone());
    let threads = context_threads(&store, pr.id, &mut chat_for, at(3), false).unwrap();
    let order: Vec<(&str, ThreadStatus, Option<&str>)> = threads
        .iter()
        .map(|thread| {
            (
                thread.channel_id.as_str(),
                thread.status,
                thread.close_reason.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        order,
        vec![
            ("D0123456789", ThreadStatus::Active, None),
            ("C0TEAM", ThreadStatus::Closed, Some("Superseded")),
        ]
    );
}

#[test]
fn mentions_search_the_pr_link_and_drop_known_messages() {
    let (_dir, store, pr, account) = with_request();
    let hit = |channel: &str, ts: &str| SearchHit {
        channel_id: channel.into(),
        channel_name: "eng".into(),
        ts: ts.into(),
        author: Some("Ada Acme".into()),
        text: "see github.com/acme/web/pull/7".into(),
        permalink: None,
        is_bot: false,
    };
    let chat = Arc::new(ThreadChat {
        account,
        hits: vec![
            hit("C0TEAM", "1700000001.000100"),
            hit("C0ENG", "1700000900.000100"),
        ],
        ..ThreadChat::default()
    });
    let provider: Arc<dyn ChatProvider> = chat.clone();
    let groups = mentions(&store, pr.id, &pr.url, &[("acme".into(), provider)], false).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].hits.len(), 1);
    assert_eq!(groups[0].hits[0].channel_id, "C0ENG");
    assert_eq!(
        chat.searches.lock().unwrap().as_slice(),
        &[(
            "github.com/acme/web/pull/7".to_owned(),
            false,
            MENTIONS_LIMIT
        )]
    );
}

#[test]
fn ts_and_phrase_helpers() {
    assert_eq!(
        ts_time("1700000000.000100").unwrap().timestamp(),
        1_700_000_000
    );
    assert_eq!(ts_time("nope"), None);
    assert_eq!(
        mention_phrase("https://github.com/acme/web/pull/7/"),
        "github.com/acme/web/pull/7"
    );
}
