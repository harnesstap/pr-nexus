//! `request_reviews`: the app's batch "Request review on Slack" for agents
//! (SPEC §10.1.5).
//!
//! The PRs are planned exactly like the bottom bar's batch: the same
//! routing, grouping (one card per channel), templates, draft warnings and
//! skips ([`bulk::preview`]). Each card then becomes one Slack send
//! [`Proposal`] under the user's send mode, confirmed in the agent when it
//! can ask, and is sent through [`bulk::send_card`], which records the
//! review requests the app shows (summary, Bump) like a batch sent from the
//! UI.

use std::collections::BTreeSet;

use chrono::Utc;
use pr_nexus_core::domain::PrId;
use rmcp::handler::server::common::schema_for_type;
use rmcp::model::JsonObject;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::elicit;
use super::sends::{self, Proposal, Status, agent_view, ask_in_agent, execute, hold, listed};
use super::session;
use super::tools::{exposed, resolve, validation, workspace_ref};
use super::upstream::Exposed;
use crate::error::AppError;
use crate::mcp::config::{self, Capability};
use crate::mcp::tools::{ToolSpec, parse};
use crate::services::bulk::{self, BulkCard, BulkPreview, BulkRequest};
use crate::services::review::ChatSource;
use crate::slack::{ReviewTarget, lock};
use crate::state::AppState;

