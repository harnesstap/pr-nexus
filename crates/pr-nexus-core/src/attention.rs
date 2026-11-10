//! Per-pull-request unread, snooze, and mute.
//!
//! Every decision takes `now`. Nothing here reads the clock or the database.

use chrono::{DateTime, Duration, Utc};

use crate::domain::{PrId, PullRequest};
use crate::error::CoreError;

/// The signals that count as activity: a new commit, a review, a CI change,
/// a review thread, or `updated_at` (a pull request comment that does not
/// open a review thread still moves it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub head_sha: String,
    pub checks: String,
    pub review: Option<String>,
    pub threads: u32,
    pub unresolved: u32,
    pub updated_at: DateTime<Utc>,
}

impl Activity {
    /// True when a commit, review, CI state, review thread, or `updated_at` differs.
    pub fn differs_from(&self, other: &Self) -> bool {
        self.head_sha != other.head_sha
            || self.checks != other.checks
            || self.review != other.review
            || self.threads != other.threads
            || self.unresolved != other.unresolved
            || self.updated_at != other.updated_at
    }

    /// Millisecond precision, matching the text the store writes and reads back.
    pub fn at_millis(mut self) -> Self {
        self.updated_at = DateTime::from_timestamp_millis(self.updated_at.timestamp_millis())
            .unwrap_or(self.updated_at);
        self
    }
}

/// One pull request's current activity, keyed so a replaced row keeps its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrActivity {
    pub pr: PrId,
    pub node_id: String,
    pub activity: Activity,
}

impl PrActivity {
    pub fn from_pull_request(pr: &PullRequest) -> Self {
        Self {
            pr: pr.id,
            node_id: pr.remote.id.clone(),
            activity: Activity {
                head_sha: pr.head_sha.clone(),
                checks: pr.checks.as_str().to_owned(),
                review: pr.review_decision.map(|value| value.as_str().to_owned()),
                threads: pr.total_threads,
                unresolved: pr.unresolved_threads,
                updated_at: pr.updated_at,
            }
            .at_millis(),
        }
    }
}

/// Hidden until a time, or until the activity changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snooze {
    Until(DateTime<Utc>),
    UntilActivity(Activity),
}

/// What the user has stored for one pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionState {
    pub seen: Activity,
    pub muted: bool,
    pub snooze: Option<Snooze>,
}

/// What the list shows. `unread` is already false while muted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AttentionFlags {
    pub unread: bool,
    pub muted: bool,
    pub snoozed: bool,
}

impl AttentionFlags {
    /// Counts toward the unread total. Snoozed rows are out of the default list,
    /// and mute has already cleared `unread`.
    pub fn counts_for_attention(self) -> bool {
        self.unread && !self.snoozed
    }
}

/// How long to snooze, or clear it. `activity` waits for the next change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnoozeChoice {
    Day,
    Week,
    UntilActivity,
    Clear,
}

pub fn parse_choice(value: &str) -> Result<SnoozeChoice, CoreError> {
    match value {
        "day" => Ok(SnoozeChoice::Day),
        "week" => Ok(SnoozeChoice::Week),
        "activity" => Ok(SnoozeChoice::UntilActivity),
        "off" => Ok(SnoozeChoice::Clear),
        _ => Err(CoreError::Validation(
            "Snooze is 1 day, 1 week, until the next update, or off".into(),
        )),
    }
}

/// The snooze to store for `choice` at `now`. `Clear` stores nothing.
pub fn snooze_from(choice: SnoozeChoice, current: &Activity, now: DateTime<Utc>) -> Option<Snooze> {
    match choice {
        SnoozeChoice::Day => Some(Snooze::Until(now + Duration::days(1))),
        SnoozeChoice::Week => Some(Snooze::Until(now + Duration::days(7))),
        SnoozeChoice::UntilActivity => Some(Snooze::UntilActivity(current.clone())),
        SnoozeChoice::Clear => None,
    }
}

/// `None` means the pull request has never been seen: not unread, not snoozed.
pub fn evaluate(
    state: Option<&AttentionState>,
    current: &Activity,
    now: DateTime<Utc>,
) -> AttentionFlags {
    let Some(state) = state else {
        return AttentionFlags::default();
    };
    let changed = state.seen.differs_from(current);
    let snoozed = state
        .snooze
        .as_ref()
        .is_some_and(|snooze| snooze_active(snooze, current, now));
    AttentionFlags {
        unread: changed && !state.muted,
        muted: state.muted,
        snoozed,
    }
}

