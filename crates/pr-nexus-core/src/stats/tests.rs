use chrono::{DateTime, Duration, Utc};

use super::*;

fn t(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn now() -> DateTime<Utc> {
    t("2026-03-31T12:00:00Z")
}

fn review(author: &str, at: &str) -> StatsReview {
    StatsReview {
        author: author.into(),
        is_bot: false,
        at: t(at),
    }
}

fn user(login: &str, at: &str) -> StatsRequest {
    StatsRequest {
        target: RequestTarget::User(login.into()),
        at: t(at),
    }
}

fn pr(repo: &str, number: u64, created: &str) -> StatsPr {
    StatsPr {
        repo: repo.into(),
        number,
        author: "ada".into(),
        created_at: t(created),
        is_draft: false,
        state: StatsPrState::Open,
        merged_at: None,
        ready_events: vec![],
        requests: vec![],
        reviews: vec![],
    }
}

/// Five PRs in acme/api and acme-labs/web over the last month.
fn fixture() -> Vec<StatsPr> {
    let mut a = pr("acme/api", 1, "2026-03-20T09:00:00Z");
    a.state = StatsPrState::Merged;
    a.merged_at = Some(t("2026-03-21T09:00:00Z"));
    a.requests = vec![
        user("grace", "2026-03-20T09:00:00Z"),
        user("linus", "2026-03-20T09:05:00Z"),
    ];
    a.reviews = vec![
        review("ada", "2026-03-20T09:30:00Z"),
        StatsReview {
            author: "ci-helper[bot]".into(),
            is_bot: true,
            at: t("2026-03-20T09:01:00Z"),
        },
        review("grace", "2026-03-20T11:00:00Z"),
        review("grace", "2026-03-20T15:00:00Z"),
    ];

    // Draft first, ready a day later, reviewed 4 h after ready.
    let mut b = pr("acme/api", 2, "2026-03-22T09:00:00Z");
    b.ready_events = vec![t("2026-03-23T09:00:00Z")];
    b.requests = vec![
        user("grace", "2026-03-23T09:00:00Z"),
        StatsRequest {
            target: RequestTarget::Team("acme/platform".into()),
            at: t("2026-03-23T09:00:00Z"),
        },
    ];
    b.reviews = vec![review("linus", "2026-03-23T13:00:00Z")];

    // Open, ready, nobody reviewed yet: waiting.
    let mut c = pr("acme-labs/web", 7, "2026-03-29T09:00:00Z");
    c.requests = vec![
        user("grace", "2026-03-29T09:00:00Z"),
        user("grace", "2026-03-30T09:00:00Z"),
    ];

    // Still a draft: no review clock.
    let mut d = pr("acme-labs/web", 8, "2026-03-30T09:00:00Z");
    d.is_draft = true;

    // Too old for a 30-day window.
    let mut e = pr("acme/api", 3, "2026-01-02T09:00:00Z");
    e.reviews = vec![review("grace", "2026-01-02T10:00:00Z")];

    // Someone else's PR never counts.
    let mut f = pr("acme/api", 4, "2026-03-25T09:00:00Z");
    f.author = "grace".into();

    vec![a, b, c, d, e, f]
}

fn slack() -> Vec<SlackRequestFact> {
    vec![
        SlackRequestFact {
            repo: "acme/api".into(),
            number: 2,
            channel: "team-reviews".into(),
            requested_at: t("2026-03-23T10:00:00Z"),
            bump_count: 1,
        },
        SlackRequestFact {
            repo: "acme-labs/web".into(),
            number: 7,
            channel: "team-reviews".into(),
            requested_at: t("2026-03-29T10:00:00Z"),
            bump_count: 2,
        },
    ]
}

#[test]
fn counts_and_first_review_over_thirty_days() {
    let stats = compute(
        &fixture(),
        &slack(),
        "ada",
        30,
        now(),
        &StatsFilter::default(),
    );
    assert_eq!(
        (stats.opened, stats.merged, stats.waiting, stats.reminders),
        (4, 1, 1, 3)
    );
    // #1: 2 h (own review and bot ignored), #2: 4 h after ready.
    assert_eq!(stats.first_review.count, 2);
    assert_eq!(stats.first_review.median_secs, Some(3 * 3600));
    assert_eq!(stats.first_review.average_secs, Some(3 * 3600));
    // #2: Slack at 10:00, review at 13:00. #7 has no review yet.
    assert_eq!(stats.slack_to_first_review.count, 1);
    assert_eq!(stats.slack_to_first_review.median_secs, Some(3 * 3600));
    assert_eq!(
        stats.repos,
        vec!["acme-labs/web".to_string(), "acme/api".to_string()]
    );
}

#[test]
fn ranks_reviewers_by_requests_with_response_rate() {
    let stats = compute(
        &fixture(),
        &slack(),
        "ada",
        30,
        now(),
        &StatsFilter::default(),
    );
    let grace = &stats.reviewers[0];
    assert_eq!(grace.target, RequestTarget::User("grace".into()));
    // Asked on #1, #2, #7 (re-request on #7 counts once); answered only #1, in 2 h.
    assert_eq!((grace.requests, grace.responded), (3, 1));
    assert!((grace.response_rate - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(grace.median_response_secs, Some(2 * 3600));

    let channel = stats
        .reviewers
        .iter()
        .find(|r| r.target == RequestTarget::SlackChannel("team-reviews".into()))
        .unwrap();
    assert_eq!((channel.requests, channel.responded), (2, 1));

    let team = stats
        .reviewers
        .iter()
        .find(|r| r.target == RequestTarget::Team("acme/platform".into()))
        .unwrap();
    assert_eq!(
        (team.requests, team.responded, team.median_response_secs),
        (1, 1, Some(4 * 3600))
    );

    let linus = stats
        .reviewers
        .iter()
        .find(|r| r.target == RequestTarget::User("linus".into()))
        .unwrap();
    assert_eq!(
        (linus.requests, linus.responded),
        (1, 0),
        "linus reviewed #2 without a request there"
    );
}

#[test]
fn filters_by_repo_and_org() {
    let by_repo = StatsFilter {
        repo: Some("acme-labs/web".into()),
        org: None,
    };
    let stats = compute(&fixture(), &slack(), "ada", 30, now(), &by_repo);
    assert_eq!((stats.opened, stats.waiting, stats.reminders), (2, 1, 2));
    assert_eq!(stats.first_review.count, 0);
    assert_eq!(stats.repos.len(), 2, "the menu still lists every repo");

    let by_org = StatsFilter {
        repo: None,
        org: Some("ACME".into()),
    };
    let stats = compute(&fixture(), &slack(), "ada", 30, now(), &by_org);
    assert_eq!(stats.opened, 2);
}

#[test]
fn period_bounds_the_set() {
    let week = compute(&fixture(), &[], "ada", 7, now(), &StatsFilter::default());
    assert_eq!(week.opened, 2);
    let quarter = compute(&fixture(), &[], "ada", 90, now(), &StatsFilter::default());
    assert_eq!(quarter.opened, 5);
    assert_eq!(quarter.first_review.count, 3);
    assert_eq!(quarter.since, now() - Duration::days(90));
}

#[test]
fn bots_and_self_requests_never_rank() {
    let mut p = pr("acme/api", 9, "2026-03-30T09:00:00Z");
    p.requests = vec![
        user("ada", "2026-03-30T09:00:00Z"),
        user("dependabot[bot]", "2026-03-30T09:00:00Z"),
    ];
    let stats = compute(&[p], &[], "ada", 30, now(), &StatsFilter::default());
    assert!(stats.reviewers.is_empty());
}

#[test]
fn a_review_on_a_draft_counts_as_zero_wait() {
    let mut p = pr("acme/api", 10, "2026-03-28T09:00:00Z");
    p.ready_events = vec![t("2026-03-29T09:00:00Z"), t("2026-03-28T12:00:00Z")];
    p.reviews = vec![review("grace", "2026-03-28T10:00:00Z")];
    let stats = compute(&[p], &[], "ada", 30, now(), &StatsFilter::default());
    assert_eq!(stats.first_review.median_secs, Some(0));
}

#[test]
fn median_of_even_count_is_the_midpoint() {
    let summary = DurationSummary::of(vec![10, 40, 20, 30]);
    assert_eq!(
        (summary.count, summary.median_secs, summary.average_secs),
        (4, Some(25), Some(25))
    );
    assert_eq!(DurationSummary::of(vec![]), DurationSummary::default());
}

#[test]
fn queries_are_scoped_and_dated() {
    let scope = TrackedScope {
        orgs: vec!["acme".into(), "acme-labs".into()],
        include_personal: true,
        viewer_login: "ada".into(),
    };
    let queries = stats_queries(&scope, t("2026-03-01T08:00:00Z"));
    assert_eq!(
        queries,
        vec!["is:pr author:@me archived:false sort:created-desc created:>=2026-03-01 org:acme org:acme-labs user:ada".to_string()]
    );
}
