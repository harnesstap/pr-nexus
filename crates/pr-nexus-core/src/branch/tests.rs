use std::sync::Mutex;

use chrono::{TimeZone, Utc};

use super::*;
use crate::domain::{
    Account, AccountId, AccountKind, AuthMode, ChecksState, Involvement, MergeStateStatus,
    Mergeable, PrState, ProviderRef, RepoRef, Repository, TeamRef, VcsKind,
};
use crate::overview::{MergeSettings, ViewerAccess};
use crate::providers::{
    OwnershipInputs, OwnershipProbe, PrQuery, RateBudget, SyncedPullRequest, TeamMembers, VcsUser,
};

#[test]
fn state_derivation_covers_every_card() {
    use BranchState::*;
    assert_eq!(derive_state("mergeable", "clean", Some(0), false), UpToDate);
    assert_eq!(derive_state("mergeable", "blocked", None, false), UpToDate);
    assert_eq!(derive_state("mergeable", "clean", Some(3), false), Behind);
    assert_eq!(
        derive_state("mergeable", "blocked", Some(3), true),
        UpdateRequired
    );
    assert_eq!(
        derive_state("mergeable", "behind", Some(2), false),
        UpdateRequired
    );
    assert_eq!(
        derive_state("mergeable", "behind", None, true),
        UpdateRequired
    );
    assert_eq!(
        derive_state("conflicting", "dirty", Some(4), true),
        Conflicts
    );
    assert_eq!(derive_state("unknown", "dirty", None, false), Conflicts);
    assert_eq!(derive_state("unknown", "unknown", Some(1), false), Checking);
}

#[test]
fn states_serialize_snake_case() {
    let names: Vec<String> = [
        BranchState::UpToDate,
        BranchState::Behind,
        BranchState::UpdateRequired,
        BranchState::Conflicts,
        BranchState::Checking,
    ]
    .iter()
    .map(|state| serde_json::to_string(state).unwrap())
    .collect();
    assert_eq!(
        names,
        [
            "\"up_to_date\"",
            "\"behind\"",
            "\"update_required\"",
            "\"conflicts\"",
            "\"checking\""
        ]
    );
    assert_eq!(UpdateMethod::parse("REBASE"), Some(UpdateMethod::Rebase));
    assert_eq!(UpdateMethod::parse("squash"), None);
}

/// A fake GitHub: answers overviews from `overview`, compares from
/// `comparison`, and records every update instead of sending it.
struct Fake {
    overview: Mutex<PrOverview>,
    comparison: BranchComparison,
    compares: Mutex<Vec<(String, String)>>,
    updates: Mutex<Vec<(String, UpdateMethod)>>,
    update_error: Mutex<Option<ProviderError>>,
}

impl Fake {
    fn new(overview: PrOverview) -> Self {
        Self {
            overview: Mutex::new(overview),
            comparison: BranchComparison {
                behind_by: 3,
                ahead_by: 2,
                authors: vec!["alice".into(), "bob".into()],
            },
            compares: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
            update_error: Mutex::new(None),
        }
    }
}

fn unused<T>() -> Result<T, ProviderError> {
    Err(ProviderError::Unsupported("not used".into()))
}

