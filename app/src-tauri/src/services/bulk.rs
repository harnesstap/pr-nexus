//! Bulk request and bump from the bottom bar (§7.9).
//!
//! [`preview`] posts nothing; [`send`] posts through a [`ChatSource`].
//!
//! The preview and the send run the same gate and planner. Send plans again
//! and refuses any channel card whose content moved since the preview.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use pr_nexus_core::domain::{
    Involvement, PrId, PrState, PullRequest, Readiness, RequestStatus, SendOutcome,
};
use pr_nexus_core::error::CoreError;
use pr_nexus_core::ownership;
use pr_nexus_core::review::batch::{
    self, BatchInput, BatchOptions, BatchPlan, MessagePlanKind, Pace, Route, Skip,
};
use pr_nexus_core::review::lifecycle::Target;
use pr_nexus_core::review::route::{self, Workspace};
use pr_nexus_core::settings;
use serde::{Deserialize, Serialize};

use crate::commands::{SlackIndicator, indicator};
use crate::error::AppError;
use crate::services::prs::pr_detail;
use crate::services::review::ChatSource;
use crate::slack::{
    PrWarning, ReviewTarget, any_usable, load_pr, mark_rejected, signed_in_account, targets_for,
    validate_target,
};
use crate::state::AppState;

#[derive(Debug, Deserialize, Clone)]
pub struct ChannelKey {
    pub workspace: String,
    pub channel_id: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BulkRequest {
    pub pr_ids: Vec<i64>,
    /// Send everything to this one channel instead of the routed ones.
    pub target: Option<ReviewTarget>,
    /// Channels switched to "one top-level request for all" (D5).
    #[serde(default)]
    pub single_top_level: Vec<ChannelKey>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BulkPr {
    pub pr_id: i64,
    pub number: u64,
    pub title: String,
    pub repo: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BulkMessage {
    /// `request`, `bump`, or `remind`.
    pub action: String,
    pub prs: Vec<BulkPr>,
    pub text: String,
    /// Thread being bumped, when known.
    pub permalink: Option<String>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BulkCard {
    pub target: ReviewTarget,
    pub messages: Vec<BulkMessage>,
    pub warnings: Vec<String>,
    pub signed_in: bool,
    pub single_top_level: bool,
    pub can_switch: bool,
    pub fingerprint: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BulkSkip {
    pub pr: BulkPr,
    pub detail: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BulkPreview {
    pub cards: Vec<BulkCard>,
    pub skipped: Vec<BulkSkip>,
    /// One per selected PR that is still a draft; drafts are sent anyway.
    pub pr_warnings: Vec<PrWarning>,
}

#[derive(Serialize)]
pub struct BulkOutcome {
    pub pr: BulkPr,
    pub target: Option<ReviewTarget>,
    /// `sent`, `skipped`, or `failed`.
    pub outcome: String,
    pub detail: String,
    pub permalink: Option<String>,
}

#[derive(Serialize)]
pub struct PrIndicator {
    pub pr_id: i64,
    pub indicator: Option<SlackIndicator>,
}

#[derive(Serialize)]
pub struct BulkResult {
    pub batch_id: String,
    pub results: Vec<BulkOutcome>,
    /// Fresh Slack indicator for every PR that was sent.
    pub indicators: Vec<PrIndicator>,
}

pub fn send(
    state: &AppState,
    chats: &dyn ChatSource,
    request: &BulkRequest,
    fingerprints: &[String],
) -> Result<BulkResult, AppError> {
    let (mut plan, prs) = plan_request(state, request)?;
    let stale: BTreeSet<(String, String)> = plan
        .cards
        .iter()
        .filter(|card| !fingerprints.contains(&card.fingerprint()))
        .map(|card| {
            (
                card.target.workspace.clone(),
                card.target.channel_id.clone(),
            )
        })
        .collect();
    let workspaces = route::workspaces(&state.store)?;
    let routed = route_cards(state, chats, &workspaces, &mut plan, fingerprints);
    let batch_id = format!("bulk-{}", Utc::now().format("%Y%m%dT%H%M%S%.3fZ"));
    let mut chat = |workspace: &str| {
        chats
            .chat(state, workspace)
            .map_err(|err| CoreError::Validation(err.message))
    };
    let results = batch::send_batch(&state.store, &mut chat, &plan, &stale, &batch_id, &RealPace)?;
    for result in &results {
        if result.detail.starts_with("Slack sign-in expired")
            && let Some(target) = &result.target
            && let Ok(Some(account)) = signed_in_account(state, target.route_key())
        {
            mark_rejected(state, account.id.0);
        }
    }
    for (from, ws) in &routed {
        let sent_here = results.iter().any(|result| {
            result.outcome == SendOutcome::Sent
                && result
                    .target
                    .as_ref()
                    .is_some_and(|target| target.channel_id == from.channel_id)
        });
        if sent_here {
            state.store.pin_mapping_team(
                from.channel_id.clone(),
                from.team_id.clone(),
                from.workspace.clone(),
                ws.team_id.clone(),
                ws.domain().to_owned(),
            )?;
        }
    }
    let mut sent: Vec<PrId> = Vec::new();
    for result in &results {
        if result.outcome == SendOutcome::Sent && !sent.contains(&result.pr) {
            sent.push(result.pr);
        }
    }
    let mut indicators = Vec::new();
    for pr in sent {
        let active = state
            .store
            .list_review_requests(pr)?
            .into_iter()
            .filter(|request| request.status == RequestStatus::Active)
            .max_by_key(|request| request.last_bumped_at.unwrap_or(request.requested_at))
            .map(|request| indicator(request, &workspaces));
        indicators.push(PrIndicator {
            pr_id: pr.0,
            indicator: active,
        });
    }
    Ok(BulkResult {
        batch_id,
        results: results
            .into_iter()
            .map(|result| BulkOutcome {
                pr: pr_dto_by_id(&prs, result.pr, &result.title),
                target: result.target.map(|target| target_dto(&workspaces, target)),
                outcome: result.outcome.as_str().to_owned(),
                detail: result.detail,
                permalink: result.permalink,
            })
            .collect(),
        indicators,
    })
}

struct RealPace;

impl Pace for RealPace {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
    fn sleep(&self, duration: Duration) {
        if let Ok(duration) = duration.to_std() {
            std::thread::sleep(duration);
        }
    }
}

/// Gate, route, and plan. Returns the PRs that were looked up for the DTOs.
/// Gate, plan and route without posting: the bottom bar's preview.
pub fn preview(state: &AppState, request: &BulkRequest) -> Result<BulkPreview, AppError> {
    let (plan, prs) = plan_request(state, request)?;
    preview_dto(state, &plan, &prs)
}

fn plan_request(
    state: &AppState,
    request: &BulkRequest,
) -> Result<(BatchPlan, Vec<PullRequest>), AppError> {
    if request.pr_ids.is_empty() {
        return Err(AppError::new(
            "validation",
            "Select at least one pull request",
        ));
    }
    let override_target = request.target.clone().map(validate_target).transpose()?;
    let flag = settings::blocked_review_required_is_ready(&state.store)?;
    let mut prs = Vec::new();
    let mut inputs = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = BTreeSet::new();
    for &id in &request.pr_ids {
        if !seen.insert(id) {
            continue;
        }
        let pr = load_pr(&state.store, id)?;
        prs.push(pr.clone());
        let skip = |detail: String| Skip {
            pr: pr.id,
            title: pr.title.clone(),
            channel: None,
            detail,
        };
        if pr.state != PrState::Open {
            skipped.push(skip("Not open".into()));
            continue;
        }
        if !pr.involvement.contains(Involvement::AUTHOR) {
            skipped.push(skip("Not authored by you".into()));
            continue;
        }
        let (blocked, teams) = if override_target.is_some() {
            (
                blocked_reasons(ownership::cached_readiness(&state.store, &pr, flag)?),
                Vec::new(),
            )
        } else {
            match pr_detail(state, pr.id.0) {
                Ok(detail) => {
                    let teams = if detail.mandatory_teams.is_empty() {
                        detail.candidate_teams
                    } else {
                        detail.mandatory_teams
                    };
                    let blocked = (detail.readiness == "blocked")
                        .then(|| without_draft(&detail.readiness_notes))
                        .flatten();
                    (blocked, teams)
                }
                Err(err) => {
                    skipped.push(skip(format!("Could not resolve owners: {}", err.message)));
                    continue;
                }
            }
        };
        if let Some(reasons) = blocked {
            skipped.push(skip(if reasons.is_empty() {
                "Blocked".into()
            } else {
                format!("Blocked: {reasons}")
            }));
            continue;
        }
        let routes: Vec<Route> = match &override_target {
            Some(target) => vec![Route {
                target: core_target(target),
                teams: teams.clone(),
            }],
            None => targets_for(&state.store, &pr, &teams)?
                .into_iter()
                .map(|choice| Route {
                    target: core_target(&choice.target),
                    teams: if choice.teams.is_empty() {
                        teams.clone()
                    } else {
                        choice.teams
                    },
                })
                .collect(),
        };
        if routes.is_empty() {
            skipped.push(skip(
                "No mapped channel for its teams. Request it alone, or send all to one channel."
                    .into(),
            ));
            continue;
        }
        inputs.push(BatchInput { pr, routes });
    }
    let options = BatchOptions {
        requester: settings::review_requester(&state.store)?,
        min_bump_interval_hours: settings::min_bump_interval_hours(&state.store)?,
        max_threads: settings::bulk_bump_max_threads(&state.store)?,
        single_top_level: request
            .single_top_level
            .iter()
            .map(|key| {
                (
                    key.workspace.trim().to_ascii_lowercase(),
                    key.channel_id.trim().to_ascii_uppercase(),
                )
            })
            .collect(),
    };
    let plan = batch::plan_batch(&state.store, &inputs, skipped, &options, Utc::now())?;
    Ok((plan, prs))
}

fn blocked_reasons(readiness: Readiness) -> Option<String> {
    match readiness {
        Readiness::Blocked { reasons } => without_draft(&reasons),
        Readiness::Ready { .. } | Readiness::Unknown { .. } => None,
    }
}

/// A draft only warns (see [`crate::slack::pr_warnings`]); any other reason
/// still blocks. `None` when being a draft was the only reason.
fn without_draft(reasons: &[String]) -> Option<String> {
    let rest: Vec<&str> = reasons
        .iter()
        .map(String::as_str)
        .filter(|reason| *reason != DRAFT_REASON)
        .collect();
    if rest.is_empty() && !reasons.is_empty() {
        None
    } else {
        Some(rest.join("; "))
    }
}

/// The readiness reason for a draft (`review::readiness`).
const DRAFT_REASON: &str = "PR is a draft";

fn preview_dto(
    state: &AppState,
    plan: &BatchPlan,
    prs: &[PullRequest],
) -> Result<BulkPreview, AppError> {
    let workspaces = route::workspaces(&state.store)?;
    let mut cards = Vec::new();
    for card in &plan.cards {
        let posts_request = card
            .messages
            .iter()
            .any(|message| matches!(message.kind, MessagePlanKind::Request));
        let signed_in = state.demo
            || if posts_request {
                any_usable(state, &workspaces, &card.target)?
            } else {
                signed_in_account(state, card.target.route_key())
                    .ok()
                    .flatten()
                    .is_some()
            };
        cards.push(BulkCard {
            target: target_dto(&workspaces, card.target.clone()),
            messages: card
                .messages
                .iter()
                .map(|message| BulkMessage {
                    action: match message.kind {
                        MessagePlanKind::Request => "request",
                        MessagePlanKind::Bump { .. } => "bump",
                        MessagePlanKind::Remind => "remind",
                    }
                    .into(),
                    prs: message.items.iter().map(|item| pr_dto(&item.pr)).collect(),
                    text: message.text.clone(),
                    permalink: match message.kind {
                        MessagePlanKind::Bump { .. } => message
                            .items
                            .iter()
                            .find_map(|item| item.active.as_ref()?.permalink.clone()),
                        _ => None,
                    },
                })
                .collect(),
            warnings: card.warnings.clone(),
            signed_in,
            single_top_level: card.single_top_level,
            can_switch: card.can_switch,
            fingerprint: card.fingerprint(),
        });
    }
    let pr_warnings = prs.iter().flat_map(crate::slack::pr_warnings).collect();
    Ok(BulkPreview {
        cards,
        pr_warnings,
        skipped: plan
            .skipped
            .iter()
            .map(|skip| BulkSkip {
                pr: pr_dto_by_id(prs, skip.pr, &skip.title),
                detail: skip.detail.clone(),
            })
            .collect(),
    })
}

/// Point every fresh card that posts a new request at the first enabled
/// workspace that can see its channel (read-only probe, §7.11). Returns
/// the original targets with the workspace chosen, for mapping pins.
/// A card no workspace can see is left as is and fails in the batch.
fn route_cards(
    state: &AppState,
    chats: &dyn ChatSource,
    workspaces: &[Workspace],
    plan: &mut BatchPlan,
    fingerprints: &[String],
) -> Vec<(Target, Workspace)> {
    let mut chat = |ws: &Workspace| {
        chats
            .chat(state, &ws.team_id)
            .map_err(|err| CoreError::Validation(err.message))
    };
    let mut routed = Vec::new();
    for card in &mut plan.cards {
        let posts_request = card
            .messages
            .iter()
            .any(|message| matches!(message.kind, MessagePlanKind::Request));
        if !posts_request || !fingerprints.contains(&card.fingerprint()) {
            continue;
        }
        if let Ok(ws) = route::resolve(workspaces, &card.target, &mut chat) {
            routed.push((card.target.clone(), ws.clone()));
            card.target = ws.stamp(&card.target);
        }
    }
    routed
}

fn core_target(target: &ReviewTarget) -> Target {
    crate::slack::to_core(target)
}

fn target_dto(workspaces: &[Workspace], target: Target) -> ReviewTarget {
    crate::slack::from_core(workspaces, target)
}

fn pr_dto(pr: &PullRequest) -> BulkPr {
    BulkPr {
        pr_id: pr.id.0,
        number: pr.number,
        title: pr.title.clone(),
        repo: format!("{}/{}", pr.repo.owner, pr.repo.name),
    }
}

fn pr_dto_by_id(prs: &[PullRequest], id: PrId, title: &str) -> BulkPr {
    prs.iter().find(|pr| pr.id == id).map_or_else(
        || BulkPr {
            pr_id: id.0,
            number: 0,
            title: title.to_owned(),
            repo: String::new(),
        },
        pr_dto,
    )
}

#[cfg(test)]
mod draft_tests {
    use super::{DRAFT_REASON, without_draft};

    #[test]
    fn a_draft_alone_does_not_block_but_other_reasons_still_do() {
        assert_eq!(without_draft(&[DRAFT_REASON.to_owned()]), None);
        assert_eq!(
            without_draft(&[DRAFT_REASON.to_owned(), "PR has merge conflicts".to_owned()]),
            Some("PR has merge conflicts".to_owned())
        );
        assert_eq!(without_draft(&[]), Some(String::new()));
    }
}
