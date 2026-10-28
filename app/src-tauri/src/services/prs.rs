//! Pull requests: the detail view and a filtered list for non-UI callers.
//!
//! [`list_prs`] reuses the list's own row building and status predicates
//! ([`listing::status_matches`], [`listing::quick_matches`]), so a status here
//! means exactly what the filter menu and quick filters show.

use chrono::Utc;
use pr_nexus_core::domain::PrId;
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
/// Demo mode uses the fixture provider; otherwise this calls GitHub.
pub fn pr_detail(state: &AppState, id: i64) -> Result<PrDetail, AppError> {
    let pr = stored_pr(state, PrId(id))?;
    let flag = settings::blocked_review_required_is_ready(&state.store)?;
    let report = with_vcs(state, |provider| {
        ownership::resolve(&state.store, provider, &pr, flag, Utc::now())
    })?;
    Ok(detail_dto(pr.id, report))
}

/// PR Nexus's own statuses, as the list's filter menu and quick filters
/// derive them. Names are stable (snake_case on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
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
    NoReview,
    /// Failing checks or merge conflicts.
    CiTrouble,
    /// Changes requested.
    Declined,
    /// Not a draft, approved, checks green and no conflicts.
    MergeReady,
    /// The list's "Waiting" chip: open, asked for review (GitHub or Slack),
    /// not approved, and waiting longer than
    /// [`listing::STALE_REVIEW_HOURS`] (48h) since the Slack request, else
    /// since creation.
    Waiting,
    /// The list's "Idle" chip: open, no GitHub update and no Slack request
    /// or bump for more than [`listing::IDLE_DAYS`] (7 days). A PR that is
    /// `waiting` is not also `idle`, as on the row.
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
    /// depend on the time).
    pub fn matches(self, item: &listing::Item, now_ms: i64) -> bool {
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
                listing::stale_cue(item, now_ms) == Some(listing::StaleCue::Waiting)
            }
            PrStatus::Idle => listing::stale_cue(item, now_ms) == Some(listing::StaleCue::Idle),
        }
    }

    /// Every status `row` is in.
    pub fn of(item: &listing::Item, now_ms: i64) -> Vec<PrStatus> {
        Self::ALL
            .into_iter()
            .filter(|status| status.matches(item, now_ms))
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
pub struct PrFilter {
    /// PRs in any of these PR Nexus statuses (OR). Empty: any status.
    pub statuses: Vec<PrStatus>,
    /// Only PRs linked to this project (`id` from `list_projects`).
    pub project_id: Option<i64>,
    /// `Some(true)`: starred only; `Some(false)`: not starred.
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
    pub limit: Option<usize>,
    /// `next_cursor` of the previous page.
    pub cursor: Option<String>,
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
    let mut items: Vec<listing::Item> = list_items(state)?
        .into_iter()
        .filter(|item| matches_filter(item, filter, now_ms))
        .filter(|item| found.as_ref().is_none_or(|ids| ids.contains(&item.row.id)))
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
            statuses: PrStatus::of(&item, now_ms),
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
pub fn matches_filter(item: &listing::Item, filter: &PrFilter, now_ms: i64) -> bool {
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
            .any(|status| status.matches(item, now_ms))
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
        let full = format!("{}/{}", row.owner, row.repo);
        if !repo.eq_ignore_ascii_case(&full) && !repo.eq_ignore_ascii_case(&row.repo) {
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
    let item = list_items(state)?
        .into_iter()
        .find(|item| item.row.id == id)
        .ok_or_else(|| AppError::new("not_found", format!("No pull request with id {id}")))?;
    let note = crate::services::project_info::pr_note(state, id)?;
    // A failed compare is "no branch info", not a failed read.
    let branch = crate::services::branch::branch_status_if_cached(state, id).unwrap_or(None);
    Ok(PrInfo {
        item: PrListItem {
            statuses: PrStatus::of(&item, now_ms),
            row: item.row,
        },
        note: note.note,
        note_updated_at: note.updated_at,
        branch,
    })
}
