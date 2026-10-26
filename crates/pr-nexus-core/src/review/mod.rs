//! Review-request planning that does not touch the network.
//!
//! Message text, CODEOWNERS, readiness, and owner routing live here. Mapping
//! import arrives in a later milestone.

pub mod codeowners;
pub mod lifecycle;
pub mod message;
pub mod owners;
pub mod permalink;
pub mod readiness;
