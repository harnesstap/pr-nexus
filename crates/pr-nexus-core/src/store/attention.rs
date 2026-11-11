//! Last-seen activity, snooze, and mute (migration 0011).
//!
//! Rows are keyed by the GitHub node id, like stars, so they outlive the
//! local pull request id.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::attention::{
    Activity, AttentionFlags, AttentionState, PrActivity, Snooze, SnoozeChoice, evaluate,
    snooze_from,
};
use crate::domain::PrId;
use crate::error::CoreError;

use super::row::conv;
use super::time::{format_ts, ts_col};
use super::{Store, db};

impl Store {
    /// Records a baseline for pull requests that have none, then returns the
    /// flags for every id. A missing row is not unread: the first sighting
    /// becomes last-seen, so existing rows do not all light up.
    pub fn sync_attention(
        &self,
        rows: &[PrActivity],
        now: DateTime<Utc>,
    ) -> Result<HashMap<PrId, AttentionFlags>, CoreError> {
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let stored = self.read(load_attention)?;
        let missing: Vec<PrActivity> = rows
            .iter()
            .filter(|row| !row.node_id.is_empty() && !stored.contains_key(&row.node_id))
            .cloned()
            .collect();
        if !missing.is_empty() {
            self.write(move |conn| {
                for row in &missing {
                    insert_baseline(conn, row, now)?;
                }
                Ok(())
            })?;
        }
        let mut flags = HashMap::new();
        for row in rows {
            let state = stored.get(&row.node_id);
            let flags_for = if row.node_id.is_empty() {
                AttentionFlags::default()
            } else if state.is_none() {
                // The baseline just stored matches `row.activity`.
                AttentionFlags::default()
            } else {
                evaluate(state, &row.activity, now)
            };
            flags.insert(row.pr, flags_for);
        }
        Ok(flags)
    }

    /// Stores the current activity as last-seen. Snooze and mute stay as they are.
    pub fn mark_attention_read(&self, ids: &[PrId], now: DateTime<Utc>) -> Result<(), CoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids = ids.to_vec();
        self.write(move |conn| {
            for id in &ids {
                let Some(current) = load_current(conn, *id)? else {
                    return Err(CoreError::Validation("Pull request not found".into()));
                };
                let mut state = load_one(conn, &current.node_id)?.unwrap_or(AttentionState {
                    seen: current.activity.clone(),
                    muted: false,
                    snooze: None,
                });
                state.seen = current.activity;
                save(conn, &current.node_id, &state, now)?;
            }
            Ok(())
        })
    }

    pub fn set_attention_snooze(
        &self,
        ids: &[PrId],
        choice: SnoozeChoice,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids = ids.to_vec();
        self.write(move |conn| {
            for id in &ids {
                let Some(current) = load_current(conn, *id)? else {
                    return Err(CoreError::Validation("Pull request not found".into()));
                };
                let mut state = load_one(conn, &current.node_id)?.unwrap_or(AttentionState {
                    seen: current.activity.clone(),
                    muted: false,
                    snooze: None,
                });
                state.snooze = snooze_from(choice, &current.activity, now);
                save(conn, &current.node_id, &state, now)?;
            }
            Ok(())
        })
    }

    pub fn set_attention_muted(
        &self,
        ids: &[PrId],
        muted: bool,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids = ids.to_vec();
        self.write(move |conn| {
            for id in &ids {
                let Some(current) = load_current(conn, *id)? else {
                    return Err(CoreError::Validation("Pull request not found".into()));
                };
                let mut state = load_one(conn, &current.node_id)?.unwrap_or(AttentionState {
                    seen: current.activity.clone(),
                    muted: false,
                    snooze: None,
                });
                state.muted = muted;
                save(conn, &current.node_id, &state, now)?;
            }
            Ok(())
        })
    }
}

struct Current {
    node_id: String,
    activity: Activity,
}

