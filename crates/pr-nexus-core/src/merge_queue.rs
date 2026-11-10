//! GitHub merge queue: what the base branch requires, where a pull request
//! sits in the queue, and whether it can be added.
//!
//! GitHub's GraphQL schema (checked against the public reference):
//! - `BranchProtectionRule` has no `requiresMergeQueue` field.
//! - `Ref` has no `mergeQueue` field.
//! - `PullRequest.isMergeQueueEnabled` is whether that base ref requires the
//!   queue. Branch protection's "Require merge queue" and a ruleset rule of
//!   type `MERGE_QUEUE` ("Merges must be performed via a merge queue") both
//!   show up here. Matching ruleset ref names in the client would redo that.
//! - `PullRequest.mergeQueue` is the queue for the base branch.
//! - `PullRequest.mergeQueueEntry` is this pull request's place in it.
//! - `enqueuePullRequest` takes `pullRequestId` and `expectedHeadOid`.
//! - `dequeuePullRequest` takes the pull request's node id as `id`.

use serde::{Deserialize, Serialize};

use crate::domain::{MergeStateStatus, Mergeable, PrState, PullRequest};
use crate::error::{CoreError, ProviderError};
use crate::overview::{self, PrOverview};
use crate::providers::VcsProvider;
use crate::store::Store;

/// How the list and the detail panel describe a merge queue entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeQueuePhase {
    /// `QUEUED`, `AWAITING_CHECKS`, or `MERGEABLE`.
    Queued,
    /// `LOCKED`: GitHub is merging this entry.
    Merging,
    /// `UNMERGEABLE`: the queue could not merge it.
    Failed,
    /// The newest queue event is a removal and the entry is gone.
    Removed,
}

impl MergeQueuePhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Merging => "merging",
            Self::Failed => "failed",
            Self::Removed => "removed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "merging" => Some(Self::Merging),
            "failed" => Some(Self::Failed),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

/// Queue facts stored for one pull request and shown on its row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeQueueSnapshot {
    /// The base branch requires the merge queue.
    pub required: bool,
    pub phase: Option<MergeQueuePhase>,
    /// 1-based position while the entry exists.
    pub position: Option<u32>,
    /// Seconds until GitHub expects this entry to merge.
    pub eta_seconds: Option<u32>,
    /// Why it failed or was removed, when GitHub sent a reason.
    pub reason: Option<String>,
}

impl MergeQueueSnapshot {
    pub fn in_queue(&self) -> bool {
        matches!(
            self.phase,
            Some(MergeQueuePhase::Queued | MergeQueuePhase::Merging | MergeQueuePhase::Failed)
        )
    }
}

/// The newest added/removed event in the small timeline slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueEvent {
    Added,
    Removed { reason: Option<String> },
}

/// Fields read off one pull request node. Absent flags stay `None` so a
/// payload that never asked about the queue does not clear a stored one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeQueueReading {
    pub enabled: Option<bool>,
    /// `mergeQueue` is a non-null object: the base branch has a queue.
    pub has_queue: bool,
    pub in_queue: bool,
    /// `mergeQueueEntry.state`, when the entry exists.
    pub entry_state: Option<String>,
    pub position: Option<u32>,
    pub eta_seconds: Option<u32>,
    pub latest_event: Option<QueueEvent>,
}

/// `MergeQueueEntry.state` to the phase the UI shows.
pub fn phase_from_entry_state(state: &str) -> Option<MergeQueuePhase> {
    match state {
        "QUEUED" | "AWAITING_CHECKS" | "MERGEABLE" => Some(MergeQueuePhase::Queued),
        "LOCKED" => Some(MergeQueuePhase::Merging),
        "UNMERGEABLE" => Some(MergeQueuePhase::Failed),
        _ => None,
    }
}

