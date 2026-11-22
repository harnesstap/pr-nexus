//! A check that did not pass, cached from the status rollup so the list can
//! name it without another GitHub read.

use serde::{Deserialize, Serialize};

/// Why a check counts as failing. The chip uses a short English verb for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailingConclusion {
    Failure,
    TimedOut,
    Cancelled,
    ActionRequired,
}

/// One check the rollup reported as not passing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailingCheck {
    pub name: String,
    pub conclusion: FailingConclusion,
    /// Details or log page. `None` when GitHub did not send one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}
