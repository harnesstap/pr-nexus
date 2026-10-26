//! Slack through the official remote MCP server (§7.11).
//!
//! - [`oauth`]: the sign-in flow (discovery, PKCE, pre-registered client,
//!   optional secret, loopback redirect) and the keychain credential store.
//! - [`mcp`]: the MCP session per workspace and [`SlackMcpChatProvider`].
//! - [`schema`]: typed tool-result parsers and the tool schema pin.
//! - [`loopback`] and [`http`]: the redirect listener and Slack's OAuth error
//!   shape.
//!
//! The only direct Web API call is `auth.test` right after sign-in, to learn
//! the workspace name and domain (§7.11 step 4). Nothing else bypasses MCP.

pub mod http;
pub mod loopback;
pub mod mcp;
pub mod oauth;
pub mod schema;

pub use loopback::CancelHandle;
pub use mcp::{SessionConfig, SlackMcpChatProvider};
pub use oauth::{ConnectConfig, Connected, connect};

/// `slack.mcp.url` default.
pub const SLACK_MCP_URL: &str = "https://mcp.slack.com/mcp";
/// Base of the one Web API call (`auth.test`).
pub const SLACK_WEB_API: &str = "https://slack.com/api";

/// User scopes requested at sign-in (§7.11).
pub const USER_SCOPES: &[&str] = &[
    "chat:write",
    "search:read.public",
    "search:read.private",
    "channels:read",
    "groups:read",
    "channels:history",
    "groups:history",
    "users:read",
];

/// Keychain key of a workspace grant (§9).
pub fn token_key(team_id: &str, user_id: &str) -> String {
    format!("slack:{team_id}:{user_id}")
}

/// Keychain key of a Slack app's client secret (§9).
pub fn client_secret_key(client_id: &str) -> String {
    format!("slack-client:{client_id}")
}

/// rmcp and reqwest 0.13 are built without a default TLS provider, so the
/// process installs `ring` once. Repeated calls are harmless.
pub fn install_tls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Errors of the sign-in flow. Messages are shown to the user as is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SlackAuthError {
    #[error("Enter your Slack app's client ID first.")]
    MissingClientId,
    #[error(
        "The redirect URI is not valid ({0}). Use http://127.0.0.1:<port>/<path>, exactly as registered on the Slack app."
    )]
    InvalidRedirect(String),
    #[error(
        "Port {0} is in use. Finish or close the other app's Slack sign-in, or change the redirect URI."
    )]
    PortInUse(u16),
    #[error("Could not listen for the Slack redirect: {0}")]
    Listener(String),
    #[error("Could not read Slack's sign-in settings: {0}")]
    Discovery(String),
    #[error("Could not open the browser: {0}")]
    Browser(String),
    #[error("Slack sign-in timed out after 10 minutes. Try again.")]
    Timeout,
    #[error("Slack sign-in was cancelled.")]
    Cancelled,
    #[error("Slack sign-in was not approved ({0}).")]
    Denied(String),
    #[error("Slack rejected the sign-in: {0}")]
    Exchange(String),
    #[error("Signed in, but could not identify the workspace: {0}")]
    Identity(String),
    #[error("Could not save the Slack token in the keychain: {0}")]
    Store(String),
}
