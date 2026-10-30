//! "Update available" check (§10 D10). Release builds compare the running
//! version with the latest GitHub release tag; dev builds (debug, built from
//! source) compare the commit they were built from with the head of `main`.
//! No auto-updater: the user installs the DMG or pulls and rebuilds.

use serde::{Deserialize, Serialize};

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

/// How the running binary was built, which decides what "newer" means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    /// A packaged release: compare semver with the latest GitHub release.
    Release,
    /// A debug build from a checkout: compare its commit with `main`.
    /// `None` when the build had no git metadata (a source tarball).
    Dev { sha: Option<String> },
}

/// The minimal HTTP surface the check needs, so tests can mock GitHub.
pub trait HttpGet {
    /// GET `url` with `accept`. `Err` is a transport failure (offline, DNS).
    fn get(&self, url: &str, accept: &str) -> Result<HttpResponse, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateKind {
    Release,
    Dev,
}

/// What the sidebar shows when something newer exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInfo {
    pub kind: UpdateKind,
    /// The running version (release) or short commit (dev).
    pub current: String,
    /// The newer version (release) or the short head of `main` (dev). It is
    /// also the key "Skip this version" stores.
    pub version: String,
    /// Release notes page, or the compare view for dev builds.
    pub url: String,
    /// What to run: a `gh release download` (release) or pull-and-rebuild (dev).
    pub command: String,
    /// Dev only: how many commits `main` has that this build lacks.
    pub behind_by: Option<u64>,
}

/// Run from the repo root to update a dev build.
pub const DEV_UPDATE_COMMAND: &str = "git pull && bun install --cwd app && bun run tauri dev";

const API: &str = "https://api.github.com";
const JSON: &str = "application/vnd.github+json";
const SHA: &str = "application/vnd.github.sha";

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Debug, Deserialize)]
struct Compare {
    status: String,
    ahead_by: u64,
    html_url: String,
}

/// Ask GitHub whether something newer than this build exists. Every failure
/// (offline, rate limit, no release yet, unknown local commit) is `None`:
/// the check is best effort and never shows an error.
pub fn check(
    http: &dyn HttpGet,
    repo: &str,
    current_version: &str,
    channel: &Channel,
) -> Option<UpdateInfo> {
    match channel {
        Channel::Release => check_release(http, repo, current_version),
        Channel::Dev { sha: Some(sha) } => check_dev(http, repo, sha),
        Channel::Dev { sha: None } => None,
    }
}

/// The body of a 2xx answer. The app's `HttpGet` logs other statuses and
/// transport errors, so this stays silent.
fn get_ok(http: &dyn HttpGet, url: &str, accept: &str) -> Option<String> {
    http.get(url, accept)
        .ok()
        .filter(|response| (200..300).contains(&response.status))
        .map(|response| response.body)
}