fn snooze_active(snooze: &Snooze, current: &Activity, now: DateTime<Utc>) -> bool {
    match snooze {
        Snooze::Until(until) => now < *until,
        Snooze::UntilActivity(baseline) => !baseline.differs_from(current),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hours: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + hours * 3600, 0).unwrap()
    }

    fn activity() -> Activity {
        Activity {
            head_sha: "aaa".into(),
            checks: "success".into(),
            review: Some("review_required".into()),
            threads: 2,
            unresolved: 1,
            updated_at: at(0),
        }
    }

    fn seen(activity: Activity) -> AttentionState {
        AttentionState {
            seen: activity,
            muted: false,
            snooze: None,
        }
    }

    #[test]
    fn a_pull_request_nobody_has_opened_is_not_unread() {
        let flags = evaluate(None, &activity(), at(1));
        assert_eq!(flags, AttentionFlags::default());
        assert!(!flags.counts_for_attention());
    }

    #[test]
    fn the_same_activity_is_read() {
        let current = activity();
        let flags = evaluate(Some(&seen(current.clone())), &current, at(1));
        assert!(!flags.unread);
    }

    #[test]
    fn each_activity_signal_marks_unread() {
        let base = seen(activity());
        let cases = [
            (
                "commit",
                Activity {
                    head_sha: "bbb".into(),
                    ..activity()
                },
            ),
            (
                "ci",
                Activity {
                    checks: "failure".into(),
                    ..activity()
                },
            ),
            (
                "review",
                Activity {
                    review: Some("approved".into()),
                    ..activity()
                },
            ),
            (
                "cleared review",
                Activity {
                    review: None,
                    ..activity()
                },
            ),
            (
                "threads",
                Activity {
                    threads: 3,
                    ..activity()
                },
            ),
            (
                "resolved thread",
                Activity {
                    unresolved: 0,
                    ..activity()
                },
            ),
            (
                "comment",
                Activity {
                    updated_at: at(2),
                    ..activity()
                },
            ),
        ];
        for (name, current) in cases {
            let flags = evaluate(Some(&base), &current, at(3));
            assert!(flags.unread, "{name}");
            assert!(flags.counts_for_attention(), "{name}");
        }
    }

    #[test]
    fn mute_hides_unread_and_drops_it_from_the_attention_count() {
        let mut state = seen(activity());
        state.muted = true;
        let current = Activity {
            head_sha: "bbb".into(),
            checks: "failure".into(),
            ..activity()
        };
        let flags = evaluate(Some(&state), &current, at(1));
        assert!(flags.muted);
        assert!(!flags.unread);
        assert!(!flags.counts_for_attention());
    }

    #[test]
    fn a_timed_snooze_hides_the_row_until_the_deadline() {
        let now = at(0);
        let current = activity();
        let day = snooze_from(SnoozeChoice::Day, &current, now).unwrap();
        let week = snooze_from(SnoozeChoice::Week, &current, now).unwrap();
        assert_eq!(day, Snooze::Until(now + Duration::days(1)));
        assert_eq!(week, Snooze::Until(now + Duration::days(7)));

        let mut state = seen(current.clone());
        state.snooze = Some(day);
        let before = evaluate(
            Some(&state),
            &current,
            now + Duration::days(1) - Duration::seconds(1),
        );
        assert!(before.snoozed);
        assert!(!before.counts_for_attention());
        let at_deadline = evaluate(Some(&state), &current, now + Duration::days(1));
        assert!(!at_deadline.snoozed);

        state.snooze = Some(week);
        assert!(evaluate(Some(&state), &current, now + Duration::days(6)).snoozed);
        assert!(!evaluate(Some(&state), &current, now + Duration::days(7)).snoozed);
    }

    #[test]
    fn a_time_snooze_stays_until_the_deadline_even_when_activity_changes() {
        let now = at(0);
        let mut state = seen(activity());
        state.snooze = snooze_from(SnoozeChoice::Day, &activity(), now);
        let pushed = Activity {
            head_sha: "ccc".into(),
            ..activity()
        };
        let flags = evaluate(Some(&state), &pushed, now + Duration::hours(1));
        assert!(flags.snoozed);
        assert!(flags.unread);
        assert!(!flags.counts_for_attention());
    }

    #[test]
    fn snooze_until_activity_returns_on_a_push_a_review_a_comment_or_ci() {
        let now = at(0);
        let current = activity();
        let mut state = seen(current.clone());
        state.snooze = snooze_from(SnoozeChoice::UntilActivity, &current, now);
        assert!(evaluate(Some(&state), &current, now + Duration::days(30)).snoozed);

        for current in [
            Activity {
                head_sha: "ddd".into(),
                ..activity()
            },
            Activity {
                checks: "pending".into(),
                ..activity()
            },
            Activity {
                review: Some("changes_requested".into()),
                ..activity()
            },
            Activity {
                threads: 9,
                ..activity()
            },
            Activity {
                updated_at: at(4),
                ..activity()
            },
        ] {
            let flags = evaluate(Some(&state), &current, now);
            assert!(!flags.snoozed);
            assert!(flags.unread);
            assert!(flags.counts_for_attention());
        }
    }

    #[test]
    fn clearing_a_snooze_stores_nothing() {
        assert!(snooze_from(SnoozeChoice::Clear, &activity(), at(0)).is_none());
    }

    #[test]
    fn snooze_choice_names_are_fixed() {
        assert_eq!(parse_choice("day").unwrap(), SnoozeChoice::Day);
        assert_eq!(parse_choice("week").unwrap(), SnoozeChoice::Week);
        assert_eq!(
            parse_choice("activity").unwrap(),
            SnoozeChoice::UntilActivity
        );
        assert_eq!(parse_choice("off").unwrap(), SnoozeChoice::Clear);
        assert!(parse_choice("forever").is_err());
    }
}
