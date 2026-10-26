use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::domain::{
    Account, AccountId, AccountKind, AuthMode, ChatThreadCacheEntry, ChecksState, CloseReason,
    DiscoveryOutcome, DiscoveryRun, Involvement, LinkSource, MappingLayer, MergeStateStatus,
    Mergeable, OwnershipCacheEntry, PrId, PrProjectLink, PrState, Project, ProjectId, ProjectRule,
    ProviderRef, PullRequest, RepoRef, Repository, RequestSource, RequestStatus, ReviewDecision,
    ReviewMessage, ReviewMessageKind, ReviewMessageOrigin, ReviewMessagePr, ReviewRequest,
    Reviewer, RuleKind, SendAttempt, SendOutcome, SettingEntry, SlackChannel, SyncStateEntry,
    TeamChannelMapping, TeamMembersCacheEntry, TeamRef, ViewState,
};

use super::{Store, db};

fn open_temp() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("pr-nexus.db")).unwrap();
    (dir, store)
}

fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-26T04:40:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn account(
    kind: AccountKind,
    external_id: &str,
    auth: AuthMode,
    workspace: Option<&str>,
) -> Account {
    Account {
        id: AccountId(0),
        kind,
        display_name: external_id.to_owned(),
        host: if kind == AccountKind::GitHub {
            "github.com".to_owned()
        } else {
            format!("{}.slack.com", workspace.unwrap_or("acme"))
        },
        external_id: external_id.to_owned(),
        slack_workspace: workspace.map(str::to_owned),
        login: "alice".to_owned(),
        auth_mode: auth,
        keychain_key: None,
        created_at: at(),
    }
}

fn seed_pr(store: &Store, node_id: &str, state: PrState) -> (Account, Repository, PullRequest) {
    let saved_account = store
        .insert_account(account(
            AccountKind::GitHub,
            &format!("gh-{node_id}"),
            AuthMode::GhCli,
            None,
        ))
        .unwrap();
    let repo = store
        .insert_repository(Repository {
            id: 0,
            account_id: saved_account.id,
            owner: "acme".to_owned(),
            name: "widgets".to_owned(),
            node_id: Some(format!("R_{node_id}")),
            default_branch: Some("main".to_owned()),
            archived: false,
        })
        .unwrap();
    let pr = store
        .upsert_pull_request(repo.id, sample_pr(&saved_account, node_id, state))
        .unwrap();
    (saved_account, repo, pr)
}

fn sample_pr(account: &Account, node_id: &str, state: PrState) -> PullRequest {
    PullRequest {
        id: PrId(0),
        remote: ProviderRef {
            account: account.id,
            id: node_id.to_owned(),
        },
        repo: RepoRef {
            host: "github.com".to_owned(),
            owner: "acme".to_owned(),
            name: "widgets".to_owned(),
        },
        number: 42,
        title: "Fix the widget latch".to_owned(),
        url: "https://github.com/acme/widgets/pull/42".to_owned(),
        author_login: "alice".to_owned(),
        body: "body".to_owned(),
        state,
        is_draft: false,
        head_ref: "fix-latch".to_owned(),
        base_ref: "main".to_owned(),
        head_sha: "abc123".to_owned(),
        labels: vec!["bug".to_owned()],
        review_decision: Some(ReviewDecision::ReviewRequired),
        checks: ChecksState::Success,
        mergeable: Mergeable::Mergeable,
        merge_state: MergeStateStatus::Clean,
        requested_reviewers: vec![
            Reviewer::User {
                login: "bob".to_owned(),
            },
            Reviewer::Team(TeamRef {
                org: "acme".to_owned(),
                slug: "team-a".to_owned(),
            }),
        ],
        unresolved_threads: 1,
        total_threads: 3,
        involvement: Involvement::AUTHOR | Involvement::REVIEW_REQUESTED,
        created_at: at(),
        updated_at: at(),
        last_activity_at: at(),
        synced_at: at(),
        hidden: false,
    }
}

