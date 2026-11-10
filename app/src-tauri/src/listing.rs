//! One page of the pull request list (§8.2).
//!
//! The backend filters, searches, groups and sorts every synced row, then
//! returns the slice after a cursor plus counts over the whole result. The
//! TypeScript reference is `app/src/list/paging.ts`; both must agree.
//!
//! The page is a flattened stream: each section contributes a header entry,
//! then its rows unless it is collapsed. A row linked to two projects appears
//! under both. Counts come from the full result, never from the loaded slice.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use pr_nexus_core::stack::{self, StackPlace, StackRef, StackState};
use serde::{Deserialize, Serialize};

use crate::commands::{ListRow, ProjectDto};

const AUTHOR: u8 = 1;
const REVIEW_REQUESTED: u8 = 2;
const BUMP_DUE_MS: i64 = 24 * 60 * 60 * 1000;
/// Largest page, so a refresh of everything loaded stays one call.
pub const MAX_LIMIT: usize = 5000;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FiltersDto {
    #[serde(default)]
    pub orgs: Vec<String>,
    /// `owner/name`. Empty means every repository.
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub reviews: Vec<String>,
    #[serde(default)]
    pub ci: Vec<String>,
    #[serde(default)]
    pub slack: Vec<String>,
    #[serde(default)]
    pub archived: bool,
    /// Only starred pull requests.
    #[serde(default)]
    pub starred: bool,
    /// Only unread pull requests. Combines with the other filters.
    #[serde(default)]
    pub unread: bool,
    /// Only snoozed pull requests. Off leaves them out of the list and its counts.
    #[serde(default)]
    pub snoozed: bool,
    /// One quick status filter from the bar above the list, or none.
    #[serde(default)]
    pub quick: Option<String>,
    /// A GitHub pull-request query saved with the view. Membership comes from
    /// [`PageRequest::github_ids`]; this only marks the filter as on.
    #[serde(default)]
    pub github: String,
}