/// Map a pull request node onto a snapshot.
///
/// `None` means the payload did not include the queue fields, so the stored
/// row should be left alone. A present `isMergeQueueEnabled` of false, or a
/// null queue and no entry, is a real "not required".
pub fn map_merge_queue(reading: &MergeQueueReading) -> Option<MergeQueueSnapshot> {
    if reading.enabled.is_none()
        && !reading.has_queue
        && reading.entry_state.is_none()
        && reading.latest_event.is_none()
    {
        return None;
    }
    let required = reading.enabled == Some(true) || reading.has_queue;
    let (phase, position, eta, reason) = if let Some(state) = reading.entry_state.as_deref() {
        let phase =
            phase_from_entry_state(state).or(reading.in_queue.then_some(MergeQueuePhase::Queued));
        let queued = phase == Some(MergeQueuePhase::Queued);
        (
            phase,
            reading
                .position
                .filter(|_| queued || phase == Some(MergeQueuePhase::Merging)),
            reading.eta_seconds.filter(|_| queued),
            None,
        )
    } else if required && matches!(reading.latest_event, Some(QueueEvent::Removed { .. })) {
        let reason = match &reading.latest_event {
            Some(QueueEvent::Removed { reason }) => reason.clone(),
            _ => None,
        };
        (Some(MergeQueuePhase::Removed), None, None, reason)
    } else if reading.in_queue {
        (
            Some(MergeQueuePhase::Queued),
            reading.position,
            reading.eta_seconds,
            None,
        )
    } else {
        (None, None, None, None)
    };
    Some(MergeQueueSnapshot {
        required,
        phase,
        position,
        eta_seconds: eta,
        reason,
    })
}

/// Why this pull request cannot be added to the merge queue.
/// `None` means it can.
pub fn enqueue_skip_reason(
    pr: &PullRequest,
    queue: Option<&MergeQueueSnapshot>,
) -> Option<&'static str> {
    if !queue.is_some_and(|queue| queue.required) {
        return Some("No merge queue on the base branch");
    }
    match pr.state {
        PrState::Closed => return Some("Already closed"),
        PrState::Merged => return Some("Already merged"),
        PrState::Open => {}
    }
    if pr.is_draft || pr.merge_state == MergeStateStatus::Draft {
        return Some("Still a draft");
    }
    if queue.is_some_and(|queue| {
        matches!(
            queue.phase,
            Some(MergeQueuePhase::Queued | MergeQueuePhase::Merging)
        )
    }) {
        return Some("Already in the merge queue");
    }
    if pr.head_sha.is_empty() {
        return Some("Head commit is unknown");
    }
    if pr.mergeable == Mergeable::Conflicting || pr.merge_state == MergeStateStatus::Dirty {
        return Some("Has merge conflicts");
    }
    if pr.merge_state == MergeStateStatus::Behind {
        return Some("Branch is behind");
    }
    if !matches!(
        pr.merge_state,
        MergeStateStatus::Clean | MergeStateStatus::HasHooks | MergeStateStatus::Unstable
    ) {
        return Some("Not ready to merge");
    }
    None
}

/// What a bulk add did to one pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueOutcome {
    Added,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueReport {
    pub id: i64,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub outcome: QueueOutcome,
    pub message: Option<String>,
}

/// Adds each pull request whose base branch requires a merge queue.
/// One failure does not stop the others. Ineligible ones are skipped with
/// a short reason and are not sent to GitHub.
pub fn enqueue_pull_requests(
    store: &Store,
    provider: &dyn VcsProvider,
    ids: &[crate::domain::PrId],
) -> Result<Vec<QueueReport>, CoreError> {
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<_> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
    let mut reports = Vec::with_capacity(unique.len());
    for id in unique {
        reports.push(enqueue_one(store, provider, id));
    }
    Ok(reports)
}

fn enqueue_one(store: &Store, provider: &dyn VcsProvider, id: crate::domain::PrId) -> QueueReport {
    let pr = match store.get_pull_request(id) {
        Ok(Some(pr)) => pr,
        Ok(None) => {
            return QueueReport {
                id: id.0,
                repo: String::new(),
                number: 0,
                title: String::new(),
                outcome: QueueOutcome::Failed,
                message: Some("This pull request is not in the list.".into()),
            };
        }
        Err(err) => {
            return QueueReport {
                id: id.0,
                repo: String::new(),
                number: 0,
                title: String::new(),
                outcome: QueueOutcome::Failed,
                message: Some(err.to_string()),
            };
        }
    };
    let stored = store.merge_queue(pr.id).ok().flatten();
    if let Some(reason) = enqueue_skip_reason(&pr, stored.as_ref()) {
        return skipped(&pr, reason);
    }
    match provider.enqueue_pull_request(&pr, &pr.head_sha) {
        Ok(snapshot) => {
            remember(store, pr.id, &snapshot);
            let _ = refresh_keeping(store, provider, &pr, &snapshot);
            added(&pr)
        }
        Err(err) => QueueReport {
            id: pr.id.0,
            repo: format!("{}/{}", pr.repo.owner, pr.repo.name),
            number: pr.number,
            title: pr.title.clone(),
            outcome: QueueOutcome::Failed,
            message: Some(plain(&err)),
        },
    }
}

