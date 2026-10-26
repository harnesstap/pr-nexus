//! Byte-exact port of the review-message script, plus the Slack wire escapes.
//!
//! [`build`] returns the text stored in `review_messages.text` and shown in the
//! preview. [`md_escape_titles`] is the only transform applied before
//! `slack_send_message`. [`mrkdwn_escape`] is the Web API fallback and is not
//! applied on the MCP path.

use crate::error::CoreError;

/// First ask, or a follow-up in the existing thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageMode {
    Request,
    Remind,
}

/// Build the Slack message for one or more pull requests.
///
/// Titles and URLs are trimmed. An empty title is an error. After trimming, a
/// URL must start with `http`. A single item uses the single-PR wording. Two
/// or more use the batch wording. Reminders never include `requester`.
pub fn build(
    mode: MessageMode,
    items: &[(&str, &str)],
    requester: Option<&str>,
    extra: Option<&str>,
) -> Result<String, CoreError> {
    if items.is_empty() {
        return Err(CoreError::Validation(
            "at least one pull request is required".into(),
        ));
    }
    let mut cleaned = Vec::with_capacity(items.len());
    for (title, url) in items {
        let title = title.trim();
        let url = url.trim();
        if title.is_empty() {
            return Err(CoreError::Validation("title must be non-empty".into()));
        }
        if !url.starts_with("http") {
            return Err(CoreError::Validation(format!(
                "url must be an http(s) URL, got: {url}"
            )));
        }
        cleaned.push((title, url));
    }

    let who = match requester {
        Some(name) if !name.is_empty() => format!(" ({name})"),
        Some(_) | None => String::new(),
    };

    let mut lines = match mode {
        MessageMode::Request if cleaned.len() == 1 => {
            let (title, url) = cleaned[0];
            vec![
                "Hello team".to_string(),
                format!("Could someone please review {title}?{who}"),
                url.to_string(),
                "Thanks!".to_string(),
            ]
        }
        MessageMode::Request => {
            let mut lines = vec![
                "Hello team".to_string(),
                format!("Could someone please review these pull requests?{who}"),
            ];
            push_pr_lines(&mut lines, &cleaned);
            lines.push("Thanks!".to_string());
            lines
        }
        MessageMode::Remind if cleaned.len() == 1 => {
            let (title, url) = cleaned[0];
            vec![
                format!(
                    "Hello, I'm still looking for a review on {title} when you have a moment :pray:"
                ),
                url.to_string(),
                "Thanks!".to_string(),
            ]
        }
        MessageMode::Remind => {
            let mut lines = vec![
                "Hello, I'm still looking for a review on these pull requests when you have a moment :pray:"
                    .to_string(),
            ];
            push_pr_lines(&mut lines, &cleaned);
            lines.push("Thanks!".to_string());
            lines
        }
    };
    append_extra(&mut lines, extra);
    Ok(finish(lines))
}

fn push_pr_lines(lines: &mut Vec<String>, items: &[(&str, &str)]) {
    for (title, url) in items {
        lines.push(format!("- {title}"));
        lines.push((*url).to_string());
    }
}

fn append_extra(lines: &mut Vec<String>, extra: Option<&str>) {
    if let Some(extra) = extra.filter(|extra| !extra.is_empty()) {
        lines.push(String::new());
        lines.push(extra.trim().to_string());
    }
}

fn finish(lines: Vec<String>) -> String {
    let mut text = lines.join("\n");
    let end = text.trim_end().len();
    text.truncate(end);
    text.push('\n');
    text
}

/// Backslash-escape Markdown-active characters inside titles only.
///
/// Active characters are `\ * _ ` ~ [ ] < >`. A title that starts with `#`,
/// `+`, `-`, or an ordered-list marker (`N.`) escapes that marker too. `>` is
/// already active, so a leading `>` is escaped once. URLs, template lines, and
/// the extra note (everything after `Thanks!`) are left alone.
pub fn md_escape_titles(text: &str) -> String {
    let ended_with_newline = text.ends_with('\n');
    let mut lines: Vec<&str> = text.split('\n').collect();
    if ended_with_newline && lines.last() == Some(&"") {
        lines.pop();
    }
    let mut out = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        if *line == "Thanks!" {
            out.extend(lines[index..].iter().map(|line| (*line).to_string()));
            break;
        }
        let next = lines.get(index + 1).copied();
        out.push(escape_line(line, next));
    }
    let mut text = out.join("\n");
    if ended_with_newline {
        text.push('\n');
    }
    text
}

