use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, TryLockError};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use pr_nexus_core::domain::{Account, AccountKind, ChecksState};
use pr_nexus_core::onboarding::{self, OnboardingState, StepValidity};
use pr_nexus_core::providers::{ChatProvider, SearchBucket};
use pr_nexus_core::providers::{ProviderRegistry, SetupStep};
use pr_nexus_core::review_filter::RemoteReviewFilter;
use pr_nexus_core::settings;
use pr_nexus_core::store::Store;
use pr_nexus_core::sync::{self, SyncKind};
use pr_nexus_providers::github::{self, GithubClient};
use pr_nexus_providers::slack_mcp::CancelHandle;
use pr_nexus_providers::slack_mcp::TokenState;
use pr_nexus_providers::vault::Vault;
use tauri::Manager;

use crate::error::AppError;

pub struct SyncStatus {
    pub phase: String,
    pub message: String,
    pub synced_at: Option<DateTime<Utc>>,
    pub paused: bool,
    /// Goes up whenever sync changed stored rows, so the list re-reads only
    /// then (§6).
    pub revision: u64,
    /// The lazy review-request sync of this session (§6).
    pub reviews: ReviewStatus,
}

/// Review requests sync only once the To review tab (or an MCP tool) asks
/// for them, and only the newest [`sync::REVIEW_WINDOW`] at first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewStatus {
    /// `off`, `loading`, `ready` or `error`.
    pub state: String,
    /// Progress while loading ("Checking reviews… 20/100"), else the last
    /// outcome.
    pub message: String,
    /// Open review requests GitHub reports in all, when known.
    pub total: Option<u64>,
    /// How many of the newest the window reads.
    pub window: usize,
    /// GitHub-side To review filters asked for this session (§6).
    pub filters: FilterCache,
}

/// How long a filtered query's answer is reused before GitHub is asked again.
pub const FILTER_FRESH_SECS: i64 = 120;

/// Answers of filtered review queries, per [`RemoteReviewFilter::key`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterCache {
    /// The filter the list shows now. A pass for another key still stores
    /// its rows but is not what the header reports.
    pub current: Option<String>,
    pub entries: HashMap<String, FilteredReviews>,
    /// GitHub rate limited a filtered search: filters run locally only
    /// until then.
    pub hold_until: Option<DateTime<Utc>>,
}

/// One filtered review query: its own window and GitHub total, so the
/// header can say "40 of 57 loaded" for the filter rather than for the
/// whole bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredReviews {
    /// `loading`, `ready`, `limited` (rate limited: local only) or `error`.
    pub state: String,
    pub message: String,
    /// Matches on GitHub (`issueCount`), when known.
    pub total: Option<u64>,
    /// Matches read from GitHub by the last pass (at most `window`).
    pub loaded: usize,
    pub window: usize,
    /// When the last pass finished.
    pub at: Option<DateTime<Utc>>,
}

impl FilteredReviews {
    fn fresh(&self, now: DateTime<Utc>) -> bool {
        self.state == "ready"
            && self
                .at
                .is_some_and(|at| now - at < chrono::Duration::seconds(FILTER_FRESH_SECS))
    }
}

impl Default for ReviewStatus {
    fn default() -> Self {
        Self {
            state: "off".into(),
            message: String::new(),
            total: None,
            window: sync::REVIEW_WINDOW,
            filters: FilterCache::default(),
        }
    }
}

impl SyncStatus {
    fn idle() -> Self {
        Self {
            phase: "idle".into(),
            message: "Not synced yet".into(),
            synced_at: None,
            paused: false,
            revision: 0,
            reviews: ReviewStatus::default(),
        }
    }

    fn demo() -> Self {
        Self {
            phase: "demo".into(),
            message: "Demo data · sync disabled".into(),
            synced_at: None,
            paused: false,
            revision: 0,
            reviews: ReviewStatus::default(),
        }
    }
}

pub struct DeviceSession {
    pub device_code: String,
    pub client_id: String,
}

pub struct AppState {
    /// The active profile's store (§8.4).
    pub store: crate::profiles::StoreSlot,
    pub profiles: Arc<crate::profiles::ProfileManager>,
    /// The profile this state serves. The main state follows switches; a
    /// state built for an MCP `profile` argument stays on one.
    pub profile_id: std::sync::RwLock<String>,
    pub vault: Arc<Vault>,
    /// One lazy Slack MCP session per connected workspace account.
    pub chats: Mutex<HashMap<i64, Arc<dyn ChatProvider>>>,
    /// Channel names read from Slack by id, keyed by (team or workspace,
    /// channel id), with when they were read. `None` is a failed lookup,
    /// kept briefly so a dead workspace isn't retried on every preview.
    /// The id is the truth; a stored name is only a fallback.
    pub channel_names: Mutex<HashMap<(String, String), NameEntry>>,
    /// Owners and blocked reason per PR from the last GitHub resolution,
    /// keyed by PR id, for the bulk preview (short TTL, see `services::bulk`).
    pub owners: Mutex<HashMap<i64, OwnersEntry>>,
    /// The Slack sign-in in progress, if any.
    pub slack_cancel: Mutex<Option<CancelHandle>>,
    /// Startup `auth.test` result per Slack account (§7.11).
    pub slack_tokens: Mutex<HashMap<i64, TokenState>>,
    /// Upstream sessions behind the Slack tools of the local MCP server.
    pub slack_proxy: crate::mcp::slack::SlackProxy,
    pub sync: Mutex<SyncStatus>,
    /// Held while a sync runs, so overlapping requests join it (see [`run_sync`]).
    pub sync_gate: Mutex<()>,
    pub device: Mutex<Option<DeviceSession>>,
    pub focused: AtomicBool,
    /// Set once review requests were asked for this session; never stored,
    /// so each launch syncs only My PRs until then.
    pub reviews_active: AtomicBool,
    pub wake: Arc<(Mutex<bool>, Condvar)>,
    /// Live updates from the notifications inbox (§6).
    pub live: crate::live::LiveShared,
    /// Behind counts for the Branch card, keyed on head and base SHAs.
    pub branch_cache: pr_nexus_core::branch::BranchCache,
    /// `PR_NEXUS_DEMO=1`: seeded local data only, never talk to GitHub.
    pub demo: bool,
}