#[test]
fn enums_round_trip_and_unknown_values_fail() {
    let (_dir, store) = open_temp();
    let modes = [
        AuthMode::GhCli,
        AuthMode::Pat,
        AuthMode::OauthDevice,
        AuthMode::SlackMcpOauth,
        AuthMode::SlackUserToken,
    ];
    for (index, mode) in modes.into_iter().enumerate() {
        let kind = if matches!(mode, AuthMode::SlackMcpOauth | AuthMode::SlackUserToken) {
            AccountKind::Slack
        } else {
            AccountKind::GitHub
        };
        let workspace = matches!(mode, AuthMode::SlackMcpOauth | AuthMode::SlackUserToken)
            .then_some(format!("ws-{index}"));
        let saved = store
            .insert_account(account(
                kind,
                &format!("ext-{index}"),
                mode,
                workspace.as_deref(),
            ))
            .unwrap();
        let loaded = store.get_account(saved.id).unwrap().unwrap();
        assert_eq!(loaded.kind, kind);
        assert_eq!(loaded.auth_mode, mode);
        assert_eq!(loaded.created_at, at());
    }

    let states = [PrState::Open, PrState::Closed, PrState::Merged];
    let checks = [
        ChecksState::Success,
        ChecksState::Failure,
        ChecksState::Pending,
        ChecksState::Error,
        ChecksState::Expected,
        ChecksState::None,
    ];
    let mergeable = [
        Mergeable::Mergeable,
        Mergeable::Conflicting,
        Mergeable::Unknown,
    ];
    let merge_states = [
        MergeStateStatus::Clean,
        MergeStateStatus::Behind,
        MergeStateStatus::Blocked,
        MergeStateStatus::Dirty,
        MergeStateStatus::Draft,
        MergeStateStatus::HasHooks,
        MergeStateStatus::Unstable,
        MergeStateStatus::Unknown,
    ];
    let decisions = [
        None,
        Some(ReviewDecision::Approved),
        Some(ReviewDecision::ChangesRequested),
        Some(ReviewDecision::ReviewRequired),
    ];

    let (gh, repo, _) = seed_pr(&store, "NODE0", PrState::Open);
    for (index, state) in states.into_iter().enumerate() {
        let mut pr = sample_pr(&gh, &format!("state-{index}"), state);
        pr.number = 100 + index as u64;
        pr.is_draft = state == PrState::Open;
        pr.hidden = state != PrState::Open;
        pr.checks = checks[index % checks.len()];
        pr.mergeable = mergeable[index % mergeable.len()];
        pr.merge_state = merge_states[index % merge_states.len()];
        pr.review_decision = decisions[index % decisions.len()];
        let saved = store.upsert_pull_request(repo.id, pr.clone()).unwrap();
        let loaded = store.get_pull_request(saved.id).unwrap().unwrap();
        assert_eq!(loaded.state, state);
        assert_eq!(loaded.is_draft, pr.is_draft);
        assert_eq!(loaded.hidden, pr.hidden);
        assert_eq!(loaded.checks, pr.checks);
        assert_eq!(loaded.mergeable, pr.mergeable);
        assert_eq!(loaded.merge_state, pr.merge_state);
        assert_eq!(loaded.review_decision, pr.review_decision);
        assert_eq!(loaded.remote.account, gh.id);
        assert_eq!(loaded.repo.owner, "acme");
        assert_eq!(loaded.labels, vec!["bug".to_owned()]);
        assert_eq!(loaded.requested_reviewers, pr.requested_reviewers);
        assert_eq!(loaded.involvement, pr.involvement);
    }

    for (index, merge_state) in merge_states.into_iter().enumerate() {
        let mut pr = sample_pr(&gh, &format!("merge-{index}"), PrState::Open);
        pr.number = 200 + index as u64;
        pr.merge_state = merge_state;
        pr.checks = checks[index % checks.len()];
        let saved = store.upsert_pull_request(repo.id, pr).unwrap();
        assert_eq!(
            store
                .get_pull_request(saved.id)
                .unwrap()
                .unwrap()
                .merge_state,
            merge_state
        );
    }

    let id = store.list_pull_requests(repo.id).unwrap()[0].id;
    store
        .write(move |conn| {
            conn.execute(
                "UPDATE pull_requests SET state = 'opened' WHERE id = ?1",
                [id.0],
            )
            .map_err(db)?;
            Ok(())
        })
        .unwrap();
    assert!(store.get_pull_request(id).is_err());
}

