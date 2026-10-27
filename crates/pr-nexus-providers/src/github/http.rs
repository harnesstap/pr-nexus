//! HTTP status and header mapping for the GitHub API.

use std::sync::{Condvar, Mutex};

use chrono::{DateTime, Utc};
use pr_nexus_core::error::ProviderError;

pub(crate) struct Gate {
    left: Mutex<usize>,
    cv: Condvar,
}

impl Gate {
    pub(crate) fn new(permits: usize) -> Self {
        Self {
            left: Mutex::new(permits),
            cv: Condvar::new(),
        }
    }

    pub(crate) fn enter(&self) {
        let mut left = self.left.lock().expect("rate gate");
        while *left == 0 {
            left = self.cv.wait(left).expect("rate gate");
        }
        *left -= 1;
    }

    pub(crate) fn exit(&self) {
        let mut left = self.left.lock().expect("rate gate");
        *left += 1;
        self.cv.notify_one();
    }
}

pub(crate) struct Permit<'a> {
    gate: &'a Gate,
}

impl<'a> Permit<'a> {
    pub(crate) fn acquire(gate: &'a Gate) -> Self {
        gate.enter();
        Self { gate }
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.gate.exit();
    }
}

pub(crate) fn classify(
    status: u16,
    retry_after: Option<&str>,
    sso: Option<&str>,
    body: &str,
    now: DateTime<Utc>,
) -> ProviderError {
    if status == 401 {
        return ProviderError::Unauthorized(message_or(body, "unauthorized"));
    }
    if status == 404 {
        return ProviderError::NotFound(message_or(body, "not found"));
    }
    if status == 403 || status == 429 {
        if let Some(url) = sso_url(sso.unwrap_or("")) {
            return ProviderError::Forbidden(format!("sso: {url}"));
        }
        if status == 429 || looks_secondary(body) || retry_after.is_some() {
            let secs = retry_after
                .and_then(|value| value.trim().parse::<i64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(60);
            return ProviderError::RateLimited {
                reset_at: now + chrono::Duration::seconds(secs),
                secondary: true,
            };
        }
        return ProviderError::Forbidden(message_or(body, "forbidden"));
    }
    if is_gateway_timeout(status) {
        // GitHub's proxy answers an HTML page here, so there is no JSON message.
        return ProviderError::Api {
            code: status.to_string(),
            message: message_or(body, "GitHub did not answer in time"),
        };
    }
    ProviderError::Api {
        code: status.to_string(),
        message: message_or(body, "github api error"),
    }
}

fn is_gateway_timeout(status: u16) -> bool {
    matches!(status, 502..=504)
}

/// GitHub gave up on the query (gateway 502/503/504, a GraphQL timeout error,
/// or the client timeout), so the same request with fewer nodes may pass.
pub(crate) fn is_timeout(err: &ProviderError) -> bool {
    match err {
        ProviderError::Api { code, message } => {
            let lower = message.to_ascii_lowercase();
            code.parse::<u16>().is_ok_and(is_gateway_timeout)
                || (code == "graphql"
                    && (lower.contains("timeout")
                        || lower.contains("timed out")
                        || lower.contains("something went wrong while executing your query")))
        }
        ProviderError::Network(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("timed out") || lower.contains("timeout")
        }
        ProviderError::RateLimited { .. }
        | ProviderError::Unauthorized(_)
        | ProviderError::Forbidden(_)
        | ProviderError::NotFound(_)
        | ProviderError::Decode(_)
        | ProviderError::Unsupported(_)
        | ProviderError::SentUnconfirmed(_) => false,
    }
}

/// The error a page reports once every retry timed out: it names the query
/// and says GitHub timed out, keeping the last status as the code.
pub(crate) fn timed_out(
    last: &ProviderError,
    what: &str,
    attempts: u32,
    size: usize,
) -> ProviderError {
    let code = match last {
        ProviderError::Api { code, .. } => code.clone(),
        _ => "timeout".to_owned(),
    };
    ProviderError::Api {
        code,
        message: format!(
            "GitHub timed out on the {what} ({attempts} tries, down to {size} per page). Try again in a minute."
        ),
    }
}

fn message_or(body: &str, fallback: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|message| message.as_str())
                .map(str::to_owned)
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

fn looks_secondary(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("secondary rate limit") || lower.contains("rate limit")
}

pub(crate) fn sso_url(header: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let index = lower.find("url=")?;
    let rest = header[index + "url=".len()..].trim();
    let url = rest.split([';', ',', ' ']).next().unwrap_or("").trim();
    if url.starts_with("https://") {
        Some(url.to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-26T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn maps_auth_sso_secondary_and_not_found() {
        assert!(matches!(
            classify(401, None, None, r#"{"message":"Bad credentials"}"#, now()),
            ProviderError::Unauthorized(_)
        ));
        match classify(
            403,
            None,
            Some("required; url=https://github.com/orgs/acme/sso?authorization_request=1"),
            "{}",
            now(),
        ) {
            ProviderError::Forbidden(message) => {
                assert!(message.contains("https://github.com/orgs/acme/sso"));
            }
            other => panic!("{other:?}"),
        }
        match classify(
            403,
            Some("30"),
            None,
            r#"{"message":"You have exceeded a secondary rate limit."}"#,
            now(),
        ) {
            ProviderError::RateLimited {
                secondary,
                reset_at,
            } => {
                assert!(secondary);
                assert_eq!(
                    reset_at,
                    DateTime::parse_from_rfc3339("2026-10-26T00:00:30Z")
                        .unwrap()
                        .with_timezone(&Utc)
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            classify(404, None, None, r#"{"message":"Not Found"}"#, now()),
            ProviderError::NotFound(_)
        ));
    }

    #[test]
    fn gateway_pages_and_timeouts_count_as_timeouts() {
        let gateway = classify(502, None, None, "<html>502 Bad Gateway</html>", now());
        match &gateway {
            ProviderError::Api { code, message } => {
                assert_eq!(code, "502");
                assert_eq!(message, "GitHub did not answer in time");
            }
            other => panic!("{other:?}"),
        }
        assert!(is_timeout(&gateway));
        assert!(is_timeout(&classify(504, None, None, "", now())));
        assert!(is_timeout(&ProviderError::Network(
            "error sending request: operation timed out".into()
        )));
        assert!(is_timeout(&ProviderError::Api {
            code: "graphql".into(),
            message: "Something went wrong while executing your query. This may be the result of a timeout".into(),
        }));
        assert!(!is_timeout(&classify(500, None, None, "{}", now())));
        assert!(!is_timeout(&ProviderError::Api {
            code: "graphql".into(),
            message: "Field 'nope' doesn't exist".into(),
        }));
        assert!(!is_timeout(&ProviderError::Network(
            "connection refused".into()
        )));
    }
}
