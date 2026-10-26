//! CODEOWNERS parsing and last-match ownership.
//!
//! Pattern matching follows the gitignore-style rules in hmarr/codeowners
//! (`match.go`): a slash anchors the pattern at the repo root, a pattern with
//! no slash matches at any depth, `*` does not cross `/`, and a trailing `/`
//! matches the directory. A last segment that is only `*` does not match
//! nested files, so `docs/*` matches `docs/a.md` and not `docs/sub/a.md`.

use std::path::Path;

use regex::Regex;

use crate::domain::TeamRef;
use crate::error::CoreError;

/// GitHub lookup order. The first file that exists wins.
pub const CODEOWNERS_CANDIDATES: [&str; 3] =
    [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    Team(TeamRef),
    /// A user login or an email. Never routed to Slack.
    User(String),
}

#[derive(Debug)]
struct Pattern {
    left_anchored_literal: Option<String>,
    regex: Option<Regex>,
}

#[derive(Debug)]
pub struct Rule {
    pub pattern: String,
    pub owners: Vec<Owner>,
    matcher: Pattern,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ownership {
    pub path_owners: Vec<(String, Vec<TeamRef>)>,
    pub teams: Vec<TeamRef>,
    pub exclusive: Vec<TeamRef>,
    pub coverage: Vec<TeamRef>,
    pub optional: Vec<TeamRef>,
    pub users_ignored_for_slack: Vec<String>,
}

/// First existing CODEOWNERS path under `root`, in GitHub's order.
pub fn find_codeowners_file(root: &Path) -> Option<std::path::PathBuf> {
    CODEOWNERS_CANDIDATES.iter().find_map(|rel| {
        let path = root.join(rel);
        path.is_file().then_some(path)
    })
}

pub fn parse_codeowners(text: &str) -> Result<Vec<Rule>, CoreError> {
    let mut rules = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let Some(tokens) = tokenize_line(raw) else {
            continue;
        };
        if tokens.is_empty() {
            continue;
        }
        let pattern = &tokens[0];
        let owners = tokens[1..]
            .iter()
            .map(|token| parse_owner(token))
            .collect::<Vec<_>>();
        let matcher = compile_pattern(pattern).map_err(|err| {
            CoreError::Validation(format!("CODEOWNERS line {}: {err}", index + 1))
        })?;
        rules.push(Rule {
            pattern: pattern.clone(),
            owners,
            matcher,
        });
    }
    Ok(rules)
}

pub fn owners_for_path<'a>(rules: &'a [Rule], path: &str) -> &'a [Owner] {
    let normalized = path.trim_start_matches('/');
    let mut matched: &[Owner] = &[];
    for rule in rules {
        if rule.matcher.matches(normalized) {
            matched = &rule.owners;
        }
    }
    matched
}

pub fn team_owners_for_paths(rules: &[Rule], paths: &[impl AsRef<str>]) -> Vec<TeamRef> {
    let mut ordered = Vec::new();
    for path in paths {
        for owner in owners_for_path(rules, path.as_ref()) {
            if let Owner::Team(team) = owner
                && !ordered.contains(team)
            {
                ordered.push(team.clone());
            }
        }
    }
    ordered
}

pub fn resolve_ownership(rules: &[Rule], paths: &[impl AsRef<str>]) -> Ownership {
    let mut path_owners = Vec::new();
    let mut teams = Vec::new();
    let mut exclusive = Vec::new();
    let mut users = Vec::new();

    for path in paths {
        let path = path.as_ref();
        let owners = owners_for_path(rules, path);
        let mut team_names = Vec::new();
        for owner in owners {
            match owner {
                Owner::Team(team) => {
                    if !team_names.contains(team) {
                        team_names.push(team.clone());
                    }
                    if !teams.contains(team) {
                        teams.push(team.clone());
                    }
                }
                Owner::User(login) => {
                    if !users.contains(login) {
                        users.push(login.clone());
                    }
                }
            }
        }
        if team_names.len() == 1 && !exclusive.contains(&team_names[0]) {
            exclusive.push(team_names[0].clone());
        }
        path_owners.push((path.to_string(), team_names));
    }

    let mut coverage = exclusive.clone();
    for (_, team_names) in &path_owners {
        if team_names.is_empty() || team_names.iter().any(|team| exclusive.contains(team)) {
            continue;
        }
        for team in team_names {
            if !coverage.contains(team) {
                coverage.push(team.clone());
            }
        }
    }
    let optional = teams
        .iter()
        .filter(|team| !coverage.contains(team))
        .cloned()
        .collect();
    Ownership {
        path_owners,
        teams,
        exclusive,
        coverage,
        optional,
        users_ignored_for_slack: users,
    }
}

