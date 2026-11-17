//! What the Settings → Agents (MCP) tab saves.

use pr_nexus_core::settings;
use pr_nexus_core::store::Store;
use serde::{Deserialize, Serialize};

use super::hosts;
use crate::error::AppError;
use crate::profiles::StoreSlot;
use pr_nexus_core::profiles::SEND_POLICY_KEY;

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
    /// Send, schedule or draft Slack messages and add reactions. Whether
    /// each one waits for the user's approval follows [`SlackSendMode`].
    SlackSends,
    /// Search and read Slack through each exposed workspace's official
    /// MCP session (public channels and the user's own channels).
    SlackReads,
    /// Search private channels and DMs too (`slack_search_public_and_private`).
    SlackPrivateSearch,
}

impl Capability {
    /// Whether this build has any tool for it. The UI shows the others as
    /// "coming soon" and the server refuses them whatever is saved.
    pub fn available(self) -> bool {
        matches!(
            self,
            Capability::Reads
                | Capability::LocalWrites
                | Capability::SlackSends
                | Capability::SlackReads
                | Capability::SlackPrivateSearch
        )
    }
}

/// Which Slack writes from agents wait for the user's approval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlackSendMode {
    /// Every write waits for approval; the allowlist is ignored.
    ApproveAll,
    /// Every write goes out at once, with no proposal to approve.
    Automatic,
    /// Writes to `slack_auto_send_channels` go out at once unless the
    /// session read another workspace; the rest wait. The default, so
    /// settings saved before the modes existed keep their behavior.
    #[default]
    Allowlist,
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
    /// Qualified channel IDs (`T…/C…`, `E…/C…`) where Slack sends go out
    /// without asking, unless the session read another workspace. Empty
    /// by default: every send waits for approval.
    pub slack_auto_send_channels: Vec<String>,
    /// How Slack writes are approved. Missing in older saves, which read
    /// as [`SlackSendMode::Allowlist`].
    pub slack_send_mode: SlackSendMode,
    /// Ask in the agent (MCP elicitation) before holding a write for
    /// approval in PR Nexus, when the client supports it. On by default,
    /// older saves included.
    pub slack_elicit: bool,
    /// Fixed port on 127.0.0.1; `None` picks a free one at each start.
    pub port: Option<u16>,
    /// Hostnames (or an IPv4 address), optionally `host:port`, whose Host
    /// and Origin headers are allowed besides `127.0.0.1`, `localhost`,
    /// and `[::1]`. Empty by default. `*` is not accepted.
    pub extra_allowed_hosts: Vec<String>,
    /// Tool names the user switched off. Empty by default, including
    /// settings saved before per-tool switches, so a tool that defaults on
    /// stays on. Turning a capability off does not change this list.
    pub disabled_tools: Vec<String>,
    /// Slack send tools that default off (anything outside
    /// [`SLACK_SEND_BASELINE`]) which the user has switched on. Empty by
    /// default. Other capabilities ignore it: a new tool there defaults on
    /// and is turned off only through [`McpSettings::disabled_tools`].
    pub enabled_slack_send_tools: Vec<String>,
}

/// Slack send tools that existed when per-tool switches shipped. A send
/// tool that is not listed here defaults off, because a send acts as the
/// user. Do not add a newly introduced send tool: leave it out so existing
/// settings keep it off until the user turns it on.
pub const SLACK_SEND_BASELINE: &[&str] = &[
    "request_reviews",
    "slack_add_reaction",
    "slack_get_proposal",
    "slack_schedule_message",
    "slack_send_message",
    "slack_send_message_draft",
];

/// Whether `name` is on until the user switches it off. Slack send tools
/// outside [`SLACK_SEND_BASELINE`] are off until named in
/// [`McpSettings::enabled_slack_send_tools`].
pub fn tool_defaults_on(capability: Capability, name: &str) -> bool {
    !matches!(capability, Capability::SlackSends)
        || SLACK_SEND_BASELINE.binary_search(&name).is_ok()
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
            slack_auto_send_channels: Vec::new(),
            slack_send_mode: SlackSendMode::default(),
            slack_elicit: true,
            port: None,
            extra_allowed_hosts: Vec::new(),
            disabled_tools: Vec::new(),
            enabled_slack_send_tools: Vec::new(),
        }
    }
}

impl McpSettings {
    /// Whether tools needing `capability` are listed and callable, before
    /// the per-tool switch.
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

