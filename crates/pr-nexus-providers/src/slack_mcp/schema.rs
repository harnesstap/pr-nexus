//! Typed parsers for Slack MCP tool results, and the tool schema pin (§7.11).
//!
//! Results arrive as text or JSON content blocks. Unknown fields are ignored.
//! A missing required field is [`ProviderError::Decode`] naming the tool.

use chrono::{Duration, Utc};
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::PostedMessage;
use pr_nexus_core::review::permalink;
use serde_json::Value;

pub const SEND_MESSAGE: &str = "slack_send_message";
pub const READ_CHANNEL: &str = "slack_read_channel";

/// Tools and input fields PR-Nexus relies on, as listed on 2026-10-25.
/// `slack_read_channel` is not pinned: it only backs the send-result
/// recovery, which degrades to "sent, link unknown" without it.
pub const PINNED_TOOLS: &[(&str, &[&str])] = &[(
    SEND_MESSAGE,
    &["channel_id", "message", "thread_ts", "reply_broadcast"],
)];

pub const SCHEMA_CHANGED: &str = "Slack MCP changed; update PR-Nexus";

/// One tool's name and input-schema property names, from `tools/list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolShape {
    pub name: String,
    pub properties: Vec<String>,
}

/// The first pinned tool or field missing from `tools/list`, if any.
pub fn pin_mismatch(tools: &[ToolShape]) -> Option<String> {
    for (name, fields) in PINNED_TOOLS {
        let Some(tool) = tools.iter().find(|tool| tool.name == *name) else {
            return Some(format!("{SCHEMA_CHANGED} (tool {name} is missing)"));
        };
        for field in *fields {
            if !tool.properties.iter().any(|property| property == field) {
                return Some(format!("{SCHEMA_CHANGED} ({name} has no {field} input)"));
            }
        }
    }
    None
}

/// The text and structured parts of one `tools/call` result.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolOutput {
    pub text: String,
    pub structured: Option<Value>,
    pub is_error: bool,
}

impl ToolOutput {
    /// Every JSON value in the result: the structured part, and the text
    /// block when it is itself JSON (the official server wraps its results
    /// as JSON text, with `\/`-escaped slashes).
    fn json_values(&self) -> Vec<Value> {
        let mut values: Vec<Value> = self.structured.iter().cloned().collect();
        if let Ok(value) = serde_json::from_str::<Value>(self.text.trim()) {
            values.push(value);
        }
        values
    }

    /// Text to search for a link: raw text plus every decoded JSON string.
    fn haystack(&self) -> String {
        let mut haystack = self.text.clone();
        for value in self.json_values() {
            collect_strings(&value, &mut haystack);
        }
        haystack
    }
}

fn collect_strings(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => {
            out.push('\n');
            out.push_str(text);
        }
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// First string under one of `keys`, searched depth-first, that `accept`s.
fn find_key<'a>(value: &'a Value, keys: &[&str], accept: &dyn Fn(&str) -> bool) -> Option<&'a str> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(Value::as_str).filter(|v| accept(v)) {
                    return Some(found);
                }
            }
            map.values().find_map(|item| find_key(item, keys, accept))
        }
        Value::Array(items) => items.iter().find_map(|item| find_key(item, keys, accept)),
        _ => None,
    }
}

fn is_channel_id(value: &str) -> bool {
    value.len() >= 3
        && value.starts_with(['C', 'G', 'D'])
        && value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// A `ts` written after a label in prose, e.g. `Message TS: 1700000000.000100`.
fn labelled_ts(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for label in ["message_ts", "message ts", "\"ts\"", "ts:", "ts="] {
        let mut from = 0;
        while let Some(offset) = lower[from..].find(label) {
            let after = from + offset + label.len();
            let candidate: String = text[after..]
                .trim_start_matches(|c: char| {
                    c.is_whitespace() || matches!(c, ':' | '=' | '"' | '`')
                })
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if permalink::is_ts(&candidate) {
                return Some(candidate);
            }
            from = after;
        }
    }
    None
}

/// Parse a `slack_send_message` result (§7.11).
///
/// The server's tool description says only "Return the message link to the
/// user"; no output schema is published. The result is a text block holding
/// JSON (`message_context.message_ts` / `channel_id`, slashes escaped) or
/// prose with the link. Order: a message link anywhere, then a `ts` field,
/// then a labelled `ts` in prose. The link is derived later from the
/// workspace domain when only `ts` is known.
pub fn parse_send(output: &ToolOutput, channel_id: &str) -> Result<PostedMessage, ProviderError> {
    if let Some(link) = permalink::find(&output.haystack()) {
        return Ok(PostedMessage {
            channel_id: link.channel_id.clone(),
            ts: link.ts.clone(),
            thread_ts: link.thread_ts.clone(),
            workspace: Some(link.workspace.clone()),
            permalink: Some(link.url),
        });
    }
    let values = output.json_values();
    let ts_keys = ["message_ts", "ts"];
    let ts = values
        .iter()
        .find_map(|value| find_key(value, &ts_keys, &permalink::is_ts))
        .map(str::to_owned)
        .or_else(|| labelled_ts(&output.haystack()))
        .ok_or_else(|| {
            ProviderError::Decode(format!(
                "{SEND_MESSAGE}: the result has no message link or ts"
            ))
        })?;
    let channel = values
        .iter()
        .find_map(|value| find_key(value, &["channel_id", "channel"], &is_channel_id))
        .unwrap_or(channel_id);
    let thread_ts = values
        .iter()
        .find_map(|value| find_key(value, &["thread_ts"], &permalink::is_ts))
        .filter(|root| *root != ts)
        .map(str::to_owned);
    Ok(PostedMessage {
        channel_id: channel.to_owned(),
        ts,
        thread_ts,
        permalink: None,
        workspace: None,
    })
}

/// One message from a `slack_read_channel` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadMessage {
    pub user_id: Option<String>,
    pub ts: String,
    pub text: String,
}

