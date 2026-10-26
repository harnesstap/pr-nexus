use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{
    AccountId, Involvement, PR_BODY_MAX_BYTES, PrId, ProviderRef, PullRequest, RepoRef, Repository,
    Reviewer, TeamRef,
};
use crate::error::CoreError;

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

    pub fn delete_pull_request(&self, id: PrId) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute("DELETE FROM pull_requests WHERE id = ?1", [id.0])
                .map_err(db)?;
            Ok(())
        })
    }
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
    let id: i64 = conn
        .query_row(
            "INSERT INTO pull_requests (
                repo_id, number, node_id, title, url, author_login, body, state, is_draft,
                head_ref, base_ref, head_sha, review_decision, checks_state, mergeable, merge_state,
                unresolved_threads, total_threads, involvement,
                created_at, updated_at, last_activity_at, synced_at, hidden
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19,
                ?20, ?21, ?22, ?23, ?24
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
                hidden = excluded.hidden
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
    for reviewer in &pr.requested_reviewers {
        let (kind, org, login_or_slug) = match reviewer {
            Reviewer::User { login } => ("user", None, login.as_str()),
            Reviewer::Team(team) => ("team", Some(team.org.as_str()), team.slug.as_str()),
        };
        conn.execute(
            "INSERT INTO pr_requested_reviewers (pr_id, kind, org, login_or_slug) VALUES (?1, ?2, ?3, ?4)",
            params![id, kind, org, login_or_slug],
        )
        .map_err(db)?;
    }

    load_pull_request(conn, PrId(id))?
        .ok_or_else(|| CoreError::Store("upserted pull request disappeared".to_owned()))
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
            "SELECT kind, org, login_or_slug FROM pr_requested_reviewers
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

fn map_pull_request(row: &rusqlite::Row<'_>) -> rusqlite::Result<PullRequest> {
    let involvement_bits = u8::try_from(row.get::<_, i64>("involvement")?)
        .map_err(|err| conv(CoreError::Validation(err.to_string())))?;
    let involvement = Involvement::from_bits(involvement_bits).ok_or_else(|| {
        conv(CoreError::Validation(format!(
            "unknown involvement bits {involvement_bits}"
        )))
    })?;
    let number: i64 = row.get("number")?;
    let number =
        u64::try_from(number).map_err(|err| conv(CoreError::Validation(err.to_string())))?;
    Ok(PullRequest {
        id: PrId(row.get("id")?),
        remote: ProviderRef {
            account: AccountId(row.get("account_id")?),
            id: row.get("node_id")?,
        },
        repo: RepoRef {
            host: row.get("repo_host")?,
            owner: row.get("repo_owner")?,
            name: row.get("repo_name")?,
        },
        number,
        title: row.get("title")?,
        url: row.get("url")?,
        author_login: row.get("author_login")?,
        body: row.get("body")?,
        state: parse_enum(row.get("state")?)?,
        is_draft: parse_bool(row.get("is_draft")?),
        head_ref: row.get("head_ref")?,
        base_ref: row.get("base_ref")?,
        head_sha: row.get("head_sha")?,
        labels: Vec::new(),
        review_decision: parse_opt_enum(row.get("review_decision")?)?,
        checks: parse_enum(row.get("checks_state")?)?,
        mergeable: parse_enum(row.get("mergeable")?)?,
        merge_state: parse_enum(row.get("merge_state")?)?,
        requested_reviewers: Vec::new(),
        unresolved_threads: row.get::<_, i64>("unresolved_threads")? as u32,
        total_threads: row.get::<_, i64>("total_threads")? as u32,
        involvement,
        created_at: ts_col(row.get("created_at")?)?,
        updated_at: ts_col(row.get("updated_at")?)?,
        last_activity_at: ts_col(row.get("last_activity_at")?)?,
        synced_at: ts_col(row.get("synced_at")?)?,
        hidden: parse_bool(row.get("hidden")?),
    })
}

fn map_reviewer(row: &rusqlite::Row<'_>) -> rusqlite::Result<Reviewer> {
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

fn cap_body(body: &str) -> String {
    if body.len() <= PR_BODY_MAX_BYTES {
        return body.to_owned();
    }
    let mut end = PR_BODY_MAX_BYTES;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    body[..end].to_owned()
}
