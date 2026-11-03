//! `gh` CLI token and the GitHub device flow.
//!
//! The device-flow client id is never committed. Callers pass one from a
//! setting or `PR_NEXUS_GITHUB_OAUTH_CLIENT_ID`.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use pr_nexus_core::error::ProviderError;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug)]
pub enum GhProbe {
    /// `gh` is missing, not logged in, or failed; `reason` says which.
    Unavailable {
        reason: String,
    },
    Ready {
        token: String,
    },
}

impl GhProbe {
    pub fn token(&self) -> Option<&str> {
        match self {
            Self::Ready { token } => Some(token.as_str()),
            Self::Unavailable { .. } => None,
        }
    }
}

/// How long `gh auth token` may run. gh can read its token from the system
/// keychain, and macOS may show an "allow access" prompt meanwhile; the
/// user needs time to answer it.
const GH_TOKEN_TIMEOUT: Duration = Duration::from_secs(90);
/// How long the login shell may take to report where `gh` is.
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(3);

const NOT_FOUND: &str =
    "GitHub CLI (gh) not found. Install it from cli.github.com, or use a personal access token.";

/// Finds the `gh` binary. Apps launched from Finder, the Dock or a desktop
/// launcher get a minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin` on macOS), so
/// PATH alone misses Homebrew and similar installs: after PATH this asks the
/// user's login shell (once per run), then checks where installers put gh.
pub fn find_gh() -> Option<PathBuf> {
    let path = std::env::var_os("PATH");
    resolve_gh(
        path.as_deref(),
        login_shell_gh,
        &gh_candidates(&|key| std::env::var_os(key).map(PathBuf::from)),
    )
}

fn resolve_gh(
    path: Option<&OsStr>,
    login_shell: impl FnOnce() -> Option<PathBuf>,
    candidates: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(found) = path.and_then(|path| search_path(path, GH_EXE)) {
        return Some(found);
    }
    if let Some(found) = login_shell().filter(|found| is_executable(found)) {
        return Some(found);
    }
    candidates.iter().find(|path| is_executable(path)).cloned()
}

const GH_EXE: &str = if cfg!(windows) { "gh.exe" } else { "gh" };

fn search_path(path: &OsStr, exe: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(exe))
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// Well-known install locations, given an environment lookup.
fn gh_candidates(env: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        if let Some(dir) = env("ProgramFiles") {
            out.push(dir.join("GitHub CLI").join("gh.exe"));
        }
        if let Some(dir) = env("LOCALAPPDATA") {
            out.push(dir.join("Programs").join("GitHub CLI").join("gh.exe"));
            out.push(
                dir.join("Microsoft")
                    .join("WinGet")
                    .join("Links")
                    .join("gh.exe"),
            );
        }
        if let Some(dir) = env("USERPROFILE") {
            out.push(dir.join("scoop").join("shims").join("gh.exe"));
        }
    } else {
        for dir in [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/home/linuxbrew/.linuxbrew/bin",
        ] {
            out.push(Path::new(dir).join("gh"));
        }
        if let Some(home) = env("HOME") {
            out.push(home.join(".local/bin/gh"));
        }
        out.push(PathBuf::from("/usr/bin/gh"));
        out.push(PathBuf::from("/snap/bin/gh"));
    }
    out
}

/// `command -v gh` in the user's login shell, asked once per run.
fn login_shell_gh() -> Option<PathBuf> {
    static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            if cfg!(windows) {
                return None;
            }
            let shell = std::env::var_os("SHELL").filter(|shell| !shell.is_empty())?;
            let child = Command::new(shell)
                .args(["-l", "-c", "command -v gh"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let (_, stdout, _) = wait_with_timeout(child, LOGIN_SHELL_TIMEOUT).ok()?;
            parse_command_v(&stdout)
        })
        .clone()
}

