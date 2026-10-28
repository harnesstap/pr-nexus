//! The Branch card (§7.2): conflicts, how far behind the base a pull
//! request is, and updating it from the base. Shared by the Tauri commands
//! and future non-Tauri callers (an MCP server).

use pr_nexus_core::branch::{self, BranchStatus, BranchUpdate, UpdateMethod};
use pr_nexus_core::domain::PrId;
use pr_nexus_core::overview;

use crate::commands::{stored_pr, with_vcs};
use crate::error::AppError;
use crate::state::AppState;

/// The Branch card with a fresh overview. One GraphQL call, plus one REST
/// compare unless the head and base SHAs are already cached.
pub fn branch_status(state: &AppState, id: i64) -> Result<BranchStatus, AppError> {
    let pr = stored_pr(state, PrId(id))?;
    with_vcs(state, |provider| {
        branch::status(&state.store, provider, &state.branch_cache, &pr)
    })
}

/// The Branch card from the cached overview (the detail panel refreshes the
/// overview itself); falls back to [`branch_status`] when none is cached.
pub fn branch_status_cached(state: &AppState, id: i64) -> Result<BranchStatus, AppError> {
    let pr = stored_pr(state, PrId(id))?;
    let Some(cached) = overview::cached(&state.store, pr.id)? else {
        return branch_status(state, id);
    };
    with_vcs(state, |provider| {
        Ok(branch::status_from(
            provider,
            &state.branch_cache,
            &pr,
            &cached,
        ))
    })
}

/// The Branch card only when an overview is cached: never fetches the
/// overview, so non-UI readers stay cheap. `None` until the detail panel
/// (or a sync) has loaded the PR once.
pub fn branch_status_if_cached(
    state: &AppState,
    id: i64,
) -> Result<Option<BranchStatus>, AppError> {
    let pr = stored_pr(state, PrId(id))?;
    let Some(cached) = overview::cached(&state.store, pr.id)? else {
        return Ok(None);
    };
    with_vcs(state, |provider| {
        Ok(Some(branch::status_from(
            provider,
            &state.branch_cache,
            &pr,
            &cached,
        )))
    })
}

/// Brings the base into the pull request's branch with `method` (`merge` or
/// `rebase`), guarded by the head SHA last shown. Callers confirm a rebase.
pub fn update_branch(state: &AppState, id: i64, method: &str) -> Result<BranchUpdate, AppError> {
    let method = UpdateMethod::parse(method)
        .ok_or_else(|| AppError::new("validation", format!("unknown update method {method}")))?;
    let pr = stored_pr(state, PrId(id))?;
    with_vcs(state, |provider| {
        branch::update(&state.store, provider, &state.branch_cache, &pr, method)
    })
}
