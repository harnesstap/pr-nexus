//! "Add to <client>" and "Open config file" in Settings → Agents.
//!
//! Each MCP client keeps its servers in its own file and format. Adding
//! touches only the `pr-nexus` entry under the client's parent key, leaves
//! the rest of the file alone, writes `<file>.bak-pr-nexus` first and
//! replaces the file atomically. A file that does not parse is never
//! rewritten. Claude Code is the exception: its `~/.claude.json` is only
//! read; adding goes through `claude mcp add`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The entry name in every client's config.
pub const ENTRY: &str = "pr-nexus";
const BACKUP_SUFFIX: &str = ".bak-pr-nexus";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Client {
    ClaudeDesktop,
    ClaudeCode,
    Cursor,
    Vscode,
    Codex,
    Opencode,
}

impl Client {
    pub const ALL: [Client; 6] = [
        Client::ClaudeDesktop,
        Client::ClaudeCode,
        Client::Cursor,
        Client::Vscode,
        Client::Codex,
        Client::Opencode,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Where config files live. Tests inject a temp dir; the app uses the real home.
#[derive(Debug, Clone)]
pub struct Env {
    pub os: Os,
    pub home: PathBuf,
    /// `%APPDATA%` on Windows.
    pub appdata: Option<PathBuf>,
}

impl Env {
    pub fn current() -> Option<Self> {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)?;
        Some(Self {
            os: Os::current(),
            home,
            appdata: std::env::var_os("APPDATA").map(PathBuf::from),
        })
    }

    /// The per-user application data dir (`Application Support`, `%APPDATA%`, `~/.config`).
    fn app_support(&self) -> PathBuf {
        match self.os {
            Os::Mac => self.home.join("Library/Application Support"),
            Os::Windows => self
                .appdata
                .clone()
                .unwrap_or_else(|| self.home.join("AppData").join("Roaming")),
            Os::Linux => self.home.join(".config"),
        }
    }
}

/// The client's user-level config file.
pub fn config_path(client: Client, env: &Env) -> PathBuf {
    match client {
        Client::ClaudeDesktop => env
            .app_support()
            .join("Claude")
            .join("claude_desktop_config.json"),
        Client::ClaudeCode => env.home.join(".claude.json"),
        Client::Cursor => env.home.join(".cursor").join("mcp.json"),
        Client::Vscode => env.app_support().join("Code").join("User").join("mcp.json"),
        Client::Codex => env.home.join(".codex").join("config.toml"),
        Client::Opencode => {
            let dir = env.home.join(".config").join("opencode");
            let jsonc = dir.join("opencode.jsonc");
            let json = dir.join("opencode.json");
            if jsonc.exists() && !json.exists() {
                jsonc
            } else {
                json
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientStatus {
    NotAdded,
    Added,
    /// There is a `pr-nexus` entry, but its port or token differ.
    Outdated,
    /// The file exists but does not parse; Add refuses to touch it.
    Unreadable,
}

/// One tab's state as the Agents tab shows it.
#[derive(Debug, Clone, Serialize)]
pub struct ClientState {
    pub client: Client,
    pub status: ClientStatus,
    pub path: String,
    pub exists: bool,
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// The JSON parent key and the entry PR Nexus wants there.
fn json_entry(client: Client, url: &str, token: &str) -> (&'static str, Value) {
    let auth = bearer(token);
    match client {
        Client::ClaudeDesktop => (
            "mcpServers",
            json!({
                "command": "npx",
                "args": ["-y", "mcp-remote", url, "--transport", "http-only", "--header", "Authorization:${PR_NEXUS_AUTH}"],
                "env": { "PR_NEXUS_AUTH": auth },
            }),
        ),
        Client::ClaudeCode => (
            "mcpServers",
            json!({ "type": "http", "url": url, "headers": { "Authorization": auth } }),
        ),
        Client::Cursor => (
            "mcpServers",
            json!({ "url": url, "headers": { "Authorization": auth } }),
        ),
        Client::Vscode => (
            "servers",
            json!({ "type": "http", "url": url, "headers": { "Authorization": auth } }),
        ),
        Client::Opencode => (
            "mcp",
            json!({ "type": "remote", "url": url, "enabled": true, "headers": { "Authorization": auth } }),
        ),
        Client::Codex => unreachable!("Codex uses TOML"),
    }
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("Could not read {}: {err}", path.display())),
    }
}

fn parse_json(path: &Path, text: &str) -> Result<Map<String, Value>, String> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!(
            "{} is not a JSON object; left unchanged.",
            path.display()
        )),
        Err(err) => Err(format!(
            "{} is not plain JSON ({err}). Comments and trailing commas are not supported; left unchanged. Copy the snippet instead.",
            path.display()
        )),
    }
}

/// Whether `current` matches what PR Nexus would write. Claude Code only
/// needs the same URL and Authorization header, whatever else `claude` adds.
fn json_matches(client: Client, current: &Value, wanted: &Value) -> bool {
    if client == Client::ClaudeCode {
        return current.get("url") == wanted.get("url")
            && current.pointer("/headers/Authorization")
                == wanted.pointer("/headers/Authorization");
    }
    current == wanted
}

fn codex_wanted(url: &str, token: &str) -> (String, String) {
    (url.to_owned(), bearer(token))
}

/// Reads the client's file and compares its `pr-nexus` entry with `url` and `token`.
pub fn status(client: Client, path: &Path, url: &str, token: &str) -> ClientStatus {
    let text = match read_optional(path) {
        Ok(Some(text)) => text,
        Ok(None) => return ClientStatus::NotAdded,
        Err(_) => return ClientStatus::Unreadable,
    };
    if client == Client::Codex {
        let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
            return ClientStatus::Unreadable;
        };
        let Some(entry) = doc.get("mcp_servers").and_then(|t| t.get(ENTRY)) else {
            return ClientStatus::NotAdded;
        };
        let (want_url, want_auth) = codex_wanted(url, token);
        let got_url = entry.get("url").and_then(|v| v.as_str());
        let got_auth = entry
            .get("http_headers")
            .and_then(|h| h.get("Authorization"))
            .and_then(|v| v.as_str());
        return if got_url == Some(want_url.as_str()) && got_auth == Some(want_auth.as_str()) {
            ClientStatus::Added
        } else {
            ClientStatus::Outdated
        };
    }
    let Ok(map) = parse_json(path, &text) else {
        return ClientStatus::Unreadable;
    };
    let (parent, wanted) = json_entry(client, url, token);
    match map.get(parent).and_then(|servers| servers.get(ENTRY)) {
        None => ClientStatus::NotAdded,
        Some(current) if json_matches(client, current, &wanted) => ClientStatus::Added,
        Some(_) => ClientStatus::Outdated,
    }
}

