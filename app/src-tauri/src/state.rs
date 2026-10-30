use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, TryLockError};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use pr_nexus_core::domain::{Account, AccountKind};
use pr_nexus_core::onboarding::{self, OnboardingState, StepValidity};
use pr_nexus_core::providers::ChatProvider;
use pr_nexus_core::providers::{ProviderRegistry, SetupStep};
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
}

impl SyncStatus {
    fn idle() -> Self {
        Self {
            phase: "idle".into(),
            message: "Not synced yet".into(),
            synced_at: None,
            paused: false,
            revision: 0,
        }
    }

    fn demo() -> Self {
        Self {
            phase: "demo".into(),
            message: "Demo data · sync disabled".into(),
            synced_at: None,
            paused: false,
            revision: 0,
        }
    }
}

pub struct DeviceSession {
    pub device_code: String,
    pub client_id: String,
}

pub struct AppState {
    pub store: Store,
    pub vault: Arc<Vault>,
    /// One lazy Slack MCP session per connected workspace account.
    pub chats: Mutex<HashMap<i64, Arc<dyn ChatProvider>>>,
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
        let store = Store::open(&path)?;
        let vault = Vault::open();
        let demo = demo_enabled();
        if demo {
            crate::demo::seed_if_empty(&store)?;
        }
        Ok(Self {
            store,
            vault: Arc::new(vault),
            chats: Mutex::new(HashMap::new()),
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
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            live: crate::live::LiveShared::default(),
            branch_cache: pr_nexus_core::branch::BranchCache::default(),
            demo,
        })
    }
}

