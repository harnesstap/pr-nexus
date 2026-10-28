//! The detail panel's overview query and the auto-merge mutations (§7.2).

use pr_nexus_core::domain::PullRequest;
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::overview::{
    AutoMergeState, CheckItem, CheckState, MergeMethod, MergeSettings, Person, PersonKind,
    PrOverview, ReviewState, Reviewer, ViewerAccess,
};
use serde_json::{Value, json};

use super::GithubClient;
use super::ownership::{fatal, segment};
use super::parse::data;

const OVERVIEW: &str = include_str!("graphql/pr_overview.graphql");
const ENABLE: &str = include_str!("graphql/enable_auto_merge.graphql");
const DISABLE: &str = include_str!("graphql/disable_auto_merge.graphql");
const TO_DRAFT: &str = include_str!("graphql/convert_to_draft.graphql");
const MARK_READY: &str = include_str!("graphql/mark_ready.graphql");
const MERGE: &str = include_str!("graphql/merge_pull_request.graphql");

pub(crate) fn fetch(client: &GithubClient, pr: &PullRequest) -> Result<PrOverview, ProviderError> {
    let body = client.graphql(
        OVERVIEW,
        json!({ "owner": pr.repo.owner, "name": pr.repo.name, "number": pr.number }),
    )?;
    let mut overview = parse(&body, chrono::Utc::now().to_rfc3339())?;
    let from_rules = rules_approvals(client, pr, &overview.base_ref)?;
    let from_protection = if overview.viewer.permission.as_deref() == Some("admin") {
        protection_approvals(client, pr, &overview.base_ref)?
    } else {
        None
    };
    overview.required_approvals = required_approvals(from_rules, from_protection);
    Ok(overview)
}

pub(crate) fn set_auto_merge(
    client: &GithubClient,
    pr: &PullRequest,
    method: Option<MergeMethod>,
) -> Result<(), ProviderError> {
    let body = match method {
        Some(method) => client.graphql(
            ENABLE,
            json!({ "id": pr.remote.id, "method": method.graphql() }),
        )?,
        None => client.graphql(DISABLE, json!({ "id": pr.remote.id }))?,
    };
    data(&body)?;
    Ok(())
}

/// Merges the pull request now with `method` (GraphQL `mergePullRequest`).
pub(crate) fn merge(
    client: &GithubClient,
    pr: &PullRequest,
    method: MergeMethod,
) -> Result<(), ProviderError> {
    let body = client.graphql(
        MERGE,
        json!({ "id": pr.remote.id, "method": method.graphql() }),
    )?;
    data(&body)?;
    Ok(())
}

/// Converts to a draft or marks ready for review. Free private repositories
/// cannot hold drafts; GitHub's error for that becomes a plain `Unsupported`.
pub(crate) fn set_draft(
    client: &GithubClient,
    pr: &PullRequest,
    draft: bool,
) -> Result<(), ProviderError> {
    let query = if draft { TO_DRAFT } else { MARK_READY };
    match client.graphql(query, json!({ "id": pr.remote.id })) {
        Ok(body) => data(&body).map(|_| ()),
        Err(err) if drafts_unsupported(&err) => Err(ProviderError::Unsupported(
            "draft pull requests aren't available on this repository's plan".into(),
        )),
        Err(err) => Err(err),
    }
}

fn drafts_unsupported(err: &ProviderError) -> bool {
    let text = err.to_string().to_ascii_lowercase();
    text.contains("draft") && (text.contains("not supported") || text.contains("not available"))
}

/// Rulesets and classic protection can both ask for approvals; the stricter
/// wins. Classic protection is hidden from non-admins, so a ruleset count
/// alone is a lower bound and a zero there means "unknown", not "none".
pub(crate) fn required_approvals(rules: Option<u32>, protection: Option<u32>) -> Option<u32> {
    match (rules, protection) {
        (Some(rules), Some(protection)) => Some(rules.max(protection)),
        (Some(rules), None) if rules > 0 => Some(rules),
        (None, Some(protection)) => Some(protection),
        _ => None,
    }
}

