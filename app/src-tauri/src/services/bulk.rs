//! Bulk request and bump from the bottom bar (§7.9).
//!
//! [`preview`] posts nothing; [`send`] posts through a [`ChatSource`].
//!
//! The preview and the send run the same gate and planner. Send plans again
//! and refuses any channel card whose content moved since the preview.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use pr_nexus_core::domain::{
    Involvement, PrId, PrState, PullRequest, Readiness, RequestStatus, SendOutcome, TeamRef,
};
use pr_nexus_core::error::CoreError;
use pr_nexus_core::mapping::lookup_mapping;
use pr_nexus_core::ownership;
use pr_nexus_core::providers::VcsProvider;
use pr_nexus_core::review::batch::{
    self, BatchInput, BatchOptions, BatchPlan, MessagePlanKind, Pace, Route, Skip,
};
use pr_nexus_core::review::groups::{self, GroupTeamChoice, OwnerGroup};
use pr_nexus_core::review::lifecycle::Target;
use pr_nexus_core::review::route::{self, Workspace};
use pr_nexus_core::settings;
use serde::{Deserialize, Serialize};

use crate::commands::{SlackIndicator, indicator};
use crate::error::AppError;
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
    /// Name channels from the session cache only (no Slack reads), so the
    /// dialog draws at once; `names_pending` then says whether a second,
    /// live preview could still rename a card.
    #[serde(default)]
    pub cached_names: bool,
    /// Team (`org/slug`) the user picked for a CODEOWNERS group.
    #[serde(default)]
    pub group_choices: Vec<GroupChoice>,
    /// Channel picked for a group, in place of a saved mapping or instead of one.
    #[serde(default)]
    pub group_channels: Vec<GroupChannelChoice>,
    /// Groups the user removed. They are not planned and not sent.
    #[serde(default)]
    pub skipped_groups: Vec<GroupRef>,
    /// Retarget one planned card. The key is the channel the routes were built
    /// for, before any earlier move in this preview.
    #[serde(default)]
    pub channel_moves: Vec<ChannelMove>,
    /// Cards the user removed, by the channel their routes were built for.
    #[serde(default)]
    pub skipped_channels: Vec<ChannelKey>,
}

/// One CODEOWNERS group's team, chosen in the preview.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct GroupChoice {
    pub pr_id: i64,
    pub group_id: String,
    pub team: String,
}

/// A channel the user picked for one CODEOWNERS group.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct GroupChannelChoice {
    pub pr_id: i64,
    pub group_id: String,
    pub target: ReviewTarget,
}

/// A group the user removed from this preview.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct GroupRef {
    pub pr_id: i64,
    pub group_id: String,
}

/// Send what was routed to `channel_id` to `target` instead.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct ChannelMove {
    pub workspace: String,
    pub channel_id: String,
    pub target: ReviewTarget,
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
    /// Why this card is not what the last request suggests, e.g. the team was
    /// remapped since: a new request here, not a bump in the old thread.
    pub notes: Vec<String>,
}

/// A CODEOWNERS group in the preview, and the team this plan asks.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct BulkGroup {
    pub pr_id: i64,
    pub number: u64,
    pub group_id: String,
    pub pattern: String,
    /// Every team on the line, `org/slug`.
    pub teams: Vec<String>,
    /// The team this plan asks. The first team on the line when none is mapped.
    pub selected: Option<String>,
    /// Set when the line has no channel yet. The preview still shows a card.
    pub needs_channel: bool,
    /// Why no channel is saved yet. The other sends stay.
    pub hint: Option<String>,
    /// Channel this group will be posted to. Empty until one is picked.
    pub channel_id: Option<String>,
    pub workspace: Option<String>,
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
    /// Drafts and failing checks. Both are notes; the PR is still sent.
    pub pr_warnings: Vec<PrWarning>,
    /// One row per CODEOWNERS group that still needs a request, or a hint.
    pub groups: Vec<BulkGroup>,
    /// Some card names a channel whose Slack name isn't known yet (only
    /// with `cached_names`): preview again without it before sending.
    pub names_pending: bool,
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
    send_with_vcs(state, chats, &LiveVcs, request, fingerprints)
}