/// The new file text with only the `pr-nexus` entry inserted or replaced.
pub fn merged(
    client: Client,
    path: &Path,
    existing: Option<&str>,
    url: &str,
    token: &str,
) -> Result<String, String> {
    if client == Client::ClaudeCode {
        return Err(
            "Claude Code is added with the claude CLI, not by editing ~/.claude.json.".to_owned(),
        );
    }
    if client == Client::Codex {
        let mut doc = existing
            .unwrap_or("")
            .parse::<toml_edit::DocumentMut>()
            .map_err(|err| {
                format!(
                    "{} is not valid TOML ({err}); left unchanged.",
                    path.display()
                )
            })?;
        let servers = doc
            .entry("mcp_servers")
            .or_insert_with(|| {
                let mut table = toml_edit::Table::new();
                table.set_implicit(true);
                toml_edit::Item::Table(table)
            })
            .as_table_mut()
            .ok_or_else(|| {
                format!(
                    "mcp_servers in {} is not a table; left unchanged.",
                    path.display()
                )
            })?;
        let (want_url, want_auth) = codex_wanted(url, token);
        let mut headers = toml_edit::InlineTable::new();
        headers.insert("Authorization", want_auth.into());
        let mut entry = toml_edit::Table::new();
        entry.insert("url", toml_edit::value(want_url));
        entry.insert("http_headers", toml_edit::value(headers));
        servers.insert(ENTRY, toml_edit::Item::Table(entry));
        return Ok(doc.to_string());
    }
    let mut map = parse_json(path, existing.unwrap_or(""))?;
    let (parent, wanted) = json_entry(client, url, token);
    let servers = map
        .entry(parent)
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(servers) = servers else {
        return Err(format!(
            "{parent} in {} is not an object; left unchanged.",
            path.display()
        ));
    };
    servers.insert(ENTRY.to_owned(), wanted);
    let mut text =
        serde_json::to_string_pretty(&Value::Object(map)).map_err(|err| err.to_string())?;
    text.push('\n');
    Ok(text)
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(BACKUP_SUFFIX);
    PathBuf::from(name)
}

