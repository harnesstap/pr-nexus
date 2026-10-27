//! The MCP session per workspace and the Slack [`ChatProvider`] (§7.11).
//!
//! The session opens lazily on the first call, checks the tool schema pin,
//! and is reused until it fails; the next call reconnects once. Access
//! tokens come from the keychain grant and refresh through rmcp.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pr_nexus_core::domain::{AccountId, ChatKind};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::{
    ChatChannel, ChatProvider, ChatUser, OutgoingMessage, Page, PostedMessage, SearchHit,
    ThreadReplies,
};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, JsonObject};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::auth::{AuthClient, OAuthClientConfig};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

use super::oauth::{self, KeychainCredentialStore};
use super::schema::{
    self, READ_CHANNEL, READ_THREAD, READ_USER_PROFILE, SEARCH_CHANNELS, SEARCH_PUBLIC,
    SEARCH_PUBLIC_AND_PRIVATE, SEARCH_USERS, SEND_MESSAGE, ToolOutput, ToolShape,
};
use super::{USER_SCOPES, install_tls_provider};
use crate::vault::Vault;

/// Slack allows about one write per second per channel (§7.11).
const WRITE_SPACING: Duration = Duration::from_secs(1);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How far back the send-result recovery reads the channel.
const RECOVERY_WINDOW: Duration = Duration::from_secs(60);

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
    /// User ID → display name, for the session's lifetime.
    names: Mutex<HashMap<String, String>>,
}

