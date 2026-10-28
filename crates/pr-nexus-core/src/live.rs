//! Live updates (§6): the notifications inbox says which pull requests moved,
//! and each one is re-read with one targeted query, between incremental syncs.
//!
//! Everything here is a pure step driven by [`LivePoller::tick`]: the app
//! calls it from a background thread with the current time, and it tells the
//! caller when to call again. Notifications are only read, never marked read.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

use crate::domain::{AccountId, ChecksState, Involvement, PrState, ProviderRef, PullRequest};
use crate::error::{CoreError, ProviderError};
use crate::providers::{NotificationsPage, NotificationsRequest, VcsProvider};
use crate::settings::{self, LiveMark};
use crate::store::Store;
use crate::sync::{self, TrackedScope, Writer};

/// GitHub's `X-Poll-Interval` floor; never poll faster.
pub const MIN_POLL_SECS: i64 = 60;
/// Pending checks are re-read this often while the window has focus …
pub const CHECKS_FOCUSED_SECS: i64 = 90;
/// … and this often in the background.
pub const CHECKS_UNFOCUSED_SECS: i64 = 300;
/// Coming back after this long away runs an incremental sync.
pub const FOCUS_AWAY_SECS: i64 = 60;
/// Pages followed in one poll while every thread on them is new.
pub const MAX_PAGES: u32 = 3;
/// Longest wait after repeated failures.
pub const MAX_BACKOFF_SECS: i64 = 900;
/// Statuses read per request, as in sync.
const STATUS_BATCH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveState {
    /// The setting is off.
    Off,
    /// On, nothing answered yet.
    Starting,
    /// Polling the inbox.
    Active,
    /// The token cannot read notifications; pending checks still refresh.
    Unavailable(String),
}

/// What the sync details panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveStatus {
    pub state: LiveState,
    /// Last answered inbox poll (304 included).
    pub checked_at: Option<DateTime<Utc>>,
    /// Last failure, cleared by the next success.
    pub error: Option<String>,
}

/// One pull request named by a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub owner: String,
    pub name: String,
    pub number: u64,
    /// The reasons say the viewer is asked to review or wrote it.
    pub involvement: Involvement,
}

/// What one tick did, for tests and logs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveTick {
    pub polled: bool,
    pub fetched: usize,
    pub checks_read: usize,
    /// Rows whose stored content changed: the caller bumps the list revision.
    pub changed: usize,
}

/// `…/repos/{owner}/{repo}/pulls/{number}` → its parts.
pub fn parse_subject_url(url: &str) -> Option<(String, String, u64)> {
    let rest = &url[url.find("/repos/")? + "/repos/".len()..];
    let mut parts = rest.split('/');
    let (owner, name, kind, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if kind != "pulls" || owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    Some((owner.to_owned(), name.to_owned(), number.parse().ok()?))
}

fn reason_involvement(reason: &str) -> Involvement {
    match reason {
        "review_requested" => Involvement::REVIEW_REQUESTED,
        "author" => Involvement::AUTHOR,
        _ => Involvement::empty(),
    }
}

/// How often pending checks are re-read.
pub fn checks_interval(focused: bool) -> Duration {
    Duration::seconds(if focused {
        CHECKS_FOCUSED_SECS
    } else {
        CHECKS_UNFOCUSED_SECS
    })
}

/// Regaining focus after more than [`FOCUS_AWAY_SECS`] away asks for an
/// incremental sync; a quick alt-tab does not.
pub fn should_sync_on_focus(blurred_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    blurred_at.is_some_and(|at| now - at > Duration::seconds(FOCUS_AWAY_SECS))
}

fn backoff(failures: u32) -> Duration {
    let secs = MIN_POLL_SECS.saturating_mul(1_i64 << failures.saturating_sub(1).min(10));
    Duration::seconds(secs.min(MAX_BACKOFF_SECS))
}

/// Inputs to one tick.
pub struct LiveInput<'a> {
    pub store: &'a Store,
    pub provider: &'a dyn VcsProvider,
    pub account_id: AccountId,
    pub enabled: bool,
    pub focused: bool,
    pub now: DateTime<Utc>,
}

#[derive(Debug)]
pub struct LivePoller {
    state: LiveState,
    checked_at: Option<DateTime<Utc>>,
    error: Option<String>,
    next_poll: Option<DateTime<Utc>>,
    checks_at: Option<DateTime<Utc>>,
    failures: u32,
    /// Targets not yet refreshed, keyed so one pull request is read once.
    pending: BTreeMap<(String, String, u64), Target>,
    /// Saved once every pending target is refreshed.
    pending_mark: Option<LiveMark>,
}

impl Default for LivePoller {
    fn default() -> Self {
        Self::new()
    }
}

