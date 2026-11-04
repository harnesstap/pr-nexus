//! "Add to <client>" and "Open config file" in Settings → Agents.
//!
//! Each MCP client keeps its servers in its own file and format. Adding
//! touches only the `pr-nexus` entry under the client's parent key, leaves
//! the rest of the file alone, writes `<file>.bak-pr-nexus` first and
//! replaces the file atomically. JSON files are read as JSONC (comments and
//! trailing commas) and edited in place, so comments and formatting stay.
//! A file that does not parse is never rewritten. Claude Code is the
//! exception: its `~/.claude.json` is only read; adding goes through
//! `claude mcp add`.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
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
    /// `$XDG_CONFIG_HOME`, which OpenCode uses instead of `~/.config` when set.
    pub xdg_config: Option<PathBuf>,
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
            xdg_config: std::env::var_os("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
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
            let dir = opencode_dir(env);
            OPENCODE_FILES
                .iter()
                .map(|name| dir.join(name))
                .find(|path| path.is_file())
                .unwrap_or_else(|| dir.join(OPENCODE_FILES[0]))
        }
    }
}

/// OpenCode's global config files, the one OpenCode reads last first: it
/// merges `opencode.json` then `opencode.jsonc` (so `.jsonc` wins), and
/// OpenCode 1.x also read a legacy `config.json` before both. A new file is
/// `opencode.jsonc`, like `opencode` itself creates.
const OPENCODE_FILES: [&str; 3] = ["opencode.jsonc", "opencode.json", "config.json"];

/// `$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`, on every OS.
fn opencode_dir(env: &Env) -> PathBuf {
    env.xdg_config
        .clone()
        .unwrap_or_else(|| env.home.join(".config"))
        .join("opencode")
}

/// The path with the home folder shown as `~`, for labels.
pub fn display_path(path: &Path, env: &Env) -> String {
    match path.strip_prefix(&env.home) {
        Ok(rest) if !rest.as_os_str().is_empty() => {
            let sep = if env.os == Os::Windows { '\\' } else { '/' };
            format!("~{sep}{}", rest.display())
        }
        _ => path.display().to_string(),
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
    /// `path` with the home folder as `~`, for the help text.
    pub display_path: String,
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
            json!({ "type": "remote", "url": url, "oauth": false, "headers": { "Authorization": auth } }),
        ),
        Client::Codex => unreachable!("Codex uses TOML"),
    }
}

/// OpenCode 1.x entry, used only in files that already list 1.x servers
/// directly under `mcp`.
fn opencode_v1_entry(url: &str, token: &str) -> Value {
    json!({ "type": "remote", "url": url, "enabled": true, "headers": { "Authorization": bearer(token) } })
}

/// A file in the OpenCode 1.x shape: servers sit directly under `mcp`
/// (OpenCode 2 nests them under `mcp.servers`). An old `pr-nexus` entry on
/// its own does not count, so it is moved to the 2.x place.
fn opencode_is_v1(map: &Map<String, Value>) -> bool {
    let Some(Value::Object(mcp)) = map.get("mcp") else {
        return false;
    };
    !mcp.contains_key("servers")
        && mcp
            .iter()
            .any(|(name, value)| name != ENTRY && value.get("type").is_some())
}

/// Where the entry goes and what it holds, plus whether an old OpenCode 1.x
/// `mcp.pr-nexus` should be removed.
struct JsonPlan {
    keys: Vec<&'static str>,
    wanted: Value,
    drop_v1_entry: bool,
}

fn json_plan(client: Client, map: &Map<String, Value>, url: &str, token: &str) -> JsonPlan {
    let (parent, wanted) = json_entry(client, url, token);
    if client != Client::Opencode {
        return JsonPlan {
            keys: vec![parent, ENTRY],
            wanted,
            drop_v1_entry: false,
        };
    }
    if opencode_is_v1(map) {
        return JsonPlan {
            keys: vec!["mcp", ENTRY],
            wanted: opencode_v1_entry(url, token),
            drop_v1_entry: false,
        };
    }
    JsonPlan {
        keys: vec!["mcp", "servers", ENTRY],
        wanted,
        drop_v1_entry: map.get("mcp").and_then(|mcp| mcp.get(ENTRY)).is_some(),
    }
}

fn lookup<'a>(map: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    let (first, rest) = keys.split_first()?;
    rest.iter()
        .try_fold(map.get(*first)?, |value, key| value.get(*key))
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("Could not read {}: {err}", path.display())),
    }
}

/// JSON with comments and trailing commas, as VS Code, Cursor and OpenCode accept.
fn jsonc_options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        ..ParseOptions::default()
    }
}

fn parse_root(path: &Path, text: &str) -> Result<CstRootNode, String> {
    CstRootNode::parse(text, &jsonc_options()).map_err(|err| {
        format!(
            "{} has a syntax error ({err}); left unchanged. Fix the file or copy the snippet instead.",
            path.display()
        )
    })
}

