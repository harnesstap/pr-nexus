//! Bulk reads for the pull request list: one query per table for a whole
//! account instead of a few queries per pull request.

use std::collections::HashMap;

use crate::domain::{AccountId, OwnershipCacheEntry, PrId, PrProjectLink, ReviewRequest};
use crate::error::CoreError;

use super::cache::map_ownership;
use super::projects::map_link;
use super::reviews::{map_request, slack_account};
use super::{Store, db};

/// Per pull request facts the list shows next to the synced fields.
#[derive(Debug, Default)]
pub struct ListFacts {
    /// Project links, including excluded ones, ordered by project id.
    pub links: HashMap<PrId, Vec<PrProjectLink>>,
    /// The newest active review request per pull request.
    pub active_requests: HashMap<PrId, ReviewRequest>,
    pub ownership: HashMap<PrId, OwnershipCacheEntry>,
}

impl Store {
    pub fn list_facts(&self, account_id: AccountId) -> Result<ListFacts, CoreError> {
        self.read(move |conn| {
            let mut facts = ListFacts::default();
            let scope = "SELECT pr.id FROM pull_requests pr
                         JOIN repositories r ON r.id = pr.repo_id
                         WHERE r.account_id = ?1";

            let mut stmt = conn
                .prepare(&format!(
                    "SELECT * FROM pr_projects WHERE pr_id IN ({scope}) ORDER BY pr_id, project_id"
                ))
                .map_err(db)?;
            for link in stmt.query_map([account_id.0], map_link).map_err(db)? {
                let link = link.map_err(db)?;
                facts.links.entry(link.pr).or_default().push(link);
            }

            let mut stmt = conn
                .prepare(&format!(
                    "SELECT * FROM review_requests
                     WHERE status = 'active' AND pr_id IN ({scope})
                     ORDER BY requested_at, id"
                ))
                .map_err(db)?;
            let mut accounts = HashMap::new();
            for request in stmt.query_map([account_id.0], map_request).map_err(db)? {
                let mut request = request.map_err(db)?;
                let account = match accounts.get(&request.workspace) {
                    Some(account) => *account,
                    None => {
                        let account = slack_account(conn, &request.workspace).map_err(db)?;
                        accounts.insert(request.workspace.clone(), account);
                        account
                    }
                };
                request.root.account = account;
                // Ordered by time, so the last one written is the newest.
                facts.active_requests.insert(request.pr, request);
            }

            let mut stmt = conn
                .prepare(&format!(
                    "SELECT * FROM ownership_cache WHERE pr_id IN ({scope})"
                ))
                .map_err(db)?;
            for entry in stmt.query_map([account_id.0], map_ownership).map_err(db)? {
                let entry = entry.map_err(db)?;
                facts.ownership.insert(entry.pr_id, entry);
            }
            Ok(facts)
        })
    }
}