impl FiltersDto {
    fn active(&self) -> bool {
        !self.orgs.is_empty()
            || !self.repos.is_empty()
            || !self.statuses.is_empty()
            || !self.reviews.is_empty()
            || !self.ci.is_empty()
            || !self.slack.is_empty()
            || self.archived
            || self.starred
            || self.unread
            || self.snoozed
            || self.quick.is_some()
            || !self.github.trim().is_empty()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageRequest {
    pub view_key: String,
    pub grouping: String,
    pub ordering: String,
    pub direction: String,
    #[serde(default)]
    pub filters: FiltersDto,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub collapsed: Vec<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    pub limit: usize,
    pub now: DateTime<Utc>,
    pub day_start: DateTime<Utc>,
    pub week_start: DateTime<Utc>,
    /// When set, these ids are the view: Mine, To review, and project links
    /// do not decide membership. Order is this order, with starred rows first.
    #[serde(default)]
    pub github_ids: Option<Vec<i64>>,
    /// Show merged and closed rows in `github_ids` (and rows marked hidden).
    /// Rows that live only in archived projects still wait for Archived.
    #[serde(default)]
    pub github_any_state: bool,
}

#[derive(Debug, Serialize)]
pub struct PageEntry {
    pub section: String,
    pub row: Option<ListRow>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PageSection {
    pub key: String,
    pub title: String,
    pub project_id: Option<i64>,
    pub count: usize,
    pub ids: Vec<i64>,
}

#[derive(Debug, Serialize, PartialEq, Eq, Default)]
pub struct PageSummary {
    pub open: usize,
    pub finished: usize,
    pub unassigned: usize,
    pub live_projects: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct OrgCount {
    pub name: String,
    pub count: usize,
}

/// Rows per quick filter over the menu-filtered, searched result, before the
/// quick filter itself, so every button shows what clicking it would leave.
#[derive(Debug, Serialize, PartialEq, Eq, Default)]
pub struct QuickCounts {
    pub draft: usize,
    pub no_review: usize,
    pub ci_trouble: usize,
    pub declined: usize,
    pub ready: usize,
}

/// Where else the search matches, so an empty result can say why and offer
/// the way there instead of a bare "no match".
#[derive(Debug, Serialize, PartialEq, Eq, Default)]
pub struct Elsewhere {
    /// Matches in the other nav views (`mine`, `to_review`), within the same
    /// open/archived scope but before any menu filter.
    pub views: Vec<ViewCount>,
    /// Matches in this view that the menu or quick filters hide.
    pub filtered: usize,
    /// Matches in this view that pass the filters but are merged, closed, or
    /// only in archived projects; "Archived and closed" brings them back.
    pub finished: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ViewCount {
    pub view_key: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct PageDto {
    pub entries: Vec<PageEntry>,
    pub sections: Vec<PageSection>,
    pub next_cursor: Option<String>,
    pub summary: PageSummary,
    pub in_view: usize,
    pub orgs: Vec<OrgCount>,
    pub repos: Vec<OrgCount>,
    pub quick: QuickCounts,
    /// Unread rows in this snooze scope after the other filters, including the quick filter.
    pub attention: usize,
    /// Snoozed rows that pass the other filters. Stable when the Snoozed chip toggles.
    pub snoozed_count: usize,
    /// Only while searching.
    pub elsewhere: Option<Elsewhere>,
}

/// A list row with the times the page logic compares, parsed once.
#[derive(Debug, Clone)]
pub struct Item {
    pub row: ListRow,
    pub updated_ms: i64,
    pub created_ms: i64,
    /// `requested_at` of the active Slack request, or 0.
    pub slack_requested_ms: i64,
    /// Last bump or request time, for "bump due".
    pub slack_stamp_ms: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Cursor {
    o: usize,
    s: String,
    r: Option<i64>,
}

struct Group<'a> {
    key: String,
    title: String,
    project_id: Option<i64>,
    items: Vec<&'a Item>,
}

/// `search`: ids matching the query, or `None` when there is no query.
pub fn page(
    items: &[Item],
    projects: &[ProjectDto],
    request: &PageRequest,
    search: Option<&HashSet<i64>>,
) -> PageDto {
    let filters = &request.filters;
    let now_ms = request.now.timestamp_millis();
    let live = displayed(projects, filters);
    let live_ids: HashSet<i64> = live.iter().map(|project| project.id).collect();
    let project_view = project_id_of(&request.view_key);
    let github: Option<HashSet<i64>> = request
        .github_ids
        .as_ref()
        .map(|ids| ids.iter().copied().collect());

    let viewed: Vec<&Item> = items
        .iter()
        .filter(|item| match &github {
            Some(ids) => ids.contains(&item.row.id),
            None => in_view(&item.row, &request.view_key, project_view),
        })
        .filter(|item| scope_row(&item.row, &live_ids, filters, request.github_any_state))
        .collect();
    let scoped: Vec<&Item> = viewed
        .iter()
        .copied()
        .filter(|item| in_attention(&item.row, filters))
        .collect();
    let matched: Vec<&Item> = scoped
        .iter()
        .copied()
        .filter(|item| passes_menu(item, filters, now_ms))
        .filter(|item| search.is_none_or(|ids| ids.contains(&item.row.id)))
        .collect();
    let quick = quick_counts(&matched);
    let elsewhere = search.map(|ids| {
        elsewhere(
            items,
            &request.view_key,
            project_view,
            &live_ids,
            filters,
            now_ms,
            ids,
        )
    });
    let mut shown: Vec<&Item> = matched
        .into_iter()
        .filter(|item| {
            filters
                .quick
                .as_deref()
                .is_none_or(|quick| quick_matches(&item.row, quick))
        })
        .collect();

    let open = shown.iter().filter(|item| item.row.state == "open").count();
    let summary = PageSummary {
        open,
        finished: shown.len() - open,
        unassigned: shown
            .iter()
            .filter(|item| !item.row.project_ids.iter().any(|id| live_ids.contains(id)))
            .count(),
        live_projects: live.len(),
    };

    if let Some(ids) = &request.github_ids {
        let pos: std::collections::HashMap<i64, usize> = ids
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect();
        shown.sort_by(|left, right| {
            right
                .row
                .starred
                .cmp(&left.row.starred)
                .then(pos.get(&left.row.id).cmp(&pos.get(&right.row.id)))
                .then(left.row.id.cmp(&right.row.id))
        });
    } else {
        let ascending = request.direction == "asc";
        shown.sort_by(|left, right| {
            let delta = rank(left, request, now_ms).cmp(&rank(right, request, now_ms));
            let delta = if ascending { delta } else { delta.reverse() };
            // Starred rows are pinned above the rest, each part in the chosen order.
            right
                .row
                .starred
                .cmp(&left.row.starred)
                .then(delta)
                .then(left.row.id.cmp(&right.row.id))
        });
    }

    let marks = stack_marks(items);
    let keep_empty: HashSet<i64> = if search.is_some() || filters.active() {
        HashSet::new()
    } else if let Some(id) = project_view {
        HashSet::from([id])
    } else if request.view_key == "to_review" {
        // To review lists only PRs asking for you; empty projects there are noise.
        HashSet::new()
    } else {
        live_ids.clone()
    };
    let groups = group(&shown, &live, &live_ids, request, &keep_empty, &marks);

    let collapsed: HashSet<&str> = request.collapsed.iter().map(String::as_str).collect();
    let mut stream: Vec<(&str, Option<&Item>)> = Vec::new();
    for group in &groups {
        stream.push((&group.key, None));
        if !collapsed.contains(group.key.as_str()) {
            stream.extend(
                group
                    .items
                    .iter()
                    .map(|item| (group.key.as_str(), Some(*item))),
            );
        }
    }
    let start = resume(&stream, request.cursor.as_deref());
    let limit = request.limit.clamp(1, MAX_LIMIT);
    let end = (start + limit).min(stream.len());
    let entries: Vec<PageEntry> = stream[start..end]
        .iter()
        .map(|(section, item)| PageEntry {
            section: (*section).to_owned(),
            row: item.map(|item| {
                let mut row = item.row.clone();
                row.stack = marks.get(&row.id).cloned();
                row
            }),
        })
        .collect();
    let next_cursor = (end < stream.len() && end > start).then(|| {
        let (section, item) = stream[end - 1];
        serde_json::to_string(&Cursor {
            o: end,
            s: section.to_owned(),
            r: item.map(|item| item.row.id),
        })
        .unwrap_or_default()
    });

    let mut orgs: BTreeMap<&str, usize> = BTreeMap::new();
    for item in items {
        *orgs.entry(item.row.owner.as_str()).or_default() += 1;
    }

    PageDto {
        entries,
        sections: groups
            .iter()
            .map(|group| PageSection {
                key: group.key.clone(),
                title: group.title.clone(),
                project_id: group.project_id,
                count: group.items.len(),
                ids: group.items.iter().map(|item| item.row.id).collect(),
            })
            .collect(),
        next_cursor,
        summary,
        in_view: scoped.len(),
        orgs: orgs
            .into_iter()
            .map(|(name, count)| OrgCount {
                name: name.to_owned(),
                count,
            })
            .collect(),
        repos: repo_choices(&scoped, filters, search, now_ms),
        quick,
        attention: attention_count(&scoped, filters, search, now_ms),
        snoozed_count: snoozed_count(&viewed, filters, search, now_ms),
        elsewhere,
    }
}

/// Snoozed rows leave the default list. The Snoozed chip shows only those rows.
fn in_attention(row: &ListRow, filters: &FiltersDto) -> bool {
    if filters.snoozed {
        row.snoozed
    } else {
        !row.snoozed
    }
}

fn attention_count(
    scoped: &[&Item],
    filters: &FiltersDto,
    search: Option<&HashSet<i64>>,
    now_ms: i64,
) -> usize {
    let mut cleared = filters.clone();
    cleared.unread = false;
    scoped
        .iter()
        .filter(|item| {
            passes_menu(item, &cleared, now_ms)
                && search.is_none_or(|ids| ids.contains(&item.row.id))
                && filters
                    .quick
                    .as_deref()
                    .is_none_or(|quick| quick_matches(&item.row, quick))
                && item.row.unread
        })
        .count()
}

fn snoozed_count(
    viewed: &[&Item],
    filters: &FiltersDto,
    search: Option<&HashSet<i64>>,
    now_ms: i64,
) -> usize {
    viewed
        .iter()
        .filter(|item| {
            item.row.snoozed
                && passes_menu(item, filters, now_ms)
                && search.is_none_or(|ids| ids.contains(&item.row.id))
                && filters
                    .quick
                    .as_deref()
                    .is_none_or(|quick| quick_matches(&item.row, quick))
        })
        .count()
}

/// The quick status filters (§8.2). Unknown keys match nothing.
///
/// - `draft`: the PR is a draft.
/// - `no_review`: not a draft, no reviewer requested on GitHub, no active
///   Slack request, and no approval or change request yet.
/// - `ci_trouble`: failing checks or merge conflicts.
/// - `declined`: changes requested.
/// - `ready`: not a draft, approved, checks green and no conflicts.
pub fn quick_matches(row: &ListRow, quick: &str) -> bool {
    match quick {
        "draft" => row.is_draft,
        "no_review" => {
            !row.is_draft
                && !row.reviewers_requested
                && row.slack.is_none()
                && !matches!(
                    row.review_decision.as_deref(),
                    Some("approved" | "changes_requested")
                )
        }
        "ci_trouble" => ci_bucket(&row.checks) == "failing" || row.conflicts,
        "declined" => row.review_decision.as_deref() == Some("changes_requested"),
        "ready" => {
            !row.is_draft
                && row.review_decision.as_deref() == Some("approved")
                && row.checks == "success"
                && !row.conflicts
        }
        _ => false,
    }
}

fn quick_counts(items: &[&Item]) -> QuickCounts {
    let count = |quick: &str| {
        items
            .iter()
            .filter(|item| quick_matches(&item.row, quick))
            .count()
    };
    QuickCounts {
        draft: count("draft"),
        no_review: count("no_review"),
        ci_trouble: count("ci_trouble"),
        declined: count("declined"),
        ready: count("ready"),
    }
}

/// Where the next page starts: after the cursor's entry when it is still in
/// the stream, so rows added or removed above it do not shift or repeat what
/// is loaded; otherwise at the saved offset.
fn resume(stream: &[(&str, Option<&Item>)], cursor: Option<&str>) -> usize {
    let Some(cursor) = cursor.and_then(|value| serde_json::from_str::<Cursor>(value).ok()) else {
        return 0;
    };
    stream
        .iter()
        .position(|(section, item)| {
            *section == cursor.s && item.map(|item| item.row.id) == cursor.r
        })
        .map(|at| at + 1)
        .unwrap_or(cursor.o.min(stream.len()))
}

pub fn project_id_of(view_key: &str) -> Option<i64> {
    view_key.strip_prefix("project:")?.parse().ok()
}

fn in_view(row: &ListRow, view_key: &str, project: Option<i64>) -> bool {
    match view_key {
        "mine" => row.involvement & AUTHOR != 0,
        "to_review" => row.involvement & REVIEW_REQUESTED != 0,
        _ => match project {
            Some(id) => row.project_ids.contains(&id),
            None => true,
        },
    }
}

/// Nav views that list rows by involvement, in sidebar order.
const NAV_VIEWS: [&str; 2] = ["mine", "to_review"];

fn elsewhere(
    items: &[Item],
    view_key: &str,
    project_view: Option<i64>,
    live_ids: &HashSet<i64>,
    filters: &FiltersDto,
    now_ms: i64,
    search: &HashSet<i64>,
) -> Elsewhere {
    let found: Vec<&Item> = items
        .iter()
        .filter(|item| !item.row.hidden && search.contains(&item.row.id))
        .collect();
    let views = NAV_VIEWS
        .iter()
        .filter(|key| **key != view_key)
        .map(|key| ViewCount {
            view_key: (*key).to_owned(),
            count: found
                .iter()
                .filter(|item| in_view(&item.row, key, None))
                .filter(|item| scope_row(&item.row, live_ids, filters, false))
                .filter(|item| in_attention(&item.row, filters))
                .count(),
        })
        .collect();
    let passes = |item: &Item| {
        passes_menu(item, filters, now_ms)
            && filters
                .quick
                .as_deref()
                .is_none_or(|quick| quick_matches(&item.row, quick))
    };
    let here: Vec<&Item> = found
        .iter()
        .copied()
        .filter(|item| in_view(&item.row, view_key, project_view))
        .filter(|item| in_attention(&item.row, filters))
        .collect();
    let filtered = here
        .iter()
        .filter(|item| scope_row(&item.row, live_ids, filters, false) && !passes(item))
        .count();
    let finished = here
        .iter()
        .filter(|item| !scope_row(&item.row, live_ids, filters, false) && passes(item))
        .count();
    Elsewhere {
        views,
        filtered,
        finished,
    }
}

fn displayed<'a>(projects: &'a [ProjectDto], filters: &FiltersDto) -> Vec<&'a ProjectDto> {
    let mut shown: Vec<&ProjectDto> = projects
        .iter()
        .filter(|project| filters.archived || !project.archived)
        .collect();
    shown.sort_by(|left, right| {
        left.sort_order
            .cmp(&right.sort_order)
            .then_with(|| left.name.cmp(&right.name))
    });
    shown
}

/// Merged and closed rows only with the Archived filter; rows linked only to
/// archived projects likewise.
fn scope_row(row: &ListRow, live: &HashSet<i64>, filters: &FiltersDto, any_state: bool) -> bool {
    if !any_state && row.hidden {
        return false;
    }
    if !any_state && row.state != "open" && !filters.archived {
        return false;
    }
    let only_archived =
        !row.project_ids.is_empty() && !row.project_ids.iter().any(|id| live.contains(id));
    !(only_archived && !filters.archived)
}

/// Repositories in the scoped view. Counts ignore the repository filter and
/// include search and the quick filter. A selected name missing from the view
/// stays at 0. Order is case-insensitive.
fn repo_choices(
    scoped: &[&Item],
    filters: &FiltersDto,
    search: Option<&HashSet<i64>>,
    now_ms: i64,
) -> Vec<OrgCount> {
    let mut cleared = filters.clone();
    cleared.repos.clear();
    let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut touch = |name: &str, hit: bool| {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return;
        }
        let key = trimmed.to_ascii_lowercase();
        counts
            .entry(key)
            .and_modify(|(_, count)| {
                if hit {
                    *count += 1;
                }
            })
            .or_insert((trimmed.to_owned(), usize::from(hit)));
    };
    for item in scoped {
        let hit = passes_menu(item, &cleared, now_ms)
            && search.is_none_or(|ids| ids.contains(&item.row.id))
            && filters
                .quick
                .as_deref()
                .is_none_or(|quick| quick_matches(&item.row, quick));
        touch(&item.row.repo, hit);
    }
    for name in &filters.repos {
        touch(name, false);
    }
    counts
        .into_values()
        .map(|(name, count)| OrgCount { name, count })
        .collect()
}

fn passes_menu(item: &Item, filters: &FiltersDto, now_ms: i64) -> bool {
    let row = &item.row;
    if filters.starred && !row.starred {
        return false;
    }
    if filters.unread && !row.unread {
        return false;
    }
    if !filters.orgs.is_empty()
        && !filters
            .orgs
            .iter()
            .any(|org| org.eq_ignore_ascii_case(&row.owner))
    {
        return false;
    }
    if !filters.repos.is_empty()
        && !filters
            .repos
            .iter()
            .any(|repo| repo.eq_ignore_ascii_case(&row.repo))
    {
        return false;
    }
    if !filters.statuses.is_empty()
        && !filters
            .statuses
            .iter()
            .any(|status| status_matches(row, status))
    {
        return false;
    }
    if !filters.reviews.is_empty()
        && !filters
            .reviews
            .iter()
            .any(|kind| kind == review_bucket(row))
    {
        return false;
    }
    if !filters.ci.is_empty() && !filters.ci.iter().any(|kind| kind == ci_bucket(&row.checks)) {
        return false;
    }
    if !filters.slack.is_empty() {
        let kinds = slack_kinds(item, now_ms);
        if !filters
            .slack
            .iter()
            .any(|kind| kinds.contains(&kind.as_str()))
        {
            return false;
        }
    }
    true
}

pub(crate) fn status_matches(row: &ListRow, status: &str) -> bool {
    match status {
        "draft" => row.is_draft,
        "ready" | "unknown" | "blocked" => !row.is_draft && row.readiness == status,
        _ => false,
    }
}

fn review_bucket(row: &ListRow) -> &'static str {
    match row.review_decision.as_deref() {
        Some("approved") => "approved",
        Some("changes_requested") => "changes_requested",
        Some("review_required") => "review_required",
        _ => "none",
    }
}

fn ci_bucket(checks: &str) -> &'static str {
    match checks {
        "success" => "passing",
        "failure" | "error" => "failing",
        "pending" | "expected" => "pending",
        _ => "none",
    }
}

fn slack_kinds(item: &Item, now_ms: i64) -> Vec<&'static str> {
    let Some(slack) = &item.row.slack else {
        return vec!["none"];
    };
    let mut kinds = vec![if slack.last_bumped_at.is_some() {
        "bumped"
    } else {
        "requested"
    }];
    if item
        .slack_stamp_ms
        .is_some_and(|stamp| now_ms - stamp >= BUMP_DUE_MS)
    {
        kinds.push("bump_due");
    }
    kinds
}

