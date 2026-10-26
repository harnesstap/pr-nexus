//! The MCP session per workspace and the Slack [`ChatProvider`] (§7.11).
//!
//! The session opens lazily on the first call, checks the tool schema pin,
//! and is reused until it fails; the next call reconnects once. Access
//! tokens come from the keychain grant and refresh through rmcp.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pr_nexus_core::domain::{AccountId, ChatKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{
    ChatChannel, ChatProvider, ChatUser, OutgoingMessage, Page, PostedMessage,
};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, JsonObject};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::auth::{AuthClient, OAuthClientConfig};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

use super::oauth::{self, KeychainCredentialStore};
use super::schema::{self, SEND_MESSAGE, ToolOutput, ToolShape};
use super::{USER_SCOPES, install_tls_provider};
use crate::vault::Vault;

/// Slack allows about one write per second per channel (§7.11).
const WRITE_SPACING: Duration = Duration::from_secs(1);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// One MCP tool call, blocking. Lets the provider be tested without a server.
pub trait ToolCaller: Send + Sync {
    fn call(&self, tool: &str, arguments: JsonObject) -> Result<ToolOutput, ProviderError>;
}

/// Everything needed to reopen a workspace session after a restart.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub mcp_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub keychain_key: String,
}

type Client = RunningService<RoleClient, ()>;

/// rmcp session on its own small runtime, so callers stay synchronous.
pub struct RmcpSession {
    runtime: Option<tokio::runtime::Runtime>,
    inner: Arc<SessionInner>,
}

struct SessionInner {
    config: SessionConfig,
    vault: Arc<Vault>,
    client: tokio::sync::Mutex<Option<Client>>,
}

impl RmcpSession {
    pub fn new(config: SessionConfig, vault: Arc<Vault>) -> Result<Self, ProviderError> {
        install_tls_provider();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("slack-mcp")
            .enable_all()
            .build()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        Ok(Self {
            runtime: Some(runtime),
            inner: Arc::new(SessionInner {
                config,
                vault,
                client: tokio::sync::Mutex::new(None),
            }),
        })
    }

    fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, ProviderError>> + Send + 'static,
    ) -> Result<T, ProviderError> {
        let (tx, rx) = std::sync::mpsc::channel();
        let runtime = self.runtime.as_ref().expect("runtime lives until drop");
        runtime.spawn(async move {
            let _ = tx.send(work.await);
        });
        rx.recv()
            .map_err(|_| ProviderError::Network("Slack MCP task stopped".into()))?
    }
}

impl Drop for RmcpSession {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl ToolCaller for RmcpSession {
    fn call(&self, tool: &str, arguments: JsonObject) -> Result<ToolOutput, ProviderError> {
        let inner = self.inner.clone();
        let tool = tool.to_owned();
        self.run(async move { inner.call(&tool, arguments).await })
    }
}

impl SessionInner {
    async fn call(&self, tool: &str, arguments: JsonObject) -> Result<ToolOutput, ProviderError> {
        let mut guard = self.client.lock().await;
        for attempt in 0..2 {
            if guard.as_ref().is_none_or(|client| client.is_closed()) {
                *guard = Some(self.open().await?);
            }
            let client = guard.as_ref().expect("session just opened");
            let params =
                CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments.clone());
            let result = tokio::time::timeout(CALL_TIMEOUT, client.call_tool(params)).await;
            match result {
                Ok(Ok(result)) => return Ok(to_output(result)),
                Ok(Err(err)) => {
                    let text = err.to_string();
                    let transport = matches!(
                        err,
                        rmcp::ServiceError::TransportClosed | rmcp::ServiceError::TransportSend(_)
                    );
                    *guard = None;
                    if transport && attempt == 0 && !is_auth_text(&text) {
                        continue;
                    }
                    return Err(map_service_error(tool, &text));
                }
                Err(_) => {
                    *guard = None;
                    return Err(ProviderError::Network(format!(
                        "{tool}: Slack MCP timed out"
                    )));
                }
            }
        }
        Err(ProviderError::Network(format!(
            "{tool}: Slack MCP connection lost"
        )))
    }

