//! When the signed-in user was asked to review.
//!
//! Sync stores the latest slice of GitHub's review timeline. The list paints
//! the request age from that cache. A missing request falls back to when the
//! pull request was marked ready, then to when it was opened.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::{Involvement, RequestedReviewer, Reviewer, TeamRef};

/// One status-query slice of review requests, submitted reviews, and the
/// latest ready-for-review time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReviewClock {
    #[serde(default)]
    pub requests: Vec<ReviewRequestEvent>,
    #[serde(default)]
    pub reviews: Vec<SubmittedReview>,
    #[serde(default)]
    pub ready_at: Option<DateTime<Utc>>,
    /// The slice was full, so an older request may be missing.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRequestEvent {
    pub at: DateTime<Utc>,
    pub target: RequestTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestTarget {
    User { login: String },
    Team { org: String, slug: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmittedReview {
    pub at: DateTime<Utc>,
    pub login: String,
}

/// Where the painted timestamp came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StampBasis {
    Request,
    Ready,
    Opened,
}

impl StampBasis {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Ready => "ready",
            Self::Opened => "opened",
        }
    }
}

/// The instant the list shows, and whether it is the request itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewStamp {
    pub at: DateTime<Utc>,
    /// False when the timestamp is a fallback, not a review request.
    pub exact: bool,
    pub basis: StampBasis,
}

/// What the list knows when it picks a timestamp for one pull request.
pub struct StampQuery<'a> {
    pub clock: Option<&'a ReviewClock>,
    pub pending: &'a [RequestedReviewer],
    pub involvement: Involvement,
    pub viewer: &'a str,
    pub teams: &'a [TeamRef],
    /// False until a team lookup has succeeded. An empty list then means the
    /// user is on no teams, not that membership is unknown.
    pub teams_known: bool,
    pub created_at: DateTime<Utc>,
}

/// The request age to paint, or `None` when review is not asked of the viewer.
///
/// The timestamp is the latest request to the viewer or a team they belong to.
/// A request that arrives after their own review replaces the earlier one.
pub fn review_stamp(query: &StampQuery<'_>) -> Option<ReviewStamp> {
    if query.viewer.trim().is_empty() {
        return None;
    }
    let pending_teams = pending_teams(query.pending);
    let user_pending = pending_has_user(query.pending, query.viewer);
    let member_pending = query.teams_known
        && pending_teams
            .iter()
            .any(|team| teams_include(query.teams, team));
    let unknown_team = !query.teams_known
        && query.involvement.contains(Involvement::REVIEW_REQUESTED)
        && !pending_teams.is_empty();
    let match_teams: &[TeamRef] = if query.teams_known {
        query.teams
    } else if unknown_team {
        &pending_teams
    } else {
        &[]
    };
    let matched = query.clock.is_some_and(|clock| {
        clock
            .requests
            .iter()
            .any(|event| event_matches(event, query.viewer, match_teams))
    });
    let asked = user_pending || member_pending || unknown_team;
    let sticky = query.involvement.contains(Involvement::REVIEW_REQUESTED) && matched;
    if !asked && !sticky {
        return None;
    }
    let empty = ReviewClock::default();
    let clock = query.clock.unwrap_or(&empty);
    Some(select_stamp(
        clock,
        query.viewer,
        match_teams,
        query.created_at,
    ))
}

/// Latest matching request, or ready-for-review, or `created_at`.
pub fn select_stamp(
    clock: &ReviewClock,
    viewer: &str,
    teams: &[TeamRef],
    created_at: DateTime<Utc>,
) -> ReviewStamp {
    if let Some(event) = chosen_request(clock, viewer, teams) {
        return ReviewStamp {
            at: event.at,
            exact: true,
            basis: StampBasis::Request,
        };
    }
    if let Some(ready) = clock.ready_at {
        return ReviewStamp {
            at: ready,
            exact: false,
            basis: StampBasis::Ready,
        };
    }
    ReviewStamp {
        at: created_at,
        exact: false,
        basis: StampBasis::Opened,
    }
}

