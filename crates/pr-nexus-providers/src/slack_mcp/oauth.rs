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
    let client_id = config.client_id.trim();
    if client_id.is_empty() {
        return Err(SlackAuthError::MissingClientId);
    }
    let target = loopback::parse_redirect(&config.redirect_uri)?;
    let listener = Loopback::bind(target).await?;
    connect_with_listener(config, vault, listener, open_browser, cancel).await
}

/// Sign in on a listener that is already bound to `127.0.0.1`. Its redirect
/// URI must be the configured one, so the browser is never sent elsewhere.
pub async fn connect_with_listener(
    config: &ConnectConfig,
    vault: Arc<Vault>,
    listener: Loopback,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
    cancel: &CancelHandle,
) -> Result<Connected, SlackAuthError> {
    install_tls_provider();
    let client_id = config.client_id.trim();
    if client_id.is_empty() {
        return Err(SlackAuthError::MissingClientId);
    }
    if listener.redirect_uri() != config.redirect_uri.trim() {
        return Err(SlackAuthError::InvalidRedirect(format!(
            "listener is bound to {}, settings say {}",
            listener.redirect_uri(),
            config.redirect_uri.trim()
        )));
    }
    let redirect_uri = listener.redirect_uri().to_owned();

    let mut manager = new_manager(&config.mcp_url).await?;
    let pending = PendingStore::default();
    manager.set_credential_store(pending.clone());

    let mut request = AuthorizationRequest::new(redirect_uri)
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

/// Team and user from the token response, completed by `auth.test` and `team.info`.
///
/// These two calls are the documented exception to "MCP only": the token
/// response has IDs but no domain, and no MCP tool returns the workspace
/// domain. `team.info` supplies the name and domain as one pair for this team.
/// An `auth.test` name, host, or user from a different team is ignored, so
/// one workspace cannot be stored with another's domain.
async fn identify(
    web_api_base: &str,
    token: &OAuthTokenResponse,
) -> Result<Connected, SlackAuthError> {
    let extra = &token.extra_fields().0;
    let token_team_id = extra_str(extra, &["team", "id"]);
    let token_team_name = extra_str(extra, &["team", "name"]);
    let token_user_id = extra_str(extra, &["authed_user", "id"]);
    let found = match read_identity(
        web_api_base,
        token.access_token().secret(),
        token_team_id.as_deref(),
        token_team_name.as_deref(),
        token_user_id.as_deref(),
        None,
    )
    .await
    {
        Ok(found) => found,
        Err(err)
            if filled(token_team_id.as_deref()).is_some()
                && filled(token_user_id.as_deref()).is_some() =>
        {
            let identity = pair_identity(
                token_team_id.as_deref(),
                token_team_name.as_deref(),
                None,
                &AuthFacts::default(),
                None,
            )
            .ok_or(SlackAuthError::Identity(err))?;
            Identified {
                identity,
                user_id: token_user_id.unwrap_or_default(),
            }
        }
        Err(err) => return Err(SlackAuthError::Identity(err)),
    };
    Ok(connected_from(found))
}

fn connected_from(found: Identified) -> Connected {
    let user_name = if found.identity.user_name.is_empty() {
        found.user_id.clone()
    } else {
        found.identity.user_name.clone()
    };
    Connected {
        team_name: found.identity.team_name,
        domain: found.identity.domain,
        user_name,
        keychain_key: String::new(),
        team_id: found.identity.team_id,
        user_id: found.user_id,
    }
}

fn web_client() -> Result<reqwest013::Client, String> {
    reqwest013::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|err| err.to_string())
}

async fn auth_test(base: &str, token: &str) -> Result<AuthTest, String> {
    let response = web_client()?
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AuthFacts {
    team_id: Option<String>,
    team_name: Option<String>,
    domain: Option<String>,
    user_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TeamFacts {
    team_id: String,
    team_name: String,
    domain: String,
}

/// Name, domain, and handle that belong to one team id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceIdentity {
    pub team_id: String,
    pub team_name: String,
    pub domain: String,
    /// Slack handle from `auth.test`. Empty when that answer was for another team.
    pub user_name: String,
}

/// One recheck of a saved grant. `error` is set only when Slack could not be reached,
/// and it never contains a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantProbe {
    pub state: TokenState,
    pub error: String,
    pub identity: Option<WorkspaceIdentity>,
}

struct Identified {
    identity: WorkspaceIdentity,
    user_id: String,
}

#[derive(Deserialize)]
struct TeamInfoBody {
    ok: bool,
    error: Option<String>,
    team: Option<TeamInfoTeam>,
}

#[derive(Deserialize)]
struct TeamInfoTeam {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    domain: String,
}

