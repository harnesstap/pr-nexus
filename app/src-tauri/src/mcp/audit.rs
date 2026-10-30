//! One hook every MCP tool call goes through, allowed or not.
//!
//! No sink sees more than argument names, the ids a call named and a short
//! summary the server writes: notes, names and queries can be private.
//! [`StoreAudit`] keeps the `mcp_audit` table behind Settings → Agents →
//! Activity; [`LogAudit`] writes a log line.

use std::sync::Arc;

use chrono::Utc;
use pr_nexus_core::store::{AuditEntry, AuditOutcome};
use serde_json::{Map, Value};

use super::Capability;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// The tool ran and failed; `kind` is the [`crate::error::AppError`] kind.
    Failed {
        kind: String,
    },
    /// Refused before running: its capability is off.
    Denied,
}

#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub tool: String,
    pub capability: Capability,
    pub arguments: Map<String, Value>,
    pub outcome: Outcome,
    pub duration_ms: u64,
    /// `clientInfo.name` from `initialize`, when the client sent one.
    pub client: Option<String>,
    /// "starred acme/api#482": ids and verbs only, never argument text.
    pub summary: String,
    /// The error message shown to the agent, for denied and failed calls.
    pub error: Option<String>,
    /// The Slack message a successful write touched.
    pub link: Option<String>,
}

impl AuditEvent {
    /// Argument names, sorted.
    pub fn fields(&self) -> String {
        let mut names: Vec<&str> = self.arguments.keys().map(String::as_str).collect();
        names.sort_unstable();
        names.join(",")
    }

    /// `project:3,pr:12,pr:13` from the id arguments; nothing else.
    pub fn target_ids(&self) -> String {
        let mut out = Vec::new();
        if let Some(id) = self.arguments.get("project_id").and_then(Value::as_i64) {
            out.push(format!("project:{id}"));
        }
        if let Some(id) = self.arguments.get("pr_id").and_then(Value::as_i64) {
            out.push(format!("pr:{id}"));
        }
        if let Some(ids) = self.arguments.get("pr_ids").and_then(Value::as_array) {
            out.extend(
                ids.iter()
                    .filter_map(Value::as_i64)
                    .map(|id| format!("pr:{id}")),
            );
        }
        for key in [
            "channel_id",
            "user_id",
            "reference",
            "file_id",
            "canvas_id",
            "list_id",
        ] {
            if let Some(id) = self.arguments.get(key).and_then(Value::as_str)
                && let Some(id) = super::slack::ids::parse(id)
            {
                out.push(match id.qualifier {
                    Some(q) => format!("slack:{q}/{}", id.id),
                    None => format!("slack:{}", id.id),
                });
            }
        }
        if let Some(ws) = self.arguments.get("workspace").and_then(Value::as_str) {
            out.push(format!("slack:{}", ws.trim()));
        }
        out.join(",")
    }
}

/// "searched 2 Slack workspaces (1 failed)": counts only, never the query.
fn slack_summary(verb: &str, result: Option<&Value>) -> String {
    let count = |key: &str| {
        result
            .and_then(|value| value.get(key))
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    };
    let (ok, failed) = (count("results"), count("errors"));
    let n = ok + failed;
    let plural = if n == 1 { "" } else { "s" };
    match failed {
        0 => format!("{verb} {n} Slack workspace{plural}"),
        _ => format!("{verb} {n} Slack workspace{plural} ({failed} failed)"),
    }
}

/// "proposed a Slack message" / "sent a Slack message (auto-send)".
fn slack_write_summary(noun: &str, result: Option<&Value>) -> String {
    let status = result
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str);
    let decided_in = result
        .and_then(|value| value.get("decided_in"))
        .and_then(Value::as_str);
    match (status, decided_in) {
        (Some("sent"), Some(agent)) => format!("sent {noun}, approved in {agent}"),
        (Some("rejected"), Some(agent)) => format!("declined {noun} in {agent}; not sent"),
        _ => slack_write_status(noun, status, result),
    }
}

/// `#name` when PR Nexus has the channel cached, else its ID.
pub fn channel_label(name: Option<&str>, id: &str) -> String {
    match name.filter(|name| !name.is_empty()) {
        Some(name) => format!("#{name}"),
        None => id.to_owned(),
    }
}

