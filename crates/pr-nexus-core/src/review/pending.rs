//! Rank code-owner teams when suggesting a Slack review channel.
//!
//! A team whose review is already covered should not be the default pick while
//! another owner team is still pending. If every team is covered, or none are,
//! the original order is kept.

/// One CODEOWNERS owner set, for Slack ranking only.
#[derive(Debug, Clone, Copy)]
pub struct GroupStatus<'a> {
    pub teams: &'a [String],
    pub satisfied: bool,
}

/// Signals that a suggested team already had its review covered.
#[derive(Debug, Clone, Copy)]
pub struct OwnerSlackEvidence<'a> {
    pub approved_teams: &'a [String],
    pub requested_teams: &'a [String],
    pub groups: &'a [GroupStatus<'a>],
}

/// `org/slug`, lowercased, without a leading `@`.
pub fn team_key(name: &str) -> String {
    name.trim().trim_start_matches('@').to_ascii_lowercase()
}

/// Whether `team` should be treated as already covered for a Slack suggestion.
pub fn team_already_covered(
    team: &str,
    among: &[String],
    evidence: &OwnerSlackEvidence<'_>,
) -> bool {
    let key = team_key(team);
    if evidence
        .approved_teams
        .iter()
        .any(|other| team_key(other) == key)
    {
        return true;
    }
    let groups: Vec<&GroupStatus<'_>> = evidence
        .groups
        .iter()
        .filter(|group| group.teams.iter().any(|other| team_key(other) == key))
        .collect();
    if !groups.is_empty() && groups.iter().all(|group| group.satisfied) {
        return true;
    }
    let requested: Vec<String> = evidence
        .requested_teams
        .iter()
        .map(|other| team_key(other))
        .collect();
    let still_waiting = among
        .iter()
        .any(|other| requested.iter().any(|want| want == &team_key(other)));
    still_waiting && !requested.iter().any(|want| want == &key)
}

/// Pending teams first, then already-covered teams, preserving order in each
/// bucket. Returns `teams` unchanged when every team is pending or every team
/// is already covered.
pub fn prefer_pending_owner_teams(
    teams: &[String],
    evidence: &OwnerSlackEvidence<'_>,
) -> Vec<String> {
    if teams.len() <= 1 {
        return teams.to_vec();
    }
    let mut pending = Vec::new();
    let mut covered = Vec::new();
    for team in teams {
        if team_already_covered(team, teams, evidence) {
            covered.push(team.clone());
        } else {
            pending.push(team.clone());
        }
    }
    if pending.is_empty() || covered.is_empty() {
        return teams.to_vec();
    }
    pending.extend(covered);
    pending
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence<'a>(
        approved: &'a [String],
        requested: &'a [String],
        groups: &'a [GroupStatus<'a>],
    ) -> OwnerSlackEvidence<'a> {
        OwnerSlackEvidence {
            approved_teams: approved,
            requested_teams: requested,
            groups,
        }
    }

    #[test]
    fn an_approved_team_sorts_after_a_pending_team() {
        let teams = vec!["acme/team-a".into(), "acme/team-b".into()];
        let approved = vec!["acme/team-a".into()];
        let ranked = prefer_pending_owner_teams(&teams, &evidence(&approved, &[], &[]));
        assert_eq!(ranked, ["acme/team-b", "acme/team-a"]);
    }

    #[test]
    fn a_team_no_longer_requested_sorts_after_one_still_requested() {
        let teams = vec!["@Acme/team-a".into(), "acme/team-b".into()];
        let requested = vec!["acme/team-b".into()];
        let ranked = prefer_pending_owner_teams(&teams, &evidence(&[], &requested, &[]));
        assert_eq!(ranked, ["acme/team-b", "@Acme/team-a"]);
    }

    #[test]
    fn a_satisfied_owner_group_sorts_its_team_after_a_pending_group() {
        let teams = vec!["acme/team-a".into(), "acme/team-b".into()];
        let a = vec!["@acme/team-a".into()];
        let b = vec!["acme/team-b".into()];
        let groups = [
            GroupStatus {
                teams: &a,
                satisfied: true,
            },
            GroupStatus {
                teams: &b,
                satisfied: false,
            },
        ];
        let ranked = prefer_pending_owner_teams(&teams, &evidence(&[], &[], &groups));
        assert_eq!(ranked, ["acme/team-b", "acme/team-a"]);
    }

    #[test]
    fn all_approved_keeps_the_original_order() {
        let teams = vec!["acme/team-a".into(), "acme/team-b".into()];
        let approved = vec!["@acme/team-a".into(), "acme/team-b".into()];
        let ranked = prefer_pending_owner_teams(&teams, &evidence(&approved, &[], &[]));
        assert_eq!(ranked, teams);
    }

    #[test]
    fn no_coverage_signal_keeps_the_original_order() {
        let teams = vec!["acme/team-a".into(), "acme/team-b".into()];
        let ranked = prefer_pending_owner_teams(&teams, &evidence(&[], &[], &[]));
        assert_eq!(ranked, teams);
    }

    #[test]
    fn a_single_team_is_unchanged() {
        let teams = vec!["acme/team-a".into()];
        let approved = vec!["acme/team-a".into()];
        let ranked = prefer_pending_owner_teams(&teams, &evidence(&approved, &[], &[]));
        assert_eq!(ranked, teams);
    }
}
