//! Local MCP server: lets agents on this machine read and change PR Nexus
//! data through the service layer.
//!
//! - [`server`]: Streamable HTTP on `127.0.0.1`, bearer token, Host and
//!   Origin checks against DNS rebinding.
//! - [`tools`]: the tool registry. Each tool names the [`Capability`] it
//!   needs; the Settings toggles gate capabilities, not tools.
//! - [`writes`]: the Local writes tools (projects, links, notes, stars).
//! - [`audit`]: every call goes through one [`audit::AuditSink`]; the
//!   default one also keeps the `mcp_audit` table shown under Activity.
//! - [`control`]: start, stop and status for the Settings → Agents tab.
//!
//! Off by default.

pub mod audit;
pub mod config;
pub mod control;
pub mod server;
pub mod token;
pub mod tools;
mod writes;

#[cfg(test)]
mod tests;

/// Emitted to the window after an agent changed local data.
pub const DATA_CHANGED_EVENT: &str = "pr-nexus://data-changed";

pub use config::{Capability, McpSettings};
