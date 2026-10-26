use chrono::{DateTime, Utc};

use super::*;
use crate::domain::{
    Account, AccountId, AccountKind, AuthMode, ChecksState, Involvement, LinkSource,
    MergeStateStatus, Mergeable, ProviderRef, RepoRef, Repository,
};

fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-26T04:40:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn pr(id: i64, repo: (&str, &str), head_ref: &str, title: &str, labels: &[&str]) -> PullRequest {
    PullRequest {
        id: PrId(id),
        remote: ProviderRef {
            account: AccountId(1),
            id: format!("PR_{id}"),
        },
        repo: RepoRef {
            host: "github.com".into(),
            owner: repo.0.into(),
            name: repo.1.into(),
        },
        number: id as u64,
        title: title.into(),
        url: format!("https://github.com/{}/{}/pull/{id}", repo.0, repo.1),
        author_login: "ada".into(),
        body: String::new(),
        state: PrState::Open,
        is_draft: false,
        head_ref: head_ref.into(),
        base_ref: "main".into(),
        head_sha: "abc".into(),
        labels: labels.iter().map(|label| label.to_string()).collect(),
        review_decision: None,
        checks: ChecksState::Success,
        mergeable: Mergeable::Mergeable,
        merge_state: MergeStateStatus::Clean,
        requested_reviewers: Vec::new(),
        unresolved_threads: 0,
        total_threads: 0,
        involvement: Involvement::AUTHOR,
        created_at: at(),
        updated_at: at(),
        last_activity_at: at(),
        synced_at: at(),
        hidden: false,
    }
}

fn rule(id: i64, project: i64, kind: RuleKind, pattern: &str, priority: i32) -> ProjectRule {
    ProjectRule {
        id,
        project: ProjectId(project),
        kind,
        pattern: pattern.into(),
        priority,
    }
}

fn set(rules: Vec<ProjectRule>) -> RuleSet {
    RuleSet::new(rules, DEFAULT_TICKET_KEY_REGEX).unwrap().0
}

fn matches(kind: RuleKind, pattern: &str, pr: &PullRequest) -> bool {
    set(vec![rule(1, 1, kind, pattern, 0)])
        .first_match(pr, ProjectId(1))
        .is_some()
}

#[test]
fn ticket_keys_come_from_branch_or_title_and_compare_case_insensitively() {
    let branch = pr(1, ("acme", "web"), "feat/proj-123-retry", "Retry", &[]);
    let title = pr(2, ("acme", "web"), "retry", "PROJ-77: Retry uploads", &[]);
    let other = pr(3, ("acme", "web"), "feat/ops-1", "OPS-1 tidy", &[]);
    assert!(matches(RuleKind::TicketKeyInBranch, "PROJ-123", &branch));
    assert!(matches(RuleKind::TicketKeyInBranch, "proj-*", &branch));
    assert!(matches(RuleKind::TicketKeyInBranch, "PROJ-*", &title));
    assert!(!matches(RuleKind::TicketKeyInBranch, "PROJ-12", &branch));
    assert!(!matches(RuleKind::TicketKeyInBranch, "PROJ-*", &other));
}

#[test]
fn branch_title_label_and_repo_rules_match() {
    let pr = pr(
        1,
        ("acme", "billing-api"),
        "billing/refunds",
        "Add refunds",
        &["billing"],
    );
    assert!(matches(RuleKind::BranchRegex, "^billing/", &pr));
    assert!(!matches(RuleKind::BranchRegex, "^auth/", &pr));
    assert!(matches(RuleKind::TitleRegex, "(?i)refund", &pr));
    assert!(matches(RuleKind::Label, "billing", &pr));
    assert!(!matches(RuleKind::Label, "bill", &pr));
    assert!(matches(RuleKind::Repo, "acme/billing-*", &pr));
    assert!(matches(RuleKind::Repo, "ACME/*", &pr));
    assert!(!matches(RuleKind::Repo, "acme/web", &pr));
}

