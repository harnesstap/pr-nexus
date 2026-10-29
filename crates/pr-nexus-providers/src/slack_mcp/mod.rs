//! Slack through the official remote MCP server (§7.11).
//!
//! - [`oauth`]: the sign-in flow (discovery, PKCE, pre-registered client,
//!   optional secret, loopback redirect) and the keychain credential store.
//! - [`mcp`]: the MCP session per workspace and [`SlackMcpChatProvider`].
//! - [`schema`]: typed tool-result parsers and the tool schema pin.
//! - [`loopback`] and [`http`]: the redirect listener and Slack's OAuth error
//!   shape.
//!
//! The only direct Web API call is `auth.test`: right after sign-in, to learn
//! the workspace name and domain (§7.11 step 4), and at startup, to check a
//! saved grant. Nothing else bypasses MCP.

pub mod http;
pub mod loopback;
pub mod mcp;
pub mod oauth;
pub mod schema;

pub use loopback::CancelHandle;
pub use mcp::{RmcpSession, SessionConfig, SlackMcpChatProvider, ToolCaller};
pub use oauth::{ConnectConfig, Connected, TokenState, connect, enterprise_id, token_state};

/// `slack.mcp.url` default.
pub const SLACK_MCP_URL: &str = "https://mcp.slack.com/mcp";
/// Base of the one Web API call (`auth.test`).
pub const SLACK_WEB_API: &str = "https://slack.com/api";

/// User scopes requested at sign-in (§7.11).
pub const USER_SCOPES: &[&str] = &[
    "chat:write",
    "search:read.public",
    "search:read.private",
    "search:read.users",
    "channels:read",
    "groups:read",
    "channels:history",
    "groups:history",
    "users:read",
    // Agent reads (§10.1.2): DMs and group DMs, reactions, emoji, files,
    // canvases and lists.
    "search:read.mpim",
    "search:read.im",
    "search:read.files",
    "mpim:read",
    "im:read",
    "mpim:history",
    "im:history",
    "reactions:read",
    "emoji:read",
    "files:read",
    "canvases:read",
    "lists:read",
];

/// The scopes a saved grant has, from the stored token JSON. `None` when the
/// grant does not say (older grants), so callers never nag on a guess.
pub fn granted_scopes(stored: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(stored).ok()?;
    let mut scopes: Vec<String> = value
        .get("granted_scopes")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if scopes.is_empty()
        && let Some(text) = value
            .pointer("/token_response/scope")
            .and_then(|v| v.as_str())
    {
        scopes = text
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
    }
    (!scopes.is_empty()).then_some(scopes)
}

/// The [`USER_SCOPES`] a grant lacks, in request order.
pub fn missing_scopes(granted: &[String]) -> Vec<String> {
    USER_SCOPES
        .iter()
        .filter(|s| !granted.iter().any(|g| g == *s))
        .map(|s| (*s).to_owned())
        .collect()
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn granted_scopes_read_both_shapes() {
        let listed = r#"{"client_id":"x","granted_scopes":["chat:write","users:read"]}"#;
        assert_eq!(
            granted_scopes(listed).unwrap(),
            vec!["chat:write", "users:read"]
        );
        let token = r#"{"client_id":"x","token_response":{"scope":"chat:write,emoji:read"}}"#;
        assert_eq!(
            granted_scopes(token).unwrap(),
            vec!["chat:write", "emoji:read"]
        );
        assert_eq!(granted_scopes(r#"{"client_id":"x"}"#), None);
        assert_eq!(granted_scopes("not json"), None);
    }

    #[test]
    fn missing_scopes_keep_request_order() {
        let all: Vec<String> = USER_SCOPES.iter().map(|s| (*s).to_owned()).collect();
        assert!(missing_scopes(&all).is_empty());
        let old: Vec<String> = USER_SCOPES[..9].iter().map(|s| (*s).to_owned()).collect();
        let missing = missing_scopes(&old);
        assert_eq!(
            missing.first().map(String::as_str),
            Some("search:read.mpim")
        );
        assert!(missing.contains(&"lists:read".to_owned()));
    }
}

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
