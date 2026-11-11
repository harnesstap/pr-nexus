//! Domain types, errors, the provider registry, the SQLite store, and review text.
//!
//! Network clients stay in `pr-nexus-providers`. The sync engine is a later milestone.

pub mod attention;
pub mod bots;
pub mod branch;
pub mod close;
pub mod domain;
pub mod error;
pub mod github_query;
pub mod jira;
pub mod linking;
pub mod live;
pub mod mapping;
pub mod merge_queue;
pub mod notify;
pub mod onboarding;
pub mod overview;
pub mod ownership;
pub mod profiles;
pub mod providers;
pub mod redact;
pub mod review;
pub mod review_filter;
pub mod search;
pub mod settings;
pub mod sla;
pub mod stack;
pub mod stats;
pub mod store;
pub mod sync;
pub mod update;
