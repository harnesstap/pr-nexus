//! Projects: list, create, rename, archive, colour, rules and manual links.
//!
//! Plain functions over [`AppState`] so the Tauri commands and any other
//! front end (an MCP server) share one implementation.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use pr_nexus_core::domain::{
    LinkSource, PrId, Project, ProjectId, ProjectRule, PullRequest, ReplacedLink, RuleKind,
};
use pr_nexus_core::linking;
use serde::{Deserialize, Serialize};

use crate::commands::{ListRow, ProjectDto, list_items, search_ids};
use crate::error::AppError;
use crate::listing;
use crate::state::{self, AppState};

pub fn all_projects(state: &AppState) -> Result<Vec<ProjectDto>, AppError> {
    Ok(state
        .store
        .list_projects()?
        .into_iter()
        .map(project_dto)
        .collect())
}

/// One slice of the list plus counts over every match (§8.2).
pub fn page_for(
    state: &AppState,
    request: &listing::PageRequest,
) -> Result<listing::PageDto, AppError> {
    let projects = all_projects(state)?;
    let items = list_items(state)?;
    let found = search_ids(state, &projects, &request.query)?;
    Ok(listing::page(&items, &projects, request, found.as_ref()))
}

/// Rows by id, in the order asked, for dialogs that outlive the loaded pages.
pub fn rows_by_id(state: &AppState, ids: Vec<i64>) -> Result<Vec<ListRow>, AppError> {
    let mut by_id: HashMap<i64, ListRow> = list_items(state)?
        .into_iter()
        .map(|item| (item.row.id, item.row))
        .collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

/// Open PRs the project dialog can link by hand: matching, not linked yet.
pub fn link_candidates(
    state: &AppState,
    project_id: i64,
    query: String,
    limit: usize,
) -> Result<Vec<ListRow>, AppError> {
    let projects = all_projects(state)?;
    let Some(found) = search_ids(state, &projects, &query)? else {
        return Ok(Vec::new());
    };
    let linked: HashSet<i64> = state
        .store
        .list_project_links(ProjectId(project_id))?
        .into_iter()
        .filter(|link| !matches!(link.source, LinkSource::Excluded))
        .map(|link| link.pr.0)
        .collect();
    let mut items: Vec<listing::Item> = list_items(state)?
        .into_iter()
        .filter(|item| {
            found.contains(&item.row.id)
                && !linked.contains(&item.row.id)
                && !item.row.hidden
                && item.row.state == "open"
        })
        .collect();
    // `12` or `#12` puts PR number 12 first, then the newest matches.
    let exact: Option<u64> = query.trim().trim_start_matches('#').parse().ok();
    items.sort_by(|left, right| {
        (Some(right.row.number) == exact)
            .cmp(&(Some(left.row.number) == exact))
            .then(right.updated_ms.cmp(&left.updated_ms))
            .then(left.row.id.cmp(&right.row.id))
    });
    Ok(items
        .into_iter()
        .take(limit.min(50))
        .map(|item| item.row)
        .collect())
}

fn project_dto(project: Project) -> ProjectDto {
    ProjectDto {
        id: project.id.0,
        name: project.name,
        color: project.color,
        archived: project.archived,
        description: project.description,
        sort_order: project.sort_order,
    }
}

pub fn list_projects(state: &AppState) -> Result<Vec<ProjectDto>, AppError> {
    Ok(state
        .store
        .list_projects()?
        .into_iter()
        .map(project_dto)
        .collect())
}

pub fn create_project(state: &AppState, name: String) -> Result<ProjectDto, AppError> {
    Ok(project_dto(state.store.create_project(name)?))
}

/// "Add to project → New project from selection…": the project and its manual
/// links are created together, or not at all.
pub fn create_project_with_prs(
    state: &AppState,
    name: String,
    pr_ids: Vec<i64>,
) -> Result<ProjectDto, AppError> {
    let (project, _) = state.store.create_project_with_prs(
        name,
        pr_ids.into_iter().map(PrId).collect(),
        Utc::now(),
    )?;
    Ok(project_dto(project))
}

pub fn rename_project(state: &AppState, id: i64, name: String) -> Result<(), AppError> {
    state.store.rename_project(ProjectId(id), name)?;
    Ok(())
}

pub fn archive_project(state: &AppState, id: i64, archived: bool) -> Result<(), AppError> {
    state.store.set_project_archived(ProjectId(id), archived)?;
    Ok(())
}

pub fn delete_project(state: &AppState, id: i64) -> Result<(), AppError> {
    state.store.delete_project(ProjectId(id))?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct RuleDto {
    pub id: Option<i64>,
    pub project_id: i64,
    pub kind: String,
    pub pattern: String,
    pub priority: i32,
}

#[derive(Serialize)]
pub struct LinkDto {
    pub pr_id: i64,
    pub source: String,
    pub rule_id: Option<i64>,
}

fn rule_dto(rule: ProjectRule) -> RuleDto {
    RuleDto {
        id: Some(rule.id),
        project_id: rule.project.0,
        kind: rule.kind.as_str().to_owned(),
        pattern: rule.pattern,
        priority: rule.priority,
    }
}

fn parse_kind(kind: &str) -> Result<RuleKind, AppError> {
    kind.parse::<RuleKind>()
        .map_err(|_| AppError::new("validation", format!("Unknown rule kind {kind}")))
}

fn account_prs(state: &AppState) -> Result<Vec<PullRequest>, AppError> {
    Ok(match state::github_account(&state.store)? {
        Some(account) => state.store.list_pull_requests_for_account(account.id)?,
        None => Vec::new(),
    })
}

fn relink_all(state: &AppState) -> Result<(), AppError> {
    linking::relink(&state.store, &account_prs(state)?)?;
    Ok(())
}

pub fn set_project_color(state: &AppState, id: i64, color: String) -> Result<ProjectDto, AppError> {
    Ok(project_dto(
        state.store.set_project_color(ProjectId(id), color)?,
    ))
}

pub fn list_project_rules(state: &AppState, project_id: i64) -> Result<Vec<RuleDto>, AppError> {
    let mut rules = state.store.list_project_rules(ProjectId(project_id))?;
    rules.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
    Ok(rules.into_iter().map(rule_dto).collect())
}

/// Creates (`id: null`) or updates a rule, then re-evaluates every rule (§8.1).
pub fn save_project_rule(state: &AppState, rule: RuleDto) -> Result<RuleDto, AppError> {
    let kind = parse_kind(&rule.kind)?;
    let pattern = linking::validate(kind, &rule.pattern)?;
    let value = ProjectRule {
        id: rule.id.unwrap_or(0),
        project: ProjectId(rule.project_id),
        kind,
        pattern,
        priority: rule.priority,
    };
    let saved = match rule.id {
        Some(_) => state.store.update_project_rule(value)?,
        None => state.store.insert_project_rule(value)?,
    };
    relink_all(state)?;
    Ok(rule_dto(saved))
}

pub fn delete_project_rule(state: &AppState, id: i64) -> Result<(), AppError> {
    state.store.delete_project_rule(id)?;
    relink_all(state)
}

/// Live preview: how many open PRs `kind`/`pattern` would link.
pub fn preview_rule(state: &AppState, kind: String, pattern: String) -> Result<usize, AppError> {
    let kind = parse_kind(&kind)?;
    Ok(linking::preview_count(
        &state.store,
        &account_prs(state)?,
        kind,
        &pattern,
    )?)
}

pub fn list_project_links(state: &AppState, project_id: i64) -> Result<Vec<LinkDto>, AppError> {
    Ok(state
        .store
        .list_project_links(ProjectId(project_id))?
        .into_iter()
        .map(|link| {
            let (source, rule_id) = match link.source {
                LinkSource::Manual => ("manual", None),
                LinkSource::Rule(id) => ("rule", Some(id)),
                LinkSource::Excluded => ("excluded", None),
            };
            LinkDto {
                pr_id: link.pr.0,
                source: source.to_owned(),
                rule_id,
            }
        })
        .collect())
}

/// What one manual link replaced: `before` is `null` (not linked), `"rule"`
/// (with `rule_id`) or `"excluded"`. The list's Undo hands these back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplacedLinkDto {
    pub pr_id: i64,
    pub before: Option<String>,
    pub rule_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct LinkOutcomeDto {
    /// Rows that became manual links; PRs already linked by hand are not counted.
    pub changed: usize,
    pub undo: Vec<ReplacedLinkDto>,
}

fn replaced_dto(item: ReplacedLink) -> ReplacedLinkDto {
    let (before, rule_id) = match item.before {
        None => (None, None),
        Some(LinkSource::Manual) => (Some("manual"), None),
        Some(LinkSource::Rule(id)) => (Some("rule"), Some(id)),
        Some(LinkSource::Excluded) => (Some("excluded"), None),
    };
    ReplacedLinkDto {
        pr_id: item.pr.0,
        before: before.map(str::to_owned),
        rule_id,
    }
}

fn replaced_from_dto(dto: ReplacedLinkDto) -> Result<ReplacedLink, AppError> {
    let before = match (dto.before.as_deref(), dto.rule_id) {
        (None, _) => None,
        (Some("manual"), _) => Some(LinkSource::Manual),
        (Some("excluded"), _) => Some(LinkSource::Excluded),
        (Some("rule"), Some(id)) => Some(LinkSource::Rule(id)),
        (Some(other), _) => {
            return Err(AppError::new(
                "validation",
                format!("Cannot undo a link that was {other}"),
            ));
        }
    };
    Ok(ReplacedLink {
        pr: PrId(dto.pr_id),
        before,
    })
}

/// Manual links: drag-and-drop onto a project, "Add to project", or "Add pull
/// request" in the project dialog. All or nothing; the outcome carries Undo.
pub fn link_prs(
    state: &AppState,
    project_id: i64,
    pr_ids: Vec<i64>,
) -> Result<LinkOutcomeDto, AppError> {
    let project = ProjectId(project_id);
    if state.store.get_project(project)?.is_none() {
        return Err(AppError::new("validation", "Project not found"));
    }
    let replaced =
        state
            .store
            .link_manual(project, pr_ids.into_iter().map(PrId).collect(), Utc::now())?;
    Ok(LinkOutcomeDto {
        changed: replaced.len(),
        undo: replaced.into_iter().map(replaced_dto).collect(),
    })
}

/// Undo for [`link_prs`]: puts each still-manual row back the way it was.
pub fn undo_link_prs(
    state: &AppState,
    project_id: i64,
    undo: Vec<ReplacedLinkDto>,
) -> Result<usize, AppError> {
    let replaced = undo
        .into_iter()
        .map(replaced_from_dto)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(state
        .store
        .undo_manual_links(ProjectId(project_id), replaced)?)
}

pub fn unlink_pr(state: &AppState, project_id: i64, pr_id: i64) -> Result<(), AppError> {
    let project = ProjectId(project_id);
    let pr = PrId(pr_id);
    let rule_matches = match state.store.get_pull_request(pr)? {
        Some(value) => linking::rule_matches(&state.store, &value, project)?,
        None => false,
    };
    state
        .store
        .unlink_pr(project, pr, rule_matches, Utc::now())?;
    Ok(())
}

pub fn restore_pr(state: &AppState, project_id: i64, pr_id: i64) -> Result<(), AppError> {
    state.store.restore_pr(ProjectId(project_id), PrId(pr_id))?;
    relink_all(state)
}