#[cfg(test)]
impl AppState {
    /// A state over a fresh database at `path`, with an in-memory vault.
    pub(crate) fn for_tests(path: &std::path::Path, demo: bool) -> Self {
        Self {
            store: Store::open(path).expect("test store"),
            vault: Arc::new(Vault::memory("test")),
            chats: Mutex::new(HashMap::new()),
            slack_cancel: Mutex::new(None),
            slack_tokens: Mutex::new(HashMap::new()),
            slack_proxy: crate::mcp::slack::SlackProxy::default(),
            sync: Mutex::new(SyncStatus::demo()),
            sync_gate: Mutex::new(()),
            device: Mutex::new(None),
            focused: AtomicBool::new(true),
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            live: crate::live::LiveShared::default(),
            branch_cache: pr_nexus_core::branch::BranchCache::default(),
            demo,
        }
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
    let accounts = store.list_accounts()?;
    Ok(accounts
        .into_iter()
        .find(|account| account.kind == AccountKind::GitHub))
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
    if let sync::SyncStage::Checking { done, total } = progress.stage {
        return format!("Syncing… checking {done}/{total}");
    }
    match (progress.full, progress.updated) {
        (true, _) => format!("Syncing… {} so far", progress.seen),
        (false, 0) => "Syncing…".into(),
        (false, updated) => format!("Syncing… {updated} updated"),
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
        .unwrap_or_else(|_| Err(AppError::new("internal", "sync stopped unexpectedly")));
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

/// The sync status, even after a panic poisoned its lock.
fn lock_status(state: &AppState) -> std::sync::MutexGuard<'_, SyncStatus> {
    state.sync.lock().unwrap_or_else(PoisonError::into_inner)
}

fn sync_once(state: &AppState, kind: SyncKind) -> Result<sync::SyncReport, AppError> {
    if state.demo {
        *state.sync.lock().expect("sync status") = SyncStatus::demo();
        return Ok(sync::SyncReport {
            fetched: 0,
            stored: 0,
            refreshed: 0,
            hidden: 0,
            updated: 0,
            full: false,
            paused: false,
            message: "Demo data · sync disabled".into(),
        });
    }
    settle(state, kind, || {
        let account = github_account(&state.store)?
            .ok_or_else(|| AppError::new("unauthorized", "Connect GitHub before syncing"))?;
        let token = resolve_github_token(state, Some(&account))?;
        let client = GithubClient::new(account.id, token)?;
        // Each search page is stored as it arrives; the header shows the
        // running count, updated at most every `PROGRESS_EVERY`.
        let mut throttle = sync::Throttle::new(sync::PROGRESS_EVERY);
        let mut shown = 0usize;
        let mut stage = sync::SyncStage::Searching;
        let report = sync::sync_account_with_progress(
            &state.store,
            &client,
            account.id,
            kind,
            Utc::now(),
            &mut |progress| {
                // A new stage shows at once: the search ending must not wait
                // on the throttle.
                let moved =
                    std::mem::discriminant(&progress.stage) != std::mem::discriminant(&stage);
                stage = progress.stage;
                if throttle.ready(std::time::Instant::now()) || moved {
                    let mut status = lock_status(state);
                    status.message = progress_message(progress);
                    if progress.updated != shown {
                        shown = progress.updated;
                        status.revision += 1;
                    }
                }
            },
        )?;
        // §8.1: re-evaluate auto-link rules after every sync.
        let prs = state.store.list_pull_requests_for_account(account.id)?;
        let links = pr_nexus_core::linking::relink(&state.store, &prs)?;
        let mut changed = report.updated > 0 || links.added + links.removed > 0;
        // §7.1: a BLOCKED pull request with no review decision needs its
        // code-owner status before the list can call it Ready.
        if !report.paused {
            let flag = settings::blocked_review_required_is_ready(&state.store)?;
            let resolved = pr_nexus_core::ownership::resolve_unknown_code_owner_reviews(
                &state.store,
                &client,
                &prs,
                flag,
                Utc::now(),
                pr_nexus_core::ownership::MAX_RESOLVES_PER_SYNC,
            )
            .unwrap_or(0);
            changed |= resolved > 0;
        }
        if changed {
            state.sync.lock().expect("sync status").revision += 1;
        }
        Ok::<_, AppError>(report)
    })
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

    fn demo_state(name: &str) -> (AppState, PathBuf) {
        let path = std::env::temp_dir().join(format!("pr-nexus-{name}-{}.db", std::process::id()));
        let state = AppState {
            store: Store::open(&path).unwrap(),
            vault: Arc::new(Vault::memory("test")),
            chats: Mutex::new(HashMap::new()),
            slack_cancel: Mutex::new(None),
            slack_tokens: Mutex::new(HashMap::new()),
            slack_proxy: crate::mcp::slack::SlackProxy::default(),
            sync: Mutex::new(SyncStatus::demo()),
            sync_gate: Mutex::new(()),
            device: Mutex::new(None),
            focused: AtomicBool::new(true),
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            live: crate::live::LiveShared::default(),
            branch_cache: pr_nexus_core::branch::BranchCache::default(),
            demo: true,
        };
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
        let _ = std::fs::remove_file(&path);
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
        let _ = std::fs::remove_file(&path);
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
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_finished_sync_leaves_the_syncing_phase() {
        let (state, path) = demo_state("settled");
        let report = settle(&state, SyncKind::Full, || {
            assert_eq!(state.sync.lock().unwrap().phase, "syncing");
            state.sync.lock().unwrap().message = progress_message(sync::SyncProgress {
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
            })
        });
        assert!(report.is_ok());
        let status = state.sync.lock().unwrap();
        assert_eq!(status.phase, "ok");
        assert_eq!(status.message, "Synced · 911 pull requests");
        assert!(status.synced_at.is_some());
        drop(status);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_status_pass_has_its_own_progress_line() {
        let checking = sync::SyncProgress {
            full: true,
            stage: sync::SyncStage::Checking {
                done: 400,
                total: 911,
            },
            seen: 911,
            updated: 0,
        };
        assert_eq!(progress_message(checking), "Syncing… checking 400/911");
    }
}