/// The last absolute path a shell printed (profiles may print banners first).
fn parse_command_v(stdout: &str) -> Option<PathBuf> {
    stdout
        .lines()
        .map(str::trim)
        .rev()
        .find(|line| line.starts_with('/'))
        .map(PathBuf::from)
}

type Finished = (std::process::ExitStatus, String, String);

/// Waits for `child`, killing it after `timeout`. Pipes are read while it
/// runs so a large output cannot block it.
fn wait_with_timeout(mut child: Child, timeout: Duration) -> Result<Finished, String> {
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out = thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = pipe.read_to_string(&mut text);
        }
        text
    });
    let err = thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.read_to_string(&mut text);
        }
        text
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
            Err(err) => return Err(err.to_string()),
        }
    };
    Ok((
        status,
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    ))
}

pub fn read_gh_token() -> GhProbe {
    read_gh_token_for(None)
}

/// `gh auth token` for one of the accounts gh knows (`--user <login>`), or
/// the active one. Never switches gh's active account.
pub fn read_gh_token_for(login: Option<&str>) -> GhProbe {
    let Some(gh) = find_gh() else {
        return GhProbe::Unavailable {
            reason: NOT_FOUND.into(),
        };
    };
    // stdin is closed: `gh auth token` never prompts on the terminal. A
    // keychain prompt on macOS is a system dialog, which the timeout allows for.
    let mut args = vec!["auth", "token", "--hostname", "github.com"];
    if let Some(login) = login {
        args.extend(["--user", login]);
    }
    let child = match Command::new(&gh)
        .args(&args)
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return GhProbe::Unavailable {
                reason: format!("gh found at {} but could not start: {err}", gh.display()),
            };
        }
    };
    gh_outcome(&gh, wait_with_timeout(child, GH_TOKEN_TIMEOUT))
}

