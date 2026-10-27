//! Plain-language sentences for the detail panel's "Details" expander (§7.2).
//!
//! The ownership report keeps machine-shaped reasons ("check ci is fail",
//! "codeowners_required=unknown (...)") for snapshots and logs. The panel
//! shows these sentences instead.

use pr_nexus_core::domain::{TeamRef, Tri};
use pr_nexus_core::ownership::OwnershipReport;
use pr_nexus_core::providers::ClassicProtection;
use pr_nexus_core::review::readiness::{
    BLOCKED_OTHER, BLOCKED_WAITING_FOR_REVIEWS, CODE_OWNER_REVIEW_UNKNOWN, FILES_TRUNCATED,
    MERGEABILITY_UNKNOWN, NO_CHECKS,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Good,
    Warn,
    Bad,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Explanation {
    pub tone: Tone,
    pub text: String,
}

fn line(tone: Tone, text: impl Into<String>) -> Explanation {
    Explanation {
        tone,
        text: text.into(),
    }
}

/// Every sentence for the report, blockers first.
pub fn explanations(report: &OwnershipReport, blocked: bool, notes: &[String]) -> Vec<Explanation> {
    let mut out: Vec<Explanation> = notes.iter().map(|note| reason(note, blocked)).collect();
    if out.is_empty() && blocked {
        out.push(line(Tone::Bad, "Something is blocking the merge."));
    }
    if !blocked && notes.is_empty() {
        out.push(line(Tone::Good, "Nothing is blocking the merge."));
    }
    out.extend(code_owners(report));
    if report.rules_error.is_some() {
        out.push(line(
            Tone::Info,
            "Branch rules could not be read, so some requirements may be missing here.",
        ));
    }
    out
}

/// One readiness reason as a sentence.
pub fn reason(text: &str, blocked: bool) -> Explanation {
    let fallback = if blocked { Tone::Bad } else { Tone::Info };
    match text {
        BLOCKED_WAITING_FOR_REVIEWS => line(Tone::Warn, "GitHub is waiting for required reviews."),
        BLOCKED_OTHER => line(
            Tone::Bad,
            "GitHub blocks the merge for a reason other than reviews, usually a branch rule.",
        ),
        CODE_OWNER_REVIEW_UNKNOWN => line(
            Tone::Info,
            "GitHub blocks the merge but gives no review decision, so code owner review may still be missing.",
        ),
        MERGEABILITY_UNKNOWN => line(
            Tone::Info,
            "GitHub is still working out whether this can merge.",
        ),
        NO_CHECKS => line(
            Tone::Warn,
            "No CI checks have reported on the latest commit.",
        ),
        FILES_TRUNCATED => line(
            Tone::Info,
            "This pull request changes so many files that the owner list may be incomplete.",
        ),
        "PR is a draft" => line(Tone::Warn, "This pull request is still a draft."),
        "PR is closed" => line(Tone::Info, "This pull request is closed."),
        "PR is merged" => line(Tone::Good, "This pull request is already merged."),
        "PR has merge conflicts" => line(Tone::Bad, "The branch has conflicts with its base."),
        "PR branch is behind the base branch" => line(
            Tone::Warn,
            "The branch is behind its base and needs an update before merging.",
        ),
        "complete review evidence is unavailable" | "complete PR check evidence is unavailable" => {
            line(
                Tone::Info,
                "Not every review or check could be read, so this summary may be incomplete.",
            )
        }
        other => {
            if let Some((name, state)) = other
                .strip_prefix("check ")
                .and_then(|rest| rest.rsplit_once(" is "))
            {
                let (tone, verb) = match state {
                    "fail" => (Tone::Bad, "failed"),
                    "pending" => (Tone::Warn, "is still running"),
                    "cancel" => (Tone::Bad, "was cancelled"),
                    "timeout" => (Tone::Bad, "timed out"),
                    "action_required" => (Tone::Warn, "needs someone to act on it"),
                    _ => (fallback, "has an unexpected status"),
                };
                return line(tone, format!("Check \u{201c}{name}\u{201d} {verb}."));
            }
            if let Some(login) = other
                .strip_prefix("human reviewer ")
                .and_then(|rest| rest.strip_suffix(" requested changes"))
            {
                return line(Tone::Bad, format!("@{login} requested changes."));
            }
            line(fallback, sentence(other))
        }
    }
}

fn code_owners(report: &OwnershipReport) -> Vec<Explanation> {
    let decision = &report.decision;
    let mut out = Vec::new();
    if report.codeowners_error.is_some() {
        out.push(line(Tone::Warn, "The CODEOWNERS file could not be read."));
        return out;
    }
    if report.codeowners_path.is_none() {
        out.push(line(Tone::Info, "This repository has no CODEOWNERS file."));
        return out;
    }
    match decision.codeowners_required {
        Tri::True => {
            let missing: Vec<&TeamRef> = decision
                .mandatory_teams
                .iter()
                .filter(|team| !decision.approved_teams.contains(team))
                .collect();
            if !decision.approved_teams.is_empty() {
                out.push(line(
                    Tone::Good,
                    format!(
                        "Approved by code owners: {}.",
                        join(decision.approved_teams.iter())
                    ),
                ));
            }
            if missing.is_empty() {
                out.push(line(
                    Tone::Good,
                    "Every required code owner team has approved.",
                ));
            } else {
                out.push(line(
                    Tone::Warn,
                    format!(
                        "Still needs a code owner review from {}.",
                        join(missing.into_iter())
                    ),
                ));
            }
        }
        Tri::False => out.push(line(
            Tone::Good,
            "Code owner review is not required on this branch.",
        )),
        Tri::Unknown => {
            let why = match &report.protection {
                ClassicProtection::Unknown { .. } => {
                    " because branch protection is only visible to repository admins"
                }
                _ => "",
            };
            out.push(line(
                Tone::Info,
                format!("Can't tell whether code owner review is required{why}."),
            ));
        }
    }
    if !decision.review_attribution_unknown.is_empty() {
        out.push(line(
            Tone::Info,
            format!(
                "Couldn't confirm which approvals count for {}.",
                join(
                    decision
                        .review_attribution_unknown
                        .iter()
                        .map(|gap| &gap.team)
                )
            ),
        ));
    }
    out
}

fn join<'a>(teams: impl Iterator<Item = &'a TeamRef>) -> String {
    teams
        .map(|team| format!("@{team}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Capitalized, with a full stop.
fn sentence(text: &str) -> String {
    let text = text.trim();
    let mut chars = text.chars();
    let mut out = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
        None => return String::new(),
    };
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_reasons_read_as_sentences() {
        assert_eq!(
            reason("check build / test is fail", true),
            line(Tone::Bad, "Check \u{201c}build / test\u{201d} failed.")
        );
        assert_eq!(
            reason("check lint is pending", true),
            line(Tone::Warn, "Check \u{201c}lint\u{201d} is still running.")
        );
        assert_eq!(
            reason("human reviewer kim requested changes", true).text,
            "@kim requested changes."
        );
        assert_eq!(
            reason(BLOCKED_WAITING_FOR_REVIEWS, true).text,
            "GitHub is waiting for required reviews."
        );
        assert_eq!(
            reason("some new reason", false),
            line(Tone::Info, "Some new reason.")
        );
    }

    #[test]
    fn no_sentence_leaks_an_internal_key() {
        for raw in [
            BLOCKED_OTHER,
            CODE_OWNER_REVIEW_UNKNOWN,
            MERGEABILITY_UNKNOWN,
            NO_CHECKS,
            FILES_TRUNCATED,
            "PR is a draft",
            "PR has merge conflicts",
            "PR branch is behind the base branch",
            "check ci is action_required",
        ] {
            let text = reason(raw, true).text;
            assert!(!text.contains('='), "{text}");
            assert!(!text.contains('_'), "{text}");
            assert!(!text.contains("BLOCKED"), "{text}");
        }
    }
}
