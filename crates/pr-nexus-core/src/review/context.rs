//! The PR detail's Slack context panel (§7.10).
//!
//! Only threads PR Nexus knows are read: the roots of the PR's
//! `review_requests` (active first, then closed) and of its other
//! `review_messages`. Each thread is read with the chat provider's
//! `read_thread` and kept in `chat_thread_cache` for
//! [`THREAD_CACHE_TTL_MINUTES`]. Nothing is browsed, crawled, or posted.

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::Serialize;

use crate::domain::{
    AccountId, AccountKind, ChatThreadCacheEntry, CloseReason, PrId, RequestStatus, ReviewMessage,
    ReviewRequest,
};
use crate::error::{CoreError, ProviderError};
use crate::providers::{ChatProvider, SearchHit, ThreadMessage, ThreadReplies};
use crate::store::Store;

use super::batch::ChatFor;
use super::permalink;

pub const THREAD_CACHE_TTL_MINUTES: i64 = 5;
/// Replies shown per thread (§7.10).
pub const THREAD_REPLY_LIMIT: u32 = 50;
/// Results per Mentions search (§7.10).
pub const MENTIONS_LIMIT: u32 = 20;

/// Where a thread came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    Active,
    Closed,
    /// A posted review message with no request row (e.g. a closed row deleted).
    Posted,
}

/// One known thread and what Slack returned for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextThread {
    pub workspace: String,
    pub channel_id: String,
    pub channel_name: String,
    pub root_ts: String,
    pub permalink: Option<String>,
    pub status: ThreadStatus,
    /// "PR merged", "Superseded", … for closed threads.
    pub close_reason: Option<String>,
    #[serde(serialize_with = "rfc3339")]
    pub requested_at: Option<DateTime<Utc>>,
    /// Stored with the request, so the card can say "Bumped · 9h ago" even
    /// when Slack cannot be read.
    #[serde(serialize_with = "rfc3339")]
    pub last_bumped_at: Option<DateTime<Utc>>,
    pub bump_count: u32,
    /// Parent first, then replies.
    pub messages: Vec<ContextMessage>,
    pub has_more: bool,
    #[serde(serialize_with = "rfc3339")]
    pub fetched_at: Option<DateTime<Utc>>,
    pub from_cache: bool,
    /// The thread could not be read now. A cached copy may still be shown.
    pub error: Option<String>,
    /// Slack rejected the token: the workspace needs to sign in again.
    #[serde(skip)]
    pub unauthorized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextMessage {
    pub ts: String,
    /// From the `ts` seconds.
    #[serde(serialize_with = "rfc3339")]
    pub at: Option<DateTime<Utc>>,
    /// Display name, else the user ID, else "Unknown".
    pub author: String,
    pub user_id: Option<String>,
    pub text: String,
    pub permalink: Option<String>,
    pub is_bot: bool,
}

fn rfc3339<S: serde::Serializer>(
    at: &Option<DateTime<Utc>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match at {
        Some(at) => serializer.serialize_some(&at.to_rfc3339()),
        None => serializer.serialize_none(),
    }
}

/// One known thread, before it is read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Root {
    workspace: String,
    channel_id: String,
    channel_name: String,
    root_ts: String,
    permalink: Option<String>,
    status: ThreadStatus,
    close_reason: Option<CloseReason>,
    requested_at: Option<DateTime<Utc>>,
    last_bumped_at: Option<DateTime<Utc>>,
    bump_count: u32,
    sort_at: DateTime<Utc>,
}

pub fn close_reason_label(reason: CloseReason) -> &'static str {
    match reason {
        CloseReason::PrMerged => "PR merged",
        CloseReason::PrClosed => "PR closed",
        CloseReason::Dismissed => "Dismissed",
        CloseReason::Superseded => "Superseded",
        CloseReason::ChannelArchived => "Channel archived",
    }
}

