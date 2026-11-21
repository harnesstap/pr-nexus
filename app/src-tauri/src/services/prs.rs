//! Pull requests: the detail view and a filtered list for non-UI callers.
//!
//! [`list_prs`] reuses the list's own row building and status predicates
//! ([`listing::status_matches`], [`listing::quick_matches`]), so a status here
//! means exactly what the filter menu and quick filters show.

use std::sync::Arc;

use chrono::Utc;
use pr_nexus_core::domain::{PrId, PullRequest};
use pr_nexus_core::ownership;
use pr_nexus_core::settings;
use serde::{Deserialize, Serialize};

use crate::commands::{ListRow, PrDetail, detail_dto, list_items, search_ids, stored_pr, with_vcs};
use crate::error::AppError;
use crate::listing;
use crate::services::projects::all_projects;
use crate::state::AppState;

/// Default and largest page for [`list_prs`].
pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 200;

/// Readiness, ownership and the explanations behind them (§6, §7.2).
/// Demo mode uses the fixture provider. A session or store hit skips the
/// GitHub probe; [`pr_detail_live`] revalidates those in the background.
pub fn pr_detail(state: &AppState, id: i64) -> Result<PrDetail, AppError> {
    let _span = tracing::debug_span!("pr_detail", pr = id).entered();
    let pr = stored_pr(state, PrId(id))?;
    if let Some(detail) = crate::services::bulk::cached_detail(state, &pr) {
        tracing::debug!(pr = id, "pr detail from session cache");
        return Ok(detail);
    }
    load_detail(state, &pr, true)
}

/// [`pr_detail`] for the window. A cache hit starts a background probe so the
/// next open sees fresh reviews.
pub fn pr_detail_live(state: &Arc<AppState>, id: i64) -> Result<PrDetail, AppError> {
    let pr = stored_pr(state, PrId(id))?;
    let from_session = crate::services::bulk::cached_detail(state, &pr).is_some();
    let detail = pr_detail(state, id)?;
    if from_session || detail.from_cache {
        schedule_owner_refresh(state, id);
    }
    Ok(detail)
}

/// Teams the review dialog can paint without calling GitHub.
///
/// Session cache, then [`ownership::report_from_cache`], then the requested
/// reviewer teams already stored on the pull request. `source` is `session`,
/// `cache`, or `requested`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoredReviewTeams {
    pub teams: Vec<String>,
    pub owner_groups: Vec<crate::commands::OwnerGroupDto>,
    pub source: String,
}

pub fn stored_review_teams(state: &AppState, id: i64) -> Result<StoredReviewTeams, AppError> {
    let pr = stored_pr(state, PrId(id))?;
    if let Some(detail) = crate::services::bulk::cached_detail(state, &pr) {
        return Ok(stored_from_detail(detail, "session"));
    }
    let flag = settings::blocked_review_required_is_ready(&state.store)?;
    if let Some(report) = ownership::report_from_cache(&state.store, &pr, flag, Utc::now())? {
        return Ok(stored_from_detail(detail_dto(pr.id, report), "cache"));
    }
    Ok(StoredReviewTeams {
        teams: requested_team_names(&pr),
        owner_groups: Vec::new(),
        source: "requested".into(),
    })
}

/// Always probes GitHub. The dialog calls this after the first paint.
pub fn pr_detail_resolve(state: &AppState, id: i64) -> Result<PrDetail, AppError> {
    let pr = stored_pr(state, PrId(id))?;
    load_detail(state, &pr, false)
}

fn stored_from_detail(detail: PrDetail, source: &str) -> StoredReviewTeams {
    let teams = if detail.mandatory_teams.is_empty() {
        detail.candidate_teams.clone()
    } else {
        detail.mandatory_teams.clone()
    };
    StoredReviewTeams {
        teams,
        owner_groups: detail.owner_groups,
        source: source.into(),
    }
}

fn requested_team_names(pr: &PullRequest) -> Vec<String> {
    pr.requested_reviewers
        .iter()
        .filter_map(|request| match &request.reviewer {
            pr_nexus_core::domain::Reviewer::Team(team) => Some(team.to_string()),
            pr_nexus_core::domain::Reviewer::User { .. } => None,
        })
        .collect()
}

