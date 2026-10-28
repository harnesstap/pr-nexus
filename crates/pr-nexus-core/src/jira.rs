//! JIRA references on projects: parse what the user typed into a key and a
//! URL, and build the link a bare key points to.
//!
//! `https://acme-labs.atlassian.net/browse/ACME-123` gives the key `ACME-123`
//! and keeps the URL. A bare `ACME-123` keeps an empty URL; it becomes a link
//! only once `jira.base_url` is set (see [`href`]).

use std::sync::LazyLock;

use regex::Regex;

use crate::error::CoreError;

static KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_]*-[0-9]+$").expect("static regex compiles"));
static KEY_IN_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:/browse/|[?&]selectedIssue=)([A-Za-z][A-Za-z0-9_]*-[0-9]+)\b")
        .expect("static regex compiles")
});

/// A key and a URL as stored; at least one is non-empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLink {
    pub key: String,
    pub url: String,
}

/// The issue key inside a JIRA URL, upper-cased, if there is one.
pub fn key_from_url(url: &str) -> Option<String> {
    KEY_IN_URL
        .captures(url)
        .map(|caps| caps[1].to_ascii_uppercase())
}

/// Whether `text` is a bare issue key such as `ACME-123`.
pub fn is_key(text: &str) -> bool {
    KEY.is_match(text.trim())
}

/// Parse one user entry: an http(s) URL, or a bare key.
pub fn parse(input: &str) -> Result<ParsedLink, CoreError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(CoreError::Validation("Enter a JIRA key or link".into()));
    }
    if is_url(input) {
        return Ok(ParsedLink {
            key: key_from_url(input).unwrap_or_default(),
            url: input.to_owned(),
        });
    }
    if is_key(input) {
        return Ok(ParsedLink {
            key: input.to_ascii_uppercase(),
            url: String::new(),
        });
    }
    Err(CoreError::Validation(format!(
        "{input:?} is neither a JIRA key like ACME-123 nor an http(s) link"
    )))
}

/// Normalize a key and URL given separately (the URL wins for the key when
/// the key is blank).
pub fn normalize(key: &str, url: &str) -> Result<ParsedLink, CoreError> {
    let (key, url) = (key.trim(), url.trim());
    if !url.is_empty() && !is_url(url) {
        return Err(CoreError::Validation(format!(
            "{url:?} is not an http(s) link"
        )));
    }
    if !key.is_empty() && !is_key(key) {
        return Err(CoreError::Validation(format!(
            "{key:?} is not a JIRA key like ACME-123"
        )));
    }
    if key.is_empty() && url.is_empty() {
        return Err(CoreError::Validation("Enter a JIRA key or link".into()));
    }
    let key = if key.is_empty() {
        key_from_url(url).unwrap_or_default()
    } else {
        key.to_ascii_uppercase()
    };
    Ok(ParsedLink {
        key,
        url: url.to_owned(),
    })
}

/// Where a stored link points: its URL, else `{base}/browse/{key}` when a
/// base URL is set, else nothing.
pub fn href(key: &str, url: &str, base_url: &str) -> Option<String> {
    if !url.is_empty() {
        return Some(url.to_owned());
    }
    let base = base_url.trim().trim_end_matches('/');
    (!base.is_empty() && !key.is_empty()).then(|| format!("{base}/browse/{key}"))
}

/// A `jira.base_url` value: empty, or an http(s) URL without a trailing slash.
pub fn valid_base_url(value: &str) -> Result<String, CoreError> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() || is_url(value) {
        Ok(value.to_owned())
    } else {
        Err(CoreError::Validation(
            "JIRA base URL must start with https:// or http://".into(),
        ))
    }
}

fn is_url(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && !text.contains(char::is_whitespace)
        && text.len() > "https://".len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browse_url_gives_its_key() {
        let link = parse("https://acme-labs.atlassian.net/browse/ACME-123").unwrap();
        assert_eq!(link.key, "ACME-123");
        assert_eq!(link.url, "https://acme-labs.atlassian.net/browse/ACME-123");
        assert_eq!(
            key_from_url(
                "https://acme-labs.atlassian.net/jira/software/projects/ACME/boards/1?selectedIssue=acme-7"
            ),
            Some("ACME-7".into())
        );
        let board = parse("https://acme-labs.atlassian.net/jira/dashboards/1").unwrap();
        assert_eq!(board.key, "");
    }

    #[test]
    fn a_bare_key_links_only_with_a_base_url() {
        let link = parse(" acme-42 ").unwrap();
        assert_eq!((link.key.as_str(), link.url.as_str()), ("ACME-42", ""));
        assert_eq!(href(&link.key, &link.url, ""), None);
        assert_eq!(
            href(&link.key, &link.url, "https://acme-labs.atlassian.net/"),
            Some("https://acme-labs.atlassian.net/browse/ACME-42".into())
        );
        assert_eq!(
            href(
                "ACME-1",
                "https://example.test/x",
                "https://acme-labs.atlassian.net"
            ),
            Some("https://example.test/x".into())
        );
    }

    #[test]
    fn junk_is_rejected() {
        assert!(parse("").is_err());
        assert!(parse("not a key").is_err());
        assert!(parse("ftp://acme").is_err());
        assert!(normalize("", "").is_err());
        assert!(normalize("bad key", "").is_err());
        assert_eq!(
            normalize("", "https://acme-labs.atlassian.net/browse/ACME-9")
                .unwrap()
                .key,
            "ACME-9"
        );
        assert_eq!(
            valid_base_url(" https://acme-labs.atlassian.net/ ").unwrap(),
            "https://acme-labs.atlassian.net"
        );
        assert_eq!(valid_base_url("").unwrap(), "");
        assert!(valid_base_url("acme-labs.atlassian.net").is_err());
    }
}
