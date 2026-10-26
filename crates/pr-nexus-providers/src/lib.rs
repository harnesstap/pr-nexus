//! Provider factories.
//!
//! GitHub is a blocking GraphQL client. Slack goes through the official
//! Slack MCP server; see [`slack_mcp`].

use pr_nexus_core::domain::{AccountId, ChatKind, VcsKind};
use pr_nexus_core::error::ProviderError;
use std::sync::Arc;

use pr_nexus_core::providers::{ChatProvider, ChatUser, VcsProvider};

pub mod github;
pub mod slack_mcp;
pub mod vault;

pub use github::GithubClient;

/// Build the VCS client for `kind`.
///
/// Coming-soon kinds return [`ProviderError::Unsupported`]. The token is kept
/// in the client and is not logged.
pub fn build_vcs(
    kind: VcsKind,
    account: AccountId,
    token: String,
) -> Result<Box<dyn VcsProvider>, ProviderError> {
    match kind {
        VcsKind::GitHub => Ok(Box::new(GithubClient::new(account, token)?)),
        VcsKind::GitLab => Err(ProviderError::Unsupported(
            "GitLab is not available yet".to_owned(),
        )),
        VcsKind::Gitea => Err(ProviderError::Unsupported(
            "Gitea is not available yet".to_owned(),
        )),
    }
}

/// Build the chat client for one signed-in workspace.
///
/// Slack talks to the official MCP server with the keychain grant. Discord
/// and Mattermost are coming soon.
pub fn build_chat(
    kind: ChatKind,
    account: AccountId,
    user: ChatUser,
    session: slack_mcp::SessionConfig,
    vault: Arc<vault::Vault>,
) -> Result<Box<dyn ChatProvider>, ProviderError> {
    match kind {
        ChatKind::Slack => Ok(Box::new(slack_mcp::SlackMcpChatProvider::connect(
            session, vault, account, user,
        )?)),
        ChatKind::Discord => Err(ProviderError::Unsupported(
            "Discord is not available yet".to_owned(),
        )),
        ChatKind::Mattermost => Err(ProviderError::Unsupported(
            "Mattermost is not available yet".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_factory_builds_and_other_kinds_stay_unsupported() {
        assert!(build_vcs(VcsKind::GitHub, AccountId(1), "token".into()).is_ok());
        assert!(build_vcs(VcsKind::GitLab, AccountId(1), "token".into()).is_err());
        let session = slack_mcp::SessionConfig {
            mcp_url: slack_mcp::SLACK_MCP_URL.into(),
            client_id: "1.2".into(),
            client_secret: None,
            redirect_uri: "http://127.0.0.1:47821/mcp/oauth/callback".into(),
            keychain_key: "slack:T1:U1".into(),
        };
        let vault = Arc::new(vault::Vault::memory("test"));
        let user = ChatUser {
            id: "U1".into(),
            name: "me".into(),
        };
        // Building opens nothing: the MCP session starts on the first call.
        assert!(
            build_chat(
                ChatKind::Slack,
                AccountId(1),
                user.clone(),
                session.clone(),
                vault.clone()
            )
            .is_ok()
        );
        assert!(build_chat(ChatKind::Discord, AccountId(1), user, session, vault).is_err());
    }
}