pub fn review_tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "request_reviews",
        title: "Request reviews on Slack",
        description: "Ask for reviews of the user's open pull requests on Slack, like PR Nexus's batch \
            \"Request review on Slack\": one message per channel, the user's templates, a new \
            request or a reminder (bump in the request's thread) per PR. Without `destination` each PR \
            goes to the channels mapped to its owning teams. Use `preview: true` first to see the \
            exact messages; nothing is sent then. Otherwise each channel's message is sent at once or \
            waits for the user's approval, per their Slack send settings (as slack_send_message). The \
            reply has one entry per channel in `results` (`sent` with `link`, `pending` with \
            `proposal_id` to check with slack_get_proposal, `declined`, or `error`), the PRs \
            `skipped` and why, and `warnings` (drafts are sent, with a warning). Do not resend a \
            pending one.",
        capability: Capability::SlackSends,
        destructive: false,
        idempotent: false,
        input_schema: schema_for_type::<RequestReviewsArgs>,
        handler: request_reviews,
    }]
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestReviewsArgs {
    /// The pull requests: PR Nexus `id`s from list_prs, GitHub PR URLs, or
    /// `owner/repo#123`.
    pub prs: Vec<PrRef>,
    /// `auto` (default, as in the app): a new request, or a reminder when the
    /// PR already has an active request in that channel. `request` sends only
    /// new requests and `reminder` only reminders; PRs that would get the
    /// other kind are skipped.
    #[serde(default)]
    pub mode: Mode,
    /// Send everything to this one place instead of each PR's mapped
    /// channels: a channel or user ID as slack_resolve returns it
    /// (`<qualifier>/C…`), `C…@domain`, a Slack link, or a bare ID with
    /// `workspace`.
    pub destination: Option<String>,
    /// A workspace from slack_list_workspaces (qualifier, team ID or
    /// domain). With `destination`, the workspace of a bare ID; without, only
    /// that workspace's channels are used.
    pub workspace: Option<String>,
    /// Return the messages without sending anything.
    #[serde(default)]
    pub preview: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PrRef {
    Id(i64),
    Text(String),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Auto,
    Request,
    Reminder,
}

/// One channel's card, sent once its proposal goes out.
#[derive(Debug, Clone)]
pub struct Card {
    pub request: BulkRequest,
    pub fingerprint: String,
}

/// Where the planned cards come from; tests swap in a fake Slack.
fn with_chats<R>(f: impl FnOnce(&dyn ChatSource) -> R) -> R {
    #[cfg(test)]
    if let Some(fake) = FAKE.with(|cell| cell.borrow().clone()) {
        return f(fake.as_ref());
    }
    f(&crate::services::review::LiveChats)
}

#[cfg(test)]
thread_local! {
    pub(super) static FAKE: std::cell::RefCell<Option<std::sync::Arc<dyn ChatSource>>> =
        const { std::cell::RefCell::new(None) };
}

/// Send `card` now: Slack's outcome per PR, and the first message's link.
pub(super) fn send(state: &AppState, card: &Card) -> (Result<Value, AppError>, Option<String>) {
    let result =
        with_chats(|chats| bulk::send_card(state, chats, &card.request, &card.fingerprint));
    let result = match result {
        Ok(result) => result,
        Err(err) => return (Err(err), None),
    };
    let sent: Vec<_> = result
        .results
        .iter()
        .filter(|outcome| outcome.outcome == "sent")
        .collect();
    let link = sent.iter().find_map(|outcome| outcome.permalink.clone());
    let value = json!({
        "batch_id": result.batch_id,
        "results": result.results.iter().map(|outcome| json!({
            "pr_id": outcome.pr.pr_id,
            "repo": outcome.pr.repo,
            "number": outcome.pr.number,
            "outcome": outcome.outcome,
            "detail": outcome.detail,
            "permalink": outcome.permalink,
        })).collect::<Vec<_>>(),
    });
    if sent.is_empty() {
        let details: Vec<&str> = result
            .results
            .iter()
            .map(|outcome| outcome.detail.as_str())
            .filter(|detail| !detail.is_empty())
            .collect();
        let message = if details.is_empty() {
            "Nothing was sent".to_owned()
        } else {
            details.join("; ")
        };
        return (Err(AppError::new("api", message)), None);
    }
    (Ok(value), link)
}

fn request_reviews(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let args: RequestReviewsArgs = parse(arguments)?;
    if args.prs.is_empty() {
        return Err(validation("prs is empty"));
    }
    let (all, _) = exposed(state)?;
    let scope = match &args.workspace {
        Some(key) => Some(sends::target(state, key)?),
        None => None,
    };
    let target = match &args.destination {
        Some(destination) => Some(destination_target(
            state,
            &all,
            scope.as_ref(),
            destination,
        )?),
        None => None,
    };
    let pr_ids = pr_ids(state, &args.prs)?;
    let mut request = BulkRequest {
        pr_ids,
        target,
        single_top_level: Vec::new(),
    };
    let mut preview = bulk::preview(state, &request)?;
    let mut skipped = skips(&preview);
    // A strict mode drops PRs that would get the other kind of message.
    if args.mode != Mode::Auto {
        let wrong: BTreeSet<i64> = preview
            .cards
            .iter()
            .flat_map(|card| &card.messages)
            .filter(|message| (message.action == "request") != (args.mode == Mode::Request))
            .flat_map(|message| message.prs.iter().map(|pr| pr.pr_id))
            .collect();
        if !wrong.is_empty() {
            for card in &preview.cards {
                for message in &card.messages {
                    for pr in &message.prs {
                        if wrong.contains(&pr.pr_id)
                            && !skipped.iter().any(|s| s["pr_id"] == pr.pr_id)
                        {
                            skipped.push(json!({
                                "pr_id": pr.pr_id,
                                "repo": pr.repo,
                                "number": pr.number,
                                "detail": match args.mode {
                                    Mode::Request => "It already has an active request; it would get a reminder (mode is request)",
                                    _ => "It has no active request to remind; it would get a new request (mode is reminder)",
                                },
                            }));
                        }
                    }
                }
            }
            request.pr_ids.retain(|id| !wrong.contains(id));
            if request.pr_ids.is_empty() {
                preview.cards.clear();
            } else {
                preview = bulk::preview(state, &request)?;
            }
        }
    }
    let warnings: Vec<Value> = preview
        .pr_warnings
        .iter()
        .map(|w| json!({ "kind": w.kind, "pr_id": w.pr_id, "number": w.number, "message": w.message }))
        .collect();
    let mut cards = Vec::new();
    let mut results = Vec::new();
    for card in preview.cards {
        let ws = all.iter().find(|ws| owns(ws, &card.target));
        match (ws, &scope) {
            (Some(ws), Some(only)) if ws.qualifier() != only.qualifier() => {
                for pr in card.messages.iter().flat_map(|m| &m.prs) {
                    skipped.push(json!({
                        "pr_id": pr.pr_id, "repo": pr.repo, "number": pr.number,
                        "detail": format!("Its channel {} is in another workspace", channel_label(&card.target)),
                    }));
                }
            }
            (Some(ws), _) => cards.push((ws.clone(), card)),
            (None, _) => results.push(json!({
                "destination": destination_json(None, &card.target),
                "status": "error",
                "prs": card_prs(&card),
                "error": { "kind": "forbidden", "message": "This channel's Slack workspace is not exposed to agents in PR Nexus → Settings → Agents." },
            })),
        }
    }
    if args.preview {
        return Ok(json!({
            "preview": true,
            "mode": args.mode,
            "cards": cards.iter().map(|(ws, card)| json!({
                "destination": destination_json(Some(ws), &card.target),
                "signed_in": card.signed_in,
                "messages": card.messages.iter().map(|m| json!({
                    "action": m.action,
                    "prs": m.prs,
                    "text": m.text,
                    "thread": m.permalink,
                })).collect::<Vec<_>>(),
                "warnings": card.warnings,
            })).collect::<Vec<_>>(),
            "unavailable": results,
            "skipped": skipped,
            "warnings": warnings,
        }));
    }
    for (ws, card) in cards {
        results.push(propose(state, &ws, &request, card));
    }
    let sent = results.iter().filter(|r| r["status"] == "sent").count();
    let link = results
        .iter()
        .find_map(|r| r["link"].as_str().map(str::to_owned));
    Ok(json!({
        "preview": false,
        "mode": args.mode,
        // For the activity log: "sent" when every channel went out.
        "status": if sent == results.len() && sent > 0 { "sent" } else if sent > 0 { "partial" } else { "not_sent" },
        "link": link,
        "results": results,
        "skipped": skipped,
        "warnings": warnings,
    }))
}

/// One channel's proposal: sent now, asked in the agent, or left pending.
fn propose(state: &AppState, ws: &Exposed, request: &BulkRequest, card: BulkCard) -> Value {
    let caller = session::current();
    let settings = match config::load(&state.store) {
        Ok(settings) => settings,
        Err(err) => return failed(ws, &card, &err),
    };
    let channel = card.target.channel_id.clone();
    let others = state
        .slack_proxy
        .read_elsewhere(caller.session.as_deref(), ws.qualifier());
    let held = hold(settings.slack_send_mode, others, || {
        listed(&settings.slack_auto_send_channels, ws, &channel)
    });
    let text = card
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    let mut arguments = JsonObject::new();
    arguments.insert("channel_id".into(), json!(channel));
    arguments.insert("message".into(), json!(text));
    arguments.insert(
        "pr_ids".into(),
        json!(
            card_prs(&card)
                .iter()
                .map(|p| p["pr_id"].clone())
                .collect::<Vec<_>>()
        ),
    );
    let proposal = lock(&state.slack_proxy.proposals).push(Proposal {
        id: 0,
        created_at: Utc::now().to_rfc3339(),
        client: caller.client.clone(),
        tool: "request_reviews".to_owned(),
        workspace: workspace_ref(ws),
        channel_id: format!("{}/{channel}", ws.qualifier()),
        arguments,
        status: Status::Pending,
        auto_sent: held.is_none(),
        held,
        result: None,
        error: None,
        decided_at: None,
        decided_in: None,
        unanswered: None,
        link: None,
        channel_name: Some(card.target.channel_name.clone()).filter(|n| !n.is_empty()),
        session: caller.session.clone(),
        review: Some(Card {
            request: request.clone(),
            fingerprint: card.fingerprint.clone(),
        }),
    });
    let view = if proposal.held.is_some() {
        match caller.ask {
            Some(ask)
                if elicit::should_ask(proposal.held.as_ref(), settings.slack_elicit, true) =>
            {
                ask_in_agent(state, ws, proposal, ask.as_ref(), caller.client)
            }
            _ => Ok(agent_view(&proposal)),
        }
    } else {
        execute(state, ws, proposal.id).map(|p| agent_view(&p))
    };
    match view {
        Ok(view) => result_entry(ws, &card, &view),
        Err(err) => failed(ws, &card, &err),
    }
}

fn result_entry(ws: &Exposed, card: &BulkCard, view: &Value) -> Value {
    let status = match view["status"].as_str() {
        Some("sent") => "sent",
        Some("pending") => "pending",
        Some("rejected") => "declined",
        _ => "error",
    };
    json!({
        "destination": destination_json(Some(ws), &card.target),
        "status": status,
        "proposal_id": view["proposal_id"],
        "auto_sent": view["auto_sent"],
        "held": view["held"],
        "decided_in": view["decided_in"],
        "link": view["link"],
        "prs": card_prs(card),
        "result": view["result"],
        "error": view["error"],
        "message": view["message"],
    })
}

fn failed(ws: &Exposed, card: &BulkCard, err: &AppError) -> Value {
    json!({
        "destination": destination_json(Some(ws), &card.target),
        "status": "error",
        "prs": card_prs(card),
        "error": { "kind": err.kind, "message": err.message },
    })
}

fn card_prs(card: &BulkCard) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    card.messages
        .iter()
        .flat_map(|m| m.prs.iter().map(move |pr| (m.action.clone(), pr)))
        .filter(|(_, pr)| seen.insert(pr.pr_id))
        .map(|(action, pr)| {
            json!({ "pr_id": pr.pr_id, "repo": pr.repo, "number": pr.number, "title": pr.title, "action": action })
        })
        .collect()
}

