//! Owner routing. A straight port of `decide_owners`, with team orgs kept.
//!
//! `requested_teams` are recorded and never become owners. Approved teams and
//! teams whose membership could not be checked leave the actionable lists.
//! The final `needs_user_choice` flag is recomputed after that filter, which
//! is what the original function does.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::domain::{TeamRef, Tri};
use crate::review::readiness::FILES_TRUNCATED;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttributedApproval {
    pub reviewer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnBehalfOf {
    pub org: String,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRecord {
    pub id: Option<String>,
    pub state: String,
    pub author_typename: String,
    pub author_login: String,
    pub commit: Option<String>,
    pub on_behalf_of: Vec<OnBehalfOf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBundle {
    pub fetched: bool,
    pub complete: bool,
    pub reviews: Vec<ReviewRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    pub complete: bool,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessWord {
    Ready,
    Blocked,
    Unknown,
}

impl ReadinessWord {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Blocked => "blocked",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GivenReadiness {
    pub status: ReadinessWord,
    pub head_sha: Option<String>,
    pub blocking_reasons: Vec<String>,
    pub unknown_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OwnerQuery {
    pub readiness: Option<GivenReadiness>,
    pub head_sha: Option<String>,
    pub reviews: Option<ReviewBundle>,
    pub protection_fetched: bool,
    pub protection_present: bool,
    pub require_code_owner_reviews: Option<bool>,
    pub ruleset_requires_code_owners: Option<bool>,
    pub requested_teams: Vec<String>,
    pub author: Option<String>,
    pub owner: Option<String>,
    pub files_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerCatalog {
    pub source: Option<String>,
    pub codeowners_file: Option<String>,
    pub teams: Vec<TeamRef>,
    pub exclusive_teams: Vec<TeamRef>,
    pub coverage_teams: Vec<TeamRef>,
    pub optional_teams: Vec<TeamRef>,
    pub users: Vec<String>,
    pub path_owners: Vec<(String, Vec<TeamRef>)>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecideOptions<'a> {
    pub user_team: Option<&'a TeamRef>,
    pub approval_attribution: Option<&'a BTreeMap<TeamRef, Vec<AttributedApproval>>>,
    pub membership: Option<&'a BTreeMap<TeamRef, Membership>>,
    pub review_attribution_unknown: Option<&'a BTreeMap<TeamRef, Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedTeam {
    pub team: TeamRef,
    pub reviews: Vec<AttributedApproval>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttributionGap {
    pub team: TeamRef,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OwnerDecision {
    pub readiness_status: String,
    pub head_sha: Option<String>,
    pub blocking_reasons: Vec<String>,
    pub unknown_reasons: Vec<String>,
    pub codeowners_required: Tri,
    pub codeowners_required_reason: String,
    pub mandatory_teams: Vec<TeamRef>,
    pub optional_teams: Vec<TeamRef>,
    pub routing_hints: Vec<TeamRef>,
    pub post_to_teams: Vec<TeamRef>,
    pub ask_about_teams: Vec<TeamRef>,
    pub candidate_teams: Vec<TeamRef>,
    pub non_actionable_suggestions: Vec<TeamRef>,
    pub approved_teams: Vec<TeamRef>,
    pub skipped_teams: Vec<SkippedTeam>,
    pub review_attribution_unknown: Vec<AttributionGap>,
    pub needs_user_choice: bool,
    pub confirmation_required: bool,
    pub no_op: bool,
    pub users_ignored_for_slack: Vec<String>,
    pub requested_teams_informational: Vec<String>,
    pub rationale: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveReadiness {
    status: ReadinessWord,
    head_sha: Option<String>,
    blocking_reasons: Vec<String>,
    unknown_reasons: Vec<String>,
}

pub fn codeowners_required(query: &OwnerQuery) -> (Tri, String) {
    let protection_required = query.require_code_owner_reviews;
    let ruleset = query.ruleset_requires_code_owners;
    if protection_required == Some(true) || ruleset == Some(true) {
        let mut parts = Vec::new();
        if protection_required == Some(true) {
            parts.push("branch protection require_code_owner_reviews=true");
        }
        if ruleset == Some(true) {
            parts.push("applicable ruleset require_code_owner_review=true");
        }
        return (Tri::True, parts.join("; "));
    }
    if protection_required == Some(false) && ruleset == Some(false) {
        return (
            Tri::False,
            "branch protection and applicable rulesets do not require code owners".into(),
        );
    }
    if protection_required == Some(false) && ruleset.is_none() {
        return (
            Tri::False,
            "branch protection require_code_owner_reviews=false (ruleset details unavailable)"
                .into(),
        );
    }
    if protection_required.is_none() && ruleset == Some(false) && query.protection_fetched {
        return (
            Tri::False,
            "no code-owner requirement in fetched protection/rulesets".into(),
        );
    }
    if ruleset == Some(false) && !query.protection_fetched && !query.protection_present {
        return (
            Tri::False,
            "applicable rulesets do not require code owners (branch protection not fetched)".into(),
        );
    }
    let mut reasons = Vec::new();
    if !query.protection_fetched {
        reasons.push("branch protection not fetched");
    } else if !query.protection_present || protection_required.is_none() {
        reasons.push("branch protection missing require_code_owner_reviews");
    }
    if ruleset.is_none() {
        reasons.push("ruleset code-owner requirement unknown");
    }
    (
        Tri::Unknown,
        if reasons.is_empty() {
            "insufficient review-requirement metadata".into()
        } else {
            reasons.join("; ")
        },
    )
}

pub fn decide_owners(
    query: &OwnerQuery,
    catalog: &OwnerCatalog,
    options: &DecideOptions<'_>,
) -> OwnerDecision {
    let mut readiness = normalize_readiness(query);
    let (required, required_reason) = codeowners_required(query);
    let coverage = catalog.coverage_teams.clone();
    let ownership_optional = optional_teams(catalog, &coverage);
    let candidate_teams = match options.user_team {
        Some(team) => vec![team.clone()],
        None => unique(
            coverage
                .iter()
                .cloned()
                .chain(ownership_optional.iter().cloned()),
        ),
    };
    let (attributed, inferred_unknown) =
        approval_evidence(query, &candidate_teams, &readiness, options);
    let mut attribution_unknown: BTreeMap<TeamRef, Vec<String>> = inferred_unknown;
    if let Some(extra) = options.review_attribution_unknown {
        for (team, reasons) in extra {
            let entry = attribution_unknown.entry(team.clone()).or_default();
            for reason in reasons {
                if !entry.contains(reason) {
                    entry.push(reason.clone());
                }
            }
        }
    }
    let approved_teams: Vec<TeamRef> = candidate_teams
        .iter()
        .filter(|team| {
            attributed
                .get(*team)
                .is_some_and(|reviews| !reviews.is_empty())
        })
        .cloned()
        .collect();
    let skipped_teams = approved_teams
        .iter()
        .map(|team| SkippedTeam {
            team: team.clone(),
            reviews: attributed.get(team).cloned().unwrap_or_default(),
        })
        .collect();
    let uncertain: Vec<TeamRef> = attribution_unknown.keys().cloned().collect();
    let remaining: Vec<TeamRef> = candidate_teams
        .iter()
        .filter(|team| !approved_teams.contains(team))
        .cloned()
        .collect();
    let non_actionable_seed: Vec<TeamRef> = remaining
        .iter()
        .filter(|team| uncertain.contains(team))
        .cloned()
        .collect();
    let mut rationale = vec![
        format!(
            "codeowners_required={} ({required_reason})",
            required.as_str()
        ),
        format!("readiness={}", readiness.status.as_str()),
    ];

    let mut mandatory = match options.user_team {
        Some(team) => vec![team.clone()],
        None => Vec::new(),
    };
    let mut optional = Vec::new();
    let mut routing_hints = Vec::new();
    let mut ask_about_teams = Vec::new();
    let mut post_to_teams = Vec::new();

    if options.user_team.is_none()
        && catalog.error.is_none()
        && matches!(required, Tri::True | Tri::False)
    {
        mandatory = coverage.clone();
        optional = ownership_optional.clone();
        if required == Tri::False && !mandatory.is_empty() {
            rationale.push(
                "code-owner approval not required by branch rules; still routing to CODEOWNERS coverage teams"
                    .into(),
            );
        }
        if (!mandatory.is_empty() && !optional.is_empty())
            || (mandatory.is_empty() && !optional.is_empty())
        {
            ask_about_teams = optional.clone();
        } else if mandatory.is_empty() {
            rationale.push("no team CODEOWNERS matched changed paths".into());
        }
    } else if options.user_team.is_none() && catalog.error.is_some() {
        rationale.push(format!(
            "CODEOWNERS unavailable: {}",
            catalog.error.as_deref().unwrap_or("")
        ));
    } else if options.user_team.is_none() {
        routing_hints = if coverage.is_empty() {
            catalog.teams.clone()
        } else {
            coverage.clone()
        };
        optional = catalog.teams.clone();
        ask_about_teams = if routing_hints.is_empty() {
            catalog.teams.clone()
        } else {
            routing_hints.clone()
        };
        rationale.push(
            "code-owner requirement unknown — treat CODEOWNERS coverage teams as routing hints and confirm with the user before posting"
                .into(),
        );
    }

    if query.files_truncated {
        ask_about_teams = if ask_about_teams.is_empty() {
            if optional.is_empty() {
                if routing_hints.is_empty() {
                    catalog.teams.clone()
                } else {
                    routing_hints.clone()
                }
            } else {
                optional.clone()
            }
        } else {
            ask_about_teams.clone()
        };
        ask_about_teams = unique(ask_about_teams);
        rationale.push(FILES_TRUNCATED.into());
        if readiness.status != ReadinessWord::Blocked {
            readiness.status = ReadinessWord::Unknown;
            readiness.unknown_reasons.push(FILES_TRUNCATED.into());
        }
    }

    if !query.requested_teams.is_empty() {
        rationale.push(format!(
            "requested_teams are informational only (not ownership): {}",
            query.requested_teams.join(", ")
        ));
    }

    mandatory.retain(|team| !approved_teams.contains(team) && !uncertain.contains(team));
    optional.retain(|team| !approved_teams.contains(team) && !uncertain.contains(team));
    routing_hints.retain(|team| !approved_teams.contains(team) && !uncertain.contains(team));
    let mut ask_filtered: Vec<TeamRef> = ask_about_teams
        .into_iter()
        .filter(|team| !approved_teams.contains(team))
        .collect();
    ask_filtered.extend(non_actionable_seed.iter().cloned());
    ask_about_teams = unique(ask_filtered);
    // The script sets this flag while choosing lists, then overwrites it from
    // the filtered lists. Later branches set it again.
    let mut needs_user_choice = !optional.is_empty()
        || !routing_hints.is_empty()
        || (candidate_teams.is_empty() && options.user_team.is_none());
    if catalog.error.is_some() && options.user_team.is_none() {
        needs_user_choice = true;
    }
    if query.files_truncated {
        needs_user_choice = true;
    }
    let mut non_actionable = non_actionable_seed;
    if !uncertain.is_empty() {
        needs_user_choice = true;
        let mut names: Vec<String> = uncertain.iter().map(ToString::to_string).collect();
        names.sort();
        rationale.push(format!(
            "review attribution is uncertain for: {}",
            names.join(", ")
        ));
    }

    let no_op = readiness.status == ReadinessWord::Ready
        && !candidate_teams.is_empty()
        && remaining.is_empty()
        && attribution_unknown.is_empty()
        && !query.files_truncated
        && (options.user_team.is_some() || catalog.error.is_none());
    let mut confirmation_required =
        readiness.status == ReadinessWord::Unknown || !attribution_unknown.is_empty();

    if readiness.status == ReadinessWord::Blocked {
        mandatory.clear();
        optional.clear();
        routing_hints.clear();
        ask_about_teams.clear();
        needs_user_choice = false;
        confirmation_required = false;
        non_actionable = remaining.clone();
        rationale.extend(readiness.blocking_reasons.iter().cloned());
    } else if no_op {
        mandatory.clear();
        optional.clear();
        routing_hints.clear();
        ask_about_teams.clear();
        needs_user_choice = false;
        rationale.push("all candidate CODEOWNERS teams have effective human approvals".into());
    } else if confirmation_required {
        mandatory.clear();
        optional.clear();
        routing_hints.clear();
        ask_about_teams = remaining.clone();
        needs_user_choice = true;
        non_actionable = remaining.clone();
        rationale.extend(readiness.unknown_reasons.iter().cloned());
    } else {
        post_to_teams = mandatory.clone();
    }

    OwnerDecision {
        readiness_status: readiness.status.as_str().to_string(),
        head_sha: readiness.head_sha,
        blocking_reasons: readiness.blocking_reasons,
        unknown_reasons: readiness.unknown_reasons,
        codeowners_required: required,
        codeowners_required_reason: required_reason,
        mandatory_teams: mandatory,
        optional_teams: optional,
        routing_hints,
        post_to_teams,
        ask_about_teams,
        candidate_teams,
        non_actionable_suggestions: non_actionable,
        approved_teams,
        skipped_teams,
        review_attribution_unknown: attribution_unknown
            .into_iter()
            .map(|(team, reasons)| AttributionGap { team, reasons })
            .collect(),
        needs_user_choice,
        confirmation_required,
        no_op,
        users_ignored_for_slack: catalog.users.clone(),
        requested_teams_informational: query.requested_teams.clone(),
        rationale,
    }
}

fn optional_teams(catalog: &OwnerCatalog, coverage: &[TeamRef]) -> Vec<TeamRef> {
    let filtered: Vec<TeamRef> = catalog
        .optional_teams
        .iter()
        .filter(|team| !coverage.contains(team))
        .cloned()
        .collect();
    if !filtered.is_empty() {
        return filtered;
    }
    catalog
        .teams
        .iter()
        .filter(|team| !coverage.contains(team))
        .cloned()
        .collect()
}

fn normalize_readiness(query: &OwnerQuery) -> LiveReadiness {
    let head_from_pr = query.head_sha.clone();
    if let Some(given) = &query.readiness {
        let head_sha = given
            .head_sha
            .clone()
            .filter(|sha| !sha.is_empty())
            .or(head_from_pr);
        let usable = given.status == ReadinessWord::Blocked
            || (matches!(given.status, ReadinessWord::Ready | ReadinessWord::Unknown)
                && head_sha.is_some());
        if usable {
            return LiveReadiness {
                status: given.status,
                head_sha,
                blocking_reasons: given.blocking_reasons.clone(),
                unknown_reasons: given.unknown_reasons.clone(),
            };
        }
    }
    LiveReadiness {
        status: ReadinessWord::Unknown,
        head_sha: query.head_sha.clone(),
        blocking_reasons: Vec::new(),
        unknown_reasons: vec!["readiness evidence is missing; rerun resolve-pr.py".into()],
    }
}

fn approval_evidence(
    query: &OwnerQuery,
    candidates: &[TeamRef],
    readiness: &LiveReadiness,
    options: &DecideOptions<'_>,
) -> (
    BTreeMap<TeamRef, Vec<AttributedApproval>>,
    BTreeMap<TeamRef, Vec<String>>,
) {
    let mut attributed: BTreeMap<TeamRef, Vec<AttributedApproval>> = candidates
        .iter()
        .map(|team| (team.clone(), Vec::new()))
        .collect();
    let mut uncertain: BTreeMap<TeamRef, Vec<String>> = BTreeMap::new();
    if let Some(preset) = options.approval_attribution {
        for team in candidates {
            attributed.insert(team.clone(), preset.get(team).cloned().unwrap_or_default());
        }
        return (attributed, uncertain);
    }
    let approvals = human_approvals(query);
    let members = options.membership;
    let reviews = query.reviews.as_ref();
    let present = reviews.is_some();
    let complete = reviews.is_some_and(|bundle| bundle.fetched && bundle.complete);
    if !complete && readiness.status == ReadinessWord::Ready {
        for team in candidates {
            uncertain.insert(
                team.clone(),
                vec!["effective review evidence is missing".into()],
            );
        }
    } else if present && !complete {
        for team in candidates {
            uncertain.insert(
                team.clone(),
                vec!["complete effective review evidence is unavailable".into()],
            );
        }
    }
    for approval in &approvals {
        let mut direct = Vec::new();
        for team in &approval.on_behalf_of {
            if let Some(candidate) = candidates.iter().find(|candidate| {
                candidate.org.eq_ignore_ascii_case(&team.org)
                    && candidate.slug.eq_ignore_ascii_case(&team.slug)
            }) {
                direct.push(candidate.clone());
            }
        }
        let stored = AttributedApproval {
            reviewer: approval.author_login.clone(),
            review_id: approval.id.clone(),
            commit: approval.commit.clone(),
        };
        for team in candidates {
            if direct.contains(team) {
                attributed
                    .get_mut(team)
                    .expect("candidate")
                    .push(stored.clone());
                continue;
            }
            let info = members.and_then(|members| members.get(team));
            if !info.is_some_and(|info| info.complete) {
                uncertain.entry(team.clone()).or_default().push(format!(
                    "could not verify whether {} belongs to {team}",
                    approval.author_login
                ));
                continue;
            }
            let logins = info
                .map(|info| {
                    info.members
                        .iter()
                        .map(|login| login.to_ascii_lowercase())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if logins.contains(&approval.author_login.to_ascii_lowercase()) {
                attributed
                    .get_mut(team)
                    .expect("candidate")
                    .push(stored.clone());
            }
        }
    }
    if complete {
        uncertain.retain(|team, _| {
            attributed
                .get(team)
                .is_none_or(|reviews| reviews.is_empty())
        });
    }
    (attributed, uncertain)
}

fn human_approvals(query: &OwnerQuery) -> Vec<ReviewRecord> {
    let author = query.author.as_deref().unwrap_or("").to_ascii_lowercase();
    let Some(bundle) = &query.reviews else {
        return Vec::new();
    };
    bundle
        .reviews
        .iter()
        .filter(|review| {
            review.author_typename == "User"
                && !review.author_login.is_empty()
                && review.author_login.to_ascii_lowercase() != author
                && review.state.eq_ignore_ascii_case("APPROVED")
        })
        .cloned()
        .collect()
}

fn unique(teams: impl IntoIterator<Item = TeamRef>) -> Vec<TeamRef> {
    let mut out = Vec::new();
    for team in teams {
        if team.slug.is_empty() || out.contains(&team) {
            continue;
        }
        out.push(team);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team(slug: &str) -> TeamRef {
        TeamRef {
            org: "acme".into(),
            slug: slug.into(),
        }
    }

    fn catalog(slugs: &[&str]) -> OwnerCatalog {
        let teams: Vec<TeamRef> = slugs.iter().copied().map(team).collect();
        OwnerCatalog {
            source: Some("test".into()),
            codeowners_file: Some("CODEOWNERS".into()),
            exclusive_teams: vec![teams[0].clone()],
            coverage_teams: vec![teams[0].clone()],
            optional_teams: teams.iter().skip(1).cloned().collect(),
            teams,
            users: Vec::new(),
            path_owners: vec![("a.py".into(), vec![team("team-a")])],
            error: None,
        }
    }

    fn ready_query() -> OwnerQuery {
        OwnerQuery {
            readiness: Some(GivenReadiness {
                status: ReadinessWord::Ready,
                head_sha: Some("abc".into()),
                blocking_reasons: Vec::new(),
                unknown_reasons: Vec::new(),
            }),
            reviews: Some(ReviewBundle {
                fetched: true,
                complete: true,
                reviews: Vec::new(),
            }),
            protection_fetched: true,
            protection_present: true,
            require_code_owner_reviews: Some(true),
            ruleset_requires_code_owners: Some(false),
            ..OwnerQuery::default()
        }
    }

    fn slugs(teams: &[TeamRef]) -> Vec<&str> {
        teams.iter().map(|team| team.slug.as_str()).collect()
    }

    #[test]
    fn rules_cover_required_optional_unknown_override_and_blocked() {
        let decision = decide_owners(
            &ready_query(),
            &catalog(&["team-a"]),
            &DecideOptions::default(),
        );
        assert_eq!(decision.codeowners_required, Tri::True);
        assert_eq!(slugs(&decision.mandatory_teams), ["team-a"]);
        assert_eq!(slugs(&decision.post_to_teams), ["team-a"]);
        assert!(!decision.needs_user_choice);
        assert!(
            !decision
                .rationale
                .iter()
                .any(|line| line.contains("manual-team"))
        );

        let mut informational = ready_query();
        informational.requested_teams = vec!["manual-team".into()];
        let decision = decide_owners(
            &informational,
            &catalog(&["team-a"]),
            &DecideOptions::default(),
        );
        assert_eq!(decision.requested_teams_informational, ["manual-team"]);
        assert!(!decision.needs_user_choice);

        let mut not_required = ready_query();
        not_required.require_code_owner_reviews = Some(false);
        let decision = decide_owners(
            &not_required,
            &catalog(&["team-a"]),
            &DecideOptions::default(),
        );
        assert_eq!(decision.codeowners_required, Tri::False);
        assert_eq!(slugs(&decision.post_to_teams), ["team-a"]);
        assert!(!decision.needs_user_choice);

        let mut unknown = ready_query();
        unknown.protection_fetched = false;
        unknown.protection_present = false;
        unknown.require_code_owner_reviews = None;
        unknown.ruleset_requires_code_owners = None;
        let shared = catalog(&["team-a", "team-b"]);
        let decision = decide_owners(&unknown, &shared, &DecideOptions::default());
        assert_eq!(decision.codeowners_required, Tri::Unknown);
        assert!(decision.needs_user_choice);
        assert_eq!(slugs(&decision.routing_hints), ["team-a"]);
        assert!(decision.post_to_teams.is_empty());

        let picked = team("devx");
        let decision = decide_owners(
            &unknown,
            &catalog(&["team-a"]),
            &DecideOptions {
                user_team: Some(&picked),
                ..DecideOptions::default()
            },
        );
        assert_eq!(slugs(&decision.post_to_teams), ["devx"]);
        assert!(!decision.needs_user_choice);

        let mut blocked = ready_query();
        blocked.readiness = Some(GivenReadiness {
            status: ReadinessWord::Blocked,
            head_sha: Some("abc".into()),
            blocking_reasons: vec!["PR is a draft".into()],
            unknown_reasons: Vec::new(),
        });
        let decision = decide_owners(&blocked, &catalog(&["team-a"]), &DecideOptions::default());
        assert!(decision.post_to_teams.is_empty());
        assert!(!decision.no_op);
        assert!(!decision.confirmation_required);
    }

    #[test]
    fn uncovered_paths_stay_mandatory_and_approvals_leave_the_actionable_lists() {
        let mut wide = catalog(&["team-data", "team-buying", "team-analytics"]);
        wide.exclusive_teams = vec![team("team-data")];
        wide.coverage_teams = vec![
            team("team-data"),
            team("team-buying"),
            team("team-analytics"),
        ];
        wide.optional_teams.clear();
        let mut query = ready_query();
        query.require_code_owner_reviews = Some(false);
        let decision = decide_owners(&query, &wide, &DecideOptions::default());
        assert_eq!(
            slugs(&decision.mandatory_teams),
            ["team-data", "team-buying", "team-analytics"]
        );
        assert_eq!(
            slugs(&decision.post_to_teams),
            slugs(&decision.mandatory_teams)
        );
        assert!(!decision.needs_user_choice);

        let mut attributed = BTreeMap::new();
        attributed.insert(
            team("team-a"),
            vec![AttributedApproval {
                reviewer: "alice".into(),
                review_id: None,
                commit: Some("old".into()),
            }],
        );
        let decision = decide_owners(
            &ready_query(),
            &catalog(&["team-a", "team-b"]),
            &DecideOptions {
                approval_attribution: Some(&attributed),
                ..DecideOptions::default()
            },
        );
        assert_eq!(slugs(&decision.approved_teams), ["team-a"]);
        assert!(decision.mandatory_teams.is_empty());
        assert_eq!(slugs(&decision.optional_teams), ["team-b"]);
        assert!(decision.post_to_teams.is_empty());
        assert_eq!(slugs(&decision.ask_about_teams), ["team-b"]);
        assert!(!decision.no_op);

        let decision = decide_owners(
            &ready_query(),
            &catalog(&["team-a"]),
            &DecideOptions {
                approval_attribution: Some(&attributed),
                ..DecideOptions::default()
            },
        );
        assert!(decision.no_op);
        assert!(decision.post_to_teams.is_empty());
        assert_eq!(decision.skipped_teams[0].team.slug, "team-a");
        insta::assert_json_snapshot!("all_approved", decision);
    }

    #[test]
    fn membership_gaps_truncation_and_direct_approval() {
        let mut query = ready_query();
        query.author = Some("author".into());
        query.owner = Some("acme".into());
        query.reviews = Some(ReviewBundle {
            fetched: true,
            complete: true,
            reviews: vec![ReviewRecord {
                id: None,
                state: "APPROVED".into(),
                author_typename: "User".into(),
                author_login: "alice".into(),
                commit: None,
                on_behalf_of: Vec::new(),
            }],
        });
        let mut members = BTreeMap::new();
        members.insert(
            team("team-a"),
            Membership {
                complete: false,
                members: Vec::new(),
            },
        );
        let decision = decide_owners(
            &query,
            &catalog(&["team-a"]),
            &DecideOptions {
                membership: Some(&members),
                ..DecideOptions::default()
            },
        );
        assert!(!decision.no_op);
        assert!(decision.confirmation_required);
        assert!(
            decision
                .review_attribution_unknown
                .iter()
                .any(|gap| gap.team.slug == "team-a")
        );

        let mut members = BTreeMap::new();
        members.insert(
            team("team-a"),
            Membership {
                complete: true,
                members: Vec::new(),
            },
        );
        members.insert(
            team("team-b"),
            Membership {
                complete: true,
                members: vec!["alice".into()],
            },
        );
        query.reviews.as_mut().unwrap().reviews[0].on_behalf_of = vec![
            OnBehalfOf {
                org: "acme".into(),
                slug: "team-a".into(),
            },
            OnBehalfOf {
                org: "other".into(),
                slug: "team-x".into(),
            },
        ];
        let (attributed, unknown) = approval_evidence(
            &query,
            &[team("team-a"), team("team-b")],
            &normalize_readiness(&query),
            &DecideOptions {
                membership: Some(&members),
                ..DecideOptions::default()
            },
        );
        assert_eq!(attributed[&team("team-a")].len(), 1);
        assert_eq!(attributed[&team("team-b")].len(), 1);
        assert!(unknown.is_empty());

        query.reviews.as_mut().unwrap().reviews = vec![
            ReviewRecord {
                id: None,
                state: "APPROVED".into(),
                author_typename: "User".into(),
                author_login: "alice".into(),
                commit: None,
                on_behalf_of: vec![OnBehalfOf {
                    org: "acme".into(),
                    slug: "team-a".into(),
                }],
            },
            ReviewRecord {
                id: None,
                state: "APPROVED".into(),
                author_typename: "User".into(),
                author_login: "bob".into(),
                commit: None,
                on_behalf_of: Vec::new(),
            },
        ];
        let decision = decide_owners(&query, &catalog(&["team-a"]), &DecideOptions::default());
        assert!(decision.no_op);
        assert!(!decision.confirmation_required);
        assert!(decision.review_attribution_unknown.is_empty());

        let mut truncated = ready_query();
        truncated.files_truncated = true;
        let decision = decide_owners(&truncated, &catalog(&["team-a"]), &DecideOptions::default());
        assert!(decision.confirmation_required);
        assert!(decision.needs_user_choice);
        assert!(!decision.no_op);
        assert!(decision.post_to_teams.is_empty());
        assert!(
            decision
                .unknown_reasons
                .iter()
                .any(|reason| reason.contains("truncated"))
        );

        let mut preset = BTreeMap::new();
        preset.insert(
            team("team-a"),
            vec![AttributedApproval {
                reviewer: "alice".into(),
                review_id: None,
                commit: None,
            }],
        );
        let decision = decide_owners(
            &truncated,
            &catalog(&["team-a"]),
            &DecideOptions {
                approval_attribution: Some(&preset),
                ..DecideOptions::default()
            },
        );
        assert!(
            decision
                .ask_about_teams
                .iter()
                .all(|team| team.slug != "team-a")
        );
    }

    #[test]
    fn missing_evidence_error_and_optional_approval_follow_the_script() {
        let mut bare = ready_query();
        bare.reviews = None;
        let decision = decide_owners(&bare, &catalog(&["team-a"]), &DecideOptions::default());
        assert!(!decision.no_op);
        assert!(!decision.review_attribution_unknown.is_empty());
        assert!(decision.confirmation_required);

        let mut broken = catalog(&["team-a"]);
        broken.teams.clear();
        broken.exclusive_teams.clear();
        broken.coverage_teams.clear();
        broken.optional_teams.clear();
        broken.error = Some("CODEOWNERS unavailable".into());
        for status in [ReadinessWord::Ready, ReadinessWord::Unknown] {
            let mut query = ready_query();
            query.readiness.as_mut().unwrap().status = status;
            let decision = decide_owners(&query, &broken, &DecideOptions::default());
            assert!(decision.needs_user_choice);
            assert!(!decision.no_op);
        }

        let mut partial = ready_query();
        partial.owner = Some("acme".into());
        partial.reviews = Some(ReviewBundle {
            fetched: true,
            complete: false,
            reviews: vec![ReviewRecord {
                id: None,
                state: "APPROVED".into(),
                author_typename: "User".into(),
                author_login: "alice".into(),
                commit: None,
                on_behalf_of: vec![OnBehalfOf {
                    org: "acme".into(),
                    slug: "team-a".into(),
                }],
            }],
        });
        let decision = decide_owners(&partial, &catalog(&["team-a"]), &DecideOptions::default());
        assert!(!decision.no_op);
        assert!(decision.confirmation_required);

        let optional = catalog(&["team-a", "team-b"]);
        let mut attributed = BTreeMap::new();
        attributed.insert(team("team-a"), Vec::new());
        attributed.insert(
            team("team-b"),
            vec![AttributedApproval {
                reviewer: "alice".into(),
                review_id: None,
                commit: None,
            }],
        );
        let decision = decide_owners(
            &ready_query(),
            &optional,
            &DecideOptions {
                approval_attribution: Some(&attributed),
                ..DecideOptions::default()
            },
        );
        assert!(decision.optional_teams.is_empty());
        assert!(decision.ask_about_teams.is_empty());
        assert!(!decision.confirmation_required);
        assert!(!decision.needs_user_choice);

        let picked = team("team-a");
        let mut picked_approval = BTreeMap::new();
        picked_approval.insert(
            team("team-a"),
            vec![AttributedApproval {
                reviewer: "alice".into(),
                review_id: None,
                commit: None,
            }],
        );
        let decision = decide_owners(
            &ready_query(),
            &optional,
            &DecideOptions {
                user_team: Some(&picked),
                approval_attribution: Some(&picked_approval),
                ..DecideOptions::default()
            },
        );
        assert_eq!(slugs(&decision.candidate_teams), ["team-a"]);
        assert!(decision.no_op);

        let mut blocked = ready_query();
        blocked.readiness = Some(GivenReadiness {
            status: ReadinessWord::Blocked,
            head_sha: None,
            blocking_reasons: vec!["PR is a draft".into(), "check CI is fail".into()],
            unknown_reasons: Vec::new(),
        });
        let decision = decide_owners(&blocked, &catalog(&["team-a"]), &DecideOptions::default());
        assert_eq!(decision.readiness_status, "blocked");
        assert!(
            decision
                .blocking_reasons
                .iter()
                .any(|reason| reason == "check CI is fail")
        );
        assert!(!decision.confirmation_required);
        assert!(decision.post_to_teams.is_empty());
    }
}
