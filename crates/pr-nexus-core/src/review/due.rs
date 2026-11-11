//! When a scheduled Slack bump is due.
//!
//! Pure: every caller passes `now`. Nothing here reads the clock, the
//! database, or Slack, and nothing here posts.

use chrono::{DateTime, Datelike, Days, Duration, NaiveTime, TimeZone, Utc, Weekday};

use crate::domain::ReviewDecision;

/// Quiet time after the last review request or bump, with no review since.
///
/// The default is 48 hours, counting only Monday–Friday in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BumpDueRule {
    /// Hours that must pass. Zero means due as soon as there is no review.
    pub after_hours: i64,
    /// Saturday and Sunday (UTC) do not count toward `after_hours`.
    pub working_days: bool,
}

impl Default for BumpDueRule {
    fn default() -> Self {
        Self {
            after_hours: 48,
            working_days: true,
        }
    }
}

/// Facts for one pull request. Times are absolute; the rule decides the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BumpFacts {
    /// Later of the request time and the last bump.
    pub asked_at: DateTime<Utc>,
    /// A review at or after `asked_at` means the bump is not due.
    pub last_review_at: Option<DateTime<Utc>>,
    /// Hidden until this instant. Due again at this time, for this same ask.
    pub snoozed_until: Option<DateTime<Utc>>,
    /// The ask the user skipped. A different `asked_at` is a new cycle.
    pub skipped_ask_at: Option<DateTime<Utc>>,
}

/// Why a bump is or is not ready for the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpStatus {
    /// Quiet long enough, and not snoozed, skipped, or reviewed.
    Due { quiet_for: Duration },
    /// Still inside the wait. `due_at` is the first instant it can be due.
    Waiting { due_at: DateTime<Utc> },
    /// A review landed at or after the ask.
    Reviewed { at: DateTime<Utc> },
    /// The user hid this ask until `until`.
    Snoozed { until: DateTime<Utc> },
    /// The user skipped this exact ask.
    Skipped,
}

/// The instant a request or bump counts from: the bump when there is one.
pub fn ask_time(
    requested_at: DateTime<Utc>,
    last_bumped_at: Option<DateTime<Utc>>,
) -> DateTime<Utc> {
    last_bumped_at.unwrap_or(requested_at)
}

/// When a stored review decision counts as activity since `asked_at`.
///
/// `Approved` and `ChangesRequested` are a review. `last_activity_at` is the
/// time when it is at or after the ask; an older decision still stands, so
/// it counts at the ask and the bump is not due. `ReviewRequired` and no
/// decision are not a review. A comment-only review is not in this signal.
pub fn review_activity_at(
    decision: Option<ReviewDecision>,
    last_activity_at: DateTime<Utc>,
    asked_at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match decision {
        Some(ReviewDecision::Approved | ReviewDecision::ChangesRequested) => {
            Some(if last_activity_at >= asked_at {
                last_activity_at
            } else {
                asked_at
            })
        }
        Some(ReviewDecision::ReviewRequired) | None => None,
    }
}

/// Classify one bump at `now`.
pub fn classify(facts: &BumpFacts, rule: &BumpDueRule, now: DateTime<Utc>) -> BumpStatus {
    if facts.skipped_ask_at == Some(facts.asked_at) {
        return BumpStatus::Skipped;
    }
    if let Some(until) = facts.snoozed_until
        && now < until
    {
        return BumpStatus::Snoozed { until };
    }
    if let Some(at) = facts.last_review_at
        && at >= facts.asked_at
    {
        return BumpStatus::Reviewed { at };
    }
    let due_at = due_at(facts.asked_at, rule);
    if now < facts.asked_at || now < due_at {
        return BumpStatus::Waiting { due_at };
    }
    BumpStatus::Due {
        quiet_for: elapsed(facts.asked_at, now, rule.working_days),
    }
}

/// True when [`classify`] is [`BumpStatus::Due`].
pub fn is_due(facts: &BumpFacts, rule: &BumpDueRule, now: DateTime<Utc>) -> bool {
    matches!(classify(facts, rule, now), BumpStatus::Due { .. })
}

