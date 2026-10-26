//! "Update available" check (§10 D10): compare the running version with the
//! latest GitHub release tag. No auto-updater: the user installs the DMG.

/// A parsed `MAJOR.MINOR.PATCH`, with an optional `v` prefix. A pre-release
/// suffix (`-rc.1`) or build metadata is ignored for ordering, but a
/// pre-release never counts as newer than the same final version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    final_release: bool,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split('+').next().unwrap_or(text);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
            final_release: pre.is_none(),
        })
    }
}

/// True when `latest_tag` is a strictly newer version than `current`.
/// Unparseable input is never "newer".
pub fn is_newer(current: &str, latest_tag: &str) -> bool {
    match (Version::parse(current), Version::parse(latest_tag)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

/// `owner/repo` from a `https://github.com/owner/repo` URL.
pub fn repo_slug(repository_url: &str) -> Option<String> {
    let rest = repository_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .strip_prefix("https://github.com/")?;
    let (owner, repo) = rest.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_tags() {
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(is_newer("0.1.9", "v0.2.0"));
        assert!(is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("0.2.0", "v0.2.0"));
        assert!(!is_newer("0.2.0", "v0.1.5"));
        assert!(!is_newer("0.2.0", "v0.10"));
        assert!(!is_newer("0.2.0", "nightly"));
    }

    #[test]
    fn a_prerelease_is_older_than_its_final_version() {
        assert!(is_newer("0.2.0-rc.1", "v0.2.0"));
        assert!(!is_newer("0.2.0", "v0.2.0-rc.2"));
        assert!(is_newer("0.1.0", "v0.2.0-rc.1"));
    }

    #[test]
    fn reads_the_repo_slug() {
        assert_eq!(
            repo_slug("https://github.com/acme/widgets").as_deref(),
            Some("acme/widgets")
        );
        assert_eq!(
            repo_slug("https://github.com/acme/widgets.git/").as_deref(),
            Some("acme/widgets")
        );
        assert_eq!(repo_slug("https://example.test/acme/widgets"), None);
        assert_eq!(repo_slug("https://github.com/acme"), None);
    }
}