/// [`send`] resolving owners through `vcs`.
pub fn send_with_vcs(
    state: &AppState,
    chats: &dyn ChatSource,
    vcs: &dyn VcsSource,
    request: &BulkRequest,
    fingerprints: &[String],
) -> Result<BulkResult, AppError> {
    let (plan, prs, _, _) = plan_request(state, chats, vcs, request, Fresh::Always)?;
    send_plan(state, chats, plan, &prs, fingerprints)
}

/// Send only the card for one channel (`fingerprint` from [`preview`]): the
/// MCP `request_reviews` tool sends each channel once it is approved, so the
/// other channels and the skipped PRs are left out, not recorded as skipped.
pub fn send_card(
    state: &AppState,
    chats: &dyn ChatSource,
    request: &BulkRequest,
    fingerprint: &str,
) -> Result<BulkResult, AppError> {
    let (mut plan, prs, _, _) = plan_request(state, chats, &LiveVcs, request, Fresh::Always)?;
    plan.cards.retain(|card| card.fingerprint() == fingerprint);
    if plan.cards.is_empty() {
        return Err(AppError::new(
            "conflict",
            "These pull requests changed since the preview; nothing was sent. Preview again.",
        ));
    }
    plan.skipped.clear();
    send_plan(state, chats, plan, &prs, &[fingerprint.to_owned()])
}

fn send_plan(
    state: &AppState,
    chats: &dyn ChatSource,
    mut plan: BatchPlan,
    prs: &[PullRequest],
    fingerprints: &[String],
) -> Result<BulkResult, AppError> {
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
                pr: pr_dto_by_id(prs, result.pr, &result.title),
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
    preview_with(
        state,
        &crate::services::review::LiveChats,
        &LiveVcs,
        request,
    )
}

/// [`preview`] reading channel names through `chats` and owners through
/// `vcs` (§7.9: the channel id is the truth, the stored name only a
/// fallback when Slack can't say).
pub fn preview_with(
    state: &AppState,
    chats: &dyn ChatSource,
    vcs: &dyn VcsSource,
    request: &BulkRequest,
) -> Result<BulkPreview, AppError> {
    let started = std::time::Instant::now();
    let (plan, prs, moved, groups) = plan_request(state, chats, vcs, request, Fresh::Cached)?;
    let preview = preview_dto(state, &plan, &prs, &moved, request.cached_names, groups)?;
    tracing::debug!(
        prs = request.pr_ids.len(),
        cards = preview.cards.len(),
        cached_names = request.cached_names,
        ms = started.elapsed().as_millis() as u64,
        "bulk preview"
    );
    Ok(preview)
}

/// Where owner lookups get their GitHub client.
pub trait VcsSource: Sync {
    fn vcs(&self, state: &AppState) -> Result<Box<dyn VcsProvider>, AppError>;
}

/// GitHub (the fixture provider in demo mode).
pub struct LiveVcs;

impl VcsSource for LiveVcs {
    fn vcs(&self, state: &AppState) -> Result<Box<dyn VcsProvider>, AppError> {
        crate::commands::vcs_client(state)
    }
}

/// How long a PR's resolved owners are reused by the preview (same head,
/// base, draft flag, and checks rollup). Send always resolves again.
pub(crate) const OWNERS_TTL: std::time::Duration = std::time::Duration::from_secs(2 * 60);
/// Owner resolutions in flight at once (each a GitHub GraphQL probe, plus
/// files and team members when the cache is stale).
pub(crate) const OWNER_LOOKUPS: usize = 4;

/// Whether owners may come from the session cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fresh {
    /// Preview: reuse owners resolved in the last [`OWNERS_TTL`].
    Cached,
    /// Send: resolve again.
    Always,
}

/// Blocked reason, fallback teams, and CODEOWNERS groups.
type Owners = (
    Option<String>,
    Vec<String>,
    Vec<crate::commands::OwnerGroupDto>,
);

/// Remember `detail` as `pr`'s owners for the next preview.
pub(crate) fn remember_owners(
    state: &AppState,
    pr: &PullRequest,
    detail: &crate::commands::PrDetail,
) {
    let (blocked, teams, groups) = owners_of(detail);
    crate::slack::lock(&state.owners).insert(
        pr.id.0,
        crate::state::OwnersEntry {
            head_sha: pr.head_sha.clone(),
            base_ref: pr.base_ref.clone(),
            is_draft: pr.is_draft,
            checks: pr.checks,
            at: std::time::Instant::now(),
            blocked,
            teams,
            groups,
        },
    );
}

