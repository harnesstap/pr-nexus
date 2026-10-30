//! Slack tools of the local MCP server: one view over every exposed
//! workspace (SPEC §10.1.2).
//!
//! Each workspace keeps its own session with the official Slack MCP
//! server ([`upstream`]). The tools here pick the workspaces a call
//! targets, call them (at most [`FANOUT`] at once), qualify every Slack ID
//! in the replies with its workspace ([`ids`]) and page with one signed
//! cursor that holds each workspace's own ([`cursor`]).
//!
//! Prepared for personal use: the user's own internal Slack apps, on
//! 127.0.0.1, for agents on the same computer.

pub mod cursor;
pub mod elicit;
pub mod fixtures;
pub mod ids;
pub mod sends;
pub mod session;
mod tools;
pub mod upstream;

#[cfg(test)]
mod tests;

pub use tools::slack_tools;
pub use upstream::SlackProxy;

/// Most workspaces called at once by `workspace: "all"`.
pub const FANOUT: usize = 4;
