//! Approve several pull requests at once (§8.2 Approve pull requests).
//!
//! [`preview`] reads each pull request fresh from the provider and says which
//! ones would be approved and why the others are skipped. [`approve`] then
//! approves exactly the head commit the preview showed: a pull request that
//! got new commits since is not approved. One failure does not stop the
//! others. Nothing here runs from sync or from the MCP server.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::close::{bounded, plain_provider_message};
use crate::domain::{PrId, PrState, PullRequest, VcsKind};
use crate::error::{CoreError, ProviderError};
use crate::providers::VcsProvider;
use crate::store::Store;
use crate::sync::Writer;

/// How many approvals are sent at once. Reviews notify people and count
/// toward GitHub's content-creation limits, so this stays small.
pub const APPROVE_CONCURRENCY: usize = 2;

/// GitHub rejects a review body longer than this.
const MAX_COMMENT_CHARS: usize = 65_536;

/// What the provider says about one pull request right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalProbe {
    /// Provider node id, the same as [`PullRequest::remote`]'s id.
    pub node_id: String,
    pub state: PrState,
    pub is_draft: bool,
    pub head_oid: String,
    pub author_login: String,
    /// State of the viewer's latest review (`APPROVED`, `COMMENTED`, …).
    pub viewer_review_state: Option<String>,
    /// Commit the viewer's latest review was left on.
    pub viewer_review_commit: Option<String>,
}

/// The fresh facts for a batch: who the viewer is, and one probe per pull
/// request the provider still returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovalProbes {
    pub viewer_login: String,
    pub probes: Vec<ApprovalProbe>,
}

/// Why a pull request is left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Merged,
    Closed,
    /// GitHub does not let anyone approve their own pull request.
    OwnPr,
    /// The viewer already approved the current head commit.
    AlreadyApproved,
    /// Only GitHub pull requests can be approved from the app.
    NotGithub,
    /// The provider no longer returns it, or it is not in the app.
    NotFound,
}

impl SkipReason {
    pub fn message(self) -> &'static str {
        match self {
            SkipReason::Merged => "Already merged.",
            SkipReason::Closed => "Closed.",
            SkipReason::OwnPr => "Your own PR. GitHub doesn't let you approve it.",
            SkipReason::AlreadyApproved => "You already approved the latest commit.",
            SkipReason::NotGithub => "Only GitHub pull requests can be approved here.",
            SkipReason::NotFound => "GitHub no longer returns this pull request.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalAction {
    Approve,
    Skip,
}

/// One row of the confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPreview {
    pub id: i64,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub action: ApprovalAction,
    /// Set when `action` is `skip`.
    pub reason: Option<SkipReason>,
    pub message: Option<String>,
    /// The head commit to approve. Empty when skipped.
    pub head_sha: String,
    pub draft: bool,
    /// The viewer approved an older commit; this approves the new head.
    pub approved_earlier: bool,
}

