use pr_nexus_core::error::{CoreError, ProviderError};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AppError {
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sso_url: Option<String>,
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for AppError {}

impl AppError {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.to_owned(),
            message: message.into(),
            retry_at: None,
            sso_url: None,
        }
    }
}

impl From<CoreError> for AppError {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::Provider(err) => err.into(),
            CoreError::Store(message) => Self::new("store", message),
            CoreError::Validation(message) => Self::new("validation", message),
            CoreError::Gate(message) => Self::new("gate", message),
        }
    }
}

impl From<ProviderError> for AppError {
    fn from(err: ProviderError) -> Self {
        match err {
            ProviderError::RateLimited {
                reset_at,
                secondary,
            } => Self {
                kind: "rate_limited".into(),
                message: if secondary {
                    "GitHub secondary rate limit".into()
                } else {
                    "GitHub rate limit".into()
                },
                retry_at: Some(reset_at.to_rfc3339()),
                sso_url: None,
            },
            ProviderError::Unauthorized(message) => Self::new("unauthorized", message),
            ProviderError::Forbidden(message) => {
                let sso_url = message.strip_prefix("sso: ").map(str::to_owned);
                Self {
                    kind: "forbidden".into(),
                    message: if sso_url.is_some() {
                        "GitHub SSO authorization is required".into()
                    } else {
                        message
                    },
                    retry_at: None,
                    sso_url,
                }
            }
            ProviderError::NotFound(message) => Self::new("not_found", message),
            ProviderError::Network(message) => Self::new("network", message),
            ProviderError::Api { code, message } => Self::new("api", format!("{code}: {message}")),
            ProviderError::Decode(message) => Self::new("decode", message),
            ProviderError::Unsupported(message) => Self::new("unsupported", message),
        }
    }
}
