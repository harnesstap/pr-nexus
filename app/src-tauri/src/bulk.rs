//! Bulk request and bump from the bottom bar (§7.9).
//!
//! The preview and the send run the same gate and planner. Send plans again
//! and refuses any channel card whose content moved since the preview.

use std::collections::BTreeSet;
use std::sync::Arc;

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
use pr_nexus_core::settings;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::{SlackIndicator, indicator, pr_detail_blocking};
use crate::error::AppError;
use crate::slack::{
    ReviewTarget, chat_for, load_pr, lock, signed_in_account, targets_for, validate_target,
};
use crate::state::AppState;

type Shared<'a> = State<'a, Arc<AppState>>;

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

#[tauri::command]
pub async fn bulk_preview(app: Shared<'_>, request: BulkRequest) -> Result<BulkPreview, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (plan, prs) = plan_request(&state, &request)?;
        preview_dto(&state, &plan, &prs)
    })
    .await
    .map_err(|err| AppError::new("internal", err.to_string()))?
}

/// Plans again and posts every card whose fingerprint still matches.
#[tauri::command]
pub async fn send_bulk(
    app: Shared<'_>,
    request: BulkRequest,
    fingerprints: Vec<String>,
) -> Result<BulkResult, AppError> {
    let state = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || send_blocking(&state, &request, &fingerprints))
        .await
        .map_err(|err| AppError::new("internal", err.to_string()))?
}

fn send_blocking(
    state: &AppState,
    request: &BulkRequest,
    fingerprints: &[String],
) -> Result<BulkResult, AppError> {
    let (plan, prs) = plan_request(state, request)?;
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
    let batch_id = format!("bulk-{}", Utc::now().format("%Y%m%dT%H%M%S%.3fZ"));
    let mut chat = |workspace: &str| {
        chat_for(state, workspace).map_err(|err| CoreError::Validation(err.message))
    };
    let results = batch::send_batch(&state.store, &mut chat, &plan, &stale, &batch_id, &RealPace)?;
    if results
        .iter()
        .any(|result| result.detail.starts_with("Slack sign-in expired"))
    {
        lock(&state.chats).clear();
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
            .map(indicator);
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
                target: result.target.map(target_dto),
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
            match pr_detail_blocking(state, pr.id) {
                Ok(detail) => {
                    let teams = if detail.mandatory_teams.is_empty() {
                        detail.candidate_teams
                    } else {
                        detail.mandatory_teams
                    };
                    let blocked =
                        (detail.readiness == "blocked").then(|| detail.readiness_notes.join("; "));
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
        Readiness::Blocked { reasons } => Some(reasons.join("; ")),
        Readiness::Ready { .. } | Readiness::Unknown { .. } => None,
    }
}

fn preview_dto(
    state: &AppState,
    plan: &BatchPlan,
    prs: &[PullRequest],
) -> Result<BulkPreview, AppError> {
    let mut cards = Vec::new();
    for card in &plan.cards {
        let signed_in =
            state.demo || signed_in_account(&state.store, &card.target.workspace)?.is_some();
        cards.push(BulkCard {
            target: target_dto(card.target.clone()),
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
    Ok(BulkPreview {
        cards,
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

fn core_target(target: &ReviewTarget) -> Target {
    Target {
        workspace: target.workspace.clone(),
        channel_id: target.channel_id.clone(),
        channel_name: target.channel_name.clone(),
    }
}

fn target_dto(target: Target) -> ReviewTarget {
    ReviewTarget {
        workspace: target.workspace,
        channel_id: target.channel_id,
        channel_name: target.channel_name,
    }
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