/// Parse a `slack_read_channel` result. The server returns JSON text
/// `{"messages": "<blocks>"}` where each block is
/// `=== Message from <name> (<user id>) at <time> ===` then `Message TS: <ts>`
/// then the message text.
pub fn parse_read_channel(output: &ToolOutput) -> Vec<ReadMessage> {
    let mut body = String::new();
    for value in output.json_values() {
        collect_strings(&value, &mut body);
    }
    if body.trim().is_empty() {
        body = output.text.clone();
    }
    let mut messages = Vec::new();
    for block in body.split("=== Message from ").skip(1) {
        let (header, rest) = block.split_once('\n').unwrap_or((block, ""));
        let user_id = header.rsplit_once(" at ").map_or(header, |(who, _)| who);
        let user_id = user_id
            .rsplit_once('(')
            .and_then(|(_, id)| id.strip_suffix(')'))
            .map(str::to_owned);
        let mut lines = rest.lines();
        let Some(ts) = lines
            .next()
            .and_then(|line| line.trim().strip_prefix("Message TS:"))
            .map(|ts| ts.trim().to_owned())
            .filter(|ts| permalink::is_ts(ts))
        else {
            continue;
        };
        messages.push(ReadMessage {
            user_id,
            ts,
            text: lines.collect::<Vec<_>>().join("\n"),
        });
    }
    messages
}

/// True when `posted` (Slack's stored text) is the message we sent as `sent`.
///
/// Slack rewrites links to `<url|label>` and appends a `Sent using <app>`
/// attribution, so this compares the URLs in `sent` and, without any, its
/// first line with punctuation dropped.
pub fn same_message(sent: &str, posted: &str) -> bool {
    let urls: Vec<&str> = sent
        .split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '|' | '(' | ')'))
        .filter_map(|word| word.strip_prefix("https://"))
        .map(|url| url.trim_end_matches(['.', ',', ']']))
        .filter(|url| !url.is_empty())
        .collect();
    if !urls.is_empty() {
        return urls.iter().all(|url| posted.contains(url));
    }
    let squash = |text: &str| -> String {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let first = sent.lines().map(squash).find(|line| !line.is_empty());
    first.is_some_and(|line| squash(posted).contains(&line))
}

/// Map a Slack error code found in a tool error or JSON-RPC error (§7.11).
pub fn map_error(tool: &str, text: &str, retry_after_secs: Option<u64>) -> ProviderError {
    let lower = text.to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|token| !token.is_empty())
        .collect();
    let has = |code: &str| tokens.contains(&code);
    if has("ratelimited") || has("rate_limited") || has("429") {
        let wait = retry_after_secs.unwrap_or(30).max(1);
        return ProviderError::RateLimited {
            reset_at: Utc::now() + Duration::seconds(wait as i64),
            secondary: false,
        };
    }
    if has("invalid_auth") || has("token_revoked") || has("not_authed") || has("401") {
        return ProviderError::Unauthorized(format!("{tool}: {}", short(text)));
    }
    if has("missing_scope") {
        return ProviderError::Forbidden(format!("{tool}: {}", short(text)));
    }
    for code in [
        "not_in_channel",
        "channel_not_found",
        "is_archived",
        "msg_too_long",
    ] {
        if has(code) {
            return ProviderError::Api {
                code: code.to_owned(),
                message: format!("{tool}: {}", short(text)),
            };
        }
    }
    ProviderError::Api {
        code: "tool_error".to_owned(),
        message: format!("{tool}: {}", short(text)),
    }
}