fn parse_owner(token: &str) -> Owner {
    if let Some(rest) = token.strip_prefix('@') {
        if let Some((org, slug)) = rest.split_once('/') {
            return Owner::Team(TeamRef {
                org: org.to_string(),
                slug: slug.to_string(),
            });
        }
        return Owner::User(rest.to_string());
    }
    Owner::User(token.to_string())
}

/// Drop blank lines and comments. ` #` starts a comment. `\#` and `\ ` stay literal.
fn tokenize_line(raw: &str) -> Option<Vec<String>> {
    if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
        return None;
    }
    let chars: Vec<char> = raw.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\\' && index + 1 < chars.len() {
            let next = chars[index + 1];
            if next == '#' || next == ' ' {
                current.push(next);
            } else {
                current.push('\\');
                current.push(next);
            }
            index += 2;
            continue;
        }
        if ch == '#' && current.is_empty() {
            break;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            index += 1;
            continue;
        }
        current.push(ch);
        index += 1;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Some(tokens)
}

fn compile_pattern(pattern: &str) -> Result<Pattern, String> {
    if pattern.contains("***") {
        return Err("pattern cannot contain three consecutive asterisks".into());
    }
    if pattern.is_empty() {
        return Err("empty pattern".into());
    }
    if !pattern.contains(['*', '?', '\\']) && pattern.starts_with('/') {
        return Ok(Pattern {
            left_anchored_literal: Some(pattern.to_string()),
            regex: None,
        });
    }
    let regex = build_pattern_regex(pattern)?;
    Ok(Pattern {
        left_anchored_literal: None,
        regex: Some(regex),
    })
}

impl Pattern {
    fn matches(&self, test_path: &str) -> bool {
        if let Some(pattern) = &self.left_anchored_literal {
            return literal_match(pattern, test_path);
        }
        self.regex
            .as_ref()
            .is_some_and(|regex| regex.is_match(test_path))
    }
}

fn literal_match(pattern: &str, test_path: &str) -> bool {
    let prefix = pattern.strip_prefix('/').unwrap_or(pattern);
    if prefix.is_empty() {
        return false;
    }
    if prefix.ends_with('/') {
        return test_path.starts_with(prefix);
    }
    if test_path.len() == prefix.len() {
        return test_path == prefix;
    }
    test_path.len() > prefix.len()
        && test_path.as_bytes().get(prefix.len()) == Some(&b'/')
        && test_path.starts_with(prefix)
}

fn build_pattern_regex(pattern: &str) -> Result<Regex, String> {
    if pattern == "/" {
        return Regex::new(r"\A\z").map_err(|err| err.to_string());
    }
    let mut segs: Vec<&str> = pattern.split('/').collect();
    if segs.first() == Some(&"") {
        segs.remove(0);
    } else if (segs.len() == 1 || (segs.len() == 2 && segs[1].is_empty()))
        && segs.first() != Some(&"**")
    {
        segs.insert(0, "**");
    }
    if segs.len() > 1 && segs.last() == Some(&"") {
        let last = segs.len() - 1;
        segs[last] = "**";
    }

    let mut re = String::from(r"\A");
    let last = segs.len() - 1;
    let mut need_slash = false;
    for (index, seg) in segs.iter().enumerate() {
        match *seg {
            "**" => match (index == 0, index == last) {
                (true, true) => re.push_str(".+"),
                (true, false) => {
                    re.push_str(r"(?:.+/)?");
                    need_slash = false;
                }
                (false, true) => re.push_str(r"/.*"),
                (false, false) => {
                    re.push_str(r"(?:/.+)?");
                    need_slash = true;
                }
            },
            "*" => {
                if need_slash {
                    re.push('/');
                }
                re.push_str("[^/]+");
                need_slash = true;
            }
            _ => {
                if need_slash {
                    re.push('/');
                }
                push_segment(&mut re, seg);
                if index == last {
                    re.push_str(r"(?:/.*)?");
                }
                need_slash = true;
            }
        }
    }
    re.push_str(r"\z");
    Regex::new(&re).map_err(|err| err.to_string())
}

