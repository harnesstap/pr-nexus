//! Required CODEOWNERS groups for a Slack review request.
//!
//! GitHub needs one approval per matched line. Any owner on that line satisfies
//! it. A Slack request therefore proposes one team per distinct owner set that
//! still needs an approval, not every team on every line.

use serde::Serialize;

use crate::domain::TeamRef;
use crate::review::codeowners::{Owner, Rule, last_matching_rule};

/// One distinct CODEOWNERS owner set on the changed files.
///
/// `teams` and `users` keep the order of the first line that produced the set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OwnerGroup {
    /// Stable identity of the owner set. Order and case do not change it.
    pub id: String,
    /// Pattern of the first line that produced this set.
    pub pattern: String,
    pub teams: Vec<TeamRef>,
    pub users: Vec<String>,
}

/// A team the user picked for a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupTeamChoice {
    pub group_id: String,
    pub team: TeamRef,
}

/// A matched group plus whether an approval already covers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarkedGroup {
    pub group: OwnerGroup,
    pub satisfied: bool,
}

/// What to do with one group that still needs an approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPlan {
    pub group: OwnerGroup,
    /// The team this request would ask. `None` when nothing was chosen.
    pub team: Option<TeamRef>,
    /// No chosen team has a Slack channel. The other plans can still send.
    pub unmapped: bool,
}

/// Last-match owner sets for `paths`, one per distinct owner set.
///
/// A line with no owners clears the path and adds no group. Two lines with the
/// same teams and users, in any order, are one group.
pub fn matched_groups(rules: &[Rule], paths: &[impl AsRef<str>]) -> Vec<OwnerGroup> {
    let mut groups = Vec::new();
    for path in paths {
        let Some(rule) = last_matching_rule(rules, path.as_ref()) else {
            continue;
        };
        let (teams, users) = split_owners(&rule.owners);
        if teams.is_empty() && users.is_empty() {
            continue;
        }
        let id = group_id(&teams, &users);
        if groups.iter().any(|group: &OwnerGroup| group.id == id) {
            continue;
        }
        groups.push(OwnerGroup {
            id,
            pattern: rule.pattern.clone(),
            teams,
            users,
        });
    }
    groups
}

/// Groups that still need an approval.
///
/// A team approval or an approval from a user named on the line satisfies the
/// whole group.
pub fn pending_groups(
    groups: &[OwnerGroup],
    approved_teams: &[TeamRef],
    approved_users: &[&str],
) -> Vec<OwnerGroup> {
    mark_groups(groups, approved_teams, approved_users)
        .into_iter()
        .filter(|group| !group.satisfied)
        .map(|group| group.group)
        .collect()
}

/// Every matched group, flagged when an approval already covers it.
pub fn mark_groups(
    groups: &[OwnerGroup],
    approved_teams: &[TeamRef],
    approved_users: &[&str],
) -> Vec<MarkedGroup> {
    groups
        .iter()
        .map(|group| MarkedGroup {
            satisfied: group_satisfied(group, approved_teams, approved_users),
            group: group.clone(),
        })
        .collect()
}

/// One request per group that is not already covered by a selected team.
///
/// `has_mapping` is true when that team has a Slack channel id. The default
/// team is the first mapped team on the line. `choices` replaces that pick
/// when the team is on the line. A group with no mapped team is left unmapped
/// so the caller can say so and still send the others. A later mapped team
/// that also sits on an earlier unmapped group covers that group.
pub fn plan_group_requests(
    groups: &[OwnerGroup],
    has_mapping: impl Fn(&TeamRef) -> bool,
    choices: &[GroupTeamChoice],
) -> Vec<GroupPlan> {
    let explicit = |group: &OwnerGroup| {
        choices.iter().find_map(|choice| {
            (choice.group_id == group.id && group_has_team(group, &choice.team))
                .then(|| choice.team.clone())
        })
    };
    let mut order: Vec<usize> = (0..groups.len())
        .filter(|index| explicit(&groups[*index]).is_some())
        .collect();
    order.extend((0..groups.len()).filter(|index| explicit(&groups[*index]).is_none()));

    let mut selected: Vec<TeamRef> = Vec::new();
    let mut plans: Vec<Option<GroupPlan>> = vec![None; groups.len()];
    for index in order {
        let group = &groups[index];
        if covered(group, &selected) {
            continue;
        }
        if let Some(team) = explicit(group) {
            if has_mapping(&team) {
                selected.push(team.clone());
                plans[index] = Some(GroupPlan {
                    group: group.clone(),
                    team: Some(team),
                    unmapped: false,
                });
            } else {
                plans[index] = Some(GroupPlan {
                    group: group.clone(),
                    team: Some(team),
                    unmapped: true,
                });
            }
            continue;
        }
        if let Some(team) = group.teams.iter().find(|team| has_mapping(team)).cloned() {
            selected.push(team.clone());
            plans[index] = Some(GroupPlan {
                group: group.clone(),
                team: Some(team),
                unmapped: false,
            });
        } else {
            plans[index] = Some(GroupPlan {
                group: group.clone(),
                team: None,
                unmapped: true,
            });
        }
    }
    for index in 0..groups.len() {
        let drop_hint = plans[index]
            .as_ref()
            .is_some_and(|plan| plan.unmapped && covered(&groups[index], &selected));
        if drop_hint {
            plans[index] = None;
        }
    }
    plans.into_iter().flatten().collect()
}

