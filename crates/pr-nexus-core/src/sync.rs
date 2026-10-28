//! GitHub sync: two search buckets scoped to the tracked orgs, a status pass,
//! a client-side org filter as a safety net, and a nodes refresh.
//!
//! A full reconcile searches every open pull request. An incremental pass
//! searches only what changed since the high-water mark and reads the status
//! of open rows that are still moving (§6).
//!
//! The provider performs HTTP. This module decides what to keep and what to hide.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};

use crate::domain::{
    AccountId, AccountKind, ChecksState, CloseReason, Involvement, Mergeable, PrState, ProviderRef,
    PullRequest, Repository, RequestStatus,
};
use crate::error::CoreError;
use crate::providers::{
    PrQuery, PrStatus, RateBudget, SearchBucket, SyncedPullRequest, VcsProvider,
};
use crate::settings::{self, SyncMark};
use crate::store::Store;

pub const MINE_QUERY: &str = "is:pr is:open author:@me archived:false sort:updated-desc";
pub const REVIEW_QUERY: &str =
    "is:pr is:open review-requested:@me archived:false sort:updated-desc";
/// Page sizes for search, status and `nodes` refresh, largest first. GitHub
/// stops a GraphQL query after about 10 s and answers 502. Mergeability is the
/// slow part, about 0.3 s per pull request, so sync starts at 10 and drops to
/// the next size on a 502/503/504 or a GraphQL timeout (§6).
pub const PAGE_SIZES: [u32; 3] = [10, 5, 3];
/// GitHub rejects a search string longer than this.
pub const SEARCH_QUERY_MAX: usize = 256;
/// Sync reports how many pull requests it has stored at most this often.
pub const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_millis(500);
/// An incremental search reaches this far behind the mark, for search-index
/// lag and clock skew.
pub const MARK_OVERLAP_MINUTES: i64 = 5;
/// A mark older than this, counted from the last full reconcile, means a full
/// reconcile: it is what catches a pull request that left a view.
pub const FULL_EVERY_HOURS: i64 = 6;
/// The status of every open pull request is read at least this often, not
/// only the ones whose checks or mergeability are still moving.
pub const ROLLUP_EVERY_MINUTES: i64 = 30;

/// Lets a progress update through at most once per `every`.
#[derive(Debug, Clone)]
pub struct Throttle {
    every: std::time::Duration,
    last: Option<std::time::Instant>,
}

impl Throttle {
    pub fn new(every: std::time::Duration) -> Self {
        Self { every, last: None }
    }

    /// True for the first call and then once `every` has passed since the
    /// last true.
    pub fn ready(&mut self, now: std::time::Instant) -> bool {
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < self.every)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    /// The timer. Pauses on a low rate budget. Incremental when it can be.
    Scheduled,
    /// The Refresh button. Ignores the pause. Incremental when it can be.
    Manual,
    /// App start and "Full resync": always a full reconcile.
    Full,
}

/// Why a pass is a full reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullReason {
    Requested,
    NoMark,
    StaleMark,
    ScopeChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPlan {
    Full(FullReason),
    /// Search `updated:>=since`; `rollup` reads the status of every open row.
    Incremental {
        since: DateTime<Utc>,
        rollup: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub fetched: usize,
    pub stored: usize,
    pub refreshed: usize,
    pub hidden: usize,
    /// Rows whose stored content changed (written or hidden/shown).
    pub updated: usize,
    pub full: bool,
    pub paused: bool,
    pub message: String,
}

impl SyncReport {
    fn idle(paused: bool, message: String) -> Self {
        Self {
            fetched: 0,
            stored: 0,
            refreshed: 0,
            hidden: 0,
            updated: 0,
            full: false,
            paused,
            message,
        }
    }
}

/// What [`sync_account_with_progress`] reports while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncProgress {
    pub full: bool,
    /// In-scope pull requests the search returned so far.
    pub seen: usize,
    /// Rows written so far because their content changed.
    pub updated: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedScope {
    pub orgs: Vec<String>,
    pub include_personal: bool,
    pub viewer_login: String,
}

pub fn search_query(bucket: SearchBucket) -> &'static str {
    match bucket {
        SearchBucket::Mine => MINE_QUERY,
        SearchBucket::ReviewRequested => REVIEW_QUERY,
    }
}

/// The bucket's search strings, narrowed on GitHub's side to the tracked
/// scope: one ` org:NAME` per tracked org, plus ` user:LOGIN` when personal
/// repositories are on. GitHub ORs several owner qualifiers. A string never
/// exceeds [`SEARCH_QUERY_MAX`]; the qualifiers spill into more strings.
/// Names with characters GitHub logins never have are skipped.
pub fn search_queries(bucket: SearchBucket, scope: &TrackedScope) -> Vec<String> {
    scoped_queries(search_query(bucket), scope)
}

/// The incremental form of [`search_queries`]: no `is:open`, so a pull
/// request merged or closed since the mark comes back too, and
/// `updated:>=since`.
pub fn incremental_queries(
    bucket: SearchBucket,
    scope: &TrackedScope,
    since: DateTime<Utc>,
) -> Vec<String> {
    let base = format!(
        "{} updated:>={}",
        search_query(bucket).replace(" is:open", ""),
        since.format("%Y-%m-%dT%H:%M:%SZ")
    );
    scoped_queries(&base, scope)
}

fn scoped_queries(base: &str, scope: &TrackedScope) -> Vec<String> {
    let mut owners: Vec<String> = Vec::new();
    let mut add = |qualifier: &str, name: &str| {
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        let term = format!(" {qualifier}:{name}");
        if valid && !owners.iter().any(|known| known.eq_ignore_ascii_case(&term)) {
            owners.push(term);
        }
    };
    for org in &scope.orgs {
        add("org", org);
    }
    if scope.include_personal {
        add("user", &scope.viewer_login);
    }
    let mut queries = Vec::new();
    let mut current = base.to_owned();
    for term in owners {
        if current.len() + term.len() > SEARCH_QUERY_MAX && current.len() > base.len() {
            queries.push(std::mem::replace(&mut current, base.to_owned()));
        }
        current.push_str(&term);
    }
    if current.len() > base.len() {
        queries.push(current);
    }
    queries
}

/// Identifies the tracked scope a mark belongs to. Order and case of the
/// orgs do not matter.
pub fn scope_key(scope: &TrackedScope) -> String {
    let mut orgs: Vec<String> = scope
        .orgs
        .iter()
        .map(|org| org.to_ascii_lowercase())
        .filter(|org| !org.is_empty())
        .collect();
    orgs.sort();
    orgs.dedup();
    format!(
        "{}|{}|{}",
        scope.viewer_login.to_ascii_lowercase(),
        scope.include_personal,
        orgs.join(",")
    )
}

/// Full or incremental, from the kind of sync and the stored mark.
pub fn plan_sync(
    kind: SyncKind,
    mark: Option<&SyncMark>,
    scope_key: &str,
    now: DateTime<Utc>,
) -> SyncPlan {
    if kind == SyncKind::Full {
        return SyncPlan::Full(FullReason::Requested);
    }
    let Some(mark) = mark else {
        return SyncPlan::Full(FullReason::NoMark);
    };
    if mark.scope != scope_key {
        return SyncPlan::Full(FullReason::ScopeChanged);
    }
    if now - mark.full_at >= Duration::hours(FULL_EVERY_HOURS) {
        return SyncPlan::Full(FullReason::StaleMark);
    }
    SyncPlan::Incremental {
        since: mark.updated_at - Duration::minutes(MARK_OVERLAP_MINUTES),
        rollup: now - mark.rollup_at >= Duration::minutes(ROLLUP_EVERY_MINUTES),
    }
}

/// An open pull request whose checks or mergeability GitHub is still working
/// out. Its status changes without a search hit, so incremental sync reads
/// it on every pass.
pub fn status_is_moving(pr: &PullRequest) -> bool {
    matches!(pr.checks, ChecksState::Pending | ChecksState::Expected)
        || pr.mergeable == Mergeable::Unknown
}