    /// The per-tool switch, ignoring the capability master. A capability
    /// that is off leaves this as the user set it.
    pub fn tool_selected(&self, capability: Capability, name: &str) -> bool {
        if self.disabled_tools.iter().any(|tool| tool == name) {
            return false;
        }
        tool_defaults_on(capability, name)
            || self
                .enabled_slack_send_tools
                .iter()
                .any(|tool| tool == name)
    }

    /// Listed and callable: the capability is on and the tool is selected.
    pub fn allows_tool(&self, capability: Capability, name: &str) -> bool {
        self.allows(capability) && self.tool_selected(capability, name)
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
        for channel in &mut self.slack_auto_send_channels {
            *channel = channel.trim().to_owned();
        }
        self.slack_auto_send_channels.retain(|channel| {
            super::slack::ids::parse(channel).is_some_and(|id| id.qualifier.is_some())
        });
        self.slack_auto_send_channels.sort();
        self.slack_auto_send_channels.dedup();
        if self.port == Some(0) {
            self.port = None;
        }
        self.extra_allowed_hosts = hosts::normalize(&self.extra_allowed_hosts);
        tidy_names(&mut self.disabled_tools);
        tidy_names(&mut self.enabled_slack_send_tools);
        self.enabled_slack_send_tools
            .retain(|name| !self.disabled_tools.contains(name));
        self
    }
}

fn tidy_names(names: &mut Vec<String>) {
    for name in names.iter_mut() {
        *name = name.trim().to_owned();
    }
    names.retain(|name| !name.is_empty());
    names.sort();
    names.dedup();
}

/// The Slack part of [`McpSettings`], kept per profile (§8.4) under
/// [`pr_nexus_core::profiles::SEND_POLICY_KEY`]: which workspaces agents
/// see and how their writes are approved. The rest is app-wide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SendPolicy {
    pub slack_workspaces: Vec<String>,
    pub slack_auto_send_channels: Vec<String>,
    pub slack_send_mode: SlackSendMode,
    pub slack_elicit: bool,
}

impl Default for SendPolicy {
    fn default() -> Self {
        Self::of(&McpSettings::default())
    }
}

impl SendPolicy {
    fn of(settings: &McpSettings) -> Self {
        Self {
            slack_workspaces: settings.slack_workspaces.clone(),
            slack_auto_send_channels: settings.slack_auto_send_channels.clone(),
            slack_send_mode: settings.slack_send_mode,
            slack_elicit: settings.slack_elicit,
        }
    }

    fn apply(self, settings: &mut McpSettings) {
        settings.slack_workspaces = self.slack_workspaces;
        settings.slack_auto_send_channels = self.slack_auto_send_channels;
        settings.slack_send_mode = self.slack_send_mode;
        settings.slack_elicit = self.slack_elicit;
    }
}

fn read<T: serde::de::DeserializeOwned>(store: &Store, key: &str) -> Result<Option<T>, AppError> {
    let Some(entry) = store.get_setting(key.to_owned())? else {
        return Ok(None);
    };
    serde_json::from_str(&entry.value_json)
        .map(Some)
        .map_err(|err| AppError::new("internal", format!("decode {key}: {err}")))
}

/// The app-wide settings with the active profile's send policy. The
/// default profile's policy falls back to what was saved before profiles.
pub fn load(store: &StoreSlot) -> Result<McpSettings, AppError> {
    let mut settings: McpSettings = read(store.home(), SETTINGS_KEY)?.unwrap_or_default();
    match read::<SendPolicy>(store.get(), SEND_POLICY_KEY)? {
        Some(policy) => policy.apply(&mut settings),
        None if store.is_home() => {}
        None => SendPolicy::default().apply(&mut settings),
    }
    Ok(settings.normalized())
}