fn owners_of(detail: &crate::commands::PrDetail) -> Owners {
    let teams = if detail.mandatory_teams.is_empty() {
        detail.candidate_teams.clone()
    } else {
        detail.mandatory_teams.clone()
    };
    let blocked = (detail.readiness == "blocked")
        .then(|| slack_skip_detail(&detail.readiness_notes))
        .flatten();
    (blocked, teams, detail.owner_groups.clone())
}

/// Owners for each of `prs`, in order: fresh cache entries as they are,
/// the rest resolved through one shared client, [`OWNER_LOOKUPS`] at a time.
fn resolve_owners(
    state: &AppState,
    vcs: &dyn VcsSource,
    prs: &[PullRequest],
    fresh: Fresh,
) -> Result<Vec<Result<Owners, AppError>>, AppError> {
    let flag = settings::blocked_review_required_is_ready(&state.store)?;
    let cached = |pr: &PullRequest| -> Option<Owners> {
        if fresh == Fresh::Always {
            return None;
        }
        let owners = crate::slack::lock(&state.owners);
        let entry = owners.get(&pr.id.0)?;
        (entry.at.elapsed() < OWNERS_TTL
            && entry.head_sha == pr.head_sha
            && entry.base_ref == pr.base_ref
            && entry.is_draft == pr.is_draft
            && entry.checks == pr.checks)
            .then(|| {
                (
                    soften_cached_block(&entry.blocked),
                    entry.teams.clone(),
                    entry.groups.clone(),
                )
            })
    };
    let mut out: Vec<Option<Result<Owners, AppError>>> =
        prs.iter().map(|pr| cached(pr).map(Ok)).collect();
    let missing: Vec<&PullRequest> = prs
        .iter()
        .zip(&out)
        .filter(|(_, hit)| hit.is_none())
        .map(|(pr, _)| pr)
        .collect();
    if !missing.is_empty() {
        let started = std::time::Instant::now();
        let resolved: Vec<Result<Owners, AppError>> = match vcs.vcs(state) {
            Ok(provider) => crate::parallel::bounded_map(&missing, OWNER_LOOKUPS, |pr| {
                let report =
                    ownership::resolve(&state.store, provider.as_ref(), pr, flag, Utc::now())?;
                let detail = crate::commands::detail_dto(pr.id, report);
                remember_owners(state, pr, &detail);
                Ok(owners_of(&detail))
            }),
            Err(err) => missing
                .iter()
                .map(|_| Err(AppError::new(&err.kind, err.message.clone())))
                .collect(),
        };
        tracing::debug!(
            prs = missing.len(),
            ms = started.elapsed().as_millis() as u64,
            "bulk preview owners"
        );
        let mut resolved = resolved.into_iter();
        for slot in out.iter_mut().filter(|slot| slot.is_none()) {
            *slot = resolved.next();
        }
    }
    Ok(out
        .into_iter()
        .map(|slot| slot.expect("every PR has owners or an error"))
        .collect())
}

/// A PR whose active request lives in a channel its teams are no longer
/// mapped to (§7.9 remap rule): it is planned as a new request in the
/// mapped channel, and its card says so.
#[derive(Debug, Clone)]
struct Moved {
    pr: PrId,
    /// Mapped channel the PR is planned in.
    to: (String, String),
    /// Label of the channel the old thread lives in.
    from: String,
}

type PlanParts = (BatchPlan, Vec<PullRequest>, Vec<Moved>, Vec<BulkGroup>);

