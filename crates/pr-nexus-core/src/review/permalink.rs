//! Slack message links: `https://<domain>.slack.com/archives/<channel>/p<16 digits>`.
//!
//! `slack_send_message` returns the link of the posted message (§7.11). The
//! message `ts` is the 16 digits with a dot before the last six, and a thread
//! reply carries its root in `?thread_ts=`. The channel can be a public (C),
//! private (G) or DM (D) channel. Links inside JSON text may have escaped
//! slashes (`https:\/\/…`); [`find`] accepts both.

/// A parsed Slack message link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permalink {
    pub url: String,
    /// Workspace domain without `.slack.com`, e.g. `acme`.
    pub workspace: String,
    pub channel_id: String,
    pub ts: String,
    pub thread_ts: Option<String>,
}

/// Parse one Slack message link. Returns `None` for anything else.
pub fn parse(url: &str) -> Option<Permalink> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    let workspace = host.strip_suffix(".slack.com")?;
    if workspace.is_empty() || !workspace.chars().all(is_domain_char) {
        return None;
    }
    let (path, query) = match path.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (path, None),
    };
    let mut parts = path.split('/');
    if parts.next()? != "archives" {
        return None;
    }
    let channel_id = parts.next()?;
    if channel_id.is_empty() || !channel_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let ts = parts.next().and_then(ts_from_segment)?;
    if parts.next().is_some_and(|extra| !extra.is_empty()) {
        return None;
    }
    let thread_ts = query.and_then(|query| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == "thread_ts" && is_ts(value)).then(|| value.to_owned())
        })
    });
    Some(Permalink {
        url: url.to_owned(),
        workspace: workspace.to_ascii_lowercase(),
        channel_id: channel_id.to_owned(),
        ts,
        thread_ts,
    })
}

/// First Slack message link found anywhere in `text` (tool output, JSON, prose).
pub fn find(text: &str) -> Option<Permalink> {
    let unescaped;
    let text = if text.contains("\\/") {
        unescaped = text.replace("\\/", "/");
        unescaped.as_str()
    } else {
        text
    };
    let mut start = 0;
    while let Some(offset) = text[start..].find("https://") {
        let begin = start + offset;
        let end = text[begin..]
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']' | '|' | ',')
            })
            .map_or(text.len(), |len| begin + len);
        if let Some(link) = parse(&text[begin..end]) {
            return Some(link);
        }
        start = begin + "https://".len();
    }
    None
}

/// The message link for `ts` in `channel_id`, as Slack's "Copy link" builds it.
pub fn build(workspace: &str, channel_id: &str, ts: &str, thread_ts: Option<&str>) -> String {
    let digits: String = ts.chars().filter(char::is_ascii_digit).collect();
    let mut url = format!("https://{workspace}.slack.com/archives/{channel_id}/p{digits}");
    if let Some(root) = thread_ts.filter(|root| *root != ts) {
        url.push_str(&format!("?thread_ts={root}&cid={channel_id}"));
    }
    url
}

fn ts_from_segment(segment: &str) -> Option<String> {
    let digits = segment.strip_prefix('p')?;
    if digits.len() != 16 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}.{}", &digits[..10], &digits[10..]))
}

/// `1700000000.000100`: seconds, a dot, six digits.
pub fn is_ts(value: &str) -> bool {
    value.split_once('.').is_some_and(|(secs, micros)| {
        !secs.is_empty()
            && micros.len() == 6
            && secs
                .bytes()
                .chain(micros.bytes())
                .all(|b| b.is_ascii_digit())
    })
}

/// Letters, digits, `-`, and `.` for Enterprise Grid hosts like `acme.enterprise`.
fn is_domain_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '.'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_top_level_message_link() {
        let link = parse("https://acme.slack.com/archives/C0123ABCD/p1700000000000100").unwrap();
        assert_eq!(link.workspace, "acme");
        assert_eq!(link.channel_id, "C0123ABCD");
        assert_eq!(link.ts, "1700000000.000100");
        assert_eq!(link.thread_ts, None);
    }

    #[test]
    fn parses_a_thread_reply_link() {
        let link = parse(
            "https://acme-dev.slack.com/archives/C0123ABCD/p1700000500000200?thread_ts=1700000000.000100&cid=C0123ABCD",
        )
        .unwrap();
        assert_eq!(link.workspace, "acme-dev");
        assert_eq!(link.ts, "1700000500.000200");
        assert_eq!(link.thread_ts.as_deref(), Some("1700000000.000100"));
    }

    #[test]
    fn rejects_other_links() {
        for url in [
            "https://github.com/acme/web/pull/1",
            "http://acme.slack.com/archives/C1/p1700000000000100",
            "https://acme.slack.com/archives/C1/p170000000000010",
            "https://acme.slack.com/messages/C1/p1700000000000100",
            "https://evil.com/acme.slack.com/archives/C1/p1700000000000100",
        ] {
            assert!(parse(url).is_none(), "{url}");
        }
    }

    #[test]
    fn finds_a_link_inside_tool_output() {
        let text =
            r#"{"ok":true,"message_link":"https://acme.slack.com/archives/C9/p1700000000000100"}"#;
        assert_eq!(find(text).unwrap().channel_id, "C9");
        let prose = "Sent. See https://github.com/x and <https://acme.slack.com/archives/C9/p1700000000000100>.";
        assert_eq!(find(prose).unwrap().ts, "1700000000.000100");
        assert!(find("no link here").is_none());
    }

    #[test]
    fn finds_dm_links_with_escaped_slashes() {
        let text = r#"{"message_link":"https:\/\/acme.slack.com\/archives\/D0123456789\/p1700000000000100"}"#;
        let link = find(text).unwrap();
        assert_eq!(link.channel_id, "D0123456789");
        assert_eq!(link.ts, "1700000000.000100");
        assert_eq!(
            link.url,
            "https://acme.slack.com/archives/D0123456789/p1700000000000100"
        );
        let grid =
            find("https://acme.enterprise.slack.com/archives/G01/p1700000000000100").unwrap();
        assert_eq!(grid.workspace, "acme.enterprise");
    }

    #[test]
    fn builds_links_that_parse_back() {
        let url = build("acme", "D0123456789", "1700000000.000100", None);
        assert_eq!(
            url,
            "https://acme.slack.com/archives/D0123456789/p1700000000000100"
        );
        let reply = build(
            "acme",
            "C01",
            "1700000500.000200",
            Some("1700000000.000100"),
        );
        let link = parse(&reply).unwrap();
        assert_eq!(link.ts, "1700000500.000200");
        assert_eq!(link.thread_ts.as_deref(), Some("1700000000.000100"));
    }
}