fn load_current(conn: &Connection, id: PrId) -> Result<Option<Current>, CoreError> {
    conn.query_row(
        "SELECT node_id, head_sha, checks_state, review_decision, total_threads, unresolved_threads, updated_at
         FROM pull_requests WHERE id = ?1",
        [id.0],
        |row| {
            let updated: String = row.get(6)?;
            Ok(Current {
                node_id: row.get(0)?,
                activity: Activity {
                    head_sha: row.get(1)?,
                    checks: row.get(2)?,
                    review: row.get(3)?,
                    threads: row.get::<_, i64>(4)? as u32,
                    unresolved: row.get::<_, i64>(5)? as u32,
                    updated_at: ts_col(updated)?,
                }
                .at_millis(),
            })
        },
    )
    .optional()
    .map_err(db)
}

fn load_attention(conn: &Connection) -> Result<HashMap<String, AttentionState>, CoreError> {
    let mut stmt = conn.prepare("SELECT * FROM pr_attention").map_err(db)?;
    let rows = stmt
        .query_map([], |row| {
            let node_id: String = row.get("node_id")?;
            Ok((node_id, map_state(row)?))
        })
        .map_err(db)?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(db)?;
    Ok(rows)
}

fn load_one(conn: &Connection, node_id: &str) -> Result<Option<AttentionState>, CoreError> {
    conn.query_row(
        "SELECT * FROM pr_attention WHERE node_id = ?1",
        [node_id],
        map_state,
    )
    .optional()
    .map_err(db)
}

fn map_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<AttentionState> {
    let seen_updated: String = row.get("seen_updated_at")?;
    let kind: Option<String> = row.get("snooze_kind")?;
    let snooze = match kind.as_deref() {
        None => None,
        Some("until") => {
            let until: String = row.get("snooze_until")?;
            Some(Snooze::Until(ts_col(until)?))
        }
        Some("activity") => {
            let updated: String = row.get("snooze_updated_at")?;
            Some(Snooze::UntilActivity(Activity {
                head_sha: row.get("snooze_sha")?,
                checks: row.get("snooze_checks")?,
                review: row.get("snooze_review")?,
                threads: opt_u32(row.get("snooze_threads")?)?,
                unresolved: opt_u32(row.get("snooze_unresolved")?)?,
                updated_at: ts_col(updated)?,
            }))
        }
        Some(other) => {
            return Err(conv(CoreError::Store(format!(
                "unknown snooze kind {other}"
            ))));
        }
    };
    Ok(AttentionState {
        seen: Activity {
            head_sha: row.get("seen_sha")?,
            checks: row.get("seen_checks")?,
            review: row.get("seen_review")?,
            threads: row.get::<_, i64>("seen_threads")? as u32,
            unresolved: row.get::<_, i64>("seen_unresolved")? as u32,
            updated_at: ts_col(seen_updated)?,
        },
        muted: row.get::<_, i64>("muted")? != 0,
        snooze,
    })
}

fn opt_u32(value: Option<i64>) -> rusqlite::Result<u32> {
    let Some(value) = value else {
        return Err(conv(CoreError::Store(
            "snooze activity is missing a thread count".into(),
        )));
    };
    u32::try_from(value).map_err(|err| conv(CoreError::Store(err.to_string())))
}

fn insert_baseline(
    conn: &Connection,
    row: &PrActivity,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    conn.execute(
        "INSERT OR IGNORE INTO pr_attention (
            node_id, seen_sha, seen_checks, seen_review, seen_threads, seen_unresolved, seen_updated_at,
            muted, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
        params![
            row.node_id,
            row.activity.head_sha,
            row.activity.checks,
            row.activity.review,
            row.activity.threads,
            row.activity.unresolved,
            format_ts(row.activity.updated_at),
            format_ts(now),
        ],
    )
    .map_err(db)?;
    Ok(())
}

