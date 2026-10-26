use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::TimeZone;

use super::*;
use crate::domain::{
    Account, AccountId, AccountKind, AuthMode, ChecksState, Involvement, MergeStateStatus,
    Mergeable, PrId, PrState, ProviderRef, RepoRef, Repository, VcsKind,
};
use crate::providers::{
    CodeownersBlob, PrQuery, RateBudget, SyncedPullRequest, TeamMembers, VcsUser,
};
use crate::review::owners::{ReviewBundle, ReviewRecord};
use crate::review::readiness::CODE_OWNER_REVIEW_UNKNOWN;

const CODEOWNERS: &str = "* @acme/core\n/docs/ @acme/docs\n";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap()
}

struct Fake {
    reviews: Vec<ReviewRecord>,
    protection: ClassicProtection,
    rules: Option<bool>,
    members: fn() -> Result<TeamMembers, ProviderError>,
    input_calls: AtomicU32,
    member_calls: Mutex<Vec<String>>,
}

impl Fake {
    fn new() -> Self {
        Self {
            reviews: Vec::new(),
            protection: ClassicProtection::Unknown {
                reason: "branch protection is only visible to repository admins".into(),
            },
            rules: Some(true),
            members: || {
                Ok(TeamMembers {
                    complete: true,
                    logins: vec!["bob".into()],
                })
            },
            input_calls: AtomicU32::new(0),
            member_calls: Mutex::new(Vec::new()),
        }
    }
}

fn approval(login: &str, behalf: &[&str]) -> ReviewRecord {
    ReviewRecord {
        id: Some(format!("R_{login}")),
        state: "APPROVED".into(),
        author_typename: "User".into(),
        author_login: login.into(),
        commit: Some("h1".into()),
        on_behalf_of: behalf
            .iter()
            .map(|slug| crate::review::owners::OnBehalfOf {
                org: "acme".into(),
                slug: (*slug).into(),
            })
            .collect(),
    }
}

impl VcsProvider for Fake {
    fn kind(&self) -> VcsKind {
        VcsKind::GitHub
    }
    fn viewer(&self) -> Result<VcsUser, ProviderError> {
        Err(ProviderError::Unsupported("not used".into()))
    }
    fn search_pull_requests(&self, _: &PrQuery) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        Err(ProviderError::Unsupported("not used".into()))
    }
    fn refresh_pull_requests(
        &self,
        _: &[ProviderRef],
    ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        Err(ProviderError::Unsupported("not used".into()))
    }
    fn ownership_probe(&self, _: &PullRequest) -> Result<OwnershipProbe, ProviderError> {
        Ok(OwnershipProbe {
            head_sha: "h1".into(),
            base_ref: "main".into(),
            codeowners: Some(CodeownersBlob {
                path: ".github/CODEOWNERS".into(),
                oid: "blob1".into(),
                text: CODEOWNERS.into(),
            }),
            reviews: ReviewBundle {
                fetched: true,
                complete: true,
                reviews: self.reviews.clone(),
            },
            viewer_is_admin: false,
        })
    }
    fn ownership_inputs(
        &self,
        _: &PullRequest,
        _: &OwnershipProbe,
    ) -> Result<OwnershipInputs, ProviderError> {
        self.input_calls.fetch_add(1, Ordering::SeqCst);
        Ok(OwnershipInputs {
            files: vec!["src/lib.rs".into()],
            files_truncated: false,
            rules_require_code_owner: self.rules,
            rules_error: None,
            protection: self.protection.clone(),
        })
    }
    fn team_members(&self, team: &TeamRef) -> Result<TeamMembers, ProviderError> {
        self.member_calls.lock().unwrap().push(team.to_string());
        (self.members)()
    }
    fn rate_limit(&self) -> Result<RateBudget, ProviderError> {
        Err(ProviderError::Unsupported("not used".into()))
    }
    fn request_team_review(&self, _: &PullRequest, _: &[TeamRef]) -> Result<(), ProviderError> {
        Err(ProviderError::Unsupported("not used".into()))
    }
}

fn seed() -> (tempfile::TempDir, Store, PullRequest) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("ownership.db")).unwrap();
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
            created_at: now(),
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
        // GitHub reports no decision while BLOCKED: the nit this milestone fixes.
        review_decision: None,
        checks: ChecksState::Success,
        mergeable: Mergeable::Mergeable,
        merge_state: MergeStateStatus::Blocked,
        requested_reviewers: Vec::new(),
        unresolved_threads: 0,
        total_threads: 0,
        involvement: Involvement::AUTHOR,
        created_at: now(),
        updated_at: now(),
        last_activity_at: now(),
        synced_at: now(),
        hidden: false,
    };
    let pr = store.upsert_pull_request(repo.id, pr).unwrap();
    (dir, store, pr)
}

#[test]
fn a_pending_code_owner_review_makes_a_null_decision_ready() {
    let (_dir, store, pr) = seed();
    let fake = Fake::new();
    let report = resolve(&store, &fake, &pr, true, now()).unwrap();
    assert_eq!(report.code_owner_review_pending, Some(true));
    assert!(
        matches!(report.readiness, Readiness::Ready { .. }),
        "{:?}",
        report.readiness
    );
    assert_eq!(report.decision.codeowners_required, Tri::True);
    assert_eq!(
        report.codeowners_path.as_deref(),
        Some(".github/CODEOWNERS")
    );
    assert_eq!(report.files[0].teams[0].to_string(), "acme/core");
    // No approval to attribute, so no membership lookups.
    assert!(fake.member_calls.lock().unwrap().is_empty());
}