fn rank(item: &Item, request: &PageRequest, now_ms: i64) -> i64 {
    match request.ordering.as_str() {
        // Descending puts the most urgent next action first, oldest first within it.
        "attention" => attention_rank(item, &request.view_key, now_ms),
        "created" => item.created_ms,
        "slack" => item.slack_requested_ms,
        "readiness" => match item.row.readiness.as_str() {
            "ready" => 0,
            "unknown" => 1,
            _ => 2,
        },
        "number" => i64::try_from(item.row.number).unwrap_or(i64::MAX),
        // Descending puts the longest wait first and rows not waiting last.
        "waiting" => waiting_since(item).map_or(i64::MIN, |since| -since),
        // Descending puts the latest bump (or request) first, unrequested last.
        "bumped" => item.slack_stamp_ms.unwrap_or(0),
        _ => item.updated_ms,
    }
}

/// When an open PR started waiting on review: its Slack request, else its
/// creation. `None` when nobody is asked to review it, or it is a draft or
/// approved. Mirrors `waitingSince` in `app/src/list/rowCues.ts`.
fn waiting_since(item: &Item) -> Option<i64> {
    let row = &item.row;
    let asked = row.reviewers_requested || row.slack.is_some();
    if row.state != "open"
        || row.is_draft
        || !asked
        || row.review_decision.as_deref() == Some("approved")
    {
        return None;
    }
    Some(if item.slack_requested_ms > 0 {
        item.slack_requested_ms
    } else {
        item.created_ms
    })
}

/// The next step on a PR (§8.2), most urgent first: the "Needs attention"
/// sort follows this order. Mirrors `NEXT_ACTIONS` in `app/src/list/nextAction.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextAction {
    Conflicts,
    CiFailing,
    ChangesRequested,
    ReadyToMerge,
    ReviewOverdue,
    Draft,
    RequestReview,
    Nudge,
    Review,
    ChecksRunning,
    WaitingReview,
    WaitingAuthor,
    Reviewed,
    AuthorDraft,
    Done,
}

impl NextAction {
    const ALL: [NextAction; 15] = [
        Self::Conflicts,
        Self::CiFailing,
        Self::ChangesRequested,
        Self::ReadyToMerge,
        Self::ReviewOverdue,
        Self::Draft,
        Self::RequestReview,
        Self::Nudge,
        Self::Review,
        Self::ChecksRunning,
        Self::WaitingReview,
        Self::WaitingAuthor,
        Self::Reviewed,
        Self::AuthorDraft,
        Self::Done,
    ];

    /// Higher is more urgent.
    fn priority(self) -> i64 {
        let index = Self::ALL
            .iter()
            .position(|action| *action == self)
            .unwrap_or(Self::ALL.len());
        i64::try_from(Self::ALL.len() - index).unwrap_or(0)
    }
}

/// `NUDGE_HOURS` in `nextAction.ts`: a Slack request or bump this old makes a reminder due.
pub const NUDGE_HOURS: i64 = 24;
/// Wider than any PR age in ms, so the action always wins over age.
const ATTENTION_STEP: i64 = 10_000_000_000_000;

