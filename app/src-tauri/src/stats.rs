//! The Activity Stats command. The logic is in [`crate::services::stats`].

use std::sync::Arc;

use pr_nexus_core::stats::StatsFilter;
use tauri::State;

use crate::blocking;
use crate::error::AppError;
use crate::services::stats::{self, ReviewStatsDto};
use crate::state::AppState;

/// Review stats for the last `days` (7, 30 or 90), optionally narrowed to
/// one `repo` (`owner/name`) or `org`. Fetches only when the cache is stale
/// or `refresh` is set.
#[tauri::command]
pub async fn review_stats(
    app: State<'_, Arc<AppState>>,
    days: u32,
    repo: Option<String>,
    org: Option<String>,
    refresh: bool,
) -> Result<ReviewStatsDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        stats::review_stats(
            app.inner().as_ref(),
            days,
            StatsFilter { repo, org },
            refresh,
        )
    })
    .await
}