#[test]
fn validate_rejects_bad_patterns() {
    assert!(validate(RuleKind::BranchRegex, "  ").is_err());
    assert!(validate(RuleKind::BranchRegex, "(").is_err());
    assert!(validate(RuleKind::TitleRegex, &"a".repeat(300)).is_err());
    assert_eq!(validate(RuleKind::Label, " bug ").unwrap(), "bug");
}

#[test]
fn plan_picks_highest_priority_rule_per_project_and_skips_invalid_rules() {
    let (rules, skipped) = RuleSet::new(
        vec![
            rule(1, 1, RuleKind::Label, "billing", 0),
            rule(2, 1, RuleKind::Repo, "acme/*", 5),
            rule(3, 2, RuleKind::Label, "billing", 0),
            rule(4, 2, RuleKind::BranchRegex, "(", 9),
        ],
        DEFAULT_TICKET_KEY_REGEX,
    )
    .unwrap();
    assert_eq!(skipped, 1);
    let plan = rules.plan(&[pr(7, ("acme", "web"), "x", "x", &["billing"])]);
    assert_eq!(
        plan,
        vec![
            RuleLink {
                pr: PrId(7),
                project: ProjectId(1),
                rule_id: 2
            },
            RuleLink {
                pr: PrId(7),
                project: ProjectId(2),
                rule_id: 3
            },
        ]
    );
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    prs: Vec<PullRequest>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("pr-nexus.db")).unwrap();
    let account = store
        .insert_account(Account {
            id: AccountId(0),
            kind: AccountKind::GitHub,
            display_name: "ada".into(),
            host: "github.com".into(),
            external_id: "1".into(),
            slack_workspace: None,
            login: "ada".into(),
            auth_mode: AuthMode::GhCli,
            keychain_key: None,
            created_at: at(),
        })
        .unwrap();
    let repo = store
        .insert_repository(Repository {
            id: 0,
            account_id: account.id,
            owner: "acme".into(),
            name: "web".into(),
            node_id: Some("R_1".into()),
            default_branch: Some("main".into()),
            archived: false,
        })
        .unwrap();
    let mut prs = Vec::new();
    for (index, (head, labels)) in [
        ("billing/a", vec!["billing"]),
        ("auth/b", vec![]),
        ("billing/c", vec![]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut value = pr(0, ("acme", "web"), head, "t", &labels);
        value.remote = ProviderRef {
            account: account.id,
            id: format!("PR_{index}"),
        };
        value.number = index as u64 + 1;
        prs.push(store.upsert_pull_request(repo.id, value).unwrap());
    }
    Fixture {
        _dir: dir,
        store,
        prs,
    }
}

fn sources(store: &Store, project: ProjectId) -> Vec<(i64, LinkSource)> {
    store
        .list_project_links(project)
        .unwrap()
        .into_iter()
        .map(|link| (link.pr.0, link.source))
        .collect::<Vec<_>>()
}

#[test]
fn relink_adds_rule_links_and_removes_stale_ones_but_keeps_manual_and_excluded() {
    let f = fixture();
    let (a, b, c) = (f.prs[0].id, f.prs[1].id, f.prs[2].id);
    let billing = f.store.create_project("Billing".into()).unwrap();
    let saved = f
        .store
        .insert_project_rule(rule(0, billing.id.0, RuleKind::BranchRegex, "^billing/", 0))
        .unwrap();

    let report = relink(&f.store, &f.prs).unwrap();
    assert_eq!((report.added, report.removed), (2, 0));
    let mut got = sources(&f.store, billing.id);
    got.sort_by_key(|(pr, _)| *pr);
    assert_eq!(
        got,
        vec![
            (a.0, LinkSource::Rule(saved.id)),
            (c.0, LinkSource::Rule(saved.id))
        ]
    );

    // A manual link on B survives; excluding C keeps it out on the next run.
    f.store.link_manual(billing.id, vec![b], at()).unwrap();
    f.store.unlink_pr(billing.id, c, false, at()).unwrap();
    let report = relink(&f.store, &f.prs).unwrap();
    assert_eq!((report.added, report.removed), (0, 0));
    let mut got = sources(&f.store, billing.id);
    got.sort_by_key(|(pr, _)| *pr);
    assert_eq!(
        got,
        vec![
            (a.0, LinkSource::Rule(saved.id)),
            (b.0, LinkSource::Manual),
            (c.0, LinkSource::Excluded),
        ]
    );

    // The rule stops matching A: its rule link goes, manual and excluded stay.
    f.store
        .update_project_rule(rule(
            saved.id,
            billing.id.0,
            RuleKind::BranchRegex,
            "^nothing/",
            0,
        ))
        .unwrap();
    let report = relink(&f.store, &f.prs).unwrap();
    assert_eq!((report.added, report.removed), (0, 1));
    let mut got = sources(&f.store, billing.id);
    got.sort_by_key(|(pr, _)| *pr);
    assert_eq!(
        got,
        vec![(b.0, LinkSource::Manual), (c.0, LinkSource::Excluded)]
    );

    // Restoring C lets rules link it again once one matches.
    f.store.restore_pr(billing.id, c).unwrap();
    f.store
        .update_project_rule(rule(
            saved.id,
            billing.id.0,
            RuleKind::BranchRegex,
            "^billing/c",
            0,
        ))
        .unwrap();
    relink(&f.store, &f.prs).unwrap();
    let mut got = sources(&f.store, billing.id);
    got.sort_by_key(|(pr, _)| *pr);
    assert_eq!(
        got,
        vec![(b.0, LinkSource::Manual), (c.0, LinkSource::Rule(saved.id))]
    );
}

