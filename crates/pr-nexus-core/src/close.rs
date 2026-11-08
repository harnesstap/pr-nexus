//! Close one or more open pull requests on the provider.
//!
//! A comment is posted only when the text is not empty. The same comment is
//! not posted again if an earlier attempt already saved it and the close
//! itself failed. Closing never merges and never deletes the branch. A pull
//! request that is already closed or merged is left alone.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::domain::{PrId, PrState, PullRequest};
use crate::error::{CoreError, ProviderError};
use crate::overview::{self, PrOverview};
use crate::providers::VcsProvider;
use crate::store::Store;
use crate::sync;

/// How many pull requests a bulk close talks to at once. The GitHub client
/// also allows at most 4 requests to one host at a time.
pub const CLOSE_CONCURRENCY: usize = 4;

/// GitHub rejects a comment longer than this.
const MAX_COMMENT_CHARS: usize = 65_536;

/// What happened to one pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseOutcome {
    /// Closed on the provider, or the provider said it was already closed
    /// and the row was updated to match.
    Closed,
    /// Already closed or merged here, so nothing was sent.
    Skipped,
    /// The provider or the store refused this pull request.
    Failed,
}

/// One pull request in a close.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseReport {
    pub id: i64,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub outcome: CloseOutcome,
    /// True when this close's comment is on the pull request, including a
    /// comment posted by an earlier attempt that did not finish the close.
    pub comment_posted: bool,
    /// Why it was skipped or why it failed. Empty after a clean close.
    pub message: Option<String>,
}

/// Closes `ids`, posting `comment` on each open one when it is not blank.
/// Results follow `ids`, with duplicates removed. One failure does not stop
/// the others. At most [`CLOSE_CONCURRENCY`] run at once.
pub fn close_pull_requests(
    store: &Store,
    provider: &dyn VcsProvider,
    ids: &[PrId],
    comment: &str,
) -> Result<Vec<CloseReport>, CoreError> {
    let comment = comment.trim();
    if comment.chars().count() > MAX_COMMENT_CHARS {
        return Err(CoreError::Validation(
            "Comment is too long. Shorten it and try again.".into(),
        ));
    }
    let comment = comment.to_owned();
    let mut seen = HashSet::new();
    let unique: Vec<PrId> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();

    let mut work = Vec::new();
    let mut early = Vec::new();
    for id in &unique {
        match store.get_pull_request(*id) {
            Ok(Some(pr)) => work.push(pr),
            Ok(None) => early.push(missing(*id)),
            Err(err) => early.push(bare(*id, CloseOutcome::Failed, false, err.to_string())),
        }
    }
    let mut done = bounded(&work, CLOSE_CONCURRENCY, |pr| {
        close_one(store, provider, pr, &comment)
    });
    done.append(&mut early);
    let by_id: HashMap<i64, CloseReport> =
        done.into_iter().map(|report| (report.id, report)).collect();
    Ok(unique
        .into_iter()
        .filter_map(|id| by_id.get(&id.0).cloned())
        .collect())
}

fn close_one(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
    comment: &str,
) -> CloseReport {
    if pr.state != PrState::Open {
        let message = match pr.state {
            PrState::Closed => "Already closed.",
            PrState::Merged => "Already merged.",
            PrState::Open => "This pull request is still open.",
        };
        return report(pr, CloseOutcome::Skipped, false, Some(message.into()));
    }

    let mut comment_posted = false;
    if !comment.is_empty() {
        match store.close_comment_body(pr.id) {
            Ok(Some(prior)) if prior == comment => comment_posted = true,
            Ok(_) => match provider.add_pull_request_comment(pr, comment) {
                Ok(()) => {
                    if let Err(err) = store.remember_close_comment(pr.id, comment) {
                        return report(
                            pr,
                            CloseOutcome::Failed,
                            true,
                            Some(format!(
                                "Comment posted, but it could not be recorded: {err}"
                            )),
                        );
                    }
                    comment_posted = true;
                }
                Err(err) => {
                    return report(
                        pr,
                        CloseOutcome::Failed,
                        false,
                        Some(plain_provider_message(&err)),
                    );
                }
            },
            Err(err) => {
                return report(pr, CloseOutcome::Failed, false, Some(err.to_string()));
            }
        }
    }

    match provider.close_pull_request(pr) {
        Ok(()) => finish_closed(store, provider, pr, comment_posted),
        Err(err) => match already_finished(&err) {
            Some(state) => finish_known(store, pr, state, comment_posted),
            None => {
                let reason = plain_provider_message(&err);
                let message = if comment_posted {
                    format!("Comment posted, but the pull request was not closed: {reason}")
                } else {
                    reason
                };
                report(pr, CloseOutcome::Failed, comment_posted, Some(message))
            }
        },
    }
}