/// "No Slack channel is saved for @acme/team-a or @acme/team-b."
pub fn unmapped_hint(teams: &[TeamRef], users: &[&str]) -> String {
    let names: Vec<String> = if teams.is_empty() {
        users.iter().map(|user| format!("@{user}")).collect()
    } else {
        teams.iter().map(|team| format!("@{team}")).collect()
    };
    format!("No Slack channel is saved for {}.", english_or(&names))
}

fn group_satisfied(
    group: &OwnerGroup,
    approved_teams: &[TeamRef],
    approved_users: &[&str],
) -> bool {
    group.teams.iter().any(|team| {
        approved_teams
            .iter()
            .any(|approved| same_team(team, approved))
    }) || group.users.iter().any(|user| {
        approved_users
            .iter()
            .any(|approved| approved.eq_ignore_ascii_case(user))
    })
}

fn covered(group: &OwnerGroup, selected: &[TeamRef]) -> bool {
    group
        .teams
        .iter()
        .any(|team| selected.iter().any(|chosen| same_team(team, chosen)))
}

fn group_has_team(group: &OwnerGroup, team: &TeamRef) -> bool {
    group.teams.iter().any(|owner| same_team(owner, team))
}

fn same_team(left: &TeamRef, right: &TeamRef) -> bool {
    left.org.eq_ignore_ascii_case(&right.org) && left.slug.eq_ignore_ascii_case(&right.slug)
}

fn split_owners(owners: &[Owner]) -> (Vec<TeamRef>, Vec<String>) {
    let mut teams = Vec::new();
    let mut users = Vec::new();
    for owner in owners {
        match owner {
            Owner::Team(team) => {
                if !teams.iter().any(|existing| same_team(existing, team)) {
                    teams.push(team.clone());
                }
            }
            Owner::User(login) => {
                if !users
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(login))
                {
                    users.push(login.clone());
                }
            }
        }
    }
    (teams, users)
}

fn group_id(teams: &[TeamRef], users: &[String]) -> String {
    let mut team_keys: Vec<String> = teams
        .iter()
        .map(|team| {
            format!(
                "{}/{}",
                team.org.to_ascii_lowercase(),
                team.slug.to_ascii_lowercase()
            )
        })
        .collect();
    team_keys.sort();
    team_keys.dedup();
    let mut user_keys: Vec<String> = users.iter().map(|user| user.to_ascii_lowercase()).collect();
    user_keys.sort();
    user_keys.dedup();
    format!(
        "teams:{}|users:{}",
        team_keys.join(","),
        user_keys.join(",")
    )
}