#[test]
fn an_unreadable_team_leaves_code_owner_status_unknown() {
    let (_dir, store, pr) = seed();
    let mut fake = Fake::new();
    fake.reviews = vec![approval("bob", &[])];
    fake.members = || {
        Ok(TeamMembers {
            complete: false,
            logins: Vec::new(),
        })
    };
    let report = resolve(&store, &fake, &pr, true, now()).unwrap();
    assert_eq!(report.code_owner_review_pending, None);
    assert_eq!(
        report.readiness,
        Readiness::Unknown {
            reasons: vec![CODE_OWNER_REVIEW_UNKNOWN.into()]
        }
    );
    assert!(!report.decision.review_attribution_unknown.is_empty());
}

#[test]
fn an_approval_on_behalf_of_the_owner_team_needs_no_lookup() {
    let (_dir, store, pr) = seed();
    let mut fake = Fake::new();
    fake.reviews = vec![approval("bob", &["core"])];
    let report = resolve(&store, &fake, &pr, true, now()).unwrap();
    assert_eq!(report.code_owner_review_pending, Some(false));
    assert!(fake.member_calls.lock().unwrap().is_empty());
    // GitHub still says BLOCKED with nothing pending: something else blocks.
    assert!(matches!(report.readiness, Readiness::Unknown { .. }));
}

#[test]
fn unknown_protection_without_rules_is_unknown_never_false() {
    let (_dir, store, pr) = seed();
    let mut fake = Fake::new();
    fake.rules = Some(false);
    let report = resolve(&store, &fake, &pr, true, now()).unwrap();
    assert_eq!(report.decision.codeowners_required, Tri::Unknown);
    assert_eq!(report.code_owner_review_pending, None);
}

#[test]
fn the_cache_skips_inputs_and_member_lookups_on_the_same_key() {
    let (_dir, store, pr) = seed();
    let mut fake = Fake::new();
    fake.reviews = vec![approval("bob", &[])];
    let first = resolve(&store, &fake, &pr, true, now()).unwrap();
    let second = resolve(&store, &fake, &pr, true, now() + Duration::hours(1)).unwrap();
    assert!(!first.from_cache);
    assert!(second.from_cache);
    assert_eq!(fake.input_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fake.member_calls.lock().unwrap().len(), 1);
    assert_eq!(
        first.decision.approved_teams,
        second.decision.approved_teams
    );

    let entry = store.get_ownership_cache(pr.id).unwrap().unwrap();
    let cached = CachedOwnership::for_pr(&entry, &pr).unwrap();
    assert_eq!(cached.code_owner_review_pending, Some(false));
    let mut moved = pr.clone();
    moved.head_sha = "h2".into();
    assert!(CachedOwnership::for_pr(&entry, &moved).is_none());
}

#[test]
fn a_rate_limit_on_team_members_stops_resolution() {
    let (_dir, store, pr) = seed();
    let mut fake = Fake::new();
    fake.reviews = vec![approval("bob", &[])];
    fake.members = || {
        Err(ProviderError::RateLimited {
            reset_at: now(),
            secondary: false,
        })
    };
    assert!(resolve(&store, &fake, &pr, true, now()).is_err());
}

#[test]
fn a_network_error_on_team_members_is_not_cached() {
    let (_dir, store, pr) = seed();
    let mut fake = Fake::new();
    fake.reviews = vec![approval("bob", &[])];
    fake.members = || Err(ProviderError::Network("reset".into()));
    let report = resolve(&store, &fake, &pr, true, now()).unwrap();
    assert_eq!(report.code_owner_review_pending, None);
    assert!(
        store
            .get_team_members("acme".into(), "core".into())
            .unwrap()
            .is_none()
    );
}

#[test]
fn list_rows_read_the_cache_and_the_sync_pass_fills_it() {
    let (_dir, store, pr) = seed();
    let fake = Fake::new();
    assert_eq!(
        cached_readiness(&store, &pr, true).unwrap(),
        Readiness::Unknown {
            reasons: vec![CODE_OWNER_REVIEW_UNKNOWN.into()]
        }
    );
    let mut clean = pr.clone();
    clean.id = PrId(pr.id.0 + 1000);
    clean.merge_state = MergeStateStatus::Clean;
    let done = resolve_unknown_code_owner_reviews(
        &store,
        &fake,
        &[clean, pr.clone()],
        true,
        now(),
        MAX_RESOLVES_PER_SYNC,
    )
    .unwrap();
    assert_eq!(done, 1);
    assert!(matches!(
        cached_readiness(&store, &pr, true).unwrap(),
        Readiness::Ready { .. }
    ));
    // Already known: the next pass makes no calls.
    let again = resolve_unknown_code_owner_reviews(
        &store,
        &fake,
        &[pr],
        true,
        now(),
        MAX_RESOLVES_PER_SYNC,
    )
    .unwrap();
    assert_eq!(again, 0);
    assert_eq!(fake.input_calls.load(Ordering::SeqCst), 1);
}