pub fn in_scope(owner: &str, scope: &TrackedScope) -> bool {
    if scope.orgs.iter().any(|org| org.eq_ignore_ascii_case(owner)) {
        return true;
    }
    scope.include_personal && scope.viewer_login.eq_ignore_ascii_case(owner)
}

pub fn scope_is_empty(scope: &TrackedScope) -> bool {
    scope.orgs.is_empty() && !scope.include_personal
}

/// Scheduled polls stop below 10% of the limit, and while a secondary limit is active.
/// Manual refresh ignores this.
pub fn scheduled_poll_paused(budget: &RateBudget, now: DateTime<Utc>) -> bool {
    if budget.secondary_until.is_some_and(|until| now < until) {
        return true;
    }
    budget.limit > 0 && u64::from(budget.remaining) * 10 < u64::from(budget.limit)
}

pub fn should_hide_closed(
    state: PrState,
    updated_at: DateTime<Utc>,
    now: DateTime<Utc>,
    keep_days: i64,
) -> bool {
    if state == PrState::Open {
        return false;
    }
    now.signed_duration_since(updated_at).num_days() >= keep_days
}

pub fn sync_account(
    store: &Store,
    provider: &dyn VcsProvider,
    account_id: AccountId,
    kind: SyncKind,
    now: DateTime<Utc>,
) -> Result<SyncReport, CoreError> {
    sync_account_with_progress(store, provider, account_id, kind, now, &mut |_| {})
}

/// [`sync_account`], storing each search page as it arrives and calling
/// `progress` as it goes. The caller throttles what it shows (see
/// [`Throttle`]).
pub fn sync_account_with_progress(
    store: &Store,
    provider: &dyn VcsProvider,
    account_id: AccountId,
    kind: SyncKind,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(SyncProgress),
) -> Result<SyncReport, CoreError> {
    let account = store
        .get_account(account_id)?
        .ok_or_else(|| CoreError::Validation("github account is missing".into()))?;
    if account.kind != AccountKind::GitHub {
        return Err(CoreError::Validation(
            "sync account is not a GitHub account".into(),
        ));
    }
    let scope = TrackedScope {
        orgs: settings::tracked_orgs(store)?,
        include_personal: settings::include_personal_repos(store)?,
        viewer_login: account.login,
    };
    if scope_is_empty(&scope) {
        return Ok(SyncReport::idle(false, "Choose orgs to track".into()));
    }
    if kind == SyncKind::Scheduled
        && let Some(budget) = settings::rate_budget(store)?
        && scheduled_poll_paused(&budget, now)
    {
        return Ok(SyncReport::idle(
            true,
            format!("Rate limited · resets {}", budget.reset_at.format("%H:%M")),
        ));
    }

    let key = scope_key(&scope);
    let mark = settings::sync_mark(store)?;
    let plan = plan_sync(kind, mark.as_ref(), &key, now);
    let full = matches!(plan, SyncPlan::Full(_));

    // Rows as they were before this sync: their status and involvement fill
    // in for what a search page does not carry.
    let existing = store.list_pull_requests_for_account(account_id)?;
    let mut writer = Writer::new(store, account_id, now, &existing)?;
    let mut fetched: Vec<SyncedPullRequest> = Vec::new();
    let mut early = EarlyRows::default();
    for bucket in [SearchBucket::Mine, SearchBucket::ReviewRequested] {
        let queries = match plan {
            SyncPlan::Full(_) => search_queries(bucket, &scope),
            SyncPlan::Incremental { since, .. } => incremental_queries(bucket, &scope, since),
        };
        let mut failed: Option<CoreError> = None;
        let page = provider
            .search_pull_requests_paged(&PrQuery { bucket, queries }, &mut |rows| {
                if failed.is_some() {
                    return;
                }
                match early.store(&mut writer, rows, &scope, &existing) {
                    Ok(()) => progress(SyncProgress {
                        full,
                        seen: early.involvement.len(),
                        updated: writer.updated(),
                    }),
                    Err(err) => failed = Some(err),
                }
            })
            .map_err(CoreError::from)?;
        if let Some(err) = failed {
            return Err(err);
        }
        merge_bucket(&mut fetched, page);
    }

    // Search is scoped on GitHub's side; this filter is the safety net.
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_orgs = settings::seen_orgs(store)?;
    let mut kept: Vec<SyncedPullRequest> = Vec::new();
    let mut latest: Option<DateTime<Utc>> = None;
    for item in fetched.iter() {
        let owner = &item.pull_request.repo.owner;
        remember_org(&mut seen_orgs, owner, &scope.viewer_login);
        seen_ids.insert(item.pull_request.remote.id.clone());
        if in_scope(owner, &scope) && keeps_hit(item, &existing) {
            latest = latest.max(Some(item.pull_request.updated_at));
            kept.push(item.clone());
        }
    }
    settings::set_seen_orgs(store, &seen_orgs)?;

    // Incremental: open rows the search did not return whose status may have
    // moved anyway: checks or mergeability still pending, or every open row
    // when the periodic rollup is due.
    let rollup = match plan {
        SyncPlan::Full(_) => false,
        SyncPlan::Incremental { rollup, .. } => rollup,
    };
    let volatile: Vec<&PullRequest> = if full {
        Vec::new()
    } else {
        existing
            .iter()
            .filter(|pr| {
                pr.state == PrState::Open
                    && in_scope(&pr.repo.owner, &scope)
                    && !seen_ids.contains(&pr.remote.id)
                    && (rollup || status_is_moving(pr))
            })
            .collect()
    };

    let ids: Vec<ProviderRef> = kept
        .iter()
        .map(|item| item.pull_request.remote.clone())
        .chain(volatile.iter().map(|pr| pr.remote.clone()))
        .collect();
    let statuses: HashMap<String, PrStatus> = if ids.is_empty() {
        HashMap::new()
    } else {
        provider
            .pull_request_status(&ids)
            .map_err(CoreError::from)?
            .into_iter()
            .map(|status| (status.remote_id.clone(), status))
            .collect()
    };
    let mut stored = 0usize;
    for item in &mut kept {
        let id = &item.pull_request.remote.id;
        let status = statuses.get(id).cloned().or_else(|| {
            existing
                .iter()
                .find(|pr| &pr.remote.id == id)
                .map(PrStatus::of)
        });
        if let Some(status) = status {
            status.apply_to(&mut item.pull_request);
        }
        writer.write(item)?;
        stored += 1;
    }
    if let Ok(budget) = provider.rate_limit() {
        settings::set_rate_budget(store, &budget)?;
    }

    // Open rows to read again in full: in a full reconcile, the ones the
    // search no longer returns; incrementally, the volatile ones whose status
    // answer says they finished or changed beyond their status.
    let mut status_only: Vec<SyncedPullRequest> = Vec::new();
    let stale: Vec<ProviderRef> = if full {
        writer
            .rows()
            .filter(|pr| {
                pr.state == PrState::Open
                    && in_scope(&pr.repo.owner, &scope)
                    && !seen_ids.contains(&pr.remote.id)
            })
            .map(|pr| pr.remote.clone())
            .collect()
    } else {
        let mut stale = Vec::new();
        for pr in &volatile {
            let Some(status) = statuses.get(&pr.remote.id) else {
                continue;
            };
            let finished = status.state.is_some_and(|state| state != PrState::Open);
            let touched = status.updated_at.is_some_and(|at| at > pr.updated_at);
            if finished || touched {
                stale.push(pr.remote.clone());
            } else {
                let mut row = (*pr).clone();
                status.apply_to(&mut row);
                status_only.push(writer.synced_from(row));
            }
        }
        stale
    };
    for item in &status_only {
        writer.write(item)?;
    }
    let mut refreshed_rows = if stale.is_empty() {
        Vec::new()
    } else {
        provider
            .refresh_pull_requests(&stale)
            .map_err(CoreError::from)?
    };
    for item in &mut refreshed_rows {
        let id = item.pull_request.remote.id.clone();
        if let Some(prev) = existing.iter().find(|pr| pr.remote.id == id) {
            item.pull_request.involvement = prev.involvement;
        }
        if let Some(status) = statuses.get(&id) {
            status.apply_to(&mut item.pull_request);
        }
    }
    let refreshed = refreshed_rows.len();
    for item in &refreshed_rows {
        if in_scope(&item.pull_request.repo.owner, &scope) {
            latest = latest.max(Some(item.pull_request.updated_at));
            writer.write(item)?;
        }
    }

    let keep_days = settings::keep_closed_days(store)?;
    let mut hidden = 0usize;
    let rows: Vec<PullRequest> = writer.rows().cloned().collect();
    for pr in rows {
        close_requests_of_finished(store, &pr, now)?;
        let out_of_scope = !in_scope(&pr.repo.owner, &scope);
        let expired = should_hide_closed(pr.state, pr.updated_at, now, keep_days);
        let next = out_of_scope || expired;
        if next != pr.hidden {
            writer.set_hidden(&pr, next)?;
            if next {
                hidden += 1;
            }
        }
    }

    // The pass succeeded: only now does the mark move.
    let previous = mark.filter(|mark| mark.scope == key);
    let updated_at = latest
        .into_iter()
        .chain(previous.as_ref().map(|mark| mark.updated_at))
        .max()
        .unwrap_or(now);
    settings::set_sync_mark(
        store,
        &SyncMark {
            scope: key,
            updated_at,
            full_at: match (&previous, full) {
                (Some(previous), false) => previous.full_at,
                _ => now,
            },
            rollup_at: match (&previous, full || rollup) {
                (Some(previous), false) => previous.rollup_at,
                _ => now,
            },
        },
    )?;

    let updated = writer.updated();
    Ok(SyncReport {
        fetched: fetched.len(),
        stored,
        refreshed,
        hidden,
        updated,
        full,
        paused: false,
        message: if full {
            format!("{stored} pull requests")
        } else {
            format!("{updated} updated")
        },
    })
}

