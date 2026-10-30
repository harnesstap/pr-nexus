//! Approving a held Slack write in the calling agent, through MCP
//! elicitation (SPEC §10.1.4).
//!
//! When a write would wait for approval and the client declared form
//! elicitation, PR Nexus asks the user right there, in the agent: accept
//! sends it, decline or cancel drops it. With no answer (timeout, error)
//! the write keeps waiting in Settings → Agents, as without elicitation.

use std::sync::Arc;
use std::time::Duration;

use rmcp::RoleServer;
use rmcp::model::{ElicitRequestParams, ElicitationAction, ElicitationSchema};
use rmcp::service::{ElicitationMode, Peer};
use serde_json::Value;

use super::sends::Held;
use super::session::{Answer, Ask};

/// How long the agent's confirmation stays open. Under five minutes, the
/// idle timeout some clients put on an HTTP tool call, so our fallback
/// answer still reaches the agent.
pub const TIMEOUT: Duration = Duration::from_secs(270);

/// Whether to ask in the agent: the write is held, the user allows it,
/// and the client can show a confirmation. Automatic mode holds nothing,
/// so it never asks.
pub fn should_ask(held: Option<&Held>, enabled: bool, can_ask: bool) -> bool {
    held.is_some() && enabled && can_ask
}

/// What happens to a held write after the agent's confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Send it now.
    Send,
    /// The user said no (declined or dismissed): drop it.
    Drop,
    /// No answer: it waits in PR Nexus. Says why.
    Wait(String),
}

pub fn outcome(answer: Answer) -> Outcome {
    match answer {
        Answer::Accept => Outcome::Send,
        Answer::Decline | Answer::Cancel => Outcome::Drop,
        Answer::Unanswered(why) => Outcome::Wait(why),
    }
}

/// What the confirmation says: action, workspace, channel and the exact
/// content.
pub fn message(tool: &str, workspace: &Value, channel_id: &str, arguments: &Value) -> String {
    let action = match tool {
        "slack_send_message" => "Send this Slack message",
        "slack_schedule_message" => "Schedule this Slack message",
        "slack_send_message_draft" => "Save this Slack draft",
        "slack_add_reaction" => "Add this Slack reaction",
        other => other,
    };
    let text = |key: &str| arguments.get(key).and_then(Value::as_str);
    let mut out = format!(
        "{action} as you?\n\nWorkspace: {} ({})\nChannel: {channel_id}",
        workspace["name"].as_str().unwrap_or_default(),
        workspace["qualifier"].as_str().unwrap_or_default(),
    );
    if let Some(thread) = text("thread_ts") {
        out.push_str(&format!("\nIn thread: {thread}"));
    }
    if let Some(at) = arguments.get("post_at").and_then(Value::as_i64) {
        let when = chrono::DateTime::from_timestamp(at, 0)
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| at.to_string());
        out.push_str(&format!("\nAt: {when}"));
    }
    if let (Some(emoji), Some(ts)) = (text("emoji"), text("message_ts")) {
        out.push_str(&format!("\nReaction: :{emoji}: on message {ts}"));
    }
    if let Some(body) = text("message") {
        out.push_str(&format!("\n\nMessage:\n{body}"));
    }
    out
}

/// Asks through the session's MCP peer.
pub struct PeerAsk {
    peer: Peer<RoleServer>,
    runtime: tokio::runtime::Handle,
}

impl PeerAsk {
    /// `Some` when the client declared form elicitation.
    pub fn for_peer(peer: &Peer<RoleServer>) -> Option<Arc<dyn Ask>> {
        if !peer
            .supported_elicitation_modes()
            .contains(&ElicitationMode::Form)
        {
            return None;
        }
        Some(Arc::new(Self {
            peer: peer.clone(),
            runtime: tokio::runtime::Handle::current(),
        }))
    }
}

impl Ask for PeerAsk {
    fn ask(&self, message: &str) -> Answer {
        // An empty schema: a yes/no confirmation, nothing to fill in.
        let request = ElicitRequestParams::FormElicitationParams {
            meta: None,
            message: message.to_owned(),
            requested_schema: ElicitationSchema::new(Default::default()),
        };
        let reply = self.runtime.block_on(
            self.peer
                .create_elicitation_with_timeout(request, Some(TIMEOUT)),
        );
        match reply {
            Ok(result) => match result.action {
                ElicitationAction::Accept => Answer::Accept,
                ElicitationAction::Decline => Answer::Decline,
                ElicitationAction::Cancel => Answer::Cancel,
                // A newer action this build does not know: no answer.
                other => Answer::Unanswered(format!("unknown answer {other:?}")),
            },
            Err(err) => Answer::Unanswered(err.to_string()),
        }
    }
}
