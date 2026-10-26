//! Domain types, errors, the provider registry, the SQLite store, and review text.
//!
//! Network clients stay in `pr-nexus-providers`. The sync engine is a later milestone.

pub mod domain;
pub mod error;
pub mod mapping;
pub mod providers;
pub mod review;
pub mod store;