fn escape_line(line: &str, next: Option<&str>) -> String {
    let next_is_bullet = next.is_some_and(|line| line.starts_with("- "));
    const BATCH_REQUEST: &str = "Could someone please review these pull requests?";
    if (line == BATCH_REQUEST || line.starts_with(&format!("{BATCH_REQUEST} ("))) && next_is_bullet
    {
        return line.to_string();
    }
    const SINGLE_PREFIX: &str = "Could someone please review ";
    if let Some(rest) = line.strip_prefix(SINGLE_PREFIX)
        && let Some((title, who)) = split_request_title(rest)
    {
        return format!("{SINGLE_PREFIX}{}?{who}", escape_title(title));
    }
    const REMIND_PREFIX: &str = "Hello, I'm still looking for a review on ";
    const REMIND_SUFFIX: &str = " when you have a moment :pray:";
    if let Some(rest) = line.strip_prefix(REMIND_PREFIX)
        && let Some(title) = rest.strip_suffix(REMIND_SUFFIX)
        && !(title == "these pull requests" && next_is_bullet)
    {
        return format!("{REMIND_PREFIX}{}{REMIND_SUFFIX}", escape_title(title));
    }
    if let Some(title) = line.strip_prefix("- ") {
        return format!("- {}", escape_title(title));
    }
    line.to_string()
}

/// Split `{title}?{who}` where `who` is empty or ` (requester)`.
fn split_request_title(rest: &str) -> Option<(&str, &str)> {
    let mut found = None;
    for (index, ch) in rest.char_indices() {
        if ch != '?' {
            continue;
        }
        let suffix = &rest[index + ch.len_utf8()..];
        let who = suffix.is_empty() || (suffix.starts_with(" (") && suffix.ends_with(')'));
        if who {
            found = Some(index);
        }
    }
    let index = found?;
    Some((&rest[..index], &rest[index + '?'.len_utf8()..]))
}

