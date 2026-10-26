//! Pure readiness gate from synced pull-request fields.
//!
//! Reason strings are the ones in the spec table. Blocked wins over Unknown.
//! A `BLOCKED` merge state whose only gap is reviews is Ready, with a note.

use crate::domain::{ChecksState, MergeStateStatus, Mergeable, PrState, Readiness, ReviewDecision};

pub const BLOCKED_WAITING_FOR_REVIEWS: &str = "GitHub reports BLOCKED: waiting for reviews";
pub const BLOCKED_OTHER: &str = "GitHub reports BLOCKED for a reason other than missing reviews";
pub const MERGEABILITY_UNKNOWN: &str = "GitHub mergeability is unknown";
pub const NO_CHECKS: &str = "no CI checks reported";
pub const FILES_TRUNCATED: &str =
    "PR file list may be truncated (≥3000 files); confirm additional owners";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckSnapshot {
    pub name: String,
    pub bucket: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewSnapshot {
    pub state: String,
    pub author_login: String,
    /// `User` counts. A bot does not.
    pub author_is_human: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence<T> {
    pub complete: bool,
    pub error: Option<String>,
    pub items: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessSnapshot {
    pub state: Option<PrState>,
    pub is_draft: Option<bool>,
    pub merge_state: Option<MergeStateStatus>,
    pub mergeable: Option<Mergeable>,
    pub head_sha: Option<String>,
    pub review_decision: Option<ReviewDecision>,
    pub checks_rollup: Option<ChecksState>,
    pub checks: Option<Evidence<Vec<CheckSnapshot>>>,
    pub reviews: Option<Evidence<Vec<ReviewSnapshot>>>,
    pub native_error: Option<String>,
    pub snapshot_changed: bool,
    pub files_truncated: bool,
}

pub fn decide_readiness(snapshot: &ReadinessSnapshot) -> Readiness {
    let mut blocking = Vec::new();
    let mut unknown = Vec::new();

    match snapshot.state {
        Some(PrState::Closed) => blocking.push("PR is closed".to_string()),
        Some(PrState::Merged) => blocking.push("PR is merged".to_string()),
        Some(PrState::Open) => {}
        None => unknown.push("PR lifecycle state is missing or unknown".to_string()),
    }
    match snapshot.is_draft {
        Some(true) => blocking.push("PR is a draft".to_string()),
        Some(false) => {}
        None => unknown.push("draft state is unavailable".to_string()),
    }
    if snapshot.head_sha.as_deref().is_none_or(str::is_empty) {
        unknown.push("PR head SHA is unavailable".to_string());
    }

    let conflicts = snapshot.merge_state == Some(MergeStateStatus::Dirty)
        || snapshot.mergeable == Some(Mergeable::Conflicting);
    if conflicts {
        blocking.push("PR has merge conflicts".to_string());
    } else if snapshot.merge_state == Some(MergeStateStatus::Behind) {
        blocking.push("PR branch is behind the base branch".to_string());
    }

    let mergeability_unknown = matches!(snapshot.mergeable, None | Some(Mergeable::Unknown))
        || matches!(
            snapshot.merge_state,
            None | Some(
                MergeStateStatus::Unknown | MergeStateStatus::HasHooks | MergeStateStatus::Unstable
            )
        );
    if !conflicts && snapshot.merge_state != Some(MergeStateStatus::Behind) && mergeability_unknown
    {
        unknown.push(MERGEABILITY_UNKNOWN.to_string());
    }

    classify_checks(snapshot, &mut blocking, &mut unknown);
    classify_reviews(snapshot, &mut blocking, &mut unknown);

    if let Some(error) = &snapshot.native_error {
        unknown.push(error.clone());
    }
    if snapshot.snapshot_changed {
        unknown.push(
            "PR head or lifecycle state changed during resolution; rerun before proceeding"
                .to_string(),
        );
    }
    if snapshot.files_truncated {
        unknown.push(FILES_TRUNCATED.to_string());
    }

    let reviews_only = blocking.is_empty()
        && unknown.is_empty()
        && snapshot.merge_state == Some(MergeStateStatus::Blocked)
        && snapshot.mergeable == Some(Mergeable::Mergeable)
        && checks_are_green(snapshot)
        && matches!(
            snapshot.review_decision,
            None | Some(ReviewDecision::ReviewRequired)
        );
    if snapshot.merge_state == Some(MergeStateStatus::Blocked)
        && !reviews_only
        && blocking.is_empty()
    {
        unknown.push(BLOCKED_OTHER.to_string());
    }

    if !blocking.is_empty() {
        return Readiness::Blocked { reasons: blocking };
    }
    if !unknown.is_empty() {
        return Readiness::Unknown { reasons: unknown };
    }
    let mut notes = Vec::new();
    if reviews_only {
        notes.push(BLOCKED_WAITING_FOR_REVIEWS.to_string());
    }
    Readiness::Ready { notes }
}

fn classify_checks(
    snapshot: &ReadinessSnapshot,
    blocking: &mut Vec<String>,
    unknown: &mut Vec<String>,
) {
    let Some(checks) = &snapshot.checks else {
        unknown.push(NO_CHECKS.to_string());
        return;
    };
    for check in &checks.items {
        let name = if check.name.is_empty() {
            "unnamed check"
        } else {
            check.name.as_str()
        };
        let bucket = normalize_bucket(&check.bucket);
        match bucket.as_str() {
            "fail" | "cancel" | "timeout" | "action_required" => {
                blocking.push(format!("check {name} is {bucket}"));
            }
            "pending" => blocking.push(format!("check {name} is pending")),
            "pass" | "skipping" => {}
            other => unknown.push(format!("check {name} has unknown status {other}")),
        }
    }
    if checks.items.is_empty() {
        unknown.push(NO_CHECKS.to_string());
    }
    if !checks.complete {
        unknown.push(
            checks
                .error
                .clone()
                .unwrap_or_else(|| "complete PR check evidence is unavailable".to_string()),
        );
    }
}

fn classify_reviews(
    snapshot: &ReadinessSnapshot,
    blocking: &mut Vec<String>,
    unknown: &mut Vec<String>,
) {
    let Some(reviews) = &snapshot.reviews else {
        unknown.push("complete review evidence is unavailable".to_string());
        return;
    };
    for review in &reviews.items {
        if !review.author_is_human {
            continue;
        }
        if review.state.eq_ignore_ascii_case("CHANGES_REQUESTED") {
            let login = if review.author_login.is_empty() {
                "human reviewer"
            } else {
                review.author_login.as_str()
            };
            blocking.push(format!("human reviewer {login} requested changes"));
        }
    }
    if !reviews.complete {
        unknown.push(
            reviews
                .error
                .clone()
                .unwrap_or_else(|| "complete review evidence is unavailable".to_string()),
        );
    }
}

fn checks_are_green(snapshot: &ReadinessSnapshot) -> bool {
    let Some(checks) = &snapshot.checks else {
        return false;
    };
    if !checks.complete || checks.items.is_empty() {
        return false;
    }
    if snapshot.checks_rollup != Some(ChecksState::Success) {
        return false;
    }
    checks.items.iter().all(|check| {
        matches!(
            normalize_bucket(&check.bucket).as_str(),
            "pass" | "skipping"
        )
    })
}

pub fn normalize_bucket(raw: &str) -> String {
    let value = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    match value.as_str() {
        "success" | "successful" | "passed" | "pass" => "pass".into(),
        "neutral" | "skipped" | "skipping" => "skipping".into(),
        "failure" | "failed" | "error" | "fail" | "startup_failure" => "fail".into(),
        "cancelled" | "canceled" | "cancel" => "cancel".into(),
        "timed_out" | "timedout" | "timeout" => "timeout".into(),
        "action_required" => "action_required".into(),
        "expected" | "queued" | "in_progress" | "pending" => "pending".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(bucket: &str) -> CheckSnapshot {
        CheckSnapshot {
            name: "ci".into(),
            bucket: bucket.into(),
        }
    }

    fn ready_base() -> ReadinessSnapshot {
        ReadinessSnapshot {
            state: Some(PrState::Open),
            is_draft: Some(false),
            merge_state: Some(MergeStateStatus::Clean),
            mergeable: Some(Mergeable::Mergeable),
            head_sha: Some("abc".into()),
            review_decision: Some(ReviewDecision::Approved),
            checks_rollup: Some(ChecksState::Success),
            checks: Some(Evidence {
                complete: true,
                error: None,
                items: vec![check("pass"), check("skipping")],
            }),
            reviews: Some(Evidence {
                complete: true,
                error: None,
                items: Vec::new(),
            }),
            native_error: None,
            snapshot_changed: false,
            files_truncated: false,
        }
    }

    fn reasons(readiness: &Readiness) -> &[String] {
        match readiness {
            Readiness::Ready { notes } => notes,
            Readiness::Blocked { reasons } | Readiness::Unknown { reasons } => reasons,
        }
    }

    #[test]
    fn each_table_row_has_the_verbatim_reason() {
        type Row = (
            fn(&mut ReadinessSnapshot),
            fn(&Readiness) -> bool,
            &'static str,
        );
        let cases: Vec<Row> = vec![
            (
                |s| s.state = Some(PrState::Closed),
                |r| matches!(r, Readiness::Blocked { .. }),
                "PR is closed",
            ),
            (
                |s| s.state = Some(PrState::Merged),
                |r| matches!(r, Readiness::Blocked { .. }),
                "PR is merged",
            ),
            (
                |s| s.is_draft = Some(true),
                |r| matches!(r, Readiness::Blocked { .. }),
                "PR is a draft",
            ),
            (
                |s| s.merge_state = Some(MergeStateStatus::Dirty),
                |r| matches!(r, Readiness::Blocked { .. }),
                "PR has merge conflicts",
            ),
            (
                |s| s.mergeable = Some(Mergeable::Conflicting),
                |r| matches!(r, Readiness::Blocked { .. }),
                "PR has merge conflicts",
            ),
            (
                |s| s.merge_state = Some(MergeStateStatus::Behind),
                |r| matches!(r, Readiness::Blocked { .. }),
                "PR branch is behind the base branch",
            ),
            (
                |s| {
                    s.checks.as_mut().unwrap().items = vec![check("fail")];
                },
                |r| matches!(r, Readiness::Blocked { .. }),
                "check ci is fail",
            ),
            (
                |s| {
                    s.checks.as_mut().unwrap().items = vec![check("cancel")];
                },
                |r| matches!(r, Readiness::Blocked { .. }),
                "check ci is cancel",
            ),
            (
                |s| {
                    s.checks.as_mut().unwrap().items = vec![check("timeout")];
                },
                |r| matches!(r, Readiness::Blocked { .. }),
                "check ci is timeout",
            ),
            (
                |s| {
                    s.checks.as_mut().unwrap().items = vec![check("action_required")];
                },
                |r| matches!(r, Readiness::Blocked { .. }),
                "check ci is action_required",
            ),
            (
                |s| {
                    s.checks.as_mut().unwrap().items = vec![check("pending")];
                },
                |r| matches!(r, Readiness::Blocked { .. }),
                "check ci is pending",
            ),
            (
                |s| {
                    s.checks.as_mut().unwrap().items = vec![check("in_progress")];
                },
                |r| matches!(r, Readiness::Blocked { .. }),
                "check ci is pending",
            ),
            (
                |s| {
                    s.reviews.as_mut().unwrap().items = vec![ReviewSnapshot {
                        state: "CHANGES_REQUESTED".into(),
                        author_login: "alice".into(),
                        author_is_human: true,
                    }];
                },
                |r| matches!(r, Readiness::Blocked { .. }),
                "human reviewer alice requested changes",
            ),
            (
                |s| s.mergeable = Some(Mergeable::Unknown),
                |r| matches!(r, Readiness::Unknown { .. }),
                MERGEABILITY_UNKNOWN,
            ),
            (
                |s| s.merge_state = Some(MergeStateStatus::HasHooks),
                |r| matches!(r, Readiness::Unknown { .. }),
                MERGEABILITY_UNKNOWN,
            ),
            (
                |s| s.merge_state = Some(MergeStateStatus::Unstable),
                |r| matches!(r, Readiness::Unknown { .. }),
                MERGEABILITY_UNKNOWN,
            ),
            (
                |s| {
                    s.merge_state = Some(MergeStateStatus::Blocked);
                    s.review_decision = Some(ReviewDecision::ReviewRequired);
                },
                |r| matches!(r, Readiness::Ready { .. }),
                BLOCKED_WAITING_FOR_REVIEWS,
            ),
            (
                |s| {
                    s.merge_state = Some(MergeStateStatus::Blocked);
                    s.review_decision = Some(ReviewDecision::Approved);
                },
                |r| matches!(r, Readiness::Unknown { .. }),
                BLOCKED_OTHER,
            ),
            (
                |s| s.files_truncated = true,
                |r| matches!(r, Readiness::Unknown { .. }),
                FILES_TRUNCATED,
            ),
            (
                |s| s.checks = None,
                |r| matches!(r, Readiness::Unknown { .. }),
                NO_CHECKS,
            ),
            (
                |s| s.native_error = Some("provider blew up".into()),
                |r| matches!(r, Readiness::Unknown { .. }),
                "provider blew up",
            ),
        ];

        for (mutate, expect, reason) in cases {
            let mut snapshot = ready_base();
            mutate(&mut snapshot);
            let readiness = decide_readiness(&snapshot);
            assert!(expect(&readiness), "{reason}: {readiness:?}");
            assert!(
                reasons(&readiness).iter().any(|item| item == reason),
                "{reason} missing from {readiness:?}"
            );
        }
    }

    #[test]
    fn open_clean_pr_is_ready_and_blocked_beats_unknown() {
        assert!(matches!(
            decide_readiness(&ready_base()),
            Readiness::Ready { notes } if notes.is_empty()
        ));
        let mut snapshot = ready_base();
        snapshot.mergeable = Some(Mergeable::Unknown);
        snapshot.checks.as_mut().unwrap().items = vec![check("fail")];
        snapshot.files_truncated = true;
        match decide_readiness(&snapshot) {
            Readiness::Blocked { reasons } => {
                assert!(reasons.iter().any(|reason| reason.contains("ci")));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bot_changes_requested_do_not_block() {
        let mut snapshot = ready_base();
        snapshot.reviews.as_mut().unwrap().items = vec![ReviewSnapshot {
            state: "CHANGES_REQUESTED".into(),
            author_login: "dependabot".into(),
            author_is_human: false,
        }];
        assert!(matches!(
            decide_readiness(&snapshot),
            Readiness::Ready { .. }
        ));
    }

    #[test]
    fn closed_pr_keeps_the_human_changes_request() {
        let mut snapshot = ready_base();
        snapshot.state = Some(PrState::Closed);
        snapshot.reviews.as_mut().unwrap().items = vec![ReviewSnapshot {
            state: "CHANGES_REQUESTED".into(),
            author_login: "alice".into(),
            author_is_human: true,
        }];
        match decide_readiness(&snapshot) {
            Readiness::Blocked { reasons } => {
                assert!(reasons.iter().any(|reason| reason.contains("closed")));
                assert!(reasons.iter().any(|reason| reason.contains("alice")));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn known_blockers_survive_incomplete_evidence_and_a_changed_snapshot_is_unknown() {
        let mut snapshot = ready_base();
        snapshot.checks = Some(Evidence {
            complete: false,
            error: Some("checks failed".into()),
            items: vec![check("fail")],
        });
        snapshot.reviews = Some(Evidence {
            complete: false,
            error: None,
            items: vec![ReviewSnapshot {
                state: "CHANGES_REQUESTED".into(),
                author_login: "alice".into(),
                author_is_human: true,
            }],
        });
        assert!(matches!(
            decide_readiness(&snapshot),
            Readiness::Blocked { .. }
        ));

        let mut changed = ready_base();
        changed.snapshot_changed = true;
        match decide_readiness(&changed) {
            Readiness::Unknown { reasons } => {
                assert!(reasons.iter().any(|reason| reason.contains("changed")));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn legacy_check_states_normalize() {
        assert_eq!(normalize_bucket("SUCCESS"), "pass");
        assert_eq!(normalize_bucket("NEUTRAL"), "skipping");
        assert_eq!(normalize_bucket("CANCELLED"), "cancel");
        assert_eq!(normalize_bucket("TIMED_OUT"), "timeout");
    }

    #[test]
    fn missing_evidence_is_unknown() {
        let snapshot = ReadinessSnapshot {
            state: Some(PrState::Open),
            is_draft: Some(false),
            merge_state: None,
            mergeable: None,
            head_sha: None,
            review_decision: None,
            checks_rollup: None,
            checks: None,
            reviews: None,
            native_error: None,
            snapshot_changed: false,
            files_truncated: false,
        };
        assert!(matches!(
            decide_readiness(&snapshot),
            Readiness::Unknown { reasons } if !reasons.is_empty()
        ));
    }

    #[test]
    fn null_review_decision_with_green_ci_is_waiting_for_reviews() {
        let mut snapshot = ready_base();
        snapshot.merge_state = Some(MergeStateStatus::Blocked);
        snapshot.review_decision = None;
        match decide_readiness(&snapshot) {
            Readiness::Ready { notes } => {
                assert_eq!(notes, [BLOCKED_WAITING_FOR_REVIEWS]);
            }
            other => panic!("{other:?}"),
        }
    }
}