impl AppState {
    pub fn open(path: PathBuf) -> Result<Self, AppError> {
        let demo = demo_enabled();
        let profiles = Arc::new(crate::profiles::ProfileManager::open(path)?);
        let active = profiles.registry().active;
        let vault = Vault::open();
        vault.set_profile(&active);
        let state = Self::build(profiles, &active, Arc::new(vault), demo)?;
        if demo {
            crate::demo::seed_if_empty(&state.store)?;
        }
        Ok(state)
    }

    /// A state over `profile`, sharing `profiles`.
    pub(crate) fn build(
        profiles: Arc<crate::profiles::ProfileManager>,
        profile: &str,
        vault: Arc<Vault>,
        demo: bool,
    ) -> Result<Self, AppError> {
        let home = profiles.store(pr_nexus_core::profiles::DEFAULT_ID)?;
        let store = profiles.store(profile)?;
        Ok(Self {
            store: crate::profiles::StoreSlot::new(store, home),
            profiles,
            profile_id: std::sync::RwLock::new(profile.to_owned()),
            vault,
            chats: Mutex::new(HashMap::new()),
            channel_names: Mutex::new(HashMap::new()),
            owners: Mutex::new(HashMap::new()),
            slack_cancel: Mutex::new(None),
            slack_tokens: Mutex::new(HashMap::new()),
            slack_proxy: crate::mcp::slack::SlackProxy::default(),
            sync: Mutex::new(if demo {
                SyncStatus::demo()
            } else {
                SyncStatus::idle()
            }),
            sync_gate: Mutex::new(()),
            device: Mutex::new(None),
            focused: AtomicBool::new(true),
            reviews_active: AtomicBool::new(false),
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            live: crate::live::LiveShared::default(),
            branch_cache: pr_nexus_core::branch::BranchCache::default(),
            demo,
        })
    }

    /// The id of the profile this state serves.
    pub fn profile(&self) -> String {
        self.profile_id
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Makes `id` the active profile: stops the sync in flight, points the
    /// store and the vault at the profile, and drops what belonged to the
    /// previous one (Slack sessions, caches, sign-ins, sync status).
    pub fn switch_profile(&self, id: &str) -> Result<(), AppError> {
        let profile = self.profiles.resolve(id)?;
        let store = self.profiles.store(&profile.id)?;
        // A running sync sees the new generation at its next progress
        // report and unwinds; holding the gate keeps a new one from
        // starting until the switch is done.
        self.profiles.bump();
        let _gate = self
            .sync_gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.profiles.set_active(&profile.id)?;
        self.store.set(store);
        self.vault.set_profile(&profile.id);
        *self
            .profile_id
            .write()
            .unwrap_or_else(PoisonError::into_inner) = profile.id.clone();
        self.chats
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.slack_tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.slack_proxy.clear();
        if let Some(cancel) = self
            .slack_cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            cancel.cancel();
        }
        *self.device.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.branch_cache.clear();
        self.reviews_active.store(false, Ordering::Relaxed);
        {
            let mut status = lock_status(self);
            let revision = status.revision + 1;
            *status = if self.demo {
                SyncStatus::demo()
            } else {
                SyncStatus::idle()
            };
            status.revision = revision;
        }
        drop(_gate);
        crate::mcp::profiles::forget(&profile.id);
        crate::live::request_sync(self);
        Ok(())
    }
}

#[cfg(test)]
impl AppState {
    /// A state over a fresh database at `path`, with an in-memory vault.
    pub(crate) fn for_tests(path: &std::path::Path, demo: bool) -> Self {
        let profiles = Arc::new(
            crate::profiles::ProfileManager::open(path.to_owned()).expect("test profiles"),
        );
        let mut state = Self::build(
            profiles,
            pr_nexus_core::profiles::DEFAULT_ID,
            Arc::new(Vault::memory("test")),
            demo,
        )
        .expect("test store");
        state.sync = Mutex::new(SyncStatus::demo());
        state
    }
}

pub fn demo_enabled() -> bool {
    std::env::var("PR_NEXUS_DEMO").ok().as_deref() == Some("1")
}

pub fn database_path(app: &tauri::App) -> PathBuf {
    if let Ok(path) = std::env::var("PR_NEXUS_DB_PATH")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("pr-nexus.db")
}