fn chosen_request<'a>(
    clock: &'a ReviewClock,
    viewer: &str,
    teams: &[TeamRef],
) -> Option<&'a ReviewRequestEvent> {
    let mut latest: Option<&ReviewRequestEvent> = None;
    let mut latest_after_review: Option<&ReviewRequestEvent> = None;
    let reviewed = latest_review_at(clock, viewer);
    for event in &clock.requests {
        if !event_matches(event, viewer, teams) {
            continue;
        }
        if latest.is_none_or(|kept| event.at > kept.at) {
            latest = Some(event);
        }
        if reviewed.is_some_and(|at| event.at > at)
            && latest_after_review.is_none_or(|kept| event.at > kept.at)
        {
            latest_after_review = Some(event);
        }
    }
    latest_after_review.or(latest)
}

fn latest_review_at(clock: &ReviewClock, viewer: &str) -> Option<DateTime<Utc>> {
    clock
        .reviews
        .iter()
        .filter(|review| same_login(&review.login, viewer))
        .map(|review| review.at)
        .max()
}

fn event_matches(event: &ReviewRequestEvent, viewer: &str, teams: &[TeamRef]) -> bool {
    match &event.target {
        RequestTarget::User { login } => same_login(login, viewer),
        RequestTarget::Team { org, slug } => teams
            .iter()
            .any(|team| same_login(&team.org, org) && same_login(&team.slug, slug)),
    }
}

fn pending_has_user(pending: &[RequestedReviewer], viewer: &str) -> bool {
    pending.iter().any(|request| match &request.reviewer {
        Reviewer::User { login } => same_login(login, viewer),
        Reviewer::Team(_) => false,
    })
}

fn pending_teams(pending: &[RequestedReviewer]) -> Vec<TeamRef> {
    pending
        .iter()
        .filter_map(|request| match &request.reviewer {
            Reviewer::Team(team) => Some(team.clone()),
            Reviewer::User { .. } => None,
        })
        .collect()
}

fn teams_include(teams: &[TeamRef], team: &TeamRef) -> bool {
    teams
        .iter()
        .any(|known| same_login(&known.org, &team.org) && same_login(&known.slug, &team.slug))
}

