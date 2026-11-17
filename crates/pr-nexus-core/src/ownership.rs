//! Ownership for one pull request: fetch, cache, and decide (§6, §7.2).
//!
//! [`resolve`] probes every time: the probe is one GraphQL call and returns
//! the cache key and the reviews, which change without the key changing.
//! Files, rules, and protection are fetched only when
//! `(head_sha, base_ref, codeowners oid)` differs from the cached row.
//! Team members are cached for 24 hours and fetched at most 4 at a time.
//!
//! [`report_from_cache`] skips the probe when a previous resolve stored one
//! for the same head and base. Callers that use it revalidate with
//! [`resolve`] in the background. [`build_report`] is pure.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::{
    OwnershipCacheEntry, PullRequest, Readiness, Reviewer, TeamMembersCacheEntry, TeamRef, Tri,
};
use crate::error::{CoreError, ProviderError};
use crate::providers::{ClassicProtection, OwnershipInputs, OwnershipProbe, VcsProvider};
use crate::review::codeowners::{parse_codeowners, resolve_ownership};
use crate::review::groups::{self, MarkedGroup};
use crate::review::owners::{
    DecideOptions, GivenReadiness, Membership, OwnerCatalog, OwnerDecision, OwnerQuery,
    ReadinessWord, decide_owners,
};
use crate::review::readiness::{
    CODE_OWNER_REVIEW_UNKNOWN, Evidence, ReadinessSnapshot, ReviewSnapshot, decide_readiness_with,
    snapshot_from_pr,
};
use crate::store::Store;

/// How long a team's member list is trusted (§6).
pub const TEAM_MEMBERS_TTL_HOURS: i64 = 24;

/// Teams looked up per resolution. More candidates than this stay unverified.
pub const MAX_MEMBERSHIP_LOOKUPS: usize = 20;

/// Team-member requests in flight at once. The GitHub client also gates each
/// host at 4 (SPEC §6), so this does not exceed that.
pub const MEMBER_LOOKUPS_IN_FLIGHT: usize = 4;

/// What `ownership_cache.decision_json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedOwnership {
    pub inputs: OwnershipInputs,
    /// From the last resolution. The list reads it while the head and base match.
    pub code_owner_review_pending: Option<bool>,
    /// The probe from that resolution, so a later open can skip the network
    /// while the head and base still match. Absent on rows written before it
    /// was stored.
    #[serde(default)]
    pub probe: Option<OwnershipProbe>,
}

impl CachedOwnership {
    /// The cached facts for `pr`, when the row still matches its head and base.
    pub fn for_pr(entry: &OwnershipCacheEntry, pr: &PullRequest) -> Option<Self> {
        if entry.head_sha != pr.head_sha || entry.base_ref != pr.base_ref {
            return None;
        }
        serde_json::from_str(&entry.decision_json).ok()
    }