/// Saves the app-wide part for every profile and the send policy for the
/// active one only.
pub fn save(store: &StoreSlot, value: &McpSettings) -> Result<McpSettings, AppError> {
    let value = value.clone().normalized();
    if value.port.is_some_and(|port| port < 1024) {
        return Err(AppError::new(
            "validation",
            "Pick a port from 1024 to 65535, or leave it empty for any free port.",
        ));
    }
    let mut global = value.clone();
    // The default profile's pre-profiles policy stays where it was, so a
    // profile without its own policy never picks up another one's.
    if let Some(home) = read::<McpSettings>(store.home(), SETTINGS_KEY)? {
        SendPolicy::of(&home).apply(&mut global);
    } else {
        SendPolicy::default().apply(&mut global);
    }
    settings::put_json(store.home(), SETTINGS_KEY, &global)?;
    settings::put_json(store.get(), SEND_POLICY_KEY, &SendPolicy::of(&value))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_saves_keep_the_allowlist_mode_and_channels() {
        let saved: McpSettings = serde_json::from_str(
            r#"{"enabled":true,"slack_sends":true,"slack_auto_send_channels":["T0ACME0001/C0123456789"]}"#,
        )
        .unwrap();
        let saved = saved.normalized();
        assert_eq!(saved.slack_send_mode, SlackSendMode::Allowlist);
        assert_eq!(saved.slack_auto_send_channels, ["T0ACME0001/C0123456789"]);
        assert_eq!(
            McpSettings::default().slack_send_mode,
            SlackSendMode::Allowlist
        );
    }

    #[test]
    fn the_mode_round_trips_in_snake_case() {
        let value = McpSettings {
            slack_send_mode: SlackSendMode::ApproveAll,
            ..McpSettings::default()
        };
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(json["slack_send_mode"], "approve_all");
        let back: McpSettings = serde_json::from_value(json).unwrap();
        assert_eq!(back, value);
    }

    #[test]
    fn extra_allowed_hosts_default_empty_and_drop_invalid_entries() {
        let saved: McpSettings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(saved.normalized().extra_allowed_hosts.is_empty());
        let noisy = McpSettings {
            extra_allowed_hosts: vec![
                " Host.Docker.Internal ".into(),
                "*".into(),
                "http://host.docker.internal".into(),
                "host.docker.internal/mcp".into(),
            ],
            ..McpSettings::default()
        }
        .normalized();
        assert_eq!(
            noisy.extra_allowed_hosts,
            vec!["host.docker.internal".to_owned()]
        );
    }

    #[test]
    fn older_saves_leave_tools_on_and_new_slack_sends_default_off() {
        let saved: McpSettings =
            serde_json::from_str(r#"{"enabled":true,"reads":true,"slack_sends":true}"#).unwrap();
        let saved = saved.normalized();
        assert!(saved.disabled_tools.is_empty());
        assert!(saved.enabled_slack_send_tools.is_empty());
        assert!(saved.allows_tool(Capability::Reads, "list_projects"));
        assert!(saved.allows_tool(Capability::SlackSends, "slack_send_message"));
        assert!(!saved.tool_selected(Capability::SlackSends, "slack_future_send"));
        assert!(!saved.allows_tool(Capability::SlackSends, "slack_future_send"));
        assert!(saved.tool_selected(Capability::Reads, "list_projects"));

        let picked = McpSettings {
            enabled: true,
            slack_sends: true,
            enabled_slack_send_tools: vec!["slack_future_send".into()],
            ..McpSettings::default()
        }
        .normalized();
        assert!(picked.tool_selected(Capability::SlackSends, "slack_future_send"));
        assert!(picked.allows_tool(Capability::SlackSends, "slack_future_send"));

        // The capability stays the master. The per-tool choice is unchanged.
        let off = McpSettings {
            slack_sends: false,
            ..picked
        };
        assert!(off.tool_selected(Capability::SlackSends, "slack_send_message"));
        assert!(!off.allows_tool(Capability::SlackSends, "slack_send_message"));
        assert!(off.tool_selected(Capability::SlackSends, "slack_future_send"));
        assert!(!off.allows_tool(Capability::SlackSends, "slack_future_send"));
    }

    #[test]
    fn tool_lists_trim_sort_and_let_disabled_win() {
        let noisy = McpSettings {
            disabled_tools: vec![
                " list_projects ".into(),
                "list_projects".into(),
                String::new(),
            ],
            enabled_slack_send_tools: vec![
                "slack_future_send".into(),
                "list_projects".into(),
                " slack_future_send ".into(),
            ],
            ..McpSettings::default()
        }
        .normalized();
        assert_eq!(noisy.disabled_tools, vec!["list_projects".to_owned()]);
        assert_eq!(
            noisy.enabled_slack_send_tools,
            vec!["slack_future_send".to_owned()]
        );
    }

    #[test]
    fn slack_send_baseline_is_the_tools_that_stay_on() {
        let mut sorted = SLACK_SEND_BASELINE.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, SLACK_SEND_BASELINE);
        assert_eq!(
            SLACK_SEND_BASELINE,
            [
                "request_reviews",
                "slack_add_reaction",
                "slack_get_proposal",
                "slack_schedule_message",
                "slack_send_message",
                "slack_send_message_draft",
            ]
        );
    }
}
