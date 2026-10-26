/// Names of the workspace libraries this shell is wired to.
///
/// Command handlers replace this probe as features land. Keeping the
/// references here makes `cargo build --workspace` compile every crate.
pub fn linked_workspace() -> [&'static str; 2] {
    [pr_nexus_core::CRATE_NAME, pr_nexus_providers::CRATE_NAME]
}

/// Starts the Tauri application. The window content is the React shell.
pub fn run() {
    let _linked = linked_workspace();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("error while running PR-Nexus");
}

#[cfg(test)]
mod tests {
    use super::linked_workspace;

    #[test]
    fn shell_links_both_workspace_crates() {
        let linked = linked_workspace();
        assert_eq!(linked, ["pr-nexus-core", "pr-nexus-providers"]);
    }

    #[test]
    fn keyring_backend_follows_the_feature_flag() {
        let backend = pr_nexus_providers::keyring_backend();
        if cfg!(feature = "keyring-mock") {
            assert_eq!(backend, "mock");
        } else {
            assert_eq!(backend, "system");
        }
    }
}
