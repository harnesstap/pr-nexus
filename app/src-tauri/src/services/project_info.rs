//! Project info and per-PR marks: descriptions, JIRA links, notes, stars.
//!
//! Plain functions over [`AppState`] for the Tauri commands and any other
//! front end (an MCP server).

use chrono::Utc;
use pr_nexus_core::domain::{JiraLink, PrId, Project, ProjectId};
use pr_nexus_core::{jira, settings};
use serde::{Deserialize, Serialize};

use crate::commands::ProjectDto;
use crate::error::AppError;
use crate::services::projects::project_dto;
use crate::state::AppState;

/// One JIRA reference as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JiraLinkDto {
    pub id: i64,
    /// `ACME-123`, or empty for a link without a key.
    pub key: String,
    /// The link as entered, or empty for a bare key.
    pub url: String,
    /// Where the chip opens: the URL, else `{jira.base_url}/browse/{key}`,
    /// else `None` (a bare key with no base URL set).
    pub href: Option<String>,
}

/// A link as the editor sends it: `text` (a key or URL to parse), or an
/// explicit key and URL.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct JiraLinkInput {
    pub text: Option<String>,
    pub key: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrNoteDto {
    pub pr_id: i64,
    /// Empty when there is no note.
    pub note: String,
    pub updated_at: Option<String>,
}

fn parse(input: &JiraLinkInput) -> Result<jira::ParsedLink, AppError> {
    Ok(match input.text.as_deref() {
        Some(text) => jira::parse(text)?,
        None => jira::normalize(&input.key, &input.url)?,
    })
}

fn link_dto(link: JiraLink, base: &str) -> JiraLinkDto {
    JiraLinkDto {
        id: link.id,
        href: jira::href(&link.key, &link.url, base),
        key: link.key,
        url: link.url,
    }
}

fn link_dtos(state: &AppState, links: Vec<JiraLink>) -> Result<Vec<JiraLinkDto>, AppError> {
    let base = settings::jira_base_url(&state.store)?;
    Ok(links
        .into_iter()
        .map(|link| link_dto(link, &base))
        .collect())
}

/// Fills `jira_links` on every project.
pub(crate) fn with_jira_links(
    state: &AppState,
    mut projects: Vec<ProjectDto>,
) -> Result<Vec<ProjectDto>, AppError> {
    let mut links = state.store.all_jira_links()?;
    let base = settings::jira_base_url(&state.store)?;
    for project in &mut projects {
        project.jira_links = links
            .remove(&ProjectId(project.id))
            .unwrap_or_default()
            .into_iter()
            .map(|link| link_dto(link, &base))
            .collect();
    }
    Ok(projects)
}

/// One project with its links.
pub(crate) fn one(state: &AppState, project: Project) -> Result<ProjectDto, AppError> {
    let mut dto = project_dto(project);
    dto.jira_links = list_jira_links(state, dto.id)?;
    Ok(dto)
}

/// Sets a project's (multi-line) description; blank clears it.
pub fn set_project_description(
    state: &AppState,
    project_id: i64,
    description: String,
) -> Result<ProjectDto, AppError> {
    let project = state
        .store
        .set_project_description(ProjectId(project_id), description)?;
    one(state, project)
}

pub fn list_jira_links(state: &AppState, project_id: i64) -> Result<Vec<JiraLinkDto>, AppError> {
    let links = state.store.list_jira_links(ProjectId(project_id))?;
    link_dtos(state, links)
}

/// Appends one link. `ACME-123` and `https://…/browse/ACME-123` both work.
pub fn add_jira_link(
    state: &AppState,
    project_id: i64,
    link: JiraLinkInput,
) -> Result<JiraLinkDto, AppError> {
    let parsed = parse(&link)?;
    let saved = state
        .store
        .add_jira_link(ProjectId(project_id), parsed, Utc::now())?;
    let base = settings::jira_base_url(&state.store)?;
    Ok(link_dto(saved, &base))
}

pub fn remove_jira_link(state: &AppState, link_id: i64) -> Result<(), AppError> {
    state.store.remove_jira_link(link_id)?;
    Ok(())
}

/// Replaces every link of a project, in order (adds, removes and reorders
/// at once). Nothing changes when one entry does not parse.
pub fn set_jira_links(
    state: &AppState,
    project_id: i64,
    links: Vec<JiraLinkInput>,
) -> Result<Vec<JiraLinkDto>, AppError> {
    let parsed = links.iter().map(parse).collect::<Result<Vec<_>, _>>()?;
    let saved = state
        .store
        .set_jira_links(ProjectId(project_id), parsed, Utc::now())?;
    link_dtos(state, saved)
}

pub fn pr_note(state: &AppState, pr_id: i64) -> Result<PrNoteDto, AppError> {
    let note = state.store.pr_note(PrId(pr_id))?;
    Ok(PrNoteDto {
        pr_id,
        note: note
            .as_ref()
            .map(|note| note.note.clone())
            .unwrap_or_default(),
        updated_at: note.map(|note| note.updated_at.to_rfc3339()),
    })
}

/// Saves a PR's note; blank deletes it.
pub fn set_pr_note(state: &AppState, pr_id: i64, note: String) -> Result<PrNoteDto, AppError> {
    let saved = state.store.set_pr_note(PrId(pr_id), note, Utc::now())?;
    Ok(PrNoteDto {
        pr_id,
        note: saved
            .as_ref()
            .map(|note| note.note.clone())
            .unwrap_or_default(),
        updated_at: saved.map(|note| note.updated_at.to_rfc3339()),
    })
}

/// Stars or unstars a PR. Idempotent; returns the new state.
pub fn set_pr_starred(state: &AppState, pr_id: i64, starred: bool) -> Result<bool, AppError> {
    state
        .store
        .set_pr_starred(PrId(pr_id), starred, Utc::now())?;
    Ok(starred)
}

pub fn is_pr_starred(state: &AppState, pr_id: i64) -> Result<bool, AppError> {
    Ok(state.store.is_pr_starred(PrId(pr_id))?)
}
