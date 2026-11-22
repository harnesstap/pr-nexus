//! How long a review may wait before it is overdue.
//!
//! The stored threshold is the settings key `review_sla_hours` (default 48).
//! [`ReviewSla::working_days`] is `review_sla_working_days`. The list and the
//! menu-bar counts read both through [`crate::settings::review_sla`] and
//! compare with [`review_wait_exceeded`]. The Bumps due queue uses
//! `review.bump_due_hours` and does not read this threshold.

use chrono::Local;

/// Hours before a review is overdue when the user has not chosen otherwise.
pub const DEFAULT_REVIEW_SLA_HOURS: i64 = 48;
/// Remaining hours at which the request age turns amber.
pub const DEFAULT_REVIEW_WARN_HOURS: i64 = 12;

/// How close a review request is to the SLA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewUrgency {
    OnTime,
    Approaching,
    Late,
}

pub const HOUR_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// The review wait from Settings. `working_days` skips Saturday and Sunday.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewSla {
    pub hours: i64,
    pub working_days: bool,
}

impl Default for ReviewSla {
    fn default() -> Self {
        Self {
            hours: DEFAULT_REVIEW_SLA_HOURS,
            working_days: false,
        }
    }
}

/// Minutes east of UTC for the machine clock. Desktop code and the webview
/// share this zone, so weekday hours agree.
pub fn local_offset_east_minutes() -> i32 {
    Local::now().offset().local_minus_utc() / 60
}

/// Milliseconds from `since_ms` to `now_ms` that fall on Monday–Friday in a
/// zone `offset_east_minutes` east of UTC. Both instants are UTC millis.
pub fn weekday_elapsed_ms(since_ms: i64, now_ms: i64, offset_east_minutes: i32) -> i64 {
    if now_ms <= since_ms {
        return 0;
    }
    let shift = i64::from(offset_east_minutes) * 60_000;
    let mut cursor = since_ms.saturating_add(shift);
    let end = now_ms.saturating_add(shift);
    if end <= cursor {
        return 0;
    }
    let mut total = 0_i64;
    while cursor < end {
        let day_index = cursor.div_euclid(DAY_MS);
        let day_start = day_index * DAY_MS;
        let next = (day_start + DAY_MS).min(end);
        if is_weekday(day_index) {
            total = total.saturating_add(next - cursor);
        }
        cursor = next;
    }
    total
}

/// Unix day 0 (1970-01-01) is Thursday. `0` is Sunday.
fn is_weekday(day_index: i64) -> bool {
    let weekday = (day_index + 4).rem_euclid(7);
    (1..=5).contains(&weekday)
}

/// True when the wait is strictly longer than the SLA. With `working_days`,
/// only weekday hours count.
pub fn review_wait_exceeded(
    since_ms: i64,
    now_ms: i64,
    sla: &ReviewSla,
    offset_east_minutes: i32,
) -> bool {
    let elapsed = if sla.working_days {
        weekday_elapsed_ms(since_ms, now_ms, offset_east_minutes)
    } else {
        (now_ms - since_ms).max(0)
    };
    elapsed > sla.hours.saturating_mul(HOUR_MS)
}

/// On time, amber, or late for a request that started at `since_ms`.
///
/// Late uses the same strict comparison as [`review_wait_exceeded`]. Amber
/// starts when the remaining wait is `warn_hours` or less. A wait of exactly
/// the SLA is not late.
pub fn review_urgency(
    since_ms: i64,
    now_ms: i64,
    sla: &ReviewSla,
    warn_hours: i64,
    offset_east_minutes: i32,
) -> ReviewUrgency {
    if review_wait_exceeded(since_ms, now_ms, sla, offset_east_minutes) {
        return ReviewUrgency::Late;
    }
    let elapsed = review_elapsed_ms(since_ms, now_ms, sla, offset_east_minutes);
    let limit = sla.hours.max(0).saturating_mul(HOUR_MS);
    let warn = warn_hours
        .clamp(1, sla.hours.max(1))
        .saturating_mul(HOUR_MS);
    if elapsed >= limit.saturating_sub(warn) {
        ReviewUrgency::Approaching
    } else {
        ReviewUrgency::OnTime
    }
}

