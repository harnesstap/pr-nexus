//! The bumps-due queue (§7.14).
//!
//! Lists pull requests whose bump is due, and records a snooze or a skip.
//! Nothing in this module posts to Slack. Sending stays on the bulk preview
//! the user confirms.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use pr_nexus_core::domain::{PrId, PrState, RequestStatus, ReviewRequest};
use pr_nexus_core::review::due::{self, BumpFacts, BumpStatus};
use pr_nexus_core::settings::{self, BumpHolds, SnoozeHold};
use serde::Serialize;

use crate::error::AppError;
use crate::state::{self, AppState};

/// How long Snooze hides the current ask. Wall clock, not working days.
pub const SNOOZE_HOURS: i64 = 24;

/// One pull request the queue may show. The destination and the message come
/// from the bulk preview, not from here, so the list cannot name a channel
/// the send will not use.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DueBump {
    pub pr_id: i64,
    pub number: u64,
    pub title: String,
    pub repo: String,
    pub asked_at: String,
    /// Whole hours since the ask, on the rule's clock.
    pub quiet_hours: i64,
    pub bump_count: u32,
}

/// Due bumps at `now`, longest quiet first. Reads the store and returns.
pub fn list_due(state: &AppState, now: DateTime<Utc>) -> Result<Vec<DueBump>, AppError> {
    let store = &state.store;
    let rule = settings::bump_due_rule(store)?;
    let holds = settings::bump_holds(store)?;
    let mut due = Vec::new();
    for account in state::github_accounts(store)? {
        let prs = store.list_pull_requests_for_account_slim(account.id)?;
        let mut by_pr: HashMap<PrId, Vec<ReviewRequest>> = HashMap::new();
        for request in store.active_requests_for_account(account.id)? {
            by_pr.entry(request.pr).or_default().push(request);
        }
        for pr in prs {
            if pr.state != PrState::Open
                || pr.hidden
                || !pr
                    .involvement
                    .contains(pr_nexus_core::domain::Involvement::AUTHOR)
            {
                continue;
            }
            let Some(active) = by_pr.get(&pr.id) else {
                continue;
            };
            let Some(anchor) = latest_ask(active) else {
                continue;
            };
            let asked_at = due::ask_time(anchor.requested_at, anchor.last_bumped_at);
            let (snoozed_until, skipped_ask_at) = holds_for(&holds, anchor.id.0, asked_at);
            let facts = BumpFacts {
                asked_at,
                last_review_at: due::review_activity_at(
                    pr.review_decision,
                    pr.last_activity_at,
                    asked_at,
                ),
                snoozed_until,
                skipped_ask_at,
            };
            if let BumpStatus::Due { quiet_for } = due::classify(&facts, &rule, now) {
                due.push(DueBump {
                    pr_id: pr.id.0,
                    number: pr.number,
                    title: pr.title,
                    repo: format!("{}/{}", pr.repo.owner, pr.repo.name),
                    asked_at: asked_at.to_rfc3339(),
                    quiet_hours: quiet_for.num_hours(),
                    bump_count: anchor.bump_count,
                });
            }
        }
    }
    due.sort_by(|left, right| {
        right
            .quiet_hours
            .cmp(&left.quiet_hours)
            .then_with(|| left.repo.cmp(&right.repo))
            .then_with(|| left.number.cmp(&right.number))
    });
    Ok(due)
}

/// Hide `pr_id`'s current ask for [`SNOOZE_HOURS`]. Posts nothing.
pub fn snooze(state: &AppState, pr_id: i64, now: DateTime<Utc>) -> Result<Vec<DueBump>, AppError> {
    let anchor = anchor(state, pr_id)?;
    let asked_at = due::ask_time(anchor.requested_at, anchor.last_bumped_at);
    let mut holds = settings::bump_holds(&state.store)?;
    holds.snooze.insert(
        anchor.id.0.to_string(),
        SnoozeHold {
            until: (now + Duration::hours(SNOOZE_HOURS)).to_rfc3339(),
            asked_at: asked_at.to_rfc3339(),
        },
    );
    settings::set_bump_holds(&state.store, &holds)?;
    list_due(state, now)
}