fn plan_request(
    state: &AppState,
    chats: &dyn ChatSource,
    vcs: &dyn VcsSource,
    request: &BulkRequest,
    fresh: Fresh,
) -> Result<PlanParts, AppError> {
    if request.pr_ids.is_empty() {
        return Err(AppError::new(
            "validation",
            "Select at least one pull request",
        ));
    }
    let names = if request.cached_names {
        crate::slack::Names::Cached
    } else {
        crate::slack::Names::Live
    };
    let override_target = match request.target.clone().map(validate_target).transpose()? {
        Some(target) => crate::slack::resolve_targets(state, chats, vec![target], names).pop(),
        None => None,
    };
    let flag = settings::blocked_review_required_is_ready(&state.store)?;
    let mut prs = Vec::new();
    let mut seen = BTreeSet::new();
    for &id in &request.pr_ids {
        if seen.insert(id) {
            prs.push(load_pr(&state.store, id)?);
        }
    }
    let eligible = |pr: &PullRequest| -> Result<(), String> {
        if pr.state != PrState::Open {
            return Err("Not open".into());
        }
        if !pr.involvement.contains(Involvement::AUTHOR) {
            return Err("Not authored by you".into());
        }
        Ok(())
    };
    // Every GitHub lookup up front, in parallel; the rest is local.
    let to_resolve: Vec<PullRequest> = if override_target.is_some() {
        Vec::new()
    } else {
        prs.iter()
            .filter(|pr| eligible(pr).is_ok())
            .cloned()
            .collect()
    };
    let mut owners = resolve_owners(state, vcs, &to_resolve, fresh)?.into_iter();

    struct Planned {
        pr: PullRequest,
        teams: Vec<String>,
        choices: Vec<crate::slack::TargetChoice>,
        groups: Vec<BulkGroup>,
    }
    let mut skipped = Vec::new();
    let mut planned = Vec::new();
    let mappings = state.store.list_mappings()?;
    for pr in &prs {
        let skip = |detail: String| Skip {
            pr: pr.id,
            title: pr.title.clone(),
            channel: None,
            detail,
        };
        if let Err(detail) = eligible(pr) {
            skipped.push(skip(detail));
            continue;
        }
        let (blocked, fallback, catalog) = if override_target.is_some() {
            (
                blocked_reasons(ownership::cached_readiness(&state.store, pr, flag)?),
                Vec::new(),
                Vec::new(),
            )
        } else {
            match owners.next().expect("one owner result per eligible PR") {
                Ok(owners) => owners,
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
        let (teams, groups) = match route_groups(pr, &fallback, &catalog, request, &mappings) {
            RouteTeams::Approved => {
                skipped.push(skip("All owning teams approved".into()));
                continue;
            }
            RouteTeams::Ask { teams, groups } => (teams, groups),
        };
        let choices = if override_target.is_some() {
            Vec::new()
        } else {
            targets_for(&state.store, pr, &teams)?
        };
        planned.push(Planned {
            pr: pr.clone(),
            teams,
            choices,
            groups,
        });
    }
    // Name every routed channel at once: each distinct channel is read
    // from Slack once, in parallel, and then comes from the session cache.
    let all: Vec<ReviewTarget> = planned
        .iter()
        .flat_map(|p| p.choices.iter().map(|choice| choice.target.clone()))
        .collect();
    let mut resolved = crate::slack::resolve_targets(state, chats, all, names).into_iter();
    for p in &mut planned {
        for choice in &mut p.choices {
            choice.target = resolved.next().expect("one name per target");
        }
    }
    let group_channels = named_targets(
        state,
        chats,
        request.group_channels.clone(),
        names,
        |choice| &mut choice.target,
    );
    let channel_moves = named_targets(
        state,
        chats,
        request.channel_moves.clone(),
        names,
        |choice| &mut choice.target,
    );

    let mut inputs = Vec::new();
    let mut moved = Vec::new();
    let mut group_views = Vec::new();
    for Planned {
        pr,
        teams,
        choices,
        mut groups,
    } in planned
    {
        let removed = groups
            .iter()
            .any(|group| group_skipped(request, pr.id.0, &group.group_id));
        groups.retain(|group| !group_skipped(request, pr.id.0, &group.group_id));
        moved.extend(mapping_moves(&pr, &choices));
        // A pull request that has CODEOWNERS groups is routed only from those
        // groups. Removing every group leaves nothing to send; an open thread
        // in some other channel must not reappear as the only route.
        let routes: Vec<Route> = match &override_target {
            Some(target) => vec![Route {
                target: core_target(target),
                teams: teams.clone(),
            }],
            None if !groups.is_empty() || removed => {
                routes_for_groups(state, &pr, &mut groups, &choices, &group_channels)?
            }
            None => {
                // Never bump a thread in a channel the team left: the card
                // would name the mapped channel and post to the old one.
                let (unresolved, rest): (Vec<_>, Vec<_>) = choices
                    .iter()
                    .cloned()
                    .partition(|choice| choice.source == "unresolved");
                if rest.iter().all(|choice| choice.source == "previous")
                    && let Some(first) = unresolved.first()
                {
                    group_views.extend(groups);
                    skipped.push(Skip {
                        pr: pr.id,
                        title: pr.title.clone(),
                        channel: None,
                        detail: format!(
                            "Its team's channel #{} has no channel ID yet. Add the ID in Settings → Mappings, or request it alone.",
                            first.target.channel_name
                        ),
                    });
                    continue;
                }
                let routes = rest
                    .into_iter()
                    .filter(|choice| choice.source != "previous")
                    .map(|choice| Route {
                        target: core_target(&choice.target),
                        teams: if choice.teams.is_empty() {
                            teams.clone()
                        } else {
                            choice.teams
                        },
                    })
                    .collect();
                edit_routes(state, &pr, routes, request, &channel_moves)?
            }
        };
        let pending = groups.iter().any(|group| group.needs_channel);
        group_views.extend(groups);
        if routes.is_empty() {
            // A line with no channel stays in the preview as a card the user
            // can fill in. Removing every card is not a "no mapping" skip.
            if !pending && !removed {
                let hints: Vec<String> = group_views
                    .iter()
                    .filter(|group| group.pr_id == pr.id.0)
                    .filter_map(|group| group.hint.clone())
                    .collect();
                skipped.push(Skip {
                    pr: pr.id,
                    title: pr.title.clone(),
                    channel: None,
                    detail: if hints.is_empty() {
                        "No mapped channel for its teams. Request it alone, or send all to one channel."
                            .into()
                    } else {
                        hints.join(" ")
                    },
                });
            }
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
    Ok((plan, prs, moved, group_views))
}

enum RouteTeams {
    /// Every matched CODEOWNERS line already has an approval.
    Approved,
    Ask {
        teams: Vec<String>,
        groups: Vec<BulkGroup>,
    },
}

/// Teams to route, and the group rows the preview shows.
///
/// No CODEOWNERS groups keeps the coverage/candidate teams. Otherwise one
/// team is chosen per line that still needs an approval.
fn route_groups(
    pr: &PullRequest,
    fallback: &[String],
    catalog: &[crate::commands::OwnerGroupDto],
    request: &BulkRequest,
    mappings: &[pr_nexus_core::domain::TeamChannelMapping],
) -> RouteTeams {
    if catalog.is_empty() {
        return RouteTeams::Ask {
            teams: fallback.to_vec(),
            groups: Vec::new(),
        };
    }
    let pending: Vec<OwnerGroup> = catalog
        .iter()
        .filter(|group| !group.satisfied)
        .map(owner_group_from_dto)
        .collect();
    if pending.is_empty() {
        return RouteTeams::Approved;
    }
    let choices = request
        .group_choices
        .iter()
        .filter(|choice| choice.pr_id == pr.id.0)
        .filter_map(|choice| {
            parse_team(&choice.team).map(|team| GroupTeamChoice {
                group_id: choice.group_id.clone(),
                team,
            })
        })
        .collect::<Vec<_>>();
    let plans = groups::plan_group_requests(
        &pending,
        |team| mapping_has_channel(mappings, team),
        &choices,
    );
    let mut teams = Vec::new();
    let mut views = Vec::new();
    for plan in &plans {
        if let Some(team) = plan.team.as_ref().filter(|_| !plan.unmapped) {
            let label = team.to_string();
            if !teams
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&label))
            {
                teams.push(label);
            }
        }
        let users: Vec<&str> = plan.group.users.iter().map(String::as_str).collect();
        let selected = plan
            .team
            .clone()
            .or_else(|| plan.group.teams.first().cloned());
        views.push(BulkGroup {
            pr_id: pr.id.0,
            number: pr.number,
            group_id: plan.group.id.clone(),
            pattern: plan.group.pattern.clone(),
            teams: plan.group.teams.iter().map(ToString::to_string).collect(),
            selected: selected.as_ref().map(ToString::to_string),
            needs_channel: plan.unmapped,
            hint: plan
                .unmapped
                .then(|| groups::unmapped_hint(&plan.group.teams, &users)),
            channel_id: None,
            workspace: None,
        });
    }
    RouteTeams::Ask {
        teams,
        groups: views,
    }
}

fn mapping_has_channel(rows: &[pr_nexus_core::domain::TeamChannelMapping], team: &TeamRef) -> bool {
    lookup_mapping(rows, &team.org, &team.slug, "")
        .is_some_and(|row| !row.slack_channel_id.trim().is_empty())
}

fn owner_group_from_dto(dto: &crate::commands::OwnerGroupDto) -> OwnerGroup {
    OwnerGroup {
        id: dto.id.clone(),
        pattern: dto.pattern.clone(),
        teams: dto
            .teams
            .iter()
            .filter_map(|team| parse_team(team))
            .collect(),
        users: dto.users.clone(),
    }
}

fn parse_team(label: &str) -> Option<TeamRef> {
    let label = label.trim().trim_start_matches('@');
    let (org, slug) = label.split_once('/')?;
    if org.is_empty() || slug.is_empty() || slug.contains('/') {
        return None;
    }
    Some(TeamRef {
        org: org.to_owned(),
        slug: slug.to_owned(),
    })
}

fn group_skipped(request: &BulkRequest, pr_id: i64, group_id: &str) -> bool {
    request
        .skipped_groups
        .iter()
        .any(|skipped| skipped.pr_id == pr_id && skipped.group_id == group_id)
}

fn channel_skipped(request: &BulkRequest, workspace: &str, channel_id: &str) -> bool {
    request.skipped_channels.iter().any(|key| {
        key.workspace.trim().eq_ignore_ascii_case(workspace)
            && key.channel_id.trim().eq_ignore_ascii_case(channel_id)
    })
}

/// Read channel names for the targets inside `items` before the plan is built.
fn named_targets<T>(
    state: &AppState,
    chats: &dyn ChatSource,
    mut items: Vec<T>,
    names: crate::slack::Names,
    mut target_of: impl FnMut(&mut T) -> &mut ReviewTarget,
) -> Vec<T> {
    let snapshot: Vec<ReviewTarget> = items
        .iter_mut()
        .map(|item| target_of(item).clone())
        .collect();
    let resolved = crate::slack::resolve_targets(state, chats, snapshot, names);
    for (item, target) in items.iter_mut().zip(resolved) {
        *target_of(item) = target;
    }
    items
}

/// "Channel changed since the last request" for a mapping that no longer
/// points at the open thread. A card shows it only when it still posts there.
fn mapping_moves(pr: &PullRequest, choices: &[crate::slack::TargetChoice]) -> Vec<Moved> {
    let mut moved = Vec::new();
    for old in choices.iter().filter(|choice| choice.source == "previous") {
        for to in choices.iter().filter(|choice| choice.source == "mapping") {
            moved.push(Moved {
                pr: pr.id,
                to: (
                    to.target.workspace.trim().to_ascii_lowercase(),
                    to.target.channel_id.clone(),
                ),
                from: crate::slack::to_core(&old.target).label(),
            });
        }
    }
    moved
}

/// One route per distinct channel the groups will actually post to.
///
/// A saved mapping is the default. `overrides` replaces it, or supplies a
/// channel for a line that has none. Lines still without a channel stay in
/// `groups` and are left out of the routes, so an earlier request in some
/// other channel cannot become the only send.
fn routes_for_groups(
    state: &AppState,
    pr: &PullRequest,
    groups: &mut [BulkGroup],
    choices: &[crate::slack::TargetChoice],
    overrides: &[GroupChannelChoice],
) -> Result<Vec<Route>, AppError> {
    let mut routes = Vec::new();
    for group in groups.iter_mut() {
        let target = if let Some(choice) = overrides
            .iter()
            .find(|choice| choice.pr_id == pr.id.0 && choice.group_id == group.group_id)
        {
            let mut target = validate_target(choice.target.clone())?;
            fill_open_request(state, pr, &mut target)?;
            Some(target)
        } else {
            mapped_choice(choices, group.selected.as_deref()).map(|choice| choice.target.clone())
        };
        let Some(target) = target else {
            group.needs_channel = true;
            group.channel_id = None;
            group.workspace = None;
            continue;
        };
        if group.selected.is_none() {
            group.selected = group.teams.first().cloned();
        }
        group.needs_channel = false;
        group.hint = None;
        group.channel_id = Some(target.channel_id.clone());
        group.workspace = Some(target.workspace.clone());
        push_route(&mut routes, &target, group.selected.clone());
    }
    Ok(routes)
}

fn mapped_choice<'a>(
    choices: &'a [crate::slack::TargetChoice],
    selected: Option<&str>,
) -> Option<&'a crate::slack::TargetChoice> {
    let selected = selected?;
    choices.iter().find(|choice| {
        choice.source != "previous"
            && choice.source != "unresolved"
            && !choice.target.channel_id.trim().is_empty()
            && choice
                .teams
                .iter()
                .any(|team| team.eq_ignore_ascii_case(selected))
    })
}