/// Known threads of `pr`: active requests (newest first), then closed ones,
/// then roots of other review messages. Each root appears once.
fn roots(store: &Store, pr: PrId) -> Result<Vec<Root>, CoreError> {
    let requests = store.list_review_requests(pr)?;
    let messages = store.list_review_messages_for_pr(pr)?;
    Ok(collect_roots(store, &requests, &messages))
}

fn collect_roots(
    store: &Store,
    requests: &[ReviewRequest],
    messages: &[ReviewMessage],
) -> Vec<Root> {
    let mut out: Vec<Root> = requests
        .iter()
        .map(|request| {
            let (status, close_reason) = match request.status {
                RequestStatus::Active => (ThreadStatus::Active, None),
                RequestStatus::Closed(reason) => (ThreadStatus::Closed, Some(reason)),
            };
            Root {
                workspace: request.workspace.clone(),
                channel_id: request.channel_id.clone(),
                channel_name: request.channel_name.clone(),
                root_ts: request.root_ts.clone(),
                permalink: request.permalink.clone(),
                status,
                close_reason,
                requested_at: Some(request.requested_at),
                last_bumped_at: request.last_bumped_at,
                bump_count: request.bump_count,
                sort_at: request.closed_at.unwrap_or(request.requested_at),
            }
        })
        .collect();
    for message in messages {
        let root_ts = message
            .thread_root_ts
            .clone()
            .unwrap_or_else(|| message.ts.clone());
        let workspace = store
            .get_account(message.slack_account_id)
            .ok()
            .flatten()
            .and_then(|account| account.slack_workspace)
            .unwrap_or_default();
        out.push(Root {
            permalink: (message.thread_root_ts.is_none())
                .then(|| message.permalink.clone())
                .flatten()
                .or_else(|| {
                    (!workspace.is_empty())
                        .then(|| permalink::build(&workspace, &message.channel_id, &root_ts, None))
                }),
            workspace,
            channel_id: message.channel_id.clone(),
            channel_name: message.channel_name.clone(),
            root_ts,
            status: ThreadStatus::Posted,
            close_reason: None,
            requested_at: Some(message.posted_at),
            last_bumped_at: None,
            bump_count: 0,
            sort_at: message.posted_at,
        });
    }
    let rank = |status: ThreadStatus| match status {
        ThreadStatus::Active => 0,
        ThreadStatus::Closed => 1,
        ThreadStatus::Posted => 2,
    };
    // Stable: within a rank, newest first.
    out.sort_by(|a, b| {
        rank(a.status)
            .cmp(&rank(b.status))
            .then(b.sort_at.cmp(&a.sort_at))
    });
    let mut seen = BTreeSet::new();
    out.retain(|root| seen.insert((root.channel_id.clone(), root.root_ts.clone())));
    out
}

/// Read every known thread of `pr`, from the cache when it is younger than
/// [`THREAD_CACHE_TTL_MINUTES`] (and `refresh` is false).
pub fn context_threads(
    store: &Store,
    pr: PrId,
    chat_for: &mut ChatFor<'_>,
    now: DateTime<Utc>,
    refresh: bool,
) -> Result<Vec<ContextThread>, CoreError> {
    let mut threads = Vec::new();
    for root in roots(store, pr)? {
        threads.push(read_root(store, &root, chat_for, now, refresh)?);
    }
    Ok(threads)
}