pub fn github_account(store: &Store) -> Result<Option<Account>, AppError> {
    Ok(github_accounts(store)?.into_iter().next())
}

/// Every connected GitHub account. A typed query runs on each of them.
pub fn github_accounts(store: &Store) -> Result<Vec<Account>, AppError> {
    Ok(store
        .list_accounts()?
        .into_iter()
        .filter(|account| account.kind == AccountKind::GitHub)
        .collect())
}

pub fn resolve_github_token(
    state: &AppState,
    account: Option<&Account>,
) -> Result<String, AppError> {
    if let Some(token) = github::github_token_from_env() {
        return Ok(token);
    }
    let Some(account) = account else {
        return Err(AppError::new(
            "unauthorized",
            "Connect GitHub before syncing",
        ));
    };
    match account.auth_mode {
        pr_nexus_core::domain::AuthMode::GhCli => match github::read_gh_token() {
            github::GhProbe::Ready { token } => Ok(token),
            github::GhProbe::Unavailable { reason } => Err(AppError::new("unauthorized", reason)),
        },
        pr_nexus_core::domain::AuthMode::Pat | pr_nexus_core::domain::AuthMode::OauthDevice => {
            let key = account
                .keychain_key
                .clone()
                .unwrap_or_else(|| format!("github:{}", account.external_id));
            state.vault.get(&key)?.ok_or_else(|| {
                AppError::new(
                    "unauthorized",
                    "GitHub token is not in the keychain. Reconnect, or set PR_NEXUS_GITHUB_TOKEN.",
                )
            })
        }
        other => Err(AppError::new(
            "unsupported",
            format!("auth mode {other} is not a GitHub token"),
        )),
    }
}

pub fn validity(state: &AppState, saved: &OnboardingState) -> Result<StepValidity, AppError> {
    let (vcs, chat) = settings::provider_kinds(&state.store)?;
    let account = github_account(&state.store)?;
    let orgs = settings::tracked_orgs(&state.store)?;
    let personal = settings::include_personal_repos(&state.store)?;
    // After setup the token is checked by each sync, which reports a lost
    // one; reading it here would run `gh` on every launch.
    let github_verified = account.is_some()
        && (saved.finished_at.is_some() || resolve_github_token(state, account.as_ref()).is_ok());
    let slack_completed = saved
        .completed
        .iter()
        .any(|step| step == SetupStep::ConnectChat.as_str());
    Ok(StepValidity {
        vcs_selected: vcs.as_deref() == Some("github"),
        chat_selected: chat.as_deref() == Some("slack"),
        github_verified,
        orgs_chosen: !orgs.is_empty() || personal,
        slack_ok: saved.slack_skipped
            || slack_completed
            || !crate::slack::workspace_accounts(&state.store)?.is_empty(),
        mapping_ok: saved.mapping_skipped
            || saved.completed.iter().any(|step| step == "chat_mapping"),
    })
}

/// Runs one sync, or joins the one already in flight. The scheduler thread,
/// `sync_now` and `finish_onboarding` can all ask at once (the startup sync
/// and the wizard's Done step do); only one talks to GitHub at a time.
/// `Ok(None)` means another sync was already running: this call waited for
/// it to end and did not start a second one. `state.sync` holds its result.
pub fn run_sync(state: &AppState, kind: SyncKind) -> Result<Option<sync::SyncReport>, AppError> {
    let _running = match state.sync_gate.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => {
            drop(
                state
                    .sync_gate
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            );
            return Ok(None);
        }
    };
    sync_once(state, kind).map(Some)
}

/// The sync status while a sync runs: during the search a full reconcile
/// counts what it has seen, an incremental pass what changed; after it, the
/// status pass counts the pull requests it has checked.
pub(crate) fn progress_message(progress: sync::SyncProgress) -> String {
    let what = match progress.bucket {
        SearchBucket::Mine => "my PRs",
        SearchBucket::ReviewRequested => "reviews",
    };
    match progress.stage {
        sync::SyncStage::Checking { done, total } => format!("Checking {what}… {done}/{total}"),
        sync::SyncStage::Rechecking { total } => format!("Rechecking stale PRs… {total}"),
        sync::SyncStage::Searching => match (progress.full, progress.updated) {
            (true, _) => format!("Syncing {what}… {}", progress.seen),
            (false, 0) => format!("Syncing {what}…"),
            (false, updated) => format!("Syncing {what}… {updated} updated"),
        },
    }
}