fn push_route(routes: &mut Vec<Route>, target: &ReviewTarget, team: Option<String>) {
    let mut teams = Vec::new();
    if let Some(team) = team {
        teams.push(team);
    }
    absorb_route(
        routes,
        Route {
            target: core_target(target),
            teams,
        },
    );
}

/// Drop removed cards and retarget the rest. Routes that land on one channel
/// for the same pull request become one message in [`batch::plan_batch`].
fn edit_routes(
    state: &AppState,
    pr: &PullRequest,
    routes: Vec<Route>,
    request: &BulkRequest,
    moves: &[ChannelMove],
) -> Result<Vec<Route>, AppError> {
    let mut out = Vec::new();
    for route in routes {
        if channel_skipped(request, &route.target.workspace, &route.target.channel_id) {
            continue;
        }
        let Some(change) = moves.iter().find(|change| {
            change
                .workspace
                .trim()
                .eq_ignore_ascii_case(&route.target.workspace)
                && change
                    .channel_id
                    .trim()
                    .eq_ignore_ascii_case(&route.target.channel_id)
        }) else {
            absorb_route(&mut out, route);
            continue;
        };
        let mut target = validate_target(change.target.clone())?;
        fill_open_request(state, pr, &mut target)?;
        absorb_route(
            &mut out,
            Route {
                target: core_target(&target),
                teams: route.teams,
            },
        );
    }
    Ok(out)
}