impl VcsProvider for Fake {
    fn kind(&self) -> VcsKind {
        VcsKind::GitHub
    }
    fn viewer(&self) -> Result<VcsUser, ProviderError> {
        unused()
    }
    fn search_pull_requests(&self, _: &PrQuery) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        unused()
    }
    fn refresh_pull_requests(
        &self,
        _: &[ProviderRef],
    ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        unused()
    }
    fn pull_request_status(
        &self,
        _: &[ProviderRef],
    ) -> Result<Vec<crate::providers::PrStatus>, ProviderError> {
        unused()
    }
    fn ownership_probe(&self, _: &PullRequest) -> Result<OwnershipProbe, ProviderError> {
        unused()
    }
    fn ownership_inputs(
        &self,
        _: &PullRequest,
        _: &OwnershipProbe,
    ) -> Result<OwnershipInputs, ProviderError> {
        unused()
    }
    fn team_members(&self, _: &TeamRef) -> Result<TeamMembers, ProviderError> {
        unused()
    }
    fn rate_limit(&self) -> Result<RateBudget, ProviderError> {
        unused()
    }
    fn request_team_review(&self, _: &PullRequest, _: &[TeamRef]) -> Result<(), ProviderError> {
        unused()
    }
    fn pull_request_overview(&self, _: &PullRequest) -> Result<PrOverview, ProviderError> {
        Ok(self.overview.lock().unwrap().clone())
    }
    fn compare_branches(
        &self,
        _: &PullRequest,
        base: &BranchRef,
        head: &BranchRef,
    ) -> Result<BranchComparison, ProviderError> {
        self.compares.lock().unwrap().push((
            format!("{}:{}", base.owner, base.name),
            format!("{}:{}", head.owner, head.name),
        ));
        Ok(self.comparison.clone())
    }
    fn update_branch(
        &self,
        _: &PullRequest,
        expected_head: &str,
        method: UpdateMethod,
    ) -> Result<(), ProviderError> {
        if let Some(err) = self.update_error.lock().unwrap().take() {
            return Err(err);
        }
        self.updates
            .lock()
            .unwrap()
            .push((expected_head.into(), method));
        let mut overview = self.overview.lock().unwrap();
        overview.head_sha = "h2".into();
        overview.merge_state = "unknown".into();
        overview.mergeable = "unknown".into();
        Ok(())
    }
}

fn overview(mergeable: &str, merge_state: &str) -> PrOverview {
    PrOverview {
        state: "open".into(),
        is_draft: false,
        base_ref: "main".into(),
        head_ref: "latch".into(),
        head_sha: "h1".into(),
        base_sha: "b1".into(),
        head_owner: "alice-fork".into(),
        mergeable: mergeable.into(),
        merge_state: merge_state.into(),
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
        viewer: ViewerAccess {
            login: "Alice".into(),
            permission: Some("write".into()),
            can_merge_as_admin: false,
            can_enable_auto_merge: false,
            can_disable_auto_merge: false,
        },
        merge_settings: MergeSettings {
            auto_merge_allowed: false,
            squash: true,
            merge: true,
            rebase: true,
        },
        auto_merge: None,
        fetched_at: "2026-10-26T12:00:00Z".into(),
    }
}

fn seed() -> (tempfile::TempDir, Store, PullRequest) {
    let now = Utc.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("branch.db")).unwrap();
    let account = store
        .insert_account(Account {
            id: AccountId(0),
            kind: AccountKind::GitHub,
            display_name: "alice".into(),
            host: "github.com".into(),
            external_id: "U_alice".into(),
            slack_workspace: None,
            login: "alice".into(),
            auth_mode: AuthMode::GhCli,
            keychain_key: None,
            created_at: now,
        })
        .unwrap();
    let repo = store
        .insert_repository(Repository {
            id: 0,
            account_id: account.id,
            owner: "acme".into(),
            name: "widgets".into(),
            node_id: Some("R_1".into()),
            default_branch: Some("main".into()),
            archived: false,
        })
        .unwrap();
    let pr = PullRequest {
        id: PrId(0),
        remote: ProviderRef {
            account: account.id,
            id: "PR_1".into(),
        },
        repo: RepoRef {
            host: "github.com".into(),
            owner: "acme".into(),
            name: "widgets".into(),
        },
        number: 7,
        title: "Tighten the widget latch".into(),
        url: "https://github.com/acme/widgets/pull/7".into(),
        author_login: "alice".into(),
        body: String::new(),
        state: PrState::Open,
        is_draft: false,
        head_ref: "latch".into(),
        base_ref: "main".into(),
        head_sha: "h1".into(),
        labels: Vec::new(),
        review_decision: None,
        checks: ChecksState::Success,
        mergeable: Mergeable::Mergeable,
        merge_state: MergeStateStatus::Clean,
        requested_reviewers: Vec::new(),
        unresolved_threads: 0,
        total_threads: 0,
        involvement: Involvement::AUTHOR,
        created_at: now,
        updated_at: now,
        last_activity_at: now,
        synced_at: now,
        hidden: false,
    };
    let pr = store.upsert_pull_request(repo.id, pr).unwrap();
    (dir, store, pr)
}

