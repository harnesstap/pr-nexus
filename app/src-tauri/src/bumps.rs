//! Bumps-due commands (§7.14). Listing, snooze and skip only. Sending is
//! `send_bulk`, and only after the user clicks Send on the queue.

use chrono::Utc;
use tauri::State;

use crate::blocking;
use crate::error::AppError;
use crate::services::bumps::{self, DueBump};
use crate::state::AppState;

type Shared<'a> = State<'a, std::sync::Arc<AppState>>;

#[tauri::command]
pub async fn due_bumps(app: Shared<'_>) -> Result<Vec<DueBump>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || bumps::list_due(app.inner().as_ref(), Utc::now())).await
}

#[tauri::command]
pub async fn snooze_bump(app: Shared<'_>, pr_id: i64) -> Result<Vec<DueBump>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || bumps::snooze(app.inner().as_ref(), pr_id, Utc::now())).await
}

#[tauri::command]
pub async fn skip_bump(app: Shared<'_>, pr_id: i64) -> Result<Vec<DueBump>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || bumps::skip(app.inner().as_ref(), pr_id, Utc::now())).await
}
