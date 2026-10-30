//! Settings → Slack → Message templates (§7.6).

use std::sync::Arc;

use pr_nexus_core::review::message_template::{self, TemplateKind};
use pr_nexus_core::settings;
use serde::Serialize;
use tauri::State;

use crate::blocking;
use crate::error::AppError;
use crate::state::AppState;

type Shared<'a> = State<'a, Arc<AppState>>;

#[derive(Debug, Serialize)]
pub struct TemplateEntry {
    pub kind: TemplateKind,
    pub text: String,
    pub default_text: String,
}

#[tauri::command]
pub async fn load_message_templates(app: Shared<'_>) -> Result<Vec<TemplateEntry>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let templates = settings::message_templates(&app.inner().store)?;
        Ok(TemplateKind::ALL
            .into_iter()
            .map(|kind| TemplateEntry {
                kind,
                text: templates.get(kind).to_owned(),
                default_text: kind.default_text().to_owned(),
            })
            .collect())
    })
    .await
}

/// Validates and saves one template; `None` resets it to the default.
#[tauri::command]
pub async fn save_message_template(
    app: Shared<'_>,
    kind: TemplateKind,
    text: Option<String>,
) -> Result<(), AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        settings::set_message_template(&app.inner().store, kind, text.as_deref())?;
        Ok(())
    })
    .await
}

/// The message a candidate template produces for a sample PR, rendered by
/// the same code that builds real requests. Invalid templates are an error.
#[tauri::command]
pub async fn preview_message_template(
    app: Shared<'_>,
    kind: TemplateKind,
    text: String,
) -> Result<String, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        let store = &app.inner().store;
        let templates = settings::message_templates(store)?;
        let requester = settings::review_requester(store)?;
        Ok(message_template::preview(
            &templates,
            kind,
            &text,
            requester.as_deref(),
        )?)
    })
    .await
}