fn save(
    conn: &Connection,
    node_id: &str,
    state: &AttentionState,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let cols = snooze_columns(state.snooze.as_ref());
    conn.execute(
        "INSERT INTO pr_attention (
            node_id, seen_sha, seen_checks, seen_review, seen_threads, seen_unresolved, seen_updated_at,
            muted, snooze_kind, snooze_until, snooze_sha, snooze_checks, snooze_review,
            snooze_threads, snooze_unresolved, snooze_updated_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT (node_id) DO UPDATE SET
            seen_sha = excluded.seen_sha,
            seen_checks = excluded.seen_checks,
            seen_review = excluded.seen_review,
            seen_threads = excluded.seen_threads,
            seen_unresolved = excluded.seen_unresolved,
            seen_updated_at = excluded.seen_updated_at,
            muted = excluded.muted,
            snooze_kind = excluded.snooze_kind,
            snooze_until = excluded.snooze_until,
            snooze_sha = excluded.snooze_sha,
            snooze_checks = excluded.snooze_checks,
            snooze_review = excluded.snooze_review,
            snooze_threads = excluded.snooze_threads,
            snooze_unresolved = excluded.snooze_unresolved,
            snooze_updated_at = excluded.snooze_updated_at,
            updated_at = excluded.updated_at",
        params![
            node_id,
            state.seen.head_sha,
            state.seen.checks,
            state.seen.review,
            state.seen.threads,
            state.seen.unresolved,
            format_ts(state.seen.updated_at),
            i64::from(state.muted),
            cols.kind,
            cols.until,
            cols.sha,
            cols.checks,
            cols.review,
            cols.threads,
            cols.unresolved,
            cols.updated_at,
            format_ts(now),
        ],
    )
    .map_err(db)?;
    Ok(())
}

struct SnoozeColumns {
    kind: Option<String>,
    until: Option<String>,
    sha: Option<String>,
    checks: Option<String>,
    review: Option<String>,
    threads: Option<i64>,
    unresolved: Option<i64>,
    updated_at: Option<String>,
}

