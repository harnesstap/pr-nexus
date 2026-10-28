//! Local MCP server: lets agents on this machine read (and, in later
//! phases, change) PR Nexus data through the service layer.
//!
//! - [`server`]: Streamable HTTP on `127.0.0.1`, bearer token, Host and
//!   Origin checks against DNS rebinding.
//! - [`tools`]: the tool registry. Each tool names the [`Capability`] it
//!   needs; the Settings toggles gate capabilities, not tools.
//! - [`audit`]: every call goes through one [`audit::AuditSink`].
//! - [`control`]: start, stop and status for the Settings → Agents tab.
//!
//! Off by default.

pub mod audit;
pub mod config;
pub mod control;
pub mod server;
pub mod token;
pub mod tools;

#[cfg(test)]
mod tests;

pub use config::{Capability, McpSettings};