/// Reads a JSON or JSONC file into its object.
fn parse_json(path: &Path, text: &str) -> Result<Map<String, Value>, String> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match parse_root(path, text)?.to_serde_value() {
        Some(Value::Object(map)) => Ok(map),
        _ => Err(format!(
            "{} is not a JSON object; left unchanged.",
            path.display()
        )),
    }
}

fn cst_value(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(cst_value).collect()),
        Value::Object(map) => CstInputValue::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), cst_value(value)))
                .collect(),
        ),
    }
}

/// Applies `plan` to a parsed object: what the file must read as afterwards.
fn apply_plan(map: &mut Map<String, Value>, plan: &JsonPlan, path: &Path) -> Result<(), String> {
    if plan.drop_v1_entry
        && let Some(Value::Object(mcp)) = map.get_mut("mcp")
    {
        mcp.shift_remove(ENTRY);
    }
    let (last, parents) = plan.keys.split_last().expect("keys are never empty");
    let mut current = map;
    for key in parents {
        let next = current
            .entry(*key)
            .or_insert_with(|| Value::Object(Map::new()));
        current = match next {
            Value::Object(inner) => inner,
            _ => {
                return Err(format!(
                    "{key} in {} is not an object; left unchanged.",
                    path.display()
                ));
            }
        };
    }
    current.insert((*last).to_owned(), plan.wanted.clone());
    Ok(())
}

