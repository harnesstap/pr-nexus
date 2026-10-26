//! Provider factories.
//!
//! GitHub is a blocking GraphQL client. Slack stays an empty module until M10.
//! The transport is the official Slack MCP server; see `slack`.

use pr_nexus_core::domain::{AccountId, ChatKind, VcsKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::VcsProvider;

pub mod github;
pub mod slack;
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

/// Build the chat client for `kind`.
///
/// Slack returns unsupported on purpose. The MCP client lands in M10.
pub fn build_chat(kind: ChatKind) -> Result<(), ProviderError> {
    match kind {
        ChatKind::Slack => Err(ProviderError::Unsupported(
            "Slack provider is not implemented yet".to_owned(),
        )),
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
        assert!(build_chat(ChatKind::Slack).is_err());
        assert!(build_chat(ChatKind::Discord).is_err());
    }
}
