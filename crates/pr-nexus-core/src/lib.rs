//! Domain types, errors, the provider registry, and the SQLite store.
//!
//! Network clients stay in `pr-nexus-providers`. The sync engine and the
//! review-request planner are later milestones.

pub mod domain;
pub mod error;
pub mod providers;
pub mod store;