/// Edits the file text in place: only the entry changes, comments and
/// formatting stay. The result is parsed again and must equal the old
/// content plus the entry, or nothing is written.
fn edit_jsonc(
    path: &Path,
    text: &str,
    plan: &JsonPlan,
    expected: &Map<String, Value>,
) -> Result<String, String> {
    let not_object = |key: &str| {
        format!(
            "{key} in {} is not an object; left unchanged.",
            path.display()
        )
    };
    let root = parse_root(path, text)?;
    let top: CstObject = root
        .object_value()
        .ok_or_else(|| format!("{} is not a JSON object; left unchanged.", path.display()))?;
    if plan.drop_v1_entry
        && let Some(old) = top
            .get("mcp")
            .and_then(|mcp| mcp.object_value())
            .and_then(|mcp| mcp.get(ENTRY))
    {
        old.remove();
    }
    let (last, parents) = plan.keys.split_last().expect("keys are never empty");
    let mut current = top;
    for key in parents {
        current = match current.get(key) {
            Some(prop) => prop.object_value().ok_or_else(|| not_object(key))?,
            None => current
                .object_value_or_create(key)
                .ok_or_else(|| not_object(key))?,
        };
    }
    match current.get(last) {
        Some(prop) => prop.set_value(cst_value(&plan.wanted)),
        None => {
            current.append(last, cst_value(&plan.wanted));
        }
    }
    let mut out = root.to_string();
    if text.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    match parse_json(path, &out) {
        Ok(after) if &after == expected => Ok(out),
        _ => Err(format!(
            "Could not add the entry to {} without changing other settings; left unchanged. Copy the snippet instead.",
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
    let plan = json_plan(client, &map, url, token);
    match lookup(&map, &plan.keys) {
        None if plan.drop_v1_entry => ClientStatus::Outdated,
        None => ClientStatus::NotAdded,
        Some(current) if !plan.drop_v1_entry && json_matches(client, current, &plan.wanted) => {
            ClientStatus::Added
        }
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
    let text = existing.unwrap_or("");
    let before = parse_json(path, text)?;
    let plan = json_plan(client, &before, url, token);
    let mut expected = before;
    apply_plan(&mut expected, &plan, path)?;
    if !text.trim().is_empty() {
        return edit_jsonc(path, text, &plan, &expected);
    }
    if client == Client::Opencode {
        let mut with_schema = Map::new();
        with_schema.insert(
            "$schema".to_owned(),
            json!("https://opencode.ai/config.json"),
        );
        with_schema.extend(expected);
        expected = with_schema;
    }
    let mut out =
        serde_json::to_string_pretty(&Value::Object(expected)).map_err(|err| err.to_string())?;
    out.push('\n');
    Ok(out)
}

/// What to try when the system opener fails on an existing file, usually
/// because no app is set for its extension (`.jsonc` on macOS): macOS opens
/// it in the default text editor, Windows in Notepad. Linux has no such
/// fallback beyond `xdg-open`.
pub fn text_editor_command(os: Os, path: &Path) -> Option<(&'static str, Vec<OsString>)> {
    match os {
        Os::Mac => Some((
            "/usr/bin/open",
            vec!["-t".into(), path.as_os_str().to_owned()],
        )),
        Os::Windows => Some(("notepad.exe", vec![path.as_os_str().to_owned()])),
        Os::Linux => None,
    }
}

/// The plain-English error when neither the opener nor the text editor worked.
pub fn open_failed_message(shown: &str, is_folder: bool) -> String {
    if is_folder {
        format!("Could not open the folder {shown}. Open it yourself, or copy the snippet instead.")
    } else {
        format!(
            "Could not open {shown}. No app is set to open this kind of file. Open it in a text editor yourself, or copy the snippet instead."
        )
    }
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
        display_path: display_path(&path, env),
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
            xdg_config: None,
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
            Path::new("/h/.config/opencode/opencode.jsonc")
        );
        assert_eq!(
            config_path(Client::Opencode, &mac),
            Path::new("/h/.config/opencode/opencode.jsonc")
        );
    }

    #[test]
    fn opencode_picks_the_file_opencode_reads_last() {
        let dir = tempfile::tempdir().unwrap();
        let e = env(dir.path(), Os::Mac);
        let oc = dir.path().join(".config/opencode");
        fs::create_dir_all(&oc).unwrap();
        assert_eq!(config_path(Client::Opencode, &e), oc.join("opencode.jsonc"));
        fs::write(oc.join("config.json"), "{}").unwrap();
        assert_eq!(config_path(Client::Opencode, &e), oc.join("config.json"));
        fs::write(oc.join("opencode.json"), "{}").unwrap();
        assert_eq!(config_path(Client::Opencode, &e), oc.join("opencode.json"));
        fs::write(oc.join("opencode.jsonc"), "{}").unwrap();
        assert_eq!(config_path(Client::Opencode, &e), oc.join("opencode.jsonc"));
    }

    #[test]
    fn opencode_follows_xdg_config_home() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = env(dir.path(), Os::Linux);
        e.xdg_config = Some(dir.path().join("xdg"));
        assert_eq!(
            config_path(Client::Opencode, &e),
            dir.path().join("xdg/opencode/opencode.jsonc")
        );
    }

    #[test]
    fn display_path_uses_a_tilde() {
        let e = env(Path::new("/Users/acme"), Os::Mac);
        assert_eq!(
            display_path(Path::new("/Users/acme/.config/opencode/opencode.jsonc"), &e),
            "~/.config/opencode/opencode.jsonc"
        );
        assert_eq!(display_path(Path::new("/etc/x.json"), &e), "/etc/x.json");
    }

    #[test]
    fn open_falls_back_to_a_text_editor() {
        let file = Path::new("/Users/acme/.config/opencode/opencode.jsonc");
        let (program, args) = text_editor_command(Os::Mac, file).unwrap();
        assert_eq!(program, "/usr/bin/open");
        assert_eq!(
            args,
            vec![OsString::from("-t"), file.as_os_str().to_owned()]
        );
        let (program, args) = text_editor_command(Os::Windows, file).unwrap();
        assert_eq!(program, "notepad.exe");
        assert_eq!(args, vec![file.as_os_str().to_owned()]);
        assert!(text_editor_command(Os::Linux, file).is_none());
        let message = open_failed_message("~/.config/opencode/opencode.jsonc", false);
        assert!(message.starts_with("Could not open ~/.config/opencode/opencode.jsonc."));
        assert!(!message.contains("ExitStatus"));
    }

    /// A real-looking OpenCode 2 file: comments, trailing commas, other servers.
    const OPENCODE_JSONC: &str = r#"{
  "$schema": "https://opencode.ai/config.json",
  // my default model
  "model": "acme/model-large",
  /* block comment */
  "mcp": {
    "servers": {
      "acme-docs": {
        "type": "remote",
        "url": "https://mcp.acme.test/mcp", // keep me
      },
    },
  },
}
"#;

    #[test]
    fn parses_jsonc_with_comments_and_trailing_commas() {
        let map = parse_json(Path::new("opencode.jsonc"), OPENCODE_JSONC).unwrap();
        assert_eq!(map["model"], "acme/model-large");
        assert_eq!(
            map["mcp"]["servers"]["acme-docs"]["url"],
            "https://mcp.acme.test/mcp"
        );
    }

    #[test]
    fn opencode_merge_keeps_comments_and_formatting() {
        let dir = tempfile::tempdir().unwrap();
        let e = env(dir.path(), Os::Mac);
        let oc = dir.path().join(".config/opencode");
        fs::create_dir_all(&oc).unwrap();
        fs::write(oc.join("opencode.jsonc"), OPENCODE_JSONC).unwrap();
        let path = config_path(Client::Opencode, &e);
        assert!(path.ends_with("opencode.jsonc"));
        assert_eq!(
            status(Client::Opencode, &path, URL, TOKEN),
            ClientStatus::NotAdded
        );
        install_file(Client::Opencode, &path, URL, TOKEN).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        for kept in [
            "// my default model",
            "/* block comment */",
            "// keep me",
            "\"acme/model-large\"",
        ] {
            assert!(text.contains(kept), "{kept} lost:\n{text}");
        }
        assert!(text.starts_with("{\n  \"$schema\""), "{text}");
        let map = parse_json(&path, &text).unwrap();
        let entry = &map["mcp"]["servers"]["pr-nexus"];
        assert_eq!(entry["type"], "remote");
        assert_eq!(entry["url"], URL);
        assert_eq!(entry["oauth"], false);
        assert_eq!(entry["headers"]["Authorization"], "Bearer pnx_example");
        assert_eq!(map["mcp"]["servers"]["acme-docs"]["type"], "remote");
        assert_eq!(
            status(Client::Opencode, &path, URL, TOKEN),
            ClientStatus::Added
        );
        assert_eq!(
            fs::read_to_string(backup_path(&path)).unwrap(),
            OPENCODE_JSONC
        );
        // Updating again only swaps the entry.
        install_file(Client::Opencode, &path, URL, "pnx_new").unwrap();
        let again = fs::read_to_string(&path).unwrap();
        assert!(
            again.contains("// keep me") && again.contains("Bearer pnx_new"),
            "{again}"
        );
        assert_eq!(again.matches("pr-nexus").count(), 1, "{again}");
    }

    #[test]
    fn opencode_adds_mcp_servers_to_a_file_without_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.jsonc");
        let before = "{\n  // theme\n  \"model\": \"acme/model\",\n}\n";
        fs::write(&path, before).unwrap();
        install_file(Client::Opencode, &path, URL, TOKEN).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("// theme"), "{text}");
        let map = parse_json(&path, &text).unwrap();
        assert_eq!(map["model"], "acme/model");
        assert_eq!(map["mcp"]["servers"]["pr-nexus"]["url"], URL);
    }

    #[test]
    fn opencode_v1_files_keep_the_v1_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.json");
        let before = r#"{"mcp":{"acme-docs":{"type":"remote","url":"https://mcp.acme.test/mcp","enabled":true}}}"#;
        fs::write(&path, before).unwrap();
        install_file(Client::Opencode, &path, URL, TOKEN).unwrap();
        let map = parse_json(&path, &fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(map["mcp"]["pr-nexus"]["enabled"], true);
        assert_eq!(map["mcp"]["pr-nexus"]["url"], URL);
        assert!(map["mcp"].get("servers").is_none());
        assert_eq!(
            status(Client::Opencode, &path, URL, TOKEN),
            ClientStatus::Added
        );
    }

    #[test]
    fn opencode_moves_an_old_v1_entry_to_mcp_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.jsonc");
        let before = format!(
            "{{\n  // mine\n  \"mcp\": {{\n    \"pr-nexus\": {}\n  }}\n}}\n",
            opencode_v1_entry(URL, TOKEN)
        );
        fs::write(&path, &before).unwrap();
        assert_eq!(
            status(Client::Opencode, &path, URL, TOKEN),
            ClientStatus::Outdated
        );
        install_file(Client::Opencode, &path, URL, TOKEN).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("// mine"), "{text}");
        let map = parse_json(&path, &text).unwrap();
        assert!(map["mcp"].get("pr-nexus").is_none(), "{text}");
        assert_eq!(map["mcp"]["servers"]["pr-nexus"]["url"], URL);
        assert_eq!(
            status(Client::Opencode, &path, URL, TOKEN),
            ClientStatus::Added
        );
    }

    #[test]
    fn new_opencode_file_has_the_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(Client::Opencode, &env(dir.path(), Os::Linux));
        install_file(Client::Opencode, &path, URL, TOKEN).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("{\n  \"$schema\": \"https://opencode.ai/config.json\""),
            "{text}"
        );
        assert_eq!(
            parse_json(&path, &text).unwrap()["mcp"]["servers"]["pr-nexus"]["url"],
            URL
        );
    }

    #[test]
    fn vscode_and_cursor_files_with_comments_are_edited_in_place() {
        for client in [Client::Vscode, Client::Cursor] {
            let dir = tempfile::tempdir().unwrap();
            let path = config_path(client, &env(dir.path(), Os::Linux));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let before = "{\n  // work servers\n  \"inputs\": [],\n}\n";
            fs::write(&path, before).unwrap();
            assert_eq!(status(client, &path, URL, TOKEN), ClientStatus::NotAdded);
            install_file(client, &path, URL, TOKEN).unwrap();
            let text = fs::read_to_string(&path).unwrap();
            assert!(text.contains("// work servers"), "{text}");
            assert_eq!(status(client, &path, URL, TOKEN), ClientStatus::Added);
        }
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
        let before = "{\n  // my servers\n  \"mcp\": {\n}\n";
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