#[test]
fn cascades_follow_foreign_keys() {
    let (_dir, store) = open_temp();
    let (gh, repo, pr) = seed_pr(&store, "NODE_CASCADE", PrState::Open);
    let slack = store
        .insert_account(account(
            AccountKind::Slack,
            "T000:U000",
            AuthMode::SlackMcpOauth,
            Some("acme"),
        ))
        .unwrap();
    let project = store
        .insert_project(Project {
            id: ProjectId(0),
            name: "Widgets".to_owned(),
            description: "Latch work".to_owned(),
            color: "#336699".to_owned(),
            archived: false,
            sort_order: 1,
        })
        .unwrap();
    let rule = store
        .insert_project_rule(ProjectRule {
            id: 0,
            project: project.id,
            kind: RuleKind::Label,
            pattern: "bug".to_owned(),
            priority: 2,
        })
        .unwrap();
    store
        .upsert_pr_project(PrProjectLink {
            pr: pr.id,
            project: project.id,
            source: LinkSource::Rule(rule.id),
            created_at: at(),
        })
        .unwrap();
    store
        .upsert_ownership_cache(OwnershipCacheEntry {
            pr_id: pr.id,
            head_sha: pr.head_sha.clone(),
            base_ref: "main".to_owned(),
            codeowners_oid: Some("oid".to_owned()),
            decision_json: "{}".to_owned(),
            computed_at: at(),
        })
        .unwrap();
    store
        .upsert_discovery_run(DiscoveryRun {
            pr_id: pr.id,
            slack_workspace: "acme".to_owned(),
            ran_at: at(),
            outcome: DiscoveryOutcome::None,
        })
        .unwrap();
    store
        .insert_review_request(ReviewRequest {
            id: crate::domain::ReviewRequestId(0),
            pr: pr.id,
            root: ProviderRef {
                account: slack.id,
                id: "C0123:1700000000.000100".to_owned(),
            },
            workspace: "acme".to_owned(),
            channel_id: "C0123".to_owned(),
            channel_name: "team-widgets".to_owned(),
            root_ts: "1700000000.000100".to_owned(),
            permalink: None,
            requested_at: at(),
            last_bumped_at: None,
            bump_count: 0,
            status: RequestStatus::Active,
            closed_at: None,
            source: RequestSource::App,
        })
        .unwrap();
    let message = store
        .insert_review_message(ReviewMessage {
            id: 0,
            slack_account_id: slack.id,
            channel_id: "C0123".to_owned(),
            channel_name: "team-widgets".to_owned(),
            ts: "1700000000.000100".to_owned(),
            thread_root_ts: None,
            kind: ReviewMessageKind::Request,
            origin: ReviewMessageOrigin::Posted,
            permalink: None,
            text: "Hello team\n".to_owned(),
            batch_id: None,
            posted_at: at(),
        })
        .unwrap();
    store
        .insert_review_message_pr(ReviewMessagePr {
            message_id: message.id,
            pr_id: pr.id,
            head_sha: Some(pr.head_sha.clone()),
            teams_json: "[]".to_owned(),
        })
        .unwrap();

    store.delete_pull_request(pr.id).unwrap();
    assert!(store.get_pull_request(pr.id).unwrap().is_none());
    assert!(store.list_pr_projects(pr.id).unwrap().is_empty());
    assert!(store.get_ownership_cache(pr.id).unwrap().is_none());
    assert!(
        store
            .get_discovery_run(pr.id, "acme".to_owned())
            .unwrap()
            .is_none()
    );
    assert!(store.list_review_requests(pr.id).unwrap().is_empty());
    assert!(
        store
            .list_review_message_prs(message.id)
            .unwrap()
            .is_empty()
    );
    assert!(store.get_review_message(message.id).unwrap().is_some());
    assert!(store.get_repository(repo.id).unwrap().is_some());

    let err = store.delete_account(slack.id).unwrap_err();
    assert!(err.to_string().contains("store:"), "{err}");
    store.delete_review_message(message.id).unwrap();
    store.delete_account(gh.id).unwrap();
    assert!(store.get_repository(repo.id).unwrap().is_none());

    store.delete_project(project.id).unwrap();
    assert!(store.get_project(project.id).unwrap().is_none());
    assert!(store.list_project_rules(project.id).unwrap().is_empty());
}

