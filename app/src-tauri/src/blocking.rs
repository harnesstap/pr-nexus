//! Commands never run on the main thread.
//!
//! Tauri v2 runs a plain `fn` command on the main thread, so a network call,
//! a keychain read or a SQLite write there freezes the whole window ("not
//! responding"). Every `#[tauri::command]` in this crate is therefore an
//! `async fn`, and any work that blocks (HTTP, Slack MCP, keychain, SQLite,
//! files) runs through [`run`], which moves it to Tauri's blocking pool.
//! `tests::every_command_is_async` enforces the first half of the rule.

use std::ops::Deref;
use std::sync::Arc;

use tauri::State;

use crate::error::AppError;
use crate::state::AppState;

/// An owned handle to the app state that a blocking closure can keep.
/// It mirrors `State::inner()` so command bodies read the same either way.
pub struct Handle(Arc<AppState>);

impl Handle {
    pub fn new(state: &State<'_, Arc<AppState>>) -> Self {
        Self(state.inner().clone())
    }

    pub fn inner(&self) -> &Arc<AppState> {
        &self.0
    }
}

impl Deref for Handle {
    type Target = AppState;

    fn deref(&self) -> &AppState {
        &self.0
    }
}

/// Runs `work` on Tauri's blocking thread pool and waits for it without
/// holding up the main thread or an async worker.
pub async fn run<T, F>(work: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|err| AppError::new("internal", format!("background task failed: {err}")))?
}

#[cfg(test)]
mod tests {
    /// Every command source file. A new file with commands must be added here.
    const SOURCES: &[(&str, &str)] = &[
        ("bulk.rs", include_str!("bulk.rs")),
        ("commands.rs", include_str!("commands.rs")),
        ("context.rs", include_str!("context.rs")),
        ("mapping_file.rs", include_str!("mapping_file.rs")),
        ("mcp/control.rs", include_str!("mcp/control.rs")),
        ("preferences.rs", include_str!("preferences.rs")),
        ("project_info.rs", include_str!("project_info.rs")),
        ("slack.rs", include_str!("slack.rs")),
        ("templates.rs", include_str!("templates.rs")),
    ];

    /// `(line, signature)` of every command in `source`, and whether it is async.
    fn commands(source: &str) -> Vec<(usize, String, bool)> {
        let mut found = Vec::new();
        let mut lines = source.lines().enumerate();
        while let Some((_, line)) = lines.next() {
            if line.trim() != "#[tauri::command]" {
                continue;
            }
            if let Some((number, next)) = lines.find(|(_, line)| !line.trim().starts_with("#[")) {
                let signature = next.trim().to_owned();
                let is_async = signature.starts_with("pub async fn ");
                found.push((number + 1, signature, is_async));
            }
        }
        found
    }

    #[test]
    fn the_check_spots_a_sync_command() {
        let source = "#[tauri::command]\npub fn ping() {}\n\n#[tauri::command]\n#[allow(unused)]\npub async fn pong() -> Result<(), ()> { Ok(()) }\n";
        let found = commands(source);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].0, found[0].2), (2, false));
        assert!(found[1].2);
    }

    #[test]
    fn every_command_is_async() {
        let mut offenders = Vec::new();
        let mut total = 0;
        for (file, source) in SOURCES {
            for (line, signature, is_async) in commands(source) {
                total += 1;
                if !is_async {
                    offenders.push(format!("{file}:{line}: {signature}"));
                }
            }
        }
        assert!(
            total > 40,
            "found only {total} commands; is SOURCES complete?"
        );
        assert!(
            offenders.is_empty(),
            "Tauri runs sync commands on the main thread; make these `async fn` and move blocking work into blocking::run:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn every_registered_command_file_is_checked() {
        let lib = include_str!("lib.rs");
        for module in [
            "bulk",
            "commands",
            "context",
            "mapping_file",
            "mcp::control",
            "preferences",
            "slack",
        ] {
            assert!(
                lib.contains(&format!("{module}::")),
                "{module} is no longer registered"
            );
        }
        let handler = lib
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("generate_handler! list");
        for entry in handler
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let path = entry.rsplit_once("::").map_or(entry, |(module, _)| module);
            let file = format!("{}.rs", path.replace("::", "/"));
            assert!(
                SOURCES.iter().any(|(source, _)| *source == file),
                "{entry} lives in a file the async check does not read"
            );
        }
    }
}
