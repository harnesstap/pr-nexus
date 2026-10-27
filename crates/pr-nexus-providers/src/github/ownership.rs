//! Ownership inputs: the probe query, changed files, rules, protection, and team members (§6).

use pr_nexus_core::domain::PullRequest;
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{
    ClassicProtection, CodeownersBlob, FILES_CAP, OwnershipInputs, OwnershipProbe, TeamMembers,
};
use pr_nexus_core::review::codeowners::CODEOWNERS_CANDIDATES;
use pr_nexus_core::review::owners::{OnBehalfOf, ReviewBundle, ReviewRecord};
use serde_json::{Value, json};

use super::GithubClient;
use super::parse::data;

const PROBE: &str = include_str!("graphql/ownership_probe.graphql");
const FILES: &str = include_str!("graphql/ownership_files.graphql");

/// Aliases in `ownership_probe.graphql`, in [`CODEOWNERS_CANDIDATES`] order.
const CODEOWNERS_ALIASES: [&str; 3] = ["githubDir", "root", "docs"];

/// Pages of 100 members. Past this the list is marked incomplete.
const MEMBER_PAGES: u32 = 10;

pub(crate) fn probe(
    client: &GithubClient,
    pr: &PullRequest,
) -> Result<OwnershipProbe, ProviderError> {
    let expression = |path: &str| format!("{}:{path}", pr.base_ref);
    let body = client.graphql(
        PROBE,
        json!({
            "owner": pr.repo.owner,
            "name": pr.repo.name,
            "number": pr.number,
            "githubDir": expression(CODEOWNERS_CANDIDATES[0]),
            "root": expression(CODEOWNERS_CANDIDATES[1]),
            "docs": expression(CODEOWNERS_CANDIDATES[2]),
        }),
    )?;
    parse_probe(&body)
}

pub(crate) fn parse_probe(body: &Value) -> Result<OwnershipProbe, ProviderError> {
    let repo = data(body)?
        .get("repository")
        .filter(|repo| !repo.is_null())
        .ok_or_else(|| ProviderError::NotFound("repository not found".into()))?;
    let pull = repo
        .get("pullRequest")
        .filter(|pull| !pull.is_null())
        .ok_or_else(|| ProviderError::NotFound("pull request not found".into()))?;
    let mut codeowners = None;
    for (alias, path) in CODEOWNERS_ALIASES.iter().zip(CODEOWNERS_CANDIDATES) {
        let Some(blob) = repo.get(*alias).filter(|blob| blob.get("oid").is_some()) else {
            continue;
        };
        if blob.get("isTruncated").and_then(Value::as_bool) == Some(true) {
            return Err(ProviderError::Decode(format!(
                "{path} is too large to read"
            )));
        }
        codeowners = Some(CodeownersBlob {
            path: path.to_owned(),
            oid: str_at(blob, "/oid"),
            text: str_at(blob, "/text"),
        });
        break;
    }
    let reviews = pull.get("latestOpinionatedReviews").unwrap_or(&Value::Null);
    let records = reviews
        .get("nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter(|node| !node.is_null())
                .map(review)
                .collect()
        })
        .unwrap_or_default();
    let has_next = reviews
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(OwnershipProbe {
        head_sha: str_at(pull, "/headRefOid"),
        base_ref: str_at(pull, "/baseRefName"),
        codeowners,
        reviews: ReviewBundle {
            fetched: reviews.is_object(),
            complete: reviews.is_object() && !has_next,
            reviews: records,
        },
        viewer_is_admin: repo.get("viewerPermission").and_then(Value::as_str) == Some("ADMIN"),
    })
}