fn absorb_route(routes: &mut Vec<Route>, route: Route) {
    if let Some(existing) = routes.iter_mut().find(|existing| {
        existing
            .target
            .channel_id
            .eq_ignore_ascii_case(&route.target.channel_id)
            && existing
                .target
                .workspace
                .eq_ignore_ascii_case(&route.target.workspace)
    }) {
        for team in route.teams {
            if !existing
                .teams
                .iter()
                .any(|have| have.eq_ignore_ascii_case(&team))
            {
                existing.teams.push(team);
            }
        }
        return;
    }
    routes.push(route);
}

/// An id typed with no workspace still bumps the open thread in that channel.
fn fill_open_request(
    state: &AppState,
    pr: &PullRequest,
    target: &mut ReviewTarget,
) -> Result<(), AppError> {
    if !target.workspace.trim().is_empty() {
        return Ok(());
    }
    let Some(existing) = state
        .store
        .list_review_requests(pr.id)?
        .into_iter()
        .find(|request| {
            request.status == RequestStatus::Active
                && request.channel_id.eq_ignore_ascii_case(&target.channel_id)
        })
    else {
        return Ok(());
    };
    target.workspace = existing.workspace;
    if target.team_id.is_none() {
        target.team_id = existing.team_id;
    }
    if target.channel_name.is_empty()
        || target.channel_name.eq_ignore_ascii_case(&target.channel_id)
    {
        target.channel_name = existing.channel_name;
    }
    Ok(())
}

