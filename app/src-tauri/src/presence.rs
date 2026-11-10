//! Menu bar icon, dock-side window, and desktop notifications (§8.5).
//!
//! Closing the window hides it. Quit is the only way out. The badge and the
//! menu counts come from [`listing::attention_counts`]. Notification text
//! comes from [`pr_nexus_core::notify`], which stays quiet on the first
//! snapshot after launch.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, PoisonError};
use std::thread;

use pr_nexus_core::domain::{Involvement, ReviewDecision};
use pr_nexus_core::notify::{self, Notice, NotifyToggles, WatchedPr};
use pr_nexus_core::settings;
use pr_nexus_core::sync::SyncKind;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::{NotificationExt, PermissionState};

use crate::error::AppError;
use crate::listing::{self, AttentionCounts, Item};
use crate::state::{self, AppState};

/// Frontend listens for this and opens the list, and the pull request when set.
pub const OPEN_EVENT: &str = "pr-nexus://open";

static APP: OnceLock<AppHandle> = OnceLock::new();
static QUITTING: AtomicBool = AtomicBool::new(false);
static LISTENING: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicI32 = AtomicI32::new(1);
static OPEN: LazyLock<Mutex<HashMap<i32, OpenTarget>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct TrayUi {
    tray: TrayIcon,
    to_review: MenuItem<tauri::Wry>,
    mine: MenuItem<tauri::Wry>,
}

#[derive(Clone)]
struct OpenTarget {
    view: String,
    pr_id: Option<i64>,
}

#[derive(Clone, Serialize)]
struct OpenPayload {
    view: String,
    pr_id: Option<i64>,
}

struct Observed {
    counts: AttentionCounts,
    notices: Vec<Notice>,
}

/// Set before [`AppHandle::exit`] so the window close that follows it is
/// allowed to finish. Also set when the system asks the process to quit.
pub fn begin_quit() {
    QUITTING.store(true, Ordering::SeqCst);
}

pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}
pub fn show_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// Builds the tray. A failure here must not stop the window from opening.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let _ = APP.set(app.clone());
    let ui = build_tray(app).map_err(|err| err.to_string())?;
    app.manage(ui);
    ensure_clicks(app);
    if let Some(state) = app.try_state::<std::sync::Arc<AppState>>() {
        refresh_counts(state.inner());
    }
    Ok(())
}

/// Recounts the badge without comparing snapshots. Profile switches and a
/// paused sync use this so stored rows are not announced as new.
pub fn refresh_counts(state: &AppState) {
    let Ok(items) = crate::commands::list_items(state) else {
        return;
    };
    let counts = listing::attention_counts(&items, chrono::Utc::now().timestamp_millis());
    deliver(counts, Vec::new());
}

/// One finished sync. The first call only records the snapshot.
pub fn note_sync(state: &AppState, include_reviews: bool) {
    if state.demo {
        refresh_counts(state);
        return;
    }
    let Some(observed) = observe(state, include_reviews) else {
        return;
    };
    deliver(observed.counts, observed.notices);
}

/// A live update. Ignored for events until a sync has recorded a baseline,
/// so a poll that races startup cannot announce pull requests already stored.
pub fn note_change(state: &AppState, include_reviews: bool) {
    let ready = state
        .notifier
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .ready();
    if state.demo || !ready {
        refresh_counts(state);
        return;
    }
    note_sync(state, include_reviews);
}

fn observe(state: &AppState, include_reviews: bool) -> Option<Observed> {
    let items = match crate::commands::list_items(state) {
        Ok(items) => items,
        Err(err) => {
            tracing::debug!(error = %err, "attention list");
            return None;
        }
    };
    let toggles = settings::notify_toggles(&state.store).unwrap_or_else(|err| {
        tracing::debug!(error = %err, "notification settings");
        NotifyToggles::default()
    });
    let now = chrono::Utc::now().timestamp_millis();
    let counts = listing::attention_counts(&items, now);
    let rows: Vec<WatchedPr> = items
        .iter()
        .filter(|item| !item.row.hidden)
        .map(watched)
        .collect();
    let events = {
        let mut watcher = state
            .notifier
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        watcher.observe(&rows, include_reviews, now)
    };
    let events: Vec<_> = events
        .into_iter()
        .filter(|event| toggles.allows(event.kind))
        .collect();
    Some(Observed {
        counts,
        notices: notify::aggregate(&events),
    })
}

fn watched(item: &Item) -> WatchedPr {
    WatchedPr {
        id: item.row.id,
        number: i64::try_from(item.row.number).unwrap_or(i64::MAX),
        title: item.row.title.clone(),
        repo: item.row.repo.clone(),
        url: item.row.url.clone(),
        open: item.row.state == "open",
        author: Involvement::from_bits_truncate(item.row.involvement).contains(Involvement::AUTHOR),
        review_requested: Involvement::from_bits_truncate(item.row.involvement)
            .contains(Involvement::REVIEW_REQUESTED),
        checks_failing: item.row.checks == "failure" || item.row.checks == "error",
        decision: decision(item.row.review_decision.as_deref()),
        total_threads: i64::from(item.row.total_threads),
    }
}

fn decision(value: Option<&str>) -> Option<ReviewDecision> {
    match value {
        Some("approved") => Some(ReviewDecision::Approved),
        Some("changes_requested") => Some(ReviewDecision::ChangesRequested),
        Some("review_required") => Some(ReviewDecision::ReviewRequired),
        Some(_) | None => None,
    }
}