fn load_detail(state: &AppState, pr: &PullRequest, use_store: bool) -> Result<PrDetail, AppError> {
    let flag = settings::blocked_review_required_is_ready(&state.store)?;
    let now = Utc::now();
    let report = with_vcs(state, |provider| {
        if use_store
            && let Some(report) = ownership::report_from_cache(&state.store, pr, flag, now)?
        {
            return Ok(report);
        }
        #[cfg(test)]
        state
            .github_resolves
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ownership::resolve(&state.store, provider, pr, flag, now)
    })?;
    let detail = detail_dto(pr.id, report);
    crate::services::bulk::remember_owners(state, pr, &detail);
    Ok(detail)
}

fn schedule_owner_refresh(state: &Arc<AppState>, id: i64) {
    {
        let mut inflight = state
            .owner_refresh
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !inflight.insert(id) {
            return;
        }
    }
    let state = Arc::clone(state);
    std::thread::spawn(move || {
        let refresh = (|| {
            let pr = stored_pr(&state, PrId(id))?;
            load_detail(&state, &pr, false)
        })();
        match refresh {
            Ok(_) => crate::state::note_list_changed(&state),
            Err(err) => tracing::debug!(pr = id, %err, "owner refresh failed"),
        }
        state
            .owner_refresh
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
    });
}

/// PR Nexus's own statuses, as the list's filter menu and quick filters
/// derive them. Names are stable (snake_case on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(description = "A PR Nexus status, as in the list's filter menu and quick filters.")]
pub enum PrStatus {
    /// The PR is a draft.
    Draft,
    /// Not a draft and readiness is `ready`: every mandatory owner is covered.
    Ready,
    /// Not a draft and readiness is `blocked`: owners or rules still missing.
    Blocked,
    /// Not a draft and readiness could not be worked out yet.
    Unknown,
    /// Not a draft, nobody requested on GitHub, no active Slack request,
    /// and no approval or change request yet.
    #[schemars(
        description = "Not a draft, nobody requested on GitHub, no active Slack request, and no approval or change request yet."
    )]
    NoReview,
    /// Failing checks or merge conflicts.
    CiTrouble,
    /// Changes requested.
    Declined,
    /// Not a draft, approved, checks green and no conflicts.
    MergeReady,
    /// The list's "Waiting" chip: open, asked for review (GitHub or Slack),
    /// not approved, and waiting longer than `review_sla_hours` (default 48)
    /// since the Slack request, else since creation. Weekends are skipped
    /// when `review_sla_working_days` is on.
    #[schemars(
        description = "Open, asked for review (GitHub or Slack), not approved, and waiting longer than review_sla_hours (default 48) since the Slack request, else since creation."
    )]
    Waiting,
    /// The list's "Idle" chip: open, no GitHub update and no Slack request
    /// or bump for more than [`listing::IDLE_DAYS`] (7 days). A PR that is
    /// `waiting` is not also `idle`, as on the row.
    #[schemars(
        description = "Open, no GitHub update and no Slack request or bump for more than 7 days. A waiting PR is not also idle."
    )]
    Idle,
}

impl PrStatus {
    pub const ALL: [PrStatus; 10] = [
        PrStatus::Draft,
        PrStatus::Ready,
        PrStatus::Blocked,
        PrStatus::Unknown,
        PrStatus::NoReview,
        PrStatus::CiTrouble,
        PrStatus::Declined,
        PrStatus::MergeReady,
        PrStatus::Waiting,
        PrStatus::Idle,
    ];

    /// Whether `item` is in this status at `now_ms` (only the stale cues
    /// depend on the time). Uses the 48 hour default.
    pub fn matches(self, item: &listing::Item, now_ms: i64) -> bool {
        self.matches_with(item, now_ms, &pr_nexus_core::sla::ReviewSla::default(), 0)
    }

