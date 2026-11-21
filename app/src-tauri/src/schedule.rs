//! How many times a timer should run after a wait.
//!
//! A sleep can jump the wall clock across many intervals. The scheduler and
//! the live thread take one pass for that wait, not one pass per missed tick.

use std::time::Duration;

/// How many runs one wait accounts for. A wait shorter than `interval` is
/// not due. A wait that covers one or many intervals is one run.
pub fn runs_due(elapsed: Duration, interval: Duration) -> u32 {
    if interval.is_zero() || elapsed >= interval {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_wait_is_not_due() {
        assert_eq!(runs_due(Duration::from_secs(7), Duration::from_secs(15)), 0);
    }

    #[test]
    fn one_interval_is_one_run() {
        let interval = Duration::from_secs(180);
        assert_eq!(runs_due(interval, interval), 1);
    }

    #[test]
    fn many_missed_intervals_are_still_one_run() {
        let interval = Duration::from_secs(180);
        assert_eq!(runs_due(interval * 50, interval), 1);
    }

    #[test]
    fn a_zero_interval_is_one_run() {
        assert_eq!(runs_due(Duration::ZERO, Duration::ZERO), 1);
    }
}
