//! Bulk request and bump (§7.9).
//!
//! [`plan_batch`] groups the routed pull requests by `(workspace, channel)`
//! and decides, per channel, which messages to post: one new request for the
//! PRs never requested there, and bumps for the rest (one reply per root up
//! to `review.bulk_bump_max_threads`, else one top-level remind that
//! supersedes the old roots). [`send_batch`] posts the messages one at a time,
//! paced per channel, and records one `send_attempts` row per PR and channel.
//! A failure never rolls back what was already posted.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Duration, Utc};

use crate::domain::{
    CloseReason, PrId, ProviderRef, PullRequest, RequestSource, RequestStatus, ReviewMessage,
    ReviewMessageKind, ReviewMessageOrigin, ReviewMessagePr, ReviewRequest, ReviewRequestId,
    SendAttempt, SendOutcome,
};
use crate::error::{CoreError, ProviderError};
use crate::providers::{ChatProvider, OutgoingMessage, PostedMessage};
use crate::store::Store;

use super::lifecycle::{Target, active_request, wire_text, with_link};
use super::message::{self, MessageMode};

/// Minimum gap between two posts in the same channel. Slack allows about one
/// message per second per channel (`chat.postMessage`, special tier).
pub const CHANNEL_SPACING: Duration = Duration::milliseconds(1100);

/// A `ratelimited` answer is retried once when Slack asks to wait at most this.
pub const MAX_RATE_LIMIT_WAIT: Duration = Duration::seconds(60);

/// One pull request and the channels it routes to.
#[derive(Debug, Clone)]
pub struct BatchInput {
    pub pr: PullRequest,
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub target: Target,
    /// Teams (`org/slug`) that led here, stored with the message.
    pub teams: Vec<String>,
}