impl SlackMcpChatProvider {
    pub fn new(caller: Box<dyn ToolCaller>, account: AccountId, user: ChatUser) -> Self {
        Self {
            caller,
            account,
            user,
            last_write: Mutex::new(HashMap::new()),
            names: Mutex::new(HashMap::new()),
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

impl SlackMcpChatProvider {
    /// Call a read tool; an error result maps through [`schema::map_error`].
    fn read(&self, tool: &str, args: JsonObject) -> Result<ToolOutput, ProviderError> {
        let output = self.caller.call(tool, args)?;
        if output.is_error {
            return Err(schema::map_error(tool, &output.text, None));
        }
        Ok(output)
    }

    /// Find a message just sent: my newest message in the channel from the
    /// last [`RECOVERY_WINDOW`] whose text matches (§7.11).
    fn recover(&self, message: &OutgoingMessage<'_>) -> Option<PostedMessage> {
        let oldest = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .saturating_sub(RECOVERY_WINDOW)
            .as_secs();
        let mut args = JsonObject::new();
        args.insert("channel_id".into(), json!(message.channel_id));
        args.insert("oldest".into(), json!(format!("{oldest}.000000")));
        args.insert("limit".into(), json!(20));
        let output = self.caller.call(READ_CHANNEL, args).ok()?;
        if output.is_error {
            return None;
        }
        let found = schema::parse_read_channel(&output)
            .into_iter()
            .filter(|read| read.user_id.as_deref() == Some(self.user.id.as_str()))
            .filter(|read| schema::same_message(message.text, &read.text))
            .max_by(|a, b| a.ts.cmp(&b.ts))?;
        Some(PostedMessage {
            channel_id: message.channel_id.to_owned(),
            ts: found.ts,
            thread_ts: message.thread_ts.map(str::to_owned),
            permalink: None,
            workspace: None,
        })
    }
}

/// `slack_read_thread` arguments: the first `limit` messages of a thread.
pub fn read_thread_arguments(channel_id: &str, root_ts: &str, limit: u32) -> JsonObject {
    let mut args = JsonObject::new();
    args.insert("channel_id".into(), json!(channel_id));
    args.insert("message_ts".into(), json!(root_ts));
    args.insert("limit".into(), json!(limit));
    args
}

/// Search arguments for the Mentions button (§7.10): the exact phrase,
/// bots included (PR Nexus posts carry the link), newest first.
pub fn search_arguments(phrase: &str, limit: u32) -> JsonObject {
    let mut args = JsonObject::new();
    args.insert("keywords".into(), json!([format!("\"{phrase}\"")]));
    args.insert("include_bots".into(), json!(true));
    args.insert("limit".into(), json!(limit.min(20)));
    args.insert("sort".into(), json!("timestamp"));
    args.insert("sort_dir".into(), json!("desc"));
    args.insert("include_context".into(), json!(false));
    args
}

/// Directory search arguments for the target picker (§7.11): every word of
/// `query` as a keyword, concise results, archived channels left out.
pub fn directory_arguments(query: &str, limit: u32, channels: bool) -> JsonObject {
    let keywords: Vec<&str> = query.split_whitespace().collect();
    let mut args = JsonObject::new();
    args.insert("keywords".into(), json!(keywords));
    args.insert("natural_language_query".into(), json!(""));
    args.insert("limit".into(), json!(limit.clamp(1, 20)));
    args.insert("response_format".into(), json!("concise"));
    if channels {
        args.insert(
            "channel_types".into(),
            json!("public_channel,private_channel"),
        );
        args.insert("include_archived".into(), json!(false));
    }
    args
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
        let mut posted = match schema::parse_send(&output, message.channel_id) {
            Ok(posted) => posted,
            Err(err) => {
                // The call succeeded, so the message is in Slack. Never report
                // a failure here: a retry would post it twice.
                eprintln!(
                    "pr-nexus: {SEND_MESSAGE} result not understood ({err}): {}",
                    output.text.chars().take(300).collect::<String>()
                );
                self.recover(message).ok_or_else(|| {
                    ProviderError::SentUnconfirmed(format!(
                        "{SEND_MESSAGE} succeeded but its result has no message link or ts"
                    ))
                })?
            }
        };
        if posted.thread_ts.is_none() {
            posted.thread_ts = message.thread_ts.map(str::to_owned);
        }
        Ok(posted)
    }

    fn account_id(&self) -> AccountId {
        self.account
    }

    fn read_thread(
        &self,
        channel_id: &str,
        root_ts: &str,
        limit: u32,
    ) -> Result<ThreadReplies, ProviderError> {
        let output = self.read(
            READ_THREAD,
            read_thread_arguments(channel_id, root_ts, limit),
        )?;
        let replies = schema::parse_read_thread(&output);
        if replies.messages.is_empty() {
            return Err(ProviderError::Decode(format!(
                "{READ_THREAD}: no messages in the result"
            )));
        }
        Ok(replies)
    }

    fn user_name(&self, user_id: &str) -> Result<String, ProviderError> {
        if let Some(name) = self.names.lock().expect("names").get(user_id) {
            return Ok(name.clone());
        }
        let mut args = JsonObject::new();
        args.insert("user_id".into(), json!(user_id));
        args.insert("response_format".into(), json!("concise"));
        let output = self.read(READ_USER_PROFILE, args)?;
        let name = schema::parse_user_name(&output).ok_or_else(|| {
            ProviderError::Decode(format!("{READ_USER_PROFILE}: no name in the result"))
        })?;
        self.names
            .lock()
            .expect("names")
            .insert(user_id.to_owned(), name.clone());
        Ok(name)
    }

    fn search_messages(
        &self,
        phrase: &str,
        private: bool,
        limit: u32,
    ) -> Result<Vec<SearchHit>, ProviderError> {
        let tool = if private {
            SEARCH_PUBLIC_AND_PRIVATE
        } else {
            SEARCH_PUBLIC
        };
        let output = self.read(tool, search_arguments(phrase, limit))?;
        Ok(schema::parse_search(&output))
    }

    fn search_channels(&self, query: &str, limit: u32) -> Result<Vec<ChatChannel>, ProviderError> {
        let output = self.read(SEARCH_CHANNELS, directory_arguments(query, limit, true))?;
        Ok(schema::parse_channels(&output))
    }

    fn search_users(&self, query: &str, limit: u32) -> Result<Vec<ChatUser>, ProviderError> {
        let output = self.read(SEARCH_USERS, directory_arguments(query, limit, false))?;
        Ok(schema::parse_users(&output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        calls: Mutex<Vec<(String, JsonObject)>>,
        reply: ToolOutput,
        read: Option<ToolOutput>,
    }

    impl ToolCaller for Arc<Fake> {
        fn call(&self, tool: &str, arguments: JsonObject) -> Result<ToolOutput, ProviderError> {
            self.calls
                .lock()
                .unwrap()
                .push((tool.to_owned(), arguments));
            if tool == READ_CHANNEL {
                return self
                    .read
                    .clone()
                    .ok_or_else(|| ProviderError::Network("offline".into()));
            }
            Ok(self.reply.clone())
        }
    }

    fn provider(reply: ToolOutput) -> (SlackMcpChatProvider, Arc<Fake>) {
        provider_with_read(reply, None)
    }

    fn provider_with_read(
        reply: ToolOutput,
        read: Option<ToolOutput>,
    ) -> (SlackMcpChatProvider, Arc<Fake>) {
        let fake = Arc::new(Fake {
            calls: Mutex::new(Vec::new()),
            reply,
            read,
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
    fn target_search_calls_the_directory_tools() {
        let (slack, fake) = provider(text(
            r###"{"results":"## Channels (1 results)\n1. #team-reviews (C0ACME0001) - public_channel - Not Archived"}"###,
        ));
        let channels = slack.search_channels("team reviews", 50).unwrap();
        assert_eq!(channels[0].id, "C0ACME0001");
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls[0].0, SEARCH_CHANNELS);
        assert_eq!(calls[0].1["keywords"], json!(["team", "reviews"]));
        assert_eq!(calls[0].1["limit"], json!(20));
        assert_eq!(
            calls[0].1["channel_types"],
            json!("public_channel,private_channel")
        );
        drop(calls);
        assert!(slack.search_users("grace", 5).unwrap().is_empty());
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls[1].0, SEARCH_USERS);
        assert!(calls[1].1.get("channel_types").is_none());
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

    fn dm(text: &str) -> OutgoingMessage<'_> {
        OutgoingMessage {
            channel_id: "D0123456789",
            text,
            thread_ts: None,
            reply_broadcast: false,
            unfurl_app_links: true,
        }
    }

    #[test]
    fn unreadable_result_is_recovered_from_the_channel() {
        let read = text(
            r#"{"messages":"=== Message from me (U0ME) at now === \nMessage TS: 1700000000.000300\nReview <https:\/\/github.com\/acme\/web\/pull\/2|github.com\/acme\/web\/pull\/2>\n=== Message from me (U0ME) at now === \nMessage TS: 1700000000.000200\nReview <https:\/\/github.com\/acme\/web\/pull\/1|github.com\/acme\/web\/pull\/1>\n*Sent using* <@U0APP|PR Nexus>\n=== Message from other (U0OTHER) at now === \nMessage TS: 1700000000.000400\nReview https:\/\/github.com\/acme\/web\/pull\/1"}"#,
        );
        let (slack, fake) = provider_with_read(text("Message sent."), Some(read));
        let posted = slack
            .post(&dm("Review [web#1](https://github.com/acme/web/pull/1)"))
            .unwrap();
        assert_eq!(posted.ts, "1700000000.000200");
        assert_eq!(posted.channel_id, "D0123456789");
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls[1].0, READ_CHANNEL);
        assert_eq!(calls[1].1["channel_id"], json!("D0123456789"));
    }

    #[test]
    fn unrecoverable_result_is_sent_unconfirmed_not_failed() {
        let (slack, _) = provider_with_read(text("Message sent."), None);
        let err = slack.post(&dm("hello")).unwrap_err();
        assert!(matches!(err, ProviderError::SentUnconfirmed(_)));
        let (slack, _) =
            provider_with_read(text("Message sent."), Some(text(r#"{"messages":""}"#)));
        assert!(matches!(
            slack.post(&dm("hello")).unwrap_err(),
            ProviderError::SentUnconfirmed(_)
        ));
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

    /// The official server's result shape: one text block holding JSON
    /// (`message_context.message_ts` / `channel_id`, slashes escaped).
    #[test]
    fn documented_result_fixtures_parse() {
        let send: rmcp::model::CallToolResult = serde_json::from_str(include_str!(
            "../../tests/fixtures/slack-mcp-send-message.json"
        ))
        .unwrap();
        let posted = schema::parse_send(&to_output(send), "D0123456789").unwrap();
        assert_eq!(posted.ts, "1700000000.000100");
        assert_eq!(posted.channel_id, "D0123456789");

        let read: rmcp::model::CallToolResult = serde_json::from_str(include_str!(
            "../../tests/fixtures/slack-mcp-read-channel.json"
        ))
        .unwrap();
        let messages = schema::parse_read_channel(&to_output(read));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].user_id.as_deref(), Some("U0ACME0001"));
        assert!(schema::same_message(
            "Hello team\n[Fix login](https://github.com/acme/web/pull/1)",
            &messages[0].text
        ));
    }

    fn fixture(json: &str) -> ToolOutput {
        let result: rmcp::model::CallToolResult = serde_json::from_str(json).unwrap();
        to_output(result)
    }

    /// The panel's read tools (§7.10), on the official server's shapes.
    #[test]
    fn thread_profile_and_search_fixtures_parse() {
        let thread = schema::parse_read_thread(&fixture(include_str!(
            "../../tests/fixtures/slack-mcp-read-thread.json"
        )));
        assert!(!thread.has_more);
        let summary: Vec<(&str, Option<&str>, Option<&str>)> = thread
            .messages
            .iter()
            .map(|m| (m.ts.as_str(), m.user_id.as_deref(), m.author.as_deref()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("1700000000.000100", Some("U0ACME0001"), Some("Ada Acme")),
                ("1700000300.000200", Some("U0ACME0002"), Some("Bob Acme")),
                ("1700003600.000300", Some("U0ACME0001"), Some("Ada Acme")),
            ]
        );
        assert!(
            thread.messages[0]
                .text
                .starts_with("Hello team\nCould someone")
        );
        assert_eq!(thread.messages[1].text, "On it\n\nLeft two comments.");
        assert!(
            thread
                .messages
                .iter()
                .all(|m| !m.text.contains('@')
                    && !m.author.as_deref().unwrap_or_default().contains('@')),
            "emails never kept"
        );

        let name = schema::parse_user_name(&fixture(include_str!(
            "../../tests/fixtures/slack-mcp-user-profile.json"
        )));
        assert_eq!(name.as_deref(), Some("Bob Acme"));

        let hits = schema::parse_search(&fixture(include_str!(
            "../../tests/fixtures/slack-mcp-search.json"
        )));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].channel_id, "C0123456789");
        assert_eq!(hits[0].channel_name, "eng-web");
        assert_eq!(hits[0].author.as_deref(), Some("Bob Acme"));
        assert_eq!(hits[0].ts, "1700007200.000400");
        assert_eq!(
            hits[0].text,
            "Does github.com/acme/web/pull/1 fix the login loop?"
        );
        assert!(!hits[0].is_bot);
        assert!(hits[1].is_bot);
        assert_eq!(hits[1].author.as_deref(), Some("Devel"));
        assert_eq!(
            hits[1].permalink.as_deref(),
            Some(
                "https://acme.slack.com/archives/C0123456780/p1700010800000500?thread_ts=1700010000.000100&cid=C0123456780"
            )
        );
    }

    #[test]
    fn panel_reads_call_the_right_tools_and_cache_names() {
        let (slack, fake) = provider(fixture(include_str!(
            "../../tests/fixtures/slack-mcp-read-thread.json"
        )));
        let replies = slack
            .read_thread("C0TEAM", "1700000000.000100", 50)
            .unwrap();
        assert_eq!(replies.messages.len(), 3);
        {
            let calls = fake.calls.lock().unwrap();
            assert_eq!(calls[0].0, READ_THREAD);
            assert_eq!(calls[0].1["message_ts"], "1700000000.000100");
            assert_eq!(calls[0].1["limit"], 50);
        }

        let (slack, fake) = provider(fixture(include_str!(
            "../../tests/fixtures/slack-mcp-user-profile.json"
        )));
        assert_eq!(slack.user_name("U0ACME0002").unwrap(), "Bob Acme");
        assert_eq!(slack.user_name("U0ACME0002").unwrap(), "Bob Acme");
        assert_eq!(fake.calls.lock().unwrap().len(), 1, "names are cached");

        let (slack, fake) = provider(fixture(include_str!(
            "../../tests/fixtures/slack-mcp-search.json"
        )));
        slack
            .search_messages("github.com/acme/web/pull/1", false, 20)
            .unwrap();
        slack
            .search_messages("github.com/acme/web/pull/1", true, 50)
            .unwrap();
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls[0].0, SEARCH_PUBLIC);
        assert_eq!(
            calls[0].1["keywords"],
            json!(["\"github.com/acme/web/pull/1\""])
        );
        assert_eq!(calls[0].1["include_bots"], true);
        assert_eq!(calls[1].0, SEARCH_PUBLIC_AND_PRIVATE);
        assert_eq!(calls[1].1["limit"], 20);
    }

    #[test]
    fn a_tool_error_on_read_maps_to_reconnect() {
        let (slack, _) = provider(ToolOutput {
            text: "invalid_auth".into(),
            is_error: true,
            ..ToolOutput::default()
        });
        assert!(matches!(
            slack.read_thread("C0TEAM", "1700000000.000100", 50),
            Err(ProviderError::Unauthorized(_))
        ));
    }
}
