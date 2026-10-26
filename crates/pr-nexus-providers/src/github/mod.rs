//! Blocking GitHub GraphQL client.
//!
//! Search uses `search(type: ISSUE)` with a cap of 500 per bucket. Org
//! filtering is not done here.

mod auth;
mod http;
mod ownership;
mod parse;

pub use auth::{
    DevicePoll, DeviceStart, GhProbe, github_token_from_env, oauth_client_id_from_env,
    poll_device_flow, read_gh_token, start_device_flow,
};

use std::sync::{Arc, Mutex};

use pr_nexus_core::domain::{AccountId, Involvement, PullRequest, TeamRef, VcsKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{
    OwnershipInputs, OwnershipProbe, PrQuery, RateBudget, SearchBucket, SyncedPullRequest,
    TeamMembers, VcsProvider, VcsUser,
};
use pr_nexus_core::sync::{SEARCH_CAP, search_query};
use serde_json::{Value, json};

const SEARCH: &str = include_str!("graphql/search.graphql");
const REFRESH: &str = include_str!("graphql/refresh.graphql");
const VIEWER: &str = include_str!("graphql/viewer.graphql");

pub struct GithubClient {
    http: reqwest::blocking::Client,
    api: String,
    token: String,
    account: AccountId,
    gate: Arc<http::Gate>,
    budget: Mutex<Option<RateBudget>>,
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
            .timeout(http::timeout())
            .build()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        Ok(Self {
            http,
            api: api.into().trim_end_matches('/').to_owned(),
            token: token.into(),
            account,
            gate: Arc::new(http::Gate::new(2)),
            budget: Mutex::new(None),
        })
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
        let involvement = match query.bucket {
            SearchBucket::Mine => Involvement::AUTHOR,
            SearchBucket::ReviewRequested => Involvement::REVIEW_REQUESTED,
        };
        let q = search_query(query.bucket);
        let mut cursor = Value::Null;
        let mut items = Vec::new();
        loop {
            let body = self.graphql(SEARCH, json!({ "query": q, "cursor": cursor }))?;
            let page = parse::parse_search(&body, self.account, involvement)?;
            items.extend(page.items);
            if items.len() as u32 >= SEARCH_CAP || !page.has_next {
                items.truncate(SEARCH_CAP as usize);
                break;
            }
            cursor = page.cursor.map(Value::String).unwrap_or(Value::Null);
        }
        Ok(items)
    }

    fn refresh_pull_requests(
        &self,
        ids: &[pr_nexus_core::domain::ProviderRef],
    ) -> Result<Vec<SyncedPullRequest>, ProviderError> {
        let mut items = Vec::new();
        for chunk in ids.chunks(50) {
            let id_list: Vec<&str> = chunk.iter().map(|id| id.id.as_str()).collect();
            let body = self.graphql(REFRESH, json!({ "ids": id_list }))?;
            items.extend(parse::parse_nodes(&body, self.account)?);
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