fn need(rule: &BumpDueRule) -> Duration {
    Duration::hours(rule.after_hours.max(0))
}

/// First instant `rule` is satisfied, starting at `asked_at`.
pub fn due_at(asked_at: DateTime<Utc>, rule: &BumpDueRule) -> DateTime<Utc> {
    let need = need(rule);
    if need == Duration::zero() || !rule.working_days {
        return asked_at + need;
    }
    let mut cursor = asked_at;
    let mut left = need;
    // A year of weekdays is more than any accepted setting.
    for _ in 0..400 {
        let Some(midnight) = next_midnight(cursor) else {
            return cursor + left;
        };
        if is_weekday(cursor) {
            let available = midnight - cursor;
            if available >= left {
                return cursor + left;
            }
            left -= available;
        }
        cursor = midnight;
    }
    cursor
}

/// Time from `start` to `end`. Weekends contribute nothing when `working_days`.
pub fn elapsed(start: DateTime<Utc>, end: DateTime<Utc>, working_days: bool) -> Duration {
    if end <= start {
        return Duration::zero();
    }
    if !working_days {
        return end - start;
    }
    let mut total = Duration::zero();
    let mut cursor = start;
    for _ in 0..400 {
        if cursor >= end {
            break;
        }
        let Some(midnight) = next_midnight(cursor) else {
            break;
        };
        let slice_end = midnight.min(end);
        if is_weekday(cursor) {
            total += slice_end - cursor;
        }
        cursor = slice_end;
    }
    total
}

fn is_weekday(at: DateTime<Utc>) -> bool {
    matches!(
        at.weekday(),
        Weekday::Mon | Weekday::Tue | Weekday::Wed | Weekday::Thu | Weekday::Fri
    )
}

