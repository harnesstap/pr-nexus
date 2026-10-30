//! Starts and stops the server from Settings, without an app restart, and
//! the commands the Settings section calls.

use std::sync::{Arc, RwLock};

use pr_nexus_core::store::AuditOutcome;
use serde::Serialize;
use tauri::State;
use tokio::sync::Mutex;

use super::Capability;
use super::audit::{AuditEvent, AuditSink, Fanout, LogAudit, Outcome, StoreAudit};
use super::server::{self, Running, Shared};
use super::slack::sends::{self, Proposal, Status};
use super::{McpSettings, clients, config, token};
use crate::blocking;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    #[default]
    Off,
    Running,
    Error,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct McpStatus {
    pub state: ServerState,
    /// `http://127.0.0.1:{port}/mcp` while running.
    pub url: Option<String>,
    pub port: Option<u16>,
    pub error: Option<String>,
}

/// What Settings shows. The token itself only goes out through [`mcp_token`].
#[derive(Debug, Clone, Serialize)]
pub struct McpView {
    pub settings: McpSettings,
    pub status: McpStatus,
    pub token_masked: Option<String>,
}

pub struct McpControl {
    shared: Arc<Shared>,
    running: Mutex<Option<Running>>,
    status: RwLock<McpStatus>,
}