#[test]
fn review_request_constraints() {
    let (_dir, store) = open_temp();
    let (_gh, _repo, pr) = seed_pr(&store, "NODE_RR", PrState::Open);
    let base = ReviewRequest {
        id: crate::domain::ReviewRequestId(0),
        pr: pr.id,
        root: ProviderRef {
            account: AccountId(0),
            id: "C100:1.000100".to_owned(),
        },
        workspace: "acme".to_owned(),
        channel_id: "C100".to_owned(),
        channel_name: "team-widgets".to_owned(),
        root_ts: "1.000100".to_owned(),
        permalink: Some("https://acme.slack.com/archives/C100/p1000100".to_owned()),
        requested_at: at(),
        last_bumped_at: None,
        bump_count: 0,
        status: RequestStatus::Active,
        closed_at: None,
        source: RequestSource::App,
    };
    let first = store.insert_review_request(base.clone()).unwrap();
    assert_eq!(first.status, RequestStatus::Active);
    assert_eq!(first.root.id, "C100:1.000100");
    assert!(
        store
            .insert_review_request(ReviewRequest {
                root_ts: "1.000200".to_owned(),
                ..base.clone()
            })
            .is_err()
    );

    let mut other_channel = base.clone();
    other_channel.channel_id = "C200".to_owned();
    other_channel.root_ts = "2.000100".to_owned();
    assert!(store.insert_review_request(other_channel).is_ok());

    let pr_id = pr.id.0;
    let closed_without_reason = store.write(move |conn| {
        conn.execute(
            "INSERT INTO review_requests (
                pr_id, slack_workspace, channel_id, channel_name, root_ts,
                requested_at, status, source
             ) VALUES (?1, 'acme', 'C300', 'team-widgets', '3.000100', ?2, 'closed', 'app')",
            params![pr_id, "2026-10-26T04:40:00.000Z"],
        )
        .map_err(db)?;
        Ok(())
    });
    assert!(closed_without_reason.is_err());

    let active_with_reason = store.write(move |conn| {
        conn.execute(
            "INSERT INTO review_requests (
                pr_id, slack_workspace, channel_id, channel_name, root_ts,
                requested_at, status, close_reason, source
             ) VALUES (?1, 'acme', 'C300', 'team-widgets', '3.000100', ?2, 'active', 'dismissed', 'app')",
            params![pr_id, "2026-10-26T04:40:00.000Z"],
        )
        .map_err(db)?;
        Ok(())
    });
    assert!(active_with_reason.is_err());

    let closed = store
        .insert_review_request(ReviewRequest {
            channel_id: "C300".to_owned(),
            root_ts: "3.000100".to_owned(),
            status: RequestStatus::Closed(CloseReason::Dismissed),
            closed_at: Some(at()),
            source: RequestSource::DiscoveredViaSearch,
            ..base.clone()
        })
        .unwrap();
    assert_eq!(closed.status, RequestStatus::Closed(CloseReason::Dismissed));
    let again = store
        .insert_review_request(ReviewRequest {
            channel_id: "C300".to_owned(),
            root_ts: "3.000200".to_owned(),
            status: RequestStatus::Closed(CloseReason::Superseded),
            closed_at: Some(at()),
            ..base
        })
        .unwrap();
    assert_eq!(again.status, RequestStatus::Closed(CloseReason::Superseded));

    let bumped = store
        .close_review_request(first.id, CloseReason::PrMerged, at())
        .unwrap();
    assert_eq!(bumped.status, RequestStatus::Closed(CloseReason::PrMerged));
    assert_eq!(bumped.closed_at, Some(at()));
}

#[test]
fn view_state_round_trips_json_bytes() {
    let (_dir, store) = open_temp();
    let json = r#"{"grouping":"project","columns":["title","repo"],"filters":[]}"#;
    store
        .put_view_state(ViewState {
            view_key: "mine".to_owned(),
            state_json: json.to_owned(),
            updated_at: at(),
        })
        .unwrap();
    let loaded = store.get_view_state("mine".to_owned()).unwrap().unwrap();
    assert_eq!(loaded.state_json, json);
    assert_eq!(loaded.updated_at, at());

    let replaced = r#"{"grouping":"repo"}"#;
    store
        .put_view_state(ViewState {
            view_key: "mine".to_owned(),
            state_json: replaced.to_owned(),
            updated_at: at(),
        })
        .unwrap();
    assert_eq!(
        store
            .get_view_state("mine".to_owned())
            .unwrap()
            .unwrap()
            .state_json,
        replaced
    );
    assert!(
        store
            .get_view_state("missing".to_owned())
            .unwrap()
            .is_none()
    );
}