/// SLA time still left. Negative once the wait is past the limit.
pub fn review_remaining_ms(
    since_ms: i64,
    now_ms: i64,
    sla: &ReviewSla,
    offset_east_minutes: i32,
) -> i64 {
    let limit = sla.hours.max(0).saturating_mul(HOUR_MS);
    let elapsed = review_elapsed_ms(since_ms, now_ms, sla, offset_east_minutes);
    limit - elapsed
}

/// Elapsed milliseconds the SLA compares against (calendar, or weekdays).
pub fn review_elapsed_ms(
    since_ms: i64,
    now_ms: i64,
    sla: &ReviewSla,
    offset_east_minutes: i32,
) -> i64 {
    if sla.working_days {
        weekday_elapsed_ms(since_ms, now_ms, offset_east_minutes)
    } else {
        (now_ms - since_ms).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn ms(text: &str) -> i64 {
        text.parse::<DateTime<Utc>>().unwrap().timestamp_millis()
    }

    #[test]
    fn a_weekend_adds_no_weekday_hours() {
        let since = ms("2026-11-07T00:00:00Z");
        let now = ms("2026-11-09T00:00:00Z");
        assert_eq!(weekday_elapsed_ms(since, now, 0), 0);
        assert_eq!(weekday_elapsed_ms(now, since, 0), 0);
    }

    #[test]
    fn friday_noon_to_monday_noon_is_one_weekday() {
        // Friday 2026-11-06 12:00Z through Monday 2026-11-09 12:00Z:
        // 12h Friday + 12h Monday.
        let since = ms("2026-11-06T12:00:00Z");
        let now = ms("2026-11-09T12:00:00Z");
        assert_eq!(weekday_elapsed_ms(since, now, 0), 24 * HOUR_MS);
        let sla = ReviewSla {
            hours: 48,
            working_days: true,
        };
        assert!(!review_wait_exceeded(since, now, &sla, 0));
        assert!(review_wait_exceeded(
            since,
            now,
            &ReviewSla {
                hours: 48,
                working_days: false,
            },
            0
        ));
    }

    #[test]
    fn the_threshold_is_strictly_greater_than_the_sla() {
        let now = ms("2026-11-10T12:00:00Z");
        let exact = now - 48 * HOUR_MS;
        let sla = ReviewSla::default();
        assert!(!review_wait_exceeded(exact, now, &sla, 0));
        assert!(review_wait_exceeded(exact - 1, now, &sla, 0));
    }

    #[test]
    fn urgency_is_on_time_then_amber_then_late() {
        let now = ms("2026-11-10T12:00:00Z");
        let sla = ReviewSla::default();
        let warn = DEFAULT_REVIEW_WARN_HOURS;
        assert_eq!(
            review_urgency(now - 36 * HOUR_MS + 1, now, &sla, warn, 0),
            ReviewUrgency::OnTime
        );
        assert_eq!(
            review_urgency(now - 36 * HOUR_MS, now, &sla, warn, 0),
            ReviewUrgency::Approaching
        );
        assert_eq!(
            review_urgency(now - 48 * HOUR_MS, now, &sla, warn, 0),
            ReviewUrgency::Approaching
        );
        assert_eq!(
            review_urgency(now - 48 * HOUR_MS - 1, now, &sla, warn, 0),
            ReviewUrgency::Late
        );
        assert_eq!(
            review_remaining_ms(now - 47 * HOUR_MS, now, &sla, 0),
            HOUR_MS
        );
        assert_eq!(
            review_remaining_ms(now - 50 * HOUR_MS, now, &sla, 0),
            -2 * HOUR_MS
        );
    }

    #[test]
    fn weekday_urgency_skips_the_weekend() {
        // Friday 2026-11-06 12:00Z through Monday 2026-11-09 12:00Z is 24 weekday hours.
        let since = ms("2026-11-06T12:00:00Z");
        let now = ms("2026-11-09T12:00:00Z");
        let sla = ReviewSla {
            hours: 48,
            working_days: true,
        };
        assert_eq!(
            review_urgency(since, now, &sla, 12, 0),
            ReviewUrgency::OnTime
        );
        // Friday 00:00 through Monday 12:00 is 36 weekday hours: the amber window.
        let since = ms("2026-11-06T00:00:00Z");
        assert_eq!(
            review_urgency(since, now, &sla, 12, 0),
            ReviewUrgency::Approaching
        );
        assert_eq!(
            review_urgency(
                since,
                now,
                &ReviewSla {
                    hours: 48,
                    working_days: false,
                },
                12,
                0
            ),
            ReviewUrgency::Late
        );
    }
}
