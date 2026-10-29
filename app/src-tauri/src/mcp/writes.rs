//! The Local writes tools: projects, their PRs and JIRA links, PR notes and
//! stars. Each is a thin call into [`crate::services`]. There is no delete
//! tool: archiving is the agent's way to put a project away.

use pr_nexus_core::domain::{PrId, ProjectId};
use rmcp::handler::server::common::schema_for_type;
use rmcp::model::JsonObject;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::Capability;
use super::tools::{ToolSpec, parse, to_value};
use crate::error::AppError;
use crate::services::project_info::{self, JiraLinkInput};
use crate::services::{projects, prs};
use crate::state::AppState;

/// Most ids one call may name, as for `limit`.
const MAX_IDS: usize = prs::MAX_LIMIT;

pub fn write_tools() -> Vec<ToolSpec> {
    let spec =
        |name, title, description, destructive, idempotent, input_schema, handler| ToolSpec {
            name,
            title,
            description,
            capability: Capability::LocalWrites,
            destructive,
            idempotent,
            input_schema,
            handler,
        };
    vec![
        spec(
            "create_project",
            "Create project",
            "Create a project (a named group of pull requests). `name` is required; `description` (free text, \
                may span lines) and `color` (hex such as #3b82f6) are optional. Returns the new project; add PRs \
                with `add_prs_to_project`.",
            false,
            false,
            schema_for_type::<CreateProjectArgs>,
            create_project,
        ),
        spec(
            "update_project",
            "Update project",
            "Change a project's `name`, `description` or `color`. Only the fields you pass change; an empty \
                `description` clears it. Pass at least one field. Returns the project as `get_project` shows it.",
            true,
            true,
            schema_for_type::<UpdateProjectArgs>,
            update_project,
        ),
        spec(
            "archive_project",
            "Archive project",
            "Archive a project: it leaves the sidebar and `list_projects` (unless `include_archived`), keeping its \
                PRs, links and notes. Undo with `unarchive_project`. Projects cannot be deleted through this server.",
            false,
            true,
            schema_for_type::<ProjectIdArgs>,
            archive_project,
        ),
        spec(
            "unarchive_project",
            "Unarchive project",
            "Bring an archived project back to the sidebar and `list_projects`.",
            false,
            true,
            schema_for_type::<ProjectIdArgs>,
            unarchive_project,
        ),
        spec(
            "add_prs_to_project",
            "Add pull requests to project",
            "Link pull requests to a project by hand, by PR Nexus `id` (from `list_prs`, not GitHub numbers). \
                All or nothing: an unknown id changes nothing. A PR may be in several projects. Returns `changed` \
                (PRs that were not linked by hand before).",
            false,
            true,
            schema_for_type::<PrListArgs>,
            add_prs_to_project,
        ),
        spec(
            "remove_prs_from_project",
            "Remove pull requests from project",
            "Unlink pull requests from a project, by PR Nexus `id`. A removed PR also stays out when one of the \
                project's auto-link rules matches it, as when the user removes it in the app. Returns `removed`.",
            true,
            true,
            schema_for_type::<PrListArgs>,
            remove_prs_from_project,
        ),
        spec(
            "set_project_jira_links",
            "Set project JIRA links",
            "Replace all of a project's JIRA links with `links`, in order (an empty list removes them all). Each \
                entry is an issue key such as ACME-123 or a URL; a bare key opens under the JIRA base URL from \
                Settings. Nothing changes if one entry does not parse.",
            true,
            true,
            schema_for_type::<SetJiraLinksArgs>,
            set_project_jira_links,
        ),
        spec(
            "add_jira_link",
            "Add JIRA link",
            "Append one JIRA link to a project: an issue key such as ACME-123 or a URL. Returns the saved link \
                with `href`, where it opens (null for a bare key when no JIRA base URL is set).",
            false,
            false,
            schema_for_type::<JiraLinkArgs>,
            add_jira_link,
        ),
        spec(
            "remove_jira_link",
            "Remove JIRA link",
            "Remove one JIRA link from a project, named by its key (ACME-123) or its URL as `list_projects` shows \
                it. A `not_found` error when the project has no such link.",
            true,
            true,
            schema_for_type::<JiraLinkArgs>,
            remove_jira_link,
        ),
        spec(
            "set_pr_note",
            "Set pull request note",
            "Replace the user's private note on a pull request (PR Nexus `id`). An empty `note` deletes it. The \
                note replaces the old one entirely; read it first with `get_pr` to append.",
            true,
            true,
            schema_for_type::<SetNoteArgs>,
            set_pr_note,
        ),
        spec(
            "set_pr_starred",
            "Star or unstar pull request",
            "Star (`starred` true) or unstar a pull request by PR Nexus `id`. Starred PRs sort first in the app \
                and in `list_prs`.",
            false,
            true,
            schema_for_type::<SetStarredArgs>,
            set_pr_starred,
        ),
    ]
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectArgs {
    #[schemars(description = "Project name, unique among projects.")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "Optional free-text description.")]
    pub description: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional hex color such as #3b82f6.")]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProjectArgs {
    #[schemars(description = "Project id from list_projects.")]
    pub project_id: i64,
    #[serde(default)]
    #[schemars(description = "New name.")]
    pub name: Option<String>,
    #[serde(default)]
    #[schemars(description = "New description; empty clears it.")]
    pub description: Option<String>,
    #[serde(default)]
    #[schemars(description = "New hex color such as #3b82f6.")]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectIdArgs {
    #[schemars(description = "Project id from list_projects.")]
    pub project_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrListArgs {
    #[schemars(description = "Project id from list_projects.")]
    pub project_id: i64,
    #[schemars(
        description = "PR Nexus pull request ids from list_prs, 1 to 200 of them.",
        length(min = 1, max = 200)
    )]
    pub pr_ids: Vec<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetJiraLinksArgs {
    #[schemars(description = "Project id from list_projects.")]
    pub project_id: i64,
    #[schemars(
        description = "Issue keys (ACME-123) or URLs, in display order, at most 200.",
        length(max = 200)
    )]
    pub links: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JiraLinkArgs {
    #[schemars(description = "Project id from list_projects.")]
    pub project_id: i64,
    #[schemars(description = "An issue key such as ACME-123, or a URL.")]
    pub link: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetNoteArgs {
    #[schemars(description = "PR Nexus pull request id from list_prs (not the GitHub number).")]
    pub pr_id: i64,
    #[schemars(description = "The whole note; empty deletes it.")]
    pub note: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetStarredArgs {
    #[schemars(description = "PR Nexus pull request id from list_prs (not the GitHub number).")]
    pub pr_id: i64,
    #[schemars(description = "True to star, false to unstar.")]
    pub starred: bool,
}

fn require_project(state: &AppState, id: i64) -> Result<(), AppError> {
    match state.store.get_project(ProjectId(id))? {
        Some(_) => Ok(()),
        None => Err(AppError::new(
            "not_found",
            format!("No project with id {id}"),
        )),
    }
}

/// `{id, repo, number}` for the reply and the audit summary.
fn pr_ref(state: &AppState, id: i64) -> Result<Value, AppError> {
    let pr = state
        .store
        .get_pull_request(PrId(id))?
        .ok_or_else(|| AppError::new("not_found", format!("No pull request with id {id}")))?;
    Ok(json!({
        "id": id,
        "repo": format!("{}/{}", pr.repo.owner, pr.repo.name),
        "number": pr.number,
    }))
}

fn check_ids(state: &AppState, ids: &[i64]) -> Result<(), AppError> {
    if ids.is_empty() || ids.len() > MAX_IDS {
        return Err(AppError::new(
            "validation",
            format!("Invalid arguments: pr_ids must list 1 to {MAX_IDS} ids"),
        ));
    }
    let mut missing = Vec::new();
    for &id in ids {
        if state.store.get_pull_request(PrId(id))?.is_none() {
            missing.push(id.to_string());
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(AppError::new(
            "not_found",
            format!("No pull request with id {}", missing.join(", ")),
        ))
    }
}

fn summary(state: &AppState, id: i64) -> Result<Value, AppError> {
    Ok(json!({ "project": projects::project_summary(state, id)? }))
}

fn create_project(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: CreateProjectArgs = parse(arguments)?;
    let id = projects::create_project(state, args.name)?.id;
    if let Some(color) = args.color {
        projects::set_project_color(state, id, color)?;
    }
    if let Some(description) = args.description {
        project_info::set_project_description(state, id, description)?;
    }
    summary(state, id)
}

fn update_project(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: UpdateProjectArgs = parse(arguments)?;
    require_project(state, args.project_id)?;
    if args.name.is_none() && args.description.is_none() && args.color.is_none() {
        return Err(AppError::new(
            "validation",
            "Invalid arguments: pass name, description or color",
        ));
    }
    if let Some(name) = args.name {
        projects::rename_project(state, args.project_id, name)?;
    }
    if let Some(color) = args.color {
        projects::set_project_color(state, args.project_id, color)?;
    }
    if let Some(description) = args.description {
        project_info::set_project_description(state, args.project_id, description)?;
    }
    summary(state, args.project_id)
}

fn set_archived(
    state: &AppState,
    arguments: JsonObject,
    archived: bool,
) -> Result<Value, AppError> {
    let args: ProjectIdArgs = parse(arguments)?;
    require_project(state, args.project_id)?;
    projects::archive_project(state, args.project_id, archived)?;
    summary(state, args.project_id)
}

fn archive_project(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    set_archived(state, arguments, true)
}

fn unarchive_project(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    set_archived(state, arguments, false)
}

fn add_prs_to_project(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: PrListArgs = parse(arguments)?;
    require_project(state, args.project_id)?;
    check_ids(state, &args.pr_ids)?;
    let outcome = projects::link_prs(state, args.project_id, args.pr_ids)?;
    Ok(json!({ "changed": outcome.changed }))
}

fn remove_prs_from_project(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: PrListArgs = parse(arguments)?;
    require_project(state, args.project_id)?;
    check_ids(state, &args.pr_ids)?;
    for &pr_id in &args.pr_ids {
        projects::unlink_pr(state, args.project_id, pr_id)?;
    }
    Ok(json!({ "removed": args.pr_ids.len() }))
}

fn link_input(text: String) -> JiraLinkInput {
    JiraLinkInput {
        text: Some(text),
        ..JiraLinkInput::default()
    }
}

fn set_project_jira_links(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: SetJiraLinksArgs = parse(arguments)?;
    require_project(state, args.project_id)?;
    if args.links.len() > MAX_IDS {
        return Err(AppError::new(
            "validation",
            format!("Invalid arguments: at most {MAX_IDS} links"),
        ));
    }
    let links = args.links.into_iter().map(link_input).collect();
    Ok(json!({ "jira_links": project_info::set_jira_links(state, args.project_id, links)? }))
}

fn add_jira_link(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: JiraLinkArgs = parse(arguments)?;
    require_project(state, args.project_id)?;
    let link = project_info::add_jira_link(state, args.project_id, link_input(args.link))?;
    Ok(json!({ "jira_link": link }))
}

fn remove_jira_link(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: JiraLinkArgs = parse(arguments)?;
    require_project(state, args.project_id)?;
    let wanted = args.link.trim();
    let links = project_info::list_jira_links(state, args.project_id)?;
    let found = links.into_iter().find(|link| {
        (!link.key.is_empty() && link.key.eq_ignore_ascii_case(wanted))
            || (!link.url.is_empty() && link.url == wanted)
            || link.href.as_deref() == Some(wanted)
    });
    let Some(link) = found else {
        return Err(AppError::new(
            "not_found",
            format!("Project {} has no JIRA link {wanted}", args.project_id),
        ));
    };
    project_info::remove_jira_link(state, link.id)?;
    Ok(json!({ "removed": link }))
}

fn set_pr_note(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: SetNoteArgs = parse(arguments)?;
    let pr = pr_ref(state, args.pr_id)?;
    let saved = project_info::set_pr_note(state, args.pr_id, args.note)?;
    Ok(json!({ "pr": pr, "note": saved.note, "updated_at": saved.updated_at }))
}

fn set_pr_starred(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: SetStarredArgs = parse(arguments)?;
    let pr = pr_ref(state, args.pr_id)?;
    let starred = project_info::set_pr_starred(state, args.pr_id, args.starred)?;
    to_value(json!({ "pr": pr, "starred": starred }))
}
