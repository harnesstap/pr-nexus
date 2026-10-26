//! Which providers exist, and the setup-wizard steps for a chosen pair.
//!
//! The async `VcsProvider` / `ChatProvider` traits land with the first real
//! client. Their input types (ownership inputs, search queries) are specified
//! across later milestones, so this module keeps the pure registry only.

use crate::domain::{ChatKind, VcsKind};

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
    /// Mapping is the only step the user can skip.
    pub const fn is_skippable(self) -> bool {
        matches!(self, Self::ChatMapping)
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
