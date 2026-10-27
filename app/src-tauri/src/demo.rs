//! Dev-only sample data. Invented org `acme`. Enabled with `PR_NEXUS_DEMO=1`.

use chrono::{Duration, Utc};
use pr_nexus_core::domain::{
    Account, AccountId, AccountKind, AuthMode, ChecksState, Involvement, LinkSource,
    MergeStateStatus, Mergeable, PrId, PrState, Project, ProjectId, ProviderRef, RepoRef,
    RequestSource, RequestStatus, ReviewDecision, ReviewRequest, ReviewRequestId,
};
use pr_nexus_core::domain::{PullRequest, TeamRef, VcsKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::onboarding::{self, OnboardingState};
use pr_nexus_core::providers::{
    ClassicProtection, CodeownersBlob, OwnershipInputs, OwnershipProbe, PrQuery, RateBudget,
    SetupStep, SyncedPullRequest, TeamMembers, VcsProvider, VcsUser,
};
use pr_nexus_core::review::owners::{OnBehalfOf, ReviewBundle, ReviewRecord};
use pr_nexus_core::settings;
use pr_nexus_core::store::Store;

use crate::error::AppError;

pub fn seed_if_empty(store: &Store) -> Result<(), AppError> {
    if !store.list_accounts()?.is_empty() {
        return Ok(());
    }
    let now = Utc::now();
    let account = store.insert_account(Account {
        id: AccountId(0),
        kind: AccountKind::GitHub,
        display_name: "ada".into(),
        host: "github.com".into(),
        external_id: "U_demo".into(),
        slack_workspace: None,
        login: "ada".into(),
        auth_mode: AuthMode::Pat,
        keychain_key: Some("github:U_demo".into()),
        created_at: now,
    })?;
    settings::set_provider_kind(store, "github", "slack")?;
    settings::set_tracked_orgs(store, &["acme".into()])?;
    settings::set_viewer_orgs(store, &["acme".into()])?;
    let repo = store.upsert_repository(pr_nexus_core::domain::Repository {
        id: 0,
        account_id: account.id,
        owner: "acme".into(),
        name: "widgets".into(),
        node_id: Some("R_demo".into()),
        default_branch: Some("main".into()),
        archived: false,
    })?;
    let auth = store.insert_project(Project {
        id: ProjectId(0),
        name: "Auth".into(),
        description: "Sign-in work".into(),
        color: "#7c5cbf".into(),
        archived: false,
        sort_order: 0,
    })?;
    let billing = store.insert_project(Project {
        id: ProjectId(0),
        name: "Billing".into(),
        description: String::new(),
        color: "#3f6f8f".into(),
        archived: false,
        sort_order: 1,
    })?;
    let specs = [
        (
            1u64,
            // BLOCKED with no review decision: readiness needs the code-owner probe.
            "Add a latch",
            ChecksState::Success,
            None,
            Involvement::AUTHOR,
            Some(auth.id),
            true,
        ),
        (
            2,
            "Refresh the drawer token",
            ChecksState::Pending,
            Some(ReviewDecision::ReviewRequired),
            Involvement::AUTHOR,
            Some(auth.id),
            false,
        ),
        (
            3,
            "Fix the tax rounding",
            ChecksState::Failure,
            Some(ReviewDecision::ChangesRequested),
            Involvement::AUTHOR,
            Some(billing.id),
            false,
        ),
        (
            4,
            "Document the latch",
            ChecksState::Success,
            Some(ReviewDecision::Approved),
            Involvement::REVIEW_REQUESTED,
            None,
            false,
        ),
    ];
    for (number, title, checks, decision, involvement, project, requested) in specs {
        let pr = store.upsert_pull_request(
            repo.id,
            pr_nexus_core::domain::PullRequest {
                id: PrId(0),
                remote: ProviderRef {
                    account: account.id,
                    id: format!("PR_demo_{number}"),
                },
                repo: RepoRef {
                    host: "github.com".into(),
                    owner: "acme".into(),
                    name: "widgets".into(),
                },
                number,
                title: title.into(),
                url: format!("https://github.com/acme/widgets/pull/{number}"),
                author_login: "ada".into(),
                body: "Demo pull request body for search.".into(),
                state: PrState::Open,
                is_draft: false,
                head_ref: format!("feat/{number}"),
                base_ref: "main".into(),
                head_sha: format!("abc{number}"),
                labels: vec!["demo".into()],
                review_decision: decision,
                checks,
                mergeable: Mergeable::Mergeable,
                merge_state: if number == 1 {
                    MergeStateStatus::Blocked
                } else {
                    MergeStateStatus::Clean
                },
                requested_reviewers: Vec::new(),
                unresolved_threads: 0,
                total_threads: 1,
                involvement,
                created_at: now - Duration::days(3),
                updated_at: now - Duration::hours(2),
                last_activity_at: now - Duration::hours(2),
                synced_at: now,
                hidden: false,
            },
        )?;
        if let Some(project) = project {
            store.upsert_pr_project(pr_nexus_core::domain::PrProjectLink {
                pr: pr.id,
                project,
                source: LinkSource::Manual,
                created_at: now,
            })?;
        }
        if requested {
            store.insert_review_request(ReviewRequest {
                id: ReviewRequestId(0),
                pr: pr.id,
                root: ProviderRef {
                    account: account.id,
                    id: "C0123456789:1700000000.000100".into(),
                },
                workspace: "acme".into(),
                channel_id: "C0123456789".into(),
                channel_name: "team-reviews".into(),
                root_ts: "1700000000.000100".into(),
                permalink: Some(
                    "https://acme.slack.com/archives/C0123456789/p1700000000000100".into(),
                ),
                requested_at: now - Duration::days(2),
                last_bumped_at: Some(now - Duration::hours(5)),
                bump_count: 1,
                status: RequestStatus::Active,
                closed_at: None,
                source: RequestSource::App,
            })?;
        }
    }
    let mut saved = OnboardingState::fresh();
    saved.step = SetupStep::Done.as_str().into();
    saved.completed = wizard_steps();
    saved.slack_skipped = true;
    saved.mapping_skipped = true;
    saved.finished_at = Some(now.to_rfc3339());
    onboarding::save(store, &saved)?;
    let prs = store.list_pull_requests_for_account(account.id)?;
    let flag = settings::blocked_review_required_is_ready(store)?;
    pr_nexus_core::ownership::resolve_unknown_code_owner_reviews(
        store,
        &DemoProvider,
        &prs,
        flag,
        now,
        pr_nexus_core::ownership::MAX_RESOLVES_PER_SYNC,
    )?;
    Ok(())
}

const DEMO_CODEOWNERS: &str = "\
# Demo owners for acme/widgets
*          @acme/platform
/docs/     @acme/docs
/billing/  @acme/billing
";

/// Fixture answers for the detail view in demo mode. Never touches the network.
pub struct DemoProvider;

impl DemoProvider {
    fn files(number: u64) -> Vec<String> {
        let files: &[&str] = match number {
            1 => &["src/latch.rs", "docs/latch.md"],
            2 => &["src/auth/token.rs"],
            3 => &["billing/tax.rs", "billing/tests/tax.rs"],
            _ => &["docs/latch.md"],
        };
        files.iter().map(|path| (*path).to_owned()).collect()
    }

    fn reviews(number: u64) -> Vec<ReviewRecord> {
        let review = |login: &str, state: &str, team: Option<&str>| ReviewRecord {
            id: Some(format!("PRR_demo_{number}_{login}")),
            state: state.into(),
            author_typename: "User".into(),
            author_login: login.into(),
            commit: Some(format!("abc{number}")),
            on_behalf_of: team
                .map(|slug| OnBehalfOf {
                    org: "acme".into(),
                    slug: slug.into(),
                })
                .into_iter()
                .collect(),
        };
        match number {
            2 => vec![review("linus", "APPROVED", None)],
            3 => vec![
                review("grace", "CHANGES_REQUESTED", None),
                review("hopper", "APPROVED", None),
            ],
            4 => vec![review("grace", "APPROVED", Some("docs"))],
            _ => Vec::new(),
        }
    }
}

impl DemoProvider {
    /// Invented overview built from the stored row and the demo reviews.
    fn overview(pr: &PullRequest) -> pr_nexus_core::overview::PrOverview {
        use pr_nexus_core::overview::{
            CheckItem, CheckState, MergeSettings, Person, PersonKind, PrOverview, ReviewState,
            Reviewer as Seat, ViewerAccess,
        };
        let person = |login: &str| Person {
            kind: PersonKind::User,
            login: login.into(),
            name: None,
            avatar_url: None,
        };
        let last = match pr.checks {
            ChecksState::Success => CheckState::Passed,
            ChecksState::Failure | ChecksState::Error => CheckState::Failed,
            ChecksState::None => CheckState::Skipped,
            _ => CheckState::Pending,
        };
        let checks = ["build", "lint", "test"]
            .iter()
            .enumerate()
            .map(|(index, name)| CheckItem {
                name: (*name).into(),
                state: if index == 2 { last } else { CheckState::Passed },
                url: None,
                required: index != 1,
            })
            .collect::<Vec<_>>();
        let mut reviewers: Vec<Seat> = Self::reviews(pr.number)
            .into_iter()
            .map(|review| Seat {
                person: person(&review.author_login),
                state: match review.state.as_str() {
                    "APPROVED" => ReviewState::Approved,
                    "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
                    _ => ReviewState::Commented,
                },
            })
            .collect();
        for request in &pr.requested_reviewers {
            let person = match &request.reviewer {
                pr_nexus_core::domain::Reviewer::User { login } => Person {
                    name: request.name.clone(),
                    avatar_url: request.avatar_url.clone(),
                    ..person(login)
                },
                pr_nexus_core::domain::Reviewer::Team(team) => Person {
                    kind: PersonKind::Team,
                    login: team.to_string(),
                    name: request.name.clone(),
                    avatar_url: request.avatar_url.clone(),
                },
            };
            reviewers.push(Seat {
                person,
                state: ReviewState::Pending,
            });
        }
        let approvals = reviewers
            .iter()
            .filter(|seat| seat.state == ReviewState::Approved)
            .count() as u32;
        let mut participants = vec![person(&pr.author_login)];
        participants.extend(
            reviewers
                .iter()
                .filter(|seat| seat.person.kind == PersonKind::User)
                .map(|seat| seat.person.clone()),
        );
        PrOverview {
            state: pr.state.as_str().into(),
            is_draft: pr.is_draft,
            base_ref: pr.base_ref.clone(),
            head_ref: pr.head_ref.clone(),
            mergeable: pr.mergeable.as_str().into(),
            merge_state: pr.merge_state.as_str().into(),
            review_decision: pr.review_decision.map(|decision| decision.as_str().into()),
            issue_comments: (pr.number % 5) as u32,
            review_comments: pr.total_threads,
            checks_total: checks.len() as u32,
            checks,
            approvals,
            required_approvals: Some(1),
            reviewers,
            participants_total: participants.len() as u32,
            participants,
            viewer: ViewerAccess {
                login: "ada".into(),
                permission: Some("write".into()),
                can_merge_as_admin: false,
                can_enable_auto_merge: !pr.is_draft,
                can_disable_auto_merge: false,
            },
            merge_settings: MergeSettings {
                auto_merge_allowed: true,
                squash: true,
                merge: true,
                rebase: false,
            },
            auto_merge: None,
            fetched_at: Utc::now().to_rfc3339(),
        }
    }
}

impl VcsProvider for DemoProvider {
    fn pull_request_overview(
        &self,
        pr: &PullRequest,
    ) -> Result<pr_nexus_core::overview::PrOverview, ProviderError> {
        Ok(Self::overview(pr))
    }
    fn kind(&self) -> VcsKind {
        VcsKind::GitHub
    }
    fn viewer(&self) -> Result<VcsUser, ProviderError> {
        Err(ProviderError::Unsupported("demo data".into()))
    }
    fn search_pull_requests(&self, _: &PrQuery) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        Err(ProviderError::Unsupported("demo data".into()))
    }
    fn refresh_pull_requests(
        &self,
        _: &[ProviderRef],
    ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        Err(ProviderError::Unsupported("demo data".into()))
    }
    fn pull_request_status(
        &self,
        _: &[ProviderRef],
    ) -> Result<Vec<pr_nexus_core::providers::PrStatus>, ProviderError> {
        Err(ProviderError::Unsupported("demo data".into()))
    }
    fn ownership_probe(&self, pr: &PullRequest) -> Result<OwnershipProbe, ProviderError> {
        Ok(OwnershipProbe {
            head_sha: pr.head_sha.clone(),
            base_ref: pr.base_ref.clone(),
            codeowners: Some(CodeownersBlob {
                path: ".github/CODEOWNERS".into(),
                oid: "demo-codeowners-1".into(),
                text: DEMO_CODEOWNERS.into(),
            }),
            reviews: ReviewBundle {
                fetched: true,
                complete: true,
                reviews: Self::reviews(pr.number),
            },
            viewer_is_admin: false,
        })
    }
    fn ownership_inputs(
        &self,
        pr: &PullRequest,
        _: &OwnershipProbe,
    ) -> Result<OwnershipInputs, ProviderError> {
        Ok(OwnershipInputs {
            files: Self::files(pr.number),
            files_truncated: false,
            rules_require_code_owner: Some(true),
            rules_error: None,
            protection: ClassicProtection::Unknown {
                reason: "branch protection is only visible to repository admins".into(),
            },
        })
    }
    fn team_members(&self, team: &TeamRef) -> Result<TeamMembers, ProviderError> {
        let logins: &[&str] = match team.slug.as_str() {
            "platform" => &["ada", "linus"],
            "docs" => &["grace"],
            // An unreadable team shows up as "attribution unknown".
            _ => {
                return Ok(TeamMembers {
                    complete: false,
                    logins: Vec::new(),
                });
            }
        };
        Ok(TeamMembers {
            complete: true,
            logins: logins.iter().map(|login| (*login).to_owned()).collect(),
        })
    }
    fn rate_limit(&self) -> Result<RateBudget, ProviderError> {
        Err(ProviderError::Unsupported("demo data".into()))
    }
    fn request_team_review(&self, _: &PullRequest, _: &[TeamRef]) -> Result<(), ProviderError> {
        Err(ProviderError::Unsupported("demo data".into()))
    }
}

