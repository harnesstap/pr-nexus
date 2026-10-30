//! The HTTP side: binds `127.0.0.1`, checks Host, Origin and the bearer
//! token on every request, then hands MCP to `rmcp`'s Streamable HTTP
//! service at `/mcp`.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ErrorData, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::audit::{AuditEvent, AuditSink, Outcome, summarize};
use super::tools::{ToolSpec, registry};
use super::{Capability, McpSettings, token};
use crate::error::AppError;
use crate::state::AppState;

/// Path of the MCP endpoint.
pub const PATH: &str = "/mcp";

const INSTRUCTIONS: &str = "PR Nexus tracks the user's GitHub pull requests, groups them into projects \
    and follows their Slack review requests. Start with list_projects or list_prs; ids returned by one \
    tool are the ids the others take. Notes are the user's private annotations.";

/// What the server reads on every request, so Settings changes (toggles,
/// a new token) apply without a restart.
pub struct Shared {
    pub state: Arc<AppState>,
    pub settings: RwLock<McpSettings>,
    pub token: RwLock<String>,
    pub audit: Arc<dyn AuditSink>,
    /// Runs after each successful write, so the app window reloads.
    pub on_write: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Shared {
    fn settings(&self) -> McpSettings {
        self.settings.read().expect("mcp settings").clone()
    }

    /// After an agent write: the list's idle poll sees a new revision, and
    /// the window (when there is one) is told at once.
    fn wrote(&self) {
        self.state.sync.lock().expect("sync status").revision += 1;
        if let Some(notify) = &self.on_write {
            notify();
        }
    }
}

/// The MCP handler: lists the tools the settings allow and runs them on
/// a blocking thread (the store and providers block).
#[derive(Clone)]
pub struct NexusMcp {
    shared: Arc<Shared>,
    tools: Arc<Vec<ToolSpec>>,
}

impl NexusMcp {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            tools: Arc::new(registry()),
        }
    }
}