fn short(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(300) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> ToolOutput {
        ToolOutput {
            text: value.to_owned(),
            ..ToolOutput::default()
        }
    }

    #[test]
    fn send_result_link_gives_ts_and_thread() {
        let posted = parse_send(
            &text("Message sent: https://acme.slack.com/archives/C0TEAM/p1700000500000200?thread_ts=1700000000.000100"),
            "C0TEAM",
        )
        .unwrap();
        assert_eq!(posted.ts, "1700000500.000200");
        assert_eq!(posted.thread_ts.as_deref(), Some("1700000000.000100"));
        assert_eq!(posted.workspace.as_deref(), Some("acme"));

        let structured = ToolOutput {
            structured: Some(
                serde_json::json!({"permalink": "https://acme.slack.com/archives/C0TEAM/p1700000000000100"}),
            ),
            ..ToolOutput::default()
        };
        assert_eq!(
            parse_send(&structured, "C0TEAM").unwrap().ts,
            "1700000000.000100"
        );
    }

    #[test]
    fn send_result_without_link_falls_back_to_ts_or_fails() {
        let posted = parse_send(&text(r#"{"ok":true,"ts":"1700000000.000100"}"#), "C1").unwrap();
        assert_eq!(posted.channel_id, "C1");
        assert_eq!(posted.permalink, None);
        let err = parse_send(&text("done"), "C1").unwrap_err();
        assert!(matches!(err, ProviderError::Decode(message) if message.contains(SEND_MESSAGE)));
    }

    #[test]
    fn send_result_json_text_with_message_context() {
        let output = text(
            r#"{"message_context":{"message_ts":"1700000000.000100","channel_id":"D0123456789"},"result":"Message sent"}"#,
        );
        let posted = parse_send(&output, "D0123456789").unwrap();
        assert_eq!(posted.ts, "1700000000.000100");
        assert_eq!(posted.channel_id, "D0123456789");
        assert_eq!(posted.permalink, None);
    }

    #[test]
    fn send_result_escaped_dm_link_and_prose_ts() {
        let output = text(
            r#"{"result":"Sent","message_link":"https:\/\/acme.slack.com\/archives\/D0123456789\/p1700000000000100"}"#,
        );
        let posted = parse_send(&output, "D0123456789").unwrap();
        assert_eq!(
            posted.permalink.as_deref(),
            Some("https://acme.slack.com/archives/D0123456789/p1700000000000100")
        );
        let prose = parse_send(
            &text("Message sent to DM.\nMessage TS: 1700000000.000100"),
            "D01",
        )
        .unwrap();
        assert_eq!(prose.ts, "1700000000.000100");
    }

    #[test]
    fn read_channel_blocks_parse_and_match_the_sent_text() {
        let output = text(
            r#"{"messages":"Channel: DM (D0123456789)\n\n=== Message from Ada Acme (U0ACME0001) at 2026-01-01 10:00:00 UTC === \nMessage TS: 1700000000.000100\nHello team\n<https:\/\/github.com\/acme\/web\/pull\/1|github.com\/acme\/web\/pull\/1>\n*Sent using* <@U0ACMEAPP|PR-Nexus>","pagination_info":"There are no more messages available.\n"}"#,
        );
        let messages = parse_read_channel(&output);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].user_id.as_deref(), Some("U0ACME0001"));
        assert_eq!(messages[0].ts, "1700000000.000100");
        let sent = "Hello team\nCould someone review [Fix login](https://github.com/acme/web/pull/1)?\nThanks!";
        assert!(same_message(sent, &messages[0].text));
        assert!(!same_message(
            "Hello team\nhttps://github.com/acme/web/pull/2",
            &messages[0].text
        ));
        assert!(same_message("Hello team", &messages[0].text));
    }

    #[test]
    fn errors_map_to_provider_errors() {
        assert!(matches!(
            map_error("t", "ratelimited", None),
            ProviderError::RateLimited { .. }
        ));
        assert!(matches!(
            map_error("t", "invalid_auth", None),
            ProviderError::Unauthorized(_)
        ));
        assert!(matches!(
            map_error("t", "token_revoked", None),
            ProviderError::Unauthorized(_)
        ));
        assert!(matches!(
            map_error("t", "missing_scope: chat:write", None),
            ProviderError::Forbidden(_)
        ));
        assert!(matches!(
            map_error("t", "Error: not_in_channel", None),
            ProviderError::Api { code, .. } if code == "not_in_channel"
        ));
        assert!(matches!(
            map_error("t", "channel_not_found", None),
            ProviderError::Api { code, .. } if code == "channel_not_found"
        ));
    }

    #[test]
    fn pin_detects_missing_tools_and_fields() {
        let good = vec![ToolShape {
            name: SEND_MESSAGE.into(),
            properties: [
                "channel_id",
                "message",
                "thread_ts",
                "reply_broadcast",
                "unfurl_app_links",
            ]
            .map(String::from)
            .to_vec(),
        }];
        assert_eq!(pin_mismatch(&good), None);
        assert!(pin_mismatch(&[]).unwrap().contains("missing"));
        let renamed = vec![ToolShape {
            name: SEND_MESSAGE.into(),
            properties: vec!["channel".into(), "text".into()],
        }];
        assert!(pin_mismatch(&renamed).unwrap().contains("channel_id"));
    }
}
