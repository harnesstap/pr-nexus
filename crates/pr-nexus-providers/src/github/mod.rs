//! Blocking GitHub GraphQL client.
//!
//! Search uses `search(type: ISSUE)` with the cheap fields only and follows
//! every page; past GitHub's 1000-result limit it splits the query by
//! creation date (see [`search`]). The query strings come from sync, already
//! scoped to the tracked orgs. The slow fields (mergeability, checks, review decision,
//! threads) come from `nodes` in [`VcsProvider::pull_request_status`]. Every
//! page shrinks and retries when GitHub times out (502/503/504 or a GraphQL
//! timeout error), and waits out a secondary rate limit before retrying.

mod auth;
mod http;
mod ownership;
mod parse;
pub mod search;

pub use auth::{
    DevicePoll, DeviceStart, GhProbe, github_token_from_env, oauth_client_id_from_env,
    poll_device_flow, read_gh_token, start_device_flow,
};

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pr_nexus_core::domain::{AccountId, Involvement, PullRequest, TeamRef, VcsKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{
    OwnershipInputs, OwnershipProbe, PrQuery, PrStatus, RateBudget, SearchBucket,
    SyncedPullRequest, TeamMembers, VcsProvider, VcsUser,
};
use pr_nexus_core::sync::PAGE_SIZES;
use serde_json::{Value, json};

const SEARCH: &str = include_str!("graphql/search.graphql");
const REFRESH: &str = include_str!("graphql/refresh.graphql");
const STATUS: &str = include_str!("graphql/status.graphql");
const VIEWER: &str = include_str!("graphql/viewer.graphql");

/// Longest one GitHub request may take. GitHub itself gives up on a GraphQL
/// query after about 10 s, so 15 s leaves room for the 502 page to arrive
/// and still fails fast when the network stalls.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Tries per page: one at each of `PAGE_SIZES` (10 → 5 → 3).
const MAX_ATTEMPTS: u32 = 3;

/// Requests one client sends at once (SPEC §6: at most 2 per host). The gate
/// enforces it; the status pass uses it to run chunks side by side.
const CONCURRENT_REQUESTS: usize = 2;

type StatusAnswer = Result<Vec<PrStatus>, ProviderError>;

/// The first wait before a retry; each later wait doubles it (1 s, 2 s).
const FIRST_BACKOFF: Duration = Duration::from_secs(1);

/// Waits on a secondary rate limit before a page gives up.
const MAX_RATE_WAITS: u32 = 3;

/// Longest single wait on a secondary rate limit. GitHub usually asks for
/// a minute; a longer ask ends the sync, which the scheduler then pauses.
const RATE_WAIT_CAP: Duration = Duration::from_secs(120);

/// Upper bound for one page when every try times out:
/// 3 × 15 s + 1 s + 2 s = 48 s. `paged` never waits longer than this.
pub fn worst_case_page_time() -> Duration {
    let waits: Duration = (1..MAX_ATTEMPTS)
        .map(|attempt| FIRST_BACKOFF * 2u32.pow(attempt - 1))
        .sum();
    REQUEST_TIMEOUT * MAX_ATTEMPTS + waits
}

pub struct GithubClient {
    http: reqwest::blocking::Client,
    api: String,
    token: String,
    account: AccountId,
    gate: Arc<http::Gate>,
    budget: Mutex<Option<RateBudget>>,
    backoff: Duration,
    rate_wait_cap: Duration,
}

impl GithubClient {
    pub fn new(account: AccountId, token: impl Into<String>) -> Result<Self, ProviderError> {
        Self::with_base("https://api.github.com", account, token)
    }

    pub fn with_base(
        api: impl Into<String>,
        account: AccountId,
        token: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let http = reqwest::blocking::Client::builder()
            .user_agent("pr-nexus")
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        Ok(Self {
            http,
            api: api.into().trim_end_matches('/').to_owned(),
            token: token.into(),
            account,
            gate: Arc::new(http::Gate::new(CONCURRENT_REQUESTS)),
            budget: Mutex::new(None),
            backoff: FIRST_BACKOFF,
            rate_wait_cap: RATE_WAIT_CAP,
        })
    }

    /// The first wait before retrying a page GitHub timed out on. Each later
    /// wait doubles it. Tests set it to zero.
    pub fn with_backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    /// The longest wait on a secondary rate limit before retrying. Tests set
    /// it to zero.
    pub fn with_rate_wait_cap(mut self, cap: Duration) -> Self {
        self.rate_wait_cap = cap;
        self
    }

    pub fn http(&self) -> &reqwest::blocking::Client {
        &self.http
    }

    fn graphql(&self, query: &str, variables: Value) -> Result<Value, ProviderError> {
        let _permit = http::Permit::acquire(&self.gate);
        let response = self
            .http
            .post(format!("{}/graphql", self.api))
            .bearer_auth(&self.token)
            .header("Accept", "application/json")
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        let status = response.status().as_u16();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let sso = response
            .headers()
            .get("x-github-sso")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .text()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(http::classify(
                status,
                retry.as_deref(),
                sso.as_deref(),
                &body,
                chrono::Utc::now(),
            ));
        }
        let value: Value = serde_json::from_str(&body)
            .map_err(|err| ProviderError::Decode(format!("graphql json: {err}")))?;
        if let Some(budget) = parse::parse_budget(&value) {
            *self.budget.lock().expect("rate budget") = Some(budget);
        }
        if let Some(err) = parse::graphql_failure(&value) {
            return Err(err);
        }
        Ok(value)
    }
}

