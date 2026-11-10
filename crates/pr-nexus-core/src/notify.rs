//! Differences between two sync snapshots, and the notices they produce.
//!
//! The shell turns [`Notice`] values into operating-system notifications.
//! This module never talks to the operating system: the same inputs always
//! produce the same events, and the first snapshot of a process is silent.

use std::collections::HashMap;

use crate::domain::{ChecksState, Involvement, PrState, PullRequest, ReviewDecision};

/// One open pull request, reduced to the fields a notification can change on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedPr {
    pub id: i64,
    pub number: i64,
    pub title: String,
    pub repo: String,
    pub url: String,
    pub open: bool,
    pub author: bool,
    pub review_requested: bool,
    pub checks_failing: bool,
    pub decision: Option<ReviewDecision>,
    pub total_threads: i64,
}

impl WatchedPr {
    pub fn from_pull_request(pr: &PullRequest) -> Self {
        Self {
            id: pr.id.0,
            number: i64::try_from(pr.number).unwrap_or(i64::MAX),
            title: pr.title.clone(),
            repo: format!("{}/{}", pr.repo.owner, pr.repo.name),
            url: pr.url.clone(),
            open: pr.state == PrState::Open,
            author: pr.involvement.contains(Involvement::AUTHOR),
            review_requested: pr.involvement.contains(Involvement::REVIEW_REQUESTED),
            checks_failing: matches!(pr.checks, ChecksState::Failure | ChecksState::Error),
            decision: pr.review_decision,
            total_threads: i64::from(pr.total_threads),
        }
    }
}

/// Why a pull request is worth a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NotifyKind {
    ReviewRequested,
    CiRed,
    Approved,
    ChangesRequested,
    Comment,
}

/// One transition on one pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyEvent {
    pub kind: NotifyKind,
    pub pr_id: i64,
    pub number: i64,
    pub title: String,
    pub repo: String,
    pub url: String,
    /// List the click should open: `mine` or `to_review`.
    pub view: &'static str,
}

/// One operating-system notification, possibly covering several pull requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: NotifyKind,
    pub title: String,
    pub body: String,
    /// Set when the notice is about exactly one pull request.
    pub pr_id: Option<i64>,
    pub view: &'static str,
}

/// Which events Settings allows. The master switch wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyToggles {
    pub enabled: bool,
    pub review_requested: bool,
    pub ci_red: bool,
    pub approved: bool,
    pub changes_requested: bool,
    pub comment: bool,
}

impl Default for NotifyToggles {
    fn default() -> Self {
        Self {
            enabled: true,
            review_requested: true,
            ci_red: true,
            approved: true,
            changes_requested: true,
            comment: false,
        }
    }
}

impl NotifyToggles {
    pub fn allows(self, kind: NotifyKind) -> bool {
        if !self.enabled {
            return false;
        }
        match kind {
            NotifyKind::ReviewRequested => self.review_requested,
            NotifyKind::CiRed => self.ci_red,
            NotifyKind::Approved => self.approved,
            NotifyKind::ChangesRequested => self.changes_requested,
            NotifyKind::Comment => self.comment,
        }
    }
}

/// How long a repeated event on the same pull request stays quiet.
pub const DEBOUNCE_MS: i64 = 2 * 60 * 1000;

/// Remembers the last snapshot and when each event was last emitted.
#[derive(Debug, Default)]
pub struct Watcher {
    seen: Option<HashMap<i64, WatchedPr>>,
    /// Review requests stay quiet until a snapshot that included them.
    review_baselined: bool,
    sent_at: HashMap<(i64, NotifyKind), i64>,
}

impl Watcher {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// True after the first snapshot, which itself produced no events.
    pub fn ready(&self) -> bool {
        self.seen.is_some()
    }

    /// Compare `after` with the previous snapshot.
    ///
    /// The first call stores `after` and returns nothing, including when the
    /// database already held every pull request. `include_reviews` is false
    /// for a sync that did not read the review bucket: review-request events
    /// from that snapshot are dropped, and the first snapshot that does
    /// include reviews is also quiet for that kind.
    pub fn observe(
        &mut self,
        after: &[WatchedPr],
        include_reviews: bool,
        now_ms: i64,
    ) -> Vec<NotifyEvent> {
        let Some(before) = self.seen.clone() else {
            self.seen = Some(index(after));
            self.review_baselined = include_reviews;
            return Vec::new();
        };
        let mut events = diff_snapshots(&before, after);
        if !self.review_baselined {
            events.retain(|event| event.kind != NotifyKind::ReviewRequested);
            if include_reviews {
                self.review_baselined = true;
            }
        }
        events.retain(|event| self.admit(event, now_ms));
        self.seen = Some(index(after));
        events
    }

