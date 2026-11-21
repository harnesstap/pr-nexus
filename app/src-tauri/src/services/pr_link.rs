//! Open a pull request named by a URL in the search field.
//!
//! A stored row is returned as it is. A pull request GitHub can see but this
//! app has not synced is read for the detail panel and is not written into
//! the list, so a later sync does not adopt it.

use pr_nexus_core::error::ProviderError;
use pr_nexus_core::overview::{self, PrOverview};
use pr_nexus_core::pr_link::PrLink;
use pr_nexus_core::providers::VcsProvider;
use pr_nexus_core::settings;
use serde::Serialize;

use crate::commands::{self, ListRow};
use crate::error::AppError;
use crate::state::AppState;

/// What the search field should show for one pull request link.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrLinkHit {
    Stored {
        row: ListRow,
    },
    Opened {
        row: ListRow,
        overview: Box<PrOverview>,
    },
    Missing {
        message: String,
    },
}

pub fn lookup(
    state: &AppState,
    provider: &dyn VcsProvider,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<PrLinkHit, AppError> {
    let link = PrLink {
        owner: owner.to_owned(),
        name: name.to_owned(),
        number,
    };
    if let Some(row) = stored_row(state, &link)? {
        return Ok(PrLinkHit::Stored { row });
    }
    let item = match provider.pull_request_by_number(owner, name, number) {
        Ok(Some(item)) => item,
        Ok(None)
        | Err(ProviderError::NotFound(_))
        | Err(ProviderError::Forbidden(_))
        | Err(ProviderError::Unsupported(_)) => return Ok(missing(owner, name, number)),
        Err(err) => return Err(err.into()),
    };
    let overview = provider.pull_request_overview(&item.pull_request)?;
    let extra = settings::bot_logins(&state.store)?;
    Ok(PrLinkHit::Opened {
        row: commands::row_for_untracked(&item.pull_request),
        overview: Box::new(overview::for_viewer(overview, &extra)),
    })
}

fn stored_row(state: &AppState, link: &PrLink) -> Result<Option<ListRow>, AppError> {
    let items = commands::list_items(state)?;
    Ok(items
        .iter()
        .find(|item| link.matches(&item.row.owner, &item.row.repo, item.row.number))
        .map(|item| item.row.clone()))
}

fn missing(owner: &str, name: &str, number: u64) -> PrLinkHit {
    PrLinkHit::Missing {
        message: format!("No pull request {owner}/{name}#{number}, or no access."),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::TimeZone;
    use chrono::Utc;
    use pr_nexus_core::domain::{
        Account, AccountId, AccountKind, AuthMode, ChecksState, Involvement, MergeStateStatus,
        Mergeable, PrId, PrState, ProviderRef, PullRequest, RepoRef, Repository, TeamRef, VcsKind,
    };
    use pr_nexus_core::error::ProviderError;
    use pr_nexus_core::overview::{MergeSettings, PrOverview, ViewerAccess};
    use pr_nexus_core::providers::{
        OwnershipInputs, OwnershipProbe, PrQuery, PrStatus, RateBudget, SyncedPullRequest,
        TeamMembers, VcsProvider, VcsUser,
    };

    use super::*;

    struct Fake {
        by_number: AtomicUsize,
        overviews: AtomicUsize,
        found: Option<PullRequest>,
    }

    impl Fake {
        fn found(pr: PullRequest) -> Self {
            Self {
                by_number: AtomicUsize::new(0),
                overviews: AtomicUsize::new(0),
                found: Some(pr),
            }
        }

        fn missing() -> Self {
            Self {
                by_number: AtomicUsize::new(0),
                overviews: AtomicUsize::new(0),
                found: None,
            }
        }
    }

    impl VcsProvider for Fake {
        fn kind(&self) -> VcsKind {
            VcsKind::GitHub
        }

        fn viewer(&self) -> Result<VcsUser, ProviderError> {
            unreachable!("viewer")
        }

        fn search_pull_requests(
            &self,
            _query: &PrQuery,
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            unreachable!("search")
        }

        fn refresh_pull_requests(
            &self,
            _ids: &[ProviderRef],
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            unreachable!("refresh")
        }

        fn pull_request_status(
            &self,
            _ids: &[ProviderRef],
        ) -> Result<Vec<PrStatus>, ProviderError> {
            unreachable!("status")
        }

        fn ownership_probe(&self, _pr: &PullRequest) -> Result<OwnershipProbe, ProviderError> {
            unreachable!("probe")
        }

        fn ownership_inputs(
            &self,
            _pr: &PullRequest,
            _probe: &OwnershipProbe,
        ) -> Result<OwnershipInputs, ProviderError> {
            unreachable!("inputs")
        }

        fn team_members(&self, _team: &TeamRef) -> Result<TeamMembers, ProviderError> {
            unreachable!("team")
        }

        fn rate_limit(&self) -> Result<RateBudget, ProviderError> {
            unreachable!("rate")
        }

        fn request_team_review(
            &self,
            _pr: &PullRequest,
            _teams: &[TeamRef],
        ) -> Result<(), ProviderError> {
            unreachable!("request")
        }

        fn pull_request_by_number(
            &self,
            _owner: &str,
            _name: &str,
            _number: u64,
        ) -> Result<Option<SyncedPullRequest>, ProviderError> {
            self.by_number.fetch_add(1, Ordering::SeqCst);
            Ok(self.found.clone().map(|pull_request| SyncedPullRequest {
                pull_request,
                repo_node_id: None,
                default_branch: Some("main".into()),
                repo_archived: false,
                merge_queue: None,
            }))
        }

        fn pull_request_overview(&self, _pr: &PullRequest) -> Result<PrOverview, ProviderError> {
            self.overviews.fetch_add(1, Ordering::SeqCst);
            Ok(overview())
        }
    }

    fn overview() -> PrOverview {
        PrOverview {
            state: "open".into(),
            is_draft: false,
            base_ref: "main".into(),
            head_ref: "feature/retry".into(),
            head_sha: "abc".into(),
            base_sha: "def".into(),
            head_owner: "acme".into(),
            mergeable: "mergeable".into(),
            merge_state: "clean".into(),
            review_decision: None,
            issue_comments: 0,
            review_comments: 0,
            checks: Vec::new(),
            checks_total: 0,
            approvals: 0,
            required_approvals: None,
            reviewers: Vec::new(),
            author: None,
            assignees: Vec::new(),
            commenters: Vec::new(),
            feedback: Vec::new(),
            bot_reviews: pr_nexus_core::bots::BotReviews::default(),
            viewer: ViewerAccess {
                login: "ada".into(),
                permission: Some("write".into()),
                can_merge_as_admin: false,
                can_enable_auto_merge: false,
                can_disable_auto_merge: false,
                can_close: Some(true),
            },
            merge_settings: MergeSettings {
                auto_merge_allowed: false,
                squash: true,
                merge: true,
                rebase: true,
            },
            auto_merge: None,
            merge_queue: None,
            updated_at: "2026-11-20T09:00:00Z".into(),
            unresolved_threads: 0,
            total_threads: 0,
            fetched_at: "2026-11-20T09:00:00Z".into(),
        }
    }

    fn sample(number: u64) -> PullRequest {
        let now = Utc.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap();
        PullRequest {
            id: PrId(0),
            remote: ProviderRef {
                account: AccountId(0),
                id: format!("PR_{number}"),
            },
            repo: RepoRef {
                host: "github.com".into(),
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number,
            title: "Add a retry budget".into(),
            url: format!("https://github.com/acme/widgets/pull/{number}"),
            author_login: "lee".into(),
            body: String::new(),
            state: PrState::Open,
            is_draft: false,
            head_ref: "feature/retry".into(),
            base_ref: "main".into(),
            head_sha: "abc".into(),
            labels: Vec::new(),
            review_decision: None,
            checks: ChecksState::None,
            mergeable: Mergeable::Unknown,
            merge_state: MergeStateStatus::Unknown,
            requested_reviewers: Vec::new(),
            assignees: Vec::new(),
            unresolved_threads: 0,
            total_threads: 0,
            involvement: Involvement::empty(),
            created_at: now,
            updated_at: now,
            last_activity_at: now,
            synced_at: now,
            hidden: false,
        }
    }

    fn state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::for_tests(&dir.path().join("pr-nexus.db"), false);
        (dir, state)
    }

    #[test]
    fn a_missing_pull_request_is_fetched_and_not_stored() {
        let (_dir, state) = state();
        let before = state.store.revision();
        let fake = Fake::found(sample(332));
        let hit = lookup(&state, &fake, "acme", "widgets", 332).unwrap();
        match hit {
            PrLinkHit::Opened { row, overview } => {
                assert_eq!(row.id, 0);
                assert_eq!(row.number, 332);
                assert_eq!(row.title, "Add a retry budget");
                assert_eq!(overview.head_ref, "feature/retry");
            }
            other => panic!("expected opened, got {other:?}"),
        }
        assert_eq!(fake.by_number.load(Ordering::SeqCst), 1);
        assert_eq!(fake.overviews.load(Ordering::SeqCst), 1);
        assert_eq!(state.store.revision(), before);
    }

    #[test]
    fn github_with_no_such_pull_request_does_not_fetch_an_overview() {
        let (_dir, state) = state();
        let before = state.store.revision();
        let fake = Fake::missing();
        let hit = lookup(&state, &fake, "acme", "widgets", 332).unwrap();
        match hit {
            PrLinkHit::Missing { message } => {
                assert_eq!(message, "No pull request acme/widgets#332, or no access.");
            }
            other => panic!("expected missing, got {other:?}"),
        }
        assert_eq!(fake.by_number.load(Ordering::SeqCst), 1);
        assert_eq!(fake.overviews.load(Ordering::SeqCst), 0);
        assert_eq!(state.store.revision(), before);
    }

    #[test]
    fn a_stored_pull_request_is_not_fetched_again() {
        let (_dir, state) = state();
        let now = Utc.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap();
        let account = state
            .store
            .insert_account(Account {
                id: AccountId(0),
                kind: AccountKind::GitHub,
                display_name: "ada".into(),
                host: "github.com".into(),
                external_id: "U_ada".into(),
                slack_workspace: None,
                login: "ada".into(),
                auth_mode: AuthMode::GhCli,
                keychain_key: None,
                created_at: now,
            })
            .unwrap();
        let repo = state
            .store
            .insert_repository(Repository {
                id: 0,
                account_id: account.id,
                owner: "acme".into(),
                name: "widgets".into(),
                node_id: Some("R_widgets".into()),
                default_branch: Some("main".into()),
                archived: false,
            })
            .unwrap();
        let mut pr = sample(332);
        pr.remote.account = account.id;
        pr.involvement = Involvement::REVIEW_REQUESTED;
        pr.author_login = "lee".into();
        state.store.upsert_pull_request(repo.id, pr).unwrap();
        let fake = Fake::found(sample(332));
        let hit = lookup(&state, &fake, "ACME", "Widgets", 332).unwrap();
        match hit {
            PrLinkHit::Stored { row } => {
                assert_ne!(row.id, 0);
                assert_eq!(row.number, 332);
                assert_eq!(row.title, "Add a retry budget");
            }
            other => panic!("expected stored, got {other:?}"),
        }
        assert_eq!(fake.by_number.load(Ordering::SeqCst), 0);
    }
}