pub(crate) fn parse(body: &Value, fetched_at: String) -> Result<PrOverview, ProviderError> {
    let root = data(body)?;
    let repo = root
        .get("repository")
        .filter(|value| !value.is_null())
        .ok_or_else(|| ProviderError::NotFound("repository".into()))?;
    let node = repo
        .get("pullRequest")
        .filter(|value| !value.is_null())
        .ok_or_else(|| ProviderError::NotFound("pull request".into()))?;

    let checks_rollup = node.pointer("/commits/nodes/0/commit/statusCheckRollup/contexts");
    let checks: Vec<CheckItem> = nodes(checks_rollup).iter().filter_map(check).collect();
    let checks_total = checks_rollup
        .and_then(|value| value.get("totalCount"))
        .and_then(Value::as_u64)
        .map_or(checks.len() as u32, |count| count as u32);

    let review_comments = nodes(node.get("reviewThreads"))
        .iter()
        .filter_map(|thread| {
            thread
                .pointer("/comments/totalCount")
                .and_then(Value::as_u64)
        })
        .sum::<u64>() as u32;

    let reviewers = reviewers(node);
    let approvals = reviewers
        .iter()
        .filter(|reviewer| reviewer.state == ReviewState::Approved)
        .count() as u32;

    let author = node.get("author").and_then(actor_person);
    let assignees = dedupe(nodes(node.get("assignees")).iter().filter_map(user_person));
    let commenters = commenters(node);

    let auto_merge = node
        .get("autoMergeRequest")
        .filter(|value| !value.is_null())
        .and_then(|request| {
            Some(AutoMergeState {
                method: MergeMethod::parse(request.get("mergeMethod")?.as_str()?)?,
                enabled_by: opt_str(request.pointer("/enabledBy/login")),
                enabled_at: opt_str(request.get("enabledAt")),
            })
        });

    Ok(PrOverview {
        state: lower(node.get("state")).unwrap_or_else(|| "open".into()),
        is_draft: flag(node.get("isDraft")),
        base_ref: opt_str(node.get("baseRefName")).unwrap_or_default(),
        head_ref: opt_str(node.get("headRefName")).unwrap_or_default(),
        head_sha: opt_str(node.get("headRefOid")).unwrap_or_default(),
        base_sha: opt_str(node.get("baseRefOid")).unwrap_or_default(),
        head_owner: opt_str(node.pointer("/headRepositoryOwner/login")).unwrap_or_default(),
        mergeable: match node.get("mergeable").and_then(Value::as_str) {
            Some("MERGEABLE") => "mergeable".into(),
            Some("CONFLICTING") => "conflicting".into(),
            _ => "unknown".into(),
        },
        merge_state: lower(node.get("mergeStateStatus")).unwrap_or_else(|| "unknown".into()),
        review_decision: lower(node.get("reviewDecision")),
        issue_comments: node
            .pointer("/comments/totalCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        review_comments,
        checks,
        checks_total,
        approvals,
        required_approvals: None,
        reviewers,
        author,
        assignees,
        commenters,
        viewer: ViewerAccess {
            login: opt_str(root.pointer("/viewer/login")).unwrap_or_default(),
            permission: lower(repo.get("viewerPermission")),
            can_merge_as_admin: flag(node.get("viewerCanMergeAsAdmin")),
            can_enable_auto_merge: flag(node.get("viewerCanEnableAutoMerge")),
            can_disable_auto_merge: flag(node.get("viewerCanDisableAutoMerge")),
        },
        merge_settings: MergeSettings {
            auto_merge_allowed: flag(repo.get("autoMergeAllowed")),
            squash: flag(repo.get("squashMergeAllowed")),
            merge: flag(repo.get("mergeCommitAllowed")),
            rebase: flag(repo.get("rebaseMergeAllowed")),
        },
        auto_merge,
        fetched_at,
    })
}

/// Latest review per person, then outstanding requests. A re-requested
/// reviewer shows as pending, like on GitHub.
fn reviewers(node: &Value) -> Vec<Reviewer> {
    let mut out: Vec<Reviewer> = Vec::new();
    for review in nodes(node.get("latestReviews")) {
        let state = match review.get("state").and_then(Value::as_str) {
            Some("APPROVED") => ReviewState::Approved,
            Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
            Some("COMMENTED") => ReviewState::Commented,
            Some("DISMISSED") => ReviewState::Dismissed,
            // A draft review only its author can see.
            _ => continue,
        };
        if let Some(person) = review.get("author").and_then(actor_person) {
            out.push(Reviewer { person, state });
        }
    }
    for request in nodes(node.get("reviewRequests")) {
        let Some(reviewer) = request.get("requestedReviewer") else {
            continue;
        };
        let person = match reviewer.get("__typename").and_then(Value::as_str) {
            Some("Team") => {
                let (Some(org), Some(slug)) = (
                    opt_str(reviewer.pointer("/organization/login")),
                    opt_str(reviewer.get("slug")),
                ) else {
                    continue;
                };
                Person {
                    kind: PersonKind::Team,
                    login: format!("{org}/{slug}"),
                    name: opt_str(reviewer.get("name")),
                    avatar_url: opt_str(reviewer.get("avatarUrl")),
                }
            }
            Some("User") => match user_person(reviewer) {
                Some(person) => person,
                None => continue,
            },
            _ => continue,
        };
        match out
            .iter_mut()
            .find(|known| known.person.kind == person.kind && known.person.login == person.login)
        {
            Some(known) => known.state = ReviewState::Pending,
            None => out.push(Reviewer {
                person,
                state: ReviewState::Pending,
            }),
        }
    }
    out
}

/// People in first-seen order, each login once (case-insensitive).
fn dedupe(people: impl IntoIterator<Item = Person>) -> Vec<Person> {
    let mut out: Vec<Person> = Vec::new();
    for person in people {
        if !out
            .iter()
            .any(|known| known.login.eq_ignore_ascii_case(&person.login))
        {
            out.push(person);
        }
    }
    out
}

/// Authors of PR comments, then review-thread comments, deduped. Bots are
/// kept and marked `Bot`; the panel drops the viewer, author and reviewers.
fn commenters(node: &Value) -> Vec<Person> {
    let threads = nodes(node.get("reviewThreads"))
        .iter()
        .flat_map(|thread| nodes(thread.get("comments")));
    dedupe(
        nodes(node.get("comments"))
            .iter()
            .chain(threads)
            .filter_map(|comment| comment.get("author").and_then(actor_person)),
    )
}

/// A comment or review author: a user, or a bot marked as one. Deleted
/// accounts come back as `null` and are skipped.
fn actor_person(node: &Value) -> Option<Person> {
    let mut person = user_person(node)?;
    if node.get("__typename").and_then(Value::as_str) == Some("Bot") {
        person.kind = PersonKind::Bot;
    }
    Some(person)
}

fn check(node: &Value) -> Option<CheckItem> {
    let required = flag(node.get("isRequired"));
    match node.get("__typename").and_then(Value::as_str)? {
        "CheckRun" => {
            let state = if node.get("status").and_then(Value::as_str) != Some("COMPLETED") {
                CheckState::Pending
            } else {
                match node.get("conclusion").and_then(Value::as_str) {
                    Some("SUCCESS") => CheckState::Passed,
                    Some("NEUTRAL" | "SKIPPED") => CheckState::Skipped,
                    _ => CheckState::Failed,
                }
            };
            Some(CheckItem {
                name: opt_str(node.get("name"))?,
                state,
                url: opt_str(node.get("detailsUrl")),
                required,
            })
        }
        "StatusContext" => Some(CheckItem {
            name: opt_str(node.get("context"))?,
            state: match node.get("state").and_then(Value::as_str) {
                Some("SUCCESS") => CheckState::Passed,
                Some("PENDING" | "EXPECTED") => CheckState::Pending,
                _ => CheckState::Failed,
            },
            url: opt_str(node.get("targetUrl")),
            required,
        }),
        _ => None,
    }
}

fn user_person(node: &Value) -> Option<Person> {
    Some(Person {
        kind: PersonKind::User,
        login: opt_str(node.get("login"))?,
        name: opt_str(node.get("name")).filter(|name| !name.is_empty()),
        avatar_url: opt_str(node.get("avatarUrl")),
    })
}

fn rules_approvals(
    client: &GithubClient,
    pr: &PullRequest,
    base: &str,
) -> Result<Option<u32>, ProviderError> {
    let path = format!(
        "/repos/{}/{}/rules/branches/{}",
        pr.repo.owner,
        pr.repo.name,
        segment(base)
    );
    match client.rest_ok(&path, &[("per_page", "100")]) {
        Ok(body) => {
            let rules: Vec<Value> = serde_json::from_str(&body)
                .map_err(|err| ProviderError::Decode(format!("rules json: {err}")))?;
            Ok(Some(
                rules
                    .iter()
                    .filter(|rule| rule.get("type").and_then(Value::as_str) == Some("pull_request"))
                    .filter_map(|rule| {
                        rule.pointer("/parameters/required_approving_review_count")
                            .and_then(Value::as_u64)
                    })
                    .max()
                    .unwrap_or(0) as u32,
            ))
        }
        Err(err) if fatal(&err) => Err(err),
        Err(_) => Ok(None),
    }
}

fn protection_approvals(
    client: &GithubClient,
    pr: &PullRequest,
    base: &str,
) -> Result<Option<u32>, ProviderError> {
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
            Ok(Some(
                value
                    .get("required_approving_review_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
            ))
        }
        // No classic protection on the branch: nothing required there.
        Err(ProviderError::NotFound(_)) => Ok(Some(0)),
        Err(err) if fatal(&err) => Err(err),
        Err(_) => Ok(None),
    }
}