fn skips(preview: &BulkPreview) -> Vec<Value> {
    preview
        .skipped
        .iter()
        .map(|s| json!({ "pr_id": s.pr.pr_id, "repo": s.pr.repo, "number": s.pr.number, "detail": s.detail }))
        .collect()
}

fn owns(ws: &Exposed, target: &ReviewTarget) -> bool {
    match target.team_id.as_deref().filter(|t| !t.is_empty()) {
        Some(team) => ws.team_id == team,
        None => ws.domain.eq_ignore_ascii_case(&target.workspace),
    }
}

fn channel_label(target: &ReviewTarget) -> String {
    if target.channel_name.is_empty() {
        target.channel_id.clone()
    } else if target.channel_id.starts_with(['U', 'W']) {
        format!("@{}", target.channel_name)
    } else {
        format!("#{}", target.channel_name)
    }
}

fn destination_json(ws: Option<&Exposed>, target: &ReviewTarget) -> Value {
    json!({
        "workspace": ws.map(workspace_ref),
        "channel_id": match ws {
            Some(ws) => format!("{}/{}", ws.qualifier(), target.channel_id),
            None => target.channel_id.clone(),
        },
        "channel_name": Some(target.channel_name.clone()).filter(|n| !n.is_empty()),
    })
}

/// `destination` as the batch's one target, resolved like slack_resolve.
fn destination_target(
    state: &AppState,
    all: &[Exposed],
    scope: Option<&Exposed>,
    destination: &str,
) -> Result<ReviewTarget, AppError> {
    let destination = destination.trim();
    let bare = !destination.contains(['/', '@', ':']);
    let reference = match scope {
        Some(ws) if bare => format!("{destination}@{}", ws.domain),
        _ => destination.to_owned(),
    };
    let mut args = JsonObject::new();
    args.insert("reference".into(), json!(reference));
    let resolved = resolve(state, args)?;
    let qualified = resolved["channel_id"]
        .as_str()
        .or_else(|| resolved["id"].as_str())
        .unwrap_or_default();
    let (qualifier, id) = qualified
        .split_once('/')
        .ok_or_else(|| validation("destination is not a channel or a person"))?;
    if !id.starts_with(['C', 'G', 'D', 'U', 'W']) {
        return Err(validation("destination is not a channel or a person"));
    }
    let ws = all
        .iter()
        .find(|ws| ws.qualifier() == qualifier)
        .ok_or_else(|| AppError::new("not_found", "That workspace is not exposed to agents."))?;
    if let Some(only) = scope
        && only.qualifier() != ws.qualifier()
    {
        return Err(validation(format!(
            "destination belongs to workspace {}, not {}",
            ws.qualifier(),
            only.qualifier()
        )));
    }
    let channel_name = state
        .store
        .get_slack_channel(ws.domain.clone(), id.to_owned())
        .ok()
        .flatten()
        .map(|cached| cached.name)
        .unwrap_or_default();
    Ok(ReviewTarget {
        workspace: ws.domain.clone(),
        team_id: Some(ws.team_id.clone()),
        workspace_name: ws.name.clone(),
        channel_id: id.to_owned(),
        channel_name,
    })
}

