//! Provider kinds shared by the registry and the provider factories.
//!
//! The pull-request, project, and review-request model from the spec lands with
//! the store milestone. Only the identifiers those later types will key on are
//! declared here.

/// Version control providers. Only GitHub is available in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VcsKind {
    GitHub,
    GitLab,
    Gitea,
}

/// Chat providers. Only Slack is available in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatKind {
    Slack,
    Discord,
    Mattermost,
}

/// Row id in `accounts`. The store milestone is the first writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountId(pub i64);

impl std::fmt::Display for VcsKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::Gitea => "Gitea",
        })
    }
}

impl std::fmt::Display for ChatKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Slack => "Slack",
            Self::Discord => "Discord",
            Self::Mattermost => "Mattermost",
        })
    }
}