/// A pull request left out of the batch, shown in the preview and recorded
/// as `skipped`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    pub pr: PrId,
    pub title: String,
    pub channel: Option<Target>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchOptions {
    pub requester: Option<String>,
    pub min_bump_interval_hours: i64,
    pub max_threads: usize,
    /// Channels (`workspace`, `channel_id`) where the user chose one
    /// top-level request for all (D5).
    pub single_top_level: BTreeSet<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePlanKind {
    /// New top-level request. Active rows of its PRs, if any, are superseded.
    Request,
    /// Reply under `root_ts`, broadcast to the channel.
    Bump { root_ts: String },
    /// Top-level remind replacing more than `max_threads` roots.
    Remind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    pub pr: PullRequest,
    pub teams: Vec<String>,
    /// The active request in this channel, when there is one.
    pub active: Option<ReviewRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedMessage {
    pub kind: MessagePlanKind,
    pub items: Vec<PlannedItem>,
    pub text: String,
    pub wire_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelCard {
    pub target: Target,
    pub messages: Vec<PlannedMessage>,
    pub warnings: Vec<String>,
    pub single_top_level: bool,
    /// True when "one top-level request for all" would change the plan.
    pub can_switch: bool,
}

impl ChannelCard {
    /// Stable digest of what would be posted, so a send can refuse a card
    /// whose plan moved since the preview.
    pub fn fingerprint(&self) -> String {
        let mut out = format!("{}/{}", self.target.workspace, self.target.channel_id);
        for message in &self.messages {
            let kind = match &message.kind {
                MessagePlanKind::Request => "request".to_owned(),
                MessagePlanKind::Bump { root_ts } => format!("bump:{root_ts}"),
                MessagePlanKind::Remind => "remind".to_owned(),
            };
            let prs: Vec<String> = message
                .items
                .iter()
                .map(|i| i.pr.id.0.to_string())
                .collect();
            out.push_str(&format!("|{kind}:{}:{}", prs.join(","), message.text));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BatchPlan {
    pub cards: Vec<ChannelCard>,
    pub skipped: Vec<Skip>,
}

/// Group and plan. Pure apart from reading the active requests.
pub fn plan_batch(
    store: &Store,
    inputs: &[BatchInput],
    skipped: Vec<Skip>,
    options: &BatchOptions,
    now: DateTime<Utc>,
) -> Result<BatchPlan, CoreError> {
    // (workspace, channel) in first-seen order.
    let mut order: Vec<(String, String)> = Vec::new();
    let mut groups: HashMap<(String, String), (Target, Vec<PlannedItem>)> = HashMap::new();
    for input in inputs {
        for route in &input.routes {
            let key = (
                route.target.workspace.clone(),
                route.target.channel_id.clone(),
            );
            let (_, items) = groups.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                (route.target.clone(), Vec::new())
            });
            if let Some(item) = items.iter_mut().find(|item| item.pr.id == input.pr.id) {
                for team in &route.teams {
                    if !item.teams.contains(team) {
                        item.teams.push(team.clone());
                    }
                }
                continue;
            }
            items.push(PlannedItem {
                pr: input.pr.clone(),
                teams: route.teams.clone(),
                active: active_request(store, &input.pr, &key.0, &key.1)?,
            });
        }
    }
    let mut cards = Vec::new();
    for key in order {
        let (target, items) = groups.remove(&key).expect("grouped key");
        let single = options.single_top_level.contains(&key);
        cards.push(plan_card(target, items, single, options, now)?);
    }
    Ok(BatchPlan { cards, skipped })
}

fn plan_card(
    target: Target,
    items: Vec<PlannedItem>,
    single: bool,
    options: &BatchOptions,
    now: DateTime<Utc>,
) -> Result<ChannelCard, CoreError> {
    let mut warnings = Vec::new();
    let (requested, fresh): (Vec<PlannedItem>, Vec<PlannedItem>) = items
        .iter()
        .cloned()
        .partition(|item| item.active.is_some());
    let can_switch = !requested.is_empty() && items.len() > 1;
    let mut messages = Vec::new();
    if single {
        messages.push(build_message(
            MessagePlanKind::Request,
            items,
            options.requester.as_deref(),
        )?);
    } else {
        let mut roots: Vec<(String, Vec<PlannedItem>)> = Vec::new();
        for item in requested {
            let root = item.active.as_ref().expect("requested").root_ts.clone();
            match roots.iter_mut().find(|(ts, _)| *ts == root) {
                Some((_, list)) => list.push(item),
                None => roots.push((root, vec![item])),
            }
        }
        let recent: Vec<&str> = roots
            .iter()
            .flat_map(|(_, list)| list)
            .filter(|item| {
                let request = item.active.as_ref().expect("requested");
                let last = request.last_bumped_at.unwrap_or(request.requested_at);
                now - last < Duration::hours(options.min_bump_interval_hours)
            })
            .map(|item| item.pr.title.as_str())
            .collect();
        if !recent.is_empty() {
            warnings.push(format!(
                "The last message in {} for {} is less than {} h old.",
                target.label(),
                recent.join(", "),
                options.min_bump_interval_hours
            ));
        }
        if roots.len() > options.max_threads {
            let all: Vec<PlannedItem> = roots.into_iter().flat_map(|(_, list)| list).collect();
            messages.push(build_message(MessagePlanKind::Remind, all, None)?);
        } else {
            for (root_ts, list) in roots {
                messages.push(build_message(
                    MessagePlanKind::Bump { root_ts },
                    list,
                    None,
                )?);
            }
        }
        if !fresh.is_empty() {
            messages.push(build_message(
                MessagePlanKind::Request,
                fresh,
                options.requester.as_deref(),
            )?);
        }
    }
    Ok(ChannelCard {
        target,
        messages,
        warnings,
        single_top_level: single,
        can_switch,
    })
}

fn build_message(
    kind: MessagePlanKind,
    items: Vec<PlannedItem>,
    requester: Option<&str>,
) -> Result<PlannedMessage, CoreError> {
    let pairs: Vec<(&str, &str)> = items
        .iter()
        .map(|item| (item.pr.title.as_str(), item.pr.url.as_str()))
        .collect();
    let (mode, requester) = match kind {
        MessagePlanKind::Request => (MessageMode::Request, requester),
        // §7.5: reminds carry no requester suffix.
        MessagePlanKind::Bump { .. } | MessagePlanKind::Remind => (MessageMode::Remind, None),
    };
    let text = message::build(mode, &pairs, requester, None)?;
    Ok(PlannedMessage {
        kind,
        wire_text: wire_text(&text),
        text,
        items,
    })
}

/// Time source and sleep, so tests run instantly.
pub trait Pace {
    fn now(&self) -> DateTime<Utc>;
    fn sleep(&self, duration: Duration);
}

/// Outcome for one pull request in one channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemResult {
    pub pr: PrId,
    pub title: String,
    pub target: Option<Target>,
    pub outcome: SendOutcome,
    pub detail: String,
    pub permalink: Option<String>,
}

/// Returns the signed-in provider for a workspace domain.
pub type ChatFor<'a> = dyn FnMut(&str) -> Result<std::sync::Arc<dyn ChatProvider>, CoreError> + 'a;

/// Post every message of `plan`, in order, one at a time.
///
/// `chat_for(workspace)` returns the signed-in provider. Cards listed in
/// `stale` are not posted: their plan moved since the preview. Every PR gets
/// a `send_attempts` row under `batch_id`.
pub fn send_batch(
    store: &Store,
    chat_for: &mut ChatFor<'_>,
    plan: &BatchPlan,
    stale: &BTreeSet<(String, String)>,
    batch_id: &str,
    pace: &dyn Pace,
) -> Result<Vec<ItemResult>, CoreError> {
    let mut results = Vec::new();
    for skip in &plan.skipped {
        results.push(ItemResult {
            pr: skip.pr,
            title: skip.title.clone(),
            target: skip.channel.clone(),
            outcome: SendOutcome::Skipped,
            detail: skip.detail.clone(),
            permalink: None,
        });
    }
    let broadcast = crate::settings::bump_broadcast(store)?;
    let mut last_post: HashMap<(String, String), DateTime<Utc>> = HashMap::new();
    for card in &plan.cards {
        let key = (
            card.target.workspace.clone(),
            card.target.channel_id.clone(),
        );
        let outcome_for_all =
            |outcome: SendOutcome, detail: &str, results: &mut Vec<ItemResult>| {
                for message in &card.messages {
                    for item in &message.items {
                        results.push(item_result(item, &card.target, outcome, detail, None));
                    }
                }
            };
        if stale.contains(&key) {
            outcome_for_all(
                SendOutcome::Skipped,
                "Changed since the preview. Preview again.",
                &mut results,
            );
            continue;
        }
        let chat = match chat_for(card.target.route_key()) {
            Ok(chat) => chat,
            Err(err) => {
                outcome_for_all(SendOutcome::Failed, &err.to_string(), &mut results);
                continue;
            }
        };
        for message in &card.messages {
            if let Some(last) = last_post.get(&key) {
                let wait = *last + CHANNEL_SPACING - pace.now();
                if wait > Duration::zero() {
                    pace.sleep(wait);
                }
            }
            let posted = post_with_retry(chat.as_ref(), &card.target, message, broadcast, pace)
                .map(|posted| with_link(posted, &card.target));
            last_post.insert(key.clone(), pace.now());
            match posted {
                // In Slack but unidentified: never `Failed`, so "Retry failed"
                // cannot post it twice. Nothing is recorded without a `ts`.
                Err(ProviderError::SentUnconfirmed(_)) => {
                    for item in &message.items {
                        results.push(item_result(
                            item,
                            &card.target,
                            SendOutcome::Sent,
                            SENT_LINK_UNKNOWN,
                            None,
                        ));
                    }
                }
                Ok(posted) => {
                    let saved = record(
                        store,
                        chat.as_ref(),
                        card,
                        message,
                        &posted,
                        batch_id,
                        pace.now(),
                    );
                    let link = posted.permalink.clone();
                    for item in &message.items {
                        results.push(match &saved {
                            Ok(()) => item_result(
                                item,
                                &card.target,
                                SendOutcome::Sent,
                                message_label(message),
                                link.clone(),
                            ),
                            Err(err) => item_result(
                                item,
                                &card.target,
                                SendOutcome::Failed,
                                &format!("Posted to Slack but could not save it: {err}"),
                                link.clone(),
                            ),
                        });
                    }
                }
                Err(err) => {
                    let detail = provider_detail(&err);
                    for item in &message.items {
                        results.push(item_result(
                            item,
                            &card.target,
                            SendOutcome::Failed,
                            &detail,
                            None,
                        ));
                    }
                }
            }
        }
    }
    let at = pace.now();
    for result in &results {
        store.insert_send_attempt(SendAttempt {
            id: 0,
            batch_id: batch_id.to_owned(),
            pr_id: Some(result.pr),
            channel_id: result.target.as_ref().map(|t| t.channel_id.clone()),
            origin: "bulk".to_owned(),
            outcome: result.outcome,
            detail: result.detail.clone(),
            at,
        })?;
    }
    Ok(results)
}

/// Detail for a message Slack accepted without a readable result.
pub const SENT_LINK_UNKNOWN: &str =
    "Sent, link unknown. Check the channel; it is not tracked and will not be retried.";

fn message_label(message: &PlannedMessage) -> &'static str {
    match message.kind {
        MessagePlanKind::Request => "Requested",
        MessagePlanKind::Bump { .. } => "Bumped in the thread",
        MessagePlanKind::Remind => "Reminded in a new message",
    }
}

fn item_result(
    item: &PlannedItem,
    target: &Target,
    outcome: SendOutcome,
    detail: &str,
    permalink: Option<String>,
) -> ItemResult {
    ItemResult {
        pr: item.pr.id,
        title: item.pr.title.clone(),
        target: Some(target.clone()),
        outcome,
        detail: detail.to_owned(),
        permalink,
    }
}

fn provider_detail(err: &ProviderError) -> String {
    match err {
        ProviderError::RateLimited { reset_at, .. } => format!(
            "Slack is rate limiting this channel until {}. Retry later.",
            reset_at.format("%H:%M:%S UTC")
        ),
        ProviderError::Unauthorized(_) => {
            "Slack sign-in expired. Reconnect the workspace.".to_owned()
        }
        other => other.to_string(),
    }
}

fn post_with_retry(
    chat: &dyn ChatProvider,
    target: &Target,
    message: &PlannedMessage,
    broadcast: bool,
    pace: &dyn Pace,
) -> Result<PostedMessage, ProviderError> {
    let thread_ts = match &message.kind {
        MessagePlanKind::Bump { root_ts } => Some(root_ts.as_str()),
        MessagePlanKind::Request | MessagePlanKind::Remind => None,
    };
    let outgoing = OutgoingMessage {
        channel_id: &target.channel_id,
        text: &message.wire_text,
        thread_ts,
        reply_broadcast: thread_ts.is_some() && broadcast,
        unfurl_app_links: thread_ts.is_none(),
    };
    match chat.post(&outgoing) {
        Err(ProviderError::RateLimited { reset_at, .. })
            if reset_at - pace.now() <= MAX_RATE_LIMIT_WAIT =>
        {
            let wait = reset_at - pace.now();
            if wait > Duration::zero() {
                pace.sleep(wait);
            }
            chat.post(&outgoing)
        }
        other => other,
    }
}

fn record(
    store: &Store,
    chat: &dyn ChatProvider,
    card: &ChannelCard,
    message: &PlannedMessage,
    posted: &PostedMessage,
    batch_id: &str,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let (thread_root_ts, kind) = match &message.kind {
        MessagePlanKind::Bump { root_ts } => (Some(root_ts.clone()), ReviewMessageKind::Bump),
        MessagePlanKind::Request => (None, ReviewMessageKind::Request),
        // A top-level remind is a new root: later bumps thread under it.
        MessagePlanKind::Remind => (None, ReviewMessageKind::Request),
    };
    let row = store.insert_review_message(ReviewMessage {
        id: 0,
        slack_account_id: chat.account_id(),
        channel_id: card.target.channel_id.clone(),
        channel_name: card.target.channel_name.clone(),
        ts: posted.ts.clone(),
        thread_root_ts,
        kind,
        origin: ReviewMessageOrigin::Posted,
        permalink: posted.permalink.clone(),
        text: message.text.clone(),
        batch_id: Some(batch_id.to_owned()),
        posted_at: now,
    })?;
    for item in &message.items {
        store.insert_review_message_pr(ReviewMessagePr {
            message_id: row.id,
            pr_id: item.pr.id,
            head_sha: Some(item.pr.head_sha.clone()),
            teams_json: serde_json::to_string(&item.teams).unwrap_or_else(|_| "[]".into()),
        })?;
        match (&message.kind, &item.active) {
            (MessagePlanKind::Bump { .. }, Some(active)) => {
                store.record_bump(active.id, now)?;
            }
            (MessagePlanKind::Request | MessagePlanKind::Remind, active) => {
                if let Some(active) = active {
                    store.close_review_request(active.id, CloseReason::Superseded, now)?;
                }
                store.insert_review_request(ReviewRequest {
                    id: ReviewRequestId(0),
                    pr: item.pr.id,
                    root: ProviderRef {
                        account: chat.account_id(),
                        id: format!("{}:{}", card.target.channel_id, posted.ts),
                    },
                    workspace: card.target.workspace.clone(),
                    team_id: card.target.team_id.clone(),
                    channel_id: card.target.channel_id.clone(),
                    channel_name: card.target.channel_name.clone(),
                    root_ts: posted.ts.clone(),
                    permalink: posted.permalink.clone(),
                    requested_at: now,
                    last_bumped_at: None,
                    bump_count: 0,
                    status: RequestStatus::Active,
                    closed_at: None,
                    source: RequestSource::App,
                })?;
            }
            (MessagePlanKind::Bump { .. }, None) => {
                return Err(CoreError::Store("bump without an active request".into()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::domain::{
        Account, AccountId, AccountKind, AuthMode, ChatKind, ChecksState, Involvement,
        MergeStateStatus, Mergeable, PrState, RepoRef, Repository, ReviewDecision,
    };
    use crate::providers::{ChatChannel, ChatUser, Page};

    /// channel, text, thread_ts, reply_broadcast
    type Post = (String, String, Option<String>, bool);

    #[derive(Default)]
    struct FakeChat {
        account: i64,
        posts: Mutex<Vec<Post>>,
        /// Channels that answer `not_in_channel`.
        failing: Vec<String>,
        /// Number of `ratelimited` answers before succeeding.
        rate_limited: Mutex<u32>,
        /// Channels where the post lands but its result is unreadable.
        unconfirmed: Vec<String>,
        /// Return only the `ts`, no link.
        ts_only: bool,
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
            if self.failing.iter().any(|c| c == message.channel_id) {
                return Err(ProviderError::Api {
                    code: "not_in_channel".into(),
                    message: "not in channel".into(),
                });
            }
            {
                let mut limited = self.rate_limited.lock().unwrap();
                if *limited > 0 {
                    *limited -= 1;
                    return Err(ProviderError::RateLimited {
                        reset_at: at(0) + Duration::seconds(5),
                        secondary: false,
                    });
                }
            }
            let mut posts = self.posts.lock().unwrap();
            posts.push((
                message.channel_id.to_owned(),
                message.text.to_owned(),
                message.thread_ts.map(str::to_owned),
                message.reply_broadcast,
            ));
            let ts = format!("1800000{:03}.000100", posts.len());
            if self.unconfirmed.iter().any(|c| c == message.channel_id) {
                return Err(ProviderError::SentUnconfirmed("no ts".into()));
            }
            if self.ts_only {
                return Ok(PostedMessage {
                    channel_id: message.channel_id.to_owned(),
                    permalink: None,
                    thread_ts: message.thread_ts.map(str::to_owned),
                    ts,
                    workspace: None,
                });
            }
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

    /// Fake clock: `sleep` advances time and is recorded.
    struct FakePace {
        now: RefCell<DateTime<Utc>>,
        slept: RefCell<Vec<Duration>>,
    }

    impl FakePace {
        fn new() -> Self {
            Self {
                now: RefCell::new(at(0)),
                slept: RefCell::new(Vec::new()),
            }
        }
    }

    impl Pace for FakePace {
        fn now(&self) -> DateTime<Utc> {
            *self.now.borrow()
        }
        fn sleep(&self, duration: Duration) {
            *self.now.borrow_mut() += duration;
            self.slept.borrow_mut().push(duration);
        }
    }

    fn at(hours: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-26T04:40:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::hours(hours)
    }

    struct World {
        _dir: tempfile::TempDir,
        store: Store,
        slack: AccountId,
        repo: i64,
        github: AccountId,
    }

    fn world() -> World {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let account = |kind, login: &str, ext: &str, ws: Option<&str>, mode| Account {
            id: AccountId(0),
            kind,
            display_name: "Acme".into(),
            host: "acme.example".into(),
            external_id: ext.into(),
            slack_workspace: ws.map(str::to_owned),
            login: login.into(),
            auth_mode: mode,
            keychain_key: None,
            created_at: at(0),
        };
        let slack = store
            .insert_account(account(
                AccountKind::Slack,
                "U0ALICE",
                "T0ACME:U0ALICE",
                Some("acme"),
                AuthMode::SlackMcpOauth,
            ))
            .unwrap()
            .id;
        let github = store
            .insert_account(account(
                AccountKind::GitHub,
                "alice",
                "U1",
                None,
                AuthMode::GhCli,
            ))
            .unwrap()
            .id;
        let repo = store
            .insert_repository(Repository {
                id: 0,
                account_id: github,
                owner: "acme".into(),
                name: "web".into(),
                node_id: None,
                default_branch: Some("main".into()),
                archived: false,
            })
            .unwrap()
            .id;
        World {
            _dir: dir,
            store,
            slack,
            repo,
            github,
        }
    }

    fn pr(world: &World, number: u64, title: &str) -> PullRequest {
        world
            .store
            .upsert_pull_request(
                world.repo,
                PullRequest {
                    id: PrId(0),
                    remote: ProviderRef {
                        account: world.github,
                        id: format!("PR_{number}"),
                    },
                    repo: RepoRef {
                        host: "github.com".into(),
                        owner: "acme".into(),
                        name: "web".into(),
                    },
                    number,
                    title: title.into(),
                    url: format!("https://github.com/acme/web/pull/{number}"),
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
            .unwrap()
    }

    fn target(workspace: &str, channel: &str) -> Target {
        Target {
            team_id: None,
            workspace: workspace.into(),
            channel_id: channel.into(),
            channel_name: channel.to_ascii_lowercase(),
        }
    }

    fn requested(
        world: &World,
        pr: &PullRequest,
        t: &Target,
        root: &str,
        hours_ago: i64,
    ) -> ReviewRequest {
        world
            .store
            .insert_review_request(ReviewRequest {
                team_id: None,
                id: ReviewRequestId(0),
                pr: pr.id,
                root: ProviderRef {
                    account: world.slack,
                    id: format!("{}:{root}", t.channel_id),
                },
                workspace: t.workspace.clone(),
                channel_id: t.channel_id.clone(),
                channel_name: t.channel_name.clone(),
                root_ts: root.into(),
                permalink: None,
                requested_at: at(-hours_ago),
                last_bumped_at: None,
                bump_count: 0,
                status: RequestStatus::Active,
                closed_at: None,
                source: RequestSource::App,
            })
            .unwrap()
    }

    fn input(pr: &PullRequest, targets: &[&Target]) -> BatchInput {
        BatchInput {
            pr: pr.clone(),
            routes: targets
                .iter()
                .map(|t| Route {
                    target: (*t).clone(),
                    teams: vec!["acme/web".into()],
                })
                .collect(),
        }
    }

    fn options() -> BatchOptions {
        BatchOptions {
            requester: Some("Alice".into()),
            min_bump_interval_hours: 24,
            max_threads: 3,
            single_top_level: BTreeSet::new(),
        }
    }

    fn kinds(card: &ChannelCard) -> Vec<(String, usize)> {
        card.messages
            .iter()
            .map(|m| {
                let kind = match &m.kind {
                    MessagePlanKind::Request => "request".to_owned(),
                    MessagePlanKind::Bump { root_ts } => format!("bump:{root_ts}"),
                    MessagePlanKind::Remind => "remind".to_owned(),
                };
                (kind, m.items.len())
            })
            .collect()
    }

    #[test]
    fn requested_and_new_prs_in_one_channel() {
        let w = world();
        let team = target("acme", "C0TEAM");
        let a = pr(&w, 1, "Fix login");
        let b = pr(&w, 2, "Add search");
        let c = pr(&w, 3, "Bump deps");
        requested(&w, &a, &team, "1700000001.000100", 48);
        let plan = plan_batch(
            &w.store,
            &[
                input(&a, &[&team]),
                input(&b, &[&team]),
                input(&c, &[&team]),
            ],
            vec![],
            &options(),
            at(0),
        )
        .unwrap();
        assert_eq!(plan.cards.len(), 1);
        let card = &plan.cards[0];
        assert_eq!(
            kinds(card),
            [("bump:1700000001.000100".into(), 1), ("request".into(), 2)]
        );
        assert!(card.can_switch);
        assert!(card.warnings.is_empty());
        assert!(card.messages[1].text.contains("Add search"));
        assert!(card.messages[1].text.contains("Bump deps"));
        assert!(
            card.messages[1].text.contains("Alice"),
            "request carries the requester"
        );
        assert!(
            !card.messages[0].text.contains("Alice"),
            "remind has no requester"
        );
    }

    #[test]
    fn one_root_gives_one_bump_for_all_its_prs() {
        let w = world();
        let team = target("acme", "C0TEAM");
        let a = pr(&w, 1, "Fix login");
        let b = pr(&w, 2, "Add search");
        requested(&w, &a, &team, "1700000001.000100", 48);
        requested(&w, &b, &team, "1700000001.000100", 48);
        let plan = plan_batch(
            &w.store,
            &[input(&a, &[&team]), input(&b, &[&team])],
            vec![],
            &options(),
            at(0),
        )
        .unwrap();
        assert_eq!(
            kinds(&plan.cards[0]),
            [("bump:1700000001.000100".into(), 2)]
        );
    }

    #[test]
    fn up_to_three_roots_bump_each_thread_and_more_becomes_one_remind() {
        let w = world();
        let team = target("acme", "C0TEAM");
        let prs: Vec<PullRequest> = (1..=4).map(|n| pr(&w, n, &format!("Change {n}"))).collect();
        for (i, p) in prs.iter().enumerate() {
            requested(&w, p, &team, &format!("170000000{i}.000100"), 48);
        }
        let three: Vec<BatchInput> = prs[..3].iter().map(|p| input(p, &[&team])).collect();
        let plan = plan_batch(&w.store, &three, vec![], &options(), at(0)).unwrap();
        assert_eq!(plan.cards[0].messages.len(), 3);
        assert!(
            plan.cards[0]
                .messages
                .iter()
                .all(|m| matches!(m.kind, MessagePlanKind::Bump { .. }))
        );

        let four: Vec<BatchInput> = prs.iter().map(|p| input(p, &[&team])).collect();
        let plan = plan_batch(&w.store, &four, vec![], &options(), at(0)).unwrap();
        assert_eq!(kinds(&plan.cards[0]), [("remind".into(), 4)]);

        // Sending the remind supersedes the four old roots.
        let chat: Arc<dyn ChatProvider> = Arc::new(FakeChat {
            account: w.slack.0,
            ..FakeChat::default()
        });
        let pace = FakePace::new();
        let results = send_batch(
            &w.store,
            &mut |_| Ok(chat.clone()),
            &plan,
            &BTreeSet::new(),
            "b1",
            &pace,
        )
        .unwrap();
        assert!(results.iter().all(|r| r.outcome == SendOutcome::Sent));
        for p in &prs {
            let rows = w.store.list_review_requests(p.id).unwrap();
            assert_eq!(rows.len(), 2);
            let active: Vec<_> = rows
                .iter()
                .filter(|r| r.status == RequestStatus::Active)
                .collect();
            assert_eq!(active.len(), 1);
            assert_eq!(active[0].root_ts, "1800000001.000100");
            assert!(
                rows.iter()
                    .any(|r| r.status == RequestStatus::Closed(CloseReason::Superseded))
            );
        }
    }

    #[test]
    fn mixed_workspaces_and_channels_get_one_card_each() {
        let w = world();
        let web = target("acme", "C0WEB");
        let api = target("acme", "C0API");
        let other = target("acme-labs", "C0WEB");
        let a = pr(&w, 1, "Fix login");
        let b = pr(&w, 2, "Add search");
        let plan = plan_batch(
            &w.store,
            &[input(&a, &[&web, &api]), input(&b, &[&web, &other])],
            vec![],
            &options(),
            at(0),
        )
        .unwrap();
        let keys: Vec<(String, String, usize)> = plan
            .cards
            .iter()
            .map(|c| {
                (
                    c.target.workspace.clone(),
                    c.target.channel_id.clone(),
                    c.messages[0].items.len(),
                )
            })
            .collect();
        assert_eq!(
            keys,
            [
                ("acme".into(), "C0WEB".into(), 2),
                ("acme".into(), "C0API".into(), 1),
                ("acme-labs".into(), "C0WEB".into(), 1),
            ]
        );
        assert!(plan.cards.iter().all(|c| !c.can_switch));
    }

    #[test]
    fn a_closed_request_plans_a_new_request() {
        let w = world();
        let team = target("acme", "C0TEAM");
        let a = pr(&w, 1, "Fix login");
        let old = requested(&w, &a, &team, "1700000001.000100", 48);
        w.store
            .close_review_request(old.id, CloseReason::Dismissed, at(-1))
            .unwrap();
        let plan = plan_batch(&w.store, &[input(&a, &[&team])], vec![], &options(), at(0)).unwrap();
        assert_eq!(kinds(&plan.cards[0]), [("request".into(), 1)]);
    }

    #[test]
    fn single_top_level_switch_supersedes_and_recent_bumps_warn() {
        let w = world();
        let team = target("acme", "C0TEAM");
        let a = pr(&w, 1, "Fix login");
        let b = pr(&w, 2, "Add search");
        requested(&w, &a, &team, "1700000001.000100", 2);
        let inputs = [input(&a, &[&team]), input(&b, &[&team])];
        let plan = plan_batch(&w.store, &inputs, vec![], &options(), at(0)).unwrap();
        assert_eq!(plan.cards[0].warnings.len(), 1, "bump under 24 h warns");

        let mut opts = options();
        opts.single_top_level
            .insert(("acme".into(), "C0TEAM".into()));
        let plan = plan_batch(&w.store, &inputs, vec![], &opts, at(0)).unwrap();
        assert_eq!(kinds(&plan.cards[0]), [("request".into(), 2)]);
        assert!(plan.cards[0].single_top_level);
    }

    #[test]
    fn partial_failure_keeps_what_was_posted_and_records_every_pr() {
        let w = world();
        let ok = target("acme", "C0OK");
        let bad = target("acme", "C0BAD");
        let a = pr(&w, 1, "Fix login");
        let b = pr(&w, 2, "Add search");
        let skip = Skip {
            pr: PrId(99),
            title: "Not mine".into(),
            channel: None,
            detail: "Not authored by you".into(),
        };
        let plan = plan_batch(
            &w.store,
            &[input(&a, &[&ok, &bad]), input(&b, &[&ok])],
            vec![skip],
            &options(),
            at(0),
        )
        .unwrap();
        let chat: Arc<dyn ChatProvider> = Arc::new(FakeChat {
            account: w.slack.0,
            failing: vec!["C0BAD".into()],
            ..FakeChat::default()
        });
        let pace = FakePace::new();
        let results = send_batch(
            &w.store,
            &mut |_| Ok(chat.clone()),
            &plan,
            &BTreeSet::new(),
            "b2",
            &pace,
        )
        .unwrap();
        let summary: Vec<(i64, Option<String>, SendOutcome)> = results
            .iter()
            .map(|r| {
                (
                    r.pr.0,
                    r.target.as_ref().map(|t| t.channel_id.clone()),
                    r.outcome,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (99, None, SendOutcome::Skipped),
                (a.id.0, Some("C0OK".into()), SendOutcome::Sent),
                (b.id.0, Some("C0OK".into()), SendOutcome::Sent),
                (a.id.0, Some("C0BAD".into()), SendOutcome::Failed),
            ]
        );
        assert!(results[3].detail.contains("not_in_channel"));
        assert_eq!(w.store.list_review_requests(a.id).unwrap().len(), 1);
        assert_eq!(w.store.list_review_requests(b.id).unwrap().len(), 1);
    }

    #[test]
    fn unreadable_send_results_are_sent_not_failed_and_ts_gets_a_link() {
        let w = world();
        let dm = target("acme", "D0LOST");
        let ok = target("acme", "D0OK");
        let a = pr(&w, 1, "Fix login");
        let plan = plan_batch(
            &w.store,
            &[input(&a, &[&dm, &ok])],
            vec![],
            &options(),
            at(0),
        )
        .unwrap();
        let chat: Arc<dyn ChatProvider> = Arc::new(FakeChat {
            account: w.slack.0,
            unconfirmed: vec!["D0LOST".into()],
            ts_only: true,
            ..FakeChat::default()
        });
        let results = send_batch(
            &w.store,
            &mut |_| Ok(chat.clone()),
            &plan,
            &BTreeSet::new(),
            "b3",
            &FakePace::new(),
        )
        .unwrap();
        let lost = results
            .iter()
            .find(|r| r.target.as_ref().unwrap().channel_id == "D0LOST")
            .unwrap();
        assert_eq!(lost.outcome, SendOutcome::Sent);
        assert_eq!(lost.detail, SENT_LINK_UNKNOWN);
        assert_eq!(lost.permalink, None);
        let sent = results
            .iter()
            .find(|r| r.target.as_ref().unwrap().channel_id == "D0OK")
            .unwrap();
        assert_eq!(
            sent.permalink.as_deref(),
            Some("https://acme.slack.com/archives/D0OK/p1800000002000100")
        );
        let requests = w.store.list_review_requests(a.id).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].channel_id, "D0OK");
    }

    #[test]
    fn posts_in_one_channel_are_spaced_and_rate_limits_wait_once() {
        let w = world();
        let team = target("acme", "C0TEAM");
        let a = pr(&w, 1, "Fix login");
        let b = pr(&w, 2, "Add search");
        requested(&w, &a, &team, "1700000001.000100", 48);
        let plan = plan_batch(
            &w.store,
            &[input(&a, &[&team]), input(&b, &[&team])],
            vec![],
            &options(),
            at(0),
        )
        .unwrap();
        let fake = Arc::new(FakeChat {
            account: w.slack.0,
            rate_limited: Mutex::new(1),
            ..FakeChat::default()
        });
        let chat: Arc<dyn ChatProvider> = fake.clone();
        let pace = FakePace::new();
        let results = send_batch(
            &w.store,
            &mut |_| Ok(chat.clone()),
            &plan,
            &BTreeSet::new(),
            "b3",
            &pace,
        )
        .unwrap();
        assert!(results.iter().all(|r| r.outcome == SendOutcome::Sent));
        assert_eq!(
            *pace.slept.borrow(),
            [Duration::seconds(5), CHANNEL_SPACING],
            "waits for the reset, then spaces the second post"
        );
        let posts = fake.posts.lock().unwrap();
        assert_eq!(posts[0].2.as_deref(), Some("1700000001.000100"));
        assert!(posts[0].3, "bump is broadcast");
        assert_eq!(posts[1].2, None);
    }

    #[test]
    fn bulk_bump_is_thread_only_when_broadcast_is_off() {
        let w = world();
        crate::settings::set_bump_broadcast(&w.store, false).unwrap();
        let team = target("acme", "C0TEAM");
        let a = pr(&w, 1, "Fix login");
        requested(&w, &a, &team, "1700000001.000100", 48);
        let plan = plan_batch(&w.store, &[input(&a, &[&team])], vec![], &options(), at(0)).unwrap();
        let fake = Arc::new(FakeChat {
            account: w.slack.0,
            ..FakeChat::default()
        });
        let chat: Arc<dyn ChatProvider> = fake.clone();
        send_batch(
            &w.store,
            &mut |_| Ok(chat.clone()),
            &plan,
            &BTreeSet::new(),
            "b4",
            &FakePace::new(),
        )
        .unwrap();
        let posts = fake.posts.lock().unwrap();
        assert_eq!(posts[0].2.as_deref(), Some("1700000001.000100"));
        assert!(!posts[0].3, "thread reply only");
    }

    #[test]
    fn stale_cards_are_skipped_without_posting() {
        let w = world();
        let team = target("acme", "C0TEAM");
        let a = pr(&w, 1, "Fix login");
        let plan = plan_batch(&w.store, &[input(&a, &[&team])], vec![], &options(), at(0)).unwrap();
        let fake = Arc::new(FakeChat {
            account: w.slack.0,
            ..FakeChat::default()
        });
        let chat: Arc<dyn ChatProvider> = fake.clone();
        let stale = BTreeSet::from([("acme".to_owned(), "C0TEAM".to_owned())]);
        let results = send_batch(
            &w.store,
            &mut |_| Ok(chat.clone()),
            &plan,
            &stale,
            "b4",
            &FakePace::new(),
        )
        .unwrap();
        assert_eq!(results[0].outcome, SendOutcome::Skipped);
        assert!(fake.posts.lock().unwrap().is_empty());
        assert!(plan.cards[0].fingerprint().contains("request"));
    }
}
