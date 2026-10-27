//! One page of the pull request list (§8.2).
//!
//! The backend filters, searches, groups and sorts every synced row, then
//! returns the slice after a cursor plus counts over the whole result. The
//! TypeScript reference is `app/src/list/paging.ts`; both must agree.
//!
//! The page is a flattened stream: each section contributes a header entry,
//! then its rows unless it is collapsed. A row linked to two projects appears
//! under both. Counts come from the full result, never from the loaded slice.

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
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
    /// One quick status filter from the bar above the list, or none.
    #[serde(default)]
    pub quick: Option<String>,
}

impl FiltersDto {
    fn active(&self) -> bool {
        !self.orgs.is_empty()
            || !self.statuses.is_empty()
            || !self.reviews.is_empty()
            || !self.ci.is_empty()
            || !self.slack.is_empty()
            || self.archived
            || self.quick.is_some()
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

#[derive(Debug, Serialize)]
pub struct PageDto {
    pub entries: Vec<PageEntry>,
    pub sections: Vec<PageSection>,
    pub next_cursor: Option<String>,
    pub summary: PageSummary,
    pub in_view: usize,
    pub orgs: Vec<OrgCount>,
    pub quick: QuickCounts,
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

    let scoped: Vec<&Item> = items
        .iter()
        .filter(|item| in_view(&item.row, &request.view_key, project_view))
        .filter(|item| scope_row(&item.row, &live_ids, filters))
        .collect();
    let matched: Vec<&Item> = scoped
        .iter()
        .copied()
        .filter(|item| passes_menu(item, filters, now_ms))
        .filter(|item| search.is_none_or(|ids| ids.contains(&item.row.id)))
        .collect();
    let quick = quick_counts(&matched);
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

    let ascending = request.direction == "asc";
    shown.sort_by(|left, right| {
        let delta = rank(left, &request.ordering).cmp(&rank(right, &request.ordering));
        let delta = if ascending { delta } else { delta.reverse() };
        delta.then(left.row.id.cmp(&right.row.id))
    });

    let keep_empty: HashSet<i64> = if search.is_some() || filters.active() {
        HashSet::new()
    } else if let Some(id) = project_view {
        HashSet::from([id])
    } else {
        live_ids.clone()
    };
    let groups = group(&shown, &live, &live_ids, request, &keep_empty);

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
            row: item.map(|item| item.row.clone()),
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
        quick,
    }
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
fn scope_row(row: &ListRow, live: &HashSet<i64>, filters: &FiltersDto) -> bool {
    if row.hidden {
        return false;
    }
    if row.state != "open" && !filters.archived {
        return false;
    }
    let only_archived =
        !row.project_ids.is_empty() && !row.project_ids.iter().any(|id| live.contains(id));
    !(only_archived && !filters.archived)
}

fn passes_menu(item: &Item, filters: &FiltersDto, now_ms: i64) -> bool {
    let row = &item.row;
    if !filters.orgs.is_empty()
        && !filters
            .orgs
            .iter()
            .any(|org| org.eq_ignore_ascii_case(&row.owner))
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

fn status_matches(row: &ListRow, status: &str) -> bool {
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

fn rank(item: &Item, ordering: &str) -> i64 {
    match ordering {
        "created" => item.created_ms,
        "slack" => item.slack_requested_ms,
        "readiness" => match item.row.readiness.as_str() {
            "ready" => 0,
            "unknown" => 1,
            _ => 2,
        },
        "number" => i64::try_from(item.row.number).unwrap_or(i64::MAX),
        _ => item.updated_ms,
    }
}

fn status_title(readiness: &str) -> &'static str {
    match readiness {
        "ready" => "Ready",
        "blocked" => "Blocked",
        _ => "Unknown",
    }
}

fn group<'a>(
    rows: &[&'a Item],
    live: &[&ProjectDto],
    live_ids: &HashSet<i64>,
    request: &PageRequest,
    keep_empty: &HashSet<i64>,
) -> Vec<Group<'a>> {
    match request.grouping.as_str() {
        "repository" => group_by(rows, |item| item.row.repo.clone()),
        "org" => group_by(rows, |item| item.row.owner.clone()),
        "status" => group_by(rows, |item| status_title(&item.row.readiness).to_owned()),
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
                groups.push(Group {
                    key: "unassigned".to_owned(),
                    title: "Unassigned".to_owned(),
                    project_id: None,
                    items: unassigned,
                });
            }
            groups
        }
    }
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
        }
    }

    fn row_ids(page: &PageDto) -> Vec<Option<i64>> {
        page.entries
            .iter()
            .map(|entry| entry.row.as_ref().map(|row| row.id))
            .collect()
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
}
