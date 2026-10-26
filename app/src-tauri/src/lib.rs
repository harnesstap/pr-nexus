mod commands;
mod demo;
mod error;
mod slack_link;
mod state;

use std::sync::Arc;

use tauri::Manager;

/// Desktop shell. The store and the GitHub client block on OS threads.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Remembers size, position and maximized state; VISIBLE stays with the config.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(window_state_flags())
                .build(),
        )
        .setup(|app| {
            let path = state::database_path(app);
            let shared = Arc::new(state::AppState::open(path)?);
            state::spawn_scheduler(Arc::clone(&shared));
            app.manage(shared);
            if let Some(window) = app.get_webview_window("main") {
                fit_to_monitor(&window);
                if !has_saved_window_state(app.handle()) {
                    // First launch: open maximized (§7.1). Later launches restore the last geometry.
                    let _ = window.maximize();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(focused) = event {
                let state = window.state::<Arc<state::AppState>>();
                commands::set_focus(state, *focused);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::save_providers,
            commands::save_onboarding,
            commands::connect_github,
            commands::start_device_flow,
            commands::poll_device_flow,
            commands::save_orgs,
            commands::save_slack_settings,
            commands::slack_manifest_url,
            commands::import_mapping,
            commands::finish_onboarding,
            commands::sync_now,
            commands::sync_status,
            commands::set_focus,
            commands::list_rows,
            commands::list_projects,
            commands::create_project,
            commands::rename_project,
            commands::archive_project,
            commands::delete_project,
            commands::set_project_color,
            commands::list_project_rules,
            commands::save_project_rule,
            commands::delete_project_rule,
            commands::preview_rule,
            commands::list_project_links,
            commands::link_prs,
            commands::unlink_pr,
            commands::restore_pr,
            commands::load_view_state,
            commands::save_view_state,
            commands::open_external,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PR-Nexus");
}

fn window_state_flags() -> tauri_plugin_window_state::StateFlags {
    use tauri_plugin_window_state::StateFlags;
    StateFlags::all() - StateFlags::VISIBLE
}

fn has_saved_window_state(app: &tauri::AppHandle) -> bool {
    app.path()
        .app_config_dir()
        .map(|dir| {
            dir.join(tauri_plugin_window_state::DEFAULT_FILENAME)
                .exists()
        })
        .unwrap_or(false)
}

/// Keeps the default 1280×800 window inside small screens (90% of the monitor).
fn fit_to_monitor(window: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let screen = monitor.size().to_logical::<f64>(scale);
    let (width, height) = fitted_size(screen.width, screen.height);
    if width < DEFAULT_SIZE.0 || height < DEFAULT_SIZE.1 {
        let _ = window.set_size(tauri::LogicalSize::new(width, height));
        let _ = window.center();
    }
}

const DEFAULT_SIZE: (f64, f64) = (1280.0, 800.0);
const MIN_SIZE: (f64, f64) = (960.0, 600.0);

fn fitted_size(screen_width: f64, screen_height: f64) -> (f64, f64) {
    let width = DEFAULT_SIZE
        .0
        .min(screen_width * 0.9)
        .max(MIN_SIZE.0.min(screen_width));
    let height = DEFAULT_SIZE
        .1
        .min(screen_height * 0.9)
        .max(MIN_SIZE.1.min(screen_height));
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_defaults_to_1280_by_800_and_shrinks_on_small_screens() {
        assert_eq!(fitted_size(1920.0, 1200.0), (1280.0, 800.0));
        assert_eq!(fitted_size(1280.0, 720.0), (1152.0, 648.0));
        assert_eq!(fitted_size(800.0, 500.0), (800.0, 500.0));
    }

    #[test]
    fn slack_manifest_link_is_built_from_the_asset() {
        let url = slack_link::create_slack_app_url("http://127.0.0.1:9/callback").unwrap();
        assert!(url.contains("manifest_json="));
    }

    #[test]
    fn github_factory_builds_without_a_network_call() {
        let built = pr_nexus_providers::build_vcs(
            pr_nexus_core::domain::VcsKind::GitHub,
            pr_nexus_core::domain::AccountId(1),
            "token".into(),
        );
        assert!(built.is_ok());
        assert!(pr_nexus_providers::build_chat(pr_nexus_core::domain::ChatKind::Slack).is_err());
    }
}
