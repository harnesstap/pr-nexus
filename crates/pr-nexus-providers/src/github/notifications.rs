//! Read-only notifications inbox (§6 live updates) and the targeted pull
//! request fetch it triggers.
//!
//! Only `GET /notifications` is ever sent: threads are never marked as read.
//! `If-Modified-Since` makes an unchanged inbox a 304, which GitHub does not
//! count against the rate limit.

use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{
    Notification, NotificationsPage, NotificationsRequest, SyncedPullRequest,
};
use serde_json::{Value, json};

use super::{GithubClient, http, parse};

const PULL_REQUEST: &str = include_str!("graphql/pull_request.graphql");

/// Notifications per page. New and updated threads come first.
pub(crate) const PER_PAGE: u32 = 50;

/// The note shown when the token cannot read notifications at all.
pub const TOKEN_TYPE_UNAVAILABLE: &str =
    "Live updates unavailable with this token type: falls back to periodic sync";
/// The note shown when a classic token lacks the scope.
pub const SCOPE_UNAVAILABLE: &str =
    "Live updates need the notifications or repo scope: falls back to periodic sync";

/// Fine-grained PATs and GitHub App tokens cannot use `/notifications`;
/// they are known by prefix, so no request is spent finding out.
pub(crate) fn token_cannot_read_notifications(token: &str) -> bool {
    ["github_pat_", "ghs_", "ghu_", "v1."]
        .iter()
        .any(|prefix| token.starts_with(prefix))
}

/// `X-OAuth-Scopes` on a classic token: does it cover notifications?
pub(crate) fn scopes_cover_notifications(header: &str) -> bool {
    header
        .split(',')
        .map(str::trim)
        .any(|scope| scope == "notifications" || scope == "repo")
}

pub(crate) fn fetch(
    client: &GithubClient,
    request: &NotificationsRequest,
) -> Result<NotificationsPage, ProviderError> {
    if token_cannot_read_notifications(&client.token) {
        return Err(ProviderError::Unsupported(TOKEN_TYPE_UNAVAILABLE.into()));
    }
    let page = request.page.max(1).to_string();
    let per_page = PER_PAGE.to_string();
    let _permit = http::Permit::acquire(&client.gate);
    let mut builder = client
        .http
        .get(format!("{}/notifications", client.api))
        .query(&[
            // Read threads too: activity the user already read on the web
            // still changed the pull request. Filtering is by watermark.
            ("all", "true"),
            // Review and team requests are not "participating".
            ("participating", "false"),
            ("per_page", per_page.as_str()),
            ("page", page.as_str()),
        ])
        .bearer_auth(&client.token)
        .header("Accept", "application/vnd.github+json");
    if request.page <= 1
        && let Some(since) = &request.if_modified_since
    {
        builder = builder.header("If-Modified-Since", since);
    }
    let response = builder
        .send()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    let status = response.status().as_u16();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let poll_interval = header("x-poll-interval").and_then(|value| value.trim().parse().ok());
    let last_modified = header("last-modified");
    let scopes = header("x-oauth-scopes");
    let retry = header("retry-after");
    let sso = header("x-github-sso");
    let has_next = header("link").is_some_and(|link| link.contains("rel=\"next\""));
    if status == 304 {
        return Ok(NotificationsPage::NotModified { poll_interval });
    }
    let body = response
        .text()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    if !(200..300).contains(&status) {
        let err = http::classify(
            status,
            retry.as_deref(),
            sso.as_deref(),
            &body,
            chrono::Utc::now(),
        );
        return Err(match err {
            // 403 or 404 that is not a rate limit: this token cannot read the
            // inbox. A classic token says which scopes it has.
            ProviderError::Forbidden(_) | ProviderError::NotFound(_) => {
                ProviderError::Unsupported(match scopes {
                    Some(scopes) if !scopes_cover_notifications(&scopes) => {
                        SCOPE_UNAVAILABLE.into()
                    }
                    _ => TOKEN_TYPE_UNAVAILABLE.into(),
                })
            }
            other => other,
        });
    }
    let items = parse_notifications(&body)?;
    Ok(NotificationsPage::Changed {
        last_modified,
        poll_interval,
        items,
        has_next,
    })
}

fn parse_notifications(body: &str) -> Result<Vec<Notification>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|err| ProviderError::Decode(format!("notifications json: {err}")))?;
    let threads = value
        .as_array()
        .ok_or_else(|| ProviderError::Decode("notifications: not a list".into()))?;
    let mut items = Vec::new();
    for thread in threads {
        let text = |key: &str| thread.get(key).and_then(Value::as_str);
        let subject = thread.get("subject").unwrap_or(&Value::Null);
        let (Some(id), Some(updated)) = (text("id"), text("updated_at")) else {
            continue;
        };
        let Ok(updated_at) = chrono::DateTime::parse_from_rfc3339(updated) else {
            continue;
        };
        items.push(Notification {
            id: id.to_owned(),
            reason: text("reason").unwrap_or_default().to_owned(),
            updated_at: updated_at.with_timezone(&chrono::Utc),
            subject_type: subject
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            subject_url: subject
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
    Ok(items)
}

/// The `PrFields` fragment, shared with the refresh query.
fn pull_request_document() -> String {
    let fragment = super::REFRESH
        .find("fragment PrFields")
        .map_or("", |start| &super::REFRESH[start..]);
    format!("{PULL_REQUEST}\n{fragment}")
}

pub(crate) fn by_number(
    client: &GithubClient,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<Option<SyncedPullRequest>, ProviderError> {
    let body = match client.graphql(
        &pull_request_document(),
        json!({ "owner": owner, "name": name, "number": number }),
    ) {
        Ok(body) => body,
        // A repository or pull request the token cannot see.
        Err(ProviderError::Api { code, message })
            if code == "graphql" && message.to_ascii_lowercase().contains("could not resolve") =>
        {
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    let node = parse::data(&body)?
        .get("repository")
        .and_then(|repo| repo.get("pullRequest"))
        .cloned()
        .unwrap_or(Value::Null);
    if node.is_null() {
        return Ok(None);
    }
    let wrapped = json!({ "data": { "nodes": [node] } });
    Ok(parse::parse_nodes(&wrapped, client.account)?
        .into_iter()
        .next())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_prefixes_and_scopes() {
        assert!(token_cannot_read_notifications("github_pat_abc"));
        assert!(token_cannot_read_notifications("ghs_abc"));
        assert!(!token_cannot_read_notifications("gho_abc"));
        assert!(!token_cannot_read_notifications("ghp_abc"));
        assert!(scopes_cover_notifications("read:org, repo"));
        assert!(scopes_cover_notifications("notifications"));
        assert!(!scopes_cover_notifications("read:org, public_repo"));
    }

    #[test]
    fn the_document_carries_the_shared_fragment() {
        let document = pull_request_document();
        assert!(document.contains("...PrFields"));
        assert!(document.contains("fragment PrFields on PullRequest"));
    }
}
