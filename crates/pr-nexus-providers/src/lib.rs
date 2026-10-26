//! GitHub and Slack implementations of the core provider traits.
//!
//! Auth helpers and HTTP clients land with the provider milestones. This crate
//! exists so the workspace layout matches `docs/SPEC.md` §3 and the desktop
//! shell can link it.

/// Crate name. The desktop shell references this so the workspace link is real.
pub const CRATE_NAME: &str = "pr-nexus-providers";

/// Keychain backend selected for this build.
///
/// `"mock"` is reserved for Linux CI and tests. `"system"` is the real OS
/// store, wired up when the vault milestone adds the `keyring` dependency.
pub fn keyring_backend() -> &'static str {
    if cfg!(feature = "keyring-mock") {
        "mock"
    } else {
        "system"
    }
}

#[cfg(test)]
mod tests {
    use super::keyring_backend;

    #[test]
    fn links_core() {
        assert_eq!(pr_nexus_core::CRATE_NAME, "pr-nexus-core");
    }

    #[test]
    fn keyring_backend_follows_the_feature_flag() {
        let backend = keyring_backend();
        if cfg!(feature = "keyring-mock") {
            assert_eq!(backend, "mock");
        } else {
            assert_eq!(backend, "system");
        }
    }
}
