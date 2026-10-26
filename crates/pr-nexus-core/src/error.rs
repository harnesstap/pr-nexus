//! Typed errors returned by providers and by core services.

use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("rate limited until {reset_at}")]
    RateLimited {
        reset_at: DateTime<Utc>,
        secondary: bool,
    },
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("network: {0}")]
    Network(String),
    #[error("api error {code}: {message}")]
    Api { code: String, message: String },
    #[error("decode: {0}")]
    Decode(String),
    #[error("provider not supported: {0}")]
    Unsupported(String),
    /// The write went through but its result could not be read, so the
    /// message's `ts` and link are unknown. Never retry it: that duplicates.
    #[error("sent, link unknown: {0}")]
    SentUnconfirmed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("store: {0}")]
    Store(String),
    #[error("validation: {0}")]
    Validation(String),
    #[error("gate: {0}")]
    Gate(String),
}