impl McpControl {
    /// Reads the saved settings. Touches the keyring only once enabled.
    /// Calls land in the Activity log and the app log; `on_write` runs after
    /// each successful write.
    pub fn new(
        state: Arc<AppState>,
        on_write: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<Self, AppError> {
        Self::with_audit(state, Arc::new(LogAudit), on_write)
    }

    /// As [`McpControl::new`], with `extra` in place of the log line.
    pub fn with_audit(
        state: Arc<AppState>,
        extra: Arc<dyn AuditSink>,
        on_write: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<Self, AppError> {
        let settings = config::load(&state.store)?;
        let audit = Arc::new(Fanout(vec![
            Arc::new(StoreAudit(Arc::clone(&state))),
            extra,
        ]));
        Ok(Self {
            shared: Arc::new(Shared {
                state,
                settings: RwLock::new(settings),
                token: RwLock::new(String::new()),
                audit,
                on_write,
            }),
            running: Mutex::new(None),
            status: RwLock::new(McpStatus::default()),
        })
    }

    #[cfg(test)]
    pub fn app_state(&self) -> &Arc<AppState> {
        &self.shared.state
    }

    pub fn settings(&self) -> McpSettings {
        self.shared.settings.read().expect("mcp settings").clone()
    }

    pub fn status(&self) -> McpStatus {
        self.status.read().expect("mcp status").clone()
    }

    pub fn view(&self) -> McpView {
        let token = self.shared.token.read().expect("mcp token").clone();
        McpView {
            settings: self.settings(),
            status: self.status(),
            token_masked: (!token.is_empty()).then(|| token::masked(&token)),
        }
    }

    /// Saves `settings` and brings the server in line with them.
    pub async fn update(&self, settings: McpSettings) -> Result<McpView, AppError> {
        let state = Arc::clone(&self.shared.state);
        let saved = blocking::run(move || config::save(&state.store, &settings)).await?;
        *self.shared.settings.write().expect("mcp settings") = saved;
        self.apply().await;
        Ok(self.view())
    }

    /// The token, created on first use.
    pub async fn token(&self) -> Result<String, AppError> {
        let current = self.shared.token.read().expect("mcp token").clone();
        if !current.is_empty() {
            return Ok(current);
        }
        let state = Arc::clone(&self.shared.state);
        let token = blocking::run(move || token::ensure(&state.vault)).await?;
        *self.shared.token.write().expect("mcp token") = token.clone();
        Ok(token)
    }

    /// A new token; the running server rejects the old one at once.
    pub async fn regenerate(&self) -> Result<McpView, AppError> {
        let state = Arc::clone(&self.shared.state);
        let token = blocking::run(move || token::regenerate(&state.vault)).await?;
        *self.shared.token.write().expect("mcp token") = token;
        Ok(self.view())
    }

    /// Starts, stops or moves the server to match the settings. Failures
    /// land in [`McpControl::status`] rather than an error.
    pub async fn apply(&self) {
        let settings = self.settings();
        let mut running = self.running.lock().await;
        let wanted_port_differs = |current: &Running| {
            settings
                .port
                .is_some_and(|port| port != current.addr.port())
        };
        if let Some(current) = running.take() {
            if settings.enabled && !wanted_port_differs(&current) {
                *running = Some(current);
                return;
            }
            current.stop().await;
        }
        if !settings.enabled {
            self.set_status(McpStatus::default());
            return;
        }
        let started = match self.token().await {
            Ok(_) => server::start(Arc::clone(&self.shared), settings.port).await,
            Err(err) => Err(err),
        };
        match started {
            Ok(server) => {
                self.set_status(McpStatus {
                    state: ServerState::Running,
                    url: Some(server.url()),
                    port: Some(server.addr.port()),
                    error: None,
                });
                *running = Some(server);
            }
            Err(err) => {
                tracing::warn!(error = %err.message, "mcp server did not start");
                self.set_status(McpStatus {
                    state: ServerState::Error,
                    url: None,
                    port: settings.port,
                    error: Some(err.message),
                });
            }
        }
    }

    /// The user approves (Slack is called now) or rejects proposal `id`.
    /// Either way it lands in Activity.
    pub async fn decide(&self, id: u64, approve: bool) -> Result<Proposal, AppError> {
        let state = Arc::clone(&self.shared.state);
        let started = std::time::Instant::now();
        let decided = blocking::run(move || sends::decide(&state, id, approve)).await;
        let verb = if approve { "approved" } else { "rejected" };
        let (tool, arguments, outcome, summary, error) = match &decided {
            Ok(proposal) => {
                let mut arguments = serde_json::Map::new();
                arguments.insert("channel_id".into(), proposal.channel_id.clone().into());
                let (outcome, error, done) = match proposal.status {
                    Status::Failed => {
                        let kind = proposal.error.as_ref().and_then(|e| e["kind"].as_str());
                        let message = proposal.error.as_ref().and_then(|e| e["message"].as_str());
                        (
                            Outcome::Failed {
                                kind: kind.unwrap_or("api").to_owned(),
                            },
                            message.map(str::to_owned),
                            ", Slack refused it",
                        )
                    }
                    Status::Sent => (Outcome::Ok, None, ", sent"),
                    _ => (Outcome::Ok, None, ""),
                };
                (
                    proposal.tool.clone(),
                    arguments,
                    outcome,
                    format!("user {verb} Slack proposal {id}{done}"),
                    error,
                )
            }
            Err(err) => (
                "slack_proposal".to_owned(),
                serde_json::Map::new(),
                Outcome::Failed {
                    kind: err.kind.clone(),
                },
                format!("user {verb} Slack proposal {id}"),
                Some(err.message.clone()),
            ),
        };
        self.shared.audit.record(&AuditEvent {
            tool,
            capability: Capability::SlackSends,
            arguments,
            outcome,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            client: Some("PR Nexus".to_owned()),
            summary,
            error,
        });
        decided
    }

    fn set_status(&self, status: McpStatus) {
        *self.status.write().expect("mcp status") = status;
    }
}

/// Settings, server status and the masked token.
#[tauri::command]
pub async fn mcp_settings(mcp: State<'_, Arc<McpControl>>) -> Result<McpView, AppError> {
    Ok(mcp.view())
}

/// Saves the Agents (MCP) settings and starts or stops the server.
#[tauri::command]
pub async fn save_mcp_settings(
    mcp: State<'_, Arc<McpControl>>,
    settings: McpSettings,
) -> Result<McpView, AppError> {
    let mcp = Arc::clone(&mcp);
    mcp.update(settings).await
}

/// The full token, for the copy button.
#[tauri::command]
pub async fn mcp_token(mcp: State<'_, Arc<McpControl>>) -> Result<String, AppError> {
    let mcp = Arc::clone(&mcp);
    mcp.token().await
}

/// Replaces the token. The UI confirms first.
#[tauri::command]
pub async fn regenerate_mcp_token(mcp: State<'_, Arc<McpControl>>) -> Result<McpView, AppError> {
    let mcp = Arc::clone(&mcp);
    mcp.regenerate().await
}

/// One Activity row as the Agents tab shows it.
#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub at: String,
    pub client: String,
    pub tool: String,
    pub capability: String,
    pub outcome: String,
    pub summary: String,
    pub error: Option<String>,
}

/// Rows the Activity list loads at once.
const ACTIVITY_PAGE: usize = 200;

/// The newest Activity rows, optionally one outcome (`allowed`, `denied`,
/// `error`) only.
#[tauri::command]
pub async fn mcp_activity(
    app: State<'_, Arc<AppState>>,
    outcome: Option<String>,
) -> Result<Vec<AuditRow>, AppError> {
    let state = Arc::clone(&app);
    blocking::run(move || {
        let outcome =
            match outcome.as_deref() {
                None | Some("") | Some("all") => None,
                Some(text) => Some(AuditOutcome::parse(text).ok_or_else(|| {
                    AppError::new("validation", format!("Unknown outcome {text}"))
                })?),
            };
        Ok(state
            .store
            .mcp_audit(outcome, ACTIVITY_PAGE)?
            .into_iter()
            .map(|entry| AuditRow {
                id: entry.id,
                at: entry.at.to_rfc3339(),
                client: entry.client_name,
                tool: entry.tool,
                capability: entry.capability,
                outcome: entry.outcome.as_str().to_owned(),
                summary: entry.summary,
                error: entry.error,
            })
            .collect())
    })
    .await
}

/// Empties the Activity log. The UI confirms first.
#[tauri::command]
pub async fn clear_mcp_activity(app: State<'_, Arc<AppState>>) -> Result<usize, AppError> {
    let state = Arc::clone(&app);
    blocking::run(move || Ok(state.store.clear_mcp_audit()?)).await
}

/// Slack sends agents asked for, newest first.
#[tauri::command]
pub async fn mcp_slack_proposals(app: State<'_, Arc<AppState>>) -> Result<Vec<Proposal>, AppError> {
    let state = Arc::clone(&app);
    blocking::run(move || Ok(sends::list(&state))).await
}

/// Approve (send now) or reject one Slack proposal.
#[tauri::command]
pub async fn decide_mcp_slack_proposal(
    mcp: State<'_, Arc<McpControl>>,
    id: u64,
    approve: bool,
) -> Result<Proposal, AppError> {
    let mcp = Arc::clone(&mcp);
    mcp.decide(id, approve).await
}

fn client_env() -> Result<clients::Env, AppError> {
    clients::Env::current()
        .ok_or_else(|| AppError::new("validation", "Could not find your home folder"))
}

/// The running server's URL and the token, or why there is nothing to add yet.
async fn endpoint(mcp: &McpControl) -> Result<(String, String), AppError> {
    let url = mcp
        .status()
        .url
        .ok_or_else(|| AppError::new("validation", "Turn the server on first"))?;
    Ok((url, mcp.token().await?))
}

/// Per client: config file path and whether pr-nexus is in it and current.
#[tauri::command]
pub async fn mcp_client_states(
    mcp: State<'_, Arc<McpControl>>,
) -> Result<Vec<clients::ClientState>, AppError> {
    let mcp = Arc::clone(&mcp);
    let env = client_env()?;
    let (url, token) = endpoint(&mcp).await?;
    blocking::run(move || {
        Ok(clients::Client::ALL
            .iter()
            .map(|client| clients::state(*client, &env, &url, &token))
            .collect())
    })
    .await
}

/// Adds or updates the pr-nexus entry in one client's config. The token
/// never leaves Rust.
#[tauri::command]
pub async fn mcp_add_to_client(
    mcp: State<'_, Arc<McpControl>>,
    client: clients::Client,
) -> Result<clients::ClientState, AppError> {
    let mcp = Arc::clone(&mcp);
    let env = client_env()?;
    let (url, token) = endpoint(&mcp).await?;
    blocking::run(move || {
        let result = if client == clients::Client::ClaudeCode {
            clients::install_claude_code(&env, &url, &token)
        } else {
            clients::install_file(client, &clients::config_path(client, &env), &url, &token)
        };
        result.map_err(|message| AppError::new("validation", message))?;
        Ok(clients::state(client, &env, &url, &token))
    })
    .await
}

/// Opens the client's config file, or reveals its folder when the file is missing.
#[tauri::command]
pub async fn mcp_open_client_config(
    app: tauri::AppHandle,
    client: clients::Client,
) -> Result<(), AppError> {
    use tauri_plugin_opener::OpenerExt;
    let env = client_env()?;
    let path = clients::config_path(client, &env);
    let opener = app.opener();
    let result = if path.exists() {
        opener.open_path(path.display().to_string(), None::<&str>)
    } else if let Some(dir) = path.parent().filter(|dir| dir.exists()) {
        opener.open_path(dir.display().to_string(), None::<&str>)
    } else {
        return Err(AppError::new(
            "validation",
            format!("{} does not exist yet", path.display()),
        ));
    };
    result.map_err(|err| AppError::new("validation", err.to_string()))
}
