//! User-editable review-request templates (§7.6).
//!
//! The defaults are the script's texts, so with no customization
//! [`super::message::build`] output is byte-identical to the golden fixtures.
//! Placeholders are `{title}`, `{url}`, `{who}`, `{requester}`, `{count}` and,
//! in batch templates, `{items}`. `{{` and `}}` are literal braces.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

pub const DEFAULT_SINGLE_REQUEST: &str =
    "Hello team\nCould someone please review {title}?{who}\n{url}\nThanks!";
pub const DEFAULT_BATCH_REQUEST: &str =
    "Hello team\nCould someone please review these pull requests?{who}\n{items}\nThanks!";
pub const DEFAULT_SINGLE_REMIND: &str = "Hello, I'm still looking for a review on {title} when you have a moment :pray:\n{url}\nThanks!";
pub const DEFAULT_BATCH_REMIND: &str = "Hello, I'm still looking for a review on these pull requests when you have a moment :pray:\n{items}\nThanks!";
pub const DEFAULT_ITEM: &str = "- {title}\n{url}";

/// One editable template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    SingleRequest,
    BatchRequest,
    SingleRemind,
    BatchRemind,
    /// One line group of `{items}` in the batch templates.
    Item,
}

impl TemplateKind {
    pub const ALL: [TemplateKind; 5] = [
        TemplateKind::SingleRequest,
        TemplateKind::BatchRequest,
        TemplateKind::SingleRemind,
        TemplateKind::BatchRemind,
        TemplateKind::Item,
    ];

    /// The `settings` key; an absent key means the default.
    pub fn setting_key(self) -> &'static str {
        match self {
            TemplateKind::SingleRequest => "review.templates.single_request",
            TemplateKind::BatchRequest => "review.templates.batch_request",
            TemplateKind::SingleRemind => "review.templates.single_remind",
            TemplateKind::BatchRemind => "review.templates.batch_remind",
            TemplateKind::Item => "review.templates.item",
        }
    }

    pub fn default_text(self) -> &'static str {
        match self {
            TemplateKind::SingleRequest => DEFAULT_SINGLE_REQUEST,
            TemplateKind::BatchRequest => DEFAULT_BATCH_REQUEST,
            TemplateKind::SingleRemind => DEFAULT_SINGLE_REMIND,
            TemplateKind::BatchRemind => DEFAULT_BATCH_REMIND,
            TemplateKind::Item => DEFAULT_ITEM,
        }
    }

    fn allowed(self) -> &'static [Placeholder] {
        use Placeholder::*;
        match self {
            TemplateKind::SingleRequest | TemplateKind::SingleRemind | TemplateKind::Item => {
                &[Title, Url, Who, Requester, Count]
            }
            TemplateKind::BatchRequest | TemplateKind::BatchRemind => {
                &[Items, Who, Requester, Count]
            }
        }
    }

    fn required(self) -> Placeholder {
        match self {
            TemplateKind::SingleRequest | TemplateKind::SingleRemind | TemplateKind::Item => {
                Placeholder::Url
            }
            TemplateKind::BatchRequest | TemplateKind::BatchRemind => Placeholder::Items,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placeholder {
    Title,
    Url,
    Who,
    Requester,
    Count,
    Items,
}

impl Placeholder {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "title" => Placeholder::Title,
            "url" => Placeholder::Url,
            "who" => Placeholder::Who,
            "requester" => Placeholder::Requester,
            "count" => Placeholder::Count,
            "items" => Placeholder::Items,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Placeholder::Title => "title",
            Placeholder::Url => "url",
            Placeholder::Who => "who",
            Placeholder::Requester => "requester",
            Placeholder::Count => "count",
            Placeholder::Items => "items",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Text(String),
    Slot(Placeholder),
}

/// Split a template into text and placeholders, or explain why it is invalid.
fn parse(kind: TemplateKind, template: &str) -> Result<Vec<Token>, CoreError> {
    let invalid = |msg: String| CoreError::Validation(msg);
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    name.push(next);
                }
                if !closed {
                    return Err(invalid(
                        "unclosed `{`; write `{{` for a literal brace".into(),
                    ));
                }
                let slot = Placeholder::parse(name.trim())
                    .ok_or_else(|| invalid(format!("unknown placeholder `{{{name}}}`")))?;
                if !kind.allowed().contains(&slot) {
                    return Err(invalid(format!(
                        "`{{{}}}` is not available in this template",
                        slot.name()
                    )));
                }
                if !text.is_empty() {
                    tokens.push(Token::Text(std::mem::take(&mut text)));
                }
                tokens.push(Token::Slot(slot));
            }
            '}' => {
                return Err(invalid("stray `}`; write `}}` for a literal brace".into()));
            }
            other => text.push(other),
        }
    }
    if !text.is_empty() {
        tokens.push(Token::Text(text));
    }
    let required = kind.required();
    if !tokens.contains(&Token::Slot(required)) {
        return Err(invalid(format!(
            "the template must contain `{{{}}}`",
            required.name()
        )));
    }
    Ok(tokens)
}

