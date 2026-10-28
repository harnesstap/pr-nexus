//! The live-updates thread (§6): polls the notifications inbox and pending
//! checks between incremental syncs, never on the UI thread.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, TryLockError};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use pr_nexus_core::domain::AccountId;
use pr_nexus_core::error::{CoreError, ProviderError};
use pr_nexus_core::live::{self, LiveInput, LivePoller, LiveState, LiveStatus};
use pr_nexus_core::onboarding;
use pr_nexus_core::settings;
use pr_nexus_providers::github::GithubClient;
use serde::Serialize;

use crate::state::{self, AppState};

/// A focus sync at most this often, however often the window flips.
const FOCUS_DEBOUNCE_SECS: i64 = 30;
/// Longest sleep, so a settings change or sign-in is noticed.
const MAX_SLEEP_SECS: i64 = 300;
/// A busy full sync: look again shortly.
const BUSY_RETRY_SECS: u64 = 5;

pub struct LiveShared {
    pub status: Mutex<LiveStatus>,
    pub wake: Arc<(Mutex<bool>, Condvar)>,
    blurred_at: Mutex<Option<DateTime<Utc>>>,
    focus_sync_at: Mutex<Option<DateTime<Utc>>>,
}

impl Default for LiveShared {
    fn default() -> Self {
        Self {
            status: Mutex::new(LiveStatus {
                state: LiveState::Starting,
                checked_at: None,
                error: None,
            }),
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            blurred_at: Mutex::new(None),
            focus_sync_at: Mutex::new(None),
        }
    }
}

impl LiveShared {
    /// Re-read settings and focus now instead of at the next due time.
    pub fn nudge(&self) {
        let (lock, cv) = &*self.wake;
        *lock.lock().unwrap_or_else(|err| err.into_inner()) = true;
        cv.notify_all();
    }
}

/// What the sync details panel shows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LiveDto {
    /// `active`, `starting`, `off`, `unavailable`, or `demo`.
    pub state: String,
    pub checked_at: Option<String>,
    /// Why it is unavailable, or the last error.
    pub detail: Option<String>,
}

pub fn live_dto(state: &AppState) -> LiveDto {
    if state.demo {
        return LiveDto {
            state: "demo".into(),
            checked_at: None,
            detail: None,
        };
    }
    let status = state.live.status.lock().expect("live status").clone();
    let (name, detail) = match status.state {
        LiveState::Off => ("off", None),
        LiveState::Starting => ("starting", status.error),
        LiveState::Active => ("active", status.error),
        LiveState::Unavailable(reason) => ("unavailable", Some(reason)),
    };
    LiveDto {
        state: name.into(),
        checked_at: status.checked_at.map(|at| at.to_rfc3339()),
        detail,
    }
}

/// Focus changes: note when the window was left; coming back after more
/// than a minute asks the scheduler for an incremental pass, debounced.
pub fn note_focus(state: &AppState, focused: bool, now: DateTime<Utc>) -> bool {
    let shared = &state.live;
    let mut blurred = shared.blurred_at.lock().expect("live focus");
    if !focused {
        blurred.get_or_insert(now);
        return false;
    }
    let away = blurred.take();
    drop(blurred);
    shared.nudge();
    if !live::should_sync_on_focus(away, now) {
        return false;
    }
    let mut last = shared.focus_sync_at.lock().expect("live focus");
    if last.is_some_and(|at| now - at < chrono::Duration::seconds(FOCUS_DEBOUNCE_SECS)) {
        return false;
    }
    *last = Some(now);
    true
}

fn sleep(shared: &LiveShared, secs: u64) {
    let (lock, cv) = &*shared.wake;
    let guard = lock.lock().unwrap_or_else(|err| err.into_inner());
    let (mut guard, _) = cv
        .wait_timeout(guard, Duration::from_secs(secs.max(1)))
        .unwrap_or_else(|err| err.into_inner());
    *guard = false;
}

struct Client {
    account: AccountId,
    token: String,
    github: GithubClient,
}

pub fn spawn(state: Arc<AppState>) {
    if state.demo {
        return;
    }
    thread::Builder::new()
        .name("pr-nexus-live".into())
        .spawn(move || run(&state))
        .expect("live updates thread");
}

fn run(state: &AppState) {
    let mut poller = LivePoller::new();
    let mut client: Option<Client> = None;
    loop {
        let wait = step(state, &mut poller, &mut client);
        *state.live.status.lock().expect("live status") = poller.status();
        sleep(&state.live, wait);
    }
}

/// One pass; returns how long to sleep.
fn step(state: &AppState, poller: &mut LivePoller, client: &mut Option<Client>) -> u64 {
    let finished = onboarding::load(&state.store)
        .ok()
        .is_some_and(|saved| saved.is_finished());
    let Ok(Some(account)) = state::github_account(&state.store) else {
        return MAX_SLEEP_SECS as u64;
    };
    if !finished {
        return MAX_SLEEP_SECS as u64;
    }
    // A sync is writing: let it finish rather than race it.
    if let Err(TryLockError::WouldBlock) = state.sync_gate.try_lock() {
        return BUSY_RETRY_SECS;
    }
    if !settings::live_updates(&state.store).unwrap_or(true) {
        poller.disable();
        return MAX_SLEEP_SECS as u64;
    }
    {
        let Ok(token) = state::resolve_github_token(state, Some(&account)) else {
            *client = None;
            return MAX_SLEEP_SECS as u64;
        };
        let stale = client
            .as_ref()
            .is_none_or(|known| known.account != account.id || known.token != token);
        if stale {
            let Ok(github) = GithubClient::new(account.id, token.clone()) else {
                return MAX_SLEEP_SECS as u64;
            };
            // A new token may read notifications where the old one could not.
            poller.reset();
            *client = Some(Client {
                account: account.id,
                token,
                github,
            });
        }
    }
    let Some(known) = client.as_ref() else {
        return MAX_SLEEP_SECS as u64;
    };
    let focused = state.focused.load(Ordering::Relaxed);
    let now = Utc::now();
    let input = LiveInput {
        store: &state.store,
        provider: &known.github,
        account_id: account.id,
        enabled: true,
        focused,
        now,
    };
    match poller.tick(&input) {
        Ok(tick) => {
            if tick.changed > 0 {
                tracing::info!(
                    changed = tick.changed,
                    fetched = tick.fetched,
                    "live update"
                );
                state.sync.lock().expect("sync status").revision += 1;
            }
        }
        Err(CoreError::Provider(ProviderError::Unauthorized(_))) => *client = None,
        Err(err) => tracing::warn!(error = %err, "live update failed"),
    }
    let due = poller.next_due(focused, Utc::now());
    (due - Utc::now()).num_seconds().clamp(1, MAX_SLEEP_SECS) as u64
}

/// Wakes the scheduler for an incremental pass, as Sync now does.
pub fn request_sync(state: &AppState) {
    let (lock, cv) = &*state.wake;
    *lock.lock().expect("sync wake") = true;
    cv.notify_all();
}
