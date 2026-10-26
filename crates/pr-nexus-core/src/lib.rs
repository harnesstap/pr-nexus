//! Domain, store, and provider traits for PR-Nexus.
//!
//! This crate has no network dependencies. HTTP implementations live in
//! `pr-nexus-providers`. The modules below are the layout from `docs/SPEC.md`
//! §3. Behavior for later milestones is intentionally not implemented yet.

pub mod domain;
pub mod error;
pub mod providers;
pub mod store;

/// Crate name. The desktop shell references this so the workspace link is real.
pub const CRATE_NAME: &str = "pr-nexus-core";

pub use error::{CoreError, ProviderError};