/// Pure: mirrors `nextAction` in `nextAction.ts`.
pub fn next_action(item: &Item, view_key: &str, now_ms: i64) -> NextAction {
    let row = &item.row;
    if row.state != "open" {
        return NextAction::Done;
    }
    let reviewer = view_key == "to_review"
        || (row.involvement & AUTHOR == 0 && row.involvement & REVIEW_REQUESTED != 0);
    let ci = ci_bucket(&row.checks);
    let decision = row.review_decision.as_deref();
    if reviewer {
        if row.is_draft {
            return NextAction::AuthorDraft;
        }
        if decision == Some("changes_requested") || row.conflicts || ci == "failing" {
            return NextAction::WaitingAuthor;
        }
        if decision == Some("approved") {
            return NextAction::Reviewed;
        }
        let since = waiting_since(item).unwrap_or(item.created_ms);
        return if now_ms - since > STALE_REVIEW_HOURS * HOUR_MS {
            NextAction::ReviewOverdue
        } else {
            NextAction::Review
        };
    }
    if row.conflicts {
        return NextAction::Conflicts;
    }
    if ci == "failing" {
        return NextAction::CiFailing;
    }
    if decision == Some("changes_requested") {
        return NextAction::ChangesRequested;
    }
    if row.is_draft {
        return NextAction::Draft;
    }
    if decision == Some("approved") {
        return if ci == "pending" {
            NextAction::ChecksRunning
        } else {
            NextAction::ReadyToMerge
        };
    }
    if !row.reviewers_requested && row.slack.is_none() {
        return NextAction::RequestReview;
    }
    let due = match (row.slack.is_some(), item.slack_stamp_ms) {
        (true, Some(stamp)) => now_ms - stamp >= NUDGE_HOURS * HOUR_MS,
        _ => waiting_since(item).is_some_and(|since| now_ms - since > STALE_REVIEW_HOURS * HOUR_MS),
    };
    if due {
        NextAction::Nudge
    } else {
        NextAction::WaitingReview
    }
}

/// Open pull requests that need the user (§8.5). A row counts once in
/// `badge` even when it is both authored and waiting on the user's review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AttentionCounts {
    pub to_review: usize,
    pub mine: usize,
    pub badge: usize,
}

const MINE_ATTENTION: &[NextAction] = &[
    NextAction::Conflicts,
    NextAction::CiFailing,
    NextAction::ChangesRequested,
    NextAction::ReadyToMerge,
    NextAction::RequestReview,
    NextAction::Nudge,
];

const REVIEW_ATTENTION: &[NextAction] = &[NextAction::Review, NextAction::ReviewOverdue];

/// Counts from the same [`next_action`] rules as the list. Hidden, closed,
/// snoozed, and muted rows are skipped. To review uses the reviewer rules;
/// Mine uses the author rules, including a pull request that is both.
pub fn attention_counts(items: &[Item], now_ms: i64) -> AttentionCounts {
    let mut counts = AttentionCounts::default();
    for item in items {
        if item.row.state != "open" || item.row.hidden || item.row.snoozed || item.row.muted {
            continue;
        }
        let author = item.row.involvement & AUTHOR != 0;
        let requested = item.row.involvement & REVIEW_REQUESTED != 0;
        let review =
            requested && REVIEW_ATTENTION.contains(&next_action(item, "to_review", now_ms));
        let mine = author && MINE_ATTENTION.contains(&next_action(item, "mine", now_ms));
        counts.to_review += usize::from(review);
        counts.mine += usize::from(mine);
        counts.badge += usize::from(review || mine);
    }
    counts
}

/// Mirrors `attentionRank` in `nextAction.ts`.
pub fn attention_rank(item: &Item, view_key: &str, now_ms: i64) -> i64 {
    let since = waiting_since(item).unwrap_or(item.created_ms);
    let age = (now_ms - since).clamp(0, ATTENTION_STEP - 1);
    next_action(item, view_key, now_ms).priority() * ATTENTION_STEP + age
}

/// The age chip a row shows (§8.2). Mirrors `staleCue` in
/// `app/src/list/rowCues.ts`, with the same thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleCue {
    /// Waiting on review longer than [`STALE_REVIEW_HOURS`].
    Waiting,
    /// No update and no Slack bump for longer than [`IDLE_DAYS`].
    Idle,
}

/// `STALE_REVIEW_HOURS` in `rowCues.ts`.
pub const STALE_REVIEW_HOURS: i64 = 48;
/// `IDLE_DAYS` in `rowCues.ts`.
pub const IDLE_DAYS: i64 = 7;
const HOUR_MS: i64 = 60 * 60 * 1000;

/// Closed and merged rows carry no cue; a long review wait wins over idleness.
pub fn stale_cue(item: &Item, now_ms: i64) -> Option<StaleCue> {
    if item.row.state != "open" {
        return None;
    }
    if waiting_since(item).is_some_and(|since| now_ms - since > STALE_REVIEW_HOURS * HOUR_MS) {
        return Some(StaleCue::Waiting);
    }
    let last = item.updated_ms.max(item.slack_stamp_ms.unwrap_or(0));
    (now_ms - last > IDLE_DAYS * 24 * HOUR_MS).then_some(StaleCue::Idle)
}

fn status_title(readiness: &str) -> &'static str {
    match readiness {
        "ready" => "Ready",
        "blocked" => "Blocked",
        _ => "Unknown",
    }
}

/// Places for stacks of two or more, from every row the page can see.
pub fn stack_marks(items: &[Item]) -> HashMap<i64, StackPlace> {
    let refs: Vec<StackRef> = items
        .iter()
        .map(|item| StackRef {
            id: item.row.id,
            repo: item.row.repo.clone(),
            number: item.row.number,
            state: match item.row.state.as_str() {
                "merged" => StackState::Merged,
                "closed" => StackState::Closed,
                _ => StackState::Open,
            },
            head_ref: item.row.head_ref.clone(),
            base_ref: item.row.base_ref.clone(),
        })
        .collect();
    stack::index(&refs).places().clone()
}

/// Writes [`stack_marks`] onto each row, for callers that return rows outside a page.
pub fn stamp_stacks(items: &mut [Item]) {
    let marks = stack_marks(items);
    for item in items {
        item.row.stack = marks.get(&item.row.id).cloned();
    }
}

fn group<'a>(
    rows: &[&'a Item],
    live: &[&ProjectDto],
    live_ids: &HashSet<i64>,
    request: &PageRequest,
    keep_empty: &HashSet<i64>,
    marks: &HashMap<i64, StackPlace>,
) -> Vec<Group<'a>> {
    match request.grouping.as_str() {
        "repository" => group_by(rows, |item| item.row.repo.clone()),
        "org" => group_by(rows, |item| item.row.owner.clone()),
        "status" => group_by(rows, |item| status_title(&item.row.readiness).to_owned()),
        "stack" => group_stacks(rows, marks),
        "updated" => {
            let day = request.day_start.timestamp_millis();
            let week = request.week_start.timestamp_millis();
            let bucket = |item: &Item| {
                if item.updated_ms >= day {
                    "Today"
                } else if item.updated_ms >= week {
                    "This week"
                } else {
                    "Older"
                }
            };
            ["Today", "This week", "Older"]
                .into_iter()
                .map(|title| Group {
                    key: title.to_owned(),
                    title: title.to_owned(),
                    project_id: None,
                    items: rows
                        .iter()
                        .copied()
                        .filter(|item| bucket(item) == title)
                        .collect(),
                })
                .filter(|group| !group.items.is_empty())
                .collect()
        }
        _ => {
            let mut groups: Vec<Group<'a>> = live
                .iter()
                .map(|project| Group {
                    key: format!("project:{}", project.id),
                    title: project.name.clone(),
                    project_id: Some(project.id),
                    items: rows
                        .iter()
                        .copied()
                        .filter(|item| item.row.project_ids.contains(&project.id))
                        .collect(),
                })
                .filter(|group| {
                    !group.items.is_empty()
                        || group.project_id.is_some_and(|id| keep_empty.contains(&id))
                })
                .collect();
            let unassigned: Vec<&Item> = rows
                .iter()
                .copied()
                .filter(|item| !item.row.project_ids.iter().any(|id| live_ids.contains(id)))
                .collect();
            if !unassigned.is_empty() {
                let section = Group {
                    key: "unassigned".to_owned(),
                    title: "Unassigned".to_owned(),
                    project_id: None,
                    items: unassigned,
                };
                // My PRs puts Unassigned first (PRs still to sort); other views keep it last.
                if request.view_key == "mine" {
                    groups.insert(0, section);
                } else {
                    groups.push(section);
                }
            }
            groups
        }
    }
}