    fn matches_with(
        self,
        item: &listing::Item,
        now_ms: i64,
        sla: &pr_nexus_core::sla::ReviewSla,
        offset_east_minutes: i32,
    ) -> bool {
        let row = &item.row;
        match self {
            PrStatus::Draft => listing::status_matches(row, "draft"),
            PrStatus::Ready => listing::status_matches(row, "ready"),
            PrStatus::Blocked => listing::status_matches(row, "blocked"),
            PrStatus::Unknown => listing::status_matches(row, "unknown"),
            PrStatus::NoReview => listing::quick_matches(row, "no_review"),
            PrStatus::CiTrouble => listing::quick_matches(row, "ci_trouble"),
            PrStatus::Declined => listing::quick_matches(row, "declined"),
            PrStatus::MergeReady => listing::quick_matches(row, "ready"),
            PrStatus::Waiting => {
                listing::stale_cue_with(item, now_ms, sla, offset_east_minutes)
                    == Some(listing::StaleCue::Waiting)
            }
            PrStatus::Idle => {
                listing::stale_cue_with(item, now_ms, sla, offset_east_minutes)
                    == Some(listing::StaleCue::Idle)
            }
        }
    }

    /// Every status `row` is in. Uses the 48 hour default.
    pub fn of(item: &listing::Item, now_ms: i64) -> Vec<PrStatus> {
        Self::of_with(item, now_ms, &pr_nexus_core::sla::ReviewSla::default(), 0)
    }

    fn of_with(
        item: &listing::Item,
        now_ms: i64,
        sla: &pr_nexus_core::sla::ReviewSla,
        offset_east_minutes: i32,
    ) -> Vec<PrStatus> {
        Self::ALL
            .into_iter()
            .filter(|status| status.matches_with(item, now_ms, sla, offset_east_minutes))
            .collect()
    }
}

/// GitHub state of the pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrStateFilter {
    Open,
    Merged,
    Closed,
}

impl PrStateFilter {
    fn as_str(self) -> &'static str {
        match self {
            PrStateFilter::Open => "open",
            PrStateFilter::Merged => "merged",
            PrStateFilter::Closed => "closed",
        }
    }
}

/// Filters for [`list_prs`]. Every field narrows; empty means no filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(
    description = "Filters for listing pull requests. Every field narrows; an empty filter matches every PR."
)]
pub struct PrFilter {
    /// PRs in any of these PR Nexus statuses (OR). Empty: any status.
    pub statuses: Vec<PrStatus>,
    /// Only PRs linked to this project (`id` from `list_projects`).
    pub project_id: Option<i64>,
    /// `Some(true)`: starred only; `Some(false)`: not starred.
    #[schemars(description = "true: starred only; false: not starred; omit: any.")]
    pub starred: Option<bool>,
    /// `owner/name`, or just `name`, case-insensitive.
    pub repo: Option<String>,
    /// Author login, case-insensitive.
    pub author: Option<String>,
    /// GitHub state. Unset: any state (closed PRs are kept a few days).
    pub state: Option<PrStateFilter>,
    /// Same search as the list's search field.
    pub query: Option<String>,
    /// Page size, 1 to 200. Default 50.
    #[schemars(range(min = 1, max = 200))]
    pub limit: Option<usize>,
    /// `next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// Search GitHub for open review requests matching these terms first (waits for it).
    pub review_search: Option<String>,
    /// Read the next window of review requests from GitHub first (waits for it).
    pub more_reviews: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrListItem {
    #[serde(flatten)]
    pub row: ListRow,
    pub statuses: Vec<PrStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrListPage {
    pub items: Vec<PrListItem>,
    /// Matches over every page.
    pub total: usize,
    pub next_cursor: Option<String>,
}

/// Synced, not hidden PRs matching `filter`: starred first, then newest
/// update first, as the list pins them.
pub fn list_prs(state: &AppState, filter: &PrFilter) -> Result<PrListPage, AppError> {
    let now_ms = Utc::now().timestamp_millis();
    let sla = settings::review_sla(&state.store)?;
    let zone = pr_nexus_core::sla::local_offset_east_minutes();
    let offset = match filter.cursor.as_deref() {
        None | Some("") => 0,
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| AppError::new("validation", "Invalid cursor"))?,
    };
    let limit = filter.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let found = match filter.query.as_deref() {
        Some(query) => search_ids(state, &all_projects(state)?, query)?,
        None => None,
    };
    let cached = list_items(state)?;
    let mut items: Vec<listing::Item> = cached
        .iter()
        .filter(|item| matches_filter(item, filter, now_ms, &sla, zone))
        .filter(|item| found.as_ref().is_none_or(|ids| ids.contains(&item.row.id)))
        .cloned()
        .collect();
    items.sort_by(|left, right| {
        right
            .row
            .starred
            .cmp(&left.row.starred)
            .then(right.updated_ms.cmp(&left.updated_ms))
            .then(left.row.id.cmp(&right.row.id))
    });
    let total = items.len();
    let page: Vec<PrListItem> = items
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|item| PrListItem {
            statuses: PrStatus::of_with(&item, now_ms, &sla, zone),
            row: item.row,
        })
        .collect();
    let end = offset + page.len();
    Ok(PrListPage {
        items: page,
        total,
        next_cursor: (end < total).then(|| end.to_string()),
    })
}