/// The provider closed it. Record that locally so the open list drops it
/// and a lagging search cannot bring it back.
fn finish_closed(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
    comment_posted: bool,
) -> CloseReport {
    let now = Utc::now();
    if let Err(err) = store.mark_closed_from_app(pr.id, now) {
        return report(
            pr,
            CloseOutcome::Failed,
            comment_posted,
            Some(format!(
                "The pull request was closed on GitHub, but it could not be updated here: {err}"
            )),
        );
    }
    let closed = PullRequest {
        state: PrState::Closed,
        updated_at: now,
        last_activity_at: now,
        ..pr.clone()
    };
    if let Err(err) = sync::close_requests_of_finished(store, &closed, now) {
        return report(
            pr,
            CloseOutcome::Failed,
            comment_posted,
            Some(format!(
                "The pull request was closed on GitHub, but its review requests could not be updated: {err}"
            )),
        );
    }
    refresh_as(store, provider, &closed, PrState::Closed);
    report(pr, CloseOutcome::Closed, comment_posted, None)
}

/// The provider says it is already closed or merged. Record that and do
/// not try again. This is not a merge and not a branch delete.
fn finish_known(
    store: &Store,
    pr: &PullRequest,
    state: PrState,
    comment_posted: bool,
) -> CloseReport {
    let now = Utc::now();
    let saved = match state {
        PrState::Closed => store.mark_closed_from_app(pr.id, now),
        PrState::Merged => store.set_pull_request_state(pr.id, PrState::Merged),
        PrState::Open => Ok(()),
    };
    if let Err(err) = saved {
        return report(
            pr,
            CloseOutcome::Failed,
            comment_posted,
            Some(format!(
                "GitHub says this pull request is already finished, but it could not be updated here: {err}"
            )),
        );
    }
    let finished = PullRequest {
        state,
        ..pr.clone()
    };
    if let Err(err) = sync::close_requests_of_finished(store, &finished, now) {
        return report(
            pr,
            CloseOutcome::Failed,
            comment_posted,
            Some(format!(
                "GitHub says this pull request is already finished, but its review requests could not be updated: {err}"
            )),
        );
    }
    let message = match state {
        PrState::Closed => "Already closed.",
        PrState::Merged => "Already merged.",
        PrState::Open => "This pull request is still open.",
    };
    report(
        pr,
        CloseOutcome::Skipped,
        comment_posted,
        Some(message.into()),
    )
}

/// Best effort. A stale or failed read must not turn a finished close into
/// an error. The stored row is already closed.
fn refresh_as(store: &Store, provider: &dyn VcsProvider, pr: &PullRequest, state: PrState) {
    let mut overview = match overview::refresh(store, provider, pr) {
        Ok(overview) => overview,
        Err(_) => match overview::cached(store, pr.id) {
            Ok(Some(overview)) => overview,
            _ => return,
        },
    };
    if overview.state != state.as_str() {
        force_overview_state(store, pr.id, &mut overview, state);
    }
}

fn force_overview_state(store: &Store, id: PrId, overview: &mut PrOverview, state: PrState) {
    overview.state = state.as_str().to_owned();
    if state != PrState::Open {
        overview.auto_merge = None;
    }
    if let Ok(json) = serde_json::to_string(overview) {
        let _ = store.upsert_pr_overview(id, json, Utc::now());
    }
}

fn already_finished(err: &ProviderError) -> Option<PrState> {
    let text = plain_provider_message(err).to_ascii_lowercase();
    // Match the finished-state phrases only. A permission or conflict error
    // can mention a merge without the pull request being merged.
    if text.contains("already merged") {
        Some(PrState::Merged)
    } else if text.contains("already closed") {
        Some(PrState::Closed)
    } else {
        None
    }
}

