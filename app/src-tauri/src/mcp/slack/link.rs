//! The Slack link of a write PR Nexus made for an agent, for Activity.
//!
//! Slack's reply usually carries the message permalink; it is reused as is
//! (with the thread added when Slack left it out). Otherwise the link is
//! built from the workspace domain, the channel and the message `ts`, as
//! Slack's "Copy link" does. A reaction links to the message it was added
//! to. A scheduled message has no link until it is posted; a draft links
//! only when Slack returns one.

use pr_nexus_core::review::permalink;
use rmcp::model::JsonObject;
use serde_json::Value;

/// The link for `tool` sent with `arguments` (bare channel ID) to the
/// workspace at `domain` (`acme`, or `acme.enterprise` on Grid), given
/// Slack's `reply`.
pub fn for_write(
    domain: &str,
    tool: &str,
    arguments: &JsonObject,
    reply: &Value,
) -> Option<String> {
    let text = |key: &str| arguments.get(key).and_then(Value::as_str);
    let channel = text("channel_id")?;
    let thread = text("thread_ts").filter(|ts| permalink::is_ts(ts));
    let reply_text = match reply {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
    // JSON-encoding may have escaped Slack's own `\/` once more.
    .replace("\\\\/", "/")
    .replace("\\/", "/");
    match tool {
        "slack_add_reaction" => {
            let ts = text("message_ts").filter(|ts| permalink::is_ts(ts))?;
            Some(build(domain, channel, ts, None))
        }
        "slack_send_message_draft" => permalink::find(&reply_text)
            .map(|link| link.url)
            .or_else(|| draft_link(&reply_text)),
        "slack_send_message" | "slack_schedule_message" => {
            if let Some(link) = permalink::find(&reply_text) {
                return Some(match (thread, &link.thread_ts) {
                    (Some(root), None) if root != link.ts => {
                        permalink::build(&link.workspace, &link.channel_id, &link.ts, Some(root))
                    }
                    _ => link.url,
                });
            }
            if tool == "slack_schedule_message" {
                return None;
            }
            let ts = message_ts(&reply_text)?;
            Some(build(domain, channel, &ts, thread))
        }
        _ => None,
    }
}

fn build(domain: &str, channel: &str, ts: &str, thread: Option<&str>) -> String {
    // A qualified ID (`T…/C…`) names the workspace; the link wants the bare one.
    let channel = channel.rsplit('/').next().unwrap_or(channel);
    permalink::build(domain, channel, ts, thread)
}

/// The posted message's `ts` from Slack's reply ("Message TS: …" or `"ts"`).
fn message_ts(text: &str) -> Option<String> {
    let text = text.replace("\\\"", "\"");
    [
        "Message TS:",
        "\"ts\":\"",
        "\"ts\": \"",
        "\"message_ts\":\"",
    ]
    .iter()
    .find_map(|marker| {
        let start = text.find(marker)? + marker.len();
        let candidate: String = text[start..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        permalink::is_ts(&candidate).then_some(candidate)
    })
}

/// `https://app.slack.com/client/…`: where Slack opens a draft.
fn draft_link(text: &str) -> Option<String> {
    let text = text.replace("\\/", "/");
    let start = text.find("https://app.slack.com/client/")?;
    let link: String = text[start..]
        .chars()
        .take_while(|c| {
            !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>' | ')' | ']' | '|' | ',')
        })
        .collect();
    Some(link)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn args(value: Value) -> JsonObject {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_top_level_message_is_built_from_domain_channel_and_ts() {
        let reply = json!({ "result": "Message sent. Message TS: 1700000000.000100" });
        let link = for_write(
            "acme",
            "slack_send_message",
            &args(json!({ "channel_id": "C0123456789" })),
            &reply,
        );
        assert_eq!(
            link.as_deref(),
            Some("https://acme.slack.com/archives/C0123456789/p1700000000000100")
        );
    }

    #[test]
    fn a_thread_reply_names_its_parent() {
        let reply = json!({ "ts": "1700000500.000200" });
        let arguments =
            args(json!({ "channel_id": "C0123456789", "thread_ts": "1700000000.000100" }));
        assert_eq!(
            for_write("acme", "slack_send_message", &arguments, &reply).as_deref(),
            Some(
                "https://acme.slack.com/archives/C0123456789/p1700000500000200?thread_ts=1700000000.000100&cid=C0123456789"
            )
        );
    }

    #[test]
    fn slacks_own_permalink_wins_and_gains_the_thread() {
        let reply = json!({ "result": "Message TS: 1700000500.000200\nPermalink: https:\\/\\/acme-labs.slack.com\\/archives\\/C0123456789\\/p1700000500000200" });
        let top = for_write(
            "other",
            "slack_send_message",
            &args(json!({ "channel_id": "C0123456789" })),
            &reply,
        );
        assert_eq!(
            top.as_deref(),
            Some("https://acme-labs.slack.com/archives/C0123456789/p1700000500000200")
        );
        let arguments =
            args(json!({ "channel_id": "C0123456789", "thread_ts": "1700000000.000100" }));
        assert_eq!(
            for_write("other", "slack_send_message", &arguments, &reply).as_deref(),
            Some(
                "https://acme-labs.slack.com/archives/C0123456789/p1700000500000200?thread_ts=1700000000.000100&cid=C0123456789"
            )
        );
    }

    #[test]
    fn grid_uses_the_enterprise_host_and_the_bare_channel_id() {
        let reply = json!({ "result": "Message TS: 1700000000.000100" });
        let arguments = args(json!({ "channel_id": "E0ACMELAB1/C0123456789" }));
        assert_eq!(
            for_write(
                "acme-labs.enterprise",
                "slack_send_message",
                &arguments,
                &reply
            )
            .as_deref(),
            Some("https://acme-labs.enterprise.slack.com/archives/C0123456789/p1700000000000100")
        );
    }

    #[test]
    fn a_reaction_links_to_the_reacted_message() {
        let arguments = args(
            json!({ "channel_id": "C0123456789", "message_ts": "1700000000.000100", "emoji": "eyes" }),
        );
        assert_eq!(
            for_write(
                "acme",
                "slack_add_reaction",
                &arguments,
                &json!({ "result": "Reaction added." })
            )
            .as_deref(),
            Some("https://acme.slack.com/archives/C0123456789/p1700000000000100")
        );
    }

    #[test]
    fn schedules_and_drafts_link_only_when_slack_says_where() {
        let arguments = args(json!({ "channel_id": "C0123456789" }));
        let scheduled = json!({ "result": "Message scheduled. Scheduled message ID: Q0123" });
        assert_eq!(
            for_write("acme", "slack_schedule_message", &arguments, &scheduled),
            None
        );
        let draft =
            json!({ "channel_link": "https://app.slack.com/client/T0ACME0001/C0123456789" });
        assert_eq!(
            for_write("acme", "slack_send_message_draft", &arguments, &draft).as_deref(),
            Some("https://app.slack.com/client/T0ACME0001/C0123456789")
        );
        assert_eq!(
            for_write("acme", "slack_send_message_draft", &arguments, &json!({})),
            None
        );
    }
}