/// PR Nexus ids for ids, GitHub PR URLs and `owner/repo#N`.
fn pr_ids(state: &AppState, refs: &[PrRef]) -> Result<Vec<i64>, AppError> {
    let mut known = None;
    let mut out = Vec::new();
    for item in refs {
        let id = match item {
            PrRef::Id(id) => *id,
            PrRef::Text(text) => {
                let text = text.trim();
                if let Ok(id) = text.parse::<i64>() {
                    id
                } else {
                    let (owner, name, number) = parse_pr(text).ok_or_else(|| {
                        validation(format!(
                            "{text:?} is not a PR id, a GitHub PR URL or owner/repo#number"
                        ))
                    })?;
                    if known.is_none() {
                        known = Some(match crate::state::github_account(&state.store)? {
                            Some(account) => {
                                state.store.list_pull_requests_for_account(account.id)?
                            }
                            None => Vec::new(),
                        });
                    }
                    known
                        .as_ref()
                        .into_iter()
                        .flatten()
                        .find(|pr| {
                            pr.number == number
                                && pr.repo.owner.eq_ignore_ascii_case(&owner)
                                && pr.repo.name.eq_ignore_ascii_case(&name)
                        })
                        .map(|pr| pr.id.0)
                        .ok_or_else(|| {
                            AppError::new(
                                "not_found",
                                format!("PR Nexus does not track {owner}/{name}#{number}"),
                            )
                        })?
                }
            }
        };
        if state.store.get_pull_request(PrId(id))?.is_none() {
            return Err(AppError::new(
                "not_found",
                format!("No pull request with id {id}"),
            ));
        }
        out.push(id);
    }
    Ok(out)
}

/// `https://github.com/o/r/pull/12` or `o/r#12`.
fn parse_pr(text: &str) -> Option<(String, String, u64)> {
    if let Some((repo, number)) = text.split_once('#') {
        let (owner, name) = repo.split_once('/')?;
        return Some((owner.to_owned(), name.to_owned(), number.parse().ok()?));
    }
    let path = text.split_once("://").map_or(text, |(_, rest)| rest);
    let mut parts = path
        .split(['/', '?', '#'])
        .filter(|p| !p.is_empty())
        .skip(1);
    let owner = parts.next()?;
    let name = parts.next()?;
    if parts.next()? != "pull" {
        return None;
    }
    Some((
        owner.to_owned(),
        name.to_owned(),
        parts.next()?.parse().ok()?,
    ))
}