/// Writes `text` over `path` through a temp file and a rename, keeping the
/// original's permissions and leaving a backup of it first.
fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("The config path has no folder")?;
    fs::create_dir_all(dir).map_err(|err| format!("Could not create {}: {err}", dir.display()))?;
    let permissions = fs::metadata(path).ok().map(|meta| meta.permissions());
    if permissions.is_some() {
        fs::copy(path, backup_path(path))
            .map_err(|err| format!("Could not back up {}: {err}", path.display()))?;
    }
    let mut temp = tempfile_in(dir)?;
    temp.1
        .write_all(text.as_bytes())
        .and_then(|()| temp.1.sync_all())
        .map_err(|err| format!("Could not write {}: {err}", temp.0.display()))?;
    drop(temp.1);
    if let Some(permissions) = permissions {
        fs::set_permissions(&temp.0, permissions).map_err(|err| err.to_string())?;
    }
    fs::rename(&temp.0, path).map_err(|err| {
        let _ = fs::remove_file(&temp.0);
        format!("Could not replace {}: {err}", path.display())
    })
}

fn tempfile_in(dir: &Path) -> Result<(PathBuf, fs::File), String> {
    for attempt in 0..100u32 {
        let candidate = dir.join(format!(".pr-nexus-{}-{attempt}.tmp", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("Could not write in {}: {err}", dir.display())),
        }
    }
    Err(format!("Could not create a temp file in {}", dir.display()))
}

/// Inserts or replaces the `pr-nexus` entry in a file-based client's config.
pub fn install_file(client: Client, path: &Path, url: &str, token: &str) -> Result<(), String> {
    let existing = read_optional(path)?;
    let text = merged(client, path, existing.as_deref(), url, token)?;
    write_atomically(path, &text)
}

