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

/// Tools and input fields PR-Nexus relies on, as listed on 2026-10-25.
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
    fn haystack(&self) -> String {
        match &self.structured {
            Some(value) => format!("{value}\n{}", self.text),
            None => self.text.clone(),
        }
    }
}

/// Parse a `slack_send_message` result. The link is required: it carries
/// `ts`, and for a reply `thread_ts`.
pub fn parse_send(output: &ToolOutput, channel_id: &str) -> Result<PostedMessage, ProviderError> {
    let haystack = output.haystack();
    if let Some(link) = permalink::find(&haystack) {
        return Ok(PostedMessage {
            channel_id: link.channel_id.clone(),
            ts: link.ts.clone(),
            thread_ts: link.thread_ts.clone(),
            workspace: Some(link.workspace.clone()),
            permalink: Some(link.url),
        });
    }
    let json = output
        .structured
        .clone()
        .or_else(|| serde_json::from_str::<Value>(output.text.trim()).ok());
    let ts = json
        .as_ref()
        .and_then(|value| value.get("ts").or_else(|| value.pointer("/message/ts")))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProviderError::Decode(format!(
                "{SEND_MESSAGE}: the result has no message link or ts"
            ))
        })?;
    Ok(PostedMessage {
        channel_id: channel_id.to_owned(),
        ts: ts.to_owned(),
        thread_ts: None,
        permalink: None,
        workspace: None,
    })
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
