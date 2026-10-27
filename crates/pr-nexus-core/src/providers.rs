//! Which providers exist, the setup-wizard steps, and the synchronous provider traits.
//!
//! HTTP clients live in `pr-nexus-providers`. These traits block; the store
//! has no async runtime (§4).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::{
    AccountId, ChatKind, ChecksState, MergeStateStatus, Mergeable, ProviderRef, PullRequest,
    ReviewDecision, TeamRef, VcsKind,
};
use crate::error::ProviderError;
use crate::review::owners::ReviewBundle;

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
    /// Mapping is optional. Slack connect can be skipped too, so the app stays
    /// usable before a workspace has approved a Slack app (§7.13 R7).
    pub const fn is_skippable(self) -> bool {
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
    pub setup: &'static [SetupStep],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatDescriptor {
    pub kind: ChatKind,
    pub label: &'static str,
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

const VCS: &[VcsDescriptor] = &[VcsDescriptor {
    kind: VcsKind::GitHub,
    label: "GitHub",
    setup: GITHUB_SETUP,
}];

const CHAT: &[ChatDescriptor] = &[ChatDescriptor {
    kind: ChatKind::Slack,
    label: "Slack",
    setup: SLACK_SETUP,
    multi_account: true,
}];

impl ProviderRegistry {
    /// v1 registry: only GitHub and Slack. Other `VcsKind` / `ChatKind` values
    /// stay in the domain for later but have no descriptor, so the wizard never
    /// offers them and they contribute no setup steps.
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
    fn v1_offers_only_github_and_slack() {
        let registry = ProviderRegistry::v1();
        let vcs: Vec<_> = registry
            .vcs
            .iter()
            .map(|descriptor| descriptor.kind)
            .collect();
        let chat: Vec<_> = registry
            .chat
            .iter()
            .map(|descriptor| descriptor.kind)
            .collect();
        assert_eq!(vcs, vec![VcsKind::GitHub]);
        assert_eq!(chat, vec![ChatKind::Slack]);
        let steps = registry.setup_steps(VcsKind::GitLab, ChatKind::Discord);
        assert_eq!(
            steps,
            vec![SetupStep::ChooseVcs, SetupStep::ChooseChat, SetupStep::Done]
        );
    }

    #[test]
    fn only_slack_steps_are_skippable() {
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
            assert_eq!(
                step.is_skippable(),
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
    }
}

/// Which GitHub search bucket to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchBucket {
    Mine,
    ReviewRequested,
}

/// One bucket, run as one or more search strings.
///
/// `queries` already carry the `org:`/`user:` qualifiers for the tracked
/// scope (see `sync::search_queries`); the provider runs each one and merges
/// the results. Sync still filters by org afterwards as a safety net.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrQuery {
    pub bucket: SearchBucket,
    pub queries: Vec<String>,
}

/// The fields of a pull request that are slow for GitHub to compute.
///
/// Search leaves them out (they made a page of 20 cross GitHub's 10 s limit);
/// sync reads them afterwards, by node ID, in small batches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrStatus {
    pub remote_id: String,
    pub review_decision: Option<ReviewDecision>,
    pub checks: ChecksState,
    pub mergeable: Mergeable,
    pub merge_state: MergeStateStatus,
    pub unresolved_threads: u32,
    pub total_threads: u32,
    pub last_commit_at: Option<DateTime<Utc>>,
}

impl PrStatus {
    /// Writes these fields onto `pr`. Last activity is the later of the last
    /// update and the last commit.
    pub fn apply_to(&self, pr: &mut PullRequest) {
        pr.review_decision = self.review_decision;
        pr.checks = self.checks;
        pr.mergeable = self.mergeable;
        pr.merge_state = self.merge_state;
        pr.unresolved_threads = self.unresolved_threads;
        pr.total_threads = self.total_threads;
        pr.last_activity_at = match self.last_commit_at {
            Some(committed) if committed > pr.updated_at => committed,
            _ => pr.updated_at,
        };
    }

    /// The same fields as already stored on `pr`.
    pub fn of(pr: &PullRequest) -> Self {
        Self {
            remote_id: pr.remote.id.clone(),
            review_decision: pr.review_decision,
            checks: pr.checks,
            mergeable: pr.mergeable,
            merge_state: pr.merge_state,
            unresolved_threads: pr.unresolved_threads,
            total_threads: pr.total_threads,
            last_commit_at: (pr.last_activity_at > pr.updated_at).then_some(pr.last_activity_at),
        }
    }
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

/// GitHub's own cap on the files a pull request lists.
pub const FILES_CAP: usize = 3000;

/// The CODEOWNERS file on the base branch: the first hit in
/// [`CODEOWNERS_CANDIDATES`](crate::review::codeowners::CODEOWNERS_CANDIDATES).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeownersBlob {
    pub path: String,
    pub oid: String,
    pub text: String,
}

