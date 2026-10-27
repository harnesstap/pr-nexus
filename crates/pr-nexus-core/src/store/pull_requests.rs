use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{
    AccountId, Involvement, PR_BODY_MAX_BYTES, PrId, ProviderRef, PullRequest, RepoRef, Repository,
    RequestedReviewer, Reviewer, TeamRef,
};
use crate::error::CoreError;
use crate::search;

use super::row::{conv, parse_enum, parse_opt_enum};
use super::time::{format_ts, parse_bool, sql_bool, ts_col};
use super::{Store, db};

impl Store {
    pub fn insert_repository(&self, repo: Repository) -> Result<Repository, CoreError> {
        self.write(move |conn| insert_repository(conn, &repo))
    }

    pub fn get_repository(&self, id: i64) -> Result<Option<Repository>, CoreError> {
        self.read(move |conn| load_repository(conn, id))
    }

    pub fn list_repositories(&self, account_id: AccountId) -> Result<Vec<Repository>, CoreError> {
        self.read(move |conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM repositories WHERE account_id = ?1 ORDER BY owner, name")
                .map_err(db)?;
            let rows = stmt
                .query_map([account_id.0], map_repository)
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }

    pub fn upsert_repository(&self, repo: Repository) -> Result<Repository, CoreError> {
        self.write(move |conn| upsert_repository(conn, &repo))
    }

    pub fn delete_repository(&self, id: i64) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute("DELETE FROM repositories WHERE id = ?1", [id])
                .map_err(db)?;
            Ok(())
        })
    }

    pub fn upsert_pull_request(
        &self,
        repo_id: i64,
        pr: PullRequest,
    ) -> Result<PullRequest, CoreError> {
        self.write(move |conn| {
            let tx = conn.transaction().map_err(db)?;
            let saved = upsert_pull_request(&tx, repo_id, &pr)?;
            tx.commit().map_err(db)?;
            Ok(saved)
        })
    }

    pub fn get_pull_request(&self, id: PrId) -> Result<Option<PullRequest>, CoreError> {
        self.read(move |conn| load_pull_request(conn, id))
    }

    pub fn list_pull_requests(&self, repo_id: i64) -> Result<Vec<PullRequest>, CoreError> {
        self.read(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM pull_requests WHERE repo_id = ?1 ORDER BY number")
                .map_err(db)?;
            let ids = stmt
                .query_map([repo_id], |row| row.get::<_, i64>(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            ids.into_iter()
                .map(|id| {
                    load_pull_request(conn, PrId(id))?
                        .ok_or_else(|| CoreError::Store(format!("pull request {id} disappeared")))
                })
                .collect()
        })
    }

    /// Every pull request of `account_id`, newest update first, with labels and
    /// requested reviewers. Four queries in total, whatever the count.
    pub fn list_pull_requests_for_account(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<PullRequest>, CoreError> {
        self.read(move |conn| list_for_account(conn, account_id, true))
    }

    /// [`Store::list_pull_requests_for_account`] without descriptions, for the list.
    pub fn list_pull_requests_for_account_slim(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<PullRequest>, CoreError> {
        self.read(move |conn| list_for_account(conn, account_id, false))
    }

    /// Ids of `account_id`'s pull requests that match every term.
    ///
    /// `terms` must already be folded ([`search::terms`]). A term matches the
    /// stored search text or the name of a project the pull request is linked
    /// to; `project_hits[i]` lists the projects whose folded name contains
    /// `terms[i]`.
    pub fn search_pull_request_ids(
        &self,
        account_id: AccountId,
        terms: Vec<String>,
        project_hits: Vec<Vec<i64>>,
    ) -> Result<Vec<PrId>, CoreError> {
        self.read(move |conn| {
            let mut sql = String::from(
                "SELECT pr.id FROM pull_requests pr
                 JOIN repositories r ON r.id = pr.repo_id
                 WHERE r.account_id = ?1",
            );
            let mut values: Vec<rusqlite::types::Value> = vec![account_id.0.into()];
            for (index, term) in terms.iter().enumerate() {
                values.push(term.clone().into());
                let slot = values.len();
                sql.push_str(&format!(" AND (instr(pr.search_text, ?{slot}) > 0"));
                let hits = project_hits.get(index).map(Vec::as_slice).unwrap_or(&[]);
                if !hits.is_empty() {
                    let ids = hits
                        .iter()
                        .map(i64::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    sql.push_str(&format!(
                        " OR pr.id IN (SELECT pr_id FROM pr_projects
                           WHERE source <> 'excluded' AND project_id IN ({ids}))"
                    ));
                }
                sql.push(')');
            }
            let mut stmt = conn.prepare(&sql).map_err(db)?;
            let ids = stmt
                .query_map(rusqlite::params_from_iter(values), |row| {
                    row.get::<_, i64>(0).map(PrId)
                })
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(ids)
        })
    }

    pub fn set_pull_request_hidden(&self, id: PrId, hidden: bool) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "UPDATE pull_requests SET hidden = ?1 WHERE id = ?2",
                params![sql_bool(hidden), id.0],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn delete_pull_request(&self, id: PrId) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute("DELETE FROM pull_requests WHERE id = ?1", [id.0])
                .map_err(db)?;
            Ok(())
        })
    }
}