/// Marks the sync running, runs `work`, and always leaves the "syncing"
/// phase afterwards: a panic inside `work` used to skip the final update,
/// so the sidebar said "Syncing…" for good.
fn settle(
    state: &AppState,
    kind: SyncKind,
    work: impl FnOnce() -> Result<sync::SyncReport, AppError>,
) -> Result<sync::SyncReport, AppError> {
    {
        let mut status = lock_status(state);
        status.phase = "syncing".into();
        status.message = "Syncing…".into();
        status.paused = false;
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|payload| Err(unwound(payload.as_ref())));
    let mut status = lock_status(state);
    match &outcome {
        Ok(report) if report.paused => {
            status.phase = "paused".into();
            status.message = report.message.clone();
            status.paused = true;
            status.synced_at = Some(Utc::now());
        }
        Ok(report) => {
            tracing::info!(kind = ?kind, message = %report.message, "sync done");
            status.phase = "ok".into();
            status.message = if report.message == "Choose orgs to track" {
                report.message.clone()
            } else {
                format!("Synced · {}", report.message)
            };
            status.paused = false;
            status.synced_at = Some(Utc::now());
        }
        Err(err) => {
            tracing::warn!(kind = ?kind, error = %err, "sync failed");
            status.phase = if err.kind == "rate_limited" {
                "paused"
            } else {
                "error"
            }
            .into();
            status.message = if err.kind == "rate_limited" {
                "Rate limited".into()
            } else {
                format!("Sync failed · {}", err.message)
            };
            status.paused = err.kind == "rate_limited";
        }
    }
    outcome
}

/// The error for a sync that unwound: a profile switch, or a panic.
fn unwound(payload: &(dyn std::any::Any + Send)) -> AppError {
    if payload.is::<crate::profiles::Cancelled>() {
        AppError::new("cancelled", "Sync stopped: switched profile")
    } else {
        AppError::new("internal", "sync stopped unexpectedly")
    }
}

/// The sync status, even after a panic poisoned its lock.
fn lock_status(state: &AppState) -> std::sync::MutexGuard<'_, SyncStatus> {
    state.sync.lock().unwrap_or_else(PoisonError::into_inner)
}

fn sync_once(state: &AppState, kind: SyncKind) -> Result<sync::SyncReport, AppError> {
    if state.demo {
        *state.sync.lock().expect("sync status") = SyncStatus::demo();
        return Ok(sync::SyncReport {
            message: "Demo data · sync disabled".into(),
            ..sync::SyncReport::default()
        });
    }
    // My PRs first, fully: search, status pass and code owners. The sync
    // phase ends with them, so the header stops saying "Syncing".
    let report = settle(state, kind, || {
        let client = github_client(state)?;
        let mut on_progress = progress_writer(state, |status, message| status.message = message);
        let report = sync::sync_buckets(
            &state.store,
            &client.1,
            client.0,
            kind,
            Utc::now(),
            &[sync::BucketPlan::mine()],
            &mut on_progress,
        )?;
        after_pass(state, &client.1, client.0, &report, SearchBucket::Mine)?;
        Ok::<_, AppError>(report)
    })?;
    // Then review requests, once this session asked for them (§6). The
    // list stays usable: their progress shows in the To review tab only.
    if !report.paused && state.reviews_active.load(Ordering::Relaxed) {
        let (window, known) = {
            let status = lock_status(state);
            (status.reviews.window, status.reviews.total.is_some())
        };
        // The first pass of a session reads the window in full, for its
        // total; later ones only what changed.
        let kind = if known { kind } else { SyncKind::Full };
        let _ = review_pass(state, kind, sync::BucketPlan::reviews(Some(window)));
    }
    Ok(report)
}

/// The GitHub account id and a client for it.
fn github_client(
    state: &AppState,
) -> Result<(pr_nexus_core::domain::AccountId, GithubClient), AppError> {
    let account = github_account(&state.store)?
        .ok_or_else(|| AppError::new("unauthorized", "Connect GitHub before syncing"))?;
    let token = resolve_github_token(state, Some(&account))?;
    Ok((account.id, GithubClient::new(account.id, token)?))
}

/// A progress callback that writes [`progress_message`] through `show`, at
/// most every `PROGRESS_EVERY` but at once on a new stage, and bumps the
/// revision when rows changed so the list re-reads them as they arrive.
fn progress_writer<'a>(
    state: &'a AppState,
    show: impl Fn(&mut SyncStatus, String) + 'a,
) -> impl FnMut(sync::SyncProgress) + 'a {
    let mut throttle = sync::Throttle::new(sync::PROGRESS_EVERY);
    let mut shown = 0usize;
    let mut stage = None;
    let generation = state.profiles.generation();
    move |progress| {
        if state.profiles.generation() != generation {
            // A profile switch: stop here. `settle` and `review_pass`
            // catch the unwind; `resume_unwind` skips the panic hook.
            std::panic::resume_unwind(Box::new(crate::profiles::Cancelled));
        }
        let key = (progress.bucket, std::mem::discriminant(&progress.stage));
        let moved = stage != Some(key);
        stage = Some(key);
        if throttle.ready(std::time::Instant::now()) || moved {
            let mut status = lock_status(state);
            show(&mut status, progress_message(progress));
            if progress.updated != shown || moved {
                shown = progress.updated;
                status.revision += 1;
            }
        }
    }
}