fn read_root(
    store: &Store,
    root: &Root,
    chat_for: &mut ChatFor<'_>,
    now: DateTime<Utc>,
    refresh: bool,
) -> Result<ContextThread, CoreError> {
    let mut thread = ContextThread {
        workspace: root.workspace.clone(),
        channel_id: root.channel_id.clone(),
        channel_name: root.channel_name.clone(),
        root_ts: root.root_ts.clone(),
        permalink: root.permalink.clone(),
        status: root.status,
        close_reason: root
            .close_reason
            .map(|reason| close_reason_label(reason).to_owned()),
        requested_at: root.requested_at,
        last_bumped_at: root.last_bumped_at,
        bump_count: root.bump_count,
        messages: Vec::new(),
        has_more: false,
        fetched_at: None,
        from_cache: false,
        error: None,
        unauthorized: false,
    };
    let chat = match chat_for(&root.workspace) {
        Ok(chat) => chat,
        Err(err) => {
            // Signed out: still show the last copy read with this workspace.
            thread.error = Some(core_message(err));
            if let Some(account) = slack_account_for(store, &root.workspace)?
                && let Some((replies, fetched_at)) = cached_replies(store, account, root)?
            {
                fill(&mut thread, &replies, fetched_at, true);
            }
            return Ok(thread);
        }
    };
    let account = chat.account_id();
    let cached = cached_replies(store, account, root)?;
    if let Some((replies, fetched_at)) = &cached
        && !refresh
        && now - *fetched_at < Duration::minutes(THREAD_CACHE_TTL_MINUTES)
    {
        fill(&mut thread, replies, *fetched_at, true);
        return Ok(thread);
    }
    match fetch(chat.as_ref(), &root.channel_id, &root.root_ts) {
        Ok(replies) => {
            store.upsert_chat_thread(ChatThreadCacheEntry {
                slack_account_id: account,
                channel_id: root.channel_id.clone(),
                thread_root_ts: root.root_ts.clone(),
                messages_json: serde_json::to_string(&replies)
                    .map_err(|err| CoreError::Store(err.to_string()))?,
                fetched_at: now,
            })?;
            fill(&mut thread, &replies, now, false);
        }
        Err(err) => {
            thread.unauthorized = matches!(err, ProviderError::Unauthorized(_));
            thread.error = Some(provider_message(&err));
            if let Some((replies, fetched_at)) = &cached {
                fill(&mut thread, replies, *fetched_at, true);
            }
        }
    }
    Ok(thread)
}

fn cached_replies(
    store: &Store,
    account: AccountId,
    root: &Root,
) -> Result<Option<(ThreadReplies, DateTime<Utc>)>, CoreError> {
    let cached = store.get_chat_thread(account, root.channel_id.clone(), root.root_ts.clone())?;
    Ok(cached.and_then(|entry| {
        serde_json::from_str::<ThreadReplies>(&entry.messages_json)
            .ok()
            .map(|replies| (replies, entry.fetched_at))
    }))
}

fn slack_account_for(store: &Store, workspace: &str) -> Result<Option<AccountId>, CoreError> {
    if workspace.is_empty() {
        return Ok(None);
    }
    Ok(store
        .list_accounts()?
        .into_iter()
        .find(|account| {
            account.kind == AccountKind::Slack
                && account
                    .slack_workspace
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(workspace))
        })
        .map(|account| account.id))
}

fn core_message(err: CoreError) -> String {
    match err {
        CoreError::Validation(message) => message,
        CoreError::Provider(err) => provider_message(&err),
        other => other.to_string(),
    }
}

/// A short line for the panel.
pub fn provider_message(err: &ProviderError) -> String {
    match err {
        ProviderError::Unauthorized(_) => {
            "Slack sign-in expired. Reconnect in Settings → Slack.".into()
        }
        ProviderError::RateLimited { reset_at, .. } => {
            format!(
                "Slack is rate limiting. Try again after {}.",
                reset_at.format("%H:%M UTC")
            )
        }
        ProviderError::Unsupported(_) => "This Slack connection cannot read threads.".into(),
        other => other.to_string(),
    }
}

/// Read one thread and name its authors.
fn fetch(
    chat: &dyn ChatProvider,
    channel_id: &str,
    root_ts: &str,
) -> Result<ThreadReplies, ProviderError> {
    let mut replies = chat.read_thread(channel_id, root_ts, THREAD_REPLY_LIMIT)?;
    for message in &mut replies.messages {
        if message.author.is_none()
            && let Some(user) = &message.user_id
        {
            // A failed lookup leaves the ID; the panel still renders.
            message.author = chat.user_name(user).ok();
        }
    }
    Ok(replies)
}