fn blocked_reasons(readiness: Readiness) -> Option<String> {
    match readiness {
        Readiness::Blocked { reasons } => slack_skip_detail(&reasons),
        Readiness::Ready { .. } | Readiness::Unknown { .. } => None,
    }
}

/// A draft and a failing check only warn (see [`crate::slack::pr_warnings`]).
/// Pending checks and every other blocked reason still leave the PR out.
/// `None` when nothing that still blocks remains.
fn slack_skip_detail(reasons: &[String]) -> Option<String> {
    let rest: Vec<&str> = reasons
        .iter()
        .map(String::as_str)
        .filter(|reason| *reason != DRAFT_REASON && !is_failing_check(reason))
        .collect();
    if rest.is_empty() && !reasons.is_empty() {
        None
    } else {
        Some(rest.join("; "))
    }
}

/// `check <name> is fail|cancel|timeout|action_required` from readiness.
/// Pending stays a blocker. List readiness is unchanged.
fn is_failing_check(reason: &str) -> bool {
    let Some(rest) = reason.strip_prefix("check ") else {
        return false;
    };
    let Some((_, bucket)) = rest.rsplit_once(" is ") else {
        return false;
    };
    matches!(bucket, "fail" | "cancel" | "timeout" | "action_required")
}

/// A session cache entry was stored after this filter. Apply it again so an
/// entry recorded as only a failing check does not keep the PR out.
fn soften_cached_block(blocked: &Option<String>) -> Option<String> {
    match blocked {
        None => None,
        Some(text) if text.is_empty() => Some(String::new()),
        Some(text) => slack_skip_detail(
            &text
                .split("; ")
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
    }
}

/// The readiness reason for a draft (`review::readiness`).
const DRAFT_REASON: &str = "PR is a draft";

fn preview_dto(
    state: &AppState,
    plan: &BatchPlan,
    prs: &[PullRequest],
    moved: &[Moved],
    cached_names: bool,
    groups: Vec<BulkGroup>,
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
            notes: moved_notes(card, moved),
        });
    }
    let sent: BTreeSet<i64> = plan
        .cards
        .iter()
        .flat_map(|card| card.messages.iter())
        .flat_map(|message| message.items.iter())
        .map(|item| item.pr.id.0)
        .collect();
    let pr_warnings = prs
        .iter()
        .flat_map(crate::slack::pr_warnings)
        .filter(|warning| warning.kind == "draft" || sent.contains(&warning.pr_id))
        .collect();
    let names_pending = cached_names
        && crate::slack::names_pending(
            state,
            &cards
                .iter()
                .map(|card| card.target.clone())
                .collect::<Vec<_>>(),
        );
    Ok(BulkPreview {
        cards,
        names_pending,
        pr_warnings,
        groups,
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

/// "Channel changed since the last request" for the PRs of `card` whose
/// old thread lives elsewhere, one line per old channel.
fn moved_notes(card: &batch::ChannelCard, moved: &[Moved]) -> Vec<String> {
    let key = (
        card.target.workspace.trim().to_ascii_lowercase(),
        card.target.channel_id.clone(),
    );
    let mut by_from: Vec<(String, Vec<String>)> = Vec::new();
    for message in &card.messages {
        if !matches!(message.kind, MessagePlanKind::Request) {
            continue;
        }
        for item in &message.items {
            for m in moved.iter().filter(|m| m.pr == item.pr.id && m.to == key) {
                let number = format!("#{}", item.pr.number);
                match by_from.iter_mut().find(|(from, _)| *from == m.from) {
                    Some((_, list)) if !list.contains(&number) => list.push(number),
                    Some(_) => {}
                    None => by_from.push((m.from.clone(), vec![number])),
                }
            }
        }
    }
    by_from
        .into_iter()
        .map(|(from, prs)| {
            format!(
                "Channel changed since the last request: new request in {} for {}. The old thread in {from} is not bumped.",
                card.target.label(),
                prs.join(", ")
            )
        })
        .collect()
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
    use super::{DRAFT_REASON, slack_skip_detail, soften_cached_block};

    #[test]
    fn a_draft_or_failing_check_does_not_block_but_other_reasons_still_do() {
        assert_eq!(slack_skip_detail(&[DRAFT_REASON.to_owned()]), None);
        assert_eq!(slack_skip_detail(&["check ci is fail".to_owned()]), None);
        assert_eq!(
            slack_skip_detail(&["check ci is pending".to_owned()]),
            Some("check ci is pending".to_owned())
        );
        assert_eq!(
            slack_skip_detail(&[
                DRAFT_REASON.to_owned(),
                "check ci is fail".to_owned(),
                "PR has merge conflicts".to_owned()
            ]),
            Some("PR has merge conflicts".to_owned())
        );
        assert_eq!(slack_skip_detail(&[]), Some(String::new()));
        assert_eq!(soften_cached_block(&None), None);
        assert_eq!(soften_cached_block(&Some("check ci is fail".into())), None);
        assert_eq!(
            soften_cached_block(&Some("check ci is fail; PR has merge conflicts".into())),
            Some("PR has merge conflicts".into())
        );
    }
}