/// What the user confirmed for one pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveTarget {
    pub id: i64,
    /// The head commit shown in the preview.
    pub head_sha: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApproveOutcome {
    Approved,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveReport {
    pub id: i64,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub outcome: ApproveOutcome,
    pub reason: Option<SkipReason>,
    /// Why it was skipped or failed. Empty after a clean approval.
    pub message: Option<String>,
    pub draft: bool,
}

/// Reads `ids` fresh and says which would be approved. Results follow
/// `ids` with duplicates removed. Sends nothing.
pub fn preview(
    store: &Store,
    provider: &dyn VcsProvider,
    ids: &[PrId],
) -> Result<Vec<ApprovalPreview>, CoreError> {
    let (prs, mut missing) = load(store, ids)?;
    if provider.kind() != VcsKind::GitHub {
        return Ok(order(
            ids,
            prs.iter()
                .map(|pr| skipped_preview(pr, SkipReason::NotGithub))
                .chain(missing.drain(..).map(missing_preview))
                .collect(),
            |row| row.id,
        ));
    }
    let probes = provider.approval_probes(&prs).map_err(scope_hint)?;
    let by_node = by_node(&probes);
    let mut rows: Vec<ApprovalPreview> = prs
        .iter()
        .map(|pr| match by_node.get(pr.remote.id.as_str()) {
            None => skipped_preview(pr, SkipReason::NotFound),
            Some(probe) => match decide(&probes.viewer_login, probe) {
                Some(reason) => ApprovalPreview {
                    draft: probe.is_draft,
                    ..skipped_preview(pr, reason)
                },
                None => ApprovalPreview {
                    id: pr.id.0,
                    repo: repo_name(pr),
                    number: pr.number,
                    title: pr.title.clone(),
                    action: ApprovalAction::Approve,
                    reason: None,
                    message: None,
                    head_sha: probe.head_oid.clone(),
                    draft: probe.is_draft,
                    approved_earlier: approved(probe),
                },
            },
        })
        .collect();
    rows.extend(missing.drain(..).map(missing_preview));
    Ok(order(ids, rows, |row| row.id))
}

/// Approves each target's `head_sha` with `comment` as the review body
/// (none when blank). A pull request whose head moved since the preview,
/// or that is now merged, closed, or already approved, is not approved.
/// Approved rows are re-read so the list shows the new review state.
pub fn approve(
    store: &Store,
    provider: &dyn VcsProvider,
    targets: &[ApproveTarget],
    comment: &str,
) -> Result<Vec<ApproveReport>, CoreError> {
    let comment = comment.trim();
    if comment.chars().count() > MAX_COMMENT_CHARS {
        return Err(CoreError::Validation(
            "Comment is too long. Shorten it and try again.".into(),
        ));
    }
    let mut seen = HashSet::new();
    let targets: Vec<&ApproveTarget> = targets.iter().filter(|t| seen.insert(t.id)).collect();
    let ids: Vec<PrId> = targets.iter().map(|t| PrId(t.id)).collect();
    let (prs, missing) = load(store, &ids)?;
    let mut reports: Vec<ApproveReport> = missing
        .into_iter()
        .map(|id| ApproveReport {
            id: id.0,
            repo: String::new(),
            number: 0,
            title: String::new(),
            outcome: ApproveOutcome::Failed,
            reason: Some(SkipReason::NotFound),
            message: Some("This pull request is not in the app.".into()),
            draft: false,
        })
        .collect();
    if provider.kind() != VcsKind::GitHub {
        reports.extend(
            prs.iter()
                .map(|pr| skipped_report(pr, SkipReason::NotGithub, false)),
        );
        return Ok(order(&ids, reports, |row| row.id));
    }
    let probes = provider.approval_probes(&prs).map_err(scope_hint)?;
    let by_node = by_node(&probes);
    let expected: HashMap<i64, &str> = targets
        .iter()
        .map(|t| (t.id, t.head_sha.as_str()))
        .collect();

    let mut work = Vec::new();
    for pr in &prs {
        let Some(probe) = by_node.get(pr.remote.id.as_str()) else {
            reports.push(skipped_report(pr, SkipReason::NotFound, false));
            continue;
        };
        if let Some(reason) = decide(&probes.viewer_login, probe) {
            reports.push(skipped_report(pr, reason, probe.is_draft));
            continue;
        }
        let want = expected.get(&pr.id.0).copied().unwrap_or("");
        if want.is_empty() || probe.head_oid != want {
            reports.push(report(
                pr,
                ApproveOutcome::Failed,
                None,
                Some(
                    "New commits were pushed after you checked. Review them, then approve again."
                        .into(),
                ),
                probe.is_draft,
            ));
            continue;
        }
        work.push((pr, probe.is_draft, want.to_owned()));
    }

    let done = bounded(
        &work,
        APPROVE_CONCURRENCY,
        |(pr, draft, head)| match provider.approve_pull_request(pr, head, comment) {
            Ok(()) => report(pr, ApproveOutcome::Approved, None, None, *draft),
            Err(err) => report(
                pr,
                ApproveOutcome::Failed,
                None,
                Some(failure_message(&err, *draft)),
                *draft,
            ),
        },
    );
    let approved: Vec<&PullRequest> = done
        .iter()
        .filter(|r| r.outcome == ApproveOutcome::Approved)
        .filter_map(|r| prs.iter().find(|pr| pr.id.0 == r.id))
        .collect();
    refresh_rows(store, provider, &approved);
    reports.extend(done);
    Ok(order(&ids, reports, |row| row.id))
}

/// `None` when the viewer can approve `probe` now.
fn decide(viewer: &str, probe: &ApprovalProbe) -> Option<SkipReason> {
    match probe.state {
        PrState::Merged => return Some(SkipReason::Merged),
        PrState::Closed => return Some(SkipReason::Closed),
        PrState::Open => {}
    }
    if !viewer.is_empty() && probe.author_login.eq_ignore_ascii_case(viewer) {
        return Some(SkipReason::OwnPr);
    }
    if approved(probe) && probe.viewer_review_commit.as_deref() == Some(probe.head_oid.as_str()) {
        return Some(SkipReason::AlreadyApproved);
    }
    None
}

fn approved(probe: &ApprovalProbe) -> bool {
    probe
        .viewer_review_state
        .as_deref()
        .is_some_and(|state| state.eq_ignore_ascii_case("APPROVED"))
}

/// A token without write access gets a message that says what to change.
fn failure_message(err: &ProviderError, draft: bool) -> String {
    let text = plain_provider_message(err);
    let lower = text.to_ascii_lowercase();
    if lower.contains("not accessible")
        || lower.contains("permission")
        || matches!(err, ProviderError::Forbidden(_))
    {
        return token_message().into();
    }
    if draft && lower.contains("draft") {
        return format!("GitHub doesn't allow approving this draft: {text}");
    }
    text
}

fn token_message() -> &'static str {
    "GitHub refused the approval. Your token needs write access to pull requests: the repo scope for a classic token, or Pull requests set to Read and write for a fine-grained token."
}