#[test]
fn every_remaining_table_round_trips() {
    let (_dir, store) = open_temp();
    assert_eq!(store.mapping_count().unwrap(), 0);
    let (gh, _repo, pr) = seed_pr(&store, "NODE_REST", PrState::Open);
    let slack = store
        .insert_account(account(
            AccountKind::Slack,
            "T001:U001",
            AuthMode::SlackUserToken,
            Some("acme"),
        ))
        .unwrap();

    let mapping = store
        .upsert_mapping(TeamChannelMapping {
            id: 0,
            layer: MappingLayer::User,
            github_org: "acme".to_owned(),
            team_slug: "team-a".to_owned(),
            team_name: "Team A".to_owned(),
            slack_workspace: "acme".to_owned(),
            slack_channel_id: "C012345".to_owned(),
            slack_channel_name: "team-widgets".to_owned(),
            disabled: false,
            note: "picked".to_owned(),
            updated_at: at(),
        })
        .unwrap();
    assert_eq!(store.list_mappings().unwrap(), vec![mapping]);

    let channel = SlackChannel {
        slack_workspace: "acme".to_owned(),
        channel_id: "C012345".to_owned(),
        name: "team-widgets".to_owned(),
        is_private: true,
        is_archived: false,
        is_member: true,
        fetched_at: at(),
    };
    store.upsert_slack_channel(channel.clone()).unwrap();
    assert_eq!(
        store
            .get_slack_channel("acme".to_owned(), "C012345".to_owned())
            .unwrap()
            .unwrap(),
        channel
    );

    store
        .upsert_team_members(TeamMembersCacheEntry {
            github_org: "acme".to_owned(),
            team_slug: "team-a".to_owned(),
            members_json: r#"["alice"]"#.to_owned(),
            complete: true,
            fetched_at: at(),
        })
        .unwrap();
    assert!(
        store
            .get_team_members("acme".to_owned(), "team-a".to_owned())
            .unwrap()
            .unwrap()
            .complete
    );

    let thread = ChatThreadCacheEntry {
        slack_account_id: slack.id,
        channel_id: "C012345".to_owned(),
        thread_root_ts: "1.000100".to_owned(),
        messages_json: "[]".to_owned(),
        fetched_at: at(),
    };
    store.upsert_chat_thread(thread.clone()).unwrap();
    assert_eq!(
        store
            .get_chat_thread(slack.id, "C012345".to_owned(), "1.000100".to_owned())
            .unwrap()
            .unwrap(),
        thread
    );

    store
        .put_sync_state(SyncStateEntry {
            key: "github.watermark".to_owned(),
            value_json: r#"{"cursor":null}"#.to_owned(),
            updated_at: at(),
        })
        .unwrap();
    assert_eq!(
        store
            .get_sync_state("github.watermark".to_owned())
            .unwrap()
            .unwrap()
            .value_json,
        r#"{"cursor":null}"#
    );

    store
        .put_setting(SettingEntry {
            key: "github.tracked_orgs".to_owned(),
            value_json: "[]".to_owned(),
        })
        .unwrap();
    assert_eq!(
        store
            .get_setting("github.tracked_orgs".to_owned())
            .unwrap()
            .unwrap()
            .value_json,
        "[]"
    );

    let attempt = store
        .insert_send_attempt(SendAttempt {
            id: 0,
            batch_id: "batch-1".to_owned(),
            pr_id: Some(pr.id),
            channel_id: Some("C012345".to_owned()),
            origin: "ui".to_owned(),
            outcome: SendOutcome::Skipped,
            detail: "blocked".to_owned(),
            at: at(),
        })
        .unwrap();
    assert_eq!(attempt.outcome, SendOutcome::Skipped);
    assert_eq!(attempt.pr_id, Some(pr.id));
    let _ = gh;
}

#[test]
fn pull_request_body_is_capped_at_8kb() {
    let (_dir, store) = open_temp();
    let (gh, repo, _) = seed_pr(&store, "NODE_BODY", PrState::Open);
    let mut pr = sample_pr(&gh, "NODE_BODY_LONG", PrState::Open);
    pr.number = 7;
    pr.body = "é".repeat(9_000);
    let saved = store.upsert_pull_request(repo.id, pr).unwrap();
    assert!(saved.body.len() <= 8 * 1024);
    assert!(saved.body.is_char_boundary(saved.body.len()));
}
