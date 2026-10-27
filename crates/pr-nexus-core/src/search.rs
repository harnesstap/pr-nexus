//! Search text for the pull request list.
//!
//! Matching is a plain substring test on folded text: compatibility forms and
//! accents removed, lower case, and every dash variant turned into `-`. There
//! is no query language: punctuation such as `-`, `/`, `.`, `:` or `#` is just
//! text, so `service-ssp`, `acme/service-ssp`, `v1.2` and `#123` match as
//! typed. Each pull request stores its folded fields in
//! `pull_requests.search_text`, so a search is one `instr` per term.
//!
//! `app/src/list/model.ts` (`normalizeSearch`) must fold the same way. Changing
//! the fold needs a migration that clears `search_text` so it is rebuilt.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Lower case with accents removed and dashes unified: `Élan` → `elan`,
/// `service‑ssp` (non-breaking hyphen) → `service-ssp`, `ＡＢＣ` → `abc`.
pub fn fold(text: &str) -> String {
    text.nfkd()
        .filter(|ch| !is_combining_mark(*ch))
        .map(|ch| if is_dash(ch) { '-' } else { ch })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Hyphen, dash and minus code points a pasted name may carry instead of `-`.
fn is_dash(ch: char) -> bool {
    matches!(
        ch,
        '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{FE58}' | '\u{FE63}' | '\u{FF0D}'
    )
}

/// Folded search terms, split on whitespace. Empty when the query is blank.
pub fn terms(query: &str) -> Vec<String> {
    fold(query).split_whitespace().map(str::to_owned).collect()
}

/// The fields a search looks at, folded and joined by newlines.
///
/// Fields: title, description, `owner/name`, `#number`, author, and branch.
/// Project names are matched separately because links change without a sync.
pub struct SearchFields<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub owner: &'a str,
    pub name: &'a str,
    pub number: u64,
    pub author: &'a str,
    pub head_ref: &'a str,
}

pub fn search_text(fields: &SearchFields<'_>) -> String {
    let joined = format!(
        "{}\n{}/{}\n#{}\n{}\n{}\n{}",
        fields.title,
        fields.owner,
        fields.name,
        fields.number,
        fields.author,
        fields.head_ref,
        fields.body,
    );
    fold(&joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_drops_accents_and_case() {
        assert_eq!(fold("Élan Café"), "elan cafe");
        assert_eq!(fold("ÅNGSTRÖM"), "angstrom");
    }

    #[test]
    fn fold_unifies_dashes_and_compatibility_forms() {
        assert_eq!(fold("Service\u{2011}SSP"), "service-ssp");
        assert_eq!(fold("a\u{2013}b\u{2212}c"), "a-b-c");
        assert_eq!(fold("\u{FF03}12"), "#12");
    }

    #[test]
    fn terms_split_on_whitespace() {
        assert_eq!(terms("  Fix   Café "), vec!["fix", "cafe"]);
        assert!(terms("   ").is_empty());
    }

    #[test]
    fn search_text_holds_every_field() {
        let text = search_text(&SearchFields {
            title: "Add Login",
            body: "Résumé of the change",
            owner: "acme",
            name: "web",
            number: 42,
            author: "Octo",
            head_ref: "feat/login",
        });
        for needle in [
            "add login",
            "resume",
            "acme/web",
            "#42",
            "octo",
            "feat/login",
        ] {
            assert!(text.contains(needle), "{needle} missing from {text}");
        }
    }
}
