//! A GitHub pull request named in the list search field.
//!
//! `app/src/list/searchText.ts` (`parsePrLink`) accepts the same strings.
//! Owner and repository are matched without case; the number is exact.

/// `owner/name` and a pull request number, parsed from a URL or `owner/name#n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrLink {
    pub owner: String,
    pub name: String,
    pub number: u64,
}

impl PrLink {
    /// `repo` is `owner/name` as the list stores it, or a bare name.
    pub fn matches(&self, owner: &str, repo: &str, number: u64) -> bool {
        if self.number != number {
            return false;
        }
        let (repo_owner, repo_name) = repo
            .rsplit_once('/')
            .map_or(("", repo), |(owner, name)| (owner, name));
        let owner_ok =
            owner.eq_ignore_ascii_case(&self.owner) || repo_owner.eq_ignore_ascii_case(&self.owner);
        owner_ok && repo_name.eq_ignore_ascii_case(&self.name)
    }
}

/// A GitHub pull request URL, or `owner/name#n`.
///
/// The URL may omit the scheme, may start with `www.`, and may continue
/// with a path (`/files`, `/commits`), a query string, or a `#fragment`.
/// Anything else is `None`, including an issue URL and a link mixed with
/// other words.
pub fn parse(text: &str) -> Option<PrLink> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    if looks_like_url(text) {
        return parse_url(text);
    }
    parse_shorthand(text)
}

fn looks_like_url(text: &str) -> bool {
    strip_scheme(text)
        .split(['/', '?', '#'])
        .next()
        .is_some_and(|host| {
            let host = strip_prefix_ci(host, "www.").unwrap_or(host);
            host.eq_ignore_ascii_case("github.com")
        })
}

fn parse_shorthand(text: &str) -> Option<PrLink> {
    let (repo, number) = text.split_once('#')?;
    if number.contains('#') {
        return None;
    }
    let (owner, name) = repo.split_once('/')?;
    if name.contains('/') || !is_login(owner) || !is_login(name) {
        return None;
    }
    Some(PrLink {
        owner: owner.to_owned(),
        name: name.to_owned(),
        number: parse_number(number)?,
    })
}

fn parse_url(text: &str) -> Option<PrLink> {
    let rest = strip_scheme(text);
    let rest = strip_prefix_ci(rest, "www.").unwrap_or(rest);
    let path = strip_prefix_ci(rest, "github.com")?;
    if !path.starts_with('/') {
        return None;
    }
    let mut parts = path.split(['/', '?', '#']).filter(|part| !part.is_empty());
    let owner = parts.next()?;
    let name = parts.next()?;
    if !parts.next()?.eq_ignore_ascii_case("pull") {
        return None;
    }
    let number = parse_number(parts.next()?)?;
    if !is_login(owner) || !is_login(name) {
        return None;
    }
    Some(PrLink {
        owner: owner.to_owned(),
        name: name.to_owned(),
        number,
    })
}

fn strip_scheme(text: &str) -> &str {
    strip_prefix_ci(text, "https://")
        .or_else(|| strip_prefix_ci(text, "http://"))
        .unwrap_or(text)
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let end = prefix.len();
    if text.len() >= end && text.get(..end)?.eq_ignore_ascii_case(prefix) {
        Some(&text[end..])
    } else {
        None
    }
}

fn parse_number(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = value.parse().ok()?;
    (number > 0).then_some(number)
}

fn is_login(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && value.len() <= 100
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_the_shorthand_name_a_pull_request() {
        let cases = [
            "https://github.com/acme/widgets/pull/332",
            "http://github.com/acme/widgets/pull/332",
            "https://www.github.com/acme/widgets/pull/332",
            "github.com/acme/widgets/pull/332",
            "www.github.com/acme/widgets/pull/332",
            "HTTPS://GitHub.com/Acme/Widgets/pull/332",
            "https://github.com/acme/widgets/pull/332/",
            "https://github.com/acme/widgets/pull/332/files",
            "https://github.com/acme/widgets/pull/332/commits",
            "https://github.com/acme/widgets/pull/332?w=1",
            "https://github.com/acme/widgets/pull/332#discussion",
            "https://github.com/acme/widgets/pull/332/files?w=1#diff",
            "  https://github.com/acme/widgets/pull/332  ",
        ];
        for text in cases {
            let link = parse(text).unwrap_or_else(|| panic!("parse {text}"));
            assert_eq!(link.number, 332, "{text}");
            assert!(link.owner.eq_ignore_ascii_case("acme"), "{text}");
            assert!(link.name.eq_ignore_ascii_case("widgets"), "{text}");
        }
        assert_eq!(
            parse("acme/widgets#332"),
            Some(PrLink {
                owner: "acme".into(),
                name: "widgets".into(),
                number: 332,
            })
        );
        assert_eq!(
            parse("ACME/Widgets#4"),
            Some(PrLink {
                owner: "ACME".into(),
                name: "Widgets".into(),
                number: 4,
            })
        );
    }

    #[test]
    fn other_text_is_not_a_pull_request_link() {
        for text in [
            "https://github.com/acme/widgets/issues/332",
            "https://gitlab.com/acme/widgets/pull/332",
            "https://github.com/acme/widgets/pull/332 extra",
            "acme/widgets/pull/332",
            "acme/widgets#332/files",
            "acme/widgets#",
            "acme/widgets#0",
            "#332",
            "not a pr",
            "",
        ] {
            assert!(parse(text).is_none(), "{text} parsed");
        }
    }

    #[test]
    fn matching_ignores_owner_case_and_requires_the_number() {
        let link = parse("HTTPS://GitHub.com/Acme/Widgets/pull/332").unwrap();
        assert!(link.matches("acme", "acme/widgets", 332));
        assert!(link.matches("ACME", "ACME/Widgets", 332));
        assert!(!link.matches("acme", "acme/widgets", 333));
        assert!(!link.matches("acme-labs", "acme-labs/widgets", 332));
    }
}