#[test]
fn status_compares_fork_sides_once_per_sha_pair_and_drops_the_viewer() {
    let (_dir, store, pr) = seed();
    let fake = Fake::new(overview("mergeable", "clean"));
    let cache = BranchCache::default();
    let first = status(&store, &fake, &cache, &pr).unwrap();
    assert_eq!(first.state, BranchState::Behind);
    assert_eq!(first.behind_by, Some(3));
    assert_eq!(first.other_authors, vec!["bob".to_string()]);
    assert_eq!(
        first.conflicts_url,
        "https://github.com/acme/widgets/pull/7/conflicts"
    );
    let again = status(&store, &fake, &cache, &pr).unwrap();
    assert_eq!(again, first);
    assert_eq!(
        *fake.compares.lock().unwrap(),
        vec![("acme:main".to_string(), "alice-fork:latch".to_string())]
    );
    fake.overview.lock().unwrap().base_sha = "b2".into();
    status(&store, &fake, &cache, &pr).unwrap();
    assert_eq!(fake.compares.lock().unwrap().len(), 2);
}

#[test]
fn conflicts_skip_the_compare() {
    let (_dir, store, pr) = seed();
    let fake = Fake::new(overview("conflicting", "dirty"));
    let found = status(&store, &fake, &BranchCache::default(), &pr).unwrap();
    assert_eq!(found.state, BranchState::Conflicts);
    assert_eq!(found.behind_by, None);
    assert!(fake.compares.lock().unwrap().is_empty());
}

#[test]
fn update_sends_the_seen_head_and_refreshes() {
    let (_dir, store, pr) = seed();
    let fake = Fake::new(overview("mergeable", "behind"));
    let cache = BranchCache::default();
    assert_eq!(
        status(&store, &fake, &cache, &pr).unwrap().state,
        BranchState::UpdateRequired
    );
    let updated = update(&store, &fake, &cache, &pr, UpdateMethod::Rebase).unwrap();
    assert_eq!(
        *fake.updates.lock().unwrap(),
        vec![("h1".to_string(), UpdateMethod::Rebase)]
    );
    assert_eq!(updated.overview.head_sha, "h2");
    assert_eq!(updated.branch.state, BranchState::Checking);
    assert_eq!(
        overview::cached(&store, pr.id).unwrap().unwrap().head_sha,
        "h2"
    );
}

#[test]
fn update_errors_become_codes_the_banner_can_explain() {
    let (_dir, store, pr) = seed();
    let cases = [
        (
            "expected head sha didn't match current head ref",
            "head_moved",
        ),
        ("merge conflict between base and head", "conflicts"),
    ];
    for (message, code) in cases {
        let fake = Fake::new(overview("mergeable", "behind"));
        *fake.update_error.lock().unwrap() = Some(ProviderError::Api {
            code: "UNPROCESSABLE".into(),
            message: message.into(),
        });
        let err = update(
            &store,
            &fake,
            &BranchCache::default(),
            &pr,
            UpdateMethod::Merge,
        )
        .unwrap_err();
        match err {
            CoreError::Provider(ProviderError::Api { code: found, .. }) => assert_eq!(found, code),
            other => panic!("unexpected {other:?}"),
        }
        assert!(fake.updates.lock().unwrap().is_empty());
    }
    let fake = Fake::new(overview("mergeable", "behind"));
    *fake.update_error.lock().unwrap() = Some(ProviderError::Api {
        code: "FORBIDDEN".into(),
        message: "Viewer does not have permission to update this branch".into(),
    });
    let err = update(
        &store,
        &fake,
        &BranchCache::default(),
        &pr,
        UpdateMethod::Merge,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        CoreError::Provider(ProviderError::Forbidden(_))
    ));
}