fn upsert_repository(conn: &Connection, repo: &Repository) -> Result<Repository, CoreError> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO repositories (account_id, owner, name, node_id, default_branch, archived)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account_id, owner, name) DO UPDATE SET
                node_id = excluded.node_id,
                default_branch = excluded.default_branch,
                archived = excluded.archived
             RETURNING id",
            params![
                repo.account_id.0,
                repo.owner,
                repo.name,
                repo.node_id,
                repo.default_branch,
                sql_bool(repo.archived),
            ],
            |row| row.get(0),
        )
        .map_err(db)?;
    load_repository(conn, id)?
        .ok_or_else(|| CoreError::Store("upserted repository disappeared".to_owned()))
}

fn insert_repository(conn: &Connection, repo: &Repository) -> Result<Repository, CoreError> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO repositories (
                account_id, owner, name, node_id, default_branch, archived
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             RETURNING id",
            params![
                repo.account_id.0,
                repo.owner,
                repo.name,
                repo.node_id,
                repo.default_branch,
                sql_bool(repo.archived),
            ],
            |row| row.get(0),
        )
        .map_err(db)?;
    load_repository(conn, id)?
        .ok_or_else(|| CoreError::Store("inserted repository disappeared".to_owned()))
}

fn load_repository(conn: &Connection, id: i64) -> Result<Option<Repository>, CoreError> {
    conn.query_row(
        "SELECT * FROM repositories WHERE id = ?1",
        [id],
        map_repository,
    )
    .optional()
    .map_err(db)
}

fn map_repository(row: &rusqlite::Row<'_>) -> rusqlite::Result<Repository> {
    Ok(Repository {
        id: row.get("id")?,
        account_id: AccountId(row.get("account_id")?),
        owner: row.get("owner")?,
        name: row.get("name")?,
        node_id: row.get("node_id")?,
        default_branch: row.get("default_branch")?,
        archived: parse_bool(row.get("archived")?),
    })
}