/// One section per stack, members bottom to top. Everything else is "Not stacked".
fn group_stacks<'a>(rows: &[&'a Item], marks: &HashMap<i64, StackPlace>) -> Vec<Group<'a>> {
    let index: HashMap<i64, usize> = rows
        .iter()
        .enumerate()
        .map(|(at, item)| (item.row.id, at))
        .collect();
    let mut buckets: HashMap<i64, Vec<&'a Item>> = HashMap::new();
    let mut plain: Vec<&'a Item> = Vec::new();
    for item in rows {
        match marks.get(&item.row.id) {
            Some(mark) => buckets.entry(mark.root_id).or_default().push(*item),
            None => plain.push(*item),
        }
    }
    let mut roots: Vec<i64> = buckets.keys().copied().collect();
    roots.sort_by_key(|root| {
        index.get(root).copied().unwrap_or_else(|| {
            buckets
                .get(root)
                .and_then(|items| {
                    items
                        .iter()
                        .filter_map(|item| index.get(&item.row.id))
                        .min()
                        .copied()
                })
                .unwrap_or(usize::MAX)
        })
    });
    let mut groups: Vec<Group<'a>> = roots
        .into_iter()
        .map(|root| {
            let mut items = buckets.remove(&root).unwrap_or_default();
            items.sort_by_key(|item| {
                marks
                    .get(&item.row.id)
                    .map(|mark| mark.position)
                    .unwrap_or(0)
            });
            let number = marks
                .get(&root)
                .map(|mark| mark.root_number)
                .unwrap_or_else(|| {
                    items
                        .iter()
                        .find_map(|item| marks.get(&item.row.id).map(|mark| mark.root_number))
                        .unwrap_or(0)
                });
            Group {
                key: format!("stack:{root}"),
                title: format!("Stack · #{number}"),
                project_id: None,
                items,
            }
        })
        .collect();
    if !plain.is_empty() {
        groups.push(Group {
            key: "unstacked".to_owned(),
            title: "Not stacked".to_owned(),
            project_id: None,
            items: plain,
        });
    }
    groups
}

