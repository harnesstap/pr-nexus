//! Domain types, errors, and the provider registry.
//!
//! Network clients stay in `pr-nexus-providers`. The SQLite store, sync engine,
//! and review-request planner are later milestones; this crate only has the
//! pieces the workspace needs to compile and to describe which providers exist.

pub mod domain;
pub mod error;
pub mod providers;