fn upsert_pull_request(
    conn: &Connection,
    repo_id: i64,
    pr: &PullRequest,
) -> Result<PullRequest, CoreError> {
    let number = i64::try_from(pr.number)
        .map_err(|_| CoreError::Validation(format!("pr number {} does not fit i64", pr.number)))?;
    let body = cap_body(&pr.body);
    let (owner, name): (String, String) = conn
        .query_row(
            "SELECT owner, name FROM repositories WHERE id = ?1",
            [repo_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db)?;
    let search_text = search::search_text(&search::SearchFields {
        title: &pr.title,
        body: &body,
        owner: &owner,
        name: &name,
        number: pr.number,
        author: &pr.author_login,
        head_ref: &pr.head_ref,
    });
    let id: i64 = conn
        .query_row(
            "INSERT INTO pull_requests (
                repo_id, number, node_id, title, url, author_login, body, state, is_draft,
                head_ref, base_ref, head_sha, review_decision, checks_state, mergeable, merge_state,
                unresolved_threads, total_threads, involvement,
                created_at, updated_at, last_activity_at, synced_at, hidden, search_text
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19,
                ?20, ?21, ?22, ?23, ?24, ?25
             )
             ON CONFLICT(node_id) DO UPDATE SET
                repo_id = excluded.repo_id,
                number = excluded.number,
                title = excluded.title,
                url = excluded.url,
                author_login = excluded.author_login,
                body = excluded.body,
                state = excluded.state,
                is_draft = excluded.is_draft,
                head_ref = excluded.head_ref,
                base_ref = excluded.base_ref,
                head_sha = excluded.head_sha,
                review_decision = excluded.review_decision,
                checks_state = excluded.checks_state,
                mergeable = excluded.mergeable,
                merge_state = excluded.merge_state,
                unresolved_threads = excluded.unresolved_threads,
                total_threads = excluded.total_threads,
                involvement = excluded.involvement,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                last_activity_at = excluded.last_activity_at,
                synced_at = excluded.synced_at,
                hidden = excluded.hidden,
                search_text = excluded.search_text
             RETURNING id",
            params![
                repo_id,
                number,
                pr.remote.id,
                pr.title,
                pr.url,
                pr.author_login,
                body,
                pr.state.as_str(),
                sql_bool(pr.is_draft),
                pr.head_ref,
                pr.base_ref,
                pr.head_sha,
                pr.review_decision.map(|value| value.as_str().to_owned()),
                pr.checks.as_str(),
                pr.mergeable.as_str(),
                pr.merge_state.as_str(),
                pr.unresolved_threads,
                pr.total_threads,
                i64::from(pr.involvement.bits()),
                format_ts(pr.created_at),
                format_ts(pr.updated_at),
                format_ts(pr.last_activity_at),
                format_ts(pr.synced_at),
                sql_bool(pr.hidden),
                search_text,
            ],
            |row| row.get(0),
        )
        .map_err(db)?;

    conn.execute("DELETE FROM pr_labels WHERE pr_id = ?1", [id])
        .map_err(db)?;
    for label in &pr.labels {
        conn.execute(
            "INSERT INTO pr_labels (pr_id, name) VALUES (?1, ?2)",
            params![id, label],
        )
        .map_err(db)?;
    }

    conn.execute("DELETE FROM pr_requested_reviewers WHERE pr_id = ?1", [id])
        .map_err(db)?;
    for request in &pr.requested_reviewers {
        let (kind, org, login_or_slug) = match &request.reviewer {
            Reviewer::User { login } => ("user", None, login.as_str()),
            Reviewer::Team(team) => ("team", Some(team.org.as_str()), team.slug.as_str()),
        };
        conn.execute(
            "INSERT INTO pr_requested_reviewers
                (pr_id, kind, org, login_or_slug, name, avatar_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                kind,
                org,
                login_or_slug,
                request.name,
                request.avatar_url
            ],
        )
        .map_err(db)?;
    }

    load_pull_request(conn, PrId(id))?
        .ok_or_else(|| CoreError::Store("upserted pull request disappeared".to_owned()))
}

fn list_for_account(
    conn: &Connection,
    account_id: AccountId,
    with_body: bool,
) -> Result<Vec<PullRequest>, CoreError> {
    let body = if with_body { "pr.body" } else { "'' AS body" };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT pr.id, pr.repo_id, pr.number, pr.node_id, pr.title, pr.url, pr.author_login,
                    {body}, pr.state, pr.is_draft, pr.head_ref, pr.base_ref, pr.head_sha,
                    pr.review_decision, pr.checks_state, pr.mergeable, pr.merge_state,
                    pr.unresolved_threads, pr.total_threads, pr.involvement, pr.created_at,
                    pr.updated_at, pr.last_activity_at, pr.synced_at, pr.hidden,
                    r.owner AS repo_owner, r.name AS repo_name, a.id AS account_id,
                    a.host AS repo_host
             FROM pull_requests pr
             JOIN repositories r ON r.id = pr.repo_id
             JOIN accounts a ON a.id = r.account_id
             WHERE r.account_id = ?1
             ORDER BY pr.updated_at DESC, pr.number DESC"
        ))
        .map_err(db)?;
    let cols = PrColumns::of(&stmt).map_err(db)?;
    let mut prs = stmt
        .query_map([account_id.0], |row| map_pull_request_at(row, &cols))
        .map_err(db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db)?;
    let index: HashMap<i64, usize> = prs
        .iter()
        .enumerate()
        .map(|(position, pr)| (pr.id.0, position))
        .collect();

    let mut labels = conn
        .prepare(
            "SELECT l.pr_id, l.name FROM pr_labels l
             JOIN pull_requests pr ON pr.id = l.pr_id
             JOIN repositories r ON r.id = pr.repo_id
             WHERE r.account_id = ?1 ORDER BY l.rowid",
        )
        .map_err(db)?;
    let rows = labels
        .query_map([account_id.0], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db)?;
    for row in rows {
        let (pr_id, name) = row.map_err(db)?;
        if let Some(&position) = index.get(&pr_id) {
            prs[position].labels.push(name);
        }
    }

    let mut reviewers = conn
        .prepare(
            "SELECT v.pr_id, v.kind, v.org, v.login_or_slug, v.name, v.avatar_url
             FROM pr_requested_reviewers v
             JOIN pull_requests pr ON pr.id = v.pr_id
             JOIN repositories r ON r.id = pr.repo_id
             WHERE r.account_id = ?1 ORDER BY v.rowid",
        )
        .map_err(db)?;
    let rows = reviewers
        .query_map([account_id.0], |row| {
            Ok((row.get::<_, i64>("pr_id")?, map_reviewer(row)?))
        })
        .map_err(db)?;
    for row in rows {
        let (pr_id, reviewer) = row.map_err(db)?;
        if let Some(&position) = index.get(&pr_id) {
            prs[position].requested_reviewers.push(reviewer);
        }
    }
    Ok(prs)
}