/// Adds one pull request and returns the refreshed overview.
/// The caller has already confirmed. A skip is a validation error.
pub fn enqueue(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
) -> Result<PrOverview, CoreError> {
    let stored = store.merge_queue(pr.id)?;
    if let Some(reason) = enqueue_skip_reason(pr, stored.as_ref()) {
        return Err(CoreError::Validation(reason.into()));
    }
    let snapshot = provider
        .enqueue_pull_request(pr, &pr.head_sha)
        .map_err(CoreError::from)?;
    remember(store, pr.id, &snapshot);
    refresh_keeping(store, provider, pr, &snapshot)
}

/// Removes one pull request from the queue and returns the refreshed overview.
pub fn dequeue(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
) -> Result<PrOverview, CoreError> {
    let stored = store.merge_queue(pr.id)?;
    if !stored.as_ref().is_some_and(|queue| queue.in_queue()) {
        return Err(CoreError::Validation("Not in the merge queue".into()));
    }
    provider.dequeue_pull_request(pr).map_err(CoreError::from)?;
    let removed = MergeQueueSnapshot {
        required: true,
        phase: Some(MergeQueuePhase::Removed),
        position: None,
        eta_seconds: None,
        reason: None,
    };
    remember(store, pr.id, &removed);
    refresh_keeping(store, provider, pr, &removed)
}

fn remember(store: &Store, id: crate::domain::PrId, queue: &MergeQueueSnapshot) {
    let _ = store.set_merge_queue(id, queue);
}

/// Refresh, but keep `fallback` when GitHub's read has not caught up yet.
fn refresh_keeping(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
    fallback: &MergeQueueSnapshot,
) -> Result<PrOverview, CoreError> {
    let mut overview = match overview::refresh(store, provider, pr) {
        Ok(overview) => overview,
        Err(_) => overview::cached(store, pr.id)?.ok_or_else(|| {
            CoreError::Provider(ProviderError::Network(
                "Added to the merge queue, but the updated pull request could not be read.".into(),
            ))
        })?,
    };
    if queue_read_is_stale(overview.merge_queue.as_ref(), fallback) {
        overview.merge_queue = Some(fallback.clone());
        if let Ok(json) = serde_json::to_string(&overview) {
            let _ = store.upsert_pr_overview(pr.id, json, chrono::Utc::now());
        }
        remember(store, pr.id, fallback);
    }
    Ok(overview)
}

/// The mutation just happened. A refresh that still shows the old phase
/// (or no phase) is behind that action.
fn queue_read_is_stale(read: Option<&MergeQueueSnapshot>, fallback: &MergeQueueSnapshot) -> bool {
    let phase = read.and_then(|queue| queue.phase);
    if fallback.phase == Some(MergeQueuePhase::Removed) {
        return !matches!(phase, Some(MergeQueuePhase::Removed));
    }
    !matches!(
        phase,
        Some(MergeQueuePhase::Queued | MergeQueuePhase::Merging | MergeQueuePhase::Failed)
    )
}

fn added(pr: &PullRequest) -> QueueReport {
    QueueReport {
        id: pr.id.0,
        repo: format!("{}/{}", pr.repo.owner, pr.repo.name),
        number: pr.number,
        title: pr.title.clone(),
        outcome: QueueOutcome::Added,
        message: None,
    }
}

fn skipped(pr: &PullRequest, reason: &str) -> QueueReport {
    QueueReport {
        id: pr.id.0,
        repo: format!("{}/{}", pr.repo.owner, pr.repo.name),
        number: pr.number,
        title: pr.title.clone(),
        outcome: QueueOutcome::Skipped,
        message: Some(reason.to_owned()),
    }
}

