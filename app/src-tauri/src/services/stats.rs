//! The Activity Stats view (§8.2): fetches the viewer's own pull requests of
//! the longest period once, caches them, and computes any shorter period or
//! filter from the cache.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration, Utc};
use pr_nexus_core::settings;
use pr_nexus_core::stats::{self, ReviewStats, StatsFetch, StatsFilter};
use pr_nexus_core::sync::TrackedScope;
use serde::Serialize;

use crate::commands::with_vcs;
use crate::error::AppError;
use crate::state::{self, AppState};

/// How long a fetch serves before the view asks GitHub again.
const TTL_MINUTES: i64 = 15;

#[derive(Debug, Clone, Serialize)]
pub struct ReviewStatsDto {
    pub stats: ReviewStats,
    pub fetched_at: DateTime<Utc>,
    /// The fetch stopped early (rate budget or size cap).
    pub partial: bool,
}

struct Cached {
    fetched_at: DateTime<Utc>,
    fetch: StatsFetch,
}

fn cache() -> &'static Mutex<HashMap<String, Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn scope(state: &AppState) -> Result<TrackedScope, AppError> {
    let viewer_login = if state.demo {
        "demo".to_owned()
    } else {
        state::github_account(&state.store)?
            .map(|account| account.login)
            .ok_or_else(|| AppError::new("unauthorized", "Connect GitHub first"))?
    };
    Ok(TrackedScope {
        orgs: settings::tracked_orgs(&state.store)?,
        include_personal: settings::include_personal_repos(&state.store)?,
        viewer_login,
    })
}

/// Per profile: each profile has its own store, account and tracked scope.
fn key(state: &AppState, scope: &TrackedScope) -> String {
    format!(
        "{}|{}|{}|{}",
        state.profile(),
        scope.viewer_login,
        scope.orgs.join(","),
        scope.include_personal
    )
}

fn longest() -> u32 {
    stats::PERIODS.iter().copied().max().unwrap_or(90)
}

fn summarize(
    state: &AppState,
    scope: &TrackedScope,
    cached: &Cached,
    days: u32,
    filter: &StatsFilter,
) -> Result<ReviewStatsDto, AppError> {
    let now = Utc::now();
    let slack = state
        .store
        .slack_request_facts(now - Duration::days(i64::from(longest())))?;
    Ok(ReviewStatsDto {
        stats: stats::compute(
            &cached.fetch.prs,
            &slack,
            &scope.viewer_login,
            days,
            now,
            filter,
        ),
        fetched_at: cached.fetched_at,
        partial: cached.fetch.partial,
    })
}

/// The stats for `days`, fetching when the cache is missing, stale or
/// `refresh` is set.
pub fn review_stats(
    state: &AppState,
    days: u32,
    filter: StatsFilter,
    refresh: bool,
) -> Result<ReviewStatsDto, AppError> {
    if !stats::PERIODS.contains(&days) {
        return Err(AppError::new("validation", "Unknown stats period"));
    }
    let scope = scope(state)?;
    let key = key(state, &scope);
    let now = Utc::now();
    let fresh = {
        let cache = cache().lock().expect("stats cache");
        cache
            .get(&key)
            .is_some_and(|hit| now - hit.fetched_at < Duration::minutes(TTL_MINUTES))
    };
    if refresh || !fresh {
        let queries = stats::stats_queries(&scope, now - Duration::days(i64::from(longest())));
        let fetch = with_vcs(state, |provider| {
            Ok(provider.review_timelines(&queries, &mut |_, _| {})?)
        })?;
        cache().lock().expect("stats cache").insert(
            key.clone(),
            Cached {
                fetched_at: now,
                fetch,
            },
        );
    }
    let cache = cache().lock().expect("stats cache");
    let hit = cache
        .get(&key)
        .ok_or_else(|| AppError::new("internal", "stats cache missing"))?;
    summarize(state, &scope, hit, days, &filter)
}

/// The stats from the cache only (no network), for the MCP tool.
pub fn cached_review_stats(
    state: &AppState,
    days: u32,
    filter: StatsFilter,
) -> Result<Option<ReviewStatsDto>, AppError> {
    if !stats::PERIODS.contains(&days) {
        return Err(AppError::new("validation", "Unknown stats period"));
    }
    let scope = scope(state)?;
    let cache = cache().lock().expect("stats cache");
    cache
        .get(&key(state, &scope))
        .map(|hit| summarize(state, &scope, hit, days, &filter))
        .transpose()
}