fn snooze_columns(snooze: Option<&Snooze>) -> SnoozeColumns {
    match snooze {
        None => SnoozeColumns {
            kind: None,
            until: None,
            sha: None,
            checks: None,
            review: None,
            threads: None,
            unresolved: None,
            updated_at: None,
        },
        Some(Snooze::Until(until)) => SnoozeColumns {
            kind: Some("until".into()),
            until: Some(format_ts(*until)),
            sha: None,
            checks: None,
            review: None,
            threads: None,
            unresolved: None,
            updated_at: None,
        },
        Some(Snooze::UntilActivity(activity)) => SnoozeColumns {
            kind: Some("activity".into()),
            until: None,
            sha: Some(activity.head_sha.clone()),
            checks: Some(activity.checks.clone()),
            review: activity.review.clone(),
            threads: Some(i64::from(activity.threads)),
            unresolved: Some(i64::from(activity.unresolved)),
            updated_at: Some(format_ts(activity.updated_at)),
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use crate::attention::SnoozeChoice;
    use crate::domain::{
        Account, AccountId, AccountKind, AuthMode, ChecksState, Involvement, MergeStateStatus,
        Mergeable, PrId, PrState, ProviderRef, PullRequest, RepoRef, Repository,
    };

    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn open() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("pr-nexus.db")).unwrap();
        (dir, store)
    }

    fn seed(store: &Store, node_id: &str, sha: &str) -> (i64, PullRequest) {
        let account = store
            .insert_account(Account {
                id: AccountId(0),
                kind: AccountKind::GitHub,
                display_name: "octocat".into(),
                host: "github.com".into(),
                external_id: format!("gh-{node_id}"),
                slack_workspace: None,
                login: "octocat".into(),
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
                node_id: Some(format!("R_{node_id}")),
                default_branch: Some("main".into()),
                archived: false,
            })
            .unwrap();
        let pr = store
            .upsert_pull_request(repo.id, pull(&account, node_id, sha))
            .unwrap();
        (repo.id, pr)
    }

    fn pull(account: &Account, node_id: &str, sha: &str) -> PullRequest {
        PullRequest {
            id: PrId(0),
            remote: ProviderRef {
                account: account.id,
                id: node_id.into(),
            },
            repo: RepoRef {
                host: "github.com".into(),
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: u64::try_from(node_id.len()).unwrap_or(7),
            title: "Latch the widget".into(),
            url: "https://github.com/acme/widgets/pull/7".into(),
            author_login: "octocat".into(),
            body: String::new(),
            state: PrState::Open,
            is_draft: false,
            head_ref: "latch".into(),
            base_ref: "main".into(),
            head_sha: sha.into(),
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
        }
    }

    #[test]
    fn the_first_sighting_is_read_and_a_later_push_is_not() {
        let (_dir, store) = open();
        let (repo, pr) = seed(&store, "PR_node", "aaa");
        let now = at();
        let flags = store
            .sync_attention(&[PrActivity::from_pull_request(&pr)], now)
            .unwrap();
        assert!(!flags[&pr.id].unread);

        let mut pushed = pr.clone();
        pushed.head_sha = "bbb".into();
        let saved = store.upsert_pull_request(repo, pushed).unwrap();
        let flags = store
            .sync_attention(&[PrActivity::from_pull_request(&saved)], now)
            .unwrap();
        assert!(flags[&saved.id].unread);

        store.mark_attention_read(&[saved.id], now).unwrap();
        let flags = store
            .sync_attention(&[PrActivity::from_pull_request(&saved)], now)
            .unwrap();
        assert!(!flags[&saved.id].unread);
    }

    #[test]
    fn snooze_and_mute_follow_the_node_id_onto_a_new_row() {
        let (_dir, store) = open();
        let (_repo, pr) = seed(&store, "PR_node", "aaa");
        let now = at();
        store
            .sync_attention(&[PrActivity::from_pull_request(&pr)], now)
            .unwrap();
        store
            .set_attention_snooze(&[pr.id], SnoozeChoice::Week, now)
            .unwrap();
        store.set_attention_muted(&[pr.id], true, now).unwrap();

        let flags = store
            .sync_attention(
                &[PrActivity::from_pull_request(&pr)],
                now + Duration::days(1),
            )
            .unwrap();
        assert!(flags[&pr.id].snoozed);
        assert!(flags[&pr.id].muted);

        store.delete_pull_request(pr.id).unwrap();
        let account = store.list_accounts().unwrap().remove(0);
        let repo = store.list_repositories(account.id).unwrap().remove(0);
        // A new row can reuse the deleted id. Occupy that id so the returning PR is a new one.
        let _other = store
            .upsert_pull_request(repo.id, pull(&account, "PR_other", "ccc"))
            .unwrap();
        let again = store
            .upsert_pull_request(repo.id, pull(&account, "PR_node", "aaa"))
            .unwrap();
        assert_ne!(again.id, pr.id);
        let flags = store
            .sync_attention(
                &[PrActivity::from_pull_request(&again)],
                now + Duration::days(1),
            )
            .unwrap();
        assert!(flags[&again.id].snoozed);
        assert!(flags[&again.id].muted);

        store
            .set_attention_snooze(&[again.id], SnoozeChoice::Clear, now)
            .unwrap();
        let flags = store
            .sync_attention(
                &[PrActivity::from_pull_request(&again)],
                now + Duration::days(1),
            )
            .unwrap();
        assert!(!flags[&again.id].snoozed);
        assert!(flags[&again.id].muted);
    }

    #[test]
    fn an_unknown_pull_request_is_rejected() {
        let (_dir, store) = open();
        let err = store.mark_attention_read(&[PrId(9)], at()).unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)));
    }
}