/// A probe that fails for lack of access says what the token needs.
fn scope_hint(err: ProviderError) -> CoreError {
    let lower = plain_provider_message(&err).to_ascii_lowercase();
    if lower.contains("not accessible") {
        return CoreError::Provider(ProviderError::Forbidden(token_message().into()));
    }
    CoreError::Provider(err)
}

/// Best effort: a failed re-read must not turn an approval into an error.
/// The next sync catches up either way.
fn refresh_rows(store: &Store, provider: &dyn VcsProvider, prs: &[&PullRequest]) {
    let Some(first) = prs.first() else {
        return;
    };
    let refs: Vec<_> = prs.iter().map(|pr| pr.remote.clone()).collect();
    let Ok(fresh) = provider.refresh_pull_requests(&refs) else {
        return;
    };
    let existing: Vec<PullRequest> = prs.iter().map(|pr| (*pr).clone()).collect();
    let Ok(mut writer) = Writer::new(store, first.remote.account, Utc::now(), &existing) else {
        return;
    };
    for mut item in fresh {
        if let Some(prev) = prs
            .iter()
            .find(|pr| pr.remote.id == item.pull_request.remote.id)
        {
            item.pull_request.involvement = prev.involvement;
        }
        let _ = writer.write(&item);
    }
}

type Loaded = (Vec<PullRequest>, Vec<PrId>);

fn load(store: &Store, ids: &[PrId]) -> Result<Loaded, CoreError> {
    let mut seen = HashSet::new();
    let mut prs = Vec::new();
    let mut missing = Vec::new();
    for id in ids.iter().filter(|id| seen.insert(**id)) {
        match store.get_pull_request(*id)? {
            Some(pr) => prs.push(pr),
            None => missing.push(*id),
        }
    }
    Ok((prs, missing))
}

fn by_node(probes: &ApprovalProbes) -> HashMap<&str, &ApprovalProbe> {
    probes
        .probes
        .iter()
        .map(|probe| (probe.node_id.as_str(), probe))
        .collect()
}

fn order<T>(ids: &[PrId], rows: Vec<T>, id_of: impl Fn(&T) -> i64) -> Vec<T> {
    let mut by_id: HashMap<i64, T> = rows.into_iter().map(|row| (id_of(&row), row)).collect();
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(**id))
        .filter_map(|id| by_id.remove(&id.0))
        .collect()
}

fn repo_name(pr: &PullRequest) -> String {
    format!("{}/{}", pr.repo.owner, pr.repo.name)
}

fn skipped_preview(pr: &PullRequest, reason: SkipReason) -> ApprovalPreview {
    ApprovalPreview {
        id: pr.id.0,
        repo: repo_name(pr),
        number: pr.number,
        title: pr.title.clone(),
        action: ApprovalAction::Skip,
        reason: Some(reason),
        message: Some(reason.message().into()),
        head_sha: String::new(),
        draft: pr.is_draft,
        approved_earlier: false,
    }
}