/// Auto-link rules and code-owner status after a pass over `bucket`.
fn after_pass(
    state: &AppState,
    client: &GithubClient,
    account: pr_nexus_core::domain::AccountId,
    report: &sync::SyncReport,
    bucket: SearchBucket,
) -> Result<(), AppError> {
    // §8.1: re-evaluate auto-link rules after every sync.
    let prs = state.store.list_pull_requests_for_account(account)?;
    let links = pr_nexus_core::linking::relink(&state.store, &prs)?;
    let mut changed = report.updated > 0 || links.added + links.removed > 0;
    // §7.1: a BLOCKED pull request with no review decision needs its
    // code-owner status before the list can call it Ready. Only the rows
    // of the bucket just synced, so authored ones never wait on reviews.
    if !report.paused {
        let owned: Vec<_> = prs
            .into_iter()
            .filter(|pr| sync::owned_by(pr, bucket))
            .collect();
        let flag = settings::blocked_review_required_is_ready(&state.store)?;
        let resolved = pr_nexus_core::ownership::resolve_unknown_code_owner_reviews(
            &state.store,
            client,
            &owned,
            flag,
            Utc::now(),
            pr_nexus_core::ownership::MAX_RESOLVES_PER_SYNC,
        )
        .unwrap_or(0);
        changed |= resolved > 0;
    }
    if changed {
        lock_status(state).revision += 1;
    }
    Ok(())
}

/// One pass over review requests: the window, or an on-demand search.
/// Never touches the sync phase, so My PRs and the header stay as they are.
fn review_pass(
    state: &AppState,
    kind: SyncKind,
    plan: sync::BucketPlan,
) -> Result<sync::SyncReport, AppError> {
    let searching = plan.terms.is_some();
    {
        let mut status = lock_status(state);
        status.reviews.state = "loading".into();
        status.reviews.message = "Loading reviews…".into();
        status.revision += 1;
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let client = github_client(state)?;
        let mut on_progress = progress_writer(state, |status, message| {
            status.reviews.message = message.replace("Syncing reviews", "Loading reviews");
        });
        let report = sync::sync_buckets(
            &state.store,
            &client.1,
            client.0,
            kind,
            Utc::now(),
            std::slice::from_ref(&plan),
            &mut on_progress,
        )?;
        after_pass(
            state,
            &client.1,
            client.0,
            &report,
            SearchBucket::ReviewRequested,
        )?;
        Ok::<_, AppError>(report)
    }))
    .unwrap_or_else(|_| {
        Err(AppError::new(
            "internal",
            "review sync stopped unexpectedly",
        ))
    });
    let mut status = lock_status(state);
    match &outcome {
        Ok(report) => {
            status.reviews.state = "ready".into();
            status.reviews.message = report.message.clone();
            // The window's issueCount is the open total; an incremental pass
            // or a search counts something else.
            if report.full && !searching {
                status.reviews.total = report.review_total.or(status.reviews.total);
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "review sync failed");
            status.reviews.state = "error".into();
            status.reviews.message = format!("Review sync failed · {}", err.message);
        }
    }
    status.revision += 1;
    outcome
}

/// Starts the review-request sync of this session, once. Later calls do
/// nothing; the scheduler then includes reviews after My PRs.
pub fn activate_reviews(state: &AppState) -> bool {
    if state.demo || state.reviews_active.swap(true, Ordering::Relaxed) {
        return false;
    }
    lock_status(state).reviews.state = "loading".into();
    true
}

/// Runs the first review pass (the newest window, in full) behind the sync
/// gate, so it never overlaps a My PRs pass.
pub fn run_reviews(state: &AppState) -> Result<(), AppError> {
    let _running = state
        .sync_gate
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let window = lock_status(state).reviews.window;
    review_pass(
        state,
        SyncKind::Full,
        sync::BucketPlan::reviews(Some(window)),
    )
    .map(|_| ())
}

/// Widens the window by one more [`sync::REVIEW_WINDOW`] and reads it.
pub fn load_more_reviews(state: &AppState) -> Result<(), AppError> {
    {
        let mut status = lock_status(state);
        let known = status
            .reviews
            .total
            .map_or(usize::MAX, |total| total as usize);
        if status.reviews.window >= known {
            return Ok(());
        }
        status.reviews.window += sync::REVIEW_WINDOW;
    }
    state.reviews_active.store(true, Ordering::Relaxed);
    run_reviews(state)
}

/// An on-demand review search (the To review search box): stores and
/// checks the matches, leaving the window and marks alone.
pub fn search_reviews(state: &AppState, terms: &str) -> Result<(), AppError> {
    let terms = terms.trim();
    if terms.is_empty() || state.demo {
        return Ok(());
    }
    let _running = state
        .sync_gate
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    review_pass(
        state,
        SyncKind::Manual,
        sync::BucketPlan::review_search(terms),
    )
    .map(|_| ())
}

/// What [`filter_reviews`] should do for one request.
#[derive(Debug, PartialEq, Eq)]
enum FilterStep {
    /// Nothing to ask GitHub: no filter, a fresh cached answer, a window
    /// that already holds every match, or a rate-limit hold.
    Done,
    Search(usize),
}