fn nodes(connection: Option<&Value>) -> &[Value] {
    connection
        .and_then(|value| value.get("nodes"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn opt_str(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

fn lower(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_ascii_lowercase)
}

fn flag(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stricter_approval_count_wins_and_zero_rules_alone_is_unknown() {
        assert_eq!(required_approvals(Some(1), Some(2)), Some(2));
        assert_eq!(required_approvals(Some(2), None), Some(2));
        assert_eq!(required_approvals(Some(0), None), None);
        assert_eq!(required_approvals(None, Some(0)), Some(0));
        assert_eq!(required_approvals(None, None), None);
    }

    #[test]
    fn rerequested_reviewer_is_pending_and_draft_reviews_are_skipped() {
        let node = json!({
            "latestReviews": {"nodes": [
                {"state": "APPROVED", "author": {"login": "kim", "name": "Kim Lee", "avatarUrl": "https://example.test/k"}},
                {"state": "CHANGES_REQUESTED", "author": {"login": "sam", "name": null}},
                {"state": "PENDING", "author": {"login": "me"}}
            ]},
            "reviewRequests": {"nodes": [
                {"requestedReviewer": {"__typename": "User", "login": "sam", "name": "Sam"}},
                {"requestedReviewer": {"__typename": "Team", "slug": "web", "name": "Web", "organization": {"login": "acme"}}},
                {"requestedReviewer": {"__typename": "Mannequin", "login": "ghost"}}
            ]}
        });
        let reviewers = reviewers(&node);
        let summary: Vec<_> = reviewers
            .iter()
            .map(|reviewer| (reviewer.person.login.as_str(), reviewer.state))
            .collect();
        assert_eq!(
            summary,
            [
                ("kim", ReviewState::Approved),
                ("sam", ReviewState::Pending),
                ("acme/web", ReviewState::Pending),
            ]
        );
        assert_eq!(reviewers[2].person.kind, PersonKind::Team);
    }

    #[test]
    fn commenters_and_assignees_are_deduped_and_keep_bots() {
        let node = json!({
            "author": {"__typename": "Bot", "login": "deps-bot", "avatarUrl": "https://example.test/d"},
            "assignees": {"nodes": [{"login": "noor", "name": "Noor"}, {"login": "NOOR"}]},
            "comments": {"totalCount": 2, "nodes": [
                {"author": {"__typename": "Bot", "login": "ci-bot"}},
                {"author": {"__typename": "User", "login": "kim", "name": "Kim"}},
                {"author": null}
            ]},
            "reviewThreads": {"nodes": [
                {"comments": {"totalCount": 1, "nodes": [{"author": {"__typename": "User", "login": "KIM"}}]}}
            ]}
        });
        let people: Vec<_> = commenters(&node)
            .into_iter()
            .map(|person| (person.login, person.kind))
            .collect();
        assert_eq!(
            people,
            [
                ("ci-bot".to_string(), PersonKind::Bot),
                ("kim".to_string(), PersonKind::User),
            ]
        );
        let assignees = dedupe(nodes(node.get("assignees")).iter().filter_map(user_person));
        assert_eq!(assignees.len(), 1);
        assert_eq!(assignees[0].name.as_deref(), Some("Noor"));
        assert_eq!(
            node.get("author").and_then(actor_person).map(|p| p.kind),
            Some(PersonKind::Bot)
        );
    }

    #[test]
    fn draft_plan_error_is_recognised() {
        let plan = ProviderError::Api {
            code: "UNPROCESSABLE".into(),
            message: "Draft pull requests are not supported in this repository.".into(),
        };
        assert!(drafts_unsupported(&plan));
        assert!(!drafts_unsupported(&ProviderError::Forbidden(
            "nope".into()
        )));
    }
}