fn wizard_steps() -> Vec<String> {
    pr_nexus_core::providers::ProviderRegistry::v1()
        .setup_steps(
            pr_nexus_core::domain::VcsKind::GitHub,
            pr_nexus_core::domain::ChatKind::Slack,
        )
        .into_iter()
        .map(|step| step.as_str().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pr_nexus_core::domain::Readiness;
    use pr_nexus_core::ownership;

    #[test]
    fn demo_seed_resolves_the_blocked_pull_request_and_details_explain_owners() {
        let dir = std::env::temp_dir().join(format!("pr-nexus-demo-owners-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("demo.db");
        let _ = std::fs::remove_file(&path);
        let store = Store::open(&path).unwrap();
        seed_if_empty(&store).unwrap();
        let account = store.list_accounts().unwrap().remove(0);
        let prs = store.list_pull_requests_for_account(account.id).unwrap();
        let by_number = |number: u64| prs.iter().find(|pr| pr.number == number).unwrap();

        // PR 1 is BLOCKED with no decision; the platform team still has to approve.
        let latch = by_number(1);
        assert!(matches!(
            ownership::cached_readiness(&store, latch, true).unwrap(),
            Readiness::Ready { .. }
        ));

        // PR 3: billing members are unreadable, so hopper's approval is unattributed.
        let tax = by_number(3);
        let report = ownership::resolve(&store, &DemoProvider, tax, true, Utc::now()).unwrap();
        assert!(
            report
                .decision
                .review_attribution_unknown
                .iter()
                .any(|gap| gap.team.slug == "billing")
        );
        // PR 4: grace approved on behalf of docs.
        let docs = by_number(4);
        let report = ownership::resolve(&store, &DemoProvider, docs, true, Utc::now()).unwrap();
        assert!(
            report
                .decision
                .approved_teams
                .iter()
                .any(|team| team.slug == "docs")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