/// Records `key` as the current filter and decides whether GitHub has to
/// be asked, and for how wide a window. Pure over the cache, so it is
/// tested without a network.
fn plan_filter(
    cache: &mut FilterCache,
    key: Option<&str>,
    more: bool,
    now: DateTime<Utc>,
) -> FilterStep {
    cache.current = key.map(str::to_owned);
    let Some(key) = key else {
        return FilterStep::Done;
    };
    if let Some(until) = cache.hold_until {
        if now < until {
            let entry = cache
                .entries
                .entry(key.to_owned())
                .or_insert_with(|| FilteredReviews {
                    state: String::new(),
                    message: String::new(),
                    total: None,
                    loaded: 0,
                    window: sync::REVIEW_WINDOW,
                    at: None,
                });
            if !entry.fresh(now) {
                entry.state = "limited".into();
                entry.message = "GitHub rate limit · filtering loaded reviews only".into();
            }
            return FilterStep::Done;
        }
        cache.hold_until = None;
    }
    let entry = cache
        .entries
        .entry(key.to_owned())
        .or_insert_with(|| FilteredReviews {
            state: "loading".into(),
            message: String::new(),
            total: None,
            loaded: 0,
            window: sync::REVIEW_WINDOW,
            at: None,
        });
    if more {
        if entry
            .total
            .is_some_and(|total| entry.window as u64 >= total)
        {
            return FilterStep::Done;
        }
        if entry.at.is_some() {
            entry.window += sync::REVIEW_WINDOW;
        }
    } else if entry.fresh(now) {
        return FilterStep::Done;
    }
    entry.state = "loading".into();
    entry.message = "Searching GitHub…".into();
    FilterStep::Search(entry.window)
}

/// Writes the outcome of a filtered pass into the cache. A rate limit
/// starts a hold so filters stay local until GitHub's reset.
fn finish_filter(
    cache: &mut FilterCache,
    key: &str,
    outcome: &Result<sync::SyncReport, AppError>,
    now: DateTime<Utc>,
) {
    let Some(entry) = cache.entries.get_mut(key) else {
        return;
    };
    match outcome {
        Ok(report) => {
            entry.state = "ready".into();
            entry.message = String::new();
            entry.total = report.review_total.or(entry.total);
            entry.loaded = report.fetched.min(entry.window);
            entry.at = Some(now);
        }
        Err(err) if err.kind == "rate_limited" => {
            let until = err
                .retry_at
                .as_deref()
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .map_or(now + chrono::Duration::minutes(1), |at| {
                    at.with_timezone(&Utc)
                });
            cache.hold_until = Some(until);
            entry.state = "limited".into();
            entry.message = "GitHub rate limit · filtering loaded reviews only".into();
        }
        Err(err) => {
            entry.state = "error".into();
            entry.message = format!("GitHub filter failed · {}", err.message);
        }
    }
}

/// The GitHub-side part of the To review filters (§6): searches review
/// requests with the filter's qualifiers, newest first, in a window of its
/// own, and stores what it finds. An empty filter just clears the current
/// one. Never fails on GitHub errors: they land in the filter's status and
/// the list keeps filtering what is loaded.
pub fn filter_reviews(
    state: &AppState,
    filter: &RemoteReviewFilter,
    more: bool,
) -> Result<(), AppError> {
    let key = (!filter.is_empty()).then(|| filter.key());
    let step = {
        let mut status = lock_status(state);
        let step = if state.demo {
            status.reviews.filters.current = None;
            FilterStep::Done
        } else {
            plan_filter(
                &mut status.reviews.filters,
                key.as_deref(),
                more,
                Utc::now(),
            )
        };
        status.revision += 1;
        step
    };
    let (FilterStep::Search(window), Some(key)) = (step, key) else {
        return Ok(());
    };
    let _running = state
        .sync_gate
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    // Superseded while waiting for the gate: the newer filter runs instead.
    {
        let mut status = lock_status(state);
        let cache = &mut status.reviews.filters;
        if cache.current.as_deref() != Some(key.as_str()) {
            if let Some(entry) = cache.entries.get_mut(&key) {
                if entry.at.is_some() {
                    entry.state = "ready".into();
                } else {
                    cache.entries.remove(&key);
                }
            }
            return Ok(());
        }
    }
    let plan = sync::BucketPlan::review_filter(filter.clone(), window);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let client = github_client(state)?;
        let report = sync::sync_buckets(
            &state.store,
            &client.1,
            client.0,
            SyncKind::Manual,
            Utc::now(),
            std::slice::from_ref(&plan),
            &mut |_| {},
        )?;
        after_pass(
            state,
            &client.1,
            client.0,
            &report,
            SearchBucket::ReviewRequested,
        )?;
        Ok::<_, AppError>(report)
    }))
    .unwrap_or_else(|_| {
        Err(AppError::new(
            "internal",
            "review filter stopped unexpectedly",
        ))
    });
    if let Err(err) = &outcome {
        tracing::warn!(error = %err, "review filter failed");
    }
    let mut status = lock_status(state);
    finish_filter(&mut status.reviews.filters, &key, &outcome, Utc::now());
    status.revision += 1;
    Ok(())
}

/// Picks the focused or unfocused poll interval (§6). Coming back after more
/// than a minute away also asks for an incremental pass. Never blocks on
/// network work: it only flips flags and wakes the background threads.
pub fn set_focused(state: &AppState, focused: bool) {
    state.focused.store(focused, Ordering::Relaxed);
    if state.demo {
        return;
    }
    if crate::live::note_focus(state, focused, Utc::now()) {
        crate::live::request_sync(state);
    }
}

