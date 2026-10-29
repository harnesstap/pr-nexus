//! Sign-in to one Slack workspace through rmcp's OAuth client (§7.11).
//!
//! 1. Validate the redirect URI and bind its port on `127.0.0.1`.
//! 2. Discover the authorization server from the MCP URL (RFC 9728/8414).
//! 3. Build the authorize URL with the pre-registered client ID, PKCE S256,
//!    `state`, and the user scopes. The client secret is added only when set.
//! 4. Open the system browser, wait for the redirect, exchange the code.
//! 5. Identify team and user, then save the grant in the keychain under
//!    `slack:<team>:<user>`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use oauth2::TokenResponse;
use pr_nexus_core::error::CoreError;
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession, CredentialStore,
    OAuthTokenResponse, StoredCredentials,
};
use serde::Deserialize;
use serde_json::Value;

use super::http::SlackOAuthHttpClient;
use super::loopback::{self, CancelHandle, Loopback};
use super::{SlackAuthError, install_tls_provider, token_key};
use crate::vault::Vault;

#[derive(Debug, Clone)]
pub struct ConnectConfig {
    pub mcp_url: String,
    pub web_api_base: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub timeout: Duration,
}

/// A workspace that just signed in. The grant is already in the keychain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connected {
    pub team_id: String,
    pub team_name: String,
    /// `acme` for `acme.slack.com`. Falls back to the team ID when unknown.
    pub domain: String,
    pub user_id: String,
    pub user_name: String,
    pub keychain_key: String,
}

/// Run the whole sign-in. `open_browser` gets the authorize URL.
pub async fn connect(
    config: &ConnectConfig,
    vault: Arc<Vault>,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
    cancel: &CancelHandle,
) -> Result<Connected, SlackAuthError> {
    install_tls_provider();
    let client_id = config.client_id.trim();
    if client_id.is_empty() {
        return Err(SlackAuthError::MissingClientId);
    }
    let target = loopback::parse_redirect(&config.redirect_uri)?;
    let listener = Loopback::bind(target.clone()).await?;

    let mut manager = new_manager(&config.mcp_url).await?;
    let pending = PendingStore::default();
    manager.set_credential_store(pending.clone());

    let mut request = AuthorizationRequest::new(target.uri.clone())
        .with_preregistered_client(client_id)
        .with_scopes(config.scopes.iter().cloned());
    if let Some(secret) = config.client_secret.as_deref().filter(|s| !s.is_empty()) {
        request = request.with_client_secret(secret);
    }
    let session = AuthorizationSession::new(manager, request)
        .await
        .map_err(|(_, err)| SlackAuthError::Discovery(err.to_string()))?;
    let auth_url = session.get_authorization_url().to_owned();
    let state = url::Url::parse(&auth_url)
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "state")
                .map(|(_, value)| value.into_owned())
        })
        .ok_or_else(|| SlackAuthError::Discovery("authorize URL has no state".into()))?;

    open_browser(&auth_url).map_err(SlackAuthError::Browser)?;
    let callback = listener.wait(&state, config.timeout, cancel).await?;
    let token = session
        .handle_callback_with_issuer(&callback.code, &callback.state, callback.issuer.as_deref())
        .await
        .map_err(exchange_error)?;
    let credentials = pending
        .take()
        .ok_or_else(|| SlackAuthError::Exchange("no token was stored".into()))?;

    let identity = identify(&config.web_api_base, &token).await?;
    let key = token_key(&identity.team_id, &identity.user_id);
    let store = KeychainCredentialStore::new(vault, key.clone());
    store
        .save(credentials)
        .await
        .map_err(|err| SlackAuthError::Store(err.to_string()))?;
    Ok(Connected {
        keychain_key: key,
        ..identity
    })
}

/// Manager with discovery done, using Slack's OAuth error adapter.
pub(crate) async fn new_manager(mcp_url: &str) -> Result<AuthorizationManager, SlackAuthError> {
    let http =
        SlackOAuthHttpClient::new().map_err(|err| SlackAuthError::Discovery(err.to_string()))?;
    let mut manager = AuthorizationManager::new_with_oauth_http_client(mcp_url, Arc::new(http))
        .await
        .map_err(|err| SlackAuthError::Discovery(err.to_string()))?;
    let resolution = manager
        .resolve_metadata()
        .await
        .map_err(|err| SlackAuthError::Discovery(err.to_string()))?;
    manager.set_metadata(resolution.metadata);
    Ok(manager)
}