impl ServerHandler for NexusMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("pr-nexus", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let settings = self.shared.settings();
        Ok(ListToolsResult::with_all_items(
            self.tools
                .iter()
                .filter(|spec| settings.allows(spec.capability))
                .map(ToolSpec::tool)
                .collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some(index) = self.tools.iter().position(|spec| spec.name == request.name) else {
            return Err(ErrorData::invalid_params(
                format!("Unknown tool {}", request.name),
                None,
            ));
        };
        let spec = &self.tools[index];
        let arguments = request.arguments.unwrap_or_default();
        let client = context
            .peer
            .peer_info()
            .map(|info| info.client_info.name.clone());
        let started = Instant::now();
        let audit =
            |outcome: Outcome, result: Option<&serde_json::Value>, error: Option<String>| {
                self.shared.audit.record(&AuditEvent {
                    tool: spec.name.to_owned(),
                    capability: spec.capability,
                    summary: summarize(spec.name, &arguments, result),
                    arguments: arguments.clone(),
                    outcome,
                    duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    client: client.clone(),
                    error,
                    link: (spec.capability == Capability::SlackSends)
                        .then(|| super::audit::write_link(result))
                        .flatten(),
                });
            };
        if !self.shared.settings().allows(spec.capability) {
            let message = format!(
                "{} is turned off in PR Nexus Settings → Agents (MCP).",
                spec.name
            );
            audit(Outcome::Denied, None, Some(message.clone()));
            return Ok(CallToolResult::structured_error(json!({
                "kind": "forbidden",
                "message": message,
            }))
            .into());
        }
        let state = Arc::clone(&self.shared.state);
        let handler = spec.handler;
        let input = arguments.clone();
        let caller = super::slack::session::Caller {
            session: context
                .extensions
                .get::<axum::http::request::Parts>()
                .and_then(|parts| parts.headers.get("mcp-session-id"))
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            client: client.clone(),
            ask: super::slack::elicit::PeerAsk::for_peer(&context.peer),
        };
        let result = tokio::task::spawn_blocking(move || {
            super::slack::session::with(caller, || handler(&state, input))
        })
        .await
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
        Ok(match result {
            Ok(value) => {
                audit(Outcome::Ok, Some(&value), None);
                if spec.capability == Capability::LocalWrites {
                    self.shared.wrote();
                }
                CallToolResult::structured(value)
            }
            Err(err) => {
                audit(
                    Outcome::Failed {
                        kind: err.kind.clone(),
                    },
                    None,
                    Some(err.message.clone()),
                );
                CallToolResult::structured_error(
                    json!({ "kind": err.kind, "message": err.message }),
                )
            }
        }
        .into())
    }
}

/// A started server. Dropping it does not stop it; call [`Running::stop`].
pub struct Running {
    pub addr: SocketAddr,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    pub fn url(&self) -> String {
        format!("http://{}{PATH}", self.addr)
    }

    pub async fn stop(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

/// Binds `127.0.0.1:{port}` (any free port when `None`) and serves until
/// stopped. A taken port is a `port_in_use` error.
pub async fn start(shared: Arc<Shared>, port: Option<u16>) -> Result<Running, AppError> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port.unwrap_or(0)))
        .await
        .map_err(|err| match (err.kind(), port) {
            (std::io::ErrorKind::AddrInUse, Some(port)) => AppError::new(
                "port_in_use",
                format!("Port {port} is already in use on 127.0.0.1. Pick another port or leave it empty for any free port."),
            ),
            _ => AppError::new("internal", format!("Could not start the MCP server: {err}")),
        })?;
    let addr = listener
        .local_addr()
        .map_err(|err| AppError::new("internal", err.to_string()))?;
    let cancel = CancellationToken::new();
    let config = StreamableHttpServerConfig::default()
        .with_cancellation_token(cancel.child_token())
        .with_allowed_hosts(allowed_hosts(addr.port()));
    let handler = NexusMcp::new(Arc::clone(&shared));
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let guard = Guard {
        shared,
        port: addr.port(),
    };
    let router = Router::new()
        .nest_service(PATH, service)
        .layer(middleware::from_fn_with_state(guard, check));
    let shutdown = cancel.clone();
    let task = tokio::spawn(async move {
        let served = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
            .await;
        if let Err(err) = served {
            tracing::warn!(error = %err, "mcp server stopped");
        }
    });
    tracing::info!(%addr, "mcp server listening");
    Ok(Running { addr, cancel, task })
}

fn allowed_hosts(port: u16) -> Vec<String> {
    vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")]
}

#[derive(Clone)]
struct Guard {
    shared: Arc<Shared>,
    port: u16,
}

/// Rejects, in order: a Host that is not this loopback address (DNS
/// rebinding), an Origin from anywhere but this server (browser pages),
/// and a missing or wrong bearer token.
async fn check(State(guard): State<Guard>, request: Request, next: Next) -> Response {
    match refusal(request.headers(), &guard) {
        None => next.run(request).await,
        Some(response) => response,
    }
}

fn refusal(headers: &HeaderMap, guard: &Guard) -> Option<Response> {
    let text = |name: header::HeaderName| {
        headers
            .get(name)
            .map(|value| value.to_str().unwrap_or("\u{0}"))
    };
    let hosts = allowed_hosts(guard.port);
    if !text(header::HOST).is_some_and(|host| {
        hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    }) {
        return Some(refuse(StatusCode::FORBIDDEN, "Host not allowed"));
    }
    if let Some(origin) = text(header::ORIGIN)
        && !hosts
            .iter()
            .any(|allowed| origin.eq_ignore_ascii_case(&format!("http://{allowed}")))
    {
        return Some(refuse(StatusCode::FORBIDDEN, "Origin not allowed"));
    }
    let expected = guard.shared.token.read().expect("mcp token").clone();
    if !token::authorized(text(header::AUTHORIZATION), &expected) {
        let mut response = refuse(StatusCode::UNAUTHORIZED, "Missing or invalid bearer token");
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            header::HeaderValue::from_static("Bearer"),
        );
        return Some(response);
    }
    None
}

fn refuse(status: StatusCode, message: &'static str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": message }).to_string(),
    )
        .into_response()
}
