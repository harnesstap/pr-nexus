//! Dev-only sample data. Invented org `acme`. Enabled with `PR_NEXUS_DEMO=1`.

use chrono::{Duration, Utc};
use pr_nexus_core::domain::{
    Account, AccountId, AccountKind, AuthMode, ChecksState, Involvement, LinkSource,
    MergeStateStatus, Mergeable, PrId, PrState, Project, ProjectId, ProviderRef, RepoRef,
    RequestSource, RequestStatus, ReviewDecision, ReviewRequest, ReviewRequestId,
};
use pr_nexus_core::onboarding::{self, OnboardingState};
use pr_nexus_core::providers::SetupStep;
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
                merge_state: MergeStateStatus::Clean,
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
    Ok(())
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