/// A search hit worth storing. A full search only returns open pull
/// requests. An incremental one also returns finished ones; those matter
/// only when the row is already stored, so an old closed pull request with a
/// fresh comment does not appear in the list.
fn keeps_hit(item: &SyncedPullRequest, existing: &[PullRequest]) -> bool {
    item.pull_request.state == PrState::Open
        || existing
            .iter()
            .any(|pr| pr.remote.id == item.pull_request.remote.id)
}

/// §7.4: once sync sees a PR merged or closed, its active review requests close
/// with that reason, so the list and the Slack panel stop offering a bump.
fn close_requests_of_finished(
    store: &Store,
    pr: &PullRequest,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let reason = match pr.state {
        PrState::Open => return Ok(()),
        PrState::Merged => CloseReason::PrMerged,
        PrState::Closed => CloseReason::PrClosed,
    };
    for request in store.list_review_requests(pr.id)? {
        if request.status == RequestStatus::Active {
            store.close_review_request(request.id, reason, now)?;
        }
    }
    Ok(())
}

fn remember_org(orgs: &mut Vec<String>, owner: &str, viewer: &str) {
    if owner.eq_ignore_ascii_case(viewer) {
        return;
    }
    if orgs.iter().any(|org| org.eq_ignore_ascii_case(owner)) {
        return;
    }
    orgs.push(owner.to_owned());
}

fn merge_bucket(into: &mut Vec<SyncedPullRequest>, page: Vec<SyncedPullRequest>) {
    for item in page {
        if let Some(existing) = into
            .iter_mut()
            .find(|current| current.pull_request.remote.id == item.pull_request.remote.id)
        {
            let involvement = existing.pull_request.involvement | item.pull_request.involvement;
            if item.pull_request.updated_at >= existing.pull_request.updated_at {
                *existing = item;
            }
            existing.pull_request.involvement = involvement;
        } else {
            into.push(item);
        }
    }
}

/// In-scope search rows stored while the search runs, so the list fills in
/// page by page. The status pass and the final write overwrite them.
#[derive(Default)]
struct EarlyRows {
    /// Involvement stored so far this sync, by node id.
    involvement: HashMap<String, Involvement>,
}

impl EarlyRows {
    /// Stores `rows` with the status they had before this sync and their
    /// involvement merged with what is known, so a page from one bucket
    /// never drops the other bucket's flag.
    fn store(
        &mut self,
        writer: &mut Writer<'_>,
        rows: &[SyncedPullRequest],
        scope: &TrackedScope,
        existing: &[PullRequest],
    ) -> Result<(), CoreError> {
        for item in rows {
            if !in_scope(&item.pull_request.repo.owner, scope) || !keeps_hit(item, existing) {
                continue;
            }
            let id = item.pull_request.remote.id.clone();
            let mut row = item.clone();
            let before = existing.iter().find(|pr| pr.remote.id == id);
            if let Some(before) = before {
                PrStatus::of(before).apply_to(&mut row.pull_request);
                row.pull_request.involvement |= before.involvement;
            }
            let known = self.involvement.entry(id).or_insert(Involvement::empty());
            *known |= row.pull_request.involvement;
            row.pull_request.involvement = *known;
            writer.write(&row)?;
        }
        Ok(())
    }
}

/// Writes a pull request or its repository only when what would be stored
/// differs from what is stored, so an unchanged row costs no transaction
/// and no list refresh.
pub(crate) struct Writer<'a> {
    store: &'a Store,
    account_id: AccountId,
    now: DateTime<Utc>,
    /// Stored rows by node id, kept current as this sync writes.
    rows: HashMap<String, PullRequest>,
    /// Node ids in the order they were first known, for a stable walk.
    order: Vec<String>,
    repos: HashMap<(String, String), Repository>,
    changed: HashSet<String>,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(
        store: &'a Store,
        account_id: AccountId,
        now: DateTime<Utc>,
        existing: &[PullRequest],
    ) -> Result<Self, CoreError> {
        let repos = store
            .list_repositories(account_id)?
            .into_iter()
            .map(|repo| ((repo.owner.clone(), repo.name.clone()), repo))
            .collect();
        Ok(Self {
            store,
            account_id,
            now,
            rows: existing
                .iter()
                .map(|pr| (pr.remote.id.clone(), pr.clone()))
                .collect(),
            order: existing.iter().map(|pr| pr.remote.id.clone()).collect(),
            repos,
            changed: HashSet::new(),
        })
    }

    pub(crate) fn updated(&self) -> usize {
        self.changed.len()
    }

    pub(crate) fn rows(&self) -> impl Iterator<Item = &PullRequest> {
        self.order.iter().filter_map(|id| self.rows.get(id))
    }

    /// A stored row as a [`SyncedPullRequest`], with its repository's facts.
    pub(crate) fn synced_from(&self, pr: PullRequest) -> SyncedPullRequest {
        let repo = self
            .repos
            .get(&(pr.repo.owner.clone(), pr.repo.name.clone()));
        SyncedPullRequest {
            repo_node_id: repo.and_then(|repo| repo.node_id.clone()),
            default_branch: repo.and_then(|repo| repo.default_branch.clone()),
            repo_archived: repo.is_some_and(|repo| repo.archived),
            pull_request: pr,
        }
    }

    pub(crate) fn write(&mut self, item: &SyncedPullRequest) -> Result<bool, CoreError> {
        let mut pr = item.pull_request.clone();
        pr.remote.account = self.account_id;
        pr.hidden = false;
        pr.body = crate::store::cap_body(&pr.body);
        let id = pr.remote.id.clone();
        let key = (pr.repo.owner.clone(), pr.repo.name.clone());
        let wanted = Repository {
            id: 0,
            account_id: self.account_id,
            owner: key.0.clone(),
            name: key.1.clone(),
            node_id: item.repo_node_id.clone(),
            default_branch: item.default_branch.clone(),
            archived: item.repo_archived,
        };
        let repo = match self.repos.get(&key) {
            Some(repo)
                if Repository {
                    id: 0,
                    ..repo.clone()
                } == wanted =>
            {
                repo.clone()
            }
            _ => {
                let repo = self.store.upsert_repository(wanted)?;
                self.repos.insert(key, repo.clone());
                repo
            }
        };
        if let Some(before) = self.rows.get(&id)
            && same_content(before, &pr)
        {
            return Ok(false);
        }
        pr.synced_at = self.now;
        let saved = self.store.upsert_pull_request(repo.id, pr)?;
        if !self.rows.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.rows.insert(id.clone(), saved);
        self.changed.insert(id);
        Ok(true)
    }

    pub(crate) fn set_hidden(&mut self, pr: &PullRequest, hidden: bool) -> Result<(), CoreError> {
        self.store.set_pull_request_hidden(pr.id, hidden)?;
        if let Some(row) = self.rows.get_mut(&pr.remote.id) {
            row.hidden = hidden;
        }
        self.changed.insert(pr.remote.id.clone());
        Ok(())
    }
}