impl LivePoller {
    pub fn new() -> Self {
        Self {
            state: LiveState::Starting,
            checked_at: None,
            error: None,
            next_poll: None,
            checks_at: None,
            failures: 0,
            pending: BTreeMap::new(),
            pending_mark: None,
        }
    }

    pub fn status(&self) -> LiveStatus {
        LiveStatus {
            state: self.state.clone(),
            checked_at: self.checked_at,
            error: self.error.clone(),
        }
    }

    /// When the next tick has work, given focus.
    pub fn next_due(&self, focused: bool, now: DateTime<Utc>) -> DateTime<Utc> {
        if self.state == LiveState::Off {
            return now + Duration::seconds(MIN_POLL_SECS);
        }
        let checks = self
            .checks_at
            .map_or(now, |at| at + checks_interval(focused));
        match (&self.state, self.next_poll) {
            (LiveState::Unavailable(_), _) => checks,
            (_, Some(poll)) => poll.min(checks),
            (_, None) => now,
        }
    }

    /// The setting is off: forget pending work; the next enabled tick polls.
    pub fn disable(&mut self) {
        self.state = LiveState::Off;
        self.pending.clear();
        self.pending_mark = None;
        self.next_poll = None;
        self.checks_at = None;
    }

    /// Forget a token-type verdict, e.g. after signing in again.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn tick(&mut self, input: &LiveInput<'_>) -> Result<LiveTick, CoreError> {
        let mut out = LiveTick::default();
        if !input.enabled {
            self.disable();
            return Ok(out);
        }
        if self.state == LiveState::Off {
            self.state = LiveState::Starting;
        }
        let Some(account) = input.store.get_account(input.account_id)? else {
            return Ok(out);
        };
        let scope = TrackedScope {
            orgs: settings::tracked_orgs(input.store)?,
            include_personal: settings::include_personal_repos(input.store)?,
            viewer_login: account.login,
        };
        if sync::scope_is_empty(&scope) {
            return Ok(out);
        }
        if let Some(budget) = settings::rate_budget(input.store)?
            && sync::scheduled_poll_paused(&budget, input.now)
        {
            return Ok(out);
        }
        let now = input.now;
        if !matches!(self.state, LiveState::Unavailable(_))
            && self.next_poll.is_none_or(|at| now >= at)
        {
            out.polled = true;
            self.poll(input, now);
        }
        if !self.pending.is_empty() {
            self.refresh_pending(input, &scope, &mut out)?;
        }
        // The first tick starts the clock: the start-up sync just read them.
        let checks_at = *self.checks_at.get_or_insert(now);
        if now - checks_at >= checks_interval(input.focused) {
            self.checks_at = Some(now);
            self.refresh_checks(input, &scope, &mut out)?;
        }
        Ok(out)
    }

    fn fail(&mut self, err: &ProviderError, now: DateTime<Utc>) {
        self.failures += 1;
        let mut next = now + backoff(self.failures);
        if let ProviderError::RateLimited { reset_at, .. } = err {
            next = next.max(*reset_at);
        }
        self.next_poll = Some(next);
        self.error = Some(err.to_string());
    }

    fn poll(&mut self, input: &LiveInput<'_>, now: DateTime<Utc>) {
        let stored = settings::live_mark(input.store).ok().flatten();
        let watermark = self
            .pending_mark
            .as_ref()
            .or(stored.as_ref())
            .map_or(now, |mark| mark.watermark);
        // A pending mark means the last poll's targets are not all read yet;
        // ask again without If-Modified-Since so they are not lost.
        let since = if self.pending_mark.is_some() {
            None
        } else {
            stored.as_ref().and_then(|mark| mark.last_modified.clone())
        };
        let mut last_modified = since.clone();
        let mut interval = None;
        let mut newest = watermark;
        for page in 1..=MAX_PAGES {
            let request = NotificationsRequest {
                if_modified_since: since.clone(),
                page,
            };
            match input.provider.notifications(&request) {
                Err(ProviderError::Unsupported(reason)) => {
                    self.state = LiveState::Unavailable(reason);
                    self.next_poll = None;
                    self.error = None;
                    return;
                }
                Err(err) => {
                    self.fail(&err, now);
                    return;
                }
                Ok(NotificationsPage::NotModified { poll_interval }) => {
                    interval = interval.or(poll_interval);
                    break;
                }
                Ok(NotificationsPage::Changed {
                    last_modified: modified,
                    poll_interval,
                    items,
                    has_next,
                }) => {
                    if page == 1 {
                        last_modified = modified.or(last_modified);
                        interval = poll_interval;
                    }
                    let mut all_new = !items.is_empty();
                    for item in items {
                        if item.updated_at <= watermark {
                            all_new = false;
                            continue;
                        }
                        newest = newest.max(item.updated_at);
                        if item.subject_type != "PullRequest" {
                            continue;
                        }
                        let Some((owner, name, number)) =
                            item.subject_url.as_deref().and_then(parse_subject_url)
                        else {
                            continue;
                        };
                        let key = (
                            owner.to_ascii_lowercase(),
                            name.to_ascii_lowercase(),
                            number,
                        );
                        let involvement = reason_involvement(&item.reason);
                        self.pending
                            .entry(key)
                            .and_modify(|target| target.involvement |= involvement)
                            .or_insert(Target {
                                owner,
                                name,
                                number,
                                involvement,
                            });
                    }
                    if !(has_next && all_new) {
                        break;
                    }
                }
            }
        }
        self.state = LiveState::Active;
        self.failures = 0;
        self.error = None;
        self.checked_at = Some(now);
        let wait = interval
            .and_then(|secs| i64::try_from(secs).ok())
            .unwrap_or(MIN_POLL_SECS)
            .max(MIN_POLL_SECS);
        self.next_poll = Some(now + Duration::seconds(wait));
        let mark = LiveMark {
            last_modified,
            watermark: newest,
        };
        if self.pending.is_empty() {
            self.pending_mark = None;
            if stored.as_ref() != Some(&mark) {
                let _ = settings::set_live_mark(input.store, &mark);
            }
        } else {
            self.pending_mark = Some(mark);
        }
    }

    /// One targeted read per pending pull request, one at a time. A failure
    /// keeps the rest for the next tick; the mark is saved only when all
    /// are done, so a crash re-reads rather than skips.
    fn refresh_pending(
        &mut self,
        input: &LiveInput<'_>,
        scope: &TrackedScope,
        out: &mut LiveTick,
    ) -> Result<(), CoreError> {
        let rows = input
            .store
            .list_pull_requests_for_account(input.account_id)?;
        let keys: Vec<_> = self.pending.keys().cloned().collect();
        for key in keys {
            let target = self.pending[&key].clone();
            match refresh_target(input, scope, &rows, &target) {
                Ok((fetched, changed)) => {
                    out.fetched += usize::from(fetched);
                    out.changed += usize::from(changed);
                    self.pending.remove(&key);
                }
                Err(CoreError::Provider(err)) => {
                    self.fail(&err, input.now);
                    return Ok(());
                }
                Err(err) => return Err(err),
            }
        }
        if let Some(mark) = self.pending_mark.take() {
            settings::set_live_mark(input.store, &mark)?;
        }
        Ok(())
    }

    /// Open pull requests whose checks are still running: re-read their
    /// rollups in batches; any that finished or moved get a targeted read.
    fn refresh_checks(
        &mut self,
        input: &LiveInput<'_>,
        scope: &TrackedScope,
        out: &mut LiveTick,
    ) -> Result<(), CoreError> {
        let rows = input
            .store
            .list_pull_requests_for_account(input.account_id)?;
        let waiting: Vec<&PullRequest> = rows
            .iter()
            .filter(|pr| {
                !pr.hidden
                    && pr.state == PrState::Open
                    && matches!(pr.checks, ChecksState::Pending | ChecksState::Expected)
                    && sync::in_scope(&pr.repo.owner, scope)
            })
            .collect();
        if waiting.is_empty() {
            return Ok(());
        }
        let mut writer = Writer::new(input.store, input.account_id, input.now, &rows)?;
        for chunk in waiting.chunks(STATUS_BATCH) {
            let ids: Vec<ProviderRef> = chunk.iter().map(|pr| pr.remote.clone()).collect();
            let statuses = match input.provider.pull_request_status(&ids) {
                Ok(statuses) => statuses,
                Err(err) => {
                    self.error = Some(err.to_string());
                    break;
                }
            };
            out.checks_read += chunk.len();
            for status in statuses {
                let Some(pr) = chunk.iter().find(|pr| pr.remote.id == status.remote_id) else {
                    continue;
                };
                let finished = status.state.is_some_and(|state| state != PrState::Open);
                let touched = status.updated_at.is_some_and(|at| at > pr.updated_at);
                if finished || touched {
                    let key = (
                        pr.repo.owner.to_ascii_lowercase(),
                        pr.repo.name.to_ascii_lowercase(),
                        pr.number,
                    );
                    self.pending.entry(key).or_insert(Target {
                        owner: pr.repo.owner.clone(),
                        name: pr.repo.name.clone(),
                        number: pr.number,
                        involvement: Involvement::empty(),
                    });
                } else {
                    let mut row = (*pr).clone();
                    status.apply_to(&mut row);
                    let item = writer.synced_from(row);
                    out.changed += usize::from(writer.write(&item)?);
                }
            }
        }
        if !self.pending.is_empty() {
            self.refresh_pending(input, scope, out)?;
        }
        Ok(())
    }
}