fn check_release(http: &dyn HttpGet, repo: &str, current: &str) -> Option<UpdateInfo> {
    let body = get_ok(http, &format!("{API}/repos/{repo}/releases/latest"), JSON)?;
    let release: Release = serde_json::from_str(&body).ok()?;
    if release.draft || release.prerelease || !is_newer(current, &release.tag_name) {
        return None;
    }
    Some(UpdateInfo {
        kind: UpdateKind::Release,
        current: current.trim_start_matches('v').to_owned(),
        command: format!(
            "gh release download {} -R {repo} -p '*.dmg'",
            release.tag_name
        ),
        version: release.tag_name.trim_start_matches('v').to_owned(),
        url: release.html_url,
        behind_by: None,
    })
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn check_dev(http: &dyn HttpGet, repo: &str, sha: &str) -> Option<UpdateInfo> {
    let sha = sha.trim();
    if sha.len() < 7 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let head = get_ok(http, &format!("{API}/repos/{repo}/commits/main"), SHA)?;
    let head = head.trim();
    if head.is_empty() || head.eq_ignore_ascii_case(sha) {
        return None;
    }
    let body = get_ok(
        http,
        &format!("{API}/repos/{repo}/compare/{sha}...{head}"),
        JSON,
    )?;
    let compare: Compare = serde_json::from_str(&body).ok()?;
    // `ahead_by` counts commits on main the build lacks. "behind" or
    // "identical" means the build already has all of main.
    if compare.ahead_by == 0 || !matches!(compare.status.as_str(), "ahead" | "diverged") {
        return None;
    }
    Some(UpdateInfo {
        kind: UpdateKind::Dev,
        current: short(sha),
        version: short(head),
        url: compare.html_url,
        command: DEV_UPDATE_COMMAND.to_owned(),
        behind_by: Some(compare.ahead_by),
    })
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

    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Canned GitHub: URL → response; unknown URLs are a transport error.
    #[derive(Default)]
    struct FakeGithub {
        routes: HashMap<String, HttpResponse>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeGithub {
        fn route(mut self, path: &str, status: u16, body: &str) -> Self {
            self.routes.insert(
                format!("{API}{path}"),
                HttpResponse {
                    status,
                    body: body.to_owned(),
                },
            );
            self
        }
    }

    impl HttpGet for FakeGithub {
        fn get(&self, url: &str, _accept: &str) -> Result<HttpResponse, String> {
            self.calls.borrow_mut().push(url.to_owned());
            self.routes
                .get(url)
                .cloned()
                .ok_or_else(|| "offline".into())
        }
    }

    const BUILD: &str = "1111111aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HEAD: &str = "2222222bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn dev() -> Channel {
        Channel::Dev {
            sha: Some(BUILD.into()),
        }
    }

    fn release_json(tag: &str, prerelease: bool) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/acme/widgets/releases/tag/{tag}","draft":false,"prerelease":{prerelease}}}"#
        )
    }

    #[test]
    fn a_newer_release_is_an_update() {
        let gh = FakeGithub::default().route(
            "/repos/acme/widgets/releases/latest",
            200,
            &release_json("v0.2.0", false),
        );
        let info = check(&gh, "acme/widgets", "0.1.0", &Channel::Release).unwrap();
        assert_eq!(info.kind, UpdateKind::Release);
        assert_eq!(info.current, "0.1.0");
        assert_eq!(info.version, "0.2.0");
        assert_eq!(
            info.command,
            "gh release download v0.2.0 -R acme/widgets -p '*.dmg'"
        );
        assert_eq!(info.behind_by, None);
        assert!(check(&gh, "acme/widgets", "0.2.0", &Channel::Release).is_none());
    }

    #[test]
    fn prereleases_missing_releases_and_failures_are_silent() {
        let pre = FakeGithub::default().route(
            "/repos/acme/widgets/releases/latest",
            200,
            &release_json("v0.3.0", true),
        );
        assert!(check(&pre, "acme/widgets", "0.1.0", &Channel::Release).is_none());
        let none = FakeGithub::default().route("/repos/acme/widgets/releases/latest", 404, "{}");
        assert!(check(&none, "acme/widgets", "0.1.0", &Channel::Release).is_none());
        let limited = FakeGithub::default().route(
            "/repos/acme/widgets/releases/latest",
            403,
            r#"{"message":"API rate limit exceeded"}"#,
        );
        assert!(check(&limited, "acme/widgets", "0.1.0", &Channel::Release).is_none());
        let garbage =
            FakeGithub::default().route("/repos/acme/widgets/releases/latest", 200, "<html>");
        assert!(check(&garbage, "acme/widgets", "0.1.0", &Channel::Release).is_none());
        let offline = FakeGithub::default();
        assert!(check(&offline, "acme/widgets", "0.1.0", &Channel::Release).is_none());
    }

    #[test]
    fn a_dev_build_behind_main_counts_the_missing_commits() {
        let gh = FakeGithub::default()
            .route("/repos/acme/widgets/commits/main", 200, HEAD)
            .route(
                &format!("/repos/acme/widgets/compare/{BUILD}...{HEAD}"),
                200,
                r#"{"status":"ahead","ahead_by":3,"behind_by":0,"html_url":"https://github.com/acme/widgets/compare/x...y"}"#,
            );
        let info = check(&gh, "acme/widgets", "0.1.0", &dev()).unwrap();
        assert_eq!(info.kind, UpdateKind::Dev);
        assert_eq!(info.current, "1111111");
        assert_eq!(info.version, "2222222");
        assert_eq!(info.behind_by, Some(3));
        assert_eq!(info.command, DEV_UPDATE_COMMAND);
        assert_eq!(info.url, "https://github.com/acme/widgets/compare/x...y");
    }

    #[test]
    fn a_dev_build_on_main_or_ahead_of_it_is_current() {
        let same = FakeGithub::default().route("/repos/acme/widgets/commits/main", 200, BUILD);
        assert!(check(&same, "acme/widgets", "0.1.0", &dev()).is_none());
        assert_eq!(
            same.calls.borrow().len(),
            1,
            "no compare call when SHAs match"
        );

        let ahead = FakeGithub::default()
            .route("/repos/acme/widgets/commits/main", 200, HEAD)
            .route(
                &format!("/repos/acme/widgets/compare/{BUILD}...{HEAD}"),
                200,
                r#"{"status":"behind","ahead_by":0,"behind_by":2,"html_url":"u"}"#,
            );
        assert!(check(&ahead, "acme/widgets", "0.1.0", &dev()).is_none());
    }

    #[test]
    fn a_diverged_branch_still_reports_what_main_has() {
        let gh = FakeGithub::default()
            .route("/repos/acme/widgets/commits/main", 200, HEAD)
            .route(
                &format!("/repos/acme/widgets/compare/{BUILD}...{HEAD}"),
                200,
                r#"{"status":"diverged","ahead_by":5,"behind_by":2,"html_url":"u"}"#,
            );
        assert_eq!(
            check(&gh, "acme/widgets", "0.1.0", &dev())
                .unwrap()
                .behind_by,
            Some(5)
        );
    }

    #[test]
    fn unknown_local_commits_missing_shas_and_offline_are_silent() {
        // A commit that was never pushed: GitHub answers 404 on compare.
        let unpushed = FakeGithub::default()
            .route("/repos/acme/widgets/commits/main", 200, HEAD)
            .route(
                &format!("/repos/acme/widgets/compare/{BUILD}...{HEAD}"),
                404,
                "{}",
            );
        assert!(check(&unpushed, "acme/widgets", "0.1.0", &dev()).is_none());
        let offline = FakeGithub::default();
        assert!(check(&offline, "acme/widgets", "0.1.0", &dev()).is_none());
        let limited = FakeGithub::default().route("/repos/acme/widgets/commits/main", 429, "");
        assert!(check(&limited, "acme/widgets", "0.1.0", &dev()).is_none());
        let gh = FakeGithub::default().route("/repos/acme/widgets/commits/main", 200, HEAD);
        assert!(check(&gh, "acme/widgets", "0.1.0", &Channel::Dev { sha: None }).is_none());
        let bad = Channel::Dev {
            sha: Some("not-a-sha".into()),
        };
        assert!(check(&gh, "acme/widgets", "0.1.0", &bad).is_none());
        assert!(gh.calls.borrow().is_empty());
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