/// Equal apart from what sync stamps on every write.
fn same_content(stored: &PullRequest, next: &PullRequest) -> bool {
    let mut next = next.clone();
    next.id = stored.id;
    next.synced_at = stored.synced_at;
    next == *stored
}

/// Remember a secondary limit so the next scheduled poll waits.
pub fn note_rate_limit(
    store: &Store,
    reset_at: DateTime<Utc>,
    secondary: bool,
) -> Result<(), CoreError> {
    let mut budget = settings::rate_budget(store)?.unwrap_or(RateBudget {
        limit: 0,
        remaining: 0,
        reset_at,
        cost: 0,
        secondary_until: None,
    });
    if secondary {
        budget.secondary_until = Some(reset_at);
    } else {
        budget.reset_at = reset_at;
        budget.remaining = 0;
    }
    settings::set_rate_budget(store, &budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        Account, AuthMode, ChecksState, Involvement, MergeStateStatus, Mergeable, RepoRef,
    };
    use crate::error::ProviderError;
    use chrono::TimeZone;

    fn ts(mins: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + mins * 60, 0).unwrap()
    }

    fn pr(node: &str, owner: &str, number: u64, involvement: Involvement) -> SyncedPullRequest {
        SyncedPullRequest {
            pull_request: PullRequest {
                id: crate::domain::PrId(0),
                remote: ProviderRef {
                    account: AccountId(0),
                    id: node.into(),
                },
                repo: RepoRef {
                    host: "github.com".into(),
                    owner: owner.into(),
                    name: "widgets".into(),
                },
                number,
                title: format!("PR {number}"),
                url: format!("https://github.com/{owner}/widgets/pull/{number}"),
                author_login: "ada".into(),
                body: String::new(),
                state: PrState::Open,
                is_draft: false,
                head_ref: "feature".into(),
                base_ref: "main".into(),
                head_sha: "abc".into(),
                labels: Vec::new(),
                review_decision: None,
                checks: ChecksState::Success,
                mergeable: Mergeable::Mergeable,
                merge_state: MergeStateStatus::Clean,
                requested_reviewers: Vec::new(),
                unresolved_threads: 0,
                total_threads: 0,
                involvement,
                created_at: ts(0),
                updated_at: ts(10),
                last_activity_at: ts(10),
                synced_at: ts(0),
                hidden: false,
            },
            repo_node_id: Some("R_1".into()),
            default_branch: Some("main".into()),
            repo_archived: false,
        }
    }

    struct Fake {
        mine: Vec<SyncedPullRequest>,
        review: Vec<SyncedPullRequest>,
        refresh: Vec<SyncedPullRequest>,
        status: Vec<PrStatus>,
        budget: RateBudget,
        queries: std::sync::Mutex<Vec<PrQuery>>,
        status_asked: std::sync::Mutex<Vec<String>>,
        /// The status pass fails, as a network error mid-sync would.
        fail_status: bool,
        refresh_asked: std::sync::Mutex<Vec<String>>,
    }

    impl VcsProvider for Fake {
        fn kind(&self) -> crate::domain::VcsKind {
            crate::domain::VcsKind::GitHub
        }
        fn viewer(&self) -> Result<crate::providers::VcsUser, ProviderError> {
            Ok(crate::providers::VcsUser {
                id: "U_1".into(),
                login: "ada".into(),
                organizations: vec!["acme".into()],
                organizations_needing_sso: Vec::new(),
            })
        }
        fn search_pull_requests(
            &self,
            query: &PrQuery,
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            self.queries.lock().unwrap().push(query.clone());
            Ok(match query.bucket {
                SearchBucket::Mine => self.mine.clone(),
                SearchBucket::ReviewRequested => self.review.clone(),
            })
        }
        /// Hands rows over in pages of 10, like GitHub.
        fn search_pull_requests_paged(
            &self,
            query: &PrQuery,
            on_page: &mut dyn FnMut(&[SyncedPullRequest]),
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            let items = self.search_pull_requests(query)?;
            for page in items.chunks(10) {
                on_page(page);
            }
            Ok(items)
        }
        fn refresh_pull_requests(
            &self,
            ids: &[ProviderRef],
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            self.refresh_asked
                .lock()
                .unwrap()
                .extend(ids.iter().map(|id| id.id.clone()));
            Ok(self
                .refresh
                .iter()
                .filter(|item| ids.iter().any(|id| id.id == item.pull_request.remote.id))
                .cloned()
                .collect())
        }
        fn pull_request_status(&self, ids: &[ProviderRef]) -> Result<Vec<PrStatus>, ProviderError> {
            self.status_asked
                .lock()
                .unwrap()
                .extend(ids.iter().map(|id| id.id.clone()));
            if self.fail_status {
                return Err(ProviderError::Network("connection reset".into()));
            }
            Ok(self
                .status
                .iter()
                .filter(|status| ids.iter().any(|id| id.id == status.remote_id))
                .cloned()
                .collect())
        }
        fn ownership_probe(
            &self,
            _pr: &PullRequest,
        ) -> Result<crate::providers::OwnershipProbe, ProviderError> {
            Err(ProviderError::Unsupported(
                "sync never reads ownership".into(),
            ))
        }
        fn ownership_inputs(
            &self,
            _pr: &PullRequest,
            _probe: &crate::providers::OwnershipProbe,
        ) -> Result<crate::providers::OwnershipInputs, ProviderError> {
            Err(ProviderError::Unsupported(
                "sync never reads ownership".into(),
            ))
        }
        fn team_members(
            &self,
            _team: &crate::domain::TeamRef,
        ) -> Result<crate::providers::TeamMembers, ProviderError> {
            Err(ProviderError::Unsupported("sync never reads teams".into()))
        }
        fn rate_limit(&self) -> Result<RateBudget, ProviderError> {
            Ok(self.budget.clone())
        }
        fn request_team_review(
            &self,
            _pr: &PullRequest,
            _teams: &[crate::domain::TeamRef],
        ) -> Result<(), ProviderError> {
            Err(ProviderError::Unsupported("github writes are M10".into()))
        }
    }

    fn open_store() -> (tempfile::TempDir, Store, AccountId) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("sync.db")).unwrap();
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

    #[test]
    fn every_page_is_stored_as_it_arrives_with_a_running_count() {
        let (_dir, store, account) = open_store();
        let mine: Vec<_> = (1..=250)
            .map(|n| pr(&format!("N{n}"), "acme", n, Involvement::AUTHOR))
            .chain((1..=5).map(|n| pr(&format!("X{n}"), "elsewhere", n, Involvement::AUTHOR)))
            .collect();
        // Also review-requested: one page from each bucket must keep both flags.
        let review = vec![pr("N7", "acme", 7, Involvement::REVIEW_REQUESTED)];
        let fake = Fake {
            mine,
            review,
            refresh: Vec::new(),
            status: Vec::new(),
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        let mut counts = Vec::new();
        let report = sync_account_with_progress(
            &store,
            &fake,
            account,
            SyncKind::Manual,
            ts(30),
            &mut |progress| {
                // Rows are in the store before the search (and status pass) ends.
                let rows = store.list_pull_requests_for_account(account).unwrap();
                assert_eq!(rows.len(), progress.seen);
                assert_eq!(progress.updated, progress.seen);
                counts.push(progress.seen);
            },
        )
        .unwrap();
        assert!(fake.status_asked.lock().unwrap().len() == 250);
        assert_eq!(counts.first(), Some(&10));
        assert_eq!(counts.last(), Some(&250));
        assert!(counts.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(counts.len() >= 25);
        assert_eq!(report.stored, 250);
        let rows = store.list_pull_requests_for_account(account).unwrap();
        assert_eq!(rows.len(), 250);
        let both = rows.iter().find(|row| row.number == 7).unwrap();
        assert_eq!(
            both.involvement,
            Involvement::AUTHOR | Involvement::REVIEW_REQUESTED
        );
    }

    #[test]
    fn throttle_lets_one_update_through_per_interval() {
        let start = std::time::Instant::now();
        let every = std::time::Duration::from_millis(500);
        let mut throttle = Throttle::new(every);
        assert!(throttle.ready(start));
        assert!(!throttle.ready(start + std::time::Duration::from_millis(10)));
        assert!(!throttle.ready(start + std::time::Duration::from_millis(499)));
        assert!(throttle.ready(start + every));
        assert!(!throttle.ready(start + every + std::time::Duration::from_millis(100)));
        assert!(throttle.ready(start + every * 2));
    }

    #[test]
    fn empty_scope_does_not_fetch() {
        let (_dir, store, account) = open_store();
        settings::set_tracked_orgs(&store, &[]).unwrap();
        let fake = Fake {
            mine: vec![pr("N1", "acme", 1, Involvement::AUTHOR)],
            review: Vec::new(),
            refresh: Vec::new(),
            status: Vec::new(),
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        let report = sync_account(&store, &fake, account, SyncKind::Manual, ts(20)).unwrap();
        assert_eq!(report.message, "Choose orgs to track");
        assert!(
            store
                .list_pull_requests_for_account(account)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn client_side_org_filter_keeps_tracked_orgs_and_merges_involvement() {
        let (_dir, store, account) = open_store();
        let mut authored = pr("N1", "acme", 1, Involvement::AUTHOR);
        let mut requested = pr("N1", "acme", 1, Involvement::REVIEW_REQUESTED);
        requested.pull_request.updated_at = ts(12);
        authored.pull_request.updated_at = ts(11);
        let other = pr("N2", "elsewhere", 2, Involvement::AUTHOR);
        let fake = Fake {
            mine: vec![authored, other],
            review: vec![requested],
            refresh: Vec::new(),
            status: Vec::new(),
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        let report = sync_account(&store, &fake, account, SyncKind::Manual, ts(30)).unwrap();
        assert_eq!(report.stored, 1);
        let rows = store.list_pull_requests_for_account(account).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].number, 1);
        assert!(rows[0].involvement.contains(Involvement::AUTHOR));
        assert!(rows[0].involvement.contains(Involvement::REVIEW_REQUESTED));
        assert!(
            settings::seen_orgs(&store)
                .unwrap()
                .iter()
                .any(|org| org == "acme")
        );
        assert!(
            settings::seen_orgs(&store)
                .unwrap()
                .iter()
                .any(|org| org == "elsewhere")
        );
    }

    #[test]
    fn disappeared_open_prs_are_refreshed_and_old_closed_prs_hide() {
        let (_dir, store, account) = open_store();
        let live = pr("N1", "acme", 1, Involvement::AUTHOR);
        let mut gone = pr("N9", "acme", 9, Involvement::AUTHOR);
        gone.pull_request.state = PrState::Merged;
        gone.pull_request.updated_at = ts(-(8 * 24 * 60));
        let fake = Fake {
            mine: vec![live.clone()],
            review: Vec::new(),
            refresh: vec![gone],
            status: Vec::new(),
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        sync_account(&store, &fake, account, SyncKind::Manual, ts(30)).unwrap();
        // Seed an open row that search no longer returns, then sync again.
        let repo_id = store.list_repositories(account).unwrap()[0].id;
        let mut missing = live.pull_request.clone();
        missing.remote.id = "N9".into();
        missing.number = 9;
        missing.remote.account = account;
        store.upsert_pull_request(repo_id, missing).unwrap();

        let report = sync_account(&store, &fake, account, SyncKind::Full, ts(30)).unwrap();
        assert_eq!(report.refreshed, 1);
        let rows = store.list_pull_requests_for_account(account).unwrap();
        let closed = rows.iter().find(|row| row.number == 9).unwrap();
        assert_eq!(closed.state, PrState::Merged);
        assert!(closed.hidden);
        let open = rows.iter().find(|row| row.number == 1).unwrap();
        assert!(!open.hidden);
    }

    #[test]
    fn a_pr_merged_since_the_last_sync_leaves_the_open_list_and_closes_its_request() {
        let (_dir, store, account) = open_store();
        let live = pr("N1", "acme", 1, Involvement::AUTHOR);
        let merged_pr = pr("N2", "acme", 2, Involvement::AUTHOR);
        let first = Fake {
            mine: vec![live.clone(), merged_pr.clone()],
            review: Vec::new(),
            refresh: Vec::new(),
            status: Vec::new(),
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        sync_account(&store, &first, account, SyncKind::Manual, ts(20)).unwrap();
        let stored = store.list_pull_requests_for_account(account).unwrap();
        let tracked = stored.iter().find(|row| row.number == 2).unwrap();
        assert_eq!(tracked.state, PrState::Open);
        let request = store
            .insert_review_request(crate::domain::ReviewRequest {
                team_id: None,
                id: crate::domain::ReviewRequestId(0),
                pr: tracked.id,
                root: ProviderRef {
                    account,
                    id: "1700000001.000100".into(),
                },
                workspace: "acme".into(),
                channel_id: "C0123456789".into(),
                channel_name: "reviews".into(),
                root_ts: "1700000001.000100".into(),
                permalink: None,
                requested_at: ts(20),
                last_bumped_at: None,
                bump_count: 0,
                status: RequestStatus::Active,
                closed_at: None,
                source: crate::domain::RequestSource::App,
            })
            .unwrap();

        // Merged an hour later: `is:open` search drops it, the nodes refresh
        // returns its final state.
        let mut done = merged_pr;
        done.pull_request.state = PrState::Merged;
        done.pull_request.updated_at = ts(60);
        let second = Fake {
            mine: vec![live],
            review: Vec::new(),
            refresh: vec![done],
            status: Vec::new(),
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        let report = sync_account(&store, &second, account, SyncKind::Full, ts(70)).unwrap();
        assert_eq!(report.refreshed, 1);
        let rows = store.list_pull_requests_for_account(account).unwrap();
        let gone = rows.iter().find(|row| row.number == 2).unwrap();
        assert_eq!(gone.state, PrState::Merged);
        // Within keep_closed_days the row is kept (the Archived filter shows it),
        // but it is no longer open.
        assert!(!gone.hidden);
        let open: Vec<u64> = rows
            .iter()
            .filter(|row| row.state == PrState::Open && !row.hidden)
            .map(|row| row.number)
            .collect();
        assert_eq!(open, vec![1]);
        let closed = store.get_review_request(request.id).unwrap().unwrap();
        assert_eq!(closed.status, RequestStatus::Closed(CloseReason::PrMerged));
    }

    #[test]
    fn scheduled_poll_pauses_under_ten_percent_and_manual_does_not() {
        let budget = sample_budget(1000, 50);
        assert!(scheduled_poll_paused(&budget, ts(1)));
        assert!(!scheduled_poll_paused(&sample_budget(1000, 100), ts(1)));
        let (_dir, store, account) = open_store();
        settings::set_rate_budget(&store, &budget).unwrap();
        let fake = Fake {
            mine: vec![pr("N1", "acme", 1, Involvement::AUTHOR)],
            review: Vec::new(),
            refresh: Vec::new(),
            status: Vec::new(),
            budget: budget.clone(),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        let paused = sync_account(&store, &fake, account, SyncKind::Scheduled, ts(30)).unwrap();
        assert!(paused.paused);
        assert!(
            store
                .list_pull_requests_for_account(account)
                .unwrap()
                .is_empty()
        );
        let manual = sync_account(&store, &fake, account, SyncKind::Manual, ts(30)).unwrap();
        assert!(!manual.paused);
        assert_eq!(manual.stored, 1);
    }

    #[test]
    fn personal_repos_are_opt_in() {
        let scope = TrackedScope {
            orgs: vec!["acme".into()],
            include_personal: false,
            viewer_login: "ada".into(),
        };
        assert!(in_scope("acme", &scope));
        assert!(!in_scope("ada", &scope));
        let mut personal = scope.clone();
        personal.include_personal = true;
        assert!(in_scope("Ada", &personal));
        assert!(scope_is_empty(&TrackedScope {
            orgs: Vec::new(),
            include_personal: false,
            viewer_login: "ada".into(),
        }));
    }

    fn scope(orgs: &[&str], personal: bool) -> TrackedScope {
        TrackedScope {
            orgs: orgs.iter().map(|org| (*org).to_owned()).collect(),
            include_personal: personal,
            viewer_login: "ada".into(),
        }
    }

    #[test]
    fn searches_are_scoped_to_the_tracked_orgs() {
        assert_eq!(
            search_queries(SearchBucket::Mine, &scope(&["acme", "acme-labs"], false)),
            vec![format!("{MINE_QUERY} org:acme org:acme-labs")]
        );
        assert_eq!(
            search_queries(SearchBucket::ReviewRequested, &scope(&["acme"], true)),
            vec![format!("{REVIEW_QUERY} org:acme user:ada")]
        );
        assert_eq!(
            search_queries(SearchBucket::Mine, &scope(&[], true)),
            vec![format!("{MINE_QUERY} user:ada")]
        );
        assert!(search_queries(SearchBucket::Mine, &scope(&[], false)).is_empty());
    }

    #[test]
    fn long_org_lists_spill_into_more_searches_under_the_length_cap() {
        let orgs: Vec<String> = (0..40).map(|n| format!("acme-team-{n:02}")).collect();
        let names: Vec<&str> = orgs.iter().map(String::as_str).collect();
        let queries = search_queries(SearchBucket::ReviewRequested, &scope(&names, true));
        assert!(queries.len() > 1, "{queries:?}");
        for query in &queries {
            assert!(query.len() <= SEARCH_QUERY_MAX, "{} chars", query.len());
            assert!(query.starts_with(REVIEW_QUERY));
        }
        let joined = queries.join(" ");
        for org in &orgs {
            assert_eq!(
                joined.matches(&format!(" org:{org} ")).count()
                    + usize::from(joined.ends_with(&format!(" org:{org}"))),
                1,
                "{org}"
            );
        }
        assert_eq!(joined.matches(" user:ada").count(), 1);
    }

    #[test]
    fn odd_or_repeated_org_names_never_reach_the_search_string() {
        let queries = search_queries(
            SearchBucket::Mine,
            &scope(&["acme", "ACME", "bad name", "x\"y", ""], false),
        );
        assert_eq!(queries, vec![format!("{MINE_QUERY} org:acme")]);
    }

    fn status(id: &str, checks: ChecksState) -> PrStatus {
        PrStatus {
            remote_id: id.into(),
            state: None,
            updated_at: None,
            review_decision: Some(crate::domain::ReviewDecision::Approved),
            checks,
            mergeable: Mergeable::Conflicting,
            merge_state: MergeStateStatus::Dirty,
            unresolved_threads: 2,
            total_threads: 5,
            last_commit_at: Some(ts(15)),
        }
    }

    #[test]
    fn sync_sends_scoped_searches_and_reads_status_only_for_kept_prs() {
        let (_dir, store, account) = open_store();
        let fake = Fake {
            mine: vec![
                pr("N1", "acme", 1, Involvement::AUTHOR),
                pr("N2", "elsewhere", 2, Involvement::AUTHOR),
            ],
            review: Vec::new(),
            refresh: Vec::new(),
            status: vec![status("N1", ChecksState::Failure)],
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        sync_account(&store, &fake, account, SyncKind::Manual, ts(30)).unwrap();
        let sent: Vec<Vec<String>> = fake
            .queries
            .lock()
            .unwrap()
            .iter()
            .map(|query| query.queries.clone())
            .collect();
        assert_eq!(
            sent,
            vec![
                vec![format!("{MINE_QUERY} org:acme")],
                vec![format!("{REVIEW_QUERY} org:acme")],
            ]
        );
        assert_eq!(*fake.status_asked.lock().unwrap(), vec!["N1".to_owned()]);
        let rows = store.list_pull_requests_for_account(account).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.checks, ChecksState::Failure);
        assert_eq!(row.mergeable, Mergeable::Conflicting);
        assert_eq!(row.merge_state, MergeStateStatus::Dirty);
        assert_eq!((row.unresolved_threads, row.total_threads), (2, 5));
        assert_eq!(row.last_activity_at, ts(15));
    }

    #[test]
    fn a_pr_without_a_status_answer_keeps_its_stored_status() {
        let (_dir, store, account) = open_store();
        let mut first = Fake {
            mine: vec![pr("N1", "acme", 1, Involvement::AUTHOR)],
            review: Vec::new(),
            refresh: Vec::new(),
            status: vec![status("N1", ChecksState::Pending)],
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        sync_account(&store, &first, account, SyncKind::Manual, ts(30)).unwrap();
        first.status.clear();
        sync_account(&store, &first, account, SyncKind::Manual, ts(40)).unwrap();
        let row = &store.list_pull_requests_for_account(account).unwrap()[0];
        assert_eq!(row.checks, ChecksState::Pending);
        assert_eq!(row.merge_state, MergeStateStatus::Dirty);
        assert_eq!(row.unresolved_threads, 2);
    }

    #[test]
    fn untracked_open_rows_are_not_refreshed() {
        let (_dir, store, account) = open_store();
        let live = pr("N1", "acme", 1, Involvement::AUTHOR);
        let fake = Fake {
            mine: vec![live.clone()],
            review: Vec::new(),
            refresh: Vec::new(),
            status: Vec::new(),
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        };
        sync_account(&store, &fake, account, SyncKind::Manual, ts(30)).unwrap();
        let repo = store
            .upsert_repository(Repository {
                id: 0,
                account_id: account,
                owner: "elsewhere".into(),
                name: "widgets".into(),
                node_id: None,
                default_branch: None,
                archived: false,
            })
            .unwrap();
        let mut stale = live.pull_request.clone();
        stale.remote.id = "N7".into();
        stale.remote.account = account;
        stale.number = 7;
        stale.repo.owner = "elsewhere".into();
        store.upsert_pull_request(repo.id, stale).unwrap();
        let report = sync_account(&store, &fake, account, SyncKind::Manual, ts(40)).unwrap();
        assert_eq!(report.refreshed, 0);
        let rows = store.list_pull_requests_for_account(account).unwrap();
        assert!(rows.iter().find(|row| row.number == 7).unwrap().hidden);
    }

    fn fake(mine: Vec<SyncedPullRequest>, status: Vec<PrStatus>) -> Fake {
        Fake {
            mine,
            review: Vec::new(),
            refresh: Vec::new(),
            status,
            budget: sample_budget(5000, 4000),
            queries: Default::default(),
            status_asked: Default::default(),
            fail_status: false,
            refresh_asked: Default::default(),
        }
    }

    fn live_status(id: &str, checks: ChecksState, updated_at: DateTime<Utc>) -> PrStatus {
        PrStatus {
            state: Some(PrState::Open),
            updated_at: Some(updated_at),
            mergeable: Mergeable::Mergeable,
            merge_state: MergeStateStatus::Clean,
            review_decision: None,
            unresolved_threads: 0,
            total_threads: 0,
            last_commit_at: None,
            ..status(id, checks)
        }
    }

    fn row(store: &Store, account: AccountId, number: u64) -> PullRequest {
        store
            .list_pull_requests_for_account(account)
            .unwrap()
            .into_iter()
            .find(|pr| pr.number == number)
            .unwrap()
    }

    #[test]
    fn plan_picks_full_for_each_trigger_and_incremental_otherwise() {
        let key = "ada|false|acme";
        let mark = SyncMark {
            scope: key.into(),
            updated_at: ts(10),
            full_at: ts(0),
            rollup_at: ts(0),
        };
        assert_eq!(
            plan_sync(SyncKind::Full, Some(&mark), key, ts(1)),
            SyncPlan::Full(FullReason::Requested)
        );
        assert_eq!(
            plan_sync(SyncKind::Manual, None, key, ts(1)),
            SyncPlan::Full(FullReason::NoMark)
        );
        assert_eq!(
            plan_sync(
                SyncKind::Scheduled,
                Some(&mark),
                "ada|false|acme,beta",
                ts(1)
            ),
            SyncPlan::Full(FullReason::ScopeChanged)
        );
        assert_eq!(
            plan_sync(
                SyncKind::Scheduled,
                Some(&mark),
                key,
                ts(FULL_EVERY_HOURS * 60)
            ),
            SyncPlan::Full(FullReason::StaleMark)
        );
        assert_eq!(
            plan_sync(SyncKind::Manual, Some(&mark), key, ts(1)),
            SyncPlan::Incremental {
                since: ts(10 - MARK_OVERLAP_MINUTES),
                rollup: false
            }
        );
        assert_eq!(
            plan_sync(
                SyncKind::Scheduled,
                Some(&mark),
                key,
                ts(ROLLUP_EVERY_MINUTES)
            ),
            SyncPlan::Incremental {
                since: ts(10 - MARK_OVERLAP_MINUTES),
                rollup: true
            }
        );
    }

    #[test]
    fn scope_key_ignores_org_order_and_case() {
        let a = scope(&["Acme", "beta"], false);
        let b = scope(&["beta", "acme", "ACME"], false);
        assert_eq!(scope_key(&a), scope_key(&b));
        assert_ne!(scope_key(&a), scope_key(&scope(&["acme", "beta"], true)));
    }

    #[test]
    fn incremental_search_drops_is_open_and_carries_the_overlap() {
        let (_dir, store, account) = open_store();
        let mut newest = pr("N2", "acme", 2, Involvement::AUTHOR);
        newest.pull_request.updated_at = ts(25);
        let first = fake(
            vec![pr("N1", "acme", 1, Involvement::AUTHOR), newest],
            Vec::new(),
        );
        let report = sync_account(&store, &first, account, SyncKind::Full, ts(30)).unwrap();
        assert!(report.full);
        let mark = settings::sync_mark(&store).unwrap().unwrap();
        assert_eq!(mark.updated_at, ts(25));
        assert_eq!(mark.full_at, ts(30));

        let second = fake(Vec::new(), Vec::new());
        let report = sync_account(&store, &second, account, SyncKind::Scheduled, ts(40)).unwrap();
        assert!(!report.full);
        assert_eq!(report.message, "0 updated");
        let queries: Vec<String> = second
            .queries
            .lock()
            .unwrap()
            .iter()
            .flat_map(|query| query.queries.clone())
            .collect();
        assert_eq!(
            queries,
            vec![
                "is:pr author:@me archived:false sort:updated-desc \
                 updated:>=2023-11-14T22:33:20Z org:acme"
                    .to_owned(),
                "is:pr review-requested:@me archived:false sort:updated-desc \
                 updated:>=2023-11-14T22:33:20Z org:acme"
                    .to_owned(),
            ]
        );
        // ts(25) minus the 5-minute overlap.
        assert_eq!(
            ts(25 - MARK_OVERLAP_MINUTES)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
            "2023-11-14T22:33:20Z"
        );
        // Nothing moving, no rollup due: no status read at all.
        assert!(second.status_asked.lock().unwrap().is_empty());
    }

    #[test]
    fn incremental_queries_respect_the_length_cap() {
        let orgs: Vec<String> = (0..40).map(|n| format!("org-number-{n}")).collect();
        let names: Vec<&str> = orgs.iter().map(String::as_str).collect();
        let queries = incremental_queries(SearchBucket::Mine, &scope(&names, false), ts(0));
        assert!(queries.len() > 1);
        assert!(queries.iter().all(|query| query.len() <= SEARCH_QUERY_MAX));
        assert!(queries.iter().all(|query| query.contains(" updated:>=")));
        assert!(queries.iter().all(|query| !query.contains("is:open")));
    }

    #[test]
    fn an_incremental_hit_that_merged_moves_to_closed_without_a_refresh() {
        let (_dir, store, account) = open_store();
        let first = fake(
            vec![
                pr("N1", "acme", 1, Involvement::AUTHOR),
                pr("N2", "acme", 2, Involvement::AUTHOR),
            ],
            Vec::new(),
        );
        sync_account(&store, &first, account, SyncKind::Full, ts(30)).unwrap();

        let mut merged = pr("N2", "acme", 2, Involvement::AUTHOR);
        merged.pull_request.state = PrState::Merged;
        merged.pull_request.updated_at = ts(35);
        // An old closed pull request with a fresh comment: never stored, so
        // the incremental search must not add it.
        let mut stranger = pr("N3", "acme", 3, Involvement::AUTHOR);
        stranger.pull_request.state = PrState::Closed;
        stranger.pull_request.updated_at = ts(36);
        let second = fake(vec![merged, stranger], Vec::new());
        let report = sync_account(&store, &second, account, SyncKind::Manual, ts(40)).unwrap();
        assert!(!report.full);
        assert_eq!(report.updated, 1);
        assert_eq!(report.refreshed, 0);
        assert!(second.refresh_asked.lock().unwrap().is_empty());
        assert_eq!(row(&store, account, 2).state, PrState::Merged);
        assert_eq!(row(&store, account, 1).state, PrState::Open);
        assert!(
            store
                .list_pull_requests_for_account(account)
                .unwrap()
                .iter()
                .all(|pr| pr.number != 3)
        );
        assert_eq!(
            settings::sync_mark(&store).unwrap().unwrap().updated_at,
            ts(35)
        );
    }

    #[test]
    fn a_rollup_only_change_lands_without_a_search_hit() {
        let (_dir, store, account) = open_store();
        let mut pending = pr("N1", "acme", 1, Involvement::AUTHOR);
        pending.pull_request.checks = ChecksState::Pending;
        let settled = pr("N2", "acme", 2, Involvement::AUTHOR);
        let first = fake(vec![pending, settled], Vec::new());
        sync_account(&store, &first, account, SyncKind::Full, ts(30)).unwrap();

        // CI finished: updatedAt does not move, so search returns nothing.
        let second = fake(
            Vec::new(),
            vec![live_status("N1", ChecksState::Success, ts(10))],
        );
        let report = sync_account(&store, &second, account, SyncKind::Scheduled, ts(40)).unwrap();
        assert_eq!(report.updated, 1);
        assert_eq!(report.refreshed, 0);
        // Only the moving row is read; the settled one waits for the rollup.
        assert_eq!(*second.status_asked.lock().unwrap(), vec!["N1".to_owned()]);
        assert_eq!(row(&store, account, 1).checks, ChecksState::Success);
        assert_eq!(row(&store, account, 1).synced_at, ts(40));
        assert_eq!(row(&store, account, 2).synced_at, ts(30));

        // The periodic rollup reads every open row.
        let third = fake(
            Vec::new(),
            vec![
                live_status("N1", ChecksState::Success, ts(10)),
                live_status("N2", ChecksState::Failure, ts(10)),
            ],
        );
        let report = sync_account(
            &store,
            &third,
            account,
            SyncKind::Scheduled,
            ts(30 + ROLLUP_EVERY_MINUTES),
        )
        .unwrap();
        assert_eq!(report.updated, 1);
        let mut asked = third.status_asked.lock().unwrap().clone();
        asked.sort();
        assert_eq!(asked, vec!["N1".to_owned(), "N2".to_owned()]);
        assert_eq!(row(&store, account, 2).checks, ChecksState::Failure);
        assert_eq!(
            settings::sync_mark(&store).unwrap().unwrap().rollup_at,
            ts(30 + ROLLUP_EVERY_MINUTES)
        );
    }

    #[test]
    fn a_volatile_row_that_finished_or_changed_is_refreshed_in_full() {
        let (_dir, store, account) = open_store();
        let mut pending = pr("N1", "acme", 1, Involvement::AUTHOR);
        pending.pull_request.checks = ChecksState::Pending;
        let mut unknown = pr("N2", "acme", 2, Involvement::REVIEW_REQUESTED);
        unknown.pull_request.mergeable = Mergeable::Unknown;
        let first = fake(vec![pending], Vec::new());
        let first = Fake {
            review: vec![unknown],
            ..first
        };
        sync_account(&store, &first, account, SyncKind::Full, ts(30)).unwrap();

        // N1 merged; N2 left the review-requested view after a review, so no
        // search finds it, but its updatedAt moved.
        let mut merged = pr("N1", "acme", 1, Involvement::empty());
        merged.pull_request.state = PrState::Merged;
        merged.pull_request.updated_at = ts(12);
        let mut reviewed = pr("N2", "acme", 2, Involvement::empty());
        reviewed.pull_request.title = "PR 2, reworked".into();
        reviewed.pull_request.updated_at = ts(14);
        let second = Fake {
            refresh: vec![merged, reviewed],
            ..fake(
                Vec::new(),
                vec![
                    PrStatus {
                        state: Some(PrState::Merged),
                        ..live_status("N1", ChecksState::Success, ts(12))
                    },
                    live_status("N2", ChecksState::Success, ts(14)),
                ],
            )
        };
        let report = sync_account(&store, &second, account, SyncKind::Scheduled, ts(40)).unwrap();
        assert_eq!(report.refreshed, 2);
        assert_eq!(report.updated, 2);
        assert_eq!(row(&store, account, 1).state, PrState::Merged);
        let n2 = row(&store, account, 2);
        assert_eq!(n2.title, "PR 2, reworked");
        // Involvement is kept from the stored row, like a full refresh.
        assert!(n2.involvement.contains(Involvement::REVIEW_REQUESTED));
        assert_eq!(
            settings::sync_mark(&store).unwrap().unwrap().updated_at,
            ts(14)
        );
    }

    #[test]
    fn unchanged_rows_are_not_rewritten_or_reported() {
        let (_dir, store, account) = open_store();
        let rows: Vec<_> = (1..=100)
            .map(|n| pr(&format!("N{n}"), "acme", n, Involvement::AUTHOR))
            .collect();
        let first = fake(rows.clone(), Vec::new());
        let report = sync_account(&store, &first, account, SyncKind::Full, ts(30)).unwrap();
        assert_eq!(report.updated, 100);

        // A full reconcile that finds the same 100 rows writes none of them.
        let again = fake(rows.clone(), Vec::new());
        let report = sync_account(&store, &again, account, SyncKind::Full, ts(40)).unwrap();
        assert_eq!(report.updated, 0);
        assert!(
            store
                .list_pull_requests_for_account(account)
                .unwrap()
                .iter()
                .all(|pr| pr.synced_at == ts(30))
        );

        // The overlap returns the newest rows again; still nothing written.
        let overlap = fake(rows[..3].to_vec(), Vec::new());
        let mut updates = Vec::new();
        let report = sync_account_with_progress(
            &store,
            &overlap,
            account,
            SyncKind::Manual,
            ts(50),
            &mut |progress| updates.push(progress.updated),
        )
        .unwrap();
        assert!(!report.full);
        assert_eq!(report.updated, 0);
        assert!(updates.iter().all(|&count| count == 0));
        assert!(
            store
                .list_pull_requests_for_account(account)
                .unwrap()
                .iter()
                .all(|pr| pr.synced_at == ts(30))
        );
    }

    #[test]
    fn changing_the_tracked_orgs_forces_a_full_reconcile() {
        let (_dir, store, account) = open_store();
        let first = fake(vec![pr("N1", "acme", 1, Involvement::AUTHOR)], Vec::new());
        sync_account(&store, &first, account, SyncKind::Full, ts(30)).unwrap();
        settings::set_tracked_orgs(&store, &["acme".into(), "beta".into()]).unwrap();
        let second = fake(vec![pr("N1", "acme", 1, Involvement::AUTHOR)], Vec::new());
        let report = sync_account(&store, &second, account, SyncKind::Scheduled, ts(40)).unwrap();
        assert!(report.full);
        assert!(
            second
                .queries
                .lock()
                .unwrap()
                .iter()
                .all(|query| query.queries.iter().all(|q| q.contains("is:open")))
        );
        // The next pass is incremental again.
        let third = fake(Vec::new(), Vec::new());
        assert!(
            !sync_account(&store, &third, account, SyncKind::Scheduled, ts(50))
                .unwrap()
                .full
        );
    }

    #[test]
    fn a_stale_mark_forces_a_full_reconcile() {
        let (_dir, store, account) = open_store();
        let first = fake(vec![pr("N1", "acme", 1, Involvement::AUTHOR)], Vec::new());
        sync_account(&store, &first, account, SyncKind::Full, ts(30)).unwrap();
        let later = ts(30 + FULL_EVERY_HOURS * 60);
        let second = fake(vec![pr("N1", "acme", 1, Involvement::AUTHOR)], Vec::new());
        let report = sync_account(&store, &second, account, SyncKind::Scheduled, later).unwrap();
        assert!(report.full);
        assert_eq!(settings::sync_mark(&store).unwrap().unwrap().full_at, later);
    }

    #[test]
    fn a_missing_mark_forces_a_full_reconcile() {
        let (_dir, store, account) = open_store();
        let first = fake(vec![pr("N1", "acme", 1, Involvement::AUTHOR)], Vec::new());
        let report = sync_account(&store, &first, account, SyncKind::Manual, ts(30)).unwrap();
        assert!(report.full);
        assert_eq!(report.message, "1 pull requests");
    }

    #[test]
    fn the_mark_moves_only_after_a_pass_that_succeeded() {
        let (_dir, store, account) = open_store();
        // A first pass that fails leaves no mark.
        let failing = Fake {
            fail_status: true,
            ..fake(vec![pr("N1", "acme", 1, Involvement::AUTHOR)], Vec::new())
        };
        assert!(sync_account(&store, &failing, account, SyncKind::Full, ts(20)).is_err());
        assert_eq!(settings::sync_mark(&store).unwrap(), None);

        let first = fake(vec![pr("N1", "acme", 1, Involvement::AUTHOR)], Vec::new());
        sync_account(&store, &first, account, SyncKind::Full, ts(30)).unwrap();
        let before = settings::sync_mark(&store).unwrap().unwrap();

        let mut newer = pr("N2", "acme", 2, Involvement::AUTHOR);
        newer.pull_request.updated_at = ts(38);
        let failing = Fake {
            fail_status: true,
            ..fake(vec![newer.clone()], Vec::new())
        };
        assert!(sync_account(&store, &failing, account, SyncKind::Manual, ts(40)).is_err());
        assert_eq!(settings::sync_mark(&store).unwrap().unwrap(), before);

        // The retry searches from the old mark and finds the same row.
        let retry = fake(vec![newer], Vec::new());
        sync_account(&store, &retry, account, SyncKind::Manual, ts(41)).unwrap();
        assert!(
            retry.queries.lock().unwrap()[0].queries[0].contains(&format!(
                "updated:>={}",
                (before.updated_at - Duration::minutes(MARK_OVERLAP_MINUTES))
                    .format("%Y-%m-%dT%H:%M:%SZ")
            ))
        );
        let after = settings::sync_mark(&store).unwrap().unwrap();
        assert_eq!(after.updated_at, ts(38));
        assert_eq!(after.full_at, before.full_at);
    }

    fn sample_budget(limit: u32, remaining: u32) -> RateBudget {
        RateBudget {
            limit,
            remaining,
            reset_at: ts(90),
            cost: 1,
            secondary_until: None,
        }
    }
}
