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
        .setup(|app| {
            let path = state::database_path(app);
            let shared = Arc::new(state::AppState::open(path)?);
            state::spawn_scheduler(Arc::clone(&shared));
            app.manage(shared);
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
            commands::open_external,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PR-Nexus");
}

#[cfg(test)]
mod tests {
    use super::*;

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