fn escape_title(title: &str) -> String {
    let chars: Vec<char> = title.chars().collect();
    let mut digit_end = 0;
    while digit_end < chars.len() && chars[digit_end].is_ascii_digit() {
        digit_end += 1;
    }
    let escape_dot_at =
        (digit_end > 0 && digit_end < chars.len() && chars[digit_end] == '.').then_some(digit_end);
    let escape_first = matches!(chars.first().copied(), Some('#' | '+' | '-'));
    let mut out = String::with_capacity(title.len());
    for (index, ch) in chars.into_iter().enumerate() {
        let active = matches!(ch, '\\' | '*' | '_' | '`' | '~' | '[' | ']' | '<' | '>');
        let leading = (index == 0 && escape_first) || escape_dot_at == Some(index);
        if active || leading {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Entity-escape `&`, `<`, and `>` for the Slack Web API fallback.
///
/// The MCP send tool takes standard Markdown and would show these entities
/// literally, so callers must not apply this on that path.
pub fn mrkdwn_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Case {
        name: &'static str,
        mode: MessageMode,
        items: &'static [(&'static str, &'static str)],
        requester: Option<&'static str>,
        extra: Option<&'static str>,
    }

    fn case(
        name: &'static str,
        mode: MessageMode,
        items: &'static [(&'static str, &'static str)],
        requester: Option<&'static str>,
        extra: Option<&'static str>,
    ) -> Case {
        Case {
            name,
            mode,
            items,
            requester,
            extra,
        }
    }

    #[test]
    fn golden_fixtures_match_the_python_script() {
        let fixtures: &[Case] = &[
            case(
                "01-single-request",
                MessageMode::Request,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/1",
                )],
                None,
                None,
            ),
            case(
                "02-single-request-requester",
                MessageMode::Request,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/1",
                )],
                Some("alice"),
                None,
            ),
            case(
                "03-single-request-extra",
                MessageMode::Request,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/1",
                )],
                None,
                Some("  Please look at the latch.  "),
            ),
            case(
                "04-single-request-requester-extra",
                MessageMode::Request,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/2",
                )],
                Some("alice"),
                Some("Needs a second look"),
            ),
            case(
                "05-single-remind",
                MessageMode::Remind,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/1",
                )],
                None,
                None,
            ),
            case(
                "06-single-remind-requester-ignored",
                MessageMode::Remind,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/1",
                )],
                Some("alice"),
                None,
            ),
            case(
                "07-single-remind-extra",
                MessageMode::Remind,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/3",
                )],
                None,
                Some("Still waiting on CI"),
            ),
            case(
                "08-batch-request",
                MessageMode::Request,
                &[
                    (
                        "Fix the widget latch",
                        "https://github.com/acme/widgets/pull/1",
                    ),
                    (
                        "Add the widget gauge",
                        "https://github.com/acme/widgets/pull/2",
                    ),
                ],
                None,
                None,
            ),
            case(
                "09-batch-request-requester",
                MessageMode::Request,
                &[
                    (
                        "Fix the widget latch",
                        "https://github.com/acme/widgets/pull/1",
                    ),
                    (
                        "Add the widget gauge",
                        "https://github.com/acme/widgets/pull/2",
                    ),
                ],
                Some("alice"),
                None,
            ),
            case(
                "10-batch-remind",
                MessageMode::Remind,
                &[
                    (
                        "Fix the widget latch",
                        "https://github.com/acme/widgets/pull/1",
                    ),
                    (
                        "Add the widget gauge",
                        "https://github.com/acme/widgets/pull/2",
                    ),
                ],
                None,
                None,
            ),
            case(
                "11-batch-remind-requester-extra",
                MessageMode::Remind,
                &[
                    (
                        "Fix the widget latch",
                        "https://github.com/acme/widgets/pull/1",
                    ),
                    (
                        "Add the widget gauge",
                        "https://github.com/acme/widgets/pull/2",
                    ),
                ],
                Some("bob"),
                Some("Batch context"),
            ),
            case(
                "12-unicode-title",
                MessageMode::Request,
                &[(
                    "Réparer le loquet 🐛",
                    "https://github.com/acme/widgets/pull/8",
                )],
                None,
                None,
            ),
            case(
                "13-markup-title",
                MessageMode::Request,
                &[("Fix <a> & <b>", "https://github.com/acme/widgets/pull/9")],
                None,
                None,
            ),
            case(
                "14-whitespace-trim",
                MessageMode::Request,
                &[(
                    "  Fix the widget latch  ",
                    "https://github.com/acme/widgets/pull/4  ",
                )],
                None,
                None,
            ),
            case(
                "15-punctuation-extra",
                MessageMode::Request,
                &[(
                    "Fix [cache](key) & retries",
                    "https://github.com/acme/widgets/pull/5",
                )],
                None,
                Some("  Additional context  "),
            ),
            case(
                "16-batch-mixed",
                MessageMode::Request,
                &[
                    (
                        "Réparer le loquet 🐛",
                        "https://github.com/acme/widgets/pull/8",
                    ),
                    ("Fix <a> & <b>", "https://github.com/acme/widgets/pull/9"),
                    (
                        "Fix [cache](key) & retries",
                        "https://github.com/acme/widgets/pull/5",
                    ),
                ],
                Some("alice"),
                Some("Please take the batch."),
            ),
            case(
                "17-whitespace-requester",
                MessageMode::Request,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/1",
                )],
                Some(" "),
                None,
            ),
            case(
                "18-whitespace-extra",
                MessageMode::Request,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/1",
                )],
                None,
                Some("   "),
            ),
            case(
                "19-single-remind-requester-extra",
                MessageMode::Remind,
                &[(
                    "Fix the widget latch",
                    "https://github.com/acme/widgets/pull/6",
                )],
                Some("alice"),
                Some("Ping"),
            ),
        ];

        for fixture in fixtures {
            let path = format!(
                "{}/tests/fixtures/messages/{}.txt",
                env!("CARGO_MANIFEST_DIR"),
                fixture.name
            );
            let expected = std::fs::read(&path).unwrap_or_else(|err| {
                panic!("read {}: {err}", fixture.name);
            });
            let actual = build(
                fixture.mode,
                fixture.items,
                fixture.requester,
                fixture.extra,
            )
            .unwrap_or_else(|err| panic!("{}: {err}", fixture.name));
            assert_eq!(actual.as_bytes(), expected.as_slice(), "{}", fixture.name);
        }
    }

    #[test]
    fn remind_drops_the_requester() {
        let items = [(
            "Fix the widget latch",
            "https://github.com/acme/widgets/pull/1",
        )];
        let without = build(MessageMode::Remind, &items, None, None).unwrap();
        let with = build(MessageMode::Remind, &items, Some("alice"), None).unwrap();
        assert_eq!(with, without);
        assert!(!with.contains("alice"));
    }

    #[test]
    fn rejects_empty_title_whitespace_title_bad_url_and_no_items() {
        let err = build(MessageMode::Request, &[], None, None).unwrap_err();
        assert!(err.to_string().contains("at least one"));
        let err = build(
            MessageMode::Request,
            &[("   ", "https://github.com/acme/widgets/pull/1")],
            None,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("non-empty"));
        let err = build(
            MessageMode::Request,
            &[(
                "Fix the widget latch",
                "ftp://github.com/acme/widgets/pull/1",
            )],
            None,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("http"));
    }

    #[test]
    fn leading_whitespace_on_a_url_is_trimmed_before_the_http_check() {
        let padded = build(
            MessageMode::Request,
            &[(
                "Fix the widget latch",
                "  https://github.com/acme/widgets/pull/4  ",
            )],
            None,
            None,
        )
        .unwrap();
        let expected = std::fs::read_to_string(format!(
            "{}/tests/fixtures/messages/14-whitespace-trim.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert_eq!(padded, expected);
    }

    #[test]
    fn bare_url_is_not_wrapped() {
        let text = build(
            MessageMode::Request,
            &[(
                "Fix [cache](key) & retries",
                "https://github.com/acme/widgets/pull/5",
            )],
            None,
            Some("  Additional context  "),
        )
        .unwrap();
        assert!(text.contains("Fix [cache](key) & retries"));
        assert_eq!(
            text.lines()
                .filter(|line| *line == "https://github.com/acme/widgets/pull/5")
                .count(),
            1
        );
        assert!(!text.contains("<https://"));
        assert!(!text.contains("](https://"));
        assert!(text.ends_with("\n\nAdditional context\n"));
        assert!(!text.contains("&amp;") && !text.contains("&lt;"));
    }

    #[test]
    fn md_escape_titles_touches_titles_only() {
        let raw = build(
            MessageMode::Request,
            &[(
                "fix *_init_* [WIP] <tag> `x` ~y~ \\path",
                "https://github.com/acme/widgets/pull/1",
            )],
            None,
            Some("keep *bold* and <tag> here"),
        )
        .unwrap();
        let wire = md_escape_titles(&raw);
        assert!(wire.contains(
            "Could someone please review fix \\*\\_init\\_\\* \\[WIP\\] \\<tag\\> \\`x\\` \\~y\\~ \\\\path?"
        ));
        assert!(wire.contains("https://github.com/acme/widgets/pull/1"));
        assert!(wire.contains("Hello team\n"));
        assert!(wire.contains("Thanks!\n\nkeep *bold* and <tag> here\n"));
        assert!(!wire.contains("&amp;") && !wire.contains("&lt;"));
    }

    #[test]
    fn md_escape_titles_escapes_leading_markers_once() {
        let titles = [
            "# heading",
            "> quote",
            "+ item",
            "- item",
            "1. first",
            "12. later",
        ];
        for title in titles {
            let raw = build(
                MessageMode::Request,
                &[(title, "https://github.com/acme/widgets/pull/1")],
                None,
                None,
            )
            .unwrap();
            let wire = md_escape_titles(&raw);
            let escaped = escape_title(title);
            assert!(
                wire.contains(&format!("Could someone please review {escaped}?")),
                "{title} -> {wire}"
            );
            assert!(!wire.contains("\\\\#") && !wire.contains("\\\\>"));
        }
        assert_eq!(escape_title(">"), "\\>");
        assert_eq!(escape_title("#"), "\\#");
        assert_eq!(escape_title("1. first"), "1\\. first");
        assert_eq!(
            escape_title("fix *_init_* [WIP]"),
            "fix \\*\\_init\\_\\* \\[WIP\\]"
        );
    }

    #[test]
    fn md_escape_titles_covers_reminders_and_batch_bullets() {
        let raw = build(
            MessageMode::Remind,
            &[
                ("*one*", "https://github.com/acme/widgets/pull/1"),
                ("# two", "https://github.com/acme/widgets/pull/2"),
            ],
            Some("alice"),
            Some("*not a title*"),
        )
        .unwrap();
        let wire = md_escape_titles(&raw);
        assert!(wire.starts_with(
            "Hello, I'm still looking for a review on these pull requests when you have a moment :pray:\n"
        ));
        assert!(wire.contains("- \\*one\\*\nhttps://github.com/acme/widgets/pull/1\n"));
        assert!(wire.contains("- \\# two\nhttps://github.com/acme/widgets/pull/2\n"));
        assert!(wire.contains("Thanks!\n\n*not a title*\n"));
        assert!(!wire.contains("alice"));
    }

    #[test]
    fn mrkdwn_escape_is_separate_from_the_mcp_transform() {
        assert_eq!(mrkdwn_escape("a & <b>"), "a &amp; &lt;b&gt;");
        let raw = build(
            MessageMode::Request,
            &[("Fix <a> & <b>", "https://github.com/acme/widgets/pull/9")],
            None,
            None,
        )
        .unwrap();
        assert_eq!(md_escape_titles(&raw).matches("&amp;").count(), 0);
        assert!(mrkdwn_escape(&raw).contains("Fix &lt;a&gt; &amp; &lt;b&gt;"));
    }
}
