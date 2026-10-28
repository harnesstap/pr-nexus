//! The detail panel's Branch card (§7.2): merge conflicts and how far the
//! head is behind its base, plus updating the branch from the base.
//!
//! Mergeability comes from the overview. The behind count costs one REST
//! compare, so it is fetched only when the panel asks (never during a bulk
//! sync) and cached per pull request, keyed on the head and base SHAs.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::domain::{PrId, PullRequest};
use crate::error::{CoreError, ProviderError};
use crate::overview::{self, PrOverview};
use crate::providers::VcsProvider;
use crate::store::Store;

/// What the Branch card shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchState {
    /// No conflicts and nothing new on the base.
    UpToDate,
    /// Behind the base; updating is optional.
    Behind,
    /// Behind the base and branch protection requires up-to-date branches.
    UpdateRequired,
    /// The head conflicts with the base.
    Conflicts,
    /// GitHub has not computed mergeability yet.
    Checking,
}

/// Works out the card's state. `mergeable` and `merge_state` are the
/// overview's lower-case values; `strict` is "the base requires branches to
/// be up to date" (GitHub reports that as `mergeStateStatus: BEHIND`).
pub fn derive_state(
    mergeable: &str,
    merge_state: &str,
    behind_by: Option<u32>,
    strict: bool,
) -> BranchState {
    if mergeable == "conflicting" || merge_state == "dirty" {
        return BranchState::Conflicts;
    }
    if merge_state == "behind" {
        return BranchState::UpdateRequired;
    }
    if mergeable != "mergeable" {
        return BranchState::Checking;
    }
    match behind_by {
        Some(count) if count > 0 && strict => BranchState::UpdateRequired,
        Some(count) if count > 0 => BranchState::Behind,
        _ => BranchState::UpToDate,
    }
}

/// How [`update`] brings the base into the head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMethod {
    /// A merge commit of the base into the head (GitHub's default).
    Merge,
    /// Rebases the head onto the base. Rewrites the branch history.
    Rebase,
}

impl UpdateMethod {
    /// GitHub's `PullRequestBranchUpdateMethod` value.
    pub fn graphql(self) -> &'static str {
        match self {
            Self::Merge => "MERGE",
            Self::Rebase => "REBASE",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "merge" => Some(Self::Merge),
            "rebase" => Some(Self::Rebase),
            _ => None,
        }
    }
}

/// One side of a comparison: `owner:ref`, so forks compare correctly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    pub owner: String,
    pub name: String,
}

/// The REST compare of base and head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchComparison {
    pub behind_by: u32,
    pub ahead_by: u32,
    /// Logins of the authors of the head's own commits, deduped. A commit
    /// without a linked GitHub account is listed by its git author name.
    pub authors: Vec<String>,
}

/// The Branch card's data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchStatus {
    pub state: BranchState,
    pub base_ref: String,
    pub head_ref: String,
    /// `None` while unknown (not compared, or the compare failed).
    pub behind_by: Option<u32>,
    /// Commit authors on the head other than the signed-in user; the rebase
    /// confirmation warns when this is not empty.
    pub other_authors: Vec<String>,
    /// Where GitHub resolves conflicts in the browser.
    pub conflicts_url: String,
    /// The head SHA this status describes.
    pub head_sha: String,
}

/// Behind counts per pull request, valid while head and base SHAs match.
#[derive(Default)]
pub struct BranchCache {
    entries: Mutex<HashMap<PrId, (String, String, BranchComparison)>>,
}

impl BranchCache {
    fn get(&self, pr: PrId, head: &str, base: &str) -> Option<BranchComparison> {
        let entries = self.entries.lock().expect("branch cache");
        entries
            .get(&pr)
            .filter(|(h, b, _)| h == head && b == base)
            .map(|(_, _, comparison)| comparison.clone())
    }

    fn put(&self, pr: PrId, head: &str, base: &str, comparison: BranchComparison) {
        self.entries
            .lock()
            .expect("branch cache")
            .insert(pr, (head.into(), base.into(), comparison));
    }

    pub fn forget(&self, pr: PrId) {
        self.entries.lock().expect("branch cache").remove(&pr);
    }
}

fn sides(pr: &PullRequest, overview: &PrOverview) -> (BranchRef, BranchRef) {
    let head_owner = if overview.head_owner.is_empty() {
        pr.repo.owner.clone()
    } else {
        overview.head_owner.clone()
    };
    (
        BranchRef {
            owner: pr.repo.owner.clone(),
            name: overview.base_ref.clone(),
        },
        BranchRef {
            owner: head_owner,
            name: overview.head_ref.clone(),
        },
    )
}