    async fn open(&self) -> Result<Client, ProviderError> {
        let config = &self.config;
        let mut manager = oauth::new_manager(&config.mcp_url)
            .await
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        let mut client_config =
            OAuthClientConfig::new(config.client_id.clone(), config.redirect_uri.clone())
                .with_scopes(
                    USER_SCOPES
                        .iter()
                        .map(|scope| (*scope).to_owned())
                        .collect(),
                );
        if let Some(secret) = config.client_secret.as_deref().filter(|s| !s.is_empty()) {
            client_config = client_config.with_client_secret(secret);
        }
        manager
            .configure_client(client_config)
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        manager.set_credential_store(KeychainCredentialStore::new(
            self.vault.clone(),
            config.keychain_key.clone(),
        ));
        let http = reqwest013::Client::builder()
            .timeout(CALL_TIMEOUT)
            .build()
            .map_err(|err| ProviderError::Network(err.to_string()))?;
        let transport = StreamableHttpClientTransport::with_client(
            AuthClient::new(http, manager),
            StreamableHttpClientTransportConfig::with_uri(config.mcp_url.clone()),
        );
        let client = ()
            .serve(transport)
            .await
            .map_err(|err| map_service_error("initialize", &err.to_string()))?;
        let tools = client
            .list_all_tools()
            .await
            .map_err(|err| map_service_error("tools/list", &err.to_string()))?;
        let shapes: Vec<ToolShape> = tools
            .iter()
            .map(|tool| ToolShape {
                name: tool.name.to_string(),
                properties: tool
                    .input_schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|props| props.keys().cloned().collect())
                    .unwrap_or_default(),
            })
            .collect();
        if let Some(mismatch) = schema::pin_mismatch(&shapes) {
            return Err(ProviderError::Api {
                code: "schema_changed".into(),
                message: mismatch,
            });
        }
        Ok(client)
    }
}

fn to_output(result: rmcp::model::CallToolResult) -> ToolOutput {
    let text = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    ToolOutput {
        text,
        structured: result.structured_content,
        is_error: result.is_error.unwrap_or(false),
    }
}

fn is_auth_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("auth required")
        || lower.contains("authorization required")
        || lower.contains("auth error")
        || lower.contains("unauthorized")
}

fn map_service_error(tool: &str, text: &str) -> ProviderError {
    if is_auth_text(text) {
        return ProviderError::Unauthorized(
            "Slack sign-in expired or was revoked. Reconnect the workspace in Settings.".into(),
        );
    }
    if text.to_ascii_lowercase().contains("insufficient scope") {
        return ProviderError::Forbidden(format!("{tool}: missing_scope"));
    }
    match schema::map_error(tool, text, None) {
        ProviderError::Api { code, .. } if code == "tool_error" => {
            ProviderError::Network(format!("{tool}: {text}"))
        }
        other => other,
    }
}

/// Slack as a [`ChatProvider`], for one signed-in workspace.
pub struct SlackMcpChatProvider {
    caller: Box<dyn ToolCaller>,
    account: AccountId,
    user: ChatUser,
    last_write: Mutex<HashMap<String, Instant>>,
}

impl SlackMcpChatProvider {
    pub fn new(caller: Box<dyn ToolCaller>, account: AccountId, user: ChatUser) -> Self {
        Self {
            caller,
            account,
            user,
            last_write: Mutex::new(HashMap::new()),
        }
    }

    /// The real provider over the official MCP server.
    pub fn connect(
        config: SessionConfig,
        vault: Arc<Vault>,
        account: AccountId,
        user: ChatUser,
    ) -> Result<Self, ProviderError> {
        Ok(Self::new(
            Box::new(RmcpSession::new(config, vault)?),
            account,
            user,
        ))
    }

    fn throttle(&self, channel_id: &str) {
        let wait = {
            let last = self.last_write.lock().expect("throttle");
            last.get(channel_id)
                .map(|at| WRITE_SPACING.saturating_sub(at.elapsed()))
                .unwrap_or_default()
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        self.last_write
            .lock()
            .expect("throttle")
            .insert(channel_id.to_owned(), Instant::now());
    }
}

/// `slack_send_message` arguments (§7.11 tool table).
pub fn send_arguments(message: &OutgoingMessage<'_>) -> JsonObject {
    let mut args = JsonObject::new();
    args.insert("channel_id".into(), json!(message.channel_id));
    args.insert("message".into(), json!(message.text));
    if let Some(thread_ts) = message.thread_ts {
        args.insert("thread_ts".into(), json!(thread_ts));
        args.insert("reply_broadcast".into(), json!(message.reply_broadcast));
    }
    if message.unfurl_app_links {
        args.insert("unfurl_app_links".into(), json!(true));
    }
    args
}

impl ChatProvider for SlackMcpChatProvider {
    fn kind(&self) -> ChatKind {
        ChatKind::Slack
    }

    fn whoami(&self) -> Result<ChatUser, ProviderError> {
        Ok(self.user.clone())
    }

    fn list_channels(&self, _cursor: Option<String>) -> Result<Page<ChatChannel>, ProviderError> {
        Err(ProviderError::Unsupported(
            "channel search is not available yet; enter the channel ID".into(),
        ))
    }

