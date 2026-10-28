//! One hook every MCP tool call goes through, allowed or not.
//!
//! Phase 3 only logs the tool name, outcome and argument names (no values:
//! notes and queries can be private). Phase 4 adds a sink that keeps a local audit
//! log with arguments, shown in Settings.

use serde_json::{Map, Value};

use super::Capability;

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
            fields = ?event.arguments.keys().collect::<Vec<_>>(),
            duration_ms = event.duration_ms,
            client = event.client.as_deref().unwrap_or("-"),
            "mcp tool call"
        );
    }
}