fn next_midnight(at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let next = at.date_naive().checked_add_days(Days::new(1))?;
    Some(Utc.from_utc_datetime(&next.and_time(NaiveTime::MIN)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn facts(asked: DateTime<Utc>) -> BumpFacts {
        BumpFacts {
            asked_at: asked,
            last_review_at: None,
            snoozed_until: None,
            skipped_ask_at: None,
        }
    }

    fn rule(hours: i64, working_days: bool) -> BumpDueRule {
        BumpDueRule {
            after_hours: hours,
            working_days,
        }
    }

    #[test]
    fn the_default_is_48_working_hours() {
        assert_eq!(
            BumpDueRule::default(),
            BumpDueRule {
                after_hours: 48,
                working_days: true,
            }
        );
    }

    #[test]
    fn a_bump_is_due_at_48_wall_hours_and_not_before() {
        // Wednesday 10:00.
        let asked = at(2026, 11, 4, 10, 0);
        let plain = rule(48, false);
        assert!(matches!(
            classify(&facts(asked), &plain, asked + Duration::hours(47)),
            BumpStatus::Waiting { .. }
        ));
        assert_eq!(
            classify(&facts(asked), &plain, asked + Duration::hours(48)),
            BumpStatus::Due {
                quiet_for: Duration::hours(48)
            }
        );
    }

    #[test]
    fn working_days_skip_the_weekend() {
        // Friday 16:00. Eight hours remain on Friday, then Monday and Tuesday.
        let asked = at(2026, 11, 6, 16, 0);
        let working = rule(48, true);
        let tuesday = at(2026, 11, 10, 16, 0);
        assert_eq!(due_at(asked, &working), tuesday);
        // Sunday noon is 8 weekday hours (Friday afternoon) and 20 wall hours of weekend.
        let sunday = at(2026, 11, 8, 12, 0);
        assert_eq!(elapsed(asked, sunday, true), Duration::hours(8));
        assert!(matches!(
            classify(&facts(asked), &working, sunday),
            BumpStatus::Waiting { due_at } if due_at == tuesday
        ));
        assert!(is_due(&facts(asked), &working, tuesday));
        assert!(!is_due(
            &facts(asked),
            &working,
            tuesday - Duration::minutes(1)
        ));
    }

    #[test]
    fn a_thursday_ask_is_due_monday_once_weekdays_add_up() {
        // Thursday 12:00 + 48 weekday hours = Monday 12:00
        // (Thu 12h + Fri 24h + Mon 12h). Sunday is not enough.
        let asked = at(2026, 11, 5, 12, 0);
        let working = rule(48, true);
        let monday = at(2026, 11, 9, 12, 0);
        assert_eq!(due_at(asked, &working), monday);
        assert_eq!(
            elapsed(asked, at(2026, 11, 8, 12, 0), true),
            Duration::hours(36)
        );
        assert!(!is_due(&facts(asked), &working, at(2026, 11, 8, 12, 0)));
        assert!(is_due(&facts(asked), &working, monday));
    }

    #[test]
    fn a_weekend_ask_starts_counting_on_monday() {
        let asked = at(2026, 11, 7, 12, 0); // Saturday
        let working = rule(48, true);
        assert_eq!(due_at(asked, &working), at(2026, 11, 11, 0, 0)); // Wednesday 00:00
        assert_eq!(
            elapsed(asked, at(2026, 11, 9, 0, 0), true),
            Duration::zero()
        );
    }

    #[test]
    fn a_review_since_the_ask_is_not_due() {
        let asked = at(2026, 11, 2, 9, 0); // Monday
        let plain = rule(48, false);
        let mut reviewed = facts(asked);
        reviewed.last_review_at = Some(asked);
        assert_eq!(
            classify(&reviewed, &plain, asked + Duration::hours(72)),
            BumpStatus::Reviewed { at: asked }
        );
        reviewed.last_review_at = Some(asked - Duration::seconds(1));
        assert!(is_due(&reviewed, &plain, asked + Duration::hours(48)));
    }

    #[test]
    fn an_approval_is_review_activity_and_a_pending_review_is_not() {
        let asked = at(2026, 11, 2, 9, 0);
        let later = asked + Duration::hours(3);
        assert_eq!(
            review_activity_at(Some(ReviewDecision::Approved), later, asked),
            Some(later)
        );
        assert_eq!(
            review_activity_at(
                Some(ReviewDecision::ChangesRequested),
                asked - Duration::hours(5),
                asked
            ),
            Some(asked),
            "an older changes-requested decision still stands"
        );
        assert_eq!(
            review_activity_at(Some(ReviewDecision::ReviewRequired), later, asked),
            None
        );
        assert_eq!(review_activity_at(None, later, asked), None);
    }

    #[test]
    fn snooze_and_skip_hide_only_this_ask() {
        let asked = at(2026, 11, 2, 9, 0);
        let plain = rule(48, false);
        let later = asked + Duration::hours(72);
        let mut snoozed = facts(asked);
        let until = later + Duration::hours(1);
        snoozed.snoozed_until = Some(until);
        assert_eq!(
            classify(&snoozed, &plain, later),
            BumpStatus::Snoozed { until }
        );
        assert!(is_due(&snoozed, &plain, until));

        let mut skipped = facts(asked);
        skipped.skipped_ask_at = Some(asked);
        assert_eq!(classify(&skipped, &plain, later), BumpStatus::Skipped);
        skipped.asked_at = asked + Duration::hours(1);
        assert!(is_due(
            &skipped,
            &plain,
            skipped.asked_at + Duration::hours(48)
        ));
    }

    #[test]
    fn an_ask_in_the_future_is_waiting() {
        let asked = at(2026, 11, 2, 9, 0);
        let plain = rule(48, false);
        assert!(matches!(
            classify(&facts(asked), &plain, asked - Duration::hours(1)),
            BumpStatus::Waiting { .. }
        ));
    }

    #[test]
    fn the_ask_time_is_the_bump_when_there_was_one() {
        let requested = at(2026, 11, 1, 9, 0);
        let bumped = at(2026, 11, 3, 9, 0);
        assert_eq!(ask_time(requested, None), requested);
        assert_eq!(ask_time(requested, Some(bumped)), bumped);
    }
}