    fn admit(&mut self, event: &NotifyEvent, now_ms: i64) -> bool {
        let key = (event.pr_id, event.kind);
        if let Some(sent) = self.sent_at.get(&key)
            && now_ms.saturating_sub(*sent) < DEBOUNCE_MS
        {
            return false;
        }
        self.sent_at.insert(key, now_ms);
        true
    }
}

fn index(rows: &[WatchedPr]) -> HashMap<i64, WatchedPr> {
    rows.iter().map(|row| (row.id, row.clone())).collect()
}

/// Transitions from `before` to `after`. Closed rows produce nothing.
pub fn diff_snapshots(before: &HashMap<i64, WatchedPr>, after: &[WatchedPr]) -> Vec<NotifyEvent> {
    let mut events = Vec::new();
    for row in after.iter().filter(|row| row.open) {
        match before.get(&row.id) {
            None => {
                if row.review_requested {
                    events.push(event(NotifyKind::ReviewRequested, row));
                }
            }
            Some(previous) => {
                if !previous.review_requested && row.review_requested {
                    events.push(event(NotifyKind::ReviewRequested, row));
                }
                if row.author && !previous.checks_failing && row.checks_failing {
                    events.push(event(NotifyKind::CiRed, row));
                }
                if row.author
                    && previous.decision != Some(ReviewDecision::Approved)
                    && row.decision == Some(ReviewDecision::Approved)
                {
                    events.push(event(NotifyKind::Approved, row));
                }
                if row.author
                    && previous.decision != Some(ReviewDecision::ChangesRequested)
                    && row.decision == Some(ReviewDecision::ChangesRequested)
                {
                    events.push(event(NotifyKind::ChangesRequested, row));
                }
                if (row.author || row.review_requested)
                    && row.total_threads > previous.total_threads
                {
                    events.push(event(NotifyKind::Comment, row));
                }
            }
        }
    }
    events.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then(left.repo.cmp(&right.repo))
            .then(left.number.cmp(&right.number))
    });
    events
}

fn event(kind: NotifyKind, row: &WatchedPr) -> NotifyEvent {
    let view = match kind {
        NotifyKind::ReviewRequested => "to_review",
        NotifyKind::CiRed | NotifyKind::Approved | NotifyKind::ChangesRequested => "mine",
        NotifyKind::Comment if row.author => "mine",
        NotifyKind::Comment => "to_review",
    };
    NotifyEvent {
        kind,
        pr_id: row.id,
        number: row.number,
        title: row.title.clone(),
        repo: row.repo.clone(),
        url: row.url.clone(),
        view,
    }
}

/// Group events of one kind into a single notice.
pub fn aggregate(events: &[NotifyEvent]) -> Vec<Notice> {
    let mut groups: Vec<(NotifyKind, Vec<&NotifyEvent>)> = Vec::new();
    for event in events {
        if let Some(group) = groups.iter_mut().find(|(kind, _)| *kind == event.kind) {
            group.1.push(event);
        } else {
            groups.push((event.kind, vec![event]));
        }
    }
    groups
        .into_iter()
        .map(|(kind, rows)| notice(kind, &rows))
        .collect()
}