fn fill(
    thread: &mut ContextThread,
    replies: &ThreadReplies,
    fetched_at: DateTime<Utc>,
    from_cache: bool,
) {
    thread.messages = replies
        .messages
        .iter()
        .take(THREAD_REPLY_LIMIT as usize + 1)
        .map(|message| context_message(thread, message))
        .collect();
    thread.has_more = replies.has_more || replies.messages.len() > THREAD_REPLY_LIMIT as usize + 1;
    thread.fetched_at = Some(fetched_at);
    thread.from_cache = from_cache;
}

fn context_message(thread: &ContextThread, message: &ThreadMessage) -> ContextMessage {
    let link = (!thread.workspace.is_empty()).then(|| {
        permalink::build(
            &thread.workspace,
            &thread.channel_id,
            &message.ts,
            Some(&thread.root_ts),
        )
    });
    ContextMessage {
        ts: message.ts.clone(),
        at: ts_time(&message.ts),
        author: message
            .author
            .clone()
            .or_else(|| message.user_id.clone())
            .unwrap_or_else(|| "Unknown".into()),
        user_id: message.user_id.clone(),
        text: message.text.clone(),
        permalink: if message.ts == thread.root_ts {
            thread.permalink.clone().or(link)
        } else {
            link
        },
        is_bot: message.is_bot,
    }
}

/// `1700000000.000100` → its UTC time.
pub fn ts_time(ts: &str) -> Option<DateTime<Utc>> {
    let (secs, micros) = ts.split_once('.')?;
    let secs: i64 = secs.parse().ok()?;
    let micros: u32 = micros.parse().ok()?;
    Utc.timestamp_opt(secs, micros.saturating_mul(1000))
        .single()
}

/// The phrase the Mentions search looks for: `github.com/ORG/REPO/pull/N`.
pub fn mention_phrase(pr_url: &str) -> String {
    let trimmed = pr_url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    trimmed.to_owned()
}

/// Mentions found in one workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MentionGroup {
    pub workspace: String,
    pub hits: Vec<SearchHit>,
    pub error: Option<String>,
    #[serde(skip)]
    pub unauthorized: bool,
}

/// One Mentions search per workspace (§7.10). Hits that are this PR's own
/// known messages are dropped: the panel already shows those threads.
pub fn mentions(
    store: &Store,
    pr: PrId,
    pr_url: &str,
    workspaces: &[(String, Arc<dyn ChatProvider>)],
    private: bool,
) -> Result<Vec<MentionGroup>, CoreError> {
    let phrase = mention_phrase(pr_url);
    let mut known: BTreeSet<(String, String)> = BTreeSet::new();
    for request in store.list_review_requests(pr)? {
        known.insert((request.channel_id, request.root_ts));
    }
    for message in store.list_review_messages_for_pr(pr)? {
        known.insert((message.channel_id, message.ts));
    }
    Ok(workspaces
        .iter()
        .map(
            |(workspace, chat)| match chat.search_messages(&phrase, private, MENTIONS_LIMIT) {
                Ok(hits) => MentionGroup {
                    workspace: workspace.clone(),
                    hits: hits
                        .into_iter()
                        .filter(|hit| !known.contains(&(hit.channel_id.clone(), hit.ts.clone())))
                        .collect(),
                    error: None,
                    unauthorized: false,
                },
                Err(err) => MentionGroup {
                    workspace: workspace.clone(),
                    hits: Vec::new(),
                    unauthorized: matches!(err, ProviderError::Unauthorized(_)),
                    error: Some(provider_message(&err)),
                },
            },
        )
        .collect())
}

#[cfg(test)]
mod tests;