fn review(node: &Value) -> ReviewRecord {
    ReviewRecord {
        id: node.get("id").and_then(Value::as_str).map(str::to_owned),
        state: str_at(node, "/state"),
        author_typename: str_at(node, "/author/__typename"),
        author_login: str_at(node, "/author/login"),
        commit: node
            .pointer("/commit/oid")
            .and_then(Value::as_str)
            .map(str::to_owned),
        on_behalf_of: node
            .pointer("/onBehalfOf/nodes")
            .and_then(Value::as_array)
            .map(|teams| {
                teams
                    .iter()
                    .map(|team| OnBehalfOf {
                        org: str_at(team, "/organization/login"),
                        slug: str_at(team, "/slug"),
                    })
                    .filter(|team| !team.org.is_empty() && !team.slug.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

pub(crate) fn inputs(
    client: &GithubClient,
    pr: &PullRequest,
    probe: &OwnershipProbe,
) -> Result<OwnershipInputs, ProviderError> {
    let (files, files_truncated) = files(client, pr)?;
    let (rules_require_code_owner, rules_error) = match rules(client, pr, &probe.base_ref) {
        Ok(required) => (Some(required), None),
        Err(err) if fatal(&err) => return Err(err),
        Err(err) => (None, Some(format!("branch rules unavailable: {err}"))),
    };
    let protection = protection(client, pr, &probe.base_ref, probe.viewer_is_admin)?;
    Ok(OwnershipInputs {
        files,
        files_truncated,
        rules_require_code_owner,
        rules_error,
        protection,
    })
}

/// Rate limits and lost auth stop the whole resolution. Anything else degrades to Unknown.
pub(crate) fn fatal(err: &ProviderError) -> bool {
    matches!(
        err,
        ProviderError::RateLimited { .. } | ProviderError::Unauthorized(_)
    )
}

fn files(client: &GithubClient, pr: &PullRequest) -> Result<(Vec<String>, bool), ProviderError> {
    let mut files = Vec::new();
    let mut cursor = Value::Null;
    let mut changed = 0u64;
    loop {
        let body = client.graphql(
            FILES,
            json!({
                "owner": pr.repo.owner,
                "name": pr.repo.name,
                "number": pr.number,
                "cursor": cursor,
            }),
        )?;
        let pull = data(&body)?
            .pointer("/repository/pullRequest")
            .filter(|pull| !pull.is_null())
            .ok_or_else(|| ProviderError::NotFound("pull request not found".into()))?;
        changed = pull
            .get("changedFiles")
            .and_then(Value::as_u64)
            .unwrap_or(changed);
        if let Some(nodes) = pull.pointer("/files/nodes").and_then(Value::as_array) {
            files.extend(
                nodes
                    .iter()
                    .filter_map(|node| node.get("path").and_then(Value::as_str))
                    .map(str::to_owned),
            );
        }
        let has_next = pull
            .pointer("/files/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !has_next || files.len() >= FILES_CAP {
            break;
        }
        cursor = pull
            .pointer("/files/pageInfo/endCursor")
            .cloned()
            .unwrap_or(Value::Null);
    }
    files.truncate(FILES_CAP);
    let truncated = files.len() >= FILES_CAP || changed > files.len() as u64;
    Ok((files, truncated))
}

fn rules(client: &GithubClient, pr: &PullRequest, base: &str) -> Result<bool, ProviderError> {
    let path = format!(
        "/repos/{}/{}/rules/branches/{}",
        pr.repo.owner,
        pr.repo.name,
        segment(base)
    );
    let mut required = false;
    for page in 1..=10 {
        let body = client.rest_ok(&path, &[("per_page", "100"), ("page", &page.to_string())])?;
        let rules: Vec<Value> = serde_json::from_str(&body)
            .map_err(|err| ProviderError::Decode(format!("rules json: {err}")))?;
        required |= rules.iter().any(|rule| {
            rule.get("type").and_then(Value::as_str) == Some("pull_request")
                && rule
                    .pointer("/parameters/require_code_owner_review")
                    .and_then(Value::as_bool)
                    == Some(true)
        });
        if rules.len() < 100 {
            break;
        }
    }
    Ok(required)
}

fn protection(
    client: &GithubClient,
    pr: &PullRequest,
    base: &str,
    viewer_is_admin: bool,
) -> Result<ClassicProtection, ProviderError> {
    let path = format!(
        "/repos/{}/{}/branches/{}/protection/required_pull_request_reviews",
        pr.repo.owner,
        pr.repo.name,
        segment(base)
    );
    match client.rest_ok(&path, &[]) {
        Ok(body) => {
            let value: Value = serde_json::from_str(&body)
                .map_err(|err| ProviderError::Decode(format!("protection json: {err}")))?;
            Ok(ClassicProtection::Present {
                require_code_owner_reviews: value
                    .get("require_code_owner_reviews")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        }
        Err(ProviderError::NotFound(_)) if viewer_is_admin => Ok(ClassicProtection::Absent),
        Err(ProviderError::NotFound(_) | ProviderError::Forbidden(_)) => {
            Ok(ClassicProtection::Unknown {
                reason: "branch protection is only visible to repository admins".into(),
            })
        }
        Err(err) if fatal(&err) => Err(err),
        Err(err) => Ok(ClassicProtection::Unknown {
            reason: format!("branch protection unavailable: {err}"),
        }),
    }
}

pub(crate) fn team_members(
    client: &GithubClient,
    org: &str,
    slug: &str,
) -> Result<TeamMembers, ProviderError> {
    let path = format!("/orgs/{org}/teams/{slug}/members");
    let mut logins = Vec::new();
    for page in 1..=MEMBER_PAGES {
        let body = match client.rest_ok(&path, &[("per_page", "100"), ("page", &page.to_string())])
        {
            Ok(body) => body,
            // §6: a team the token cannot read is "attribution unknown", not an error.
            Err(ProviderError::NotFound(_) | ProviderError::Forbidden(_)) => {
                return Ok(TeamMembers {
                    complete: false,
                    logins: Vec::new(),
                });
            }
            Err(err) => return Err(err),
        };
        let members: Vec<Value> = serde_json::from_str(&body)
            .map_err(|err| ProviderError::Decode(format!("team members json: {err}")))?;
        let full = members.len() == 100;
        logins.extend(
            members
                .iter()
                .filter_map(|member| member.get("login").and_then(Value::as_str))
                .map(str::to_owned),
        );
        if !full {
            return Ok(TeamMembers {
                complete: true,
                logins,
            });
        }
    }
    Ok(TeamMembers {
        complete: false,
        logins,
    })
}

/// Percent-encodes a branch name for one URL path segment (`release/1.0` → `release%2F1.0`).
pub(crate) fn segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn str_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_are_one_path_segment() {
        assert_eq!(segment("main"), "main");
        assert_eq!(segment("release/1.0"), "release%2F1.0");
    }

    #[test]
    fn probe_takes_the_first_codeowners_hit_and_reads_reviews() {
        let body = json!({"data": {"repository": {
            "viewerPermission": "WRITE",
            "githubDir": null,
            "root": {"oid": "abc", "text": "* @acme/core\n", "isTruncated": false},
            "docs": {"oid": "def", "text": "* @acme/docs\n", "isTruncated": false},
            "pullRequest": {
                "headRefOid": "h1",
                "baseRefName": "main",
                "latestOpinionatedReviews": {
                    "pageInfo": {"hasNextPage": false},
                    "nodes": [{
                        "id": "R1",
                        "state": "APPROVED",
                        "commit": {"oid": "h1"},
                        "author": {"__typename": "User", "login": "bob"},
                        "onBehalfOf": {"nodes": [{"slug": "core", "organization": {"login": "acme"}}]}
                    }]
                }
            }
        }}});
        let probe = parse_probe(&body).unwrap();
        let blob = probe.codeowners.unwrap();
        assert_eq!(
            (blob.path.as_str(), blob.oid.as_str()),
            ("CODEOWNERS", "abc")
        );
        assert!(!probe.viewer_is_admin);
        assert!(probe.reviews.fetched && probe.reviews.complete);
        assert_eq!(probe.reviews.reviews[0].on_behalf_of[0].slug, "core");
    }

    #[test]
    fn a_truncated_codeowners_blob_is_an_error() {
        let body = json!({"data": {"repository": {
            "viewerPermission": "ADMIN",
            "githubDir": {"oid": "abc", "text": null, "isTruncated": true},
            "pullRequest": {"headRefOid": "h1", "baseRefName": "main", "latestOpinionatedReviews": null}
        }}});
        assert!(matches!(parse_probe(&body), Err(ProviderError::Decode(_))));
    }
}