/// Everything in [`PrFilter`] that only looks at one row.
pub fn matches_filter(
    item: &listing::Item,
    filter: &PrFilter,
    now_ms: i64,
    sla: &pr_nexus_core::sla::ReviewSla,
    offset_east_minutes: i32,
) -> bool {
    let row = &item.row;
    if row.hidden {
        return false;
    }
    if filter.starred.is_some_and(|starred| starred != row.starred) {
        return false;
    }
    if !filter.statuses.is_empty()
        && !filter
            .statuses
            .iter()
            .any(|status| status.matches_with(item, now_ms, sla, offset_east_minutes))
    {
        return false;
    }
    if filter
        .project_id
        .is_some_and(|id| !row.project_ids.contains(&id))
    {
        return false;
    }
    if let Some(repo) = filter.repo.as_deref().map(str::trim) {
        // `row.repo` is already `owner/name`.
        let name = row
            .repo
            .rsplit_once('/')
            .map_or(row.repo.as_str(), |(_, name)| name);
        if !repo.eq_ignore_ascii_case(&row.repo) && !repo.eq_ignore_ascii_case(name) {
            return false;
        }
    }
    if let Some(author) = filter.author.as_deref().map(str::trim)
        && !author
            .trim_start_matches('@')
            .eq_ignore_ascii_case(&row.author_login)
    {
        return false;
    }
    filter.state.is_none_or(|state| row.state == state.as_str())
}

/// One PR for non-UI callers: the list row and statuses, the note, the
/// Branch card when cached, and the Slack request (`row.slack`). Reads
/// local data only; it never calls GitHub for the overview.
#[derive(Debug, Clone, Serialize)]
pub struct PrInfo {
    #[serde(flatten)]
    pub item: PrListItem,
    /// Empty when there is no note.
    pub note: String,
    pub note_updated_at: Option<String>,
    /// `None` until the PR's overview has been loaded once.
    pub branch: Option<pr_nexus_core::branch::BranchStatus>,
}

pub fn pr_info(state: &AppState, id: i64) -> Result<PrInfo, AppError> {
    let now_ms = Utc::now().timestamp_millis();
    let sla = settings::review_sla(&state.store)?;
    let zone = pr_nexus_core::sla::local_offset_east_minutes();
    let item = list_items(state)?
        .iter()
        .find(|item| item.row.id == id)
        .cloned()
        .ok_or_else(|| AppError::new("not_found", format!("No pull request with id {id}")))?;
    let note = crate::services::project_info::pr_note(state, id)?;
    // A failed compare is "no branch info", not a failed read.
    let branch = crate::services::branch::branch_status_if_cached(state, id).unwrap_or(None);
    Ok(PrInfo {
        item: PrListItem {
            statuses: PrStatus::of_with(&item, now_ms, &sla, zone),
            row: item.row,
        },
        note: note.note,
        note_updated_at: note.updated_at,
        branch,
    })
}
