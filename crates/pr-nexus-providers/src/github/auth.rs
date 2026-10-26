//! `gh` CLI token and the GitHub device flow.
//!
//! The device-flow client id is never committed. Callers pass one from a
//! setting or `PR_NEXUS_GITHUB_OAUTH_CLIENT_ID`.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use pr_nexus_core::error::ProviderError;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug)]
pub enum GhProbe {
    /// `gh` is missing, or the user is not logged in.
    Unavailable {
        reason: String,
    },
    Ready {
        token: String,
    },
}

impl GhProbe {
    pub fn token(&self) -> Option<&str> {
        match self {
            Self::Ready { token } => Some(token.as_str()),
            Self::Unavailable { .. } => None,
        }
    }
}

pub fn read_gh_token() -> GhProbe {
    let mut child = match Command::new("gh")
        .args(["auth", "token", "--hostname", "github.com"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return GhProbe::Unavailable {
                reason: format!("gh is not available ({err})"),
            };
        }
    };
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let status = child.wait();
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            let _ = pipe.read_to_string(&mut stdout);
        }
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let _ = tx.send((status, stdout, stderr));
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok((Ok(status), stdout, _stderr)) if status.success() => {
            let token = stdout.trim().to_owned();
            if token.is_empty() {
                GhProbe::Unavailable {
                    reason: "gh returned an empty token".into(),
                }
            } else {
                GhProbe::Ready { token }
            }
        }
        Ok((Ok(status), _stdout, stderr)) => GhProbe::Unavailable {
            reason: format!("gh auth token exited {status}: {}", stderr.trim()),
        },
        Ok((Err(err), _, _)) => GhProbe::Unavailable {
            reason: format!("gh auth token failed: {err}"),
        },
        Err(_) => GhProbe::Unavailable {
            reason: "gh auth token timed out after 5s".into(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    Pending { slow_down: bool },
    Denied,
    Expired,
    Token(String),
}

pub fn start_device_flow(
    http: &reqwest::blocking::Client,
    login_base: &str,
    client_id: &str,
) -> Result<DeviceStart, ProviderError> {
    if client_id.trim().is_empty() {
        return Err(ProviderError::Unsupported(
            "GitHub device flow needs github.oauth_client_id or PR_NEXUS_GITHUB_OAUTH_CLIENT_ID"
                .into(),
        ));
    }
    let response = http
        .post(format!("{login_base}/login/device/code"))
        .header("Accept", "application/json")
        .json(&json!({
            "client_id": client_id,
            "scope": "repo read:org"
        }))
        .send()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(crate::github::http::classify(
            status,
            None,
            None,
            &body,
            chrono::Utc::now(),
        ));
    }
    let parsed: DeviceCodeBody = serde_json::from_str(&body)
        .map_err(|err| ProviderError::Decode(format!("device code: {err}")))?;
    Ok(DeviceStart {
        device_code: parsed.device_code,
        user_code: parsed.user_code,
        verification_uri: parsed.verification_uri,
        expires_in: parsed.expires_in,
        interval: parsed.interval.max(1),
    })
}

pub fn poll_device_flow(
    http: &reqwest::blocking::Client,
    login_base: &str,
    client_id: &str,
    device_code: &str,
) -> Result<DevicePoll, ProviderError> {
    let response = http
        .post(format!("{login_base}/login/oauth/access_token"))
        .header("Accept", "application/json")
        .json(&json!({
            "client_id": client_id,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
        }))
        .send()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    let body = response
        .text()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    let parsed: DevicePollBody = serde_json::from_str(&body)
        .map_err(|err| ProviderError::Decode(format!("device poll: {err}")))?;
    if let Some(token) = parsed.access_token.filter(|token| !token.is_empty()) {
        return Ok(DevicePoll::Token(token));
    }
    match parsed.error.as_deref() {
        Some("authorization_pending") => Ok(DevicePoll::Pending { slow_down: false }),
        Some("slow_down") => Ok(DevicePoll::Pending { slow_down: true }),
        Some("access_denied") => Ok(DevicePoll::Denied),
        Some("expired_token") => Ok(DevicePoll::Expired),
        Some(other) => Err(ProviderError::Api {
            code: other.to_owned(),
            message: parsed
                .error_description
                .unwrap_or_else(|| "device flow failed".into()),
        }),
        None => Err(ProviderError::Decode(
            "device flow response had neither a token nor an error".into(),
        )),
    }
}

#[derive(Deserialize)]
struct DeviceCodeBody {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct DevicePollBody {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

pub fn oauth_client_id_from_env() -> Option<String> {
    std::env::var("PR_NEXUS_GITHUB_OAUTH_CLIENT_ID")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn github_token_from_env() -> Option<String> {
    std::env::var("PR_NEXUS_GITHUB_TOKEN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}
