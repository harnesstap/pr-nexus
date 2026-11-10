//! Runs a GitHub pull-request query the user typed and stores the hits.
//!
//! The query is prepared in core (`@today`, `sort:`, `@me`, `is:pr`), then
//! each connected GitHub account searches its own host. Hits are merged onto
//! rows that already exist so a search does not wipe review status, labels,
//! or how the user is involved.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use pr_nexus_core::domain::{
    Account, AccountId, ChecksState, MergeStateStatus, Mergeable, PullRequest, Repository,
};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::github_query::{self, QueryClock, SearchOrder};
use pr_nexus_core::providers::SyncedPullRequest;
use pr_nexus_providers::github::GithubClient;

use crate::error::AppError;
use crate::state::{self, AppState};

const RESULT_CAP: usize = 1000;

pub struct GithubQueryInput {
    pub query: String,
    pub repos: Vec<String>,
    pub offset_minutes: i32,
    pub limit: usize,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubQueryDto {
    pub ok: bool,
    pub message: String,
    pub ids: Vec<i64>,
    pub total: u64,
    /// The query asks only for open pull requests.
    pub open_only: bool,
}

struct Held {
    synced: SyncedPullRequest,
    comments: u32,
    account: AccountId,
}

pub fn run(state: &AppState, input: GithubQueryInput) -> GithubQueryDto {
    match search(state, input) {
        Ok(dto) => dto,
        Err(err) => fail(&calm(&err)),
    }
}

fn search(state: &AppState, input: GithubQueryInput) -> Result<GithubQueryDto, AppError> {
    if input.query.trim().is_empty() {
        return Ok(empty_query());
    }
    // Free text never reaches GitHub. Only qualifier tokens do, and a field
    // that is only free text does not search at all.
    let parts = match github_query::partition(&input.query) {
        Ok(parts) => parts,
        Err(err) => return Ok(fail(&err.message())),
    };
    if parts.github.is_empty() {
        return Ok(empty_query());
    }
    if state.demo {
        return Ok(fail("GitHub search is off in demo mode."));
    }
    let accounts = state::github_accounts(&state.store)?;
    if accounts.is_empty() {
        return Ok(fail("Connect GitHub to search."));
    }
    let clock = QueryClock {
        now: Utc::now(),
        offset_east_minutes: input.offset_minutes,
    };
    let limit = input.limit.clamp(1, RESULT_CAP);
    let mut plans = Vec::new();
    for account in &accounts {
        let prepared = match github_query::prepare(&parts.github, &account.login, &clock) {
            Ok(prepared) => prepared,
            Err(err) => return Ok(fail(&err.message())),
        };
        let packed = match github_query::and_repos(&prepared.q, &input.repos) {
            Ok(packed) => packed,
            Err(err) => return Ok(fail(&err.message())),
        };
        plans.push((prepared, packed));
    }
    let open_only = plans
        .first()
        .is_some_and(|(prepared, _)| prepared.open_only);
    let order = plans
        .first()
        .and_then(|(prepared, _)| prepared.order.clone());

    let mut held = Vec::new();
    let mut total = 0u64;
    let mut saw_ok = false;
    let mut first_fail: Option<String> = None;
    for (account, (prepared, packed)) in accounts.iter().zip(plans) {
        let sort = prepared.order.as_ref().map(|order| order.sort.as_str());
        let descending = prepared.order.as_ref().is_none_or(|order| order.descending);
        match search_account(state, account, &packed, sort, descending, limit) {
            Ok(found) => {
                saw_ok = true;
                total += found.total;
                held.extend(found.hits);
            }
            Err(SearchFail::Rejected(message)) => return Ok(fail(&message)),
            Err(SearchFail::Other(message)) => {
                if first_fail.is_none() {
                    first_fail = Some(message);
                }
            }
        }
    }
    if !saw_ok {
        return Ok(fail(
            &first_fail.unwrap_or_else(|| "GitHub search failed.".to_owned()),
        ));
    }
    dedupe(&mut held);
    sort_hits(&mut held, order.as_ref());
    held.truncate(limit);
    let ids = save_hits(state, &held)?;
    Ok(GithubQueryDto {
        ok: true,
        message: String::new(),
        ids,
        total,
        open_only,
    })
}

struct AccountHits {
    total: u64,
    hits: Vec<Held>,
}

enum SearchFail {
    /// GitHub rejected the query. Drop every account's hits.
    Rejected(String),
    Other(String),
}

/// `Err` is a plain sentence. A 422 fails the whole query.
fn search_account(
    state: &AppState,
    account: &Account,
    packed: &[String],
    sort: Option<&str>,
    descending: bool,
    limit: usize,
) -> Result<AccountHits, SearchFail> {
    let token = state::resolve_github_token(state, Some(account))
        .map_err(|err| SearchFail::Other(calm(&err)))?;
    let client = GithubClient::with_base(api_base(&account.host), account.id, token)
        .map_err(|err| SearchFail::Other(plain_provider(&err)))?;
    let host = display_host(&account.host);
    let mut hits = Vec::new();
    let mut total = 0u64;
    for query in packed {
        match client.search_issues(query, sort, descending, host, limit) {
            Ok(page) => {
                total += page.total;
                hits.extend(page.hits.into_iter().map(|hit| Held {
                    synced: hit.synced,
                    comments: hit.comments,
                    account: account.id,
                }));
            }
            Err(err) if is_validation(&err) => {
                return Err(SearchFail::Rejected(plain_provider(&err)));
            }
            Err(err) => return Err(SearchFail::Other(plain_provider(&err))),
        }
    }
    Ok(AccountHits { total, hits })
}

fn is_validation(err: &ProviderError) -> bool {
    matches!(err, ProviderError::Api { code, .. } if code == "422")
}

fn dedupe(hits: &mut Vec<Held>) {
    let mut seen = HashSet::new();
    hits.retain(|hit| seen.insert(hit.synced.pull_request.remote.id.clone()));
}

fn sort_hits(hits: &mut [Held], order: Option<&SearchOrder>) {
    let Some(order) = order else {
        return;
    };
    let key = match order.sort.as_str() {
        "updated" | "created" | "comments" => order.sort.as_str(),
        _ => return,
    };
    let descending = order.descending;
    hits.sort_by(|left, right| {
        let cmp = match key {
            "created" => left
                .synced
                .pull_request
                .created_at
                .cmp(&right.synced.pull_request.created_at),
            "comments" => left.comments.cmp(&right.comments),
            _ => left
                .synced
                .pull_request
                .updated_at
                .cmp(&right.synced.pull_request.updated_at),
        };
        if descending { cmp.reverse() } else { cmp }
    });
}

fn save_hits(state: &AppState, hits: &[Held]) -> Result<Vec<i64>, AppError> {
    let mut by_account: HashMap<AccountId, Vec<usize>> = HashMap::new();
    for (index, hit) in hits.iter().enumerate() {
        by_account.entry(hit.account).or_default().push(index);
    }
    let mut ids = vec![0; hits.len()];
    for (account, indexes) in by_account {
        let existing = state.store.list_pull_requests_for_account(account)?;
        let mut by_node: HashMap<String, PullRequest> = existing
            .into_iter()
            .map(|pr| (pr.remote.id.clone(), pr))
            .collect();
        let mut repo_ids: HashMap<(String, String), i64> = state
            .store
            .list_repositories(account)?
            .into_iter()
            .map(|repo| {
                (
                    (
                        repo.owner.to_ascii_lowercase(),
                        repo.name.to_ascii_lowercase(),
                    ),
                    repo.id,
                )
            })
            .collect();
        for index in indexes {
            let incoming = hits[index].synced.pull_request.clone();
            let node = incoming.remote.id.clone();
            let pr = merge_hit(by_node.remove(&node), incoming);
            let key = (
                pr.repo.owner.to_ascii_lowercase(),
                pr.repo.name.to_ascii_lowercase(),
            );
            let repo_id = if let Some(id) = repo_ids.get(&key).copied() {
                id
            } else {
                let saved = state.store.insert_repository(Repository {
                    id: 0,
                    account_id: account,
                    owner: pr.repo.owner.clone(),
                    name: pr.repo.name.clone(),
                    node_id: None,
                    default_branch: None,
                    archived: false,
                })?;
                repo_ids.insert(key, saved.id);
                saved.id
            };
            ids[index] = state.store.upsert_pull_request(repo_id, pr)?.id.0;
        }
    }
    Ok(ids)
}

/// Keeps review status, labels, reviewers, and involvement when the search
/// document does not carry them. State, title, and the other identity fields
/// come from the search.
fn merge_hit(existing: Option<PullRequest>, mut incoming: PullRequest) -> PullRequest {
    let Some(kept) = existing else {
        return incoming;
    };
    incoming.involvement |= kept.involvement;
    if incoming.head_ref.is_empty() {
        incoming.head_ref = kept.head_ref;
    }
    if incoming.base_ref.is_empty() {
        incoming.base_ref = kept.base_ref;
    }
    if incoming.head_sha.is_empty() {
        incoming.head_sha = kept.head_sha;
    }
    if incoming.labels.is_empty() {
        incoming.labels = kept.labels;
    }
    if incoming.review_decision.is_none() {
        incoming.review_decision = kept.review_decision;
    }
    if incoming.checks == ChecksState::None {
        incoming.checks = kept.checks;
    }
    if incoming.mergeable == Mergeable::Unknown {
        incoming.mergeable = kept.mergeable;
    }
    if incoming.merge_state == MergeStateStatus::Unknown {
        incoming.merge_state = kept.merge_state;
    }
    if incoming.unresolved_threads == 0 {
        incoming.unresolved_threads = kept.unresolved_threads;
    }
    if incoming.total_threads == 0 {
        incoming.total_threads = kept.total_threads;
    }
    if incoming.requested_reviewers.is_empty() {
        incoming.requested_reviewers = kept.requested_reviewers;
    }
    incoming.hidden = kept.hidden;
    incoming
}

fn api_base(host: &str) -> String {
    let host = host.trim();
    if host.is_empty()
        || host.eq_ignore_ascii_case("github.com")
        || host.eq_ignore_ascii_case("api.github.com")
        || !plain_host(host)
    {
        "https://api.github.com".to_owned()
    } else {
        format!("https://{host}/api/v3")
    }
}

fn display_host(host: &str) -> &str {
    let host = host.trim();
    if host.is_empty() || !plain_host(host) {
        "github.com"
    } else {
        host
    }
}

fn plain_host(host: &str) -> bool {
    !host.is_empty()
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
}

fn plain_provider(err: &ProviderError) -> String {
    match err {
        ProviderError::Api { code, message } if code == "422" => {
            format!("GitHub could not run that query. {message}")
        }
        ProviderError::RateLimited { .. } => {
            "GitHub is rate limiting searches. Try again in a minute.".to_owned()
        }
        ProviderError::Unauthorized(_) => "Connect GitHub to search.".to_owned(),
        ProviderError::Network(_) => "GitHub could not be reached. Try again.".to_owned(),
        ProviderError::Forbidden(message) => message.clone(),
        ProviderError::NotFound(message) => message.clone(),
        ProviderError::Decode(message) => message.clone(),
        ProviderError::Unsupported(message) => message.clone(),
        ProviderError::Api { message, .. } => message.clone(),
        ProviderError::SentUnconfirmed(message) => message.clone(),
    }
}

fn calm(err: &AppError) -> String {
    if err.kind == "rate_limited" {
        "GitHub is rate limiting searches. Try again in a minute.".to_owned()
    } else if err.kind == "unauthorized" {
        "Connect GitHub to search.".to_owned()
    } else {
        err.message.clone()
    }
}

fn empty_query() -> GithubQueryDto {
    GithubQueryDto {
        ok: true,
        message: String::new(),
        ids: Vec::new(),
        total: 0,
        open_only: true,
    }
}

fn fail(message: &str) -> GithubQueryDto {
    GithubQueryDto {
        ok: false,
        message: message.to_owned(),
        ids: Vec::new(),
        total: 0,
        open_only: true,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use pr_nexus_core::domain::{
        AccountKind, AuthMode, Involvement, PrId, PrState, ProviderRef, RepoRef, ReviewDecision,
    };

    use super::*;

    fn state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::for_tests(&dir.path().join("pr-nexus.db"), false);
        (dir, state)
    }

    fn account(state: &AppState, login: &str) -> Account {
        state
            .store
            .insert_account(Account {
                id: AccountId(0),
                kind: AccountKind::GitHub,
                display_name: login.to_owned(),
                host: "github.com".to_owned(),
                external_id: format!("gh-{login}"),
                slack_workspace: None,
                login: login.to_owned(),
                auth_mode: AuthMode::Pat,
                keychain_key: None,
                created_at: Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap(),
            })
            .unwrap()
    }

    #[test]
    fn free_text_does_not_search_github() {
        let (_dir, state) = state();
        let plain = run(
            &state,
            GithubQueryInput {
                query: "passkeys".into(),
                repos: Vec::new(),
                offset_minutes: 0,
                limit: 100,
            },
        );
        assert!(plain.ok);
        assert!(plain.ids.is_empty());
        assert!(plain.message.is_empty());
        let mixed = run(
            &state,
            GithubQueryInput {
                query: "author:octocat passkeys".into(),
                repos: Vec::new(),
                offset_minutes: 0,
                limit: 100,
            },
        );
        assert!(!mixed.ok);
        assert_eq!(mixed.message, "Connect GitHub to search.");
    }

    #[test]
    fn a_bad_query_is_a_plain_sentence() {
        let (_dir, state) = state();
        account(&state, "octocat");
        let dto = run(
            &state,
            GithubQueryInput {
                query: "label:\"bug".into(),
                repos: Vec::new(),
                offset_minutes: -480,
                limit: 100,
            },
        );
        assert!(!dto.ok);
        assert_eq!(dto.message, "The query has an unfinished quote.");
        assert!(dto.ids.is_empty());
        assert!(!dto.message.contains("422"));
    }

    #[test]
    fn no_account_and_demo_stay_plain() {
        let (_dir, state) = state();
        let dto = run(
            &state,
            GithubQueryInput {
                query: "author:octocat".into(),
                repos: Vec::new(),
                offset_minutes: 0,
                limit: 100,
            },
        );
        assert!(!dto.ok);
        assert_eq!(dto.message, "Connect GitHub to search.");
        let (_dir, demo) = {
            let dir = tempfile::tempdir().unwrap();
            let state = AppState::for_tests(&dir.path().join("pr-nexus.db"), true);
            (dir, state)
        };
        let dto = run(
            &demo,
            GithubQueryInput {
                query: "author:octocat".into(),
                repos: Vec::new(),
                offset_minutes: 0,
                limit: 100,
            },
        );
        assert_eq!(dto.message, "GitHub search is off in demo mode.");
    }

    #[test]
    fn an_empty_repository_overlap_does_not_call_github() {
        let (_dir, state) = state();
        account(&state, "octocat");
        let dto = run(
            &state,
            GithubQueryInput {
                query: "repo:acme/widgets author:octocat".into(),
                repos: vec!["acme-labs/widgets".into()],
                offset_minutes: 0,
                limit: 100,
            },
        );
        assert!(!dto.ok);
        assert_eq!(
            dto.message,
            "No repository is in both the query and the repository filter."
        );
    }

    #[test]
    fn saving_a_hit_keeps_review_details_and_can_insert() {
        let (_dir, state) = state();
        let saved = account(&state, "octocat");
        let repo = state
            .store
            .insert_repository(Repository {
                id: 0,
                account_id: saved.id,
                owner: "acme".into(),
                name: "widgets".into(),
                node_id: Some("R_widgets".into()),
                default_branch: Some("main".into()),
                archived: false,
            })
            .unwrap();
        let existing = state
            .store
            .upsert_pull_request(
                repo.id,
                PullRequest {
                    id: PrId(0),
                    remote: ProviderRef {
                        account: saved.id,
                        id: "PR_node".into(),
                    },
                    repo: RepoRef {
                        host: "github.com".into(),
                        owner: "acme".into(),
                        name: "widgets".into(),
                    },
                    number: 7,
                    title: "Old title".into(),
                    url: "https://github.com/acme/widgets/pull/7".into(),
                    author_login: "octocat".into(),
                    body: "old".into(),
                    state: PrState::Open,
                    is_draft: false,
                    head_ref: "topic".into(),
                    base_ref: "main".into(),
                    head_sha: "abc".into(),
                    labels: vec!["bug".into()],
                    review_decision: Some(ReviewDecision::Approved),
                    checks: ChecksState::Success,
                    mergeable: Mergeable::Mergeable,
                    merge_state: MergeStateStatus::Clean,
                    requested_reviewers: Vec::new(),
                    unresolved_threads: 2,
                    total_threads: 4,
                    involvement: Involvement::AUTHOR,
                    created_at: Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap(),
                    updated_at: Utc.with_ymd_and_hms(2026, 11, 2, 0, 0, 0).unwrap(),
                    last_activity_at: Utc.with_ymd_and_hms(2026, 11, 2, 0, 0, 0).unwrap(),
                    synced_at: Utc.with_ymd_and_hms(2026, 11, 2, 0, 0, 0).unwrap(),
                    hidden: false,
                },
            )
            .unwrap();
        let merged = hit(&saved, "PR_node", PrState::Merged, 7, "Ship the widget");
        let fresh = hit(&saved, "PR_new", PrState::Closed, 8, "A closed one");
        let ids = save_hits(&state, &[merged, fresh]).unwrap();
        assert_eq!(ids.len(), 2);
        let kept = state.store.get_pull_request(existing.id).unwrap().unwrap();
        assert_eq!(kept.state, PrState::Merged);
        assert_eq!(kept.title, "Ship the widget");
        assert!(kept.involvement.contains(Involvement::AUTHOR));
        assert_eq!(kept.checks, ChecksState::Success);
        assert_eq!(kept.labels, vec!["bug".to_owned()]);
        assert_eq!(kept.head_ref, "topic");
        assert_eq!(kept.unresolved_threads, 2);
        let repos = state.store.list_repositories(saved.id).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].node_id.as_deref(), Some("R_widgets"));
        assert_eq!(repos[0].default_branch.as_deref(), Some("main"));
        let inserted = state.store.get_pull_request(PrId(ids[1])).unwrap().unwrap();
        assert_eq!(inserted.state, PrState::Closed);
        assert_eq!(inserted.number, 8);
        assert!(inserted.involvement.is_empty());
    }

    fn hit(account: &Account, node: &str, pr_state: PrState, number: u64, title: &str) -> Held {
        Held {
            comments: 1,
            account: account.id,
            synced: SyncedPullRequest {
                repo_node_id: None,
                default_branch: None,
                repo_archived: false,
                merge_queue: None,
                pull_request: PullRequest {
                    id: PrId(0),
                    remote: ProviderRef {
                        account: account.id,
                        id: node.to_owned(),
                    },
                    repo: RepoRef {
                        host: "github.com".into(),
                        owner: "acme".into(),
                        name: "widgets".into(),
                    },
                    number,
                    title: title.to_owned(),
                    url: format!("https://github.com/acme/widgets/pull/{number}"),
                    author_login: "octocat".into(),
                    body: "from search".into(),
                    state: pr_state,
                    is_draft: false,
                    head_ref: String::new(),
                    base_ref: String::new(),
                    head_sha: String::new(),
                    labels: Vec::new(),
                    review_decision: None,
                    checks: ChecksState::None,
                    mergeable: Mergeable::Unknown,
                    merge_state: MergeStateStatus::Unknown,
                    requested_reviewers: Vec::new(),
                    unresolved_threads: 0,
                    total_threads: 0,
                    involvement: Involvement::empty(),
                    created_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
                    updated_at: Utc.with_ymd_and_hms(2026, 11, 8, 0, 0, 0).unwrap(),
                    last_activity_at: Utc.with_ymd_and_hms(2026, 11, 8, 0, 0, 0).unwrap(),
                    synced_at: Utc::now(),
                    hidden: false,
                },
            },
        }
    }

    #[test]
    fn enterprise_hosts_use_their_own_api() {
        assert_eq!(api_base("github.com"), "https://api.github.com");
        assert_eq!(api_base(""), "https://api.github.com");
        assert_eq!(
            api_base("ghe.example.test"),
            "https://ghe.example.test/api/v3"
        );
        assert_eq!(api_base("not a host"), "https://api.github.com");
    }
}