/// Re-read one pull request and store it if it changed. A row not stored yet
/// is added only when it is open, in scope, and the viewer is involved.
/// Returns (fetched, changed).
fn refresh_target(
    input: &LiveInput<'_>,
    scope: &TrackedScope,
    rows: &[PullRequest],
    target: &Target,
) -> Result<(bool, bool), CoreError> {
    let existing = rows.iter().find(|pr| {
        pr.number == target.number
            && pr.repo.owner.eq_ignore_ascii_case(&target.owner)
            && pr.repo.name.eq_ignore_ascii_case(&target.name)
    });
    if existing.is_none() && !sync::in_scope(&target.owner, scope) {
        return Ok((false, false));
    }
    let Some(mut item) =
        input
            .provider
            .pull_request_by_number(&target.owner, &target.name, target.number)?
    else {
        return Ok((true, false));
    };
    let pr = &mut item.pull_request;
    let mut involvement =
        existing.map_or(Involvement::empty(), |row| row.involvement) | target.involvement;
    if pr.author_login.eq_ignore_ascii_case(&scope.viewer_login) {
        involvement |= Involvement::AUTHOR;
    }
    pr.involvement = involvement;
    if existing.is_none()
        && (pr.state != PrState::Open
            || involvement.is_empty()
            || !sync::in_scope(&pr.repo.owner, scope))
    {
        return Ok((true, false));
    }
    let existing: Vec<PullRequest> = existing.cloned().into_iter().collect();
    let mut writer = Writer::new(input.store, input.account_id, input.now, &existing)?;
    let changed = writer.write(&item)?;
    let keep_days = settings::keep_closed_days(input.store)?;
    if sync::should_hide_closed(
        item.pull_request.state,
        item.pull_request.updated_at,
        input.now,
        keep_days,
    ) {
        writer.set_hidden(&item.pull_request, true)?;
    }
    Ok((true, changed))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use chrono::TimeZone;

    use super::*;
    use crate::domain::{
        Account, AccountKind, AuthMode, MergeStateStatus, Mergeable, PrId, RepoRef,
    };
    use crate::providers::{
        Notification, OwnershipInputs, OwnershipProbe, PrQuery, PrStatus, RateBudget,
        SyncedPullRequest, TeamMembers, VcsUser,
    };

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn pr(owner: &str, number: u64, author: &str, checks: ChecksState) -> SyncedPullRequest {
        SyncedPullRequest {
            pull_request: PullRequest {
                id: PrId(0),
                remote: ProviderRef {
                    account: AccountId(0),
                    id: format!("PR_{owner}_{number}"),
                },
                repo: RepoRef {
                    host: "github.com".into(),
                    owner: owner.into(),
                    name: "widgets".into(),
                },
                number,
                title: format!("PR {number}"),
                url: format!("https://github.com/{owner}/widgets/pull/{number}"),
                author_login: author.into(),
                body: String::new(),
                state: PrState::Open,
                is_draft: false,
                head_ref: "feature".into(),
                base_ref: "main".into(),
                head_sha: "abc".into(),
                labels: Vec::new(),
                review_decision: None,
                checks,
                mergeable: Mergeable::Mergeable,
                merge_state: MergeStateStatus::Clean,
                requested_reviewers: Vec::new(),
                unresolved_threads: 0,
                total_threads: 0,
                involvement: Involvement::empty(),
                created_at: ts(-3600),
                updated_at: ts(-600),
                last_activity_at: ts(-600),
                synced_at: ts(-600),
                hidden: false,
            },
            repo_node_id: Some("R_1".into()),
            default_branch: Some("main".into()),
            repo_archived: false,
        }
    }

    fn note(id: &str, kind: &str, url: &str, reason: &str, at: DateTime<Utc>) -> Notification {
        Notification {
            id: id.into(),
            reason: reason.into(),
            updated_at: at,
            subject_type: kind.into(),
            subject_url: Some(url.into()),
        }
    }

    fn pull_url(owner: &str, number: u64) -> String {
        format!("https://api.github.com/repos/{owner}/widgets/pulls/{number}")
    }

    fn changed(last_modified: &str, items: Vec<Notification>) -> NotificationsPage {
        NotificationsPage::Changed {
            last_modified: Some(last_modified.into()),
            poll_interval: Some(60),
            items,
            has_next: false,
        }
    }

    #[derive(Default)]
    struct Fake {
        inbox: Mutex<VecDeque<Result<NotificationsPage, ProviderError>>>,
        asked: Mutex<Vec<NotificationsRequest>>,
        pulls: Vec<SyncedPullRequest>,
        fetched: Mutex<Vec<(String, String, u64)>>,
        statuses: Vec<PrStatus>,
        status_asked: Mutex<Vec<usize>>,
    }

    impl Fake {
        fn answer(&self, page: Result<NotificationsPage, ProviderError>) {
            self.inbox.lock().unwrap().push_back(page);
        }
        fn polls(&self) -> usize {
            self.asked.lock().unwrap().len()
        }
        fn fetches(&self) -> Vec<(String, String, u64)> {
            self.fetched.lock().unwrap().clone()
        }
    }

    impl VcsProvider for Fake {
        fn kind(&self) -> crate::domain::VcsKind {
            crate::domain::VcsKind::GitHub
        }
        fn viewer(&self) -> Result<VcsUser, ProviderError> {
            unreachable!("live updates never read the viewer")
        }
        fn search_pull_requests(
            &self,
            _: &PrQuery,
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            unreachable!("live updates never search")
        }
        fn search_pull_requests_paged(
            &self,
            _: &PrQuery,
            _: &mut dyn FnMut(&[SyncedPullRequest]),
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            unreachable!("live updates never search")
        }
        fn refresh_pull_requests(
            &self,
            _: &[ProviderRef],
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            unreachable!("live updates read one pull request at a time")
        }
        fn pull_request_status(&self, ids: &[ProviderRef]) -> Result<Vec<PrStatus>, ProviderError> {
            self.status_asked.lock().unwrap().push(ids.len());
            Ok(self
                .statuses
                .iter()
                .filter(|status| ids.iter().any(|id| id.id == status.remote_id))
                .cloned()
                .collect())
        }
        fn ownership_probe(&self, _: &PullRequest) -> Result<OwnershipProbe, ProviderError> {
            unreachable!()
        }
        fn ownership_inputs(
            &self,
            _: &PullRequest,
            _: &OwnershipProbe,
        ) -> Result<OwnershipInputs, ProviderError> {
            unreachable!()
        }
        fn team_members(&self, _: &crate::domain::TeamRef) -> Result<TeamMembers, ProviderError> {
            unreachable!()
        }
        fn rate_limit(&self) -> Result<RateBudget, ProviderError> {
            unreachable!()
        }
        fn request_team_review(
            &self,
            _: &PullRequest,
            _: &[crate::domain::TeamRef],
        ) -> Result<(), ProviderError> {
            unreachable!()
        }
        fn notifications(
            &self,
            request: &NotificationsRequest,
        ) -> Result<NotificationsPage, ProviderError> {
            self.asked.lock().unwrap().push(request.clone());
            self.inbox
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(NotificationsPage::NotModified {
                    poll_interval: Some(60),
                }))
        }
        fn pull_request_by_number(
            &self,
            owner: &str,
            name: &str,
            number: u64,
        ) -> Result<Option<SyncedPullRequest>, ProviderError> {
            self.fetched
                .lock()
                .unwrap()
                .push((owner.into(), name.into(), number));
            Ok(self
                .pulls
                .iter()
                .find(|item| {
                    item.pull_request.number == number
                        && item.pull_request.repo.owner.eq_ignore_ascii_case(owner)
                })
                .cloned())
        }
    }

    fn open_store() -> (tempfile::TempDir, Store, AccountId) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("live.db")).unwrap();
        let account = store
            .insert_account(Account {
                id: AccountId(0),
                kind: AccountKind::GitHub,
                display_name: "ada".into(),
                host: "github.com".into(),
                external_id: "U_1".into(),
                slack_workspace: None,
                login: "ada".into(),
                auth_mode: AuthMode::GhCli,
                keychain_key: None,
                created_at: ts(0),
            })
            .unwrap();
        settings::set_tracked_orgs(&store, &["acme".into()]).unwrap();
        (dir, store, account.id)
    }

    fn seed(store: &Store, account: AccountId, item: &SyncedPullRequest) {
        let mut writer = Writer::new(store, account, ts(-600), &[]).unwrap();
        writer.write(item).unwrap();
    }

    fn input<'a>(
        store: &'a Store,
        provider: &'a Fake,
        account_id: AccountId,
        now: DateTime<Utc>,
    ) -> LiveInput<'a> {
        LiveInput {
            store,
            provider,
            account_id,
            enabled: true,
            focused: true,
            now,
        }
    }

    #[test]
    fn subject_urls_map_to_pull_requests_only() {
        assert_eq!(
            parse_subject_url("https://api.github.com/repos/acme/widgets/pulls/42"),
            Some(("acme".into(), "widgets".into(), 42))
        );
        assert_eq!(
            parse_subject_url("https://api.github.com/repos/acme/widgets/issues/42"),
            None
        );
        assert_eq!(
            parse_subject_url("https://api.github.com/repos/acme/widgets/pulls/x"),
            None
        );
        assert_eq!(
            parse_subject_url("https://api.github.com/repos/acme/widgets/pulls/4/files"),
            None
        );
    }

    #[test]
    fn last_modified_round_trips_and_a_304_costs_no_fetch() {
        let (_dir, store, account) = open_store();
        let fake = Fake::default();
        fake.answer(Ok(changed("Mon, 01 Jan 2024 00:00:00 GMT", Vec::new())));
        let mut poller = LivePoller::new();
        poller.tick(&input(&store, &fake, account, ts(0))).unwrap();
        assert_eq!(poller.status().state, LiveState::Active);
        assert_eq!(poller.status().checked_at, Some(ts(0)));
        let mark = settings::live_mark(&store).unwrap().unwrap();
        assert_eq!(
            mark.last_modified.as_deref(),
            Some("Mon, 01 Jan 2024 00:00:00 GMT")
        );

        let tick = poller.tick(&input(&store, &fake, account, ts(60))).unwrap();
        assert!(tick.polled);
        assert_eq!(tick.fetched, 0);
        let asked = fake.asked.lock().unwrap();
        assert_eq!(asked[0].if_modified_since, None);
        assert_eq!(
            asked[1].if_modified_since.as_deref(),
            Some("Mon, 01 Jan 2024 00:00:00 GMT")
        );
    }

    #[test]
    fn x_poll_interval_is_honored_and_never_below_sixty_seconds() {
        let (_dir, store, account) = open_store();
        let fake = Fake::default();
        fake.answer(Ok(NotificationsPage::Changed {
            last_modified: Some("A".into()),
            poll_interval: Some(120),
            items: Vec::new(),
            has_next: false,
        }));
        fake.answer(Ok(NotificationsPage::NotModified {
            poll_interval: Some(5),
        }));
        let mut poller = LivePoller::new();
        poller.tick(&input(&store, &fake, account, ts(0))).unwrap();
        assert!(
            !poller
                .tick(&input(&store, &fake, account, ts(60)))
                .unwrap()
                .polled
        );
        assert!(
            !poller
                .tick(&input(&store, &fake, account, ts(119)))
                .unwrap()
                .polled
        );
        assert!(
            poller
                .tick(&input(&store, &fake, account, ts(120)))
                .unwrap()
                .polled
        );
        // A 5 s hint is raised to the 60 s floor.
        assert!(
            !poller
                .tick(&input(&store, &fake, account, ts(150)))
                .unwrap()
                .polled
        );
        assert!(
            poller
                .tick(&input(&store, &fake, account, ts(180)))
                .unwrap()
                .polled
        );
        assert_eq!(fake.polls(), 3);
        assert!(poller.next_due(true, ts(180)) <= ts(240));
    }

    #[test]
    fn several_notifications_for_one_pull_request_cost_one_fetch() {
        let (_dir, store, account) = open_store();
        let mut known = pr("acme", 7, "grace", ChecksState::Success);
        known.pull_request.involvement = Involvement::REVIEW_REQUESTED;
        seed(&store, account, &known);
        let mut moved = known.clone();
        moved.pull_request.title = "Renamed".into();
        moved.pull_request.involvement = Involvement::empty();
        let fake = Fake {
            pulls: vec![moved, pr("acme", 9, "ada", ChecksState::Success)],
            ..Fake::default()
        };
        settings::set_live_mark(
            &store,
            &LiveMark {
                last_modified: Some("A".into()),
                watermark: ts(-60),
            },
        )
        .unwrap();
        fake.answer(Ok(changed(
            "B",
            vec![
                note("1", "PullRequest", &pull_url("acme", 7), "comment", ts(-30)),
                note("2", "PullRequest", &pull_url("ACME", 7), "mention", ts(-20)),
                note(
                    "3",
                    "PullRequest",
                    &pull_url("acme", 7),
                    "review_requested",
                    ts(-10),
                ),
                note("4", "PullRequest", &pull_url("acme", 9), "author", ts(-5)),
            ],
        )));
        let mut poller = LivePoller::new();
        let tick = poller.tick(&input(&store, &fake, account, ts(0))).unwrap();
        assert_eq!(tick.fetched, 2);
        assert_eq!(tick.changed, 2);
        assert_eq!(fake.fetches().len(), 2);
        let rows = store.list_pull_requests_for_account(account).unwrap();
        let seven = rows.iter().find(|row| row.number == 7).unwrap();
        assert_eq!(seven.title, "Renamed");
        // The stored involvement survives a fetch that does not carry it.
        assert_eq!(seven.involvement, Involvement::REVIEW_REQUESTED);
        let nine = rows.iter().find(|row| row.number == 9).unwrap();
        assert_eq!(nine.involvement, Involvement::AUTHOR);
        let mark = settings::live_mark(&store).unwrap().unwrap();
        assert_eq!(mark.last_modified.as_deref(), Some("B"));
        assert_eq!(mark.watermark, ts(-5));
    }

    #[test]
    fn notifications_that_are_not_pull_requests_are_ignored() {
        let (_dir, store, account) = open_store();
        let fake = Fake::default();
        settings::set_live_mark(
            &store,
            &LiveMark {
                last_modified: None,
                watermark: ts(-60),
            },
        )
        .unwrap();
        fake.answer(Ok(changed(
            "B",
            vec![
                note(
                    "1",
                    "Issue",
                    "https://api.github.com/repos/acme/widgets/issues/3",
                    "mention",
                    ts(-10),
                ),
                note(
                    "2",
                    "Release",
                    "https://api.github.com/repos/acme/widgets/releases/1",
                    "subscribed",
                    ts(-10),
                ),
                note(
                    "3",
                    "CheckSuite",
                    "https://api.github.com/repos/acme/widgets/check-suites/1",
                    "ci_activity",
                    ts(-10),
                ),
            ],
        )));
        let tick = LivePoller::new()
            .tick(&input(&store, &fake, account, ts(0)))
            .unwrap();
        assert!(tick.polled);
        assert!(fake.fetches().is_empty());
    }

    #[test]
    fn new_pull_requests_are_added_only_in_scope_and_when_involved() {
        let (_dir, store, account) = open_store();
        let fake = Fake {
            pulls: vec![
                pr("acme", 1, "grace", ChecksState::Success),
                pr("acme", 2, "grace", ChecksState::Success),
                pr("elsewhere", 3, "ada", ChecksState::Success),
            ],
            ..Fake::default()
        };
        settings::set_live_mark(
            &store,
            &LiveMark {
                last_modified: None,
                watermark: ts(-60),
            },
        )
        .unwrap();
        fake.answer(Ok(changed(
            "B",
            vec![
                note(
                    "1",
                    "PullRequest",
                    &pull_url("acme", 1),
                    "review_requested",
                    ts(-10),
                ),
                note(
                    "2",
                    "PullRequest",
                    &pull_url("acme", 2),
                    "subscribed",
                    ts(-10),
                ),
                note(
                    "3",
                    "PullRequest",
                    &pull_url("elsewhere", 3),
                    "author",
                    ts(-10),
                ),
            ],
        )));
        LivePoller::new()
            .tick(&input(&store, &fake, account, ts(0)))
            .unwrap();
        // Out of scope is not even fetched.
        assert_eq!(fake.fetches().len(), 2);
        let rows = store.list_pull_requests_for_account(account).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].number, 1);
        assert_eq!(rows[0].involvement, Involvement::REVIEW_REQUESTED);
    }

    #[test]
    fn an_unsupported_token_disables_polling_once_without_retries() {
        let (_dir, store, account) = open_store();
        let fake = Fake::default();
        fake.answer(Err(ProviderError::Unsupported(
            "Live updates unavailable with this token type: falls back to periodic sync".into(),
        )));
        let mut poller = LivePoller::new();
        for secs in [0, 60, 600, 3600] {
            poller
                .tick(&input(&store, &fake, account, ts(secs)))
                .unwrap();
        }
        assert_eq!(fake.polls(), 1);
        assert!(
            matches!(poller.status().state, LiveState::Unavailable(ref reason) if reason.contains("token type"))
        );
        assert_eq!(poller.status().error, None);
    }

    #[test]
    fn failures_back_off_and_keep_the_mark() {
        let (_dir, store, account) = open_store();
        let fake = Fake::default();
        fake.answer(Err(ProviderError::Network("reset".into())));
        fake.answer(Err(ProviderError::Network("reset".into())));
        let mut poller = LivePoller::new();
        poller.tick(&input(&store, &fake, account, ts(0))).unwrap();
        assert!(
            !poller
                .tick(&input(&store, &fake, account, ts(59)))
                .unwrap()
                .polled
        );
        assert!(
            poller
                .tick(&input(&store, &fake, account, ts(60)))
                .unwrap()
                .polled
        );
        // Second failure: 120 s.
        assert!(
            !poller
                .tick(&input(&store, &fake, account, ts(179)))
                .unwrap()
                .polled
        );
        assert!(
            poller
                .tick(&input(&store, &fake, account, ts(180)))
                .unwrap()
                .polled
        );
        assert!(poller.status().error.is_none());
        assert_eq!(backoff(20), Duration::seconds(MAX_BACKOFF_SECS));
    }

    #[test]
    fn the_watermark_survives_a_restart() {
        let (_dir, store, account) = open_store();
        let fake = Fake {
            pulls: vec![pr("acme", 1, "ada", ChecksState::Success)],
            ..Fake::default()
        };
        // First run: the backlog belongs to the start-up sync.
        fake.answer(Ok(changed(
            "A",
            vec![note(
                "1",
                "PullRequest",
                &pull_url("acme", 1),
                "author",
                ts(-10),
            )],
        )));
        LivePoller::new()
            .tick(&input(&store, &fake, account, ts(0)))
            .unwrap();
        assert!(fake.fetches().is_empty());
        assert_eq!(
            settings::live_mark(&store).unwrap().unwrap().watermark,
            ts(0)
        );

        // Restart: a fresh poller reads the stored mark.
        fake.answer(Ok(changed(
            "B",
            vec![
                note("1", "PullRequest", &pull_url("acme", 1), "author", ts(30)),
                note("1", "PullRequest", &pull_url("acme", 1), "author", ts(-10)),
            ],
        )));
        let tick = LivePoller::new()
            .tick(&input(&store, &fake, account, ts(100)))
            .unwrap();
        assert_eq!(tick.fetched, 1);
        assert_eq!(
            fake.asked.lock().unwrap()[1].if_modified_since.as_deref(),
            Some("A")
        );
        assert_eq!(
            settings::live_mark(&store).unwrap().unwrap().watermark,
            ts(30)
        );
    }

    #[test]
    fn turning_live_updates_off_stops_every_request() {
        let (_dir, store, account) = open_store();
        seed(&store, account, &pr("acme", 1, "ada", ChecksState::Pending));
        let fake = Fake::default();
        let mut poller = LivePoller::new();
        for secs in [0, 600, 1200] {
            let mut off = input(&store, &fake, account, ts(secs));
            off.enabled = false;
            assert_eq!(poller.tick(&off).unwrap(), LiveTick::default());
        }
        assert_eq!(poller.status().state, LiveState::Off);
        assert_eq!(fake.polls(), 0);
        assert!(fake.status_asked.lock().unwrap().is_empty());
        // Back on: polling resumes at once.
        assert!(
            poller
                .tick(&input(&store, &fake, account, ts(1300)))
                .unwrap()
                .polled
        );
    }

    #[test]
    fn pending_checks_are_read_every_90s_focused_and_5min_unfocused() {
        let (_dir, store, account) = open_store();
        seed(&store, account, &pr("acme", 1, "ada", ChecksState::Pending));
        seed(&store, account, &pr("acme", 2, "ada", ChecksState::Success));
        let fake = Fake::default();
        let mut poller = LivePoller::new();
        let at = |secs, focused| {
            let mut i = input(&store, &fake, account, ts(secs));
            i.focused = focused;
            i
        };
        assert_eq!(poller.tick(&at(0, true)).unwrap().checks_read, 0);
        assert_eq!(poller.tick(&at(89, true)).unwrap().checks_read, 0);
        // Only the pull request with running checks is read.
        assert_eq!(poller.tick(&at(90, true)).unwrap().checks_read, 1);
        assert_eq!(poller.tick(&at(180, false)).unwrap().checks_read, 0);
        assert_eq!(poller.tick(&at(389, false)).unwrap().checks_read, 0);
        assert_eq!(poller.tick(&at(390, false)).unwrap().checks_read, 1);
        // Focus back: 90 s since the last read is enough.
        assert_eq!(poller.tick(&at(480, true)).unwrap().checks_read, 1);
        assert_eq!(*fake.status_asked.lock().unwrap(), vec![1, 1, 1]);
    }

    #[test]
    fn a_finished_check_run_is_stored() {
        let (_dir, store, account) = open_store();
        seed(&store, account, &pr("acme", 1, "ada", ChecksState::Pending));
        let fake = Fake {
            statuses: vec![PrStatus {
                remote_id: "PR_acme_1".into(),
                state: None,
                updated_at: None,
                review_decision: None,
                checks: ChecksState::Success,
                mergeable: Mergeable::Mergeable,
                merge_state: MergeStateStatus::Clean,
                unresolved_threads: 0,
                total_threads: 0,
                last_commit_at: None,
            }],
            ..Fake::default()
        };
        let mut poller = LivePoller::new();
        poller.tick(&input(&store, &fake, account, ts(0))).unwrap();
        let tick = poller.tick(&input(&store, &fake, account, ts(90))).unwrap();
        assert_eq!(tick.changed, 1);
        let rows = store.list_pull_requests_for_account(account).unwrap();
        assert_eq!(rows[0].checks, ChecksState::Success);
    }

    #[test]
    fn focus_after_a_minute_away_asks_for_a_sync() {
        assert!(!should_sync_on_focus(None, ts(0)));
        assert!(!should_sync_on_focus(Some(ts(0)), ts(60)));
        assert!(should_sync_on_focus(Some(ts(0)), ts(61)));
    }
}