pub(crate) fn plain_provider_message(err: &ProviderError) -> String {
    match err {
        ProviderError::Forbidden(message)
        | ProviderError::Unauthorized(message)
        | ProviderError::NotFound(message)
        | ProviderError::Unsupported(message) => message.clone(),
        ProviderError::Api { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

fn report(
    pr: &PullRequest,
    outcome: CloseOutcome,
    comment_posted: bool,
    message: Option<String>,
) -> CloseReport {
    CloseReport {
        id: pr.id.0,
        repo: format!("{}/{}", pr.repo.owner, pr.repo.name),
        number: pr.number,
        title: pr.title.clone(),
        outcome,
        comment_posted,
        message,
    }
}

fn missing(id: PrId) -> CloseReport {
    bare(
        id,
        CloseOutcome::Failed,
        false,
        "This pull request is not in the app.".into(),
    )
}

fn bare(id: PrId, outcome: CloseOutcome, comment_posted: bool, message: String) -> CloseReport {
    CloseReport {
        id: id.0,
        repo: String::new(),
        number: 0,
        title: String::new(),
        outcome,
        comment_posted,
        message: Some(message),
    }
}

/// `f` over `items` on at most `limit` threads, in input order.
fn bounded<T, R>(items: &[T], limit: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R>
where
    T: Sync,
    R: Send,
{
    let workers = limit.max(1).min(items.len());
    if workers <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    let result = f(item);
                    *slots[index]
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .expect("every slot is filled")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use crate::domain::{
        Account, AccountId, AccountKind, AuthMode, ChecksState, CloseReason, Involvement,
        MergeStateStatus, Mergeable, ProviderRef, RepoRef, Repository, RequestSource,
        RequestStatus, ReviewRequest, ReviewRequestId, TeamRef, VcsKind,
    };
    use crate::error::ProviderError;
    use crate::providers::{
        OwnershipInputs, OwnershipProbe, PrQuery, RateBudget, SyncedPullRequest, TeamMembers,
        VcsProvider, VcsUser,
    };
    use chrono::{Duration as ChronoDuration, TimeZone, Utc};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Step {
        Comment(String, String),
        Close(String),
        Merge(String),
    }

    struct Script {
        log: Mutex<Vec<Step>>,
        fail_close: HashSet<String>,
        already_closed: HashSet<String>,
        already_merged: HashSet<String>,
        delay: Duration,
        inflight: AtomicUsize,
        peak: AtomicUsize,
    }

    impl Script {
        fn note(&self, step: Step) {
            let now = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            if !self.delay.is_zero() {
                thread::sleep(self.delay);
            }
            self.log.lock().unwrap().push(step);
            self.inflight.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl VcsProvider for Script {
        fn kind(&self) -> VcsKind {
            VcsKind::GitHub
        }
        fn viewer(&self) -> Result<VcsUser, ProviderError> {
            unused()
        }
        fn search_pull_requests(
            &self,
            _: &PrQuery,
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            unused()
        }
        fn refresh_pull_requests(
            &self,
            _: &[ProviderRef],
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            unused()
        }
        fn pull_request_status(
            &self,
            _: &[ProviderRef],
        ) -> Result<Vec<crate::providers::PrStatus>, ProviderError> {
            unused()
        }
        fn ownership_probe(&self, _: &PullRequest) -> Result<OwnershipProbe, ProviderError> {
            unused()
        }
        fn ownership_inputs(
            &self,
            _: &PullRequest,
            _: &OwnershipProbe,
        ) -> Result<OwnershipInputs, ProviderError> {
            unused()
        }
        fn team_members(&self, _: &TeamRef) -> Result<TeamMembers, ProviderError> {
            unused()
        }
        fn rate_limit(&self) -> Result<RateBudget, ProviderError> {
            unused()
        }
        fn request_team_review(&self, _: &PullRequest, _: &[TeamRef]) -> Result<(), ProviderError> {
            unused()
        }
        fn merge_pull_request(
            &self,
            pr: &PullRequest,
            _: crate::overview::MergeMethod,
        ) -> Result<(), ProviderError> {
            self.note(Step::Merge(pr.remote.id.clone()));
            Err(ProviderError::Unsupported(
                "merge is not part of close".into(),
            ))
        }
        fn add_pull_request_comment(
            &self,
            pr: &PullRequest,
            body: &str,
        ) -> Result<(), ProviderError> {
            self.note(Step::Comment(pr.remote.id.clone(), body.to_owned()));
            Ok(())
        }
        fn close_pull_request(&self, pr: &PullRequest) -> Result<(), ProviderError> {
            self.note(Step::Close(pr.remote.id.clone()));
            if self.fail_close.contains(&pr.remote.id) {
                return Err(ProviderError::Forbidden(
                    "You don't have permission to close this pull request.".into(),
                ));
            }
            if self.already_closed.contains(&pr.remote.id) {
                return Err(ProviderError::Api {
                    code: "graphql".into(),
                    message: "Pull request is already closed".into(),
                });
            }
            if self.already_merged.contains(&pr.remote.id) {
                return Err(ProviderError::Api {
                    code: "graphql".into(),
                    message: "Pull request is already merged".into(),
                });
            }
            Ok(())
        }
    }

    fn unused<T>() -> Result<T, ProviderError> {
        Err(ProviderError::Unsupported("not used".into()))
    }

    fn open_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("close.db")).unwrap();
        (dir, store)
    }

    fn at() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
    }

    fn seed(store: &Store, node: &str, number: u64, state: PrState) -> PullRequest {
        let account = store
            .insert_account(Account {
                id: AccountId(0),
                kind: AccountKind::GitHub,
                display_name: "ada".into(),
                host: "github.com".into(),
                external_id: format!("gh-{node}"),
                slack_workspace: None,
                login: "ada".into(),
                auth_mode: AuthMode::GhCli,
                keychain_key: None,
                created_at: at(),
            })
            .unwrap();
        let repo = store
            .insert_repository(Repository {
                id: 0,
                account_id: account.id,
                owner: "acme".into(),
                name: "widgets".into(),
                node_id: Some(format!("R_{node}")),
                default_branch: Some("main".into()),
                archived: false,
            })
            .unwrap();
        store
            .upsert_pull_request(
                repo.id,
                PullRequest {
                    id: PrId(0),
                    remote: ProviderRef {
                        account: account.id,
                        id: node.into(),
                    },
                    repo: RepoRef {
                        host: "github.com".into(),
                        owner: "acme".into(),
                        name: "widgets".into(),
                    },
                    number,
                    title: format!("Change {number}"),
                    url: format!("https://github.com/acme/widgets/pull/{number}"),
                    author_login: "ada".into(),
                    body: String::new(),
                    state,
                    is_draft: false,
                    head_ref: "feature/latch".into(),
                    base_ref: "main".into(),
                    head_sha: "abc".into(),
                    labels: Vec::new(),
                    review_decision: None,
                    checks: ChecksState::Success,
                    mergeable: Mergeable::Mergeable,
                    merge_state: MergeStateStatus::Clean,
                    requested_reviewers: Vec::new(),
                    unresolved_threads: 0,
                    total_threads: 0,
                    involvement: Involvement::AUTHOR,
                    created_at: at(),
                    updated_at: at(),
                    last_activity_at: at(),
                    synced_at: at(),
                    hidden: false,
                },
            )
            .unwrap()
    }

    fn script() -> Script {
        Script {
            log: Mutex::new(Vec::new()),
            fail_close: HashSet::new(),
            already_closed: HashSet::new(),
            already_merged: HashSet::new(),
            delay: Duration::ZERO,
            inflight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    fn log_of(script: &Script) -> Vec<Step> {
        script.log.lock().unwrap().clone()
    }

    #[test]
    fn a_comment_is_posted_before_close_and_a_blank_one_is_not() {
        let (_dir, store) = open_store();
        let with_note = seed(&store, "PR_note", 7, PrState::Open);
        let plain = seed(&store, "PR_plain", 8, PrState::Open);
        let blank = seed(&store, "PR_blank", 9, PrState::Open);
        let script = script();

        let noted =
            close_pull_requests(&store, &script, &[with_note.id], "  Please close this. \n")
                .unwrap();
        assert_eq!(noted[0].outcome, CloseOutcome::Closed);
        assert!(noted[0].comment_posted);
        assert_eq!(noted[0].message, None);
        assert_eq!(
            log_of(&script),
            vec![
                Step::Comment("PR_note".into(), "Please close this.".into()),
                Step::Close("PR_note".into()),
            ]
        );
        assert_eq!(
            store.get_pull_request(with_note.id).unwrap().unwrap().state,
            PrState::Closed
        );

        let quiet = close_pull_requests(&store, &script, &[plain.id], "").unwrap();
        assert_eq!(quiet[0].outcome, CloseOutcome::Closed);
        assert!(!quiet[0].comment_posted);

        let spaces = close_pull_requests(&store, &script, &[blank.id], " \n\t ").unwrap();
        assert_eq!(spaces[0].outcome, CloseOutcome::Closed);
        assert!(!spaces[0].comment_posted);
        let comments: Vec<_> = log_of(&script)
            .into_iter()
            .filter(|step| matches!(step, Step::Comment(_, _)))
            .collect();
        assert_eq!(comments.len(), 1);
        assert!(
            log_of(&script)
                .iter()
                .all(|step| !matches!(step, Step::Merge(_)))
        );
    }

    #[test]
    fn a_posted_comment_is_not_repeated_when_close_fails_and_is_retried() {
        let (_dir, store) = open_store();
        let pr = seed(&store, "PR_retry", 11, PrState::Open);
        let mut script = script();
        script.fail_close.insert("PR_retry".into());

        let failed = close_pull_requests(&store, &script, &[pr.id], "No longer needed").unwrap();
        assert_eq!(failed[0].outcome, CloseOutcome::Failed);
        assert!(failed[0].comment_posted);
        assert_eq!(
            failed[0].message.as_deref(),
            Some(
                "Comment posted, but the pull request was not closed: You don't have permission to close this pull request."
            )
        );
        assert_eq!(
            store.get_pull_request(pr.id).unwrap().unwrap().state,
            PrState::Open
        );

        script.fail_close.clear();
        let again = close_pull_requests(&store, &script, &[pr.id], "No longer needed").unwrap();
        assert_eq!(again[0].outcome, CloseOutcome::Closed);
        assert!(again[0].comment_posted);
        let comments = log_of(&script)
            .into_iter()
            .filter(|step| matches!(step, Step::Comment(_, _)))
            .count();
        assert_eq!(comments, 1, "retry must not post the same comment again");
        assert_eq!(
            log_of(&script)
                .into_iter()
                .filter(|step| matches!(step, Step::Close(_)))
                .count(),
            2
        );
    }

    #[test]
    fn a_changed_comment_on_retry_is_posted() {
        let (_dir, store) = open_store();
        let pr = seed(&store, "PR_edit", 12, PrState::Open);
        let mut script = script();
        script.fail_close.insert("PR_edit".into());
        close_pull_requests(&store, &script, &[pr.id], "First").unwrap();
        script.fail_close.clear();
        close_pull_requests(&store, &script, &[pr.id], "Second").unwrap();
        let comments: Vec<_> = log_of(&script)
            .into_iter()
            .filter_map(|step| match step {
                Step::Comment(_, body) => Some(body),
                _ => None,
            })
            .collect();
        assert_eq!(comments, vec!["First".to_owned(), "Second".to_owned()]);
    }

    #[test]
    fn bulk_close_reports_each_pull_request_and_skips_finished_ones() {
        let (_dir, store) = open_store();
        let ok = seed(&store, "PR_ok", 1, PrState::Open);
        let denied = seed(&store, "PR_no", 2, PrState::Open);
        let closed = seed(&store, "PR_closed", 3, PrState::Closed);
        let merged = seed(&store, "PR_merged", 4, PrState::Merged);
        let known = seed(&store, "PR_known", 5, PrState::Open);
        let mut script = script();
        script.fail_close.insert("PR_no".into());
        script.already_merged.insert("PR_known".into());

        let request = seed_request(&store, denied.id);
        let reports = close_pull_requests(
            &store,
            &script,
            &[ok.id, denied.id, closed.id, merged.id, known.id, PrId(404)],
            "Done",
        )
        .unwrap();
        assert_eq!(reports.len(), 6);
        assert_eq!(reports[0].outcome, CloseOutcome::Closed);
        assert!(reports[0].comment_posted);
        assert_eq!(reports[1].outcome, CloseOutcome::Failed);
        assert!(reports[1].comment_posted);
        assert!(
            reports[1]
                .message
                .as_deref()
                .unwrap()
                .contains("Comment posted, but")
        );
        assert_eq!(reports[2].outcome, CloseOutcome::Skipped);
        assert_eq!(reports[2].message.as_deref(), Some("Already closed."));
        assert!(!reports[2].comment_posted);
        assert_eq!(reports[3].message.as_deref(), Some("Already merged."));
        assert_eq!(reports[4].outcome, CloseOutcome::Skipped);
        assert_eq!(reports[4].message.as_deref(), Some("Already merged."));
        assert_eq!(
            store.get_pull_request(known.id).unwrap().unwrap().state,
            PrState::Merged
        );
        assert_eq!(reports[5].outcome, CloseOutcome::Failed);
        assert_eq!(
            store.get_pull_request(denied.id).unwrap().unwrap().state,
            PrState::Open
        );
        let touched: Vec<_> = log_of(&script)
            .into_iter()
            .map(|step| match step {
                Step::Comment(id, _) | Step::Close(id) | Step::Merge(id) => id,
            })
            .collect();
        assert!(
            touched
                .iter()
                .all(|id| id == "PR_ok" || id == "PR_no" || id == "PR_known")
        );
        assert!(
            log_of(&script)
                .iter()
                .all(|step| !matches!(step, Step::Merge(_)))
        );
        let still = store.get_review_request(request).unwrap().unwrap();
        assert_eq!(still.status, RequestStatus::Active);
    }

    #[test]
    fn closing_ends_an_active_review_request() {
        let (_dir, store) = open_store();
        let pr = seed(&store, "PR_review", 21, PrState::Open);
        let request = seed_request(&store, pr.id);
        let script = script();
        let reports = close_pull_requests(&store, &script, &[pr.id], "").unwrap();
        assert_eq!(reports[0].outcome, CloseOutcome::Closed);
        let saved = store.get_review_request(request).unwrap().unwrap();
        assert_eq!(saved.status, RequestStatus::Closed(CloseReason::PrClosed));
    }

    #[test]
    fn an_already_closed_provider_error_is_recorded_and_not_retried_as_open() {
        let (_dir, store) = open_store();
        let pr = seed(&store, "PR_done", 22, PrState::Open);
        let mut script = script();
        script.already_closed.insert("PR_done".into());
        let reports = close_pull_requests(&store, &script, &[pr.id], "note").unwrap();
        assert_eq!(reports[0].outcome, CloseOutcome::Skipped);
        assert_eq!(reports[0].message.as_deref(), Some("Already closed."));
        assert!(reports[0].comment_posted);
        assert_eq!(
            store.get_pull_request(pr.id).unwrap().unwrap().state,
            PrState::Closed
        );
        assert!(
            store
                .app_closed_at_by_node()
                .unwrap()
                .contains_key("PR_done")
        );
    }

    #[test]
    fn bulk_close_stays_within_the_concurrency_limit() {
        let (_dir, store) = open_store();
        let ids: Vec<PrId> = (1..=8)
            .map(|number| seed(&store, &format!("PR_{number}"), number, PrState::Open).id)
            .collect();
        let mut script = script();
        script.delay = Duration::from_millis(40);
        close_pull_requests(&store, &script, &ids, "").unwrap();
        let peak = script.peak.load(Ordering::SeqCst);
        assert!(
            peak > 1 && peak <= CLOSE_CONCURRENCY,
            "peak {peak}, limit {CLOSE_CONCURRENCY}"
        );
        assert_eq!(
            log_of(&script)
                .iter()
                .filter(|step| matches!(step, Step::Close(_)))
                .count(),
            8
        );
        for id in ids {
            assert_eq!(
                store.get_pull_request(id).unwrap().unwrap().state,
                PrState::Closed
            );
        }
    }

    #[test]
    fn a_stale_open_upsert_does_not_reopen_an_app_close_and_a_newer_one_does() {
        let (_dir, store) = open_store();
        let pr = seed(&store, "PR_lag", 30, PrState::Open);
        let repo_id = store
            .list_repositories(pr.remote.account)
            .unwrap()
            .into_iter()
            .find(|repo| repo.name == "widgets")
            .unwrap()
            .id;
        store.mark_closed_from_app(pr.id, Utc::now()).unwrap();

        let mut stale = store.get_pull_request(pr.id).unwrap().unwrap();
        stale.state = PrState::Open;
        stale.updated_at = at();
        let kept = store.upsert_pull_request(repo_id, stale).unwrap();
        assert_eq!(kept.state, PrState::Closed);
        assert!(kept.updated_at > at());

        let mut reopened = kept.clone();
        reopened.state = PrState::Open;
        reopened.updated_at = Utc::now() + ChronoDuration::hours(1);
        let back = store.upsert_pull_request(repo_id, reopened).unwrap();
        assert_eq!(back.state, PrState::Open);
        assert!(store.app_closed_at_by_node().unwrap().is_empty());
    }

    fn seed_request(store: &Store, pr: PrId) -> ReviewRequestId {
        store
            .insert_review_request(ReviewRequest {
                id: ReviewRequestId(0),
                pr,
                root: ProviderRef {
                    account: AccountId(1),
                    id: "1700000001.000100".into(),
                },
                workspace: "acme".into(),
                team_id: None,
                channel_id: "C0ACME0001".into(),
                channel_name: "reviews".into(),
                root_ts: "1700000001.000100".into(),
                permalink: None,
                requested_at: at(),
                last_bumped_at: None,
                bump_count: 0,
                status: RequestStatus::Active,
                closed_at: None,
                source: RequestSource::App,
            })
            .unwrap()
            .id
    }
}