/// Fills `search_text` where it is missing: rows written before migration 2.
pub(super) fn backfill_search_text(conn: &mut Connection) -> Result<usize, CoreError> {
    let tx = conn.transaction().map_err(db)?;
    let pending = {
        let mut stmt = tx
            .prepare(
                "SELECT pr.id, pr.title, pr.body, r.owner, r.name, pr.number, pr.author_login,
                        pr.head_ref
                 FROM pull_requests pr JOIN repositories r ON r.id = pr.repo_id
                 WHERE pr.search_text IS NULL",
            )
            .map_err(db)?;
        stmt.query_map([], |row| {
            let fields = (
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            );
            Ok((row.get::<_, i64>(0)?, fields))
        })
        .map_err(db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db)?
    };
    {
        let mut update = tx
            .prepare("UPDATE pull_requests SET search_text = ?1 WHERE id = ?2")
            .map_err(db)?;
        for (id, (title, body, owner, name, number, author, head_ref)) in &pending {
            let text = search::search_text(&search::SearchFields {
                title,
                body,
                owner,
                name,
                number: u64::try_from(*number).unwrap_or_default(),
                author,
                head_ref,
            });
            update.execute(params![text, id]).map_err(db)?;
        }
    }
    tx.commit().map_err(db)?;
    Ok(pending.len())
}

fn load_pull_request(conn: &Connection, id: PrId) -> Result<Option<PullRequest>, CoreError> {
    let Some(mut pr) = conn
        .query_row(
            "SELECT pr.*, r.owner AS repo_owner, r.name AS repo_name, a.id AS account_id, a.host AS repo_host
             FROM pull_requests pr
             JOIN repositories r ON r.id = pr.repo_id
             JOIN accounts a ON a.id = r.account_id
             WHERE pr.id = ?1",
            [id.0],
            map_pull_request,
        )
        .optional()
        .map_err(db)?
    else {
        return Ok(None);
    };

    let mut labels = conn
        .prepare("SELECT name FROM pr_labels WHERE pr_id = ?1 ORDER BY rowid")
        .map_err(db)?;
    pr.labels = labels
        .query_map([id.0], |row| row.get(0))
        .map_err(db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db)?;

    let mut reviewers = conn
        .prepare(
            "SELECT kind, org, login_or_slug, name, avatar_url FROM pr_requested_reviewers
             WHERE pr_id = ?1 ORDER BY rowid",
        )
        .map_err(db)?;
    pr.requested_reviewers = reviewers
        .query_map([id.0], map_reviewer)
        .map_err(db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db)?;
    Ok(Some(pr))
}

/// Column positions for [`map_pull_request_at`], resolved once per statement.
/// Looking columns up by name on every row cost most of the list query time.
struct PrColumns {
    involvement: usize,
    number: usize,
    id: usize,
    account_id: usize,
    node_id: usize,
    repo_host: usize,
    repo_owner: usize,
    repo_name: usize,
    title: usize,
    url: usize,
    author_login: usize,
    body: usize,
    state: usize,
    is_draft: usize,
    head_ref: usize,
    base_ref: usize,
    head_sha: usize,
    review_decision: usize,
    checks_state: usize,
    mergeable: usize,
    merge_state: usize,
    unresolved_threads: usize,
    total_threads: usize,
    created_at: usize,
    updated_at: usize,
    last_activity_at: usize,
    synced_at: usize,
    hidden: usize,
}

impl PrColumns {
    fn of(stmt: &rusqlite::Statement<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            involvement: stmt.column_index("involvement")?,
            number: stmt.column_index("number")?,
            id: stmt.column_index("id")?,
            account_id: stmt.column_index("account_id")?,
            node_id: stmt.column_index("node_id")?,
            repo_host: stmt.column_index("repo_host")?,
            repo_owner: stmt.column_index("repo_owner")?,
            repo_name: stmt.column_index("repo_name")?,
            title: stmt.column_index("title")?,
            url: stmt.column_index("url")?,
            author_login: stmt.column_index("author_login")?,
            body: stmt.column_index("body")?,
            state: stmt.column_index("state")?,
            is_draft: stmt.column_index("is_draft")?,
            head_ref: stmt.column_index("head_ref")?,
            base_ref: stmt.column_index("base_ref")?,
            head_sha: stmt.column_index("head_sha")?,
            review_decision: stmt.column_index("review_decision")?,
            checks_state: stmt.column_index("checks_state")?,
            mergeable: stmt.column_index("mergeable")?,
            merge_state: stmt.column_index("merge_state")?,
            unresolved_threads: stmt.column_index("unresolved_threads")?,
            total_threads: stmt.column_index("total_threads")?,
            created_at: stmt.column_index("created_at")?,
            updated_at: stmt.column_index("updated_at")?,
            last_activity_at: stmt.column_index("last_activity_at")?,
            synced_at: stmt.column_index("synced_at")?,
            hidden: stmt.column_index("hidden")?,
        })
    }
}

