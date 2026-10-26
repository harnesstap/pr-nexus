//! Domain types (`docs/SPEC.md` §4.1).
//!
//! Pull requests, projects, review requests, and ownership decisions land with
//! the store and sync milestones. This module is the home for those types.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name_is_stable() {
        assert_eq!(crate::CRATE_NAME, "pr-nexus-core");
    }
}
