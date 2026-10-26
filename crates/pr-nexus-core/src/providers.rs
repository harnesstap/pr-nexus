//! Which providers exist, the setup-wizard steps, and the synchronous provider traits.
//!
//! HTTP clients live in `pr-nexus-providers`. These traits block; the store
//! has no async runtime (§4).

use chrono::{DateTime, Utc};

use crate::domain::{AccountId, ChatKind, ProviderRef, PullRequest, TeamRef, VcsKind};
use crate::error::ProviderError;

/// Whether the wizard can connect a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Available,
    ComingSoon,
}

/// One screen of the setup wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStep {
    ChooseVcs,
    ChooseChat,
    ConnectVcs,
    ChooseVcsScope,
    ConnectChat,
    ChatMapping,
    Done,
}

impl SetupStep {
    /// Mapping is the only step the user can skip once Slack sign-in exists.
    pub const fn is_skippable(self) -> bool {
        matches!(self, Self::ChatMapping)
    }

    /// Until M10, the Slack connect screen can be skipped. The screen still exists.
    pub const fn is_skippable_before_slack_oauth(self) -> bool {
        matches!(self, Self::ChatMapping | Self::ConnectChat)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChooseVcs => "choose_vcs",
            Self::ChooseChat => "choose_chat",
            Self::ConnectVcs => "connect_vcs",
            Self::ChooseVcsScope => "choose_vcs_scope",
            Self::ConnectChat => "connect_chat",
            Self::ChatMapping => "chat_mapping",
            Self::Done => "done",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "choose_vcs" => Self::ChooseVcs,
            "choose_chat" => Self::ChooseChat,
            "connect_vcs" => Self::ConnectVcs,
            "choose_vcs_scope" => Self::ChooseVcsScope,
            "connect_chat" => Self::ConnectChat,
            "chat_mapping" => Self::ChatMapping,
            "done" => Self::Done,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcsDescriptor {
    pub kind: VcsKind,
    pub label: &'static str,
    pub availability: Availability,
    pub setup: &'static [SetupStep],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatDescriptor {
    pub kind: ChatKind,
    pub label: &'static str,
    pub availability: Availability,
    pub setup: &'static [SetupStep],
    pub multi_account: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderRegistry {
    pub vcs: &'static [VcsDescriptor],
    pub chat: &'static [ChatDescriptor],
}

const GITHUB_SETUP: &[SetupStep] = &[SetupStep::ConnectVcs, SetupStep::ChooseVcsScope];
const SLACK_SETUP: &[SetupStep] = &[SetupStep::ConnectChat, SetupStep::ChatMapping];

const VCS: &[VcsDescriptor] = &[
    VcsDescriptor {
        kind: VcsKind::GitHub,
        label: "GitHub",
        availability: Availability::Available,
        setup: GITHUB_SETUP,
    },
    VcsDescriptor {
        kind: VcsKind::GitLab,
        label: "GitLab",
        availability: Availability::ComingSoon,
        setup: &[],
    },
    VcsDescriptor {
        kind: VcsKind::Gitea,
        label: "Gitea",
        availability: Availability::ComingSoon,
        setup: &[],
    },
];

const CHAT: &[ChatDescriptor] = &[
    ChatDescriptor {
        kind: ChatKind::Slack,
        label: "Slack",
        availability: Availability::Available,
        setup: SLACK_SETUP,
        multi_account: true,
    },
    ChatDescriptor {
        kind: ChatKind::Discord,
        label: "Discord",
        availability: Availability::ComingSoon,
        setup: &[],
        multi_account: false,
    },
    ChatDescriptor {
        kind: ChatKind::Mattermost,
        label: "Mattermost",
        availability: Availability::ComingSoon,
        setup: &[],
        multi_account: false,
    },
];

impl ProviderRegistry {
    /// v1 registry: GitHub and Slack are available. The other kinds are shown
    /// as coming soon and contribute no setup steps.
    pub const fn v1() -> Self {
        Self {
            vcs: VCS,
            chat: CHAT,
        }
    }

    /// `[ChooseVcs, ChooseChat]` + the chosen descriptors' steps + `[Done]`.
    pub fn setup_steps(&self, vcs: VcsKind, chat: ChatKind) -> Vec<SetupStep> {
        let mut steps = vec![SetupStep::ChooseVcs, SetupStep::ChooseChat];
        if let Some(descriptor) = self.vcs.iter().find(|descriptor| descriptor.kind == vcs) {
            steps.extend(descriptor.setup.iter().copied());
        }
        if let Some(descriptor) = self.chat.iter().find(|descriptor| descriptor.kind == chat) {
            steps.extend(descriptor.setup.iter().copied());
        }
        steps.push(SetupStep::Done);
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_and_slack_setup_order() {
        let steps = ProviderRegistry::v1().setup_steps(VcsKind::GitHub, ChatKind::Slack);
        assert_eq!(
            steps,
            vec![
                SetupStep::ChooseVcs,
                SetupStep::ChooseChat,
                SetupStep::ConnectVcs,
                SetupStep::ChooseVcsScope,
                SetupStep::ConnectChat,
                SetupStep::ChatMapping,
                SetupStep::Done,
            ]
        );
    }

    #[test]
    fn coming_soon_providers_add_no_steps() {
        let registry = ProviderRegistry::v1();
        let steps = registry.setup_steps(VcsKind::GitLab, ChatKind::Discord);
        assert_eq!(
            steps,
            vec![SetupStep::ChooseVcs, SetupStep::ChooseChat, SetupStep::Done]
        );
        for descriptor in registry.vcs {
            if descriptor.availability == Availability::ComingSoon {
                assert!(descriptor.setup.is_empty());
            }
        }
        for descriptor in registry.chat {
            if descriptor.availability == Availability::ComingSoon {
                assert!(descriptor.setup.is_empty());
            }
        }
    }

    #[test]
    fn only_chat_mapping_is_skippable() {
        let skippable = [
            SetupStep::ChooseVcs,
            SetupStep::ChooseChat,
            SetupStep::ConnectVcs,
            SetupStep::ChooseVcsScope,
            SetupStep::ConnectChat,
            SetupStep::ChatMapping,
            SetupStep::Done,
        ];
        for step in skippable {
            assert_eq!(step.is_skippable(), step == SetupStep::ChatMapping);
            assert_eq!(
                step.is_skippable_before_slack_oauth(),
                matches!(step, SetupStep::ChatMapping | SetupStep::ConnectChat)
            );
        }
    }

    #[test]
    fn slack_allows_multiple_accounts() {
        let slack = ProviderRegistry::v1()
            .chat
            .iter()
            .find(|descriptor| descriptor.kind == ChatKind::Slack)
            .expect("slack descriptor");
        assert!(slack.multi_account);
        assert_eq!(slack.availability, Availability::Available);
    }
}

/// Which GitHub search bucket to run. Org filtering happens after the fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchBucket {
    Mine,
    ReviewRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrQuery {
    pub bucket: SearchBucket,
}

/// A pull request plus the repository fields the store upserts with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedPullRequest {
    pub pull_request: PullRequest,
    pub repo_node_id: Option<String>,
    pub default_branch: Option<String>,
    pub repo_archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcsUser {
    pub id: String,
    pub login: String,
    /// Every org the user belongs to, including ones the token cannot read yet.
    pub organizations: Vec<String>,
    /// Orgs listed by membership but hidden from the token, usually because
    /// the org enforces SAML SSO and the token is not authorized for it.
    pub organizations_needing_sso: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateBudget {
    pub limit: u32,
    pub remaining: u32,
    pub reset_at: DateTime<Utc>,
    pub cost: u32,
    /// Set when GitHub returns a secondary limit. Scheduled polls wait until then.
    pub secondary_until: Option<DateTime<Utc>>,
}

/// Placeholder until M9 fills files, CODEOWNERS, and rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipInputs {
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMembers {
    pub complete: bool,
    pub logins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatUser {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatChannel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

pub trait VcsProvider: Send + Sync {
    fn kind(&self) -> VcsKind;
    fn viewer(&self) -> Result<VcsUser, ProviderError>;
    fn search_pull_requests(
        &self,
        query: &PrQuery,
    ) -> Result<Vec<SyncedPullRequest>, ProviderError>;
    fn refresh_pull_requests(
        &self,
        ids: &[ProviderRef],
    ) -> Result<Vec<SyncedPullRequest>, ProviderError>;
    fn ownership_inputs(&self, pr: &PullRequest) -> Result<OwnershipInputs, ProviderError>;
    fn team_members(&self, team: &TeamRef) -> Result<TeamMembers, ProviderError>;
    fn rate_limit(&self) -> Result<RateBudget, ProviderError>;
    fn request_team_review(&self, pr: &PullRequest, teams: &[TeamRef])
    -> Result<(), ProviderError>;
}

pub trait ChatProvider: Send + Sync {
    fn kind(&self) -> ChatKind;
    fn whoami(&self) -> Result<ChatUser, ProviderError>;
    fn list_channels(&self, cursor: Option<String>) -> Result<Page<ChatChannel>, ProviderError>;
    fn channel_info(&self, channel_id: &str) -> Result<ChatChannel, ProviderError>;
    fn post(&self, channel_id: &str, text: &str) -> Result<(), ProviderError>;
    fn account_id(&self) -> AccountId;
}