fn group_by<'a>(rows: &[&'a Item], key_of: impl Fn(&Item) -> String) -> Vec<Group<'a>> {
    let mut order: Vec<String> = Vec::new();
    let mut buckets: std::collections::HashMap<String, Vec<&'a Item>> =
        std::collections::HashMap::new();
    for item in rows {
        let key = key_of(item);
        match buckets.get_mut(&key) {
            Some(bucket) => bucket.push(item),
            None => {
                order.push(key.clone());
                buckets.insert(key, vec![*item]);
            }
        }
    }
    order
        .into_iter()
        .map(|key| Group {
            title: key.clone(),
            items: buckets.remove(&key).unwrap_or_default(),
            key,
            project_id: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(hours_ago: i64) -> DateTime<Utc> {
        now() - chrono::Duration::hours(hours_ago)
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 10, 12, 0, 0).unwrap()
    }

    fn item(id: i64, projects: &[i64], updated_hours_ago: i64) -> Item {
        let updated = at(updated_hours_ago);
        Item {
            row: ListRow {
                id,
                number: u64::try_from(id).unwrap(),
                title: format!("Change {id}"),
                url: format!("https://github.com/acme/api/pull/{id}"),
                owner: if id % 2 == 0 { "acme" } else { "acme-labs" }.into(),
                repo: "acme/api".into(),
                author_login: "octo".into(),
                head_ref: format!("topic-{id}"),
                base_ref: "main".into(),
                stack: None,
                is_draft: false,
                state: "open".into(),
                checks: "success".into(),
                review_decision: None,
                unresolved_threads: 0,
                total_threads: 0,
                involvement: AUTHOR,
                updated_at: updated.to_rfc3339(),
                created_at: updated.to_rfc3339(),
                hidden: false,
                starred: false,
                unread: false,
                muted: false,
                snoozed: false,
                project_ids: projects.to_vec(),
                readiness: "ready".into(),
                readiness_notes: Vec::new(),
                conflicts: false,
                reviewers_requested: false,
                pending_reviewers: Vec::new(),
                slack: None,
            },
            updated_ms: updated.timestamp_millis(),
            created_ms: updated.timestamp_millis(),
            slack_requested_ms: 0,
            slack_stamp_ms: None,
        }
    }

    fn project(id: i64, name: &str, archived: bool) -> ProjectDto {
        ProjectDto {
            id,
            name: name.into(),
            color: "blue".into(),
            archived,
            description: String::new(),
            sort_order: i32::try_from(id).unwrap(),
            jira_links: Vec::new(),
        }
    }

    fn request(grouping: &str) -> PageRequest {
        PageRequest {
            view_key: "all".into(),
            grouping: grouping.into(),
            ordering: "updated".into(),
            direction: "desc".into(),
            filters: FiltersDto::default(),
            query: String::new(),
            collapsed: Vec::new(),
            cursor: None,
            limit: 50,
            now: now(),
            day_start: at(12),
            week_start: at(12 + 6 * 24),
            github_ids: None,
            github_any_state: false,
        }
    }

    #[test]
    fn snoozed_rows_leave_the_default_list_and_unread_combines() {
        let quiet = item(1, &[], 1);
        let mut fresh = item(2, &[], 2);
        fresh.row.unread = true;
        let mut asleep = item(3, &[], 3);
        asleep.row.unread = true;
        asleep.row.snoozed = true;
        let mut hushed = item(4, &[], 4);
        hushed.row.muted = true;
        let items = vec![quiet, fresh, asleep, hushed];

        let shown = page(&items, &[], &request("repository"), None);
        let ids: Vec<i64> = row_ids(&shown).into_iter().flatten().collect();
        assert_eq!(ids, vec![1, 2, 4]);
        assert_eq!(shown.in_view, 3);
        assert_eq!(shown.summary.open, 3);
        assert_eq!(shown.attention, 1);
        assert_eq!(shown.snoozed_count, 1);

        let mut unread = request("repository");
        unread.filters.unread = true;
        let only: Vec<i64> = row_ids(&page(&items, &[], &unread, None))
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(only, vec![2]);
        assert_eq!(page(&items, &[], &unread, None).attention, 1);

        let mut snoozed = request("repository");
        snoozed.filters.snoozed = true;
        let waiting = page(&items, &[], &snoozed, None);
        let waiting_ids: Vec<i64> = row_ids(&waiting).into_iter().flatten().collect();
        assert_eq!(waiting_ids, vec![3]);
        assert_eq!(waiting.attention, 1);
        assert_eq!(waiting.snoozed_count, 1);
        assert!(waiting.summary.open == 1);
    }

    fn row_ids(page: &PageDto) -> Vec<Option<i64>> {
        page.entries
            .iter()
            .map(|entry| entry.row.as_ref().map(|row| row.id))
            .collect()
    }

    #[test]
    fn a_github_query_uses_its_ids_its_order_and_the_repository_filter() {
        let mut open = item(1, &[], 1);
        open.row.repo = "acme/widgets".into();
        open.row.involvement = 0;
        let mut merged = item(2, &[], 2);
        merged.row.repo = "acme/widgets".into();
        merged.row.state = "merged".into();
        merged.row.involvement = 0;
        let mut other = item(3, &[], 3);
        other.row.repo = "acme-labs/widgets".into();
        other.row.owner = "acme-labs".into();
        other.row.involvement = 0;
        let mut starred = item(4, &[], 4);
        starred.row.repo = "acme/widgets".into();
        starred.row.starred = true;
        starred.row.involvement = 0;
        let items = vec![open, merged, other, starred];

        let mut queried = request("repository");
        queried.view_key = "to_review".into();
        queried.github_ids = Some(vec![3, 1, 2, 4]);
        queried.github_any_state = true;
        queried.filters.repos = vec!["acme/widgets".into()];
        queried.filters.github = "is:pr author:octocat".into();
        let ids: Vec<i64> = row_ids(&page(&items, &[], &queried, None))
            .into_iter()
            .flatten()
            .collect();
        // Starred stays first. The repository filter drops acme-labs.
        // Merged stays because the query is not open-only.
        assert_eq!(ids, vec![4, 1, 2]);

        queried.github_any_state = false;
        let open_ids: Vec<i64> = row_ids(&page(&items, &[], &queried, None))
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(open_ids, vec![4, 1]);

        let mut plain = request("repository");
        plain.view_key = "to_review".into();
        let none: Vec<i64> = row_ids(&page(&items, &[], &plain, None))
            .into_iter()
            .flatten()
            .collect();
        assert!(none.is_empty());
        plain.filters.github = "is:pr".into();
        assert!(plain.filters.active());
    }

    #[test]
    fn starred_rows_pin_first_and_filter_alone() {
        let mut items: Vec<Item> = (1..=5).map(|id| item(id, &[], id)).collect();
        items[3].row.starred = true;
        let ids: Vec<i64> = row_ids(&page(&items, &[], &request("repository"), None))
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(ids, [4, 1, 2, 3, 5]);
        let mut only = request("repository");
        only.filters.starred = true;
        assert!(only.filters.active());
        let ids: Vec<i64> = row_ids(&page(&items, &[], &only, None))
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(ids, [4]);
    }

    #[test]
    fn stale_cue_matches_the_row_chip() {
        let now_ms = now().timestamp_millis();
        let fresh = item(1, &[], 1);
        assert_eq!(stale_cue(&fresh, now_ms), None);
        let idle = item(2, &[], IDLE_DAYS * 24 + 1);
        assert_eq!(stale_cue(&idle, now_ms), Some(StaleCue::Idle));
        let mut waiting = item(3, &[], IDLE_DAYS * 24 + 1);
        waiting.row.readiness = "ready".into();
        waiting.row.reviewers_requested = true;
        waiting.row.review_decision = Some("REVIEW_REQUIRED".into());
        assert_eq!(stale_cue(&waiting, now_ms), Some(StaleCue::Waiting));
        let mut closed = idle.clone();
        closed.row.state = "merged".into();
        assert_eq!(stale_cue(&closed, now_ms), None);
        let mut bumped = item(4, &[], IDLE_DAYS * 24 + 1);
        bumped.slack_stamp_ms = Some(at(1).timestamp_millis());
        assert_eq!(stale_cue(&bumped, now_ms), None);
        let mut young = item(5, &[], STALE_REVIEW_HOURS - 1);
        young.row.reviewers_requested = true;
        assert_eq!(stale_cue(&young, now_ms), None);
    }

    #[test]
    fn counts_cover_every_match_beyond_the_page() {
        let items: Vec<Item> = (1..=250).map(|id| item(id, &[], id)).collect();
        let page = page(&items, &[], &request("repository"), None);
        assert_eq!(page.entries.len(), 50);
        assert_eq!(page.summary.open, 250);
        assert_eq!(page.sections[0].count, 250);
        assert_eq!(page.sections[0].ids.len(), 250);
        assert!(page.next_cursor.is_some());
    }

    #[test]
    fn cursors_walk_the_stream_once_without_gaps() {
        let items: Vec<Item> = (1..=130).map(|id| item(id, &[], id)).collect();
        let mut request = request("repository");
        let mut seen = Vec::new();
        loop {
            let page = page(&items, &[], &request, None);
            seen.extend(row_ids(&page));
            match page.next_cursor {
                Some(cursor) => request.cursor = Some(cursor),
                None => break,
            }
        }
        // One header, then every row newest first.
        assert_eq!(seen.len(), 131);
        assert_eq!(seen[0], None);
        let ids: Vec<i64> = seen.into_iter().flatten().collect();
        assert_eq!(ids, (1..=130).collect::<Vec<_>>());
    }

    #[test]
    fn cursor_resumes_after_its_row_when_rows_arrive_above() {
        let mut items: Vec<Item> = (1..=60).map(|id| item(id, &[], id + 10)).collect();
        let first = page(&items, &[], &request("repository"), None);
        let last_loaded = first
            .entries
            .last()
            .and_then(|entry| entry.row.as_ref())
            .unwrap()
            .id;
        // A sync brings two newer PRs that sort to the top.
        items.push(item(900, &[], 1));
        items.push(item(901, &[], 2));
        let mut next = request("repository");
        next.cursor = first.next_cursor;
        let second = page(&items, &[], &next, None);
        let ids: Vec<i64> = row_ids(&second).into_iter().flatten().collect();
        assert_eq!(ids.first(), Some(&(last_loaded + 1)));
        assert!(!ids.contains(&900));
    }

    #[test]
    fn project_grouping_repeats_shared_rows_and_keeps_empty_projects() {
        let projects = vec![
            project(1, "Billing", false),
            project(2, "Search", false),
            project(3, "Empty", false),
        ];
        let items = vec![item(1, &[1, 2], 1), item(2, &[], 2), item(3, &[2], 3)];
        let page = page(&items, &projects, &request("project"), None);
        let keys: Vec<(&str, usize)> = page
            .sections
            .iter()
            .map(|section| (section.key.as_str(), section.count))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("project:1", 1),
                ("project:2", 2),
                ("project:3", 0),
                ("unassigned", 1)
            ]
        );
        assert_eq!(page.summary.unassigned, 1);
        assert_eq!(page.summary.live_projects, 3);
    }

    #[test]
    fn unassigned_comes_first_in_my_prs_and_last_in_to_review() {
        let projects = vec![project(1, "Billing", false)];
        let mut items = vec![item(1, &[1], 1), item(2, &[], 2)];
        for entry in &mut items {
            entry.row.involvement = AUTHOR | REVIEW_REQUESTED;
        }
        let keys = |view: &str| -> Vec<String> {
            let mut request = request("project");
            request.view_key = view.into();
            page(&items, &projects, &request, None)
                .sections
                .into_iter()
                .map(|section| section.key)
                .collect()
        };
        assert_eq!(keys("mine"), vec!["unassigned", "project:1"]);
        assert_eq!(keys("to_review"), vec!["project:1", "unassigned"]);
    }

    #[test]
    fn collapsed_sections_send_only_their_header() {
        let projects = vec![project(1, "Billing", false)];
        let items = vec![item(1, &[1], 1), item(2, &[], 2)];
        let mut request = request("project");
        request.collapsed = vec!["project:1".into()];
        let page = page(&items, &projects, &request, None);
        assert_eq!(row_ids(&page), vec![None, None, Some(2)]);
        assert_eq!(page.sections[0].ids, vec![1]);
    }

    #[test]
    fn search_ids_filter_rows_and_drop_kept_empty_projects() {
        let projects = vec![project(1, "Billing", false)];
        let items: Vec<Item> = (1..=10).map(|id| item(id, &[], id)).collect();
        let found = HashSet::from([3, 7]);
        let page = page(&items, &projects, &request("project"), Some(&found));
        assert_eq!(row_ids(&page), vec![None, Some(3), Some(7)]);
        assert_eq!(page.sections.len(), 1);
        assert_eq!(page.in_view, 10);
    }

    #[test]
    fn closed_rows_and_archived_only_links_need_the_archived_filter() {
        let projects = vec![project(1, "Old", true)];
        let mut closed = item(2, &[], 2);
        closed.row.state = "merged".into();
        let items = vec![item(1, &[1], 1), closed, item(3, &[], 3)];
        let open_only = page(&items, &projects, &request("repository"), None);
        assert_eq!(open_only.summary.open, 1);
        let mut request = request("repository");
        request.filters.archived = true;
        let all = page(&items, &projects, &request, None);
        assert_eq!(all.summary.open, 2);
        assert_eq!(all.summary.finished, 1);
    }

    #[test]
    fn menu_filters_and_updated_buckets() {
        let mut draft = item(2, &[], 30);
        draft.row.is_draft = true;
        let items = vec![item(1, &[], 1), draft, item(3, &[], 24 * 9)];
        let mut request = request("updated");
        let buckets = page(&items, &[], &request, None);
        let titles: Vec<&str> = buckets
            .sections
            .iter()
            .map(|section| section.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Today", "This week", "Older"]);
        request.filters.statuses = vec!["draft".into()];
        let drafts = page(&items, &[], &request, None);
        assert_eq!(pageless_ids(&drafts), vec![2]);
        request.filters.statuses.clear();
        request.filters.orgs = vec!["ACME".into()];
        assert_eq!(pageless_ids(&page(&items, &[], &request, None)), vec![2]);
    }

    fn pageless_ids(page: &PageDto) -> Vec<i64> {
        page.sections
            .iter()
            .flat_map(|section| section.ids.clone())
            .collect()
    }

    #[test]
    fn org_options_count_every_synced_row() {
        let items: Vec<Item> = (1..=5).map(|id| item(id, &[], id)).collect();
        let mut request = request("repository");
        request.filters.orgs = vec!["acme".into()];
        let page = page(&items, &[], &request, None);
        assert_eq!(
            page.orgs,
            vec![
                OrgCount {
                    name: "acme".into(),
                    count: 2
                },
                OrgCount {
                    name: "acme-labs".into(),
                    count: 3
                },
            ]
        );
    }

    #[test]
    fn repository_options_follow_the_view_and_keep_a_selected_name() {
        let mut items: Vec<Item> = (1..=3).map(|id| item(id, &[], id)).collect();
        items[0].row.repo = "acme-labs/widgets".into();
        items[1].row.repo = "acme/widgets".into();
        items[2].row.repo = "acme/billing".into();
        items[2].row.hidden = true;
        let mut request = request("mine");
        request.filters.repos = vec!["acme/missing".into(), "ACME/widgets".into()];
        let page = page(&items, &[], &request, None);
        let names: Vec<_> = page
            .repos
            .iter()
            .map(|item| (item.name.as_str(), item.count))
            .collect();
        assert_eq!(
            names,
            vec![
                ("acme-labs/widgets", 1),
                ("acme/missing", 0),
                ("acme/widgets", 1),
            ]
        );
        assert_eq!(pageless_ids(&page), vec![2]);
    }

    fn quick_fixture() -> Vec<Item> {
        // 1 draft, 2 nobody asked, 3 failing CI, 4 conflicts, 5 declined,
        // 6 ready, 7 asked on Slack, 8 reviewer requested on GitHub.
        let mut items: Vec<Item> = (1..=8).map(|id| item(id, &[], id)).collect();
        items[0].row.is_draft = true;
        items[2].row.checks = "failure".into();
        items[2].row.reviewers_requested = true;
        items[3].row.conflicts = true;
        items[3].row.review_decision = Some("approved".into());
        items[4].row.review_decision = Some("changes_requested".into());
        items[5].row.review_decision = Some("approved".into());
        items[6].row.slack = Some(crate::commands::SlackIndicator {
            workspace_name: "acme".into(),
            workspace_disabled: false,
            team_id: None,
            requested_at: at(3).to_rfc3339(),
            last_bumped_at: None,
            bump_count: 0,
            workspace: "acme".into(),
            channel_id: "C0ACME0001".into(),
            channel_name: "team-reviews".into(),
            permalink: None,
            root_ts: "1.0".into(),
            source: "app".into(),
        });
        items[7].row.reviewers_requested = true;
        items
    }

    fn ids(page: &PageDto) -> Vec<i64> {
        let mut ids: Vec<i64> = page.sections.iter().flat_map(|s| s.ids.clone()).collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn quick_filters_match_their_definitions() {
        let items = quick_fixture();
        let mut request = request("repository");
        for (quick, expected) in [
            ("draft", vec![1]),
            ("no_review", vec![2]),
            ("ci_trouble", vec![3, 4]),
            ("declined", vec![5]),
            ("ready", vec![6]),
            ("bogus", vec![]),
        ] {
            request.filters.quick = Some(quick.into());
            assert_eq!(ids(&page(&items, &[], &request, None)), expected, "{quick}");
        }
    }

    #[test]
    fn quick_counts_ignore_the_quick_filter_but_follow_menu_and_search() {
        let items = quick_fixture();
        let mut request = request("repository");
        request.filters.quick = Some("draft".into());
        let all = page(&items, &[], &request, None);
        assert_eq!(
            all.quick,
            QuickCounts {
                draft: 1,
                no_review: 1,
                ci_trouble: 2,
                declined: 1,
                ready: 1
            }
        );
        assert_eq!(all.summary.open, 1);

        request.filters.orgs = vec!["acme".into()];
        let even = page(&items, &[], &request, None);
        assert_eq!(even.quick.ci_trouble, 1);
        assert_eq!(even.quick.ready, 1);
        assert_eq!(even.quick.draft, 0);

        request.filters.orgs.clear();
        request.filters.quick = Some("ci_trouble".into());
        let search = HashSet::from([4, 5]);
        let searched = page(&items, &[], &request, Some(&search));
        assert_eq!(ids(&searched), vec![4]);
        assert_eq!(searched.quick.declined, 1);
        assert_eq!(searched.quick.ci_trouble, 1);
    }

    #[test]
    fn elsewhere_counts_matches_hidden_by_view_filters_and_scope() {
        let mut items = quick_fixture();
        items[0].row.involvement = REVIEW_REQUESTED;
        items[1].row.state = "closed".into();
        let all: HashSet<i64> = items.iter().map(|item| item.row.id).collect();
        let mut request = request("repository");
        request.view_key = "mine".into();
        request.filters.orgs = vec!["acme".into()];
        let searched = page(&items, &[], &request, Some(&all));
        assert_eq!(ids(&searched), vec![4, 6, 8]);
        let elsewhere = searched.elsewhere.unwrap();
        assert_eq!(
            elsewhere.views,
            vec![ViewCount {
                view_key: "to_review".into(),
                count: 1
            }]
        );
        // 3, 5 and 7 are in another org; 2 is closed but in the org.
        assert_eq!(elsewhere.filtered, 3);
        assert_eq!(elsewhere.finished, 1);

        // The quick filter hides rows like a menu filter does.
        request.filters.quick = Some("ready".into());
        let quick = page(&items, &[], &request, Some(&all));
        assert_eq!(ids(&quick), vec![6]);
        assert_eq!(quick.elsewhere.unwrap().filtered, 5);

        request.filters.archived = true;
        request.filters.quick = None;
        let with_closed = page(&items, &[], &request, Some(&all)).elsewhere.unwrap();
        assert_eq!(with_closed.finished, 0);
        assert!(page(&items, &[], &request, None).elsewhere.is_none());
    }

    #[test]
    fn a_quick_filter_drops_kept_empty_projects() {
        let projects = vec![project(1, "Billing", false)];
        let items = quick_fixture();
        let mut request = request("project");
        request.filters.quick = Some("ready".into());
        let page = page(&items, &projects, &request, None);
        assert_eq!(page.sections.len(), 1);
        assert_eq!(page.sections[0].key, "unassigned");
    }

    #[test]
    fn to_review_drops_empty_project_groups() {
        let projects = vec![project(1, "Billing", false), project(2, "Empty", false)];
        let mut items = vec![item(1, &[1], 1)];
        items[0].row.involvement = REVIEW_REQUESTED;
        let mut request = request("project");
        request.view_key = "to_review".into();
        let page = page(&items, &projects, &request, None);
        let keys: Vec<&str> = page.sections.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, vec!["project:1"]);
        let page = super::page(&[], &projects, &request, None);
        assert!(page.sections.is_empty());
    }

    #[test]
    fn next_action_follows_the_author_priority() {
        let now_ms = now().timestamp_millis();
        let mut base = item(1, &[], 1);
        base.row.reviewers_requested = true;
        base.created_ms = at(1).timestamp_millis();
        let check = |edit: &dyn Fn(&mut Item), want: NextAction| {
            let mut entry = base.clone();
            edit(&mut entry);
            assert_eq!(next_action(&entry, "mine", now_ms), want);
        };
        check(&|_| {}, NextAction::WaitingReview);
        check(&|entry| entry.row.conflicts = true, NextAction::Conflicts);
        check(
            &|entry| {
                entry.row.conflicts = true;
                entry.row.checks = "failure".into();
            },
            NextAction::Conflicts,
        );
        check(
            &|entry| entry.row.checks = "error".into(),
            NextAction::CiFailing,
        );
        check(
            &|entry| entry.row.review_decision = Some("changes_requested".into()),
            NextAction::ChangesRequested,
        );
        check(
            &|entry| entry.row.review_decision = Some("approved".into()),
            NextAction::ReadyToMerge,
        );
        check(
            &|entry| {
                entry.row.review_decision = Some("approved".into());
                entry.row.checks = "pending".into();
            },
            NextAction::ChecksRunning,
        );
        check(&|entry| entry.row.is_draft = true, NextAction::Draft);
        check(
            &|entry| entry.row.reviewers_requested = false,
            NextAction::RequestReview,
        );
        check(
            &|entry| entry.created_ms = at(49).timestamp_millis(),
            NextAction::Nudge,
        );
        check(&|entry| entry.row.state = "merged".into(), NextAction::Done);
    }

    #[test]
    fn attention_counts_badge_each_open_pr_once() {
        let now_ms = now().timestamp_millis();
        let mine = item(1, &[], 1);
        let mut waiting = item(2, &[], 1);
        waiting.row.reviewers_requested = true;
        let mut review = item(3, &[], 1);
        review.row.involvement = REVIEW_REQUESTED;
        review.created_ms = at(1).timestamp_millis();
        let mut both = item(4, &[], 1);
        both.row.involvement = AUTHOR | REVIEW_REQUESTED;
        both.row.checks = "failure".into();
        let mut hidden = item(5, &[], 1);
        hidden.row.hidden = true;
        let mut closed = item(6, &[], 1);
        closed.row.state = "closed".into();
        let mut draft = item(7, &[], 1);
        draft.row.is_draft = true;
        let mut snoozed = item(8, &[], 1);
        snoozed.row.snoozed = true;
        snoozed.row.checks = "failure".into();
        let mut muted = item(9, &[], 1);
        muted.row.muted = true;
        muted.row.involvement = REVIEW_REQUESTED;
        muted.created_ms = at(1).timestamp_millis();
        let counts = attention_counts(
            &[
                mine, waiting, review, both, hidden, closed, draft, snoozed, muted,
            ],
            now_ms,
        );
        assert_eq!(
            counts,
            AttentionCounts {
                to_review: 1,
                mine: 2,
                badge: 3,
            }
        );
    }

    #[test]
    fn next_action_reads_to_review_as_the_reviewer() {
        let now_ms = now().timestamp_millis();
        let mut entry = item(1, &[], 1);
        entry.row.involvement = REVIEW_REQUESTED;
        entry.created_ms = at(1).timestamp_millis();
        assert_eq!(next_action(&entry, "to_review", now_ms), NextAction::Review);
        entry.created_ms = at(49).timestamp_millis();
        assert_eq!(
            next_action(&entry, "to_review", now_ms),
            NextAction::ReviewOverdue
        );
        assert_eq!(
            next_action(&entry, "project:1", now_ms),
            NextAction::ReviewOverdue
        );
        entry.row.checks = "failure".into();
        assert_eq!(
            next_action(&entry, "to_review", now_ms),
            NextAction::WaitingAuthor
        );
        entry.row.is_draft = true;
        assert_eq!(
            next_action(&entry, "to_review", now_ms),
            NextAction::AuthorDraft
        );
    }

    #[test]
    fn attention_ordering_puts_urgent_then_oldest_first() {
        let mut items = vec![
            item(1, &[], 1),
            item(2, &[], 1),
            item(3, &[], 1),
            item(4, &[], 1),
        ];
        for entry in &mut items {
            entry.row.reviewers_requested = true;
        }
        items[0].created_ms = at(2).timestamp_millis();
        items[1].row.conflicts = true;
        items[2].row.checks = "failure".into();
        items[2].created_ms = at(5).timestamp_millis();
        items[3].row.checks = "failure".into();
        items[3].created_ms = at(30).timestamp_millis();
        let mut req = request("repository");
        req.ordering = "attention".into();
        let order: Vec<i64> = row_ids(&page(&items, &[], &req, None))
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(order, vec![2, 4, 3, 1]);
    }

    #[test]
    fn waiting_and_bumped_orderings_put_unrequested_rows_last() {
        let mut waiting_new = item(1, &[], 1);
        waiting_new.row.reviewers_requested = true;
        let mut waiting_old = item(2, &[], 1);
        waiting_old.row.reviewers_requested = true;
        waiting_old.created_ms = at(100).timestamp_millis();
        let idle = item(3, &[], 1);
        let mut bumped = item(4, &[], 1);
        bumped.slack_stamp_ms = Some(at(2).timestamp_millis());
        let mut draft = item(5, &[], 1);
        draft.row.is_draft = true;
        draft.row.reviewers_requested = true;
        let items = vec![waiting_new, waiting_old, idle, bumped, draft];
        let mut req = request("repository");
        req.ordering = "waiting".into();
        let page = page(&items, &[], &req, None);
        let order: Vec<i64> = row_ids(&page).into_iter().flatten().collect();
        assert_eq!(order, vec![2, 1, 3, 4, 5]);
        req.ordering = "bumped".into();
        let page = super::page(&items, &[], &req, None);
        assert_eq!(row_ids(&page).into_iter().flatten().next(), Some(4));
    }

    #[test]
    fn pending_reviewers_are_users_then_teams_in_login_order() {
        use pr_nexus_core::domain::{RequestedReviewer, Reviewer, TeamRef};
        let user = |login: &str| {
            RequestedReviewer::bare(Reviewer::User {
                login: login.to_owned(),
            })
        };
        let team = |slug: &str| {
            RequestedReviewer::bare(Reviewer::Team(TeamRef {
                org: "acme".to_owned(),
                slug: slug.to_owned(),
            }))
        };
        let requests = vec![
            team("web"),
            user("zoe"),
            team("api"),
            user("Bob"),
            user("alice"),
        ];
        let logins: Vec<String> = crate::commands::pending_reviewers(&requests)
            .into_iter()
            .map(|reviewer| format!("{}:{}", reviewer.kind, reviewer.login))
            .collect();
        assert_eq!(
            logins,
            vec![
                "user:alice",
                "user:Bob",
                "user:zoe",
                "team:acme/api",
                "team:acme/web"
            ]
        );
        let mut reversed = requests.clone();
        reversed.reverse();
        assert_eq!(
            crate::commands::pending_reviewers(&reversed),
            crate::commands::pending_reviewers(&requests)
        );
    }

    #[test]
    fn stack_grouping_keeps_members_in_merge_order_and_marks_the_row() {
        let mut bottom = item(1, &[], 1);
        bottom.row.number = 10;
        bottom.row.repo = "acme/api".into();
        bottom.row.head_ref = "feat-a".into();
        bottom.row.base_ref = "main".into();
        let mut middle = item(2, &[], 2);
        middle.row.number = 11;
        middle.row.repo = "acme/api".into();
        middle.row.head_ref = "feat-b".into();
        middle.row.base_ref = "feat-a".into();
        let mut top = item(3, &[], 3);
        top.row.number = 12;
        top.row.repo = "acme/api".into();
        top.row.head_ref = "feat-c".into();
        top.row.base_ref = "feat-b".into();
        // The same branch name in another repository is not part of the stack.
        let mut alone = item(4, &[], 4);
        alone.row.repo = "acme/billing".into();
        alone.row.head_ref = "feat-a".into();
        let page = page(&[bottom, middle, top, alone], &[], &request("stack"), None);
        let titles: Vec<&str> = page
            .sections
            .iter()
            .map(|section| section.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Stack · #10", "Not stacked"]);
        assert_eq!(
            row_ids(&page),
            vec![None, Some(1), Some(2), Some(3), None, Some(4)]
        );
        let marked = page
            .entries
            .iter()
            .find_map(|entry| entry.row.as_ref().filter(|row| row.id == 2))
            .unwrap();
        let place = marked.stack.as_ref().unwrap();
        assert_eq!(place.position, 2);
        assert_eq!(place.size, 3);
        assert!(!place.blocking);
        assert_eq!(place.parent_number, Some(10));
    }
}