fn exchange_error(err: AuthError) -> SlackAuthError {
    let text = err.to_string();
    if text.contains("invalid_client") {
        return SlackAuthError::Exchange(format!(
            "{text}. Check the client ID, and add the client secret unless the app has PKCE enabled."
        ));
    }
    SlackAuthError::Exchange(text)
}

#[derive(Deserialize)]
struct AuthTest {
    ok: bool,
    error: Option<String>,
    url: Option<String>,
    team: Option<String>,
    user: Option<String>,
    team_id: Option<String>,
    user_id: Option<String>,
    /// Set on Enterprise Grid tokens (`E…`).
    #[serde(default)]
    enterprise_id: Option<String>,
}

/// Team and user from the token response, completed by `auth.test`.
///
/// `auth.test` is the documented exception to "MCP only": the token response
/// has IDs but no names, and no MCP tool returns the workspace domain.
async fn identify(
    web_api_base: &str,
    token: &OAuthTokenResponse,
) -> Result<Connected, SlackAuthError> {
    let extra = &token.extra_fields().0;
    let pick = |path: &[&str]| {
        let mut value: Option<&Value> = extra.get(path[0]);
        for key in &path[1..] {
            value = value.and_then(|inner| inner.get(*key));
        }
        value.and_then(Value::as_str).map(str::to_owned)
    };
    let mut team_id = pick(&["team", "id"]);
    let mut team_name = pick(&["team", "name"]);
    let mut user_id = pick(&["authed_user", "id"]);
    let mut domain = None;
    let mut user_name = None;

    match auth_test(web_api_base, token.access_token().secret()).await {
        Ok(test) => {
            team_id = team_id.or(test.team_id);
            user_id = user_id.or(test.user_id);
            team_name = test.team.or(team_name);
            user_name = test.user;
            domain = test.url.as_deref().and_then(domain_from_url);
        }
        Err(err) if team_id.is_none() || user_id.is_none() => {
            return Err(SlackAuthError::Identity(err));
        }
        Err(_) => {}
    }
    let team_id = team_id.ok_or_else(|| SlackAuthError::Identity("no team ID".into()))?;
    let user_id = user_id.ok_or_else(|| SlackAuthError::Identity("no user ID".into()))?;
    Ok(Connected {
        team_name: team_name.unwrap_or_else(|| team_id.clone()),
        domain: domain.unwrap_or_else(|| team_id.to_ascii_lowercase()),
        user_name: user_name.unwrap_or_else(|| user_id.clone()),
        keychain_key: String::new(),
        team_id,
        user_id,
    })
}

async fn auth_test(base: &str, token: &str) -> Result<AuthTest, String> {
    let client = reqwest013::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|err| err.to_string())?;
    let response = client
        .post(format!("{}/auth.test", base.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    let body: AuthTest = response.json().await.map_err(|err| err.to_string())?;
    if !body.ok {
        return Err(format!("auth.test: {}", body.error.unwrap_or_default()));
    }
    Ok(body)
}

/// Whether a saved workspace grant still works (§7.11, checked at startup).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    /// `auth.test` accepted the token, or it can be refreshed.
    Valid,
    /// Nothing saved under the key, e.g. an in-memory vault after a restart.
    Missing,
    /// Slack rejected the token (`invalid_auth`, `token_revoked`, …).
    Rejected,
    /// Could not tell (offline). Treated as signed in.
    Unknown,
}

/// Check the grant under `key` with one `auth.test`.
pub async fn token_state(web_api_base: &str, vault: Arc<Vault>, key: &str) -> TokenState {
    install_tls_provider();
    let store = KeychainCredentialStore::new(vault, key.to_owned());
    let token = match store.load().await {
        Ok(Some(credentials)) => credentials.token_response,
        Ok(None) => return TokenState::Missing,
        Err(_) => return TokenState::Unknown,
    };
    let Some(token) = token else {
        return TokenState::Missing;
    };
    match auth_test(web_api_base, token.access_token().secret()).await {
        Ok(_) => TokenState::Valid,
        Err(err) => classify_auth_test_error(&err, token.refresh_token().is_some()),
    }
}

/// The Enterprise Grid org (`E…`) of the grant under `key`, from one
/// `auth.test`; `None` outside Grid or when it cannot tell.
pub async fn enterprise_id(web_api_base: &str, vault: Arc<Vault>, key: &str) -> Option<String> {
    install_tls_provider();
    let store = KeychainCredentialStore::new(vault, key.to_owned());
    let token = store.load().await.ok()??.token_response?;
    auth_test(web_api_base, token.access_token().secret())
        .await
        .ok()?
        .enterprise_id
        .filter(|id| !id.is_empty())
}