/// The cheap part of ownership, fetched every time a PR is opened (§6).
///
/// It carries the cache key `(head_sha, base_ref, codeowners oid)` and the
/// reviews, which change without the key changing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipProbe {
    pub head_sha: String,
    pub base_ref: String,
    pub codeowners: Option<CodeownersBlob>,
    pub reviews: ReviewBundle,
    /// The viewer has admin permission on the repository. Only then does a
    /// 404 from classic protection mean "no protection".
    pub viewer_is_admin: bool,
}

/// Classic branch protection: `required_pull_request_reviews` on the base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClassicProtection {
    /// 200: protection requires reviews, with or without code owners.
    Present { require_code_owner_reviews: bool },
    /// 404 while the viewer is an admin: no review protection.
    Absent,
    /// 403, a 404 for a non-admin, or another failure. Never read as false.
    Unknown { reason: String },
}

/// The expensive part of ownership, cached by `(head_sha, base_ref, codeowners oid)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipInputs {
    /// Changed paths, at most [`FILES_CAP`].
    pub files: Vec<String>,
    /// The list stopped at the cap, or GitHub reports more changed files.
    pub files_truncated: bool,
    /// `rules/branches/{base}`: does any applying rule require code-owner review.
    /// `None` when the call failed; the error is in `rules_error`.
    pub rules_require_code_owner: Option<bool>,
    pub rules_error: Option<String>,
    pub protection: ClassicProtection,
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
    /// Review decision, checks, mergeability, and thread counts for `ids`.
    /// A pull request GitHub no longer returns is left out.
    fn pull_request_status(&self, ids: &[ProviderRef]) -> Result<Vec<PrStatus>, ProviderError>;
    /// Head, base, CODEOWNERS blob, viewer permission, and effective reviews in one call.
    fn ownership_probe(&self, pr: &PullRequest) -> Result<OwnershipProbe, ProviderError>;
    /// Files, rules, and protection for the head and base in `probe`.
    fn ownership_inputs(
        &self,
        pr: &PullRequest,
        probe: &OwnershipProbe,
    ) -> Result<OwnershipInputs, ProviderError>;
    /// Team members. A team the token cannot read returns `complete: false`.
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
    fn post(&self, message: &OutgoingMessage<'_>) -> Result<PostedMessage, ProviderError>;
    fn account_id(&self) -> AccountId;
    /// A thread's parent and its first `limit` replies (§7.10, `slack_read_thread`).
    fn read_thread(
        &self,
        _channel_id: &str,
        _root_ts: &str,
        _limit: u32,
    ) -> Result<ThreadReplies, ProviderError> {
        Err(ProviderError::Unsupported(
            "reading threads is not supported".into(),
        ))
    }
    /// A user's display name (`slack_read_user_profile`). Implementations cache it.
    fn user_name(&self, _user_id: &str) -> Result<String, ProviderError> {
        Err(ProviderError::Unsupported(
            "user lookup is not supported".into(),
        ))
    }
    /// Message search for the Mentions panel (§7.10). `private` widens it to
    /// private channels and DMs (`slack_search_public_and_private`).
    fn search_messages(
        &self,
        _phrase: &str,
        _private: bool,
        _limit: u32,
    ) -> Result<Vec<SearchHit>, ProviderError> {
        Err(ProviderError::Unsupported("search is not supported".into()))
    }
}

/// One message of a thread read for the context panel (§7.10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub ts: String,
    pub user_id: Option<String>,
    /// Display name, from the result or the profile lookup.
    pub author: Option<String>,
    pub text: String,
    #[serde(default)]
    pub is_bot: bool,
}

/// `messages[0]` is the parent when the thread could be read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadReplies {
    pub messages: Vec<ThreadMessage>,
    /// More replies exist past the limit.
    #[serde(default)]
    pub has_more: bool,
}

/// One message found by the Mentions search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    pub channel_id: String,
    pub channel_name: String,
    pub ts: String,
    pub author: Option<String>,
    pub text: String,
    pub permalink: Option<String>,
    pub is_bot: bool,
}

/// One `slack_send_message` call (§7.7). `thread_ts` makes it a thread reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage<'a> {
    pub channel_id: &'a str,
    pub text: &'a str,
    pub thread_ts: Option<&'a str>,
    /// Also show a thread reply in the channel. Bumps set it (§7.7).
    pub reply_broadcast: bool,
    pub unfurl_app_links: bool,
}

/// What the provider learned about a message it posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedMessage {
    pub channel_id: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub permalink: Option<String>,
    /// Workspace domain taken from the permalink (`acme` for `acme.slack.com`).
    pub workspace: Option<String>,
}