fn english_or(names: &[String]) -> String {
    match names {
        [] => "this owner".to_owned(),
        [one] => one.clone(),
        [first, second] => format!("{first} or {second}"),
        many => {
            let last = many.last().expect("at least three names");
            let rest = &many[..many.len() - 1];
            format!("{}, or {last}", rest.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::codeowners::parse_codeowners;

    fn team(slug: &str) -> TeamRef {
        TeamRef {
            org: "acme".into(),
            slug: slug.into(),
        }
    }

    fn slugs(groups: &[OwnerGroup]) -> Vec<Vec<&str>> {
        groups
            .iter()
            .map(|group| group.teams.iter().map(|team| team.slug.as_str()).collect())
            .collect()
    }

    #[test]
    fn last_match_wins_and_identical_owner_sets_collapse() {
        let rules = parse_codeowners(
            "\
* @acme/team-a @acme/team-b
/src/a.rs @acme/team-a @acme/team-b
/src/b.rs @acme/team-b @acme/team-a
/src/c.rs @acme/team-c @acme/team-d
/src/old.rs @acme/team-a
/src/old.rs @acme/team-c @acme/team-d
/generated/out.rs
",
        )
        .unwrap();
        let groups = matched_groups(
            &rules,
            &[
                "README.md",
                "src/a.rs",
                "src/b.rs",
                "src/c.rs",
                "src/old.rs",
                "generated/out.rs",
            ],
        );
        assert_eq!(
            slugs(&groups),
            vec![vec!["team-a", "team-b"], vec!["team-c", "team-d"],]
        );
        assert_eq!(groups[0].pattern, "*");
        assert_eq!(groups[1].pattern, "/src/c.rs");
    }

    #[test]
    fn an_approval_drops_the_group_and_a_user_owner_counts() {
        let rules = parse_codeowners(
            "\
/src/a.rs @acme/team-a @acme/team-b
/src/b.rs @acme/team-c @octocat
/src/c.rs @acme/team-d
",
        )
        .unwrap();
        let groups = matched_groups(&rules, &["src/a.rs", "src/b.rs", "src/c.rs"]);
        let pending = pending_groups(&groups, &[team("team-a")], &["octocat"]);
        assert_eq!(slugs(&pending), vec![vec!["team-d"]]);
    }

    #[test]
    fn two_lines_propose_two_sends_and_a_shared_owner_covers_the_other_line() {
        let rules = parse_codeowners(
            "\
/src/a.rs @acme/team-a @acme/team-b
/src/b.rs @acme/team-c @acme/team-d
/src/c.rs @acme/team-a @acme/team-e
",
        )
        .unwrap();
        let groups = matched_groups(&rules, &["src/a.rs", "src/b.rs"]);
        let mapped = |team: &TeamRef| team.slug == "team-a" || team.slug == "team-c";
        let plans = plan_group_requests(&groups, mapped, &[]);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].team, Some(team("team-a")));
        assert_eq!(plans[1].team, Some(team("team-c")));
        assert!(plans.iter().all(|plan| !plan.unmapped));

        let overlapping = matched_groups(&rules, &["src/a.rs", "src/c.rs"]);
        let plans = plan_group_requests(&overlapping, mapped, &[]);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].team, Some(team("team-a")));
    }

    #[test]
    fn a_line_with_no_channel_is_a_hint_and_the_other_line_still_sends() {
        let rules = parse_codeowners(
            "\
/src/a.rs @acme/team-a @acme/team-b
/src/b.rs @acme/team-c @acme/team-d
",
        )
        .unwrap();
        let groups = matched_groups(&rules, &["src/a.rs", "src/b.rs"]);
        let mapped = |team: &TeamRef| team.slug == "team-b";
        let plans = plan_group_requests(&groups, mapped, &[]);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].team, Some(team("team-b")));
        assert!(!plans[0].unmapped);
        assert!(plans[1].unmapped);
        assert_eq!(plans[1].team, None);
        assert_eq!(
            unmapped_hint(&plans[1].group.teams, &[]),
            "No Slack channel is saved for @acme/team-c or @acme/team-d."
        );
    }

    #[test]
    fn switching_team_uncovers_a_line_the_previous_pick_had_covered() {
        let rules = parse_codeowners(
            "\
/src/a.rs @acme/team-a @acme/team-b
/src/c.rs @acme/team-a @acme/team-e
",
        )
        .unwrap();
        let groups = matched_groups(&rules, &["src/a.rs", "src/c.rs"]);
        let mapped = |team: &TeamRef| matches!(team.slug.as_str(), "team-a" | "team-b" | "team-e");
        let plans = plan_group_requests(
            &groups,
            mapped,
            &[GroupTeamChoice {
                group_id: groups[0].id.clone(),
                team: team("team-b"),
            }],
        );
        assert_eq!(
            plans
                .iter()
                .filter_map(|plan| plan.team.as_ref().map(|team| team.slug.as_str()))
                .collect::<Vec<_>>(),
            vec!["team-b", "team-a"]
        );
    }

    #[test]
    fn a_selected_team_covers_a_line_whose_own_pick_has_no_channel() {
        let rules = parse_codeowners(
            "\
/src/a.rs @acme/team-a @acme/team-b
/src/c.rs @acme/team-a @acme/team-e
",
        )
        .unwrap();
        let groups = matched_groups(&rules, &["src/a.rs", "src/c.rs"]);
        let mapped = |team: &TeamRef| team.slug == "team-a";
        let plans = plan_group_requests(
            &groups,
            mapped,
            &[GroupTeamChoice {
                group_id: groups[0].id.clone(),
                team: team("team-b"),
            }],
        );
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].team, Some(team("team-a")));
        assert!(!plans[0].unmapped);
    }
}
