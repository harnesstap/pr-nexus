//! The tools agents see. Each one is a thin call into [`crate::services`]
//! plus a schema and a description written for an agent.
//!
//! Adding a tool (phase 5): add a [`ToolSpec`] to [`registry`] with
//! the [`Capability`] it needs. Listing, gating and auditing follow.

use std::sync::Arc;

use rmcp::handler::server::common::schema_for_type;
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::Capability;
use crate::error::AppError;
use crate::services::projects;
use crate::services::prs::{self, PrFilter};
use crate::state::AppState;

pub type Handler = fn(&AppState, JsonObject) -> Result<Value, AppError>;

pub struct ToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub capability: Capability,
    /// May drop or overwrite data the user had (an agent should confirm).
    pub destructive: bool,
    /// Calling again with the same arguments changes nothing more.
    pub idempotent: bool,
    pub input_schema: fn() -> Arc<JsonObject>,
    pub handler: Handler,
}

impl ToolSpec {
    pub fn tool(&self) -> Tool {
        let read_only = matches!(
            self.capability,
            Capability::Reads | Capability::SlackReads | Capability::SlackPrivateSearch
        );
        debug_assert!(!read_only || (!self.destructive && self.idempotent));
        Tool::new(self.name, self.description, (self.input_schema)())
            .with_title(self.title)
            .with_annotations(
                ToolAnnotations::with_title(self.title)
                    .read_only(read_only)
                    .destructive(self.destructive)
                    .idempotent(self.idempotent)
                    .open_world(!matches!(
                        self.capability,
                        Capability::Reads | Capability::LocalWrites
                    )),
            )
    }
}

/// Every tool this build has, in the order `tools/list` returns them.
pub fn registry() -> Vec<ToolSpec> {
    let mut tools = read_tools();
    tools.extend(super::writes::write_tools());
    tools.extend(super::slack::slack_tools());
    tools
}

fn read_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "list_projects",
            title: "List projects",
            description: "List PR Nexus projects (user-defined groups of pull requests) in display order. \
                Each has `id`, `name`, `description`, `jira_links` (key, url, href), `archived`, \
                `pr_count` (linked, visible PRs in any state) and `open_pr_count`. \
                Archived projects are left out unless `include_archived` is true. \
                Use an `id` with `get_project` or as `project_id` in `list_prs`.",
            capability: Capability::Reads,
            destructive: false,
            idempotent: true,
            input_schema: schema_for_type::<ListProjectsArgs>,
            handler: list_projects,
        },
        ToolSpec {
            name: "get_project",
            title: "Get project",
            description: "One project with its description, JIRA links, counts and a page of its linked pull requests \
                (same item shape and order as `list_prs`: starred first, then most recently updated). \
                Page with `limit` and the returned `prs.next_cursor`.",
            capability: Capability::Reads,
            destructive: false,
            idempotent: true,
            input_schema: schema_for_type::<GetProjectArgs>,
            handler: get_project,
        },
        ToolSpec {
            name: "list_prs",
            title: "List pull requests",
            description: "List synced pull requests, starred first, then most recently updated. Every filter narrows; \
                omit a filter to not filter on it. `statuses` are PR Nexus's own derived statuses, as in the app's \
                filter menu (a PR matches if it is in any of them): draft; ready (every mandatory code owner covered); \
                blocked (owners or rules missing); unknown (readiness not worked out yet); no_review (nobody asked yet); \
                ci_trouble (failing checks or merge conflicts); declined (changes requested); merge_ready (approved, \
                green, no conflicts); waiting (asked for review over 48h ago, not approved); idle (no activity for 7 days). \
                Each item lists its `statuses`, `starred`, `project_ids`, checks, review state and Slack request (`slack`, \
                null when none). Returns `items`, `total` (over all pages) and `next_cursor` (null on the last page).",
            capability: Capability::Reads,
            destructive: false,
            idempotent: true,
            input_schema: schema_for_type::<PrFilter>,
            handler: list_prs,
        },
        ToolSpec {
            name: "get_pr",
            title: "Get pull request",
            description: "One pull request by PR Nexus `id` (from `list_prs`, not the GitHub number): the list fields \
                and statuses, the user's private `note` (empty when none), `starred`, the Slack review request (`slack`: \
                channel, requested_at, bump_count, permalink; null when none) and `branch` (behind count and update \
                state; null until the PR was opened once in the app). Reads local data only.",
            capability: Capability::Reads,
            destructive: false,
            idempotent: true,
            input_schema: schema_for_type::<GetPrArgs>,
            handler: get_pr,
        },
    ]
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ListProjectsArgs {
    /// Include archived projects. Default false.
    pub include_archived: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetProjectArgs {
    /// Project `id` from `list_projects`.
    pub project_id: i64,
    /// Linked PRs per page, 1 to 200. Default 50.
    #[serde(default)]
    #[schemars(range(min = 1, max = 200))]
    pub limit: Option<usize>,
    /// `prs.next_cursor` of the previous page.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPrArgs {
    /// PR Nexus pull request `id` from `list_prs` (not the GitHub number).
    pub pr_id: i64,
}

/// Arguments as `T`; a mismatch is a `validation` error naming the field.
pub fn parse<T: DeserializeOwned>(arguments: JsonObject) -> Result<T, AppError> {
    serde_json::from_value(Value::Object(arguments))
        .map_err(|err| AppError::new("validation", format!("Invalid arguments: {err}")))
}

/// `limit` outside 1..=200 is refused (not clamped), so an agent learns the
/// range instead of silently getting a different page size.
pub(super) fn check_limit(limit: Option<usize>) -> Result<(), AppError> {
    match limit {
        Some(limit) if !(1..=prs::MAX_LIMIT).contains(&limit) => Err(AppError::new(
            "validation",
            format!(
                "Invalid arguments: limit must be between 1 and {}",
                prs::MAX_LIMIT
            ),
        )),
        _ => Ok(()),
    }
}

pub(super) fn to_value<T: serde::Serialize>(value: T) -> Result<Value, AppError> {
    serde_json::to_value(value).map_err(|err| AppError::new("internal", err.to_string()))
}

fn list_projects(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: ListProjectsArgs = parse(arguments)?;
    Ok(json!({ "projects": projects::project_summaries(state, args.include_archived)? }))
}

fn get_project(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: GetProjectArgs = parse(arguments)?;
    check_limit(args.limit)?;
    let project = projects::project_summary(state, args.project_id)?;
    let page = prs::list_prs(
        state,
        &PrFilter {
            project_id: Some(args.project_id),
            limit: args.limit,
            cursor: args.cursor,
            ..PrFilter::default()
        },
    )?;
    Ok(json!({ "project": project, "prs": page }))
}

fn list_prs(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let filter: PrFilter = parse(arguments)?;
    check_limit(filter.limit)?;
    to_value(prs::list_prs(state, &filter)?)
}

fn get_pr(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: GetPrArgs = parse(arguments)?;
    to_value(prs::pr_info(state, args.pr_id)?)
}
