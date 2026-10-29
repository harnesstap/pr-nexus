//! Workspace-qualified Slack IDs: `T0ACME0001/C0ACME0042`.
//!
//! The prefix is the team (`T…`), or the Enterprise Grid org (`E…`) when
//! the session is a Grid one: Grid search spans the org's workspaces and
//! its results do not name the team, so the org is the most precise
//! prefix PR Nexus can vouch for (no `conversations.info` lookups).

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// A Slack object ID: channel, group, DM, user, bot, file (canvases and
/// lists are files too).
const ID: &str = r"[CGDUWBF][A-Z0-9]{8,}";

static IN_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?P<pre>\*\*ID:\*\* |\(ID: |User ID: |\(|<@|<#)(?P<id>{ID})\b"
    ))
    .expect("id pattern")
});
static QUALIFIED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^(?P<q>[TE][A-Z0-9]{{6,}})/(?P<id>{ID})$")).expect("qualified")
});
static BARE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!("^{ID}$")).expect("bare"));

/// `text` with every Slack ID it names prefixed with `qualifier/`. Only IDs
/// in the places the official server prints them are touched, so words in
/// message text stay as written.
pub fn qualify_text(qualifier: &str, text: &str) -> String {
    IN_TEXT
        .replace_all(text, |caps: &Captures| {
            format!("{}{qualifier}/{}", &caps["pre"], &caps["id"])
        })
        .into_owned()
}

/// An ID argument split into its qualifier (if any) and the bare ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdRef {
    pub qualifier: Option<String>,
    pub id: String,
}

pub fn parse(value: &str) -> Option<IdRef> {
    let value = value.trim();
    if let Some(caps) = QUALIFIED.captures(value) {
        return Some(IdRef {
            qualifier: Some(caps["q"].to_owned()),
            id: caps["id"].to_owned(),
        });
    }
    BARE.is_match(value).then(|| IdRef {
        qualifier: None,
        id: value.to_owned(),
    })
}

/// A Slack message permalink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permalink {
    /// `acme` from `acme.slack.com`; for Grid, the org label of
    /// `acme.enterprise.slack.com`.
    pub label: String,
    pub enterprise: bool,
    pub channel_id: String,
    pub message_ts: Option<String>,
    pub thread_ts: Option<String>,
}

static PERMALINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^https://(?P<label>[a-z0-9-]+)(?P<ent>\.enterprise)?\.slack\.com/archives/(?P<channel>{ID})(?:/p(?P<sec>\d{{10}})(?P<usec>\d{{6}}))?/?(?:\?(?P<query>\S*))?$"
    ))
    .expect("permalink")
});
static THREAD_TS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|&)thread_ts=(?P<ts>\d{10}\.\d{6})").expect("thread_ts"));

pub fn parse_permalink(url: &str) -> Option<Permalink> {
    let caps = PERMALINK.captures(url.trim())?;
    let thread_ts = caps
        .name("query")
        .and_then(|query| THREAD_TS.captures(query.as_str()))
        .map(|ts| ts["ts"].to_owned());
    Some(Permalink {
        label: caps["label"].to_owned(),
        enterprise: caps.name("ent").is_some(),
        channel_id: caps["channel"].to_owned(),
        message_ts: caps
            .name("sec")
            .zip(caps.name("usec"))
            .map(|(sec, usec)| format!("{}.{}", sec.as_str(), usec.as_str())),
        thread_ts,
    })
}