/// Hide `pr_id`'s current ask until a newer request or bump. Posts nothing.
pub fn skip(state: &AppState, pr_id: i64, now: DateTime<Utc>) -> Result<Vec<DueBump>, AppError> {
    let anchor = anchor(state, pr_id)?;
    let asked_at = due::ask_time(anchor.requested_at, anchor.last_bumped_at);
    let mut holds = settings::bump_holds(&state.store)?;
    holds
        .skip
        .insert(anchor.id.0.to_string(), asked_at.to_rfc3339());
    settings::set_bump_holds(&state.store, &holds)?;
    list_due(state, now)
}

fn anchor(state: &AppState, pr_id: i64) -> Result<ReviewRequest, AppError> {
    let pr = state
        .store
        .get_pull_request(PrId(pr_id))?
        .ok_or_else(|| AppError::new("not_found", "pull request not found"))?;
    latest_ask(&state.store.list_review_requests(pr.id)?)
        .cloned()
        .ok_or_else(|| {
            AppError::new(
                "validation",
                "This pull request has no Slack request to bump",
            )
        })
}

fn latest_ask(requests: &[ReviewRequest]) -> Option<&ReviewRequest> {
    requests
        .iter()
        .filter(|request| request.status == RequestStatus::Active)
        .max_by_key(|request| {
            (
                due::ask_time(request.requested_at, request.last_bumped_at),
                request.id.0,
            )
        })
}

fn holds_for(
    holds: &BumpHolds,
    request_id: i64,
    asked_at: DateTime<Utc>,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let key = request_id.to_string();
    let snoozed = holds.snooze.get(&key).and_then(|hold| {
        let until = DateTime::parse_from_rfc3339(&hold.until).ok()?;
        let asked = DateTime::parse_from_rfc3339(&hold.asked_at).ok()?;
        (asked.with_timezone(&Utc) == asked_at).then_some(until.with_timezone(&Utc))
    });
    let skipped = holds.skip.get(&key).and_then(|text| {
        let asked = DateTime::parse_from_rfc3339(text).ok()?;
        (asked.with_timezone(&Utc) == asked_at).then_some(asked.with_timezone(&Utc))
    });
    (snoozed, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fixture {
        state: AppState,
        dir: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "pr-nexus-bumps-{}-{}",
            std::process::id(),
            Utc::now().timestamp_millis()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = AppState::for_tests(&dir.join("test.db"), true);
        crate::demo::seed_if_empty(&state.store).unwrap();
        Fixture { state, dir }
    }

    #[test]
    fn a_quiet_request_becomes_due_and_snooze_or_skip_only_hides_it() {
        let fx = fixture();
        let now = Utc::now();
        let fresh = list_due(&fx.state, now).unwrap();
        assert!(
            fresh.iter().all(|item| item.number != 1),
            "a bump from a few hours ago is not due yet: {fresh:?}"
        );

        let later = now + Duration::days(5);
        let due = list_due(&fx.state, later).unwrap();
        let latch = due
            .iter()
            .find(|item| item.number == 1)
            .expect("the seeded request is due after five days");
        assert_eq!(latch.repo, "acme/widgets");
        assert!(latch.quiet_hours >= 48);

        let id = latch.pr_id;
        let snoozed = snooze(&fx.state, id, later).unwrap();
        assert!(snoozed.iter().all(|item| item.pr_id != id));
        let after = list_due(&fx.state, later + Duration::hours(SNOOZE_HOURS)).unwrap();
        assert!(after.iter().any(|item| item.pr_id == id));

        let skipped = skip(&fx.state, id, later + Duration::hours(SNOOZE_HOURS)).unwrap();
        assert!(skipped.iter().all(|item| item.pr_id != id));
    }
}