fn same_login(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn at(hours: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::hours(hours)
    }

    fn user(login: &str, hours: i64) -> ReviewRequestEvent {
        ReviewRequestEvent {
            at: at(hours),
            target: RequestTarget::User {
                login: login.into(),
            },
        }
    }

    fn team(org: &str, slug: &str, hours: i64) -> ReviewRequestEvent {
        ReviewRequestEvent {
            at: at(hours),
            target: RequestTarget::Team {
                org: org.into(),
                slug: slug.into(),
            },
        }
    }

    fn reviewed(login: &str, hours: i64) -> SubmittedReview {
        SubmittedReview {
            at: at(hours),
            login: login.into(),
        }
    }

    fn web() -> TeamRef {
        TeamRef {
            org: "acme".into(),
            slug: "web".into(),
        }
    }

    fn asked_user(login: &str) -> RequestedReviewer {
        RequestedReviewer::bare(Reviewer::User {
            login: login.into(),
        })
    }

    fn asked_team(org: &str, slug: &str) -> RequestedReviewer {
        RequestedReviewer::bare(Reviewer::Team(TeamRef {
            org: org.into(),
            slug: slug.into(),
        }))
    }

    fn query<'a>(
        clock: Option<&'a ReviewClock>,
        pending: &'a [RequestedReviewer],
        teams: &'a [TeamRef],
        known: bool,
        involvement: Involvement,
    ) -> StampQuery<'a> {
        StampQuery {
            clock,
            pending,
            involvement,
            viewer: "ada",
            teams,
            teams_known: known,
            created_at: at(0),
        }
    }

    #[test]
    fn a_re_request_after_a_review_uses_the_later_request() {
        let clock = ReviewClock {
            requests: vec![user("ada", 1), user("ada", 5), user("sam", 6)],
            reviews: vec![reviewed("ada", 3)],
            ..ReviewClock::default()
        };
        let stamp = select_stamp(&clock, "ada", &[], at(0));
        assert_eq!(stamp.at, at(5));
        assert!(stamp.exact);
        assert_eq!(stamp.basis, StampBasis::Request);
    }

    #[test]
    fn a_review_without_a_new_request_keeps_the_original_ask() {
        let clock = ReviewClock {
            requests: vec![user("ada", 1), team("acme", "web", 2)],
            reviews: vec![reviewed("ada", 4)],
            ..ReviewClock::default()
        };
        let stamp = select_stamp(&clock, "ada", &[web()], at(0));
        assert_eq!(stamp.at, at(2));
    }

    #[test]
    fn a_team_request_counts_and_another_team_does_not() {
        let clock = ReviewClock {
            requests: vec![team("acme", "payments", 4), team("acme", "web", 2)],
            ..ReviewClock::default()
        };
        let stamp = select_stamp(&clock, "ada", &[web()], at(0));
        assert_eq!(stamp.at, at(2));
        assert_eq!(stamp.basis, StampBasis::Request);
    }

    #[test]
    fn someone_elses_request_falls_through_to_ready_then_opened() {
        let ready = ReviewClock {
            requests: vec![user("sam", 3)],
            ready_at: Some(at(1)),
            ..ReviewClock::default()
        };
        let stamp = select_stamp(&ready, "ada", &[], at(0));
        assert_eq!(stamp.at, at(1));
        assert!(!stamp.exact);
        assert_eq!(stamp.basis, StampBasis::Ready);

        let opened = ReviewClock {
            requests: vec![user("sam", 3)],
            ..ReviewClock::default()
        };
        let stamp = select_stamp(&opened, "ada", &[], at(0));
        assert_eq!(stamp.at, at(0));
        assert!(!stamp.exact);
        assert_eq!(stamp.basis, StampBasis::Opened);
    }

    #[test]
    fn the_list_shows_a_direct_request_and_a_known_team() {
        let clock = ReviewClock {
            requests: vec![user("ada", 2)],
            ..ReviewClock::default()
        };
        let pending = [asked_user("Ada")];
        let stamp = review_stamp(&query(
            Some(&clock),
            &pending,
            &[],
            true,
            Involvement::empty(),
        ))
        .unwrap();
        assert_eq!(stamp.at, at(2));

        let clock = ReviewClock {
            requests: vec![team("acme", "web", 3), team("acme", "payments", 6)],
            ..ReviewClock::default()
        };
        let pending = [asked_team("acme", "web")];
        let stamp = review_stamp(&query(
            Some(&clock),
            &pending,
            &[web()],
            true,
            Involvement::REVIEW_REQUESTED,
        ))
        .unwrap();
        assert_eq!(stamp.at, at(3));
    }

    #[test]
    fn an_unknown_membership_uses_only_the_pending_team() {
        let clock = ReviewClock {
            requests: vec![team("acme", "payments", 6), team("acme", "web", 2)],
            ..ReviewClock::default()
        };
        let pending = [asked_team("acme", "web")];
        let stamp = review_stamp(&query(
            Some(&clock),
            &pending,
            &[],
            false,
            Involvement::REVIEW_REQUESTED,
        ))
        .unwrap();
        assert_eq!(stamp.at, at(2));
    }

    #[test]
    fn a_known_empty_team_list_does_not_claim_another_team() {
        let clock = ReviewClock {
            requests: vec![team("acme", "web", 2)],
            ..ReviewClock::default()
        };
        let pending = [asked_team("acme", "web")];
        assert!(
            review_stamp(&query(
                Some(&clock),
                &pending,
                &[],
                true,
                Involvement::REVIEW_REQUESTED,
            ))
            .is_none()
        );
    }

    #[test]
    fn a_finished_request_keeps_its_time_and_an_unknown_one_does_not_guess() {
        let clock = ReviewClock {
            requests: vec![user("ada", 2)],
            reviews: vec![reviewed("ada", 4)],
            ..ReviewClock::default()
        };
        let stamp = review_stamp(&query(
            Some(&clock),
            &[],
            &[],
            true,
            Involvement::REVIEW_REQUESTED,
        ))
        .unwrap();
        assert_eq!(stamp.at, at(2));

        assert!(
            review_stamp(&query(None, &[], &[], true, Involvement::REVIEW_REQUESTED,)).is_none()
        );
    }

    #[test]
    fn a_pending_request_with_no_event_is_approximate() {
        let pending = [asked_user("ada")];
        let clock = ReviewClock {
            ready_at: Some(at(1)),
            ..ReviewClock::default()
        };
        let stamp = review_stamp(&query(
            Some(&clock),
            &pending,
            &[],
            true,
            Involvement::REVIEW_REQUESTED,
        ))
        .unwrap();
        assert_eq!(stamp.basis, StampBasis::Ready);
        assert!(!stamp.exact);

        let stamp = review_stamp(&query(
            None,
            &pending,
            &[],
            true,
            Involvement::REVIEW_REQUESTED,
        ))
        .unwrap();
        assert_eq!(stamp.basis, StampBasis::Opened);
        assert_eq!(stamp.at, at(0));
    }
}