pub fn spawn_scheduler(state: Arc<AppState>) {
    if state.demo {
        return;
    }
    thread::Builder::new()
        .name("pr-nexus-sync".into())
        .spawn(move || {
            if onboarding::load(&state.store)
                .ok()
                .is_some_and(|saved| saved.is_finished())
            {
                // App start: a full reconcile (§6).
                let _ = run_sync(&state, SyncKind::Full);
            }
            loop {
                let interval = interval_secs(&state);
                let (lock, cv) = &*state.wake;
                let guard = lock.lock().expect("sync wake");
                let (mut guard, wait) = cv
                    .wait_timeout(guard, Duration::from_secs(interval))
                    .expect("sync wake");
                let manual = *guard || !wait.timed_out();
                *guard = false;
                drop(guard);
                let finished = onboarding::load(&state.store)
                    .ok()
                    .is_some_and(|saved| saved.is_finished());
                if !finished {
                    continue;
                }
                let kind = if manual {
                    SyncKind::Manual
                } else {
                    SyncKind::Scheduled
                };
                let _ = run_sync(&state, kind);
            }
        })
        .expect("sync scheduler");
}

fn interval_secs(state: &AppState) -> u64 {
    let focused = state.focused.load(Ordering::Relaxed);
    if focused {
        settings::focused_interval_s(&state.store).unwrap_or(180)
    } else {
        settings::unfocused_interval_s(&state.store).unwrap_or(900)
    }
}