#[test]
fn unlink_deletes_plain_manual_links_and_excludes_rule_backed_ones() {
    let f = fixture();
    let (a, b) = (f.prs[0].id, f.prs[1].id);
    let project = f.store.create_project("Auth".into()).unwrap();
    f.store.link_manual(project.id, vec![a, b], at()).unwrap();
    f.store.unlink_pr(project.id, a, false, at()).unwrap();
    f.store.unlink_pr(project.id, b, true, at()).unwrap();
    assert_eq!(
        sources(&f.store, project.id),
        vec![(b.0, LinkSource::Excluded)]
    );
    // A manual link overrides an exclusion.
    f.store.link_manual(project.id, vec![b], at()).unwrap();
    assert_eq!(
        sources(&f.store, project.id),
        vec![(b.0, LinkSource::Manual)]
    );
}

#[test]
fn deleting_a_rule_drops_its_links_and_another_rule_can_take_over() {
    let f = fixture();
    let project = f.store.create_project("Billing".into()).unwrap();
    let high = f
        .store
        .insert_project_rule(rule(0, project.id.0, RuleKind::Label, "billing", 9))
        .unwrap();
    let low = f
        .store
        .insert_project_rule(rule(
            0,
            project.id.0,
            RuleKind::BranchRegex,
            "^billing/a",
            0,
        ))
        .unwrap();
    relink(&f.store, &f.prs).unwrap();
    assert_eq!(
        sources(&f.store, project.id),
        vec![(f.prs[0].id.0, LinkSource::Rule(high.id))]
    );
    f.store.delete_project_rule(high.id).unwrap();
    assert!(sources(&f.store, project.id).is_empty());
    relink(&f.store, &f.prs).unwrap();
    assert_eq!(
        sources(&f.store, project.id),
        vec![(f.prs[0].id.0, LinkSource::Rule(low.id))]
    );
}

#[test]
fn preview_counts_open_visible_matches_only() {
    let f = fixture();
    let mut prs = f.prs.clone();
    prs[2].state = PrState::Merged;
    assert_eq!(
        preview_count(&f.store, &prs, RuleKind::BranchRegex, "^billing/").unwrap(),
        1
    );
    prs[0].hidden = true;
    assert_eq!(
        preview_count(&f.store, &prs, RuleKind::BranchRegex, "^billing/").unwrap(),
        0
    );
    assert!(preview_count(&f.store, &prs, RuleKind::BranchRegex, "(").is_err());
}

#[test]
fn colors_are_validated() {
    let f = fixture();
    let project = f.store.create_project("Ops".into()).unwrap();
    assert_eq!(
        f.store
            .set_project_color(project.id, "#A1B2C3".into())
            .unwrap()
            .color,
        "#a1b2c3"
    );
    assert!(f.store.set_project_color(project.id, "red".into()).is_err());
}