fn push_segment(re: &mut String, seg: &str) {
    let mut escape = false;
    for ch in seg.chars() {
        if escape {
            re.push_str(&regex::escape(&ch.to_string()));
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '[' | ']' => {
                re.push('\\');
                re.push(ch);
            }
            _ => re.push_str(&regex::escape(&ch.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team(org: &str, slug: &str) -> TeamRef {
        TeamRef {
            org: org.into(),
            slug: slug.into(),
        }
    }

    fn team_names(owners: &[Owner]) -> Vec<String> {
        owners
            .iter()
            .map(|owner| match owner {
                Owner::Team(team) => format!("{}/{}", team.org, team.slug),
                Owner::User(login) => login.clone(),
            })
            .collect()
    }

    #[test]
    fn last_match_wins_and_keeps_the_team_org() {
        let rules = parse_codeowners(
            "\
/services/gateway                                       @acme/team-data
/services/*/cloud/iam                                   @other/team-infra
",
        )
        .unwrap();
        let owners = owners_for_path(&rules, "services/gateway/cloud/iam/gateway-oidc.tf");
        assert_eq!(team_names(owners), ["other/team-infra"]);
        let teams = team_owners_for_paths(
            &rules,
            &[
                "services/gateway/cloud/router/main.tf",
                "services/gateway/cloud/iam/gateway-oidc.tf",
            ],
        );
        assert_eq!(
            teams,
            vec![team("acme", "team-data"), team("other", "team-infra")]
        );
    }

    #[test]
    fn docs_star_does_not_match_nested_files() {
        let rules = parse_codeowners("docs/* @acme/team-docs\n").unwrap();
        assert_eq!(
            team_names(owners_for_path(&rules, "docs/a.md")),
            ["acme/team-docs"]
        );
        assert!(owners_for_path(&rules, "docs/sub/a.md").is_empty());
        assert!(owners_for_path(&rules, "other/docs/a.md").is_empty());
    }

    #[test]
    fn unanchored_pattern_matches_any_depth_and_trailing_slash_is_a_directory() {
        let rules = parse_codeowners("*.md @acme/team-docs\n/users/ @acme/team-data\n").unwrap();
        assert_eq!(
            team_names(owners_for_path(&rules, "dir/sub/readme.md")),
            ["acme/team-docs"]
        );
        assert_eq!(
            team_names(owners_for_path(&rules, "users/accounts.rs")),
            ["acme/team-data"]
        );
        assert!(owners_for_path(&rules, "other/users/accounts.rs").is_empty());
    }

    #[test]
    fn empty_owner_list_resets_and_inline_comments_are_stripped() {
        let rules = parse_codeowners(
            "\
* @acme/team-a
docs/generated
/src/app @acme/team-b # primary owners
/src/hash\\#file @acme/team-c
",
        )
        .unwrap();
        assert!(owners_for_path(&rules, "docs/generated/out.txt").is_empty());
        assert_eq!(
            team_names(owners_for_path(&rules, "src/app/main.rs")),
            ["acme/team-b"]
        );
        assert_eq!(
            team_names(owners_for_path(&rules, "src/hash#file")),
            ["acme/team-c"]
        );
        assert_eq!(
            team_names(owners_for_path(&rules, "README.md")),
            ["acme/team-a"]
        );
    }

    #[test]
    fn users_and_emails_are_not_teams() {
        let rules = parse_codeowners(
            "\
/src/user-only @alice
/src/mixed @acme/team-a @bob alice@acme.test
",
        )
        .unwrap();
        let ownership = resolve_ownership(&rules, &["src/user-only/a.py", "src/mixed/b.py"]);
        assert_eq!(ownership.path_owners[0].1, Vec::<TeamRef>::new());
        assert_eq!(ownership.path_owners[1].1, vec![team("acme", "team-a")]);
        assert_eq!(ownership.teams, vec![team("acme", "team-a")]);
        assert_eq!(ownership.exclusive, vec![team("acme", "team-a")]);
        assert_eq!(
            ownership.users_ignored_for_slack,
            ["alice", "bob", "alice@acme.test"]
        );
    }

    #[test]
    fn partition_matches_the_ported_cases() {
        let exclusive = parse_codeowners(
            "\
/src/exclusive @acme/team-a
/src/shared @acme/team-a @acme/team-b
",
        )
        .unwrap();
        let ownership =
            resolve_ownership(&exclusive, &["src/exclusive/file.py", "src/shared/file.py"]);
        assert_eq!(ownership.path_owners[0].1, vec![team("acme", "team-a")]);
        assert_eq!(
            ownership.path_owners[1].1,
            vec![team("acme", "team-a"), team("acme", "team-b")]
        );
        assert_eq!(ownership.exclusive, vec![team("acme", "team-a")]);
        assert_eq!(ownership.coverage, vec![team("acme", "team-a")]);
        assert_eq!(ownership.optional, vec![team("acme", "team-b")]);

        let uncovered = parse_codeowners(
            "\
* @acme/team-billing @acme/team-analytics
/users/ @acme/team-data
",
        )
        .unwrap();
        let ownership = resolve_ownership(&uncovered, &["users/accounts.rs", "models/bucket.rs"]);
        assert_eq!(ownership.exclusive, vec![team("acme", "team-data")]);
        assert_eq!(
            ownership.coverage,
            vec![
                team("acme", "team-data"),
                team("acme", "team-billing"),
                team("acme", "team-analytics"),
            ]
        );
        assert!(ownership.optional.is_empty());
    }

    #[test]
    fn find_codeowners_prefers_the_github_directory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".github")).unwrap();
        std::fs::write(root.path().join(".github/CODEOWNERS"), "* @acme/team-a\n").unwrap();
        std::fs::write(root.path().join("CODEOWNERS"), "* @acme/team-b\n").unwrap();
        let found = find_codeowners_file(root.path()).unwrap();
        assert_eq!(found, root.path().join(".github/CODEOWNERS"));
    }

    #[test]
    fn reference_matcher_agrees_on_generated_patterns() {
        use proptest::prelude::*;
        proptest!(|(case in pattern_case())| {
            let compiled = compile_pattern(&case.pattern).unwrap();
            let actual = compiled.matches(&case.path);
            let expected = reference_match(&case.pattern, &case.path);
            prop_assert_eq!(actual, expected, "pattern {} path {}", case.pattern, case.path);
        });
    }

    #[derive(Debug, Clone)]
    struct PatternCase {
        pattern: String,
        path: String,
    }

    fn pattern_case() -> impl proptest::strategy::Strategy<Value = PatternCase> {
        use proptest::prelude::*;
        let seg = prop_oneof![
            Just("a".to_string()),
            Just("ab".to_string()),
            Just("src".to_string()),
            Just("*".to_string()),
            Just("**".to_string()),
        ];
        (
            prop::collection::vec(seg, 1..4),
            any::<bool>(),
            prop::collection::vec(Just("file".to_string()), 1..5),
        )
            .prop_map(|(mut segs, leading, path_segs)| {
                if segs.iter().filter(|seg| seg.as_str() == "**").count() > 1 {
                    segs.retain(|seg| seg != "**");
                    segs.push("**".into());
                }
                if segs.is_empty() {
                    segs.push("a".into());
                }
                let mut pattern = segs.join("/");
                if leading {
                    pattern.insert(0, '/');
                }
                PatternCase {
                    pattern,
                    path: path_segs.join("/"),
                }
            })
    }

    fn reference_match(pattern: &str, path: &str) -> bool {
        let path = path.trim_start_matches('/');
        if pattern.contains("***") || pattern.is_empty() {
            return false;
        }
        if !pattern.contains(['*', '?', '\\']) && pattern.starts_with('/') {
            return literal_match(pattern, path);
        }
        let mut segs: Vec<&str> = pattern.split('/').collect();
        if segs.first() == Some(&"") {
            segs.remove(0);
        } else if (segs.len() == 1 || (segs.len() == 2 && segs[1].is_empty()))
            && segs.first() != Some(&"**")
        {
            segs.insert(0, "**");
        }
        if segs.len() > 1 && segs.last() == Some(&"") {
            let last = segs.len() - 1;
            segs[last] = "**";
        }
        let path_segs: Vec<&str> = if path.is_empty() {
            Vec::new()
        } else {
            path.split('/').collect()
        };
        match_from(&segs, &path_segs)
    }

    /// Backtracking matcher for whole-segment `*`, `**`, and literals.
    /// A final literal matches descendants. A final `*` does not.
    fn match_from(pat: &[&str], path: &[&str]) -> bool {
        if pat.is_empty() {
            return path.is_empty();
        }
        match pat[0] {
            "**" => {
                if pat.len() == 1 {
                    return !path.is_empty();
                }
                if match_from(&pat[1..], path) {
                    return true;
                }
                if path.is_empty() {
                    return false;
                }
                match_from(pat, &path[1..])
            }
            "*" => {
                if path.len() != 1 && pat.len() == 1 {
                    return false;
                }
                if path.is_empty() {
                    return false;
                }
                if pat.len() == 1 {
                    return true;
                }
                match_from(&pat[1..], &path[1..])
            }
            literal => {
                if path.is_empty() || path[0] != literal {
                    return false;
                }
                if pat.len() == 1 {
                    return true;
                }
                match_from(&pat[1..], &path[1..])
            }
        }
    }
}
