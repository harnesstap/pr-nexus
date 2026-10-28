//! Bulk request and bump commands (§7.9). The logic is in
//! [`crate::services::bulk`].

use std::sync::Arc;

use tauri::State;

use crate::error::AppError;
use crate::services::bulk::{self, BulkPreview, BulkRequest, BulkResult};
use crate::services::review::LiveChats;
use crate::state::AppState;

type Shared<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub async fn bulk_preview(app: Shared<'_>, request: BulkRequest) -> Result<BulkPreview, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || bulk::preview(&state, &request))
        .await
        .map_err(|err| AppError::new("internal", err.to_string()))?
}

/// Plans again and posts every card whose fingerprint still matches.
#[tauri::command]
pub async fn send_bulk(
    app: Shared<'_>,
    request: BulkRequest,
    fingerprints: Vec<String>,
) -> Result<BulkResult, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        bulk::send(&state, &LiveChats, &request, &fingerprints)
    })
    .await
    .map_err(|err| AppError::new("internal", err.to_string()))?
}
