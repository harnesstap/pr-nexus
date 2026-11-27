//! Batch approval reads and the approving review mutation (§8.2 Approve pull requests).

use pr_nexus_core::approve::{ApprovalProbe, ApprovalProbes};
use pr_nexus_core::domain::{PrState, PullRequest};
use pr_nexus_core::error::ProviderError;
use serde_json::{Value, json};

use super::GithubClient;
use super::parse::data;

const PROBES: &str = include_str!("graphql/approval_probes.graphql");
const APPROVE: &str = include_str!("graphql/approve_pull_request.graphql");

/// GitHub's `nodes(ids:)` takes at most 100 ids.
const NODES_PER_QUERY: usize = 100;

pub(crate) fn probes(
    client: &GithubClient,
    prs: &[PullRequest],
) -> Result<ApprovalProbes, ProviderError> {
    let mut out = ApprovalProbes::default();
    for chunk in prs.chunks(NODES_PER_QUERY) {
        let ids: Vec<&str> = chunk.iter().map(|pr| pr.remote.id.as_str()).collect();
        let body = client.graphql(PROBES, json!({ "ids": ids }))?;
        let parsed = parse_probes(data(&body)?);
        if out.viewer_login.is_empty() {
            out.viewer_login = parsed.viewer_login;
        }
        out.probes.extend(parsed.probes);
    }
    Ok(out)
}

pub(crate) fn parse_probes(data: &Value) -> ApprovalProbes {
    let viewer_login = data
        .pointer("/viewer/login")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let probes = data
        .get("nodes")
        .and_then(Value::as_array)
        .map(|nodes| nodes.iter().filter_map(probe).collect())
        .unwrap_or_default();
    ApprovalProbes {
        viewer_login,
        probes,
    }
}

fn probe(node: &Value) -> Option<ApprovalProbe> {
    let text = |key: &str| node.get(key).and_then(Value::as_str).map(str::to_owned);
    let state = match text("state")?.to_ascii_uppercase().as_str() {
        "OPEN" => PrState::Open,
        "MERGED" => PrState::Merged,
        _ => PrState::Closed,
    };
    let review = node.get("viewerLatestReview").filter(|v| v.is_object());
    Some(ApprovalProbe {
        node_id: text("id")?,
        state,
        is_draft: node.get("isDraft").and_then(Value::as_bool) == Some(true),
        head_oid: text("headRefOid").unwrap_or_default(),
        author_login: node
            .pointer("/author/login")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        viewer_review_state: review
            .and_then(|r| r.get("state"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        viewer_review_commit: review
            .and_then(|r| r.pointer("/commit/oid"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// Approves `head_oid` exactly. An empty `body` sends no review text.
pub(crate) fn approve(
    client: &GithubClient,
    pr: &PullRequest,
    head_oid: &str,
    body: &str,
) -> Result<(), ProviderError> {
    let body = if body.is_empty() {
        Value::Null
    } else {
        Value::String(body.to_owned())
    };
    let response = client.graphql(
        APPROVE,
        json!({ "id": pr.remote.id, "commit": head_oid, "body": body }),
    )?;
    let state = data(&response)?
        .pointer("/addPullRequestReview/pullRequestReview/state")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !state.eq_ignore_ascii_case("APPROVED") {
        return Err(ProviderError::Api {
            code: "graphql".into(),
            message: format!("GitHub did not record an approval (review state {state:?})."),
        });
    }
    Ok(())
}