/// "replied in a thread in #team-web (auto-send)": a reply is found by its
/// thread, not by its channel alone.
fn thread_reply_summary(arguments: &Map<String, Value>, result: Option<&Value>) -> String {
    let get = |key: &str| result.and_then(|v| v.get(key)).and_then(Value::as_str);
    let id = get("channel_id")
        .or_else(|| arguments.get("channel_id").and_then(Value::as_str))
        .unwrap_or_default();
    let place = format!("in a thread in {}", channel_label(get("channel_name"), id));
    match (get("status"), get("decided_in")) {
        (Some("sent"), Some(agent)) => format!("replied {place}, approved in {agent}"),
        (Some("sent"), None) => format!("replied {place} (auto-send)"),
        (Some("rejected"), Some(agent)) => {
            format!("declined a reply {place} in {agent}; not sent")
        }
        (Some("pending"), _) => match result
            .and_then(|v| v.pointer("/held/reason"))
            .and_then(Value::as_str)
        {
            Some("read_other_workspace") => {
                format!("proposed a reply {place}; auto-send blocked: read another workspace")
            }
            _ => format!("proposed a reply {place}"),
        },
        _ => format!("reply {place}"),
    }
}

/// The link Activity shows for a Slack write that went out.
pub fn write_link(result: Option<&Value>) -> Option<String> {
    let result = result?;
    if result.get("status").and_then(Value::as_str) != Some("sent") {
        return None;
    }
    result
        .get("link")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn slack_write_status(noun: &str, status: Option<&str>, result: Option<&Value>) -> String {
    match status {
        Some("sent") => format!("sent {noun} (auto-send)"),
        Some("pending") => {
            let held = result
                .and_then(|value| value.pointer("/held/reason"))
                .and_then(Value::as_str);
            match held {
                Some("read_other_workspace") => {
                    format!("proposed {noun}; auto-send blocked: read another workspace")
                }
                _ => format!("proposed {noun}"),
            }
        }
        _ => format!("send {noun}"),
    }
}

fn int(arguments: &Map<String, Value>, key: &str) -> String {
    arguments
        .get(key)
        .and_then(Value::as_i64)
        .map_or_else(|| "?".to_owned(), |id| id.to_string())
}

/// The Activity line for a call. `result` is the tool's reply when it ran;
/// only ids, counts and GitHub numbers are read from it.
pub fn summarize(tool: &str, arguments: &Map<String, Value>, result: Option<&Value>) -> String {
    let project = || format!("project {}", int(arguments, "project_id"));
    let pr = || {
        result
            .and_then(|value| value.get("pr"))
            .and_then(|pr| {
                Some(format!(
                    "{}#{}",
                    pr.get("repo")?.as_str()?,
                    pr.get("number")?
                ))
            })
            .unwrap_or_else(|| format!("PR id {}", int(arguments, "pr_id")))
    };
    let count = |key: &str| {
        let n = arguments
            .get(key)
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let noun = if key == "links" { "JIRA link" } else { "PR" };
        format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
    };
    let changed_fields = || {
        ["name", "description", "color"]
            .into_iter()
            .filter(|key| arguments.contains_key(*key))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match tool {
        "slack_list_workspaces" => "listed Slack workspaces".to_owned(),
        "slack_resolve" => "resolved a Slack reference".to_owned(),
        "slack_search_public" => slack_summary("searched messages in", result),
        "slack_search_channels" => slack_summary("searched channels in", result),
        "slack_search_users" => slack_summary("searched people in", result),
        "slack_list_user_channels" => slack_summary("listed channels in", result),
        "slack_read_channel" => "read a Slack channel".to_owned(),
        "slack_read_thread" => "read a Slack thread".to_owned(),
        "slack_read_user_profile" => "read a Slack profile".to_owned(),
        "slack_search_public_and_private" => {
            slack_summary("searched messages, private included, in", result)
        }
        "slack_search_emojis" => slack_summary("searched emoji in", result),
        "slack_get_reactions" => "read a Slack message's reactions".to_owned(),
        "slack_list_channel_members" => "listed a Slack channel's members".to_owned(),
        "slack_read_file" => "read a Slack file".to_owned(),
        "slack_read_canvas" => "read a Slack canvas".to_owned(),
        "slack_read_list" => "read a Slack list".to_owned(),
        "slack_send_message" if arguments.get("thread_ts").is_some_and(|ts| !ts.is_null()) => {
            thread_reply_summary(arguments, result)
        }
        "slack_send_message" => slack_write_summary("a Slack message", result),
        "slack_schedule_message" => slack_write_summary("a scheduled Slack message", result),
        "slack_send_message_draft" => slack_write_summary("a Slack draft", result),
        "slack_add_reaction" => slack_write_summary("a Slack reaction", result),
        "slack_get_proposal" => "checked a Slack proposal".to_owned(),
        "list_projects" => "listed projects".to_owned(),
        "get_project" => format!("read {}", project()),
        "list_prs" => "listed pull requests".to_owned(),
        "get_pr" => format!("read {}", pr()),
        "create_project" => match result.and_then(|value| value.pointer("/project/id")) {
            Some(id) => format!("created project {id}"),
            None => "create a project".to_owned(),
        },
        "update_project" => format!("updated {} ({})", project(), changed_fields()),
        "archive_project" => format!("archived {}", project()),
        "unarchive_project" => format!("unarchived {}", project()),
        "add_prs_to_project" => format!("added {} to {}", count("pr_ids"), project()),
        "remove_prs_from_project" => format!("removed {} from {}", count("pr_ids"), project()),
        "set_project_jira_links" => format!("set {} on {}", count("links"), project()),
        "add_jira_link" => match result.and_then(|value| value.pointer("/jira_link/key")) {
            Some(Value::String(key)) if !key.is_empty() => {
                format!("linked {key} to {}", project())
            }
            _ => format!("added a JIRA link to {}", project()),
        },
        "remove_jira_link" => match result.and_then(|value| value.pointer("/removed/key")) {
            Some(Value::String(key)) if !key.is_empty() => {
                format!("unlinked {key} from {}", project())
            }
            _ => format!("removed a JIRA link from {}", project()),
        },
        "set_pr_note" => {
            let empty = arguments
                .get("note")
                .and_then(Value::as_str)
                .is_some_and(|note| note.trim().is_empty());
            format!("{} note on {}", if empty { "cleared" } else { "set" }, pr())
        }
        "set_pr_starred" => {
            let on = arguments
                .get("starred")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            format!("{} {}", if on { "starred" } else { "unstarred" }, pr())
        }
        other => other.to_owned(),
    }
}

pub trait AuditSink: Send + Sync {
    fn record(&self, event: &AuditEvent);
}

/// Writes one log line per call.
pub struct LogAudit;

impl AuditSink for LogAudit {
    fn record(&self, event: &AuditEvent) {
        tracing::info!(
            tool = %event.tool,
            capability = ?event.capability,
            outcome = ?event.outcome,
            fields = %event.fields(),
            duration_ms = event.duration_ms,
            client = event.client.as_deref().unwrap_or("-"),
            "mcp tool call"
        );
    }
}

/// Keeps every call in the `mcp_audit` table (bounded on insert).
pub struct StoreAudit(pub Arc<AppState>);

impl AuditSink for StoreAudit {
    fn record(&self, event: &AuditEvent) {
        let outcome = match event.outcome {
            Outcome::Ok => AuditOutcome::Allowed,
            Outcome::Denied => AuditOutcome::Denied,
            Outcome::Failed { .. } => AuditOutcome::Error,
        };
        let capability = serde_json::to_value(event.capability)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        let entry = AuditEntry {
            id: 0,
            at: Utc::now(),
            client_name: event.client.clone().unwrap_or_default(),
            tool: event.tool.clone(),
            capability,
            outcome,
            fields: event.fields(),
            target_ids: event.target_ids(),
            summary: event.summary.clone(),
            error: event.error.clone(),
            duration_ms: i64::try_from(event.duration_ms).unwrap_or(i64::MAX),
            link: event.link.clone(),
        };
        if let Err(err) = self.0.store.append_mcp_audit(entry) {
            tracing::warn!(error = %err, "mcp audit row not saved");
        }
    }
}

/// Hands each call to several sinks, in order.
pub struct Fanout(pub Vec<Arc<dyn AuditSink>>);

impl AuditSink for Fanout {
    fn record(&self, event: &AuditEvent) {
        for sink in &self.0 {
            sink.record(event);
        }
    }
}