    fn channel_info(&self, _channel_id: &str) -> Result<ChatChannel, ProviderError> {
        Err(ProviderError::Unsupported(
            "channel lookup is not available yet; enter the channel ID".into(),
        ))
    }

    fn post(&self, message: &OutgoingMessage<'_>) -> Result<PostedMessage, ProviderError> {
        self.throttle(message.channel_id);
        let output = self.caller.call(SEND_MESSAGE, send_arguments(message))?;
        if output.is_error {
            return Err(schema::map_error(SEND_MESSAGE, &output.text, None));
        }
        let mut posted = schema::parse_send(&output, message.channel_id)?;
        if posted.thread_ts.is_none() {
            posted.thread_ts = message.thread_ts.map(str::to_owned);
        }
        Ok(posted)
    }

    fn account_id(&self) -> AccountId {
        self.account
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        calls: Mutex<Vec<(String, JsonObject)>>,
        reply: ToolOutput,
    }

    impl ToolCaller for Arc<Fake> {
        fn call(&self, tool: &str, arguments: JsonObject) -> Result<ToolOutput, ProviderError> {
            self.calls
                .lock()
                .unwrap()
                .push((tool.to_owned(), arguments));
            Ok(self.reply.clone())
        }
    }

    fn provider(reply: ToolOutput) -> (SlackMcpChatProvider, Arc<Fake>) {
        let fake = Arc::new(Fake {
            calls: Mutex::new(Vec::new()),
            reply,
        });
        let user = ChatUser {
            id: "U0ME".into(),
            name: "me".into(),
        };
        (
            SlackMcpChatProvider::new(Box::new(fake.clone()), AccountId(7), user),
            fake,
        )
    }

    fn text(value: &str) -> ToolOutput {
        ToolOutput {
            text: value.into(),
            ..ToolOutput::default()
        }
    }

    #[test]
    fn request_posts_top_level_with_unfurl() {
        let (slack, fake) = provider(text(
            "https://acme.slack.com/archives/C0TEAM/p1700000000000100",
        ));
        let posted = slack
            .post(&OutgoingMessage {
                channel_id: "C0TEAM",
                text: "hello",
                thread_ts: None,
                reply_broadcast: false,
                unfurl_app_links: true,
            })
            .unwrap();
        assert_eq!(posted.ts, "1700000000.000100");
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls[0].0, SEND_MESSAGE);
        assert_eq!(
            Value::Object(calls[0].1.clone()),
            json!({"channel_id": "C0TEAM", "message": "hello", "unfurl_app_links": true})
        );
    }

    #[test]
    fn bump_replies_in_thread_with_broadcast() {
        let (slack, fake) = provider(text(
            "https://acme.slack.com/archives/C0TEAM/p1700000500000200?thread_ts=1700000000.000100",
        ));
        let posted = slack
            .post(&OutgoingMessage {
                channel_id: "C0TEAM",
                text: "bump",
                thread_ts: Some("1700000000.000100"),
                reply_broadcast: true,
                unfurl_app_links: false,
            })
            .unwrap();
        assert_eq!(posted.thread_ts.as_deref(), Some("1700000000.000100"));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(
            Value::Object(calls[0].1.clone()),
            json!({"channel_id": "C0TEAM", "message": "bump", "thread_ts": "1700000000.000100", "reply_broadcast": true})
        );
    }

    #[test]
    fn tool_errors_are_mapped() {
        let (slack, _) = provider(ToolOutput {
            text: "Error: not_in_channel".into(),
            is_error: true,
            ..ToolOutput::default()
        });
        let err = slack
            .post(&OutgoingMessage {
                channel_id: "C1",
                text: "x",
                thread_ts: None,
                reply_broadcast: false,
                unfurl_app_links: true,
            })
            .unwrap_err();
        assert!(matches!(err, ProviderError::Api { code, .. } if code == "not_in_channel"));
    }

    #[test]
    fn writes_to_one_channel_are_spaced() {
        let (slack, _) = provider(text("https://acme.slack.com/archives/C1/p1700000000000100"));
        let message = OutgoingMessage {
            channel_id: "C1",
            text: "x",
            thread_ts: None,
            reply_broadcast: false,
            unfurl_app_links: false,
        };
        let start = Instant::now();
        slack.post(&message).unwrap();
        slack.post(&message).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(900));
    }

    #[test]
    fn service_errors_map_to_reconnect_hint() {
        assert!(matches!(
            map_service_error("t", "Transport send error: Auth required"),
            ProviderError::Unauthorized(_)
        ));
        assert!(matches!(
            map_service_error("t", "Transport closed"),
            ProviderError::Network(_)
        ));
    }
}
