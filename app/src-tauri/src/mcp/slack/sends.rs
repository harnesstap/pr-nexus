//! Slack writes agents ask for: send, schedule, draft, react (SPEC §10.1.2).
//!
//! Each call names its workspace and becomes a [`Proposal`]. Whether it
//! waits for the user follows [`SlackSendMode`]: `approve_all` holds every
//! write; `automatic` sends every write at once; `allowlist` sends at once
//! only when the channel is on the auto-send list and the session has read
//! no other workspace, so content read in one workspace cannot be carried
//! to another unseen. A held proposal is approved or rejected in Settings
//! → Agents, and only then does PR Nexus call Slack.
//! Slack's reply comes back as Slack sent it.

use std::collections::VecDeque;

use chrono::Utc;
use rmcp::handler::server::common::schema_for_type;
use rmcp::model::JsonObject;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::elicit::{self, Outcome};
use super::ids;
use super::session;
use super::tools::{exposed, matches, run, validation, workspace_ref};
use super::upstream::Exposed;
use crate::error::AppError;
use crate::mcp::config::{self, Capability, SlackSendMode};
use crate::mcp::tools::{ToolSpec, parse};
use crate::slack::lock;
use crate::state::AppState;

/// Proposals kept, newest last; the oldest go first.
pub const KEPT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Sent,
    Failed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum Held {
    /// The user approves every write (`approve_all` mode).
    ApproveAll,
    /// The channel is not on the auto-send list.
    NotListed,
    /// The session read these other workspaces first.
    ReadOtherWorkspace { workspaces: Vec<String> },
}

#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub id: u64,
    pub created_at: String,
    pub client: Option<String>,
    /// The official tool PR Nexus calls once approved.
    pub tool: String,
    pub workspace: Value,
    /// Qualified (`<qualifier>/C…`).
    pub channel_id: String,
    /// What Slack receives, bare IDs.
    pub arguments: JsonObject,
    pub status: Status,
    /// Why it waited for approval; `None` when it went out by itself.
    pub held: Option<Held>,
    pub auto_sent: bool,
    /// Slack's reply, IDs qualified, otherwise as Slack sent it.
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub decided_at: Option<String>,
    /// The agent (client name) where the user approved or declined it
    /// through MCP elicitation; `None` when decided in PR Nexus.
    pub decided_in: Option<String>,
    /// Why asking in the agent got no answer, so it waits in PR Nexus.
    pub unanswered: Option<String>,
    /// The Slack message once sent (see [`super::link`]).
    pub link: Option<String>,
    /// The channel's name when PR Nexus has it cached.
    pub channel_name: Option<String>,
    #[serde(skip)]
    session: Option<String>,
}

#[derive(Debug, Default)]
pub struct Proposals {
    next: u64,
    items: VecDeque<Proposal>,
}