fn map_pull_request(row: &rusqlite::Row<'_>) -> rusqlite::Result<PullRequest> {
    map_pull_request_at(row, &PrColumns::of(row.as_ref())?)
}

fn map_pull_request_at(row: &rusqlite::Row<'_>, cols: &PrColumns) -> rusqlite::Result<PullRequest> {
    let involvement_bits = u8::try_from(row.get::<_, i64>(cols.involvement)?)
        .map_err(|err| conv(CoreError::Validation(err.to_string())))?;
    let involvement = Involvement::from_bits(involvement_bits).ok_or_else(|| {
        conv(CoreError::Validation(format!(
            "unknown involvement bits {involvement_bits}"
        )))
    })?;
    let number: i64 = row.get(cols.number)?;
    let number =
        u64::try_from(number).map_err(|err| conv(CoreError::Validation(err.to_string())))?;
    Ok(PullRequest {
        id: PrId(row.get(cols.id)?),
        remote: ProviderRef {
            account: AccountId(row.get(cols.account_id)?),
            id: row.get(cols.node_id)?,
        },
        repo: RepoRef {
            host: row.get(cols.repo_host)?,
            owner: row.get(cols.repo_owner)?,
            name: row.get(cols.repo_name)?,
        },
        number,
        title: row.get(cols.title)?,
        url: row.get(cols.url)?,
        author_login: row.get(cols.author_login)?,
        body: row.get(cols.body)?,
        state: parse_enum(row.get(cols.state)?)?,
        is_draft: parse_bool(row.get(cols.is_draft)?),
        head_ref: row.get(cols.head_ref)?,
        base_ref: row.get(cols.base_ref)?,
        head_sha: row.get(cols.head_sha)?,
        labels: Vec::new(),
        review_decision: parse_opt_enum(row.get(cols.review_decision)?)?,
        checks: parse_enum(row.get(cols.checks_state)?)?,
        mergeable: parse_enum(row.get(cols.mergeable)?)?,
        merge_state: parse_enum(row.get(cols.merge_state)?)?,
        requested_reviewers: Vec::new(),
        unresolved_threads: row.get::<_, i64>(cols.unresolved_threads)? as u32,
        total_threads: row.get::<_, i64>(cols.total_threads)? as u32,
        involvement,
        created_at: ts_col(row.get(cols.created_at)?)?,
        updated_at: ts_col(row.get(cols.updated_at)?)?,
        last_activity_at: ts_col(row.get(cols.last_activity_at)?)?,
        synced_at: ts_col(row.get(cols.synced_at)?)?,
        hidden: parse_bool(row.get(cols.hidden)?),
    })
}

fn map_reviewer(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestedReviewer> {
    Ok(RequestedReviewer {
        reviewer: map_reviewer_ref(row)?,
        name: row.get("name")?,
        avatar_url: row.get("avatar_url")?,
    })
}

fn map_reviewer_ref(row: &rusqlite::Row<'_>) -> rusqlite::Result<Reviewer> {
    let kind: String = row.get("kind")?;
    let login_or_slug: String = row.get("login_or_slug")?;
    let org: Option<String> = row.get("org")?;
    match kind.as_str() {
        "user" => Ok(Reviewer::User {
            login: login_or_slug,
        }),
        "team" => {
            let org = org.ok_or_else(|| {
                conv(CoreError::Validation(
                    "team reviewer is missing org".to_owned(),
                ))
            })?;
            Ok(Reviewer::Team(TeamRef {
                org,
                slug: login_or_slug,
            }))
        }
        other => Err(conv(CoreError::Validation(format!(
            "unknown reviewer kind {other}"
        )))),
    }
}

pub(crate) fn cap_body(body: &str) -> String {
    if body.len() <= PR_BODY_MAX_BYTES {
        return body.to_owned();
    }
    let mut end = PR_BODY_MAX_BYTES;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    body[..end].to_owned()
}