fn notice(kind: NotifyKind, rows: &[&NotifyEvent]) -> Notice {
    let count = rows.len();
    let title = match (kind, count) {
        (NotifyKind::ReviewRequested, 1) => "Review requested".to_owned(),
        (NotifyKind::ReviewRequested, n) => format!("{n} review requests"),
        (NotifyKind::CiRed, 1) => "CI failed".to_owned(),
        (NotifyKind::CiRed, n) => format!("CI failed on {n} pull requests"),
        (NotifyKind::Approved, 1) => "Pull request approved".to_owned(),
        (NotifyKind::Approved, n) => format!("{n} pull requests approved"),
        (NotifyKind::ChangesRequested, 1) => "Changes requested".to_owned(),
        (NotifyKind::ChangesRequested, n) => format!("Changes requested on {n} pull requests"),
        (NotifyKind::Comment, 1) => "New review thread".to_owned(),
        (NotifyKind::Comment, n) => format!("New review threads on {n} pull requests"),
    };
    let mut lines: Vec<String> = rows
        .iter()
        .take(3)
        .map(|row| format!("{} #{} {}", row.repo, row.number, row.title))
        .collect();
    if count > 3 {
        lines.push(format!("and {} more", count - 3));
    }
    Notice {
        kind,
        title,
        body: lines.join("\n"),
        pr_id: (count == 1).then(|| rows[0].pr_id),
        view: rows[0].view,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64) -> WatchedPr {
        WatchedPr {
            id,
            number: 10 + id,
            title: format!("Change {id}"),
            repo: "acme/widgets".to_owned(),
            url: format!("https://github.com/acme/widgets/pull/{id}"),
            open: true,
            author: true,
            review_requested: false,
            checks_failing: false,
            decision: None,
            total_threads: 0,
        }
    }

    #[test]
    fn the_first_snapshot_is_silent_even_when_it_already_needs_attention() {
        let mut watcher = Watcher::default();
        let mut existing = row(1);
        existing.review_requested = true;
        existing.checks_failing = true;
        existing.decision = Some(ReviewDecision::ChangesRequested);
        existing.total_threads = 4;
        assert!(watcher.observe(&[existing], true, 0).is_empty());
        assert!(watcher.ready());
    }

    #[test]
    fn a_later_snapshot_reports_only_transitions() {
        let mut watcher = Watcher::default();
        let start = row(1);
        watcher.observe(std::slice::from_ref(&start), true, 0);

        let mut next = start;
        next.checks_failing = true;
        next.decision = Some(ReviewDecision::Approved);
        next.total_threads = 2;
        next.review_requested = true;
        let events = watcher.observe(&[next], true, 1_000);
        let kinds: Vec<_> = events.iter().map(|event| event.kind).collect();
        assert_eq!(
            kinds,
            vec![
                NotifyKind::ReviewRequested,
                NotifyKind::CiRed,
                NotifyKind::Approved,
                NotifyKind::Comment,
            ]
        );
    }

    #[test]
    fn a_new_pull_request_notifies_only_a_review_request() {
        let mut watcher = Watcher::default();
        watcher.observe(&[row(1)], true, 0);
        let mut fresh = row(2);
        fresh.author = false;
        fresh.review_requested = true;
        fresh.checks_failing = true;
        fresh.decision = Some(ReviewDecision::Approved);
        let events = watcher.observe(&[row(1), fresh], true, 1_000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, NotifyKind::ReviewRequested);
        assert_eq!(events[0].view, "to_review");
    }

    #[test]
    fn review_requests_wait_for_the_first_review_snapshot() {
        let mut watcher = Watcher::default();
        let mut mine = row(1);
        mine.checks_failing = false;
        watcher.observe(&[mine.clone()], false, 0);

        let mut asked = mine.clone();
        asked.review_requested = true;
        let mut failing = mine.clone();
        failing.checks_failing = true;
        let first_reviews = watcher.observe(&[asked, failing.clone()], true, 1_000);
        assert_eq!(
            first_reviews
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![NotifyKind::CiRed]
        );

        let mut another = row(2);
        another.author = false;
        another.review_requested = true;
        let later = watcher.observe(&[failing, another], true, 2_000);
        assert_eq!(later[0].kind, NotifyKind::ReviewRequested);
    }

    #[test]
    fn closed_and_unchanged_rows_stay_quiet() {
        let mut watcher = Watcher::default();
        let open = row(1);
        watcher.observe(std::slice::from_ref(&open), true, 0);
        let mut closed = open.clone();
        closed.open = false;
        closed.checks_failing = true;
        assert!(watcher.observe(&[closed.clone()], true, 1_000).is_empty());
        closed.open = true;
        assert!(watcher.observe(&[closed], true, 2_000).is_empty());
    }

    #[test]
    fn the_same_event_is_quiet_for_two_minutes() {
        let mut watcher = Watcher::default();
        watcher.observe(&[row(1)], true, 0);
        let mut failing = row(1);
        failing.checks_failing = true;
        assert_eq!(watcher.observe(&[failing.clone()], true, 1_000).len(), 1);
        watcher.observe(&[row(1)], true, 1_000);
        assert!(
            watcher
                .observe(&[failing.clone()], true, 1_000 + DEBOUNCE_MS - 1)
                .is_empty()
        );
        // The quiet flap was still recorded. A new one after the window notifies.
        watcher.observe(&[row(1)], true, 1_000 + DEBOUNCE_MS);
        assert_eq!(
            watcher.observe(&[failing], true, 1_000 + DEBOUNCE_MS).len(),
            1
        );
    }

    #[test]
    fn comments_on_someone_elses_unrelated_pr_are_ignored() {
        let mut watcher = Watcher::default();
        let mut other = row(1);
        other.author = false;
        watcher.observe(&[other.clone()], true, 0);
        other.total_threads = 3;
        assert!(watcher.observe(&[other], true, 1_000).is_empty());
    }

    #[test]
    fn toggles_drop_a_kind_and_the_master_switch_drops_all() {
        let toggles = NotifyToggles::default();
        assert!(toggles.allows(NotifyKind::CiRed));
        assert!(!toggles.allows(NotifyKind::Comment));
        let off = NotifyToggles {
            enabled: false,
            ..toggles
        };
        assert!(!off.allows(NotifyKind::CiRed));
        let comments = NotifyToggles {
            comment: true,
            ..toggles
        };
        assert!(comments.allows(NotifyKind::Comment));
    }

    #[test]
    fn several_events_of_one_kind_become_one_notice() {
        let events = [
            NotifyEvent {
                kind: NotifyKind::ReviewRequested,
                pr_id: 1,
                number: 4,
                title: "One".to_owned(),
                repo: "acme/widgets".to_owned(),
                url: "https://github.com/acme/widgets/pull/4".to_owned(),
                view: "to_review",
            },
            NotifyEvent {
                kind: NotifyKind::CiRed,
                pr_id: 9,
                number: 8,
                title: "Red".to_owned(),
                repo: "acme/widgets".to_owned(),
                url: "https://github.com/acme/widgets/pull/8".to_owned(),
                view: "mine",
            },
            NotifyEvent {
                kind: NotifyKind::ReviewRequested,
                pr_id: 2,
                number: 5,
                title: "Two".to_owned(),
                repo: "acme/widgets".to_owned(),
                url: "https://github.com/acme/widgets/pull/5".to_owned(),
                view: "to_review",
            },
            NotifyEvent {
                kind: NotifyKind::ReviewRequested,
                pr_id: 3,
                number: 6,
                title: "Three".to_owned(),
                repo: "acme/widgets".to_owned(),
                url: "https://github.com/acme/widgets/pull/6".to_owned(),
                view: "to_review",
            },
            NotifyEvent {
                kind: NotifyKind::ReviewRequested,
                pr_id: 4,
                number: 7,
                title: "Four".to_owned(),
                repo: "acme/widgets".to_owned(),
                url: "https://github.com/acme/widgets/pull/7".to_owned(),
                view: "to_review",
            },
        ];
        let notices = aggregate(&events);
        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0].title, "4 review requests");
        assert_eq!(notices[0].pr_id, None);
        assert!(notices[0].body.contains("acme/widgets #4 One"));
        assert!(notices[0].body.ends_with("and 1 more"));
        assert_eq!(notices[1].title, "CI failed");
        assert_eq!(notices[1].pr_id, Some(9));
        assert_eq!(notices[1].view, "mine");
    }

    #[test]
    fn a_pull_request_row_keeps_the_fields_a_notice_can_change() {
        use crate::domain::{
            AccountId, MergeStateStatus, Mergeable, PrId, ProviderRef, PullRequest, RepoRef,
        };
        use chrono::TimeZone;

        let at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let pr = PullRequest {
            id: PrId(7),
            remote: ProviderRef {
                account: AccountId(1),
                id: "PR_7".into(),
            },
            repo: RepoRef {
                host: "github.com".into(),
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 12,
            title: "Tighten the tray".into(),
            url: "https://github.com/acme/widgets/pull/12".into(),
            author_login: "octocat".into(),
            body: String::new(),
            state: PrState::Open,
            is_draft: false,
            head_ref: "tray".into(),
            base_ref: "main".into(),
            head_sha: "abc".into(),
            labels: Vec::new(),
            review_decision: Some(ReviewDecision::ChangesRequested),
            checks: ChecksState::Error,
            mergeable: Mergeable::Mergeable,
            merge_state: MergeStateStatus::Clean,
            requested_reviewers: Vec::new(),
            unresolved_threads: 1,
            total_threads: 2,
            involvement: Involvement::AUTHOR | Involvement::REVIEW_REQUESTED,
            created_at: at,
            updated_at: at,
            last_activity_at: at,
            synced_at: at,
            hidden: false,
        };
        let watched = WatchedPr::from_pull_request(&pr);
        assert_eq!(watched.repo, "acme/widgets");
        assert!(watched.open && watched.author && watched.review_requested);
        assert!(watched.checks_failing);
        assert_eq!(watched.decision, Some(ReviewDecision::ChangesRequested));
        assert_eq!(watched.total_threads, 2);
    }

    #[test]
    fn reset_makes_the_next_snapshot_a_new_baseline() {
        let mut watcher = Watcher::default();
        watcher.observe(&[row(1)], true, 0);
        watcher.reset();
        let mut failing = row(1);
        failing.checks_failing = true;
        assert!(watcher.observe(&[failing], true, 5_000).is_empty());
    }
}