fn classify_auth_test_error(err: &str, refreshable: bool) -> TokenState {
    let rejected = [
        "invalid_auth",
        "not_authed",
        "token_revoked",
        "account_inactive",
    ];
    if rejected.iter().any(|code| err.contains(code)) {
        TokenState::Rejected
    } else if err.contains("token_expired") {
        // The MCP session refreshes it on the next call.
        if refreshable {
            TokenState::Valid
        } else {
            TokenState::Rejected
        }
    } else {
        TokenState::Unknown
    }
}

/// `https://acme.slack.com/` → `acme`.
pub fn domain_from_url(url: &str) -> Option<String> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
    let domain = host.strip_suffix(".slack.com")?;
    (!domain.is_empty() && !domain.contains('.')).then(|| domain.to_owned())
}

/// Holds the grant between the exchange and the identity lookup.
#[derive(Clone, Default)]
struct PendingStore(Arc<Mutex<Option<StoredCredentials>>>);

impl PendingStore {
    fn take(&self) -> Option<StoredCredentials> {
        self.0.lock().expect("pending credentials").take()
    }
}

#[async_trait]
impl CredentialStore for PendingStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        Ok(self.0.lock().expect("pending credentials").clone())
    }
    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        *self.0.lock().expect("pending credentials") = Some(credentials);
        Ok(())
    }
    async fn clear(&self) -> Result<(), AuthError> {
        *self.0.lock().expect("pending credentials") = None;
        Ok(())
    }
}

/// rmcp credential store over the keychain vault. Refreshed tokens are saved
/// back here, so the next app start reuses them.
#[derive(Clone)]
pub struct KeychainCredentialStore {
    vault: Arc<Vault>,
    key: String,
}

impl KeychainCredentialStore {
    pub fn new(vault: Arc<Vault>, key: String) -> Self {
        Self { vault, key }
    }
}

/// Runs a vault call on tokio's blocking pool: the first keychain read can
/// wait on a system prompt, which must not stall an async worker.
async fn off_runtime<T, F>(work: F) -> Result<T, AuthError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| AuthError::CredentialStoreError(err.to_string()))?
        .map_err(|err| AuthError::CredentialStoreError(err.to_string()))
}

#[async_trait]
impl CredentialStore for KeychainCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let (vault, key) = (self.vault.clone(), self.key.clone());
        let Some(raw) = off_runtime(move || vault.get(&key)).await? else {
            return Ok(None);
        };
        serde_json::from_str(&raw)
            .map(Some)
            .map_err(|err| AuthError::CredentialStoreError(err.to_string()))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let raw = serde_json::to_string(&credentials)
            .map_err(|err| AuthError::CredentialStoreError(err.to_string()))?;
        let (vault, key) = (self.vault.clone(), self.key.clone());
        off_runtime(move || vault.set(&key, &raw)).await
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let (vault, key) = (self.vault.clone(), self.key.clone());
        off_runtime(move || vault.remove(&key)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_test_errors_classify() {
        assert_eq!(
            classify_auth_test_error("auth.test: invalid_auth", false),
            TokenState::Rejected
        );
        assert_eq!(
            classify_auth_test_error("auth.test: token_revoked", true),
            TokenState::Rejected
        );
        assert_eq!(
            classify_auth_test_error("auth.test: token_expired", true),
            TokenState::Valid
        );
        assert_eq!(
            classify_auth_test_error("auth.test: token_expired", false),
            TokenState::Rejected
        );
        assert_eq!(
            classify_auth_test_error("connection refused", false),
            TokenState::Unknown
        );
    }

    #[test]
    fn missing_grant_is_missing() {
        let vault = Arc::new(Vault::memory("test"));
        let state = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(token_state("http://127.0.0.1:9", vault, "slack:T0:U0"));
        assert_eq!(state, TokenState::Missing);
    }

    #[test]
    fn domain_comes_from_the_workspace_url() {
        assert_eq!(
            domain_from_url("https://acme.slack.com/").as_deref(),
            Some("acme")
        );
        assert_eq!(
            domain_from_url("https://Acme-Dev.slack.com").as_deref(),
            Some("acme-dev")
        );
        assert_eq!(domain_from_url("https://example.com/"), None);
        assert_eq!(domain_from_url("https://a.b.slack.com/"), None);
    }
}
