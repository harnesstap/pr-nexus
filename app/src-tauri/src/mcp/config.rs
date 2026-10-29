//! What the Settings → Agents (MCP) tab saves.

use pr_nexus_core::settings;
use pr_nexus_core::store::Store;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Settings key holding [`McpSettings`] as JSON.
pub const SETTINGS_KEY: &str = "mcp.server";

/// What a tool needs to run. Each has its own toggle in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Read projects and pull requests.
    Reads,
    /// Change local data: projects, links, notes, stars.
    LocalWrites,
    /// Post to Slack, always behind a preview (phase 5).
    SlackSends,
    /// Search and read Slack through each exposed workspace's official
    /// MCP session (public channels and the user's own channels).
    SlackReads,
    /// Search private channels and DMs too (not built yet).
    SlackPrivateSearch,
}

impl Capability {
    /// Whether this build has any tool for it. The UI shows the others as
    /// "coming soon" and the server refuses them whatever is saved.
    pub fn available(self) -> bool {
        matches!(
            self,
            Capability::Reads | Capability::LocalWrites | Capability::SlackReads
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    /// Master switch: the server runs only while this is on.
    pub enabled: bool,
    pub reads: bool,
    pub local_writes: bool,
    pub slack_sends: bool,
    pub slack_reads: bool,
    pub slack_private_search: bool,
    /// Team IDs (`T…`) of the Slack workspaces agents may see. Empty by
    /// default: turning Slack reads on exposes nothing until one is ticked.
    pub slack_workspaces: Vec<String>,
    /// Fixed port on 127.0.0.1; `None` picks a free one at each start.
    pub port: Option<u16>,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            reads: true,
            local_writes: false,
            slack_sends: false,
            slack_reads: false,
            slack_private_search: false,
            slack_workspaces: Vec::new(),
            port: None,
        }
    }
}

impl McpSettings {
    /// Whether tools needing `capability` are listed and callable.
    pub fn allows(&self, capability: Capability) -> bool {
        capability.available()
            && match capability {
                Capability::Reads => self.reads,
                Capability::LocalWrites => self.local_writes,
                Capability::SlackSends => self.slack_sends,
                Capability::SlackReads => self.slack_reads,
                Capability::SlackPrivateSearch => self.slack_private_search,
            }
    }

    /// Clears what this build cannot do, so a saved `true` never outlives
    /// the toggle being disabled.
    pub fn normalized(mut self) -> Self {
        self.local_writes &= Capability::LocalWrites.available();
        self.slack_sends &= Capability::SlackSends.available();
        self.slack_private_search &= Capability::SlackPrivateSearch.available();
        self.slack_workspaces.retain(|team| !team.trim().is_empty());
        self.slack_workspaces.sort();
        self.slack_workspaces.dedup();
        if self.port == Some(0) {
            self.port = None;
        }
        self
    }
}

pub fn load(store: &Store) -> Result<McpSettings, AppError> {
    let Some(entry) = store.get_setting(SETTINGS_KEY.to_owned())? else {
        return Ok(McpSettings::default());
    };
    let saved: McpSettings = serde_json::from_str(&entry.value_json)
        .map_err(|err| AppError::new("internal", format!("decode {SETTINGS_KEY}: {err}")))?;
    Ok(saved.normalized())
}

pub fn save(store: &Store, value: &McpSettings) -> Result<McpSettings, AppError> {
    let value = value.clone().normalized();
    if value.port.is_some_and(|port| port < 1024) {
        return Err(AppError::new(
            "validation",
            "Pick a port from 1024 to 65535, or leave it empty for any free port.",
        ));
    }
    settings::put_json(store, SETTINGS_KEY, &value)?;
    Ok(value)
}