impl Proposals {
    fn push(&mut self, mut proposal: Proposal) -> Proposal {
        self.next += 1;
        proposal.id = self.next;
        if self.items.len() >= KEPT {
            self.items.pop_front();
        }
        self.items.push_back(proposal.clone());
        proposal
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Proposal> {
        self.items.iter_mut().find(|p| p.id == id)
    }
}

pub fn send_tools() -> Vec<ToolSpec> {
    let spec = |name, title, description, input_schema, handler| ToolSpec {
        name,
        title,
        description,
        capability: Capability::SlackSends,
        destructive: false,
        idempotent: false,
        input_schema,
        handler,
    };
    const AFTER: &str = " Name the workspace in `workspace` (a qualifier, team ID or domain from \
        slack_list_workspaces). Depending on the user's settings it is sent at once \
        (`status` `sent`) or waits for their approval in PR Nexus (`status` `pending`, with `held` \
        saying why). The reply has `proposal_id`, `status` and `message`; while it is `pending`, \
        check it with slack_get_proposal and do not resend it.";
    vec![
        spec(
            "slack_send_message",
            "Send a Slack message",
            leak(format!(
                "Send a message to a channel, or to a person with their user ID as `channel_id`; \
                    `thread_ts` replies in a thread.{AFTER}"
            )),
            schema_for_type::<SendArgs>,
            |state, args| propose::<SendArgs>(state, args, "slack_send_message"),
        ),
        spec(
            "slack_schedule_message",
            "Schedule a Slack message",
            leak(format!(
                "Schedule a message for `post_at` (Unix seconds, at least 2 minutes ahead and at most \
                    120 days).{AFTER}"
            )),
            schema_for_type::<ScheduleArgs>,
            |state, args| propose::<ScheduleArgs>(state, args, "slack_schedule_message"),
        ),
        spec(
            "slack_send_message_draft",
            "Draft a Slack message",
            leak(format!(
                "Save a draft in the user's Slack \"Drafts & Sent\" for a channel, without sending it.{AFTER}"
            )),
            schema_for_type::<DraftArgs>,
            |state, args| propose::<DraftArgs>(state, args, "slack_send_message_draft"),
        ),
        spec(
            "slack_add_reaction",
            "React to a Slack message",
            leak(format!(
                "Add an emoji reaction (name without colons) to the message `message_ts`.{AFTER}"
            )),
            schema_for_type::<ReactionArgs>,
            |state, args| propose::<ReactionArgs>(state, args, "slack_add_reaction"),
        ),
        ToolSpec {
            name: "slack_get_proposal",
            title: "Check a Slack send",
            description: "The state of a send, schedule, draft or reaction this session asked for: \
                `pending` (waiting for the user), `sent` (with Slack's reply in `result`), `failed` \
                (Slack's error) or `rejected` by the user.",
            capability: Capability::SlackSends,
            destructive: false,
            idempotent: true,
            input_schema: schema_for_type::<ProposalArgs>,
            handler: get_proposal,
        },
    ]
}

/// Tool descriptions live for the whole run.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// Arguments of one write: the workspace, the channel and what Slack gets.
trait WriteArgs: serde::de::DeserializeOwned {
    fn workspace(&self) -> &str;
    fn channel_id(&self) -> &str;
    /// Slack's own arguments, `channel_id` left out.
    fn rest(&self) -> JsonObject;
}

fn put(out: &mut JsonObject, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        out.insert(key.to_owned(), value);
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendArgs {
    /// Workspace to send in (required).
    pub workspace: String,
    /// Channel, DM or user ID; qualified IDs must be this workspace's.
    pub channel_id: String,
    /// Message, Slack markdown.
    pub message: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
    /// With `thread_ts`: also post the reply to the channel.
    #[serde(default)]
    pub reply_broadcast: Option<bool>,
}

impl WriteArgs for SendArgs {
    fn workspace(&self) -> &str {
        &self.workspace
    }
    fn channel_id(&self) -> &str {
        &self.channel_id
    }
    fn rest(&self) -> JsonObject {
        let mut out = JsonObject::new();
        put(&mut out, "message", Some(json!(self.message)));
        put(
            &mut out,
            "thread_ts",
            self.thread_ts.clone().map(Value::String),
        );
        put(
            &mut out,
            "reply_broadcast",
            self.reply_broadcast.map(Value::Bool),
        );
        out
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleArgs {
    /// Workspace to send in (required).
    pub workspace: String,
    /// Channel, DM or user ID; qualified IDs must be this workspace's.
    pub channel_id: String,
    /// Message, Slack markdown.
    pub message: String,
    /// Unix seconds.
    pub post_at: i64,
    #[serde(default)]
    pub thread_ts: Option<String>,
    #[serde(default)]
    pub reply_broadcast: Option<bool>,
}

impl WriteArgs for ScheduleArgs {
    fn workspace(&self) -> &str {
        &self.workspace
    }
    fn channel_id(&self) -> &str {
        &self.channel_id
    }
    fn rest(&self) -> JsonObject {
        let mut out = JsonObject::new();
        put(&mut out, "message", Some(json!(self.message)));
        put(&mut out, "post_at", Some(json!(self.post_at)));
        put(
            &mut out,
            "thread_ts",
            self.thread_ts.clone().map(Value::String),
        );
        put(
            &mut out,
            "reply_broadcast",
            self.reply_broadcast.map(Value::Bool),
        );
        out
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DraftArgs {
    /// Workspace of the draft (required).
    pub workspace: String,
    /// Channel, DM or user ID; qualified IDs must be this workspace's.
    pub channel_id: String,
    /// Draft, Slack markdown.
    pub message: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
}

impl WriteArgs for DraftArgs {
    fn workspace(&self) -> &str {
        &self.workspace
    }
    fn channel_id(&self) -> &str {
        &self.channel_id
    }
    fn rest(&self) -> JsonObject {
        let mut out = JsonObject::new();
        put(&mut out, "message", Some(json!(self.message)));
        put(
            &mut out,
            "thread_ts",
            self.thread_ts.clone().map(Value::String),
        );
        out
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReactionArgs {
    /// Workspace of the message (required).
    pub workspace: String,
    /// Channel of the message; qualified IDs must be this workspace's.
    pub channel_id: String,
    /// Timestamp of the message.
    pub message_ts: String,
    /// Emoji name without colons, e.g. `eyes`.
    pub emoji: String,
}

impl WriteArgs for ReactionArgs {
    fn workspace(&self) -> &str {
        &self.workspace
    }
    fn channel_id(&self) -> &str {
        &self.channel_id
    }
    fn rest(&self) -> JsonObject {
        let mut out = JsonObject::new();
        put(&mut out, "message_ts", Some(json!(self.message_ts)));
        put(&mut out, "emoji", Some(json!(self.emoji)));
        out
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalArgs {
    /// `proposal_id` from the send.
    pub proposal_id: u64,
}

/// The one exposed workspace `workspace` names; "all" and none are refused.
fn target(state: &AppState, workspace: &str) -> Result<Exposed, AppError> {
    let workspace = workspace.trim();
    if workspace.is_empty() || workspace.eq_ignore_ascii_case("all") {
        return Err(validation(
            "workspace must name one workspace slack_list_workspaces returns",
        ));
    }
    let (all, _) = exposed(state)?;
    all.into_iter()
        .find(|ws| matches(ws, workspace))
        .ok_or_else(|| {
            validation(format!(
                "workspace {workspace:?} is not one slack_list_workspaces returns"
            ))
        })
}

/// The bare channel ID, refusing one qualified for another workspace.
fn channel_in(ws: &Exposed, value: &str) -> Result<String, AppError> {
    let id = ids::parse(value)
        .ok_or_else(|| validation(format!("channel_id {value:?} is not a Slack ID")))?;
    match id.qualifier {
        Some(q) if q != ws.qualifier() && q != ws.team_id => Err(validation(format!(
            "channel_id belongs to workspace {q}, not {}",
            ws.qualifier()
        ))),
        _ => Ok(id.id),
    }
}

/// Whether `channel` (qualified) is on the auto-send list.
fn listed(list: &[String], ws: &Exposed, channel: &str) -> bool {
    list.iter().any(|entry| {
        ids::parse(entry).is_some_and(|id| {
            id.id == channel
                && id
                    .qualifier
                    .is_some_and(|q| q == ws.qualifier() || q == ws.team_id)
        })
    })
}

/// Why a write waits for approval under `mode`, or `None` to send it now.
/// `others` are the workspaces this session read besides the target.
pub(crate) fn hold(
    mode: SlackSendMode,
    others: Vec<String>,
    listed: impl FnOnce() -> bool,
) -> Option<Held> {
    match mode {
        SlackSendMode::ApproveAll => Some(Held::ApproveAll),
        // The user asked for no approval at all, safeguard included.
        SlackSendMode::Automatic => None,
        SlackSendMode::Allowlist if !others.is_empty() => {
            Some(Held::ReadOtherWorkspace { workspaces: others })
        }
        SlackSendMode::Allowlist if !listed() => Some(Held::NotListed),
        SlackSendMode::Allowlist => None,
    }
}

fn propose<A: WriteArgs>(
    state: &AppState,
    arguments: JsonObject,
    tool: &str,
) -> Result<Value, AppError> {
    let args: A = parse(arguments)?;
    let ws = target(state, args.workspace())?;
    let channel = channel_in(&ws, args.channel_id())?;
    let mut upstream = JsonObject::new();
    upstream.insert("channel_id".into(), json!(channel));
    upstream.extend(args.rest());
    let caller = session::current();
    let others = state
        .slack_proxy
        .read_elsewhere(caller.session.as_deref(), ws.qualifier());
    let settings = config::load(&state.store)?;
    let held = hold(settings.slack_send_mode, others, || {
        listed(&settings.slack_auto_send_channels, &ws, &channel)
    });
    let proposal = lock(&state.slack_proxy.proposals).push(Proposal {
        id: 0,
        created_at: Utc::now().to_rfc3339(),
        client: caller.client.clone(),
        tool: tool.to_owned(),
        workspace: workspace_ref(&ws),
        channel_id: format!("{}/{channel}", ws.qualifier()),
        arguments: upstream,
        status: Status::Pending,
        auto_sent: held.is_none(),
        held,
        result: None,
        error: None,
        decided_at: None,
        decided_in: None,
        unanswered: None,
        link: None,
        channel_name: state
            .store
            .get_slack_channel(ws.domain.clone(), channel.clone())
            .ok()
            .flatten()
            .map(|cached| cached.name),
        session: caller.session,
    });
    if proposal.held.is_some() {
        return match caller.ask {
            Some(ask)
                if elicit::should_ask(proposal.held.as_ref(), settings.slack_elicit, true) =>
            {
                ask_in_agent(state, &ws, proposal, ask.as_ref(), caller.client)
            }
            _ => Ok(agent_view(&proposal)),
        };
    }
    let proposal = execute(state, &ws, proposal.id)?;
    match &proposal.error {
        // Sent by itself: Slack's error is the call's error, as for a read.
        Some(error) => Err(AppError::new(
            error["kind"].as_str().unwrap_or("api"),
            error["message"].as_str().unwrap_or_default(),
        )),
        None => Ok(agent_view(&proposal)),
    }
}

/// Ask the user to confirm held `proposal` in their agent, then send it,
/// drop it, or leave it waiting in PR Nexus.
fn ask_in_agent(
    state: &AppState,
    ws: &Exposed,
    proposal: Proposal,
    ask: &dyn session::Ask,
    client: Option<String>,
) -> Result<Value, AppError> {
    let text = elicit::message(
        &proposal.tool,
        &proposal.workspace,
        &proposal.channel_id,
        &Value::Object(proposal.arguments.clone()),
    );
    let agent = client.unwrap_or_else(|| "the agent".to_owned());
    let id = proposal.id;
    let outcome = elicit::outcome(ask.ask(&text));
    {
        let mut proposals = lock(&state.slack_proxy.proposals);
        let Some(proposal) = proposals.get_mut(id) else {
            return Err(AppError::new(
                "not_found",
                format!("No Slack proposal {id}"),
            ));
        };
        // Decided in PR Nexus while the agent asked: that answer stands.
        if proposal.status != Status::Pending {
            return Ok(agent_view(proposal));
        }
        match &outcome {
            Outcome::Send => proposal.decided_in = Some(agent.clone()),
            Outcome::Drop => {
                proposal.status = Status::Rejected;
                proposal.decided_in = Some(agent.clone());
                proposal.decided_at = Some(Utc::now().to_rfc3339());
            }
            Outcome::Wait(why) => proposal.unanswered = Some(why.clone()),
        }
        if outcome != Outcome::Send {
            return Ok(agent_view(proposal));
        }
    }
    match execute(state, ws, id) {
        Ok(proposal) => Ok(agent_view(&proposal)),
        // Approved in PR Nexus at the same moment: report where it stands.
        Err(err) if err.kind == "conflict" => {
            let proposals = lock(&state.slack_proxy.proposals);
            let found = proposals.items.iter().find(|p| p.id == id);
            found.map(agent_view).ok_or(err)
        }
        Err(err) => Err(err),
    }
}

/// Call Slack for proposal `id` and record how it went.
fn execute(state: &AppState, ws: &Exposed, id: u64) -> Result<Proposal, AppError> {
    let (tool, arguments) = {
        let mut proposals = lock(&state.slack_proxy.proposals);
        let proposal = proposals
            .get_mut(id)
            .ok_or_else(|| AppError::new("not_found", format!("No Slack proposal {id}")))?;
        if proposal.status != Status::Pending {
            return Err(AppError::new(
                "conflict",
                format!("Slack proposal {id} is already {:?}", proposal.status).to_lowercase(),
            ));
        }
        // Claimed before calling Slack, so a double click sends once.
        proposal.status = Status::Sent;
        (proposal.tool.clone(), proposal.arguments.clone())
    };
    let outcome = run(state, &tool, vec![ws.clone()], None, |_| arguments.clone());
    let link = outcome
        .as_ref()
        .ok()
        .and_then(|reply| super::link::for_write(&ws.domain, &tool, &arguments, reply));
    let mut proposals = lock(&state.slack_proxy.proposals);
    let proposal = proposals
        .get_mut(id)
        .ok_or_else(|| AppError::new("not_found", format!("No Slack proposal {id}")))?;
    proposal.decided_at = Some(Utc::now().to_rfc3339());
    match outcome {
        Ok(value) => {
            proposal.result = Some(value);
            proposal.link = link;
        }
        Err(err) => {
            proposal.status = Status::Failed;
            proposal.error = Some(json!({ "kind": err.kind, "message": err.message }));
        }
    }
    Ok(proposal.clone())
}

/// What the agent sees of a proposal.
fn agent_view(proposal: &Proposal) -> Value {
    json!({
        "proposal_id": proposal.id,
        "status": proposal.status,
        "tool": proposal.tool,
        "workspace": proposal.workspace,
        "channel_id": proposal.channel_id,
        "auto_sent": proposal.auto_sent,
        "held": proposal.held,
        "result": proposal.result,
        "error": proposal.error,
        "decided_in": proposal.decided_in,
        "link": proposal.link,
        "channel_name": proposal.channel_name,
        "message": agent_message(proposal),
    })
}

fn agent_message(proposal: &Proposal) -> String {
    let place = proposal.decided_in.as_deref();
    match (proposal.status, place) {
        (Status::Pending, None) if proposal.unanswered.is_some() => {
            "The user did not answer the confirmation in the agent, so it now waits for \
                approval in PR Nexus."
                .to_owned()
        }
        (Status::Pending, _) => "Waiting for the user to approve it in PR Nexus.".to_owned(),
        (Status::Sent, _) if proposal.auto_sent => {
            "Sent at once, without approval, per the user's settings; `result` is Slack's reply."
                .to_owned()
        }
        (Status::Sent, Some(agent)) => format!(
            "The user approved it in {agent} and Slack accepted it; `result` is Slack's reply."
        ),
        (Status::Sent, None) => {
            "The user approved it and Slack accepted it; `result` is Slack's reply.".to_owned()
        }
        (Status::Failed, _) => "Slack refused it; `error` is Slack's answer.".to_owned(),
        (Status::Rejected, Some(agent)) => {
            format!("The user declined it in {agent}; it was not sent. Do not retry it unasked.")
        }
        (Status::Rejected, None) => {
            "The user rejected it in PR Nexus. Do not retry it unasked.".to_owned()
        }
    }
}

fn get_proposal(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let ProposalArgs { proposal_id } = parse(arguments)?;
    let session = session::current().session;
    let mut proposals = lock(&state.slack_proxy.proposals);
    match proposals.get_mut(proposal_id) {
        Some(proposal) if proposal.session == session => Ok(agent_view(proposal)),
        _ => Err(AppError::new(
            "not_found",
            format!("No Slack proposal {proposal_id} in this session"),
        )),
    }
}

/// Newest first, for Settings → Agents.
pub fn list(state: &AppState) -> Vec<Proposal> {
    lock(&state.slack_proxy.proposals)
        .items
        .iter()
        .rev()
        .cloned()
        .collect()
}

/// The user's answer to proposal `id`: approving calls Slack now.
pub fn decide(state: &AppState, id: u64, approve: bool) -> Result<Proposal, AppError> {
    if !approve {
        let mut proposals = lock(&state.slack_proxy.proposals);
        let proposal = proposals
            .get_mut(id)
            .ok_or_else(|| AppError::new("not_found", format!("No Slack proposal {id}")))?;
        if proposal.status == Status::Pending {
            proposal.status = Status::Rejected;
            proposal.decided_at = Some(Utc::now().to_rfc3339());
        }
        return Ok(proposal.clone());
    }
    if !config::load(&state.store)?.allows(Capability::SlackSends) {
        return Err(AppError::new(
            "forbidden",
            "Slack sends is turned off in Settings → Agents (MCP).",
        ));
    }
    let qualifier = {
        let mut proposals = lock(&state.slack_proxy.proposals);
        let proposal = proposals
            .get_mut(id)
            .ok_or_else(|| AppError::new("not_found", format!("No Slack proposal {id}")))?;
        proposal.workspace["qualifier"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    let (all, _) = exposed(state)?;
    let ws = all
        .into_iter()
        .find(|ws| ws.qualifier() == qualifier)
        .ok_or_else(|| {
            AppError::new(
                "forbidden",
                "This workspace is no longer exposed to agents in Settings → Agents.",
            )
        })?;
    execute(state, &ws, id)
}