fn head_sha(pr: &PullRequest, overview: &PrOverview) -> String {
    if overview.head_sha.is_empty() {
        pr.head_sha.clone()
    } else {
        overview.head_sha.clone()
    }
}

/// The Branch card for `pr` from an `overview` the caller already has.
/// Compares only an open pull request without conflicts; a failed compare
/// leaves the count unknown rather than failing the card.
pub fn status_from(
    provider: &dyn VcsProvider,
    cache: &BranchCache,
    pr: &PullRequest,
    overview: &PrOverview,
) -> BranchStatus {
    let head = head_sha(pr, overview);
    let quick = derive_state(&overview.mergeable, &overview.merge_state, None, false);
    let comparison = if overview.state == "open" && quick != BranchState::Conflicts {
        match cache.get(pr.id, &head, &overview.base_sha) {
            Some(hit) if !head.is_empty() && !overview.base_sha.is_empty() => Some(hit),
            _ => {
                let (base, head_side) = sides(pr, overview);
                match provider.compare_branches(pr, &base, &head_side) {
                    Ok(fresh) => {
                        if !head.is_empty() && !overview.base_sha.is_empty() {
                            cache.put(pr.id, &head, &overview.base_sha, fresh.clone());
                        }
                        Some(fresh)
                    }
                    Err(_) => None,
                }
            }
        }
    } else {
        None
    };
    let behind_by = comparison.as_ref().map(|found| found.behind_by);
    let viewer = overview.viewer.login.to_ascii_lowercase();
    let other_authors = comparison
        .map(|found| found.authors)
        .unwrap_or_default()
        .into_iter()
        .filter(|login| login.to_ascii_lowercase() != viewer)
        .collect();
    BranchStatus {
        state: derive_state(
            &overview.mergeable,
            &overview.merge_state,
            behind_by,
            overview.merge_state == "behind",
        ),
        base_ref: overview.base_ref.clone(),
        head_ref: overview.head_ref.clone(),
        behind_by,
        other_authors,
        conflicts_url: format!(
            "https://{}/{}/{}/pull/{}/conflicts",
            pr.repo.host, pr.repo.owner, pr.repo.name, pr.number
        ),
        head_sha: head,
    }
}

/// The Branch card with a fresh overview (one GraphQL call) plus the compare.
pub fn status(
    store: &Store,
    provider: &dyn VcsProvider,
    cache: &BranchCache,
    pr: &PullRequest,
) -> Result<BranchStatus, CoreError> {
    let fresh = overview::refresh(store, provider, pr)?;
    Ok(status_from(provider, cache, pr, &fresh))
}

/// The overview and Branch card after [`update`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchUpdate {
    pub overview: PrOverview,
    pub branch: BranchStatus,
}

/// Brings the base into the head with `method`, guarded by the head SHA the
/// user last saw (the cached overview's), then refreshes both cards. A head
/// that moved meanwhile fails with `head_moved` and nothing changes.
pub fn update(
    store: &Store,
    provider: &dyn VcsProvider,
    cache: &BranchCache,
    pr: &PullRequest,
    method: UpdateMethod,
) -> Result<BranchUpdate, CoreError> {
    let seen = match overview::cached(store, pr.id)? {
        Some(cached) => cached,
        None => overview::refresh(store, provider, pr)?,
    };
    let expected = head_sha(pr, &seen);
    provider
        .update_branch(pr, &expected, method)
        .map_err(classify_update_error)?;
    cache.forget(pr.id);
    let fresh = overview::refresh(store, provider, pr)?;
    let branch = status_from(provider, cache, pr, &fresh);
    Ok(BranchUpdate {
        overview: fresh,
        branch,
    })
}

/// Turns GitHub's free-text update errors into codes the UI can explain.
fn classify_update_error(err: ProviderError) -> ProviderError {
    let ProviderError::Api { code, message } = &err else {
        return err;
    };
    let text = message.to_ascii_lowercase();
    if text.contains("expected head") || text.contains("head sha") {
        ProviderError::Api {
            code: "head_moved".into(),
            message: "The branch changed on GitHub since this panel loaded. Refresh and try again."
                .into(),
        }
    } else if text.contains("conflict") {
        ProviderError::Api {
            code: "conflicts".into(),
            message: "The base can't be merged in cleanly: resolve the conflicts on GitHub.".into(),
        }
    } else if text.contains("permission") || text.contains("not authorized") {
        ProviderError::Forbidden("You don't have permission to update this branch.".into())
    } else {
        ProviderError::Api {
            code: code.clone(),
            message: message.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