fn missing_preview(id: PrId) -> ApprovalPreview {
    ApprovalPreview {
        id: id.0,
        repo: String::new(),
        number: 0,
        title: String::new(),
        action: ApprovalAction::Skip,
        reason: Some(SkipReason::NotFound),
        message: Some("This pull request is not in the app.".into()),
        head_sha: String::new(),
        draft: false,
        approved_earlier: false,
    }
}

fn skipped_report(pr: &PullRequest, reason: SkipReason, draft: bool) -> ApproveReport {
    report(
        pr,
        ApproveOutcome::Skipped,
        Some(reason),
        Some(reason.message().into()),
        draft,
    )
}

fn report(
    pr: &PullRequest,
    outcome: ApproveOutcome,
    reason: Option<SkipReason>,
    message: Option<String>,
    draft: bool,
) -> ApproveReport {
    ApproveReport {
        id: pr.id.0,
        repo: repo_name(pr),
        number: pr.number,
        title: pr.title.clone(),
        outcome,
        reason,
        message,
        draft,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use chrono::TimeZone;

    use crate::domain::{
        Account, AccountId, AccountKind, AuthMode, ChecksState, Involvement, MergeStateStatus,
        Mergeable, ProviderRef, RepoRef, Repository, ReviewDecision, TeamRef,
    };
    use crate::providers::{
        OwnershipInputs, OwnershipProbe, PrQuery, RateBudget, SyncedPullRequest, TeamMembers,
        VcsUser,
    };

    use super::*;

    struct Script {
        kind: VcsKind,
        probes: Mutex<HashMap<String, ApprovalProbe>>,
        probe_calls: AtomicUsize,
        approvals: Mutex<Vec<(String, String, String)>>,
        fail: HashMap<String, String>,
        refreshed: Mutex<Vec<String>>,
        numbers: HashMap<String, u64>,
        delay: Duration,
        inflight: AtomicUsize,
        peak: AtomicUsize,
    }

    fn script() -> Script {
        Script {
            kind: VcsKind::GitHub,
            probes: Mutex::new(HashMap::new()),
            probe_calls: AtomicUsize::new(0),
            approvals: Mutex::new(Vec::new()),
            fail: HashMap::new(),
            refreshed: Mutex::new(Vec::new()),
            numbers: HashMap::new(),
            delay: Duration::ZERO,
            inflight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    impl Script {
        fn set(&self, probe: ApprovalProbe) {
            self.probes
                .lock()
                .unwrap()
                .insert(probe.node_id.clone(), probe);
        }
        fn approvals(&self) -> Vec<(String, String, String)> {
            self.approvals.lock().unwrap().clone()
        }
    }

    fn unused<T>() -> Result<T, ProviderError> {
        Err(ProviderError::Unsupported("not used".into()))
    }

    impl VcsProvider for Script {
        fn kind(&self) -> VcsKind {
            self.kind
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
            ids: &[ProviderRef],
        ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
            let mut refreshed = self.refreshed.lock().unwrap();
            Ok(ids
                .iter()
                .map(|remote| {
                    refreshed.push(remote.id.clone());
                    let mut pr = row(&remote.id, 1, PrState::Open, "grace");
                    pr.remote = remote.clone();
                    pr.number = self.numbers.get(&remote.id).copied().unwrap_or(0);
                    pr.review_decision = Some(ReviewDecision::Approved);
                    pr.involvement = Involvement::empty();
                    SyncedPullRequest {
                        pull_request: pr,
                        repo_node_id: Some("R_widgets".into()),
                        default_branch: Some("main".into()),
                        repo_archived: false,
                        merge_queue: None,
                        failing_checks: None,
                        review_clock: None,
                    }
                })
                .collect())
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
        fn approval_probes(&self, prs: &[PullRequest]) -> Result<ApprovalProbes, ProviderError> {
            self.probe_calls.fetch_add(1, Ordering::SeqCst);
            let probes = self.probes.lock().unwrap();
            Ok(ApprovalProbes {
                viewer_login: "Ada".into(),
                probes: prs
                    .iter()
                    .filter_map(|pr| probes.get(&pr.remote.id).cloned())
                    .collect(),
            })
        }
        fn approve_pull_request(
            &self,
            pr: &PullRequest,
            head_oid: &str,
            body: &str,
        ) -> Result<(), ProviderError> {
            let now = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            if !self.delay.is_zero() {
                thread::sleep(self.delay);
            }
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            if let Some(message) = self.fail.get(&pr.remote.id) {
                return Err(ProviderError::Api {
                    code: "graphql".into(),
                    message: message.clone(),
                });
            }
            self.approvals.lock().unwrap().push((
                pr.remote.id.clone(),
                head_oid.to_owned(),
                body.to_owned(),
            ));
            Ok(())
        }
    }

    fn at() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
    }

    fn row(node: &str, number: u64, state: PrState, author: &str) -> PullRequest {
        PullRequest {
            id: PrId(0),
            remote: ProviderRef {
                account: AccountId(0),
                id: node.into(),
            },
            repo: RepoRef {
                host: "github.com".into(),
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number,
            title: format!("Bump lib {number}"),
            url: format!("https://github.com/acme/widgets/pull/{number}"),
            author_login: author.into(),
            body: String::new(),
            state,
            is_draft: false,
            head_ref: "deps/bump".into(),
            base_ref: "main".into(),
            head_sha: "sha-old".into(),
            labels: Vec::new(),
            review_decision: None,
            checks: ChecksState::Success,
            mergeable: Mergeable::Mergeable,
            merge_state: MergeStateStatus::Clean,
            requested_reviewers: Vec::new(),
            assignees: Vec::new(),
            unresolved_threads: 0,
            total_threads: 0,
            involvement: Involvement::REVIEW_REQUESTED,
            created_at: at(),
            updated_at: at(),
            last_activity_at: at(),
            synced_at: at(),
            hidden: false,
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        account: AccountId,
        repo_id: i64,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("approve.db")).unwrap();
        let account = store
            .insert_account(Account {
                id: AccountId(0),
                kind: AccountKind::GitHub,
                display_name: "ada".into(),
                host: "github.com".into(),
                external_id: "gh-ada".into(),
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
                node_id: Some("R_widgets".into()),
                default_branch: Some("main".into()),
                archived: false,
            })
            .unwrap();
        Fixture {
            _dir: dir,
            store,
            account: account.id,
            repo_id: repo.id,
        }
    }

    impl Fixture {
        fn seed(&self, node: &str, number: u64) -> PullRequest {
            let mut pr = row(node, number, PrState::Open, "grace");
            pr.remote.account = self.account;
            self.store.upsert_pull_request(self.repo_id, pr).unwrap()
        }
    }

    fn probe(node: &str, author: &str) -> ApprovalProbe {
        ApprovalProbe {
            node_id: node.into(),
            state: PrState::Open,
            is_draft: false,
            head_oid: format!("head-{node}"),
            author_login: author.into(),
            viewer_review_state: None,
            viewer_review_commit: None,
        }
    }

    #[test]
    fn preview_says_which_are_approved_and_why_others_are_skipped() {
        let fx = fixture();
        let open = fx.seed("PR_open", 1);
        let own = fx.seed("PR_own", 2);
        let merged = fx.seed("PR_merged", 3);
        let closed = fx.seed("PR_closed", 4);
        let done = fx.seed("PR_done", 5);
        let earlier = fx.seed("PR_earlier", 6);
        let draft = fx.seed("PR_draft", 7);
        let gone = fx.seed("PR_gone", 8);
        let script = script();
        script.set(probe("PR_open", "grace"));
        script.set(probe("PR_own", "ada"));
        script.set(ApprovalProbe {
            state: PrState::Merged,
            ..probe("PR_merged", "grace")
        });
        script.set(ApprovalProbe {
            state: PrState::Closed,
            ..probe("PR_closed", "grace")
        });
        script.set(ApprovalProbe {
            viewer_review_state: Some("APPROVED".into()),
            viewer_review_commit: Some("head-PR_done".into()),
            ..probe("PR_done", "grace")
        });
        script.set(ApprovalProbe {
            viewer_review_state: Some("APPROVED".into()),
            viewer_review_commit: Some("older".into()),
            ..probe("PR_earlier", "grace")
        });
        script.set(ApprovalProbe {
            is_draft: true,
            ..probe("PR_draft", "grace")
        });

        let ids = [
            open.id,
            own.id,
            merged.id,
            closed.id,
            done.id,
            earlier.id,
            draft.id,
            gone.id,
            open.id,
            PrId(9999),
        ];
        let rows = preview(&fx.store, &script, &ids).unwrap();
        let summary: Vec<_> = rows
            .iter()
            .map(|row| (row.id, row.action, row.reason, row.head_sha.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (open.id.0, ApprovalAction::Approve, None, "head-PR_open"),
                (own.id.0, ApprovalAction::Skip, Some(SkipReason::OwnPr), ""),
                (
                    merged.id.0,
                    ApprovalAction::Skip,
                    Some(SkipReason::Merged),
                    ""
                ),
                (
                    closed.id.0,
                    ApprovalAction::Skip,
                    Some(SkipReason::Closed),
                    ""
                ),
                (
                    done.id.0,
                    ApprovalAction::Skip,
                    Some(SkipReason::AlreadyApproved),
                    ""
                ),
                (
                    earlier.id.0,
                    ApprovalAction::Approve,
                    None,
                    "head-PR_earlier"
                ),
                (draft.id.0, ApprovalAction::Approve, None, "head-PR_draft"),
                (
                    gone.id.0,
                    ApprovalAction::Skip,
                    Some(SkipReason::NotFound),
                    ""
                ),
                (9999, ApprovalAction::Skip, Some(SkipReason::NotFound), ""),
            ]
        );
        assert!(rows[5].approved_earlier);
        assert!(rows[6].draft);
        assert_eq!(rows[0].repo, "acme/widgets");
        assert_eq!(
            rows[1].message.as_deref(),
            Some("Your own PR. GitHub doesn't let you approve it.")
        );
        assert_eq!(script.probe_calls.load(Ordering::SeqCst), 1);
        assert!(script.approvals().is_empty());
    }

    #[test]
    fn approve_sends_the_previewed_head_and_keeps_going_after_a_failure() {
        let fx = fixture();
        let a = fx.seed("PR_a", 1);
        let b = fx.seed("PR_b", 2);
        let moved = fx.seed("PR_moved", 3);
        let denied = fx.seed("PR_denied", 4);
        let mut script = script();
        for node in ["PR_a", "PR_b", "PR_moved", "PR_denied"] {
            script.set(probe(node, "grace"));
        }
        script.fail.insert("PR_b".into(), "Something broke".into());
        script.fail.insert(
            "PR_denied".into(),
            "Resource not accessible by personal access token".into(),
        );
        let targets = [
            ApproveTarget {
                id: a.id.0,
                head_sha: "head-PR_a".into(),
            },
            ApproveTarget {
                id: b.id.0,
                head_sha: "head-PR_b".into(),
            },
            ApproveTarget {
                id: moved.id.0,
                head_sha: "seen-before-push".into(),
            },
            ApproveTarget {
                id: denied.id.0,
                head_sha: "head-PR_denied".into(),
            },
        ];
        let reports = approve(&fx.store, &script, &targets, "  LGTM \n").unwrap();
        let outcomes: Vec<_> = reports.iter().map(|r| (r.id, r.outcome)).collect();
        assert_eq!(
            outcomes,
            vec![
                (a.id.0, ApproveOutcome::Approved),
                (b.id.0, ApproveOutcome::Failed),
                (moved.id.0, ApproveOutcome::Failed),
                (denied.id.0, ApproveOutcome::Failed),
            ]
        );
        assert_eq!(
            script.approvals(),
            vec![("PR_a".into(), "head-PR_a".into(), "LGTM".into())]
        );
        assert_eq!(reports[1].message.as_deref(), Some("Something broke"));
        assert!(
            reports[2]
                .message
                .as_deref()
                .unwrap()
                .starts_with("New commits were pushed")
        );
        assert!(
            reports[3]
                .message
                .as_deref()
                .unwrap()
                .contains("Pull requests set to Read and write")
        );
    }

    #[test]
    fn approve_skips_what_changed_since_the_preview_and_refreshes_approved_rows() {
        let fx = fixture();
        let ok = fx.seed("PR_ok", 1);
        let merged = fx.seed("PR_merged", 2);
        let done = fx.seed("PR_done", 3);
        let mut script = script();
        script.numbers.insert("PR_ok".into(), 1);
        script.set(probe("PR_ok", "grace"));
        script.set(ApprovalProbe {
            state: PrState::Merged,
            ..probe("PR_merged", "grace")
        });
        script.set(ApprovalProbe {
            viewer_review_state: Some("APPROVED".into()),
            viewer_review_commit: Some("head-PR_done".into()),
            ..probe("PR_done", "grace")
        });
        let targets: Vec<ApproveTarget> =
            [(&ok, "PR_ok"), (&merged, "PR_merged"), (&done, "PR_done")]
                .iter()
                .map(|(pr, node)| ApproveTarget {
                    id: pr.id.0,
                    head_sha: format!("head-{node}"),
                })
                .collect();
        let reports = approve(&fx.store, &script, &targets, "").unwrap();
        assert_eq!(reports[0].outcome, ApproveOutcome::Approved);
        assert_eq!(reports[1].outcome, ApproveOutcome::Skipped);
        assert_eq!(reports[1].reason, Some(SkipReason::Merged));
        assert_eq!(reports[2].outcome, ApproveOutcome::Skipped);
        assert_eq!(reports[2].reason, Some(SkipReason::AlreadyApproved));
        assert_eq!(
            script.approvals(),
            vec![("PR_ok".into(), "head-PR_ok".into(), String::new())]
        );
        assert_eq!(*script.refreshed.lock().unwrap(), vec!["PR_ok".to_owned()]);
        let stored = fx.store.get_pull_request(ok.id).unwrap().unwrap();
        assert_eq!(stored.review_decision, Some(ReviewDecision::Approved));
        assert_eq!(stored.involvement, Involvement::REVIEW_REQUESTED);
        let untouched = fx.store.get_pull_request(merged.id).unwrap().unwrap();
        assert_eq!(untouched.review_decision, None);
    }

    #[test]
    fn a_non_github_provider_skips_everything_without_a_request() {
        let fx = fixture();
        let pr = fx.seed("PR_x", 1);
        let mut script = script();
        script.kind = VcsKind::GitLab;
        script.set(probe("PR_x", "grace"));
        let rows = preview(&fx.store, &script, &[pr.id]).unwrap();
        assert_eq!(rows[0].reason, Some(SkipReason::NotGithub));
        let reports = approve(
            &fx.store,
            &script,
            &[ApproveTarget {
                id: pr.id.0,
                head_sha: "head-PR_x".into(),
            }],
            "",
        )
        .unwrap();
        assert_eq!(reports[0].outcome, ApproveOutcome::Skipped);
        assert_eq!(script.probe_calls.load(Ordering::SeqCst), 0);
        assert!(script.approvals().is_empty());
    }

    #[test]
    fn approvals_run_at_most_two_at_a_time() {
        let fx = fixture();
        let mut script = script();
        script.delay = Duration::from_millis(30);
        let mut targets = Vec::new();
        for n in 1..=6 {
            let node = format!("PR_{n}");
            let pr = fx.seed(&node, n);
            script.set(probe(&node, "grace"));
            targets.push(ApproveTarget {
                id: pr.id.0,
                head_sha: format!("head-{node}"),
            });
        }
        let reports = approve(&fx.store, &script, &targets, "").unwrap();
        assert!(
            reports
                .iter()
                .all(|r| r.outcome == ApproveOutcome::Approved)
        );
        assert_eq!(script.peak.load(Ordering::SeqCst), APPROVE_CONCURRENCY);
    }

    #[test]
    fn a_too_long_comment_is_refused_before_any_request() {
        let fx = fixture();
        let pr = fx.seed("PR_long", 1);
        let script = script();
        script.set(probe("PR_long", "grace"));
        let long = "x".repeat(MAX_COMMENT_CHARS + 1);
        let err = approve(
            &fx.store,
            &script,
            &[ApproveTarget {
                id: pr.id.0,
                head_sha: "head-PR_long".into(),
            }],
            &long,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)));
        assert_eq!(script.probe_calls.load(Ordering::SeqCst), 0);
    }
}