    /// Applies the cached facts to a list-row snapshot.
    pub fn apply(&self, snapshot: &mut ReadinessSnapshot) {
        snapshot.files_truncated = self.inputs.files_truncated;
        snapshot.code_owner_review_pending = self.code_owner_review_pending;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathOwners {
    pub path: String,
    pub teams: Vec<TeamRef>,
}

/// Everything the detail view shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OwnershipReport {
    pub readiness: Readiness,
    pub decision: OwnerDecision,
    pub head_sha: String,
    pub base_ref: String,
    pub codeowners_path: Option<String>,
    pub codeowners_error: Option<String>,
    pub files: Vec<PathOwners>,
    pub files_truncated: bool,
    pub rules_error: Option<String>,
    pub protection: ClassicProtection,
    pub code_owner_review_pending: Option<bool>,
    /// Files, rules, and protection came from the cache.
    pub from_cache: bool,
    /// Every distinct CODEOWNERS owner set on the changed files, including
    /// sets an approval already satisfied. Empty when CODEOWNERS did not load.
    pub owner_groups: Vec<MarkedGroup>,
}

/// Owner catalog from the CODEOWNERS text and the changed files.
pub fn catalog(probe: &OwnershipProbe, files: &[String]) -> OwnerCatalog {
    let Some(blob) = &probe.codeowners else {
        return OwnerCatalog {
            source: Some("github".into()),
            codeowners_file: None,
            teams: Vec::new(),
            exclusive_teams: Vec::new(),
            coverage_teams: Vec::new(),
            optional_teams: Vec::new(),
            users: Vec::new(),
            path_owners: files
                .iter()
                .map(|path| (path.clone(), Vec::new()))
                .collect(),
            error: Some(format!("no CODEOWNERS file on {}", probe.base_ref)),
        };
    };
    match parse_codeowners(&blob.text) {
        Ok(rules) => {
            let owned = resolve_ownership(&rules, files);
            OwnerCatalog {
                source: Some("github".into()),
                codeowners_file: Some(blob.path.clone()),
                teams: owned.teams,
                exclusive_teams: owned.exclusive,
                coverage_teams: owned.coverage,
                optional_teams: owned.optional,
                users: owned.users_ignored_for_slack,
                path_owners: owned.path_owners,
                error: None,
            }
        }
        Err(err) => OwnerCatalog {
            source: Some("github".into()),
            codeowners_file: Some(blob.path.clone()),
            teams: Vec::new(),
            exclusive_teams: Vec::new(),
            coverage_teams: Vec::new(),
            optional_teams: Vec::new(),
            users: Vec::new(),
            path_owners: files
                .iter()
                .map(|path| (path.clone(), Vec::new()))
                .collect(),
            error: Some(err.to_string()),
        },
    }
}

/// Candidate teams whose membership matters: some human approval is not
/// already attributed to them with `onBehalfOf`.
pub fn teams_needing_membership(
    pr: &PullRequest,
    probe: &OwnershipProbe,
    catalog: &OwnerCatalog,
) -> Vec<TeamRef> {
    let author = pr.author_login.to_ascii_lowercase();
    let approvals: Vec<_> = probe
        .reviews
        .reviews
        .iter()
        .filter(|review| {
            review.author_typename == "User"
                && review.state.eq_ignore_ascii_case("APPROVED")
                && review.author_login.to_ascii_lowercase() != author
        })
        .collect();
    let mut teams = Vec::new();
    for team in catalog.coverage_teams.iter().chain(&catalog.teams) {
        if teams.contains(team) {
            continue;
        }
        let unattributed = approvals.iter().any(|review| {
            !review.on_behalf_of.iter().any(|behalf| {
                behalf.org.eq_ignore_ascii_case(&team.org)
                    && behalf.slug.eq_ignore_ascii_case(&team.slug)
            })
        });
        if unattributed {
            teams.push(team.clone());
        }
    }
    teams
}

/// Readiness, owners, and the rationale for one pull request. Pure.
///
/// `blocked_review_required_is_ready` is the §7.12 setting.
pub fn build_report(
    pr: &PullRequest,
    probe: &OwnershipProbe,
    inputs: &OwnershipInputs,
    membership: &BTreeMap<TeamRef, Membership>,
    blocked_review_required_is_ready: bool,
    from_cache: bool,
) -> OwnershipReport {
    let catalog = catalog(probe, &inputs.files);
    let mut snapshot = snapshot_from_pr(pr);
    snapshot.reviews = Some(Evidence {
        complete: probe.reviews.complete,
        error: (!probe.reviews.complete)
            .then(|| "more than 100 effective reviews; review evidence is incomplete".into()),
        items: probe
            .reviews
            .reviews
            .iter()
            .map(|review| ReviewSnapshot {
                state: review.state.clone(),
                author_login: review.author_login.clone(),
                author_is_human: review.author_typename == "User",
            })
            .collect(),
    });
    snapshot.files_truncated = inputs.files_truncated;
    snapshot.snapshot_changed = probe.head_sha != pr.head_sha || probe.base_ref != pr.base_ref;
    snapshot.head_sha = Some(probe.head_sha.clone());

    // Attribution does not depend on readiness, so a first pass finds the
    // pending code-owner review that the final readiness needs.
    let provisional = decide_readiness_with(&snapshot, blocked_review_required_is_ready);
    let first = decide(pr, probe, inputs, &catalog, membership, &provisional);
    let pending = code_owner_review_pending(&first, &catalog);
    snapshot.code_owner_review_pending = pending;
    let readiness = decide_readiness_with(&snapshot, blocked_review_required_is_ready);
    let decision = decide(pr, probe, inputs, &catalog, membership, &readiness);
    let owner_groups = marked_owner_groups(pr, probe, &inputs.files, &decision.approved_teams);

    OwnershipReport {
        readiness,
        decision,
        head_sha: probe.head_sha.clone(),
        base_ref: probe.base_ref.clone(),
        codeowners_path: catalog.codeowners_file.clone(),
        codeowners_error: catalog.error.clone(),
        files: catalog
            .path_owners
            .iter()
            .map(|(path, teams)| PathOwners {
                path: path.clone(),
                teams: teams.clone(),
            })
            .collect(),
        files_truncated: inputs.files_truncated,
        rules_error: inputs.rules_error.clone(),
        protection: inputs.protection.clone(),
        code_owner_review_pending: pending,
        from_cache,
        owner_groups,
    }
}

/// Matched CODEOWNERS groups, each flagged when an approval already covers it.
fn marked_owner_groups(
    pr: &PullRequest,
    probe: &OwnershipProbe,
    files: &[String],
    approved_teams: &[TeamRef],
) -> Vec<MarkedGroup> {
    let Some(blob) = &probe.codeowners else {
        return Vec::new();
    };
    let Ok(rules) = parse_codeowners(&blob.text) else {
        return Vec::new();
    };
    let matched = groups::matched_groups(&rules, files);
    let logins = approved_user_logins(pr, probe);
    let users: Vec<&str> = logins.iter().map(String::as_str).collect();
    groups::mark_groups(&matched, approved_teams, &users)
}

/// Logins of human approvers other than the pull request author.
pub fn approved_user_logins(pr: &PullRequest, probe: &OwnershipProbe) -> Vec<String> {
    let author = pr.author_login.to_ascii_lowercase();
    let mut logins = Vec::new();
    for review in &probe.reviews.reviews {
        if review.author_typename != "User"
            || !review.state.eq_ignore_ascii_case("APPROVED")
            || review.author_login.is_empty()
            || review.author_login.to_ascii_lowercase() == author
        {
            continue;
        }
        if !logins
            .iter()
            .any(|login: &String| login.eq_ignore_ascii_case(&review.author_login))
        {
            logins.push(review.author_login.clone());
        }
    }
    logins
}

fn decide(
    pr: &PullRequest,
    probe: &OwnershipProbe,
    inputs: &OwnershipInputs,
    catalog: &OwnerCatalog,
    membership: &BTreeMap<TeamRef, Membership>,
    readiness: &Readiness,
) -> OwnerDecision {
    let (status, blocking_reasons, unknown_reasons) = match readiness {
        Readiness::Ready { .. } => (ReadinessWord::Ready, Vec::new(), Vec::new()),
        Readiness::Blocked { reasons } => (ReadinessWord::Blocked, reasons.clone(), Vec::new()),
        Readiness::Unknown { reasons } => (ReadinessWord::Unknown, Vec::new(), reasons.clone()),
    };
    // §6: a 403, or a 404 the viewer cannot interpret, is Unknown and never False.
    // `protection_present` stays true so `codeowners_required` cannot infer absence.
    let (protection_fetched, protection_present, require_code_owner_reviews) =
        match &inputs.protection {
            ClassicProtection::Present {
                require_code_owner_reviews,
            } => (true, true, Some(*require_code_owner_reviews)),
            ClassicProtection::Absent => (true, false, None),
            ClassicProtection::Unknown { .. } => (false, true, None),
        };
    let query = OwnerQuery {
        readiness: Some(GivenReadiness {
            status,
            head_sha: Some(probe.head_sha.clone()),
            blocking_reasons,
            unknown_reasons,
        }),
        head_sha: Some(probe.head_sha.clone()),
        reviews: Some(probe.reviews.clone()),
        protection_fetched,
        protection_present,
        require_code_owner_reviews,
        ruleset_requires_code_owners: inputs.rules_require_code_owner,
        requested_teams: pr
            .requested_reviewers
            .iter()
            .filter_map(|request| match &request.reviewer {
                Reviewer::Team(team) => Some(team.to_string()),
                Reviewer::User { .. } => None,
            })
            .collect(),
        author: Some(pr.author_login.clone()),
        owner: Some(pr.repo.owner.clone()),
        files_truncated: inputs.files_truncated,
    };
    decide_owners(
        &query,
        catalog,
        &DecideOptions {
            membership: Some(membership),
            ..DecideOptions::default()
        },
    )
}

/// Whether a code-owner team still has to approve.
///
/// `None` when the requirement is unknown, or when the only teams left are
/// ones whose approval could not be attributed.
pub fn code_owner_review_pending(decision: &OwnerDecision, catalog: &OwnerCatalog) -> Option<bool> {
    match decision.codeowners_required {
        Tri::False => Some(false),
        Tri::Unknown => None,
        Tri::True => {
            let uncertain: Vec<&TeamRef> = decision
                .review_attribution_unknown
                .iter()
                .map(|gap| &gap.team)
                .collect();
            let left: Vec<&TeamRef> = catalog
                .coverage_teams
                .iter()
                .filter(|team| !decision.approved_teams.contains(team))
                .collect();
            if left.iter().any(|team| !uncertain.contains(team)) {
                Some(true)
            } else if left.is_empty() {
                Some(false)
            } else {
                None
            }
        }
    }
}

/// Most pull requests resolved after one sync. Each costs about four calls.
pub const MAX_RESOLVES_PER_SYNC: usize = 10;

/// List-row readiness: the synced fields plus the cached ownership facts when
/// they still match the pull request's head and base.
pub fn cached_readiness(
    store: &Store,
    pr: &PullRequest,
    blocked_review_required_is_ready: bool,
) -> Result<Readiness, CoreError> {
    let entry = store.get_ownership_cache(pr.id)?;
    Ok(cached_readiness_with(
        entry.as_ref(),
        pr,
        blocked_review_required_is_ready,
    ))
}

/// [`cached_readiness`] with the cache entry already loaded, for bulk listing.
pub fn cached_readiness_with(
    entry: Option<&OwnershipCacheEntry>,
    pr: &PullRequest,
    blocked_review_required_is_ready: bool,
) -> Readiness {
    let mut snapshot = snapshot_from_pr(pr);
    if let Some(cached) = entry.and_then(|entry| CachedOwnership::for_pr(entry, pr)) {
        cached.apply(&mut snapshot);
    }
    decide_readiness_with(&snapshot, blocked_review_required_is_ready)
}

/// Resolves ownership for open pull requests whose list readiness hinges on an
/// unknown code-owner review, at most `limit` of them.
///
/// Rate limits and lost auth stop the pass. Other failures skip the pull request.
pub fn resolve_unknown_code_owner_reviews(
    store: &Store,
    provider: &dyn VcsProvider,
    prs: &[PullRequest],
    blocked_review_required_is_ready: bool,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<usize, CoreError> {
    let mut resolved = 0;
    for pr in prs {
        if resolved >= limit {
            break;
        }
        if pr.hidden || pr.state != crate::domain::PrState::Open {
            continue;
        }
        let waiting = matches!(
            cached_readiness(store, pr, blocked_review_required_is_ready)?,
            Readiness::Unknown { reasons } if reasons.iter().any(|reason| reason == CODE_OWNER_REVIEW_UNKNOWN)
        );
        if !waiting {
            continue;
        }
        match resolve(store, provider, pr, blocked_review_required_is_ready, now) {
            Ok(_) => resolved += 1,
            Err(CoreError::Provider(
                err @ (ProviderError::RateLimited { .. } | ProviderError::Unauthorized(_)),
            )) => return Err(err.into()),
            Err(_) => {}
        }
    }
    Ok(resolved)
}

/// The last report for `pr` when its head and base still match a stored
/// probe, without calling the provider. `None` when the row is missing, the
/// head or base moved, the probe was not stored, or a team that needs a
/// membership check has no fresh cache entry.
pub fn report_from_cache(
    store: &Store,
    pr: &PullRequest,
    blocked_review_required_is_ready: bool,
    now: DateTime<Utc>,
) -> Result<Option<OwnershipReport>, CoreError> {
    let Some(entry) = store.get_ownership_cache(pr.id)? else {
        return Ok(None);
    };
    let Some(cached) = CachedOwnership::for_pr(&entry, pr) else {
        return Ok(None);
    };
    let Some(probe) = cached.probe else {
        return Ok(None);
    };
    if probe.head_sha != pr.head_sha || probe.base_ref != pr.base_ref {
        return Ok(None);
    }
    let catalog = catalog(&probe, &cached.inputs.files);
    let teams: Vec<_> = teams_needing_membership(pr, &probe, &catalog)
        .into_iter()
        .take(MAX_MEMBERSHIP_LOOKUPS)
        .collect();
    let mut membership = BTreeMap::new();
    for team in teams {
        let Some(members) = cached_team_members(store, &team, now)? else {
            return Ok(None);
        };
        membership.insert(team, members);
    }
    Ok(Some(build_report(
        pr,
        &probe,
        &cached.inputs,
        &membership,
        blocked_review_required_is_ready,
        true,
    )))
}

/// Probes, reuses or refreshes the cached inputs, checks team membership, and
/// stores the result.
pub fn resolve(
    store: &Store,
    provider: &dyn VcsProvider,
    pr: &PullRequest,
    blocked_review_required_is_ready: bool,
    now: DateTime<Utc>,
) -> Result<OwnershipReport, CoreError> {
    let started = std::time::Instant::now();
    let probe = {
        let _span = tracing::debug_span!("ownership_probe", pr = pr.id.0).entered();
        provider.ownership_probe(pr)?
    };
    tracing::debug!(
        pr = pr.id.0,
        ms = started.elapsed().as_millis() as u64,
        "ownership probe"
    );
    let oid = probe.codeowners.as_ref().map(|blob| blob.oid.clone());
    let cached = store.get_ownership_cache(pr.id)?.and_then(|entry| {
        let same_key = entry.head_sha == probe.head_sha
            && entry.base_ref == probe.base_ref
            && entry.codeowners_oid == oid;
        if same_key {
            serde_json::from_str::<CachedOwnership>(&entry.decision_json).ok()
        } else {
            None
        }
    });
    let from_cache = cached.is_some();
    let inputs_started = std::time::Instant::now();
    let inputs = match cached {
        Some(cached) => cached.inputs,
        None => {
            let _span = tracing::debug_span!("ownership_inputs", pr = pr.id.0).entered();
            provider.ownership_inputs(pr, &probe)?
        }
    };
    tracing::debug!(
        pr = pr.id.0,
        cached = from_cache,
        ms = inputs_started.elapsed().as_millis() as u64,
        "ownership inputs"
    );

    let catalog = catalog(&probe, &inputs.files);
    let teams: Vec<_> = teams_needing_membership(pr, &probe, &catalog)
        .into_iter()
        .take(MAX_MEMBERSHIP_LOOKUPS)
        .collect();
    let members_started = std::time::Instant::now();
    let looked_up = {
        let _span =
            tracing::debug_span!("ownership_members", pr = pr.id.0, teams = teams.len()).entered();
        crate::parallel::bounded_map(&teams, MEMBER_LOOKUPS_IN_FLIGHT, |team| {
            team_members(store, provider, team, now)
        })
    };
    tracing::debug!(
        pr = pr.id.0,
        teams = teams.len(),
        ms = members_started.elapsed().as_millis() as u64,
        "ownership members"
    );
    let mut membership = BTreeMap::new();
    for (team, members) in teams.into_iter().zip(looked_up) {
        membership.insert(team, members?);
    }

    let report = build_report(
        pr,
        &probe,
        &inputs,
        &membership,
        blocked_review_required_is_ready,
        from_cache,
    );
    let decision_json = serde_json::to_string(&CachedOwnership {
        inputs,
        code_owner_review_pending: report.code_owner_review_pending,
        probe: Some(probe.clone()),
    })
    .map_err(|err| CoreError::Store(err.to_string()))?;
    store.upsert_ownership_cache(OwnershipCacheEntry {
        pr_id: pr.id,
        head_sha: probe.head_sha.clone(),
        base_ref: probe.base_ref.clone(),
        codeowners_oid: oid,
        decision_json,
        computed_at: now,
    })?;
    Ok(report)
}

/// A fresh team-member row, or `None` when it is missing or older than
/// [`TEAM_MEMBERS_TTL_HOURS`]. Does not call the provider.
fn cached_team_members(
    store: &Store,
    team: &TeamRef,
    now: DateTime<Utc>,
) -> Result<Option<Membership>, CoreError> {
    let Some(entry) = store.get_team_members(team.org.clone(), team.slug.clone())? else {
        return Ok(None);
    };
    if now - entry.fetched_at >= Duration::hours(TEAM_MEMBERS_TTL_HOURS) {
        return Ok(None);
    }
    let members: Vec<String> = serde_json::from_str(&entry.members_json).unwrap_or_default();
    Ok(Some(Membership {
        complete: entry.complete,
        members,
    }))
}

/// Members from the cache when fresh, otherwise from the provider.
///
/// A provider failure other than rate limiting or lost auth leaves the team
/// incomplete, which puts it in `attribution_unknown`. Only real answers are
/// cached, so a network blip is retried next time.
fn team_members(
    store: &Store,
    provider: &dyn VcsProvider,
    team: &TeamRef,
    now: DateTime<Utc>,
) -> Result<Membership, CoreError> {
    if let Some(entry) = store.get_team_members(team.org.clone(), team.slug.clone())? {
        let fresh = now - entry.fetched_at < Duration::hours(TEAM_MEMBERS_TTL_HOURS);
        if fresh {
            let members: Vec<String> =
                serde_json::from_str(&entry.members_json).unwrap_or_default();
            return Ok(Membership {
                complete: entry.complete,
                members,
            });
        }
    }
    let fetched = match provider.team_members(team) {
        Ok(members) => members,
        Err(err @ (ProviderError::RateLimited { .. } | ProviderError::Unauthorized(_))) => {
            return Err(err.into());
        }
        Err(_) => {
            return Ok(Membership {
                complete: false,
                members: Vec::new(),
            });
        }
    };
    store.upsert_team_members(TeamMembersCacheEntry {
        github_org: team.org.clone(),
        team_slug: team.slug.clone(),
        members_json: serde_json::to_string(&fetched.logins)
            .map_err(|err| CoreError::Store(err.to_string()))?,
        complete: fetched.complete,
        fetched_at: now,
    })?;
    Ok(Membership {
        complete: fetched.complete,
        members: fetched.logins,
    })
}

#[cfg(test)]
mod tests;