impl GithubClient {
    /// Runs one page with `PAGE_SIZES[*level]`. When GitHub times out, waits,
    /// moves `level` to the next smaller size (it stays there for later pages),
    /// and tries again, at most `MAX_ATTEMPTS` times. On a secondary rate
    /// limit it waits as GitHub asks, doubling each time (SPEC §6), at most
    /// `MAX_RATE_WAITS` times.
    fn paged<T>(
        &self,
        what: &str,
        level: &mut usize,
        mut call: impl FnMut(usize) -> Result<T, ProviderError>,
    ) -> Result<T, ProviderError> {
        let mut attempt = 1;
        let mut rate_waits = 0;
        loop {
            let size = PAGE_SIZES[*level] as usize;
            match call(size) {
                Ok(value) => return Ok(value),
                Err(ProviderError::RateLimited {
                    reset_at,
                    secondary: true,
                }) if rate_waits < MAX_RATE_WAITS => {
                    let asked = (reset_at - chrono::Utc::now())
                        .to_std()
                        .unwrap_or(Duration::ZERO)
                        .max(Duration::from_secs(1));
                    let wait = (asked * 2u32.pow(rate_waits)).min(self.rate_wait_cap);
                    std::thread::sleep(wait);
                    rate_waits += 1;
                }
                Err(err) if http::is_timeout(&err) => {
                    if attempt >= MAX_ATTEMPTS {
                        return Err(http::timed_out(&err, what, attempt, size));
                    }
                    std::thread::sleep(self.backoff * 2u32.pow(attempt - 1));
                    *level = (*level + 1).min(PAGE_SIZES.len() - 1);
                    attempt += 1;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Runs `document` (a `nodes(ids: $ids)` query) over `ids` in chunks of
    /// the current page size, shrinking like [`Self::paged`].
    fn nodes_paged(
        &self,
        what: &str,
        document: &str,
        ids: &[pr_nexus_core::domain::ProviderRef],
        mut each: impl FnMut(&Value) -> Result<(), ProviderError>,
    ) -> Result<(), ProviderError> {
        let mut level = 0;
        let mut start = 0;
        while start < ids.len() {
            let mut taken = 0;
            let body = self.paged(what, &mut level, |size| {
                taken = size.min(ids.len() - start);
                let id_list: Vec<&str> = ids[start..start + taken]
                    .iter()
                    .map(|id| id.id.as_str())
                    .collect();
                self.graphql(document, json!({ "ids": id_list }))
            })?;
            each(&body)?;
            start += taken;
        }
        Ok(())
    }
}

impl GithubClient {
    /// A REST `GET` that returns the body of a 2xx and a classified error otherwise.
    fn rest_ok(&self, path: &str, query: &[(&str, &str)]) -> Result<String, ProviderError> {
        let _permit = http::Permit::acquire(&self.gate);
        let response = self
            .http
            .get(format!("{}{path}", self.api))
            .query(query)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        let status = response.status().as_u16();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let sso = response
            .headers()
            .get("x-github-sso")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .text()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(http::classify(
                status,
                retry.as_deref(),
                sso.as_deref(),
                &body,
                chrono::Utc::now(),
            ));
        }
        Ok(body)
    }

    /// `GET /user/orgs`, following pagination (100 per page, at most 10 pages).
    fn rest_user_orgs(&self) -> Result<Vec<String>, ProviderError> {
        let mut orgs = Vec::new();
        for page in 1..=10 {
            let body = self.rest_ok(
                "/user/orgs",
                &[("per_page", "100"), ("page", &page.to_string())],
            )?;
            let batch = parse::parse_rest_orgs(&body)?;
            let full = batch.len() == 100;
            orgs.extend(batch);
            if !full {
                break;
            }
        }
        Ok(orgs)
    }
}

impl VcsProvider for GithubClient {
    fn kind(&self) -> VcsKind {
        VcsKind::GitHub
    }

    fn viewer(&self) -> Result<VcsUser, ProviderError> {
        let body = self.graphql(VIEWER, json!({}))?;
        let mut user = parse::parse_viewer(&body)?;
        // REST lists every membership; GraphQL hides SSO-protected orgs the
        // token is not authorized for. A REST failure keeps the GraphQL list.
        if let Ok(rest) = self.rest_user_orgs() {
            parse::merge_orgs(&mut user, rest);
        }
        Ok(user)
    }

    fn search_pull_requests(
        &self,
        query: &PrQuery,
    ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        self.search_pull_requests_paged(query, &mut |_| {})
    }

    /// Every result, however many pages: no cap, and GitHub's 1000-result
    /// limit is lifted by splitting the query by creation date.
    fn search_pull_requests_paged(
        &self,
        query: &PrQuery,
        on_page: &mut dyn FnMut(&[SyncedPullRequest]),
    ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        let involvement = match query.bucket {
            SearchBucket::Mine => Involvement::AUTHOR,
            SearchBucket::ReviewRequested => Involvement::REVIEW_REQUESTED,
        };
        let what = match query.bucket {
            SearchBucket::Mine => "search for your pull requests",
            SearchBucket::ReviewRequested => "search for review requests",
        };
        let mut seen = std::collections::HashSet::new();
        let mut items: Vec<SyncedPullRequest> = Vec::new();
        for q in &query.queries {
            self.search_all(what, q, involvement, &mut seen, &mut |page| {
                on_page(&page);
                items.extend(page);
            })?;
        }
        Ok(items)
    }

    fn refresh_pull_requests(
        &self,
        ids: &[pr_nexus_core::domain::ProviderRef],
    ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        let mut items = Vec::new();
        self.nodes_paged(
            "refresh of pull requests that left the search",
            REFRESH,
            ids,
            |body| {
                items.extend(parse::parse_nodes(body, self.account)?);
                Ok(())
            },
        )?;
        Ok(items)
    }

    /// Chunks of `PAGE_SIZES[0]` ids, [`CONCURRENT_REQUESTS`] at a time: GitHub
    /// spends about 0.3 s per pull request on mergeability, so two chunks in
    /// flight halve the wall time. Each chunk shrinks on its own after a timeout.
    fn pull_request_status(
        &self,
        ids: &[pr_nexus_core::domain::ProviderRef],
    ) -> Result<Vec<PrStatus>, ProviderError> {
        let chunks: Vec<_> = ids.chunks(PAGE_SIZES[0] as usize).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let answers: Mutex<Vec<Option<StatusAnswer>>> =
            Mutex::new((0..chunks.len()).map(|_| None).collect());
        std::thread::scope(|scope| {
            for _ in 0..CONCURRENT_REQUESTS.min(chunks.len()) {
                scope.spawn(|| {
                    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let Some(chunk) = chunks.get(index) else {
                            break;
                        };
                        let mut items = Vec::new();
                        let answer = self
                            .nodes_paged("status of pull requests", STATUS, chunk, |body| {
                                items.extend(parse::parse_status(body)?);
                                Ok(())
                            })
                            .map(|()| items);
                        if answer.is_err() {
                            stop.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        answers.lock().expect("status answers")[index] = Some(answer);
                    }
                });
            }
        });
        let mut items = Vec::new();
        for answer in answers.into_inner().expect("status answers") {
            match answer {
                Some(Ok(chunk)) => items.extend(chunk),
                Some(Err(err)) => return Err(err),
                // Left untaken after another chunk failed; that error is returned.
                None => {}
            }
        }
        Ok(items)
    }

    fn ownership_probe(&self, pr: &PullRequest) -> Result<OwnershipProbe, ProviderError> {
        ownership::probe(self, pr)
    }

    fn ownership_inputs(
        &self,
        pr: &PullRequest,
        probe: &OwnershipProbe,
    ) -> Result<OwnershipInputs, ProviderError> {
        ownership::inputs(self, pr, probe)
    }

    fn team_members(&self, team: &TeamRef) -> Result<TeamMembers, ProviderError> {
        ownership::team_members(self, &team.org, &team.slug)
    }

    fn rate_limit(&self) -> Result<RateBudget, ProviderError> {
        if let Some(budget) = self.budget.lock().expect("rate budget").clone() {
            return Ok(budget);
        }
        let body = self.graphql(VIEWER, json!({}))?;
        self.budget
            .lock()
            .expect("rate budget")
            .clone()
            .or_else(|| parse::parse_budget(&body))
            .ok_or_else(|| ProviderError::Decode("rate limit missing".into()))
    }

    fn request_team_review(
        &self,
        _pr: &PullRequest,
        _teams: &[TeamRef],
    ) -> Result<(), ProviderError> {
        Err(ProviderError::Unsupported(
            "requesting GitHub reviewers arrives in M10".into(),
        ))
    }
}