fn gh_outcome(gh: &Path, finished: Result<Finished, String>) -> GhProbe {
    let at = gh.display();
    match finished {
        Ok((status, stdout, stderr)) => {
            let token = stdout.trim();
            if status.success() && !token.is_empty() {
                return GhProbe::Ready {
                    token: token.to_owned(),
                };
            }
            let detail = stderr.trim();
            let lower = detail.to_ascii_lowercase();
            let reason = if token.is_empty()
                && (status.success()
                    || lower.contains("not logged")
                    || lower.contains("no oauth token")
                    || lower.contains("gh auth login"))
            {
                format!("gh found at {at} but not logged in. Run `gh auth login`, then try again.")
            } else if detail.is_empty() {
                format!("gh found at {at} but `gh auth token` failed ({status}).")
            } else {
                format!("gh found at {at} but `gh auth token` failed: {detail}")
            };
            GhProbe::Unavailable { reason }
        }
        Err(err) => GhProbe::Unavailable {
            reason: format!(
                "gh found at {at} but `gh auth token` {err} after {}s. If macOS asked for keychain access, allow it and try again.",
                GH_TOKEN_TIMEOUT.as_secs()
            ),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    Pending { slow_down: bool },
    Denied,
    Expired,
    Token(String),
}

pub fn start_device_flow(
    http: &reqwest::blocking::Client,
    login_base: &str,
    client_id: &str,
) -> Result<DeviceStart, ProviderError> {
    if client_id.trim().is_empty() {
        return Err(ProviderError::Unsupported(
            "GitHub device flow needs github.oauth_client_id or PR_NEXUS_GITHUB_OAUTH_CLIENT_ID"
                .into(),
        ));
    }
    let response = http
        .post(format!("{login_base}/login/device/code"))
        .header("Accept", "application/json")
        .json(&json!({
            "client_id": client_id,
            "scope": "repo read:org"
        }))
        .send()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(crate::github::http::classify(
            status,
            None,
            None,
            &body,
            chrono::Utc::now(),
        ));
    }
    let parsed: DeviceCodeBody = serde_json::from_str(&body)
        .map_err(|err| ProviderError::Decode(format!("device code: {err}")))?;
    Ok(DeviceStart {
        device_code: parsed.device_code,
        user_code: parsed.user_code,
        verification_uri: parsed.verification_uri,
        expires_in: parsed.expires_in,
        interval: parsed.interval.max(1),
    })
}

pub fn poll_device_flow(
    http: &reqwest::blocking::Client,
    login_base: &str,
    client_id: &str,
    device_code: &str,
) -> Result<DevicePoll, ProviderError> {
    let response = http
        .post(format!("{login_base}/login/oauth/access_token"))
        .header("Accept", "application/json")
        .json(&json!({
            "client_id": client_id,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
        }))
        .send()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    let body = response
        .text()
        .map_err(|err| ProviderError::Network(err.to_string()))?;
    let parsed: DevicePollBody = serde_json::from_str(&body)
        .map_err(|err| ProviderError::Decode(format!("device poll: {err}")))?;
    if let Some(token) = parsed.access_token.filter(|token| !token.is_empty()) {
        return Ok(DevicePoll::Token(token));
    }
    match parsed.error.as_deref() {
        Some("authorization_pending") => Ok(DevicePoll::Pending { slow_down: false }),
        Some("slow_down") => Ok(DevicePoll::Pending { slow_down: true }),
        Some("access_denied") => Ok(DevicePoll::Denied),
        Some("expired_token") => Ok(DevicePoll::Expired),
        Some(other) => Err(ProviderError::Api {
            code: other.to_owned(),
            message: parsed
                .error_description
                .unwrap_or_else(|| "device flow failed".into()),
        }),
        None => Err(ProviderError::Decode(
            "device flow response had neither a token nor an error".into(),
        )),
    }
}

#[derive(Deserialize)]
struct DeviceCodeBody {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct DevicePollBody {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

pub fn oauth_client_id_from_env() -> Option<String> {
    std::env::var("PR_NEXUS_GITHUB_OAUTH_CLIENT_ID")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn github_token_from_env() -> Option<String> {
    std::env::var("PR_NEXUS_GITHUB_TOKEN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prn-gh-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_gh(dir: &Path, mode: u32) -> PathBuf {
        let path = dir.join("gh");
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn path_wins_over_the_login_shell_and_known_places() {
        let root = scratch("path");
        let on_path = fake_gh(&root.join("a").tap_mkdir(), 0o755);
        let known = fake_gh(&root.join("b").tap_mkdir(), 0o755);
        let path = std::env::join_paths([root.join("missing"), root.join("a")]).unwrap();
        let found = resolve_gh(Some(&path), || panic!("login shell not needed"), &[known]);
        assert_eq!(found, Some(on_path));
    }

    #[test]
    fn a_minimal_gui_path_falls_back_to_the_login_shell() {
        let root = scratch("shell");
        let brew = fake_gh(&root, 0o755);
        let path = std::env::join_paths([root.join("usr-bin")]).unwrap();
        let found = resolve_gh(Some(&path), || Some(brew.clone()), &[]);
        assert_eq!(found, Some(brew));
    }

    #[test]
    fn known_install_places_are_the_last_resort() {
        let root = scratch("known");
        let not_exec = fake_gh(&root.join("x").tap_mkdir(), 0o644);
        let brew = fake_gh(&root.join("y").tap_mkdir(), 0o755);
        let found = resolve_gh(
            None,
            || Some(root.join("gone/gh")),
            &[root.join("nope/gh"), not_exec, brew.clone()],
        );
        assert_eq!(found, Some(brew));
        assert_eq!(resolve_gh(None, || None, &[root.join("nope/gh")]), None);
    }

    #[test]
    fn candidates_cover_homebrew_and_the_home_dir() {
        let list = gh_candidates(&|key| (key == "HOME").then(|| PathBuf::from("/home/ada")));
        assert_eq!(list[0], PathBuf::from("/opt/homebrew/bin/gh"));
        assert!(list.contains(&PathBuf::from("/usr/local/bin/gh")));
        assert!(list.contains(&PathBuf::from("/home/ada/.local/bin/gh")));
        assert!(list.contains(&PathBuf::from("/snap/bin/gh")));
    }

    #[test]
    fn command_v_output_skips_profile_banners() {
        assert_eq!(
            parse_command_v("Welcome!\n/opt/homebrew/bin/gh\n"),
            Some(PathBuf::from("/opt/homebrew/bin/gh"))
        );
        assert_eq!(parse_command_v("gh: not found\n"), None);
    }

    #[test]
    fn outcomes_say_what_to_do() {
        let gh = Path::new("/opt/homebrew/bin/gh");
        let ok = std::process::ExitStatus::from_raw(0);
        let fail = std::process::ExitStatus::from_raw(1 << 8);
        assert_eq!(
            gh_outcome(gh, Ok((ok, "gho_x\n".into(), String::new()))).token(),
            Some("gho_x")
        );
        let GhProbe::Unavailable { reason } = gh_outcome(
            gh,
            Ok((
                fail,
                String::new(),
                "no oauth token found for github.com".into(),
            )),
        ) else {
            panic!("expected unavailable");
        };
        assert_eq!(
            reason,
            "gh found at /opt/homebrew/bin/gh but not logged in. Run `gh auth login`, then try again."
        );
        let GhProbe::Unavailable { reason } = gh_outcome(gh, Err("timed out".into())) else {
            panic!("expected unavailable");
        };
        assert!(reason.contains("keychain"), "{reason}");
    }

    #[test]
    fn a_slow_child_is_killed_at_the_timeout() {
        let child = Command::new("sleep")
            .arg("5")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        assert!(wait_with_timeout(child, Duration::from_millis(200)).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    trait TapMkdir {
        fn tap_mkdir(self) -> Self;
    }
    impl TapMkdir for PathBuf {
        fn tap_mkdir(self) -> Self {
            std::fs::create_dir_all(&self).unwrap();
            self
        }
    }
}

/// The github.com logins gh is signed in to, active first, read from
/// `gh auth status`. Empty when gh is missing or signed out.
pub fn gh_accounts() -> Vec<String> {
    let Some(gh) = find_gh() else {
        return Vec::new();
    };
    let Ok(child) = Command::new(&gh)
        .args(["auth", "status", "--hostname", "github.com"])
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return Vec::new();
    };
    match wait_with_timeout(child, GH_TOKEN_TIMEOUT) {
        Ok((_, stdout, stderr)) => parse_gh_accounts(&format!("{stdout}\n{stderr}")),
        Err(_) => Vec::new(),
    }
}

/// Parses `gh auth status` lines such as
/// `✓ Logged in to github.com account alice (keyring)` and the following
/// `- Active account: true`.
pub fn parse_gh_accounts(output: &str) -> Vec<String> {
    let mut accounts: Vec<(String, bool)> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.split("Logged in to github.com account ").nth(1) {
            if let Some(login) = rest.split_whitespace().next() {
                accounts.push((login.to_owned(), false));
            }
        } else if line.contains("Active account: true")
            && let Some(last) = accounts.last_mut()
        {
            last.1 = true;
        }
    }
    accounts.sort_by_key(|(_, active)| !active);
    let mut out: Vec<String> = Vec::new();
    for (login, _) in accounts {
        if !out.contains(&login) {
            out.push(login);
        }
    }
    out
}

#[cfg(test)]
mod gh_accounts_tests {
    use super::parse_gh_accounts;

    #[test]
    fn lists_every_account_active_first() {
        let output = "github.com\n  ✓ Logged in to github.com account alice (keyring)\n  - Active account: false\n  ✓ Logged in to github.com account acme-bot (keyring)\n  - Active account: true\n";
        assert_eq!(parse_gh_accounts(output), ["acme-bot", "alice"]);
        assert!(parse_gh_accounts("You are not logged into any GitHub hosts.").is_empty());
    }
}
