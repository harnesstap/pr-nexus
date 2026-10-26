//! HTTP status and header mapping for the GitHub API.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

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
    if !(200..300).contains(&status) {
        return ProviderError::Api {
            code: status.to_string(),
            message: message_or(body, "github api error"),
        };
    }
    ProviderError::Api {
        code: status.to_string(),
        message: message_or(body, "github api error"),
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

pub(crate) fn timeout() -> Duration {
    Duration::from_secs(30)
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
}
