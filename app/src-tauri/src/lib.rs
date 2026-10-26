use pr_nexus_core::domain::{ChatKind, VcsKind};
use pr_nexus_core::providers::{ProviderRegistry, SetupStep};
use pr_nexus_providers::{build_chat, build_vcs};

/// Reference the workspace crates so the shell cannot build without them.
fn link_workspace() {
    let steps = ProviderRegistry::v1().setup_steps(VcsKind::GitHub, ChatKind::Slack);
    debug_assert_eq!(steps.last().copied(), Some(SetupStep::Done));
    // Factories are stubs until their milestones. Touch them so the providers
    // crate stays linked into the binary.
    let _ = build_vcs(VcsKind::GitHub);
    let _ = build_chat(ChatKind::Slack);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    link_workspace();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("error while running PR-Nexus");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_crates_link() {
        link_workspace();
        assert!(build_vcs(VcsKind::GitHub).is_err());
        assert!(build_chat(ChatKind::Slack).is_err());
    }
}
