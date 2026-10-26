//! Shared error types (`docs/SPEC.md` §4.3).

use chrono::{DateTime, Utc};

/// Failure from a VCS or chat provider. No `String` errors leak past this type.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The provider asked us to wait. `secondary` marks a secondary rate limit.
    #[error("rate limited until {reset_at}")]
    RateLimited {
        reset_at: DateTime<Utc>,
        secondary: bool,
    },
    /// HTTP 401, or Slack `invalid_auth` / `token_revoked`.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// HTTP 403 that is not a rate limit, including SSO and `missing_scope`.
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// The remote object does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// Timeouts, DNS, and TLS failures.
    #[error("network: {0}")]
    Network(String),
    /// A provider-specific API error with a code and message.
    #[error("api error {code}: {message}")]
    Api { code: String, message: String },
    /// The response body did not match the expected shape.
    #[error("decode: {0}")]
    Decode(String),
    /// The provider kind is registered as coming soon and has no factory.
    #[error("provider not supported: {0}")]
    Unsupported(String),
}

/// Errors raised by local logic: the store, validation, and the review gate.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// A provider call failed.
    #[error(transparent)]
    Provider(#[from] ProviderError),
    /// SQLite or the store actor failed.
    #[error("store: {0}")]
    Store(String),
    /// The caller passed something the domain rejects.
    #[error("validation: {0}")]
    Validation(String),
    /// A review-request gate refused the action.
    #[error("gate: {0}")]
    Gate(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_time() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp")
    }

    #[test]
    fn provider_error_display_matches_the_spec() {
        let reset_at = sample_time();
        let cases = [
            (
                ProviderError::RateLimited {
                    reset_at,
                    secondary: true,
                },
                format!("rate limited until {reset_at}"),
            ),
            (
                ProviderError::Unauthorized("token".into()),
                "unauthorized: token".into(),
            ),
            (
                ProviderError::Forbidden("sso".into()),
                "forbidden: sso".into(),
            ),
            (ProviderError::NotFound("pr".into()), "not found: pr".into()),
            (
                ProviderError::Network("timeout".into()),
                "network: timeout".into(),
            ),
            (
                ProviderError::Api {
                    code: "404".into(),
                    message: "missing".into(),
                },
                "api error 404: missing".into(),
            ),
            (ProviderError::Decode("json".into()), "decode: json".into()),
            (
                ProviderError::Unsupported("gitlab".into()),
                "provider not supported: gitlab".into(),
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn core_error_wraps_provider_errors() {
        let error: CoreError = ProviderError::NotFound("pr".into()).into();
        assert_eq!(error.to_string(), "not found: pr");
        assert!(matches!(
            error,
            CoreError::Provider(ProviderError::NotFound(_))
        ));
    }

    #[test]
    fn core_error_display_matches_the_spec() {
        assert_eq!(
            CoreError::Store("locked".into()).to_string(),
            "store: locked"
        );
        assert_eq!(
            CoreError::Validation("empty".into()).to_string(),
            "validation: empty"
        );
        assert_eq!(
            CoreError::Gate("blocked".into()).to_string(),
            "gate: blocked"
        );
    }
}