fn plain(err: &ProviderError) -> String {
    match err {
        ProviderError::Forbidden(message)
        | ProviderError::Unauthorized(message)
        | ProviderError::NotFound(message)
        | ProviderError::Unsupported(message)
        | ProviderError::Network(message) => message.clone(),
        ProviderError::Api { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AccountId, ChecksState, Involvement, PrId, ProviderRef, RepoRef};
    use chrono::{TimeZone, Utc};

    fn pr(state: PrState, merge_state: MergeStateStatus) -> PullRequest {
        let at = Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap();
        PullRequest {
            id: PrId(7),
            remote: ProviderRef {
                account: AccountId(1),
                id: "PR_acme_7".into(),
            },
            repo: RepoRef {
                host: "github.com".into(),
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 7,
            title: "Tighten the latch".into(),
            url: "https://github.com/acme/widgets/pull/7".into(),
            author_login: "octocat".into(),
            body: String::new(),
            state,
            is_draft: false,
            head_ref: "latch".into(),
            base_ref: "main".into(),
            head_sha: "abc123".into(),
            labels: Vec::new(),
            review_decision: None,
            checks: ChecksState::Success,
            mergeable: Mergeable::Mergeable,
            merge_state,
            requested_reviewers: Vec::new(),
            unresolved_threads: 0,
            total_threads: 0,
            involvement: Involvement::AUTHOR,
            created_at: at,
            updated_at: at,
            last_activity_at: at,
            synced_at: at,
            hidden: false,
        }
    }

    fn required(phase: Option<MergeQueuePhase>) -> MergeQueueSnapshot {
        MergeQueueSnapshot {
            required: true,
            phase,
            position: None,
            eta_seconds: None,
            reason: None,
        }
    }

    #[test]
    fn acme_widgets_clean_pull_request_can_join_the_queue() {
        let queue = required(None);
        assert_eq!(
            enqueue_skip_reason(&pr(PrState::Open, MergeStateStatus::Clean), Some(&queue)),
            None
        );
        assert_eq!(
            enqueue_skip_reason(&pr(PrState::Open, MergeStateStatus::Unstable), Some(&queue)),
            None
        );
    }

    #[test]
    fn acme_widgets_skips_name_the_reason() {
        let open = pr(PrState::Open, MergeStateStatus::Clean);
        let queue = required(None);
        assert_eq!(
            enqueue_skip_reason(&open, None),
            Some("No merge queue on the base branch")
        );
        let mut off = queue.clone();
        off.required = false;
        assert_eq!(
            enqueue_skip_reason(&open, Some(&off)),
            Some("No merge queue on the base branch")
        );
        assert_eq!(
            enqueue_skip_reason(&pr(PrState::Closed, MergeStateStatus::Clean), Some(&queue)),
            Some("Already closed")
        );
        assert_eq!(
            enqueue_skip_reason(&pr(PrState::Merged, MergeStateStatus::Clean), Some(&queue)),
            Some("Already merged")
        );
        let mut draft = open.clone();
        draft.is_draft = true;
        assert_eq!(
            enqueue_skip_reason(&draft, Some(&queue)),
            Some("Still a draft")
        );
        assert_eq!(
            enqueue_skip_reason(&open, Some(&required(Some(MergeQueuePhase::Queued)))),
            Some("Already in the merge queue")
        );
        assert_eq!(
            enqueue_skip_reason(&open, Some(&required(Some(MergeQueuePhase::Merging)))),
            Some("Already in the merge queue")
        );
        // A failed or removed entry can be added again once the PR is ready.
        assert_eq!(
            enqueue_skip_reason(&open, Some(&required(Some(MergeQueuePhase::Failed)))),
            None
        );
        assert_eq!(
            enqueue_skip_reason(&open, Some(&required(Some(MergeQueuePhase::Removed)))),
            None
        );
        let mut no_sha = open.clone();
        no_sha.head_sha.clear();
        assert_eq!(
            enqueue_skip_reason(&no_sha, Some(&queue)),
            Some("Head commit is unknown")
        );
        let mut conflicts = open.clone();
        conflicts.mergeable = Mergeable::Conflicting;
        conflicts.merge_state = MergeStateStatus::Dirty;
        assert_eq!(
            enqueue_skip_reason(&conflicts, Some(&queue)),
            Some("Has merge conflicts")
        );
        assert_eq!(
            enqueue_skip_reason(&pr(PrState::Open, MergeStateStatus::Behind), Some(&queue)),
            Some("Branch is behind")
        );
        assert_eq!(
            enqueue_skip_reason(&pr(PrState::Open, MergeStateStatus::Blocked), Some(&queue)),
            Some("Not ready to merge")
        );
    }

    #[test]
    fn entry_states_map_to_queued_merging_and_failed() {
        for state in ["QUEUED", "AWAITING_CHECKS", "MERGEABLE"] {
            let snap = map_merge_queue(&MergeQueueReading {
                enabled: Some(true),
                has_queue: true,
                in_queue: true,
                entry_state: Some(state.into()),
                position: Some(2),
                eta_seconds: Some(300),
                latest_event: None,
            })
            .unwrap();
            assert_eq!(snap.phase, Some(MergeQueuePhase::Queued), "{state}");
            assert_eq!(snap.position, Some(2));
            assert_eq!(snap.eta_seconds, Some(300));
            assert!(snap.required);
        }
        let merging = map_merge_queue(&MergeQueueReading {
            enabled: Some(true),
            has_queue: true,
            in_queue: true,
            entry_state: Some("LOCKED".into()),
            position: Some(1),
            eta_seconds: Some(20),
            latest_event: None,
        })
        .unwrap();
        assert_eq!(merging.phase, Some(MergeQueuePhase::Merging));
        assert_eq!(merging.eta_seconds, None);

        let failed = map_merge_queue(&MergeQueueReading {
            enabled: Some(true),
            has_queue: true,
            in_queue: true,
            entry_state: Some("UNMERGEABLE".into()),
            position: Some(4),
            eta_seconds: None,
            latest_event: None,
        })
        .unwrap();
        assert_eq!(failed.phase, Some(MergeQueuePhase::Failed));
        assert_eq!(failed.position, None);
    }

    #[test]
    fn a_removal_with_no_entry_is_removed_and_a_missing_payload_is_left_alone() {
        let removed = map_merge_queue(&MergeQueueReading {
            enabled: Some(true),
            has_queue: true,
            in_queue: false,
            entry_state: None,
            position: None,
            eta_seconds: None,
            latest_event: Some(QueueEvent::Removed {
                reason: Some("Required status check build failed.".into()),
            }),
        })
        .unwrap();
        assert_eq!(removed.phase, Some(MergeQueuePhase::Removed));
        assert_eq!(
            removed.reason.as_deref(),
            Some("Required status check build failed.")
        );

        // An entry still on the queue wins over an older removal.
        let still = map_merge_queue(&MergeQueueReading {
            enabled: Some(true),
            has_queue: true,
            in_queue: true,
            entry_state: Some("QUEUED".into()),
            position: Some(3),
            eta_seconds: None,
            latest_event: Some(QueueEvent::Removed { reason: None }),
        })
        .unwrap();
        assert_eq!(still.phase, Some(MergeQueuePhase::Queued));

        assert!(
            map_merge_queue(&MergeQueueReading {
                enabled: None,
                has_queue: false,
                in_queue: false,
                entry_state: None,
                position: None,
                eta_seconds: None,
                latest_event: None,
            })
            .is_none()
        );

        let off = map_merge_queue(&MergeQueueReading {
            enabled: Some(false),
            has_queue: false,
            in_queue: false,
            entry_state: None,
            position: None,
            eta_seconds: None,
            latest_event: None,
        })
        .unwrap();
        assert!(!off.required);
        assert!(off.phase.is_none());
    }

    #[test]
    fn merge_queue_on_the_base_ref_counts_even_without_the_enabled_flag() {
        // `PullRequest.mergeQueue` is the queue for the base branch.
        // `BranchProtectionRule` has no `requiresMergeQueue` field, so a
        // protection object alone is not a signal; the queue object is.
        let snap = map_merge_queue(&MergeQueueReading {
            enabled: None,
            has_queue: true,
            in_queue: false,
            entry_state: None,
            position: None,
            eta_seconds: None,
            latest_event: None,
        })
        .unwrap();
        assert!(snap.required);
        assert!(snap.phase.is_none());
    }
}
