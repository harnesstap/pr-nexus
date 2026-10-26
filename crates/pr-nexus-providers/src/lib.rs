//! Provider factories.
//!
//! GitHub (octocrab + GraphQL) arrives with sync. Slack stays an empty module
//! until M10. The transport is the official Slack MCP server; see `slack`.

use pr_nexus_core::domain::{ChatKind, VcsKind};
use pr_nexus_core::error::ProviderError;

pub mod github;
pub mod slack;

/// Build the VCS client for `kind`.
///
/// Every kind returns [`ProviderError::Unsupported`] until the GitHub client
/// exists. Coming-soon kinds stay unsupported after that.
pub fn build_vcs(kind: VcsKind) -> Result<(), ProviderError> {
    match kind {
        VcsKind::GitHub => Err(ProviderError::Unsupported(
            "GitHub provider is not implemented yet".to_owned(),
        )),
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
    fn factories_are_unsupported_stubs() {
        assert!(build_vcs(VcsKind::GitHub).is_err());
        assert!(build_vcs(VcsKind::GitLab).is_err());
        assert!(build_chat(ChatKind::Slack).is_err());
        assert!(build_chat(ChatKind::Discord).is_err());
    }
}