/// Check a template before it is saved.
pub fn validate(kind: TemplateKind, template: &str) -> Result<(), CoreError> {
    parse(kind, template).map(|_| ())
}

/// Values for one rendering. `title` is already a code span.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Values<'a> {
    pub title: &'a str,
    pub url: &'a str,
    pub who: &'a str,
    pub requester: &'a str,
    pub count: usize,
    pub items: &'a str,
}

pub(crate) fn render(
    kind: TemplateKind,
    template: &str,
    values: &Values<'_>,
) -> Result<String, CoreError> {
    let mut out = String::new();
    for token in parse(kind, template)? {
        match token {
            Token::Text(text) => out.push_str(&text),
            Token::Slot(Placeholder::Title) => out.push_str(values.title),
            Token::Slot(Placeholder::Url) => out.push_str(values.url),
            Token::Slot(Placeholder::Who) => out.push_str(values.who),
            Token::Slot(Placeholder::Requester) => out.push_str(values.requester),
            Token::Slot(Placeholder::Count) => out.push_str(&values.count.to_string()),
            Token::Slot(Placeholder::Items) => out.push_str(values.items),
        }
    }
    Ok(out)
}

/// The five templates in effect. `Default` is the script's wording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageTemplates {
    pub single_request: String,
    pub batch_request: String,
    pub single_remind: String,
    pub batch_remind: String,
    pub item: String,
}

impl Default for MessageTemplates {
    fn default() -> Self {
        Self {
            single_request: DEFAULT_SINGLE_REQUEST.into(),
            batch_request: DEFAULT_BATCH_REQUEST.into(),
            single_remind: DEFAULT_SINGLE_REMIND.into(),
            batch_remind: DEFAULT_BATCH_REMIND.into(),
            item: DEFAULT_ITEM.into(),
        }
    }
}

impl MessageTemplates {
    pub fn get(&self, kind: TemplateKind) -> &str {
        match kind {
            TemplateKind::SingleRequest => &self.single_request,
            TemplateKind::BatchRequest => &self.batch_request,
            TemplateKind::SingleRemind => &self.single_remind,
            TemplateKind::BatchRemind => &self.batch_remind,
            TemplateKind::Item => &self.item,
        }
    }

    pub fn set(&mut self, kind: TemplateKind, text: String) {
        *match kind {
            TemplateKind::SingleRequest => &mut self.single_request,
            TemplateKind::BatchRequest => &mut self.batch_request,
            TemplateKind::SingleRemind => &mut self.single_remind,
            TemplateKind::BatchRemind => &mut self.batch_remind,
            TemplateKind::Item => &mut self.item,
        } = text;
    }
}

/// Sample PR for the Settings preview: `_` and a backtick show the code span.
pub const SAMPLE_ITEMS: [(&str, &str); 2] = [
    (
        "fix: handle user_id in `parser`",
        "https://github.com/acme/widgets/pull/42",
    ),
    (
        "feat: add retry_count to the client",
        "https://github.com/acme/widgets/pull/43",
    ),
];

