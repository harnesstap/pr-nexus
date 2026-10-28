//! The Branch card: REST compare for the behind count and the
//! `updatePullRequestBranch` mutation (§7.2).

use pr_nexus_core::branch::{BranchComparison, BranchRef, UpdateMethod};
use pr_nexus_core::domain::PullRequest;
use pr_nexus_core::error::ProviderError;
use serde_json::{Value, json};

use super::GithubClient;
use super::ownership::segment;
use super::parse::data;

const UPDATE: &str = include_str!("graphql/update_branch.graphql");

/// `GET /repos/{o}/{r}/compare/{base_owner}:{base}...{head_owner}:{head}`,
/// owner-qualified on both sides so a fork's head resolves.
pub(crate) fn compare(
    client: &GithubClient,
    pr: &PullRequest,
    base: &BranchRef,
    head: &BranchRef,
) -> Result<BranchComparison, ProviderError> {
    let path = format!(
        "/repos/{}/{}/compare/{}:{}...{}:{}",
        segment(&pr.repo.owner),
        segment(&pr.repo.name),
        segment(&base.owner),
        segment(&base.name),
        segment(&head.owner),
        segment(&head.name),
    );
    let body = client.rest_ok(&path, &[("per_page", "100")])?;
    parse_compare(&body)
}

pub(crate) fn parse_compare(body: &str) -> Result<BranchComparison, ProviderError> {
    let value: Value =
        serde_json::from_str(body).map_err(|err| ProviderError::Decode(err.to_string()))?;
    let count = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_u64)
            .map(|found| found as u32)
            .ok_or_else(|| ProviderError::Decode(format!("compare: missing {key}")))
    };
    let mut authors: Vec<String> = Vec::new();
    for commit in value
        .get("commits")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let login = commit
            .pointer("/author/login")
            .and_then(Value::as_str)
            .or_else(|| {
                commit
                    .pointer("/commit/author/name")
                    .and_then(Value::as_str)
            });
        if let Some(login) = login
            && !authors.iter().any(|seen| seen.eq_ignore_ascii_case(login))
        {
            authors.push(login.to_owned());
        }
    }
    Ok(BranchComparison {
        behind_by: count("behind_by")?,
        ahead_by: count("ahead_by")?,
        authors,
    })
}

/// GraphQL `updatePullRequestBranch`, guarded by `expected_head`.
pub(crate) fn update(
    client: &GithubClient,
    pr: &PullRequest,
    expected_head: &str,
    method: UpdateMethod,
) -> Result<(), ProviderError> {
    let body = client.graphql(
        UPDATE,
        json!({
            "id": pr.remote.id,
            "expectedHeadOid": expected_head,
            "method": method.graphql(),
        }),
    )?;
    data(&body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_counts_and_dedupes_authors() {
        let body = r#"{"status":"diverged","ahead_by":3,"behind_by":5,"commits":[
          {"author":{"login":"alice"},"commit":{"author":{"name":"Alice"}}},
          {"author":null,"commit":{"author":{"name":"Build Robot"}}},
          {"author":{"login":"ALICE"},"commit":{"author":{"name":"Alice"}}}
        ]}"#;
        let found = parse_compare(body).unwrap();
        assert_eq!(found.behind_by, 5);
        assert_eq!(found.ahead_by, 3);
        assert_eq!(found.authors, vec!["alice", "Build Robot"]);
        assert!(parse_compare("{}").is_err());
    }
}