/// The `claude` binary: PATH first, then where its installers put it (a GUI
/// app does not get the login shell's PATH on macOS).
pub fn find_claude(env: &Env) -> Option<PathBuf> {
    let name = if env.os == Os::Windows {
        "claude.exe"
    } else {
        "claude"
    };
    let from_path = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let common = [
        env.home.join(".claude/local").join(name),
        env.home.join(".local/bin").join(name),
        PathBuf::from("/opt/homebrew/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
    ];
    from_path
        .into_iter()
        .chain(common)
        .find(|path| path.is_file())
}

/// `claude mcp remove` then `claude mcp add`, user scope, no shell.
pub fn install_claude_code(env: &Env, url: &str, token: &str) -> Result<(), String> {
    let claude = find_claude(env).ok_or(
        "Could not find the claude command (PATH, ~/.claude/local, ~/.local/bin, /opt/homebrew/bin). Copy the command instead.",
    )?;
    let path = config_path(Client::ClaudeCode, env);
    if status(Client::ClaudeCode, &path, url, token) != ClientStatus::NotAdded {
        // Fails harmlessly when only another scope has it; `add` reports real problems.
        let _ = Command::new(&claude)
            .args(["mcp", "remove", "--scope", "user", ENTRY])
            .output();
    }
    let header = format!("Authorization: {}", bearer(token));
    let output = Command::new(&claude)
        .args([
            "mcp",
            "add",
            "--scope",
            "user",
            "--transport",
            "http",
            ENTRY,
            url,
            "--header",
            &header,
        ])
        .output()
        .map_err(|err| format!("Could not run {}: {err}", claude.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).replace(token, "pnx_…");
        Err(format!("claude mcp add failed: {}", stderr.trim()))
    }
}

pub fn state(client: Client, env: &Env, url: &str, token: &str) -> ClientState {
    let path = config_path(client, env);
    ClientState {
        client,
        status: status(client, &path, url, token),
        exists: path.exists(),
        path: path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:47900/mcp";
    const TOKEN: &str = "pnx_example";

    fn env(dir: &Path, os: Os) -> Env {
        Env {
            os,
            home: dir.to_path_buf(),
            appdata: Some(dir.join("AppData")),
        }
    }

    #[test]
    fn paths_per_os() {
        let home = Path::new("/h");
        let mac = env(home, Os::Mac);
        assert_eq!(
            config_path(Client::ClaudeDesktop, &mac),
            Path::new("/h/Library/Application Support/Claude/claude_desktop_config.json")
        );
        assert_eq!(
            config_path(Client::Vscode, &mac),
            Path::new("/h/Library/Application Support/Code/User/mcp.json")
        );
        let linux = env(home, Os::Linux);
        assert_eq!(
            config_path(Client::ClaudeDesktop, &linux),
            Path::new("/h/.config/Claude/claude_desktop_config.json")
        );
        assert_eq!(
            config_path(Client::Vscode, &linux),
            Path::new("/h/.config/Code/User/mcp.json")
        );
        let win = env(home, Os::Windows);
        assert_eq!(
            config_path(Client::ClaudeDesktop, &win),
            Path::new("/h/AppData/Claude/claude_desktop_config.json")
        );
        assert_eq!(
            config_path(Client::Cursor, &linux),
            Path::new("/h/.cursor/mcp.json")
        );
        assert_eq!(
            config_path(Client::Codex, &linux),
            Path::new("/h/.codex/config.toml")
        );
        assert_eq!(
            config_path(Client::ClaudeCode, &linux),
            Path::new("/h/.claude.json")
        );
        assert_eq!(
            config_path(Client::Opencode, &linux),
            Path::new("/h/.config/opencode/opencode.json")
        );
    }

    #[test]
    fn opencode_uses_an_existing_jsonc() {
        let dir = tempfile::tempdir().unwrap();
        let e = env(dir.path(), Os::Linux);
        fs::create_dir_all(dir.path().join(".config/opencode")).unwrap();
        fs::write(dir.path().join(".config/opencode/opencode.jsonc"), "{}").unwrap();
        assert!(config_path(Client::Opencode, &e).ends_with("opencode.jsonc"));
    }

    #[test]
    fn creates_a_missing_file_and_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(Client::Cursor, &env(dir.path(), Os::Linux));
        assert_eq!(
            status(Client::Cursor, &path, URL, TOKEN),
            ClientStatus::NotAdded
        );
        install_file(Client::Cursor, &path, URL, TOKEN).unwrap();
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["mcpServers"]["pr-nexus"]["url"], URL);
        assert_eq!(
            value["mcpServers"]["pr-nexus"]["headers"]["Authorization"],
            "Bearer pnx_example"
        );
        assert_eq!(
            status(Client::Cursor, &path, URL, TOKEN),
            ClientStatus::Added
        );
        assert!(!backup_path(&path).exists());
    }

    #[test]
    fn merges_keeping_other_servers_and_keys_and_writes_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(Client::ClaudeDesktop, &env(dir.path(), Os::Mac));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = r#"{"theme":"dark","mcpServers":{"other":{"command":"other-mcp"}}}"#;
        fs::write(&path, before).unwrap();
        install_file(Client::ClaudeDesktop, &path, URL, TOKEN).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["mcpServers"]["other"]["command"], "other-mcp");
        assert_eq!(
            value["mcpServers"]["pr-nexus"]["env"]["PR_NEXUS_AUTH"],
            "Bearer pnx_example"
        );
        assert!(
            text.find("theme").unwrap() < text.find("mcpServers").unwrap(),
            "key order kept"
        );
        assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), before);
    }

    #[test]
    fn replaces_an_existing_entry_and_reports_outdated_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(Client::Vscode, &env(dir.path(), Os::Linux));
        install_file(
            Client::Vscode,
            &path,
            "http://127.0.0.1:47901/mcp",
            "pnx_old",
        )
        .unwrap();
        assert_eq!(
            status(Client::Vscode, &path, URL, TOKEN),
            ClientStatus::Outdated
        );
        install_file(Client::Vscode, &path, URL, TOKEN).unwrap();
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["servers"]["pr-nexus"]["type"], "http");
        assert_eq!(value["servers"]["pr-nexus"]["url"], URL);
        assert_eq!(value["servers"].as_object().unwrap().len(), 1);
        assert_eq!(
            status(Client::Vscode, &path, URL, TOKEN),
            ClientStatus::Added
        );
    }

    #[test]
    fn leaves_an_unparseable_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(Client::Opencode, &env(dir.path(), Os::Linux));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = "{\n  // my servers\n  \"mcp\": {},\n}\n";
        fs::write(&path, before).unwrap();
        assert_eq!(
            status(Client::Opencode, &path, URL, TOKEN),
            ClientStatus::Unreadable
        );
        let err = install_file(Client::Opencode, &path, URL, TOKEN).unwrap_err();
        assert!(err.contains("left unchanged"), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert!(!backup_path(&path).exists());
    }

    #[test]
    fn codex_toml_keeps_comments_and_other_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(Client::Codex, &env(dir.path(), Os::Linux));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = "# my model\nmodel = \"some-model\"\n\n[mcp_servers.other]\ncommand = \"other-mcp\" # keep me\n";
        fs::write(&path, before).unwrap();
        install_file(Client::Codex, &path, URL, TOKEN).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(before), "{text}");
        assert!(text.contains("[mcp_servers.pr-nexus]"));
        assert!(text.contains("url = \"http://127.0.0.1:47900/mcp\""));
        assert!(text.contains("Authorization = \"Bearer pnx_example\""));
        assert_eq!(
            status(Client::Codex, &path, URL, TOKEN),
            ClientStatus::Added
        );
        assert_eq!(
            status(Client::Codex, &path, URL, "pnx_other"),
            ClientStatus::Outdated
        );
        assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn keeps_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(Client::Cursor, &env(dir.path(), Os::Linux));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        install_file(Client::Cursor, &path, URL, TOKEN).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn claude_code_status_reads_the_top_level_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(Client::ClaudeCode, &env(dir.path(), Os::Linux));
        fs::write(
            &path,
            r#"{"projects":{},"mcpServers":{"pr-nexus":{"type":"http","url":"http://127.0.0.1:47900/mcp","headers":{"Authorization":"Bearer pnx_example"}}}}"#,
        )
        .unwrap();
        assert_eq!(
            status(Client::ClaudeCode, &path, URL, TOKEN),
            ClientStatus::Added
        );
        assert_eq!(
            status(Client::ClaudeCode, &path, URL, "pnx_new"),
            ClientStatus::Outdated
        );
        assert!(merged(Client::ClaudeCode, &path, None, URL, TOKEN).is_err());
    }
}
