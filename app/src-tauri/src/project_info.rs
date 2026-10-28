//! Project info and per-PR mark commands. The logic is in
//! [`crate::services::project_info`].

use std::sync::Arc;

use tauri::State;

use crate::blocking;
use crate::commands::ProjectDto;
use crate::error::AppError;
use crate::services::project_info::{self, JiraLinkDto, JiraLinkInput, PrNoteDto};
use crate::state::AppState;

type Shared<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub async fn set_project_description(
    app: Shared<'_>,
    project_id: i64,
    description: String,
) -> Result<ProjectDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        project_info::set_project_description(app.inner().as_ref(), project_id, description)
    })
    .await
}

#[tauri::command]
pub async fn add_jira_link(
    app: Shared<'_>,
    project_id: i64,
    link: JiraLinkInput,
) -> Result<JiraLinkDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || project_info::add_jira_link(app.inner().as_ref(), project_id, link)).await
}

#[tauri::command]
pub async fn remove_jira_link(app: Shared<'_>, link_id: i64) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || project_info::remove_jira_link(app.inner().as_ref(), link_id)).await
}

#[tauri::command]
pub async fn set_jira_links(
    app: Shared<'_>,
    project_id: i64,
    links: Vec<JiraLinkInput>,
) -> Result<Vec<JiraLinkDto>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || project_info::set_jira_links(app.inner().as_ref(), project_id, links))
        .await
}

#[tauri::command]
pub async fn pr_note(app: Shared<'_>, pr_id: i64) -> Result<PrNoteDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || project_info::pr_note(app.inner().as_ref(), pr_id)).await
}

#[tauri::command]
pub async fn set_pr_note(app: Shared<'_>, pr_id: i64, note: String) -> Result<PrNoteDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || project_info::set_pr_note(app.inner().as_ref(), pr_id, note)).await
}

#[tauri::command]
pub async fn set_pr_starred(app: Shared<'_>, pr_id: i64, starred: bool) -> Result<bool, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || project_info::set_pr_starred(app.inner().as_ref(), pr_id, starred)).await
}