fn filled(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

fn extra_str(extra: &std::collections::HashMap<String, Value>, path: &[&str]) -> Option<String> {
    let mut value = extra.get(path.first().copied()?);
    for key in &path[1..] {
        value = value.and_then(|inner| inner.get(*key));
    }
    value.and_then(Value::as_str).map(str::to_owned)
}

fn facts_from_test(test: &AuthTest) -> AuthFacts {
    AuthFacts {
        team_id: test
            .team_id
            .as_deref()
            .and_then(|id| filled(Some(id)))
            .map(str::to_owned),
        team_name: test
            .team
            .as_deref()
            .and_then(|name| filled(Some(name)))
            .map(str::to_owned),
        domain: test.url.as_deref().and_then(domain_from_url),
        user_name: test
            .user
            .as_deref()
            .and_then(|name| filled(Some(name)))
            .map(str::to_owned),
    }
}

/// `acme.slack.com` or `acme` → `acme`. A host with an extra dot is rejected.
fn workspace_domain(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_end_matches('/').to_ascii_lowercase();
    let raw = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .unwrap_or(&raw);
    let raw = raw.strip_suffix(".slack.com").unwrap_or(raw);
    let raw = raw.split('/').next().unwrap_or(raw);
    (!raw.is_empty() && !raw.contains('.') && !raw.contains(' ')).then(|| raw.to_owned())
}

/// Pair a name with the domain of the same team.
///
/// `expected_team_id` is the row being refreshed. `auth.test` fields are used
/// only when that answer names this team (or names no team). `team.info` wins
/// for both the name and the domain when its id matches, so a host from
/// another workspace is not stored here.
fn pair_identity(
    token_team_id: Option<&str>,
    token_team_name: Option<&str>,
    expected_team_id: Option<&str>,
    auth: &AuthFacts,
    team: Option<&TeamFacts>,
) -> Option<WorkspaceIdentity> {
    let team_id = filled(expected_team_id)
        .or(filled(token_team_id))
        .or(filled(auth.team_id.as_deref()))
        .or(team.and_then(|info| filled(Some(info.team_id.as_str()))))?;
    let auth_matches = match filled(auth.team_id.as_deref()) {
        Some(id) => id == team_id,
        None => true,
    };
    let auth_name = auth_matches
        .then(|| filled(auth.team_name.as_deref()))
        .flatten();
    let auth_domain = auth_matches
        .then(|| filled(auth.domain.as_deref()))
        .flatten();
    let auth_user = auth_matches
        .then(|| filled(auth.user_name.as_deref()))
        .flatten();
    let info =
        team.filter(|info| info.team_id == team_id && filled(Some(info.domain.as_str())).is_some());
    let (team_name, domain) = if let Some(info) = info {
        let name = filled(Some(info.team_name.as_str()))
            .or(auth_name)
            .or(filled(token_team_name))
            .unwrap_or(team_id);
        (name.to_owned(), info.domain.trim().to_owned())
    } else {
        let name = auth_name.or(filled(token_team_name)).unwrap_or(team_id);
        let domain = auth_domain
            .map(str::to_owned)
            .unwrap_or_else(|| team_id.to_ascii_lowercase());
        (name.to_owned(), domain)
    };
    Some(WorkspaceIdentity {
        team_id: team_id.to_owned(),
        team_name,
        domain,
        user_name: auth_user.unwrap_or("").to_owned(),
    })
}

async fn team_info(base: &str, token: &str) -> Result<TeamFacts, String> {
    let response = web_client()?
        .post(format!("{}/team.info", base.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    let body: TeamInfoBody = response.json().await.map_err(|err| err.to_string())?;
    if !body.ok {
        return Err(format!("team.info: {}", body.error.unwrap_or_default()));
    }
    let team = body.team.ok_or_else(|| "team.info: no team".to_owned())?;
    let domain = workspace_domain(&team.domain).ok_or_else(|| "team.info: no domain".to_owned())?;
    let team_id = team.id.trim();
    if team_id.is_empty() {
        return Err("team.info: no team id".into());
    }
    Ok(TeamFacts {
        team_id: team_id.to_owned(),
        team_name: team.name.trim().to_owned(),
        domain,
    })
}

async fn read_identity(
    web_api_base: &str,
    access_token: &str,
    token_team_id: Option<&str>,
    token_team_name: Option<&str>,
    token_user_id: Option<&str>,
    expected_team_id: Option<&str>,
) -> Result<Identified, String> {
    let test = auth_test(web_api_base, access_token).await?;
    let auth = facts_from_test(&test);
    let team = team_info(web_api_base, access_token).await.ok();
    let identity = pair_identity(
        token_team_id,
        token_team_name,
        expected_team_id,
        &auth,
        team.as_ref(),
    )
    .ok_or_else(|| "no team ID".to_owned())?;
    let auth_user_id = match filled(test.team_id.as_deref()) {
        Some(id) if id != identity.team_id => None,
        _ => filled(test.user_id.as_deref()),
    };
    let user_id = filled(token_user_id)
        .or(auth_user_id)
        .ok_or_else(|| "no user ID".to_owned())?
        .to_owned();
    Ok(Identified { identity, user_id })
}

/// Recheck a saved grant, ignoring any previous result. The identity is set
/// only when `auth.test` accepts the token.
pub async fn probe_grant(
    web_api_base: &str,
    vault: Arc<Vault>,
    key: &str,
    expected_team_id: Option<&str>,
) -> GrantProbe {
    install_tls_provider();
    let store = KeychainCredentialStore::new(vault, key.to_owned());
    let token = match store.load().await {
        Ok(Some(credentials)) => credentials.token_response,
        Ok(None) => {
            return GrantProbe {
                state: TokenState::Missing,
                error: String::new(),
                identity: None,
            };
        }
        Err(err) => {
            return GrantProbe {
                state: TokenState::Unknown,
                error: redact_check_error(&err.to_string()),
                identity: None,
            };
        }
    };
    let Some(token) = token else {
        return GrantProbe {
            state: TokenState::Missing,
            error: String::new(),
            identity: None,
        };
    };
    let extra = &token.extra_fields().0;
    let refreshable = token.refresh_token().is_some();
    match read_identity(
        web_api_base,
        token.access_token().secret(),
        extra_str(extra, &["team", "id"]).as_deref(),
        extra_str(extra, &["team", "name"]).as_deref(),
        extra_str(extra, &["authed_user", "id"]).as_deref(),
        expected_team_id,
    )
    .await
    {
        Ok(found) => GrantProbe {
            state: TokenState::Valid,
            error: String::new(),
            identity: Some(found.identity),
        },
        Err(err) => {
            let state = classify_auth_test_error(&err, refreshable);
            GrantProbe {
                error: if state == TokenState::Unknown {
                    redact_check_error(&err)
                } else {
                    String::new()
                },
                state,
                identity: None,
            }
        }
    }
}

/// Drop anything that might be a token. Callers show this string in the UI.
fn redact_check_error(err: &str) -> String {
    let lower = err.to_ascii_lowercase();
    if lower.contains("xox") || lower.contains("bearer ") {
        return "Could not check the Slack sign-in.".into();
    }
    let trimmed = err.trim();
    if trimmed.is_empty() {
        return "Could not reach Slack.".into();
    }
    trimmed.chars().take(200).collect()
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

    #[tokio::test]
    async fn a_listener_must_match_the_configured_redirect() {
        let listener = Loopback::bind_ephemeral("127.0.0.1", "/cb").await.unwrap();
        let config = ConnectConfig {
            mcp_url: "http://127.0.0.1:9/mcp".into(),
            web_api_base: "http://127.0.0.1:9/api".into(),
            client_id: "1111111111.2222222222".into(),
            client_secret: None,
            redirect_uri: "http://127.0.0.1:9/cb".into(),
            scopes: vec!["chat:write".into()],
            timeout: Duration::from_secs(1),
        };
        let err = connect_with_listener(
            &config,
            Arc::new(Vault::memory("test")),
            listener,
            |_| Ok(()),
            &CancelHandle::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SlackAuthError::InvalidRedirect(_)), "{err}");
    }

    #[test]
    fn a_foreign_auth_test_does_not_rename_this_workspace() {
        let auth = AuthFacts {
            team_id: Some("T0LABS0001".into()),
            team_name: Some("Acme Labs".into()),
            domain: Some("acme-labs".into()),
            user_name: Some("ada".into()),
        };
        let identity = pair_identity(
            Some("T0ACME0001"),
            Some("Acme"),
            Some("T0ACME0001"),
            &auth,
            None,
        )
        .expect("team id");
        assert_eq!(identity.team_id, "T0ACME0001");
        assert_eq!(identity.team_name, "Acme");
        assert_eq!(identity.domain, "t0acme0001");
        assert!(identity.user_name.is_empty());
    }

    #[test]
    fn team_info_pairs_name_and_domain_over_a_swapped_url() {
        let auth = AuthFacts {
            team_id: Some("T0ACME0001".into()),
            team_name: Some("Acme".into()),
            domain: Some("acme-labs".into()),
            user_name: Some("ada".into()),
        };
        let team = TeamFacts {
            team_id: "T0ACME0001".into(),
            team_name: "Acme".into(),
            domain: "acme".into(),
        };
        let identity = pair_identity(
            Some("T0ACME0001"),
            Some("Acme"),
            Some("T0ACME0001"),
            &auth,
            Some(&team),
        )
        .expect("team id");
        assert_eq!(identity.team_name, "Acme");
        assert_eq!(identity.domain, "acme");
        assert_eq!(identity.user_name, "ada");
    }

    #[test]
    fn matching_auth_test_is_used_when_team_info_is_missing() {
        let auth = AuthFacts {
            team_id: Some("T0ACME".into()),
            team_name: Some("Acme Inc".into()),
            domain: Some("acme".into()),
            user_name: Some("jdoe".into()),
        };
        let identity = pair_identity(Some("T0ACME"), None, None, &auth, None).expect("team id");
        assert_eq!(identity.team_name, "Acme Inc");
        assert_eq!(identity.domain, "acme");
        assert_eq!(identity.user_name, "jdoe");
    }

    #[test]
    fn check_errors_never_keep_a_token() {
        assert_eq!(
            redact_check_error("auth.test failed for Bearer xoxp-acme"),
            "Could not check the Slack sign-in."
        );
        assert!(!redact_check_error("connection refused").contains("xox"));
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
