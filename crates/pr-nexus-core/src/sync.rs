//! GitHub sync: two search buckets scoped to the tracked orgs, a status pass,
//! a client-side org filter as a safety net, and a nodes refresh.
//!
//! The provider performs HTTP. This module decides what to keep and what to hide.

use chrono::{DateTime, Utc};

use crate::domain::{
    AccountId, AccountKind, CloseReason, PrState, ProviderRef, PullRequest, Repository,
    RequestStatus,
};
use crate::error::CoreError;
use crate::providers::{
    PrQuery, PrStatus, RateBudget, SearchBucket, SyncedPullRequest, VcsProvider,
};
use crate::settings;
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
pub const SEARCH_CAP: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    Scheduled,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub fetched: usize,
    pub stored: usize,
    pub refreshed: usize,
    pub hidden: usize,
    pub paused: bool,
    pub message: String,
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
    let base = search_query(bucket);
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
        return Ok(SyncReport {
            fetched: 0,
            stored: 0,
            refreshed: 0,
            hidden: 0,
            paused: false,
            message: "Choose orgs to track".into(),
        });
    }
    if kind == SyncKind::Scheduled
        && let Some(budget) = settings::rate_budget(store)?
        && scheduled_poll_paused(&budget, now)
    {
        return Ok(SyncReport {
            fetched: 0,
            stored: 0,
            refreshed: 0,
            hidden: 0,
            paused: true,
            message: format!("Rate limited · resets {}", budget.reset_at.format("%H:%M")),
        });
    }

    let mut fetched: Vec<SyncedPullRequest> = Vec::new();
    for bucket in [SearchBucket::Mine, SearchBucket::ReviewRequested] {
        let page = provider
            .search_pull_requests(&PrQuery {
                bucket,
                queries: search_queries(bucket, &scope),
            })
            .map_err(CoreError::from)?;
        merge_bucket(&mut fetched, page);
    }

    // Search is scoped on GitHub's side; this filter is the safety net.
    let mut seen_ids = Vec::new();
    let mut seen_orgs = settings::seen_orgs(store)?;
    let mut kept: Vec<SyncedPullRequest> = Vec::new();
    for item in fetched.iter() {
        let owner = &item.pull_request.repo.owner;
        remember_org(&mut seen_orgs, owner, &scope.viewer_login);
        seen_ids.push(item.pull_request.remote.id.clone());
        if in_scope(owner, &scope) {
            kept.push(item.clone());
        }
    }
    settings::set_seen_orgs(store, &seen_orgs)?;

    let existing = store.list_pull_requests_for_account(account_id)?;
    let ids: Vec<ProviderRef> = kept
        .iter()
        .map(|item| item.pull_request.remote.clone())
        .collect();
    let statuses = if ids.is_empty() {
        Vec::new()
    } else {
        provider
            .pull_request_status(&ids)
            .map_err(CoreError::from)?
    };
    let mut stored = 0usize;
    for item in &mut kept {
        let id = &item.pull_request.remote.id;
        let status = statuses
            .iter()
            .find(|status| &status.remote_id == id)
            .cloned()
            .or_else(|| {
                existing
                    .iter()
                    .find(|pr| &pr.remote.id == id)
                    .map(PrStatus::of)
            });
        if let Some(status) = status {
            status.apply_to(&mut item.pull_request);
        }
        upsert_synced(store, account_id, item, now)?;
        stored += 1;
    }
    if let Ok(budget) = provider.rate_limit() {
        settings::set_rate_budget(store, &budget)?;
    }

    let existing = store.list_pull_requests_for_account(account_id)?;
    let missing: Vec<ProviderRef> = existing
        .iter()
        .filter(|pr| {
            pr.state == PrState::Open
                && in_scope(&pr.repo.owner, &scope)
                && !seen_ids.iter().any(|id| id == &pr.remote.id)
        })
        .map(|pr| pr.remote.clone())
        .collect();
    let mut refreshed_rows = if missing.is_empty() {
        Vec::new()
    } else {
        provider
            .refresh_pull_requests(&missing)
            .map_err(CoreError::from)?
    };
    for item in &mut refreshed_rows {
        if let Some(prev) = existing
            .iter()
            .find(|pr| pr.remote.id == item.pull_request.remote.id)
        {
            item.pull_request.involvement = prev.involvement;
        }
    }
    let refreshed = refreshed_rows.len();
    for item in &refreshed_rows {
        seen_ids.push(item.pull_request.remote.id.clone());
        if in_scope(&item.pull_request.repo.owner, &scope) {
            upsert_synced(store, account_id, item, now)?;
        }
    }

    let keep_days = settings::keep_closed_days(store)?;
    let existing = store.list_pull_requests_for_account(account_id)?;
    let mut hidden = 0usize;
    for pr in existing {
        close_requests_of_finished(store, &pr, now)?;
        let out_of_scope = !in_scope(&pr.repo.owner, &scope);
        let expired = should_hide_closed(pr.state, pr.updated_at, now, keep_days);
        let next = out_of_scope || expired;
        if next != pr.hidden {
            store.set_pull_request_hidden(pr.id, next)?;
            if next {
                hidden += 1;
            }
        }
    }

    Ok(SyncReport {
        fetched: fetched.len(),
        stored,
        refreshed,
        hidden,
        paused: false,
        message: format!("{stored} pull requests"),
    })
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

fn upsert_synced(
    store: &Store,
    account_id: AccountId,
    item: &SyncedPullRequest,
    now: DateTime<Utc>,
) -> Result<PullRequest, CoreError> {
    let repo = store.upsert_repository(Repository {
        id: 0,
        account_id,
        owner: item.pull_request.repo.owner.clone(),
        name: item.pull_request.repo.name.clone(),
        node_id: item.repo_node_id.clone(),
        default_branch: item.default_branch.clone(),
        archived: item.repo_archived,
    })?;
    let mut pr = item.pull_request.clone();
    pr.remote.account = account_id;
    pr.synced_at = now;
    pr.hidden = false;
    store.upsert_pull_request(repo.id, pr)
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
        fn refresh_pull_requests(
            &self,
            ids: &[ProviderRef],
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
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
        };
        sync_account(&store, &fake, account, SyncKind::Manual, ts(30)).unwrap();
        // Seed an open row that search no longer returns, then sync again.
        let repo_id = store.list_repositories(account).unwrap()[0].id;
        let mut missing = live.pull_request.clone();
        missing.remote.id = "N9".into();
        missing.number = 9;
        missing.remote.account = account;
        store.upsert_pull_request(repo_id, missing).unwrap();

        let report = sync_account(&store, &fake, account, SyncKind::Manual, ts(30)).unwrap();
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
        };
        sync_account(&store, &first, account, SyncKind::Manual, ts(20)).unwrap();
        let stored = store.list_pull_requests_for_account(account).unwrap();
        let tracked = stored.iter().find(|row| row.number == 2).unwrap();
        assert_eq!(tracked.state, PrState::Open);
        let request = store
            .insert_review_request(crate::domain::ReviewRequest {
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
        };
        let report = sync_account(&store, &second, account, SyncKind::Manual, ts(70)).unwrap();
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
