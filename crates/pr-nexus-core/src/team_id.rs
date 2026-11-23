//! GitHub team identifiers used in list filters and view rules.
//!
//! Pending reviewers are stored as `org/slug`. Older saved filters and
//! typed queries sometimes have only the slug or the team name. A bare
//! value matches every team with that slug; a full `org/slug` matches one.

/// Lowercased `org/slug` when `value` has both parts, otherwise `None`.
pub fn team_id(value: &str) -> Option<(String, String)> {
    let trimmed = value.trim().trim_start_matches('@');
    let (org, slug) = trimmed.split_once('/')?;
    if org.is_empty() || slug.is_empty() || slug.contains('/') {
        return None;
    }
    if !plain_login(org) || !plain_login(slug) {
        return None;
    }
    Some((org.to_ascii_lowercase(), slug.to_ascii_lowercase()))
}

/// The slug part of `org/slug`, or the whole trimmed value when there is no org.
pub fn team_slug(value: &str) -> String {
    let trimmed = value.trim().trim_start_matches('@');
    match team_id(trimmed) {
        Some((_, slug)) => slug,
        None => trimmed.to_ascii_lowercase(),
    }
}

/// Whether a pending reviewer login (`org/slug`) satisfies a filter value.
///
/// A full `org/slug` matches that team only. A bare slug matches any team
/// with that slug, so a saved filter written before orgs were part of the
/// key still works. If two orgs share the slug, both match.
pub fn pending_team_matches(have: &str, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return false;
    }
    if have.eq_ignore_ascii_case(filter) {
        return true;
    }
    match (team_id(have), team_id(filter)) {
        (Some(have), Some(want)) => have == want,
        (Some((_, have_slug)), None) => have_slug == team_slug(filter),
        (None, Some(_)) => false,
        (None, None) => have.eq_ignore_ascii_case(filter),
    }
}

/// Whether a pending reviewer matches a filter, also accepting a display name
/// when the filter is not an `org/slug`.
pub fn pending_team_matches_named(have_login: &str, have_name: Option<&str>, filter: &str) -> bool {
    if pending_team_matches(have_login, filter) {
        return true;
    }
    if team_id(filter).is_some() {
        return false;
    }
    have_name.is_some_and(|name| name.trim().eq_ignore_ascii_case(filter.trim()))
}

/// Turns saved filter values into `org/slug` logins from `known`.
///
/// A bare slug or name that matches one known team becomes that login. One
/// that matches several becomes every matching login, so the filter keeps
/// covering both orgs. A value with no match is kept as-is.
pub fn expand_team_filters(values: &[String], known: &[(String, Option<String>)]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let matches: Vec<&str> = known
            .iter()
            .filter(|(login, name)| pending_team_matches_named(login, name.as_deref(), value))
            .map(|(login, _)| login.as_str())
            .collect();
        if matches.is_empty() {
            push_unique(&mut out, value);
        } else {
            for login in matches {
                push_unique(&mut out, login);
            }
        }
    }
    out
}

fn push_unique(out: &mut Vec<String>, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if out.iter().any(|have| have.eq_ignore_ascii_case(trimmed)) {
        return;
    }
    out.push(trimmed.to_owned());
}

fn plain_login(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_slug_matches_only_that_team() {
        assert!(pending_team_matches("acme/platform", "acme/platform"));
        assert!(pending_team_matches("Acme/Platform", "acme/platform"));
        assert!(!pending_team_matches("acme-labs/platform", "acme/platform"));
        assert!(!pending_team_matches("acme/platform", "acme/payments"));
    }

    #[test]
    fn a_bare_slug_matches_every_org_with_that_slug() {
        assert!(pending_team_matches("acme/platform", "platform"));
        assert!(pending_team_matches("acme-labs/platform", "Platform"));
        assert!(!pending_team_matches("acme/web-platform", "platform"));
        assert!(!pending_team_matches("acme/platform", "payments"));
    }

    #[test]
    fn a_bare_name_matches_when_the_slug_does_not() {
        assert!(pending_team_matches_named(
            "acme/web-platform",
            Some("Web platform"),
            "Web platform"
        ));
        assert!(!pending_team_matches_named(
            "acme/web-platform",
            Some("Web platform"),
            "acme-labs/web-platform"
        ));
    }

    #[test]
    fn expand_rewrites_an_unambiguous_slug_and_keeps_both_when_shared() {
        let known = vec![
            ("acme/platform".into(), Some("Platform".into())),
            ("acme-labs/platform".into(), Some("Platform".into())),
            ("acme/payments".into(), Some("Payments".into())),
        ];
        assert_eq!(
            expand_team_filters(&["payments".into()], &known),
            vec!["acme/payments"]
        );
        assert_eq!(
            expand_team_filters(&["platform".into()], &known),
            vec!["acme/platform", "acme-labs/platform"]
        );
        assert_eq!(
            expand_team_filters(&["acme/platform".into()], &known),
            vec!["acme/platform"]
        );
        assert_eq!(
            expand_team_filters(&["missing".into()], &known),
            vec!["missing"]
        );
    }
}