fn deliver(counts: AttentionCounts, notices: Vec<Notice>) {
    let Some(app) = APP.get().cloned() else {
        return;
    };
    let posted = app.clone();
    let _ = app.run_on_main_thread(move || paint(&posted, counts, notices));
}

fn paint(app: &AppHandle, counts: AttentionCounts, notices: Vec<Notice>) {
    if let Some(ui) = app.try_state::<TrayUi>() {
        let _ = ui
            .to_review
            .set_text(format!("To review: {}", counts.to_review));
        let _ = ui.mine.set_text(format!("Mine: {}", counts.mine));
        let title = (counts.badge > 0).then(|| counts.badge.to_string());
        let _ = ui.tray.set_title(title.as_deref());
        let tip = match counts.badge {
            0 => "PR Nexus".to_owned(),
            1 => "PR Nexus — 1 needs attention".to_owned(),
            n => format!("PR Nexus — {n} need attention"),
        };
        let _ = ui.tray.set_tooltip(Some(tip));
    }
    if let Some(window) = app.get_webview_window("main") {
        let badge = (counts.badge > 0).then(|| i64::try_from(counts.badge).unwrap_or(i64::MAX));
        let _ = window.set_badge_count(badge);
    }
    if notices.is_empty() || !may_notify(app) {
        return;
    }
    ensure_clicks(app);
    for notice in &notices {
        show_one(app, notice);
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<TrayUi> {
    let open = MenuItem::with_id(app, "open", "Open PR Nexus", true, None::<&str>)?;
    let to_review = MenuItem::with_id(app, "to-review", "To review: 0", true, None::<&str>)?;
    let mine = MenuItem::with_id(app, "mine", "Mine: 0", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let above = PredefinedMenuItem::separator(app)?;
    let below = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&open, &above, &to_review, &mine, &below, &refresh, &quit],
    )?;
    let mut builder = TrayIconBuilder::with_id("pr-nexus")
        .tooltip("PR Nexus")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = builder.build(app)?;
    Ok(TrayUi {
        tray,
        to_review,
        mine,
    })
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "open" => show_main(app),
        "to-review" => reveal(app, "to_review", None),
        "mine" => reveal(app, "mine", None),
        "refresh" => refresh(app),
        "quit" => quit(app),
        _ => {}
    }
}

fn reveal(app: &AppHandle, view: &str, pr_id: Option<i64>) {
    show_main(app);
    let _ = app.emit(
        OPEN_EVENT,
        OpenPayload {
            view: view.to_owned(),
            pr_id,
        },
    );
}

fn refresh(app: &AppHandle) {
    let Some(state) = app.try_state::<std::sync::Arc<AppState>>() else {
        return;
    };
    let state = state.inner().clone();
    let _ = thread::Builder::new()
        .name("pr-nexus-refresh".into())
        .spawn(move || {
            let _ = state::run_sync(&state, SyncKind::Manual);
        });
}

fn quit(app: &AppHandle) {
    begin_quit();
    app.exit(0);
}

fn ensure_clicks(app: &AppHandle) {
    if LISTENING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    if let Err(err) = app.notification().on_action(move |performed| {
        let action = performed.action_id();
        if action != "tap" && action != "default" {
            return;
        }
        let Some(note) = performed.notification() else {
            return;
        };
        let target = OPEN
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&note.id());
        if let Some(target) = target {
            reveal(&handle, &target.view, target.pr_id);
        }
    }) {
        LISTENING.store(false, Ordering::SeqCst);
        tracing::debug!(error = %err, "notification clicks");
    }
}

fn show_one(app: &AppHandle, notice: &Notice) {
    let mut id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if id <= 0 {
        id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    }
    {
        let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        if open.len() > 40
            && let Some(oldest) = open.keys().copied().min()
        {
            open.remove(&oldest);
        }
        open.insert(
            id,
            OpenTarget {
                view: notice.view.to_owned(),
                pr_id: notice.pr_id,
            },
        );
    }
    let shown = app
        .notification()
        .builder()
        .id(id)
        .title(&notice.title)
        .body(&notice.body)
        .extra("view", notice.view)
        .extra("pr_id", notice.pr_id)
        .auto_cancel()
        .show();
    if let Err(err) = shown {
        OPEN.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        tracing::debug!(error = %err, "notification not shown");
    }
}

/// Granted, or a prompt the user just accepted. Denied stays quiet: Settings
/// explains it, and a failed post is not an error toast.
fn may_notify(app: &AppHandle) -> bool {
    match app.notification().permission_state() {
        Ok(PermissionState::Granted) => true,
        Ok(PermissionState::Denied) => false,
        Ok(_) | Err(_) => matches!(
            app.notification().request_permission(),
            Ok(PermissionState::Granted)
        ),
    }
}

fn permission_name(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        PermissionState::Prompt | PermissionState::PromptWithRationale => "prompt",
    }
}

fn permission_of(app: &AppHandle) -> &'static str {
    match app.notification().permission_state() {
        Ok(state) => permission_name(state),
        Err(_) => "prompt",
    }
}

#[tauri::command]
pub async fn notification_permission(app: AppHandle) -> Result<String, AppError> {
    Ok(permission_of(&app).to_owned())
}

#[tauri::command]
pub async fn request_notification_permission(app: AppHandle) -> Result<String, AppError> {
    let name = match app.notification().request_permission() {
        Ok(state) => permission_name(state),
        Err(_) => "prompt",
    };
    Ok(name.to_owned())
}
