//! Starts and stops the server from Settings, without an app restart, and
//! the commands the Settings section calls.

use std::sync::{Arc, RwLock};

use serde::Serialize;
use tauri::State;
use tokio::sync::Mutex;

use super::audit::{AuditSink, LogAudit};
use super::server::{self, Running, Shared};
use super::{McpSettings, config, token};
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
    pub fn new(state: Arc<AppState>) -> Result<Self, AppError> {
        Self::with_audit(state, Arc::new(LogAudit))
    }

    pub fn with_audit(state: Arc<AppState>, audit: Arc<dyn AuditSink>) -> Result<Self, AppError> {
        let settings = config::load(&state.store)?;
        Ok(Self {
            shared: Arc::new(Shared {
                state,
                settings: RwLock::new(settings),
                token: RwLock::new(String::new()),
                audit,
            }),
            running: Mutex::new(None),
            status: RwLock::new(McpStatus::default()),
        })
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