pub fn registry_steps() -> Vec<SetupStep> {
    ProviderRegistry::v1().setup_steps(
        pr_nexus_core::domain::VcsKind::GitHub,
        pr_nexus_core::domain::ChatKind::Slack,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(fetched: usize, total: u64) -> sync::SyncReport {
        sync::SyncReport {
            fetched,
            stored: fetched,
            refreshed: 0,
            hidden: 0,
            updated: 0,
            full: true,
            paused: false,
            message: String::new(),
            review_total: Some(total),
        }
    }

    fn at(minute: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_800_000_000 + minute * 60, 0).unwrap()
    }

    #[test]
    fn a_filter_searches_once_then_reuses_its_fresh_answer() {
        let mut cache = FilterCache::default();
        let step = plan_filter(&mut cache, Some("k"), false, at(0));
        assert_eq!(step, FilterStep::Search(sync::REVIEW_WINDOW));
        assert_eq!(cache.entries["k"].state, "loading");
        finish_filter(&mut cache, "k", &Ok(report(100, 340)), at(0));
        assert_eq!(
            (cache.entries["k"].loaded, cache.entries["k"].total),
            (100, Some(340))
        );
        assert_eq!(
            plan_filter(&mut cache, Some("k"), false, at(1)),
            FilterStep::Done
        );
        // Stale after FILTER_FRESH_SECS: asked again, same window.
        assert_eq!(
            plan_filter(&mut cache, Some("k"), false, at(3)),
            FilterStep::Search(sync::REVIEW_WINDOW)
        );
    }

    #[test]
    fn more_widens_a_filter_window_until_it_holds_every_match() {
        let mut cache = FilterCache::default();
        plan_filter(&mut cache, Some("k"), false, at(0));
        finish_filter(&mut cache, "k", &Ok(report(100, 150)), at(0));
        assert_eq!(
            plan_filter(&mut cache, Some("k"), true, at(0)),
            FilterStep::Search(2 * sync::REVIEW_WINDOW)
        );
        finish_filter(&mut cache, "k", &Ok(report(150, 150)), at(0));
        assert_eq!(cache.entries["k"].loaded, 150);
        assert_eq!(
            plan_filter(&mut cache, Some("k"), true, at(0)),
            FilterStep::Done
        );
    }

    #[test]
    fn a_rate_limit_holds_filters_local_until_the_reset() {
        let mut cache = FilterCache::default();
        plan_filter(&mut cache, Some("k"), false, at(0));
        let limited = AppError {
            kind: "rate_limited".into(),
            message: "GitHub rate limit".into(),
            retry_at: Some(at(10).to_rfc3339()),
            sso_url: None,
        };
        finish_filter(&mut cache, "k", &Err(limited), at(0));
        assert_eq!(cache.entries["k"].state, "limited");
        assert_eq!(
            plan_filter(&mut cache, Some("other"), false, at(5)),
            FilterStep::Done
        );
        assert_eq!(cache.entries["other"].state, "limited");
        assert_eq!(
            plan_filter(&mut cache, Some("other"), false, at(11)),
            FilterStep::Search(sync::REVIEW_WINDOW)
        );
        assert_eq!(cache.hold_until, None);
    }

    #[test]
    fn clearing_the_filter_forgets_the_current_key_but_keeps_the_cache() {
        let mut cache = FilterCache::default();
        plan_filter(&mut cache, Some("k"), false, at(0));
        assert_eq!(
            plan_filter(&mut cache, None, false, at(0)),
            FilterStep::Done
        );
        assert_eq!(cache.current, None);
        assert!(cache.entries.contains_key("k"));
    }

    #[test]
    fn demo_mode_never_searches_a_filter() {
        let (state, path) = demo_state("filter");
        let filter = RemoteReviewFilter {
            draft: Some(true),
            ..Default::default()
        };
        filter_reviews(&state, &filter, false).unwrap();
        assert_eq!(lock_status(&state).reviews.filters.current, None);
        drop(state);
        let _ = std::fs::remove_file(path);
    }

    fn demo_state(name: &str) -> (AppState, PathBuf) {
        // Each state gets its own directory: profiles.json sits next to the db.
        let dir = std::env::temp_dir().join(format!("pr-nexus-{name}-{}", std::process::id()));
        let path = dir.join("pr-nexus.db");
        let state = AppState::for_tests(&path, true);
        (state, path)
    }

    #[test]
    fn demo_state_never_syncs_and_reports_demo_phase() {
        let (state, path) = demo_state("demo");
        let Some(report) = run_sync(&state, SyncKind::Manual).unwrap() else {
            panic!("nothing else was syncing");
        };
        assert_eq!(report.fetched, 0);
        let status = state.sync.lock().unwrap();
        assert_eq!(status.phase, "demo");
        assert!(!status.message.contains("failed"));
        drop(status);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_sync_requested_while_one_runs_waits_and_joins_it() {
        let (state, path) = demo_state("coalesce");
        let state = Arc::new(state);
        let running = state.sync_gate.lock().unwrap();
        let waiter = {
            let state = Arc::clone(&state);
            thread::spawn(move || run_sync(&state, SyncKind::Manual).unwrap())
        };
        thread::sleep(Duration::from_millis(100));
        assert!(
            !waiter.is_finished(),
            "the second caller waits for the first"
        );
        drop(running);
        assert!(
            waiter.join().unwrap().is_none(),
            "it joined instead of syncing again"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Regression: a sync that panicked left the phase at "syncing" forever.
    #[test]
    fn a_panicking_sync_ends_in_an_error_not_syncing() {
        let (state, path) = demo_state("panic");
        let outcome = settle(&state, SyncKind::Manual, || panic!("boom"));
        assert!(outcome.is_err());
        let status = state.sync.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(status.phase, "error");
        assert!(status.message.starts_with("Sync failed"));
        drop(status);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_finished_sync_leaves_the_syncing_phase() {
        let (state, path) = demo_state("settled");
        let report = settle(&state, SyncKind::Full, || {
            assert_eq!(state.sync.lock().unwrap().phase, "syncing");
            state.sync.lock().unwrap().message = progress_message(sync::SyncProgress {
                bucket: SearchBucket::Mine,
                full: true,
                stage: sync::SyncStage::Searching,
                seen: 911,
                updated: 0,
            });
            Ok(sync::SyncReport {
                fetched: 911,
                stored: 911,
                refreshed: 0,
                hidden: 0,
                updated: 0,
                full: true,
                paused: false,
                message: "911 pull requests".into(),
                review_total: None,
            })
        });
        assert!(report.is_ok());
        let status = state.sync.lock().unwrap();
        assert_eq!(status.phase, "ok");
        assert_eq!(status.message, "Synced · 911 pull requests");
        assert!(status.synced_at.is_some());
        drop(status);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    fn progress(
        bucket: SearchBucket,
        full: bool,
        stage: sync::SyncStage,
        seen: usize,
        updated: usize,
    ) -> String {
        progress_message(sync::SyncProgress {
            bucket,
            full,
            stage,
            seen,
            updated,
        })
    }

    #[test]
    fn each_stage_and_category_has_its_own_progress_line() {
        use sync::SyncStage::{Checking, Rechecking, Searching};
        let mine = SearchBucket::Mine;
        let reviews = SearchBucket::ReviewRequested;
        assert_eq!(
            progress(mine, true, Searching, 132, 132),
            "Syncing my PRs… 132"
        );
        assert_eq!(
            progress(
                mine,
                true,
                Checking {
                    done: 40,
                    total: 132
                },
                132,
                132
            ),
            "Checking my PRs… 40/132"
        );
        assert_eq!(
            progress(reviews, true, Searching, 60, 60),
            "Syncing reviews… 60"
        );
        assert_eq!(
            progress(
                reviews,
                true,
                Checking {
                    done: 20,
                    total: 100
                },
                100,
                100
            ),
            "Checking reviews… 20/100"
        );
        assert_eq!(
            progress(mine, true, Rechecking { total: 3 }, 132, 132),
            "Rechecking stale PRs… 3"
        );
        assert_eq!(progress(mine, false, Searching, 0, 0), "Syncing my PRs…");
        assert_eq!(
            progress(reviews, false, Searching, 4, 4),
            "Syncing reviews… 4 updated"
        );
    }

    #[test]
    fn reviews_stay_off_until_asked_for_once() {
        let (mut state, path) = demo_state("reviews");
        assert_eq!(state.sync.lock().unwrap().reviews.state, "off");
        state.demo = false;
        assert!(activate_reviews(&state));
        assert!(!activate_reviews(&state), "a second ask does nothing");
        assert_eq!(state.sync.lock().unwrap().reviews.state, "loading");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

/// A channel-name lookup result and when it was made.
#[derive(Debug, Clone)]
pub struct NameEntry {
    pub name: Option<String>,
    pub at: std::time::Instant,
}

/// A PR's owner teams and blocked reason, valid for its head and base.
#[derive(Debug, Clone)]
pub struct OwnersEntry {
    pub head_sha: String,
    pub base_ref: String,
    pub is_draft: bool,
    pub checks: ChecksState,
    pub at: std::time::Instant,
    /// `Some` when readiness is blocked for a reason other than draft.
    pub blocked: Option<String>,
    pub teams: Vec<String>,
}