/// Render `kind` with a candidate text and sample PRs, for Settings.
/// `Item` previews through the batch request template.
pub fn preview(
    templates: &MessageTemplates,
    kind: TemplateKind,
    text: &str,
    requester: Option<&str>,
) -> Result<String, CoreError> {
    use super::message::{MessageMode, build_with};
    validate(kind, text)?;
    let mut templates = templates.clone();
    templates.set(kind, text.to_owned());
    let (mode, items): (MessageMode, &[(&str, &str)]) = match kind {
        TemplateKind::SingleRequest => (MessageMode::Request, &SAMPLE_ITEMS[..1]),
        TemplateKind::SingleRemind => (MessageMode::Remind, &SAMPLE_ITEMS[..1]),
        TemplateKind::BatchRequest | TemplateKind::Item => (MessageMode::Request, &SAMPLE_ITEMS),
        TemplateKind::BatchRemind => (MessageMode::Remind, &SAMPLE_ITEMS),
    };
    build_with(&templates, mode, items, requester, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::message::{MessageMode, build, build_with};

    const URL: &str = "https://github.com/acme/widgets/pull/1";

    fn with(kind: TemplateKind, text: &str) -> MessageTemplates {
        let mut templates = MessageTemplates::default();
        templates.set(kind, text.into());
        templates
    }

    #[test]
    fn defaults_validate() {
        for kind in TemplateKind::ALL {
            validate(kind, kind.default_text()).unwrap();
        }
    }

    #[test]
    fn placeholders_render() {
        let templates = with(
            TemplateKind::SingleRequest,
            "Hi! {count} PR from {requester}{who}: {title} {url}",
        );
        let text = build_with(
            &templates,
            MessageMode::Request,
            &[("fix: user_id", URL)],
            Some("alice"),
            None,
        )
        .unwrap();
        assert_eq!(
            text,
            format!("Hi! 1 PR from alice (alice): `fix: user_id` {URL}\n")
        );
    }

    #[test]
    fn requester_off_renders_empty() {
        let templates = with(TemplateKind::SingleRequest, "[{requester}]{who} {url}");
        let text = build_with(&templates, MessageMode::Request, &[("t", URL)], None, None).unwrap();
        assert_eq!(text, format!("[] {URL}\n"));
    }

    #[test]
    fn reminders_can_opt_into_who() {
        let templates = with(TemplateKind::SingleRemind, "Ping{who} {url}");
        let text = build_with(
            &templates,
            MessageMode::Remind,
            &[("t", URL)],
            Some("bob"),
            None,
        )
        .unwrap();
        assert_eq!(text, format!("Ping (bob) {URL}\n"));
        // The default remind still drops the requester.
        let text = build(MessageMode::Remind, &[("t", URL)], Some("bob"), None).unwrap();
        assert!(!text.contains("bob"));
    }

    #[test]
    fn double_braces_are_literal() {
        let templates = with(TemplateKind::SingleRequest, "{{not a slot}} {{{url}}}");
        let text = build_with(&templates, MessageMode::Request, &[("t", URL)], None, None).unwrap();
        assert_eq!(text, format!("{{not a slot}} {{{URL}}}\n"));
    }

    #[test]
    fn unknown_placeholder_is_refused() {
        let err = validate(TemplateKind::SingleRequest, "{url} {branch}").unwrap_err();
        assert!(
            err.to_string().contains("unknown placeholder `{branch}`"),
            "{err}"
        );
        let err = validate(TemplateKind::SingleRequest, "{url} {oops").unwrap_err();
        assert!(err.to_string().contains("unclosed"), "{err}");
        let err = validate(TemplateKind::SingleRequest, "{url} }").unwrap_err();
        assert!(err.to_string().contains("stray"), "{err}");
        let err = validate(TemplateKind::BatchRequest, "{items} {url}").unwrap_err();
        assert!(err.to_string().contains("not available"), "{err}");
        let err = validate(TemplateKind::SingleRequest, "{url} {items}").unwrap_err();
        assert!(err.to_string().contains("not available"), "{err}");
    }

    #[test]
    fn required_placeholders() {
        let err = validate(TemplateKind::SingleRequest, "Hello {title}").unwrap_err();
        assert!(err.to_string().contains("must contain `{url}`"), "{err}");
        let err = validate(TemplateKind::SingleRemind, "Ping").unwrap_err();
        assert!(err.to_string().contains("`{url}`"));
        let err = validate(TemplateKind::BatchRequest, "Hello {count}").unwrap_err();
        assert!(err.to_string().contains("must contain `{items}`"), "{err}");
        let err = validate(TemplateKind::BatchRemind, "").unwrap_err();
        assert!(err.to_string().contains("`{items}`"));
        // An escaped `{{url}}` is not the placeholder.
        assert!(validate(TemplateKind::SingleRequest, "{{url}}").is_err());
    }

    #[test]
    fn batch_items_use_the_item_template() {
        let mut templates = with(TemplateKind::BatchRequest, "{count} PRs{who}:\n{items}");
        templates.set(TemplateKind::Item, "* {title} -> {url}".into());
        let text = build_with(
            &templates,
            MessageMode::Request,
            &[
                ("one_two", "https://github.com/acme/widgets/pull/1"),
                ("use `x`", "https://github.com/acme/widgets/pull/2"),
            ],
            Some("alice"),
            Some(" note "),
        )
        .unwrap();
        assert_eq!(
            text,
            "2 PRs (alice):\n* `one_two` -> https://github.com/acme/widgets/pull/1\n* `use 'x'` -> https://github.com/acme/widgets/pull/2\n\nnote\n"
        );
    }

    #[test]
    fn post_processing_still_applies() {
        let templates = with(TemplateKind::SingleRequest, "{url}   \n\n  \n");
        let text = build_with(&templates, MessageMode::Request, &[("t", URL)], None, None).unwrap();
        assert_eq!(text, format!("{URL}\n"));
    }

    #[test]
    fn saved_invalid_template_errors_at_build() {
        let templates = with(TemplateKind::SingleRequest, "{nope}");
        assert!(build_with(&templates, MessageMode::Request, &[("t", URL)], None, None).is_err());
    }

    #[test]
    fn preview_uses_the_sample_pr() {
        let text = preview(
            &MessageTemplates::default(),
            TemplateKind::SingleRequest,
            DEFAULT_SINGLE_REQUEST,
            None,
        )
        .unwrap();
        assert!(text.contains("`fix: handle user_id in 'parser'`"));
        let text = preview(
            &MessageTemplates::default(),
            TemplateKind::Item,
            "> {title}\n{url}",
            None,
        )
        .unwrap();
        assert!(text.contains("> `feat: add retry_count to the client`\n"));
        assert!(
            preview(
                &MessageTemplates::default(),
                TemplateKind::Item,
                "{title}",
                None
            )
            .is_err()
        );
    }
}
