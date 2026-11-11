//! Sidebar view queries.
//!
//! My PRs, To review, and Bumps due are these rules, evaluated on synced rows
//! so the lists stay what they were before the rules were written down.
//! `bump:` never goes to GitHub. Qualifiers this module can decide stay local
//! too. Anything else is a GitHub search, with `bump:` removed.

use serde::{Deserialize, Serialize};

use crate::domain::SettingEntry;
use crate::error::CoreError;
use crate::github_query::{self, Scanned};
use crate::store::Store;

pub const AUTHOR: u8 = 1;
pub const REVIEW_REQUESTED: u8 = 2;

pub const MINE_QUERY: &str = "is:open author:@me";
pub const REVIEW_QUERY: &str = "is:open review-requested:@me";
pub const BUMP_QUERY: &str = "is:open author:@me bump:due";

const CATALOG_KEY: &str = "sidebar.views";
const NAME_MAX: usize = 80;
const QUERY_MAX: usize = 1024;

/// How a view query splits, and which rows it keeps when it is local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analyzed {
    /// Blank or whitespace. Membership stays with the view key.
    pub empty: bool,
    /// Every qualifier is decided from synced rows. Nothing is sent to GitHub.
    pub fully_local: bool,
    /// Qualifiers to send when `fully_local` is false. `bump:` is omitted.
    pub github: String,
    /// Words that are not qualifiers.
    pub text: String,
    /// The rule asks for merged or closed rows, so the open-list scope lets them through.
    pub shows_finished: bool,
    /// The query could not be read (an unfinished quote). It matches nothing.
    pub invalid: bool,
}

/// One saved sidebar view. Built-in keys are `mine`, `to_review`, and `bumps_due`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarView {
    pub key: String,
    pub name: String,
    pub query: String,
    pub builtin: bool,
    pub sort_order: i32,
}

pub fn analyze(raw: &str) -> Analyzed {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Analyzed {
            empty: true,
            fully_local: true,
            github: String::new(),
            text: String::new(),
            shows_finished: false,
            invalid: false,
        };
    }
    let tokens = match github_query::scan(trimmed) {
        Ok(tokens) => tokens,
        Err(_) => {
            return Analyzed {
                empty: false,
                fully_local: false,
                github: String::new(),
                text: String::new(),
                shows_finished: false,
                invalid: true,
            };
        }
    };
    let mut github = Vec::new();
    let mut text = Vec::new();
    let mut fully_local = true;
    let mut shows_finished = false;
    for token in &tokens {
        let Some(key) = token.key.as_deref() else {
            text.push(token.value.as_str());
            continue;
        };
        if key.eq_ignore_ascii_case("bump") {
            shows_finished |= finished_token(token);
            continue;
        }
        shows_finished |= finished_token(token);
        if !local_pair(key, &token.value) {
            fully_local = false;
        }
        github.push(render_qualifier(token));
    }
    Analyzed {
        empty: false,
        fully_local,
        github: if fully_local {
            String::new()
        } else {
            github.join(" ")
        },
        text: text.join(" "),
        shows_finished,
        invalid: false,
    }
}

/// True when a fully local rule keeps this row. Non-local qualifiers are not
/// expected here; an unknown one does not match.
pub fn matches_local(involvement: u8, state: &str, is_draft: bool, due: bool, raw: &str) -> bool {
    let Ok(tokens) = github_query::scan(raw) else {
        return false;
    };
    tokens
        .iter()
        .all(|token| match_token(token, involvement, state, is_draft, due, true))
}

/// After a GitHub search, only `bump:` still narrows the rows. Other
/// qualifiers were already sent.
pub fn matches_bump_only(due: bool, raw: &str) -> bool {
    let Ok(tokens) = github_query::scan(raw) else {
        return false;
    };
    tokens.iter().all(|token| {
        if token.key.as_deref() != Some("bump") {
            return true;
        }
        match_token(token, 0, "open", false, due, false)
    })
}

/// The bump queue screen: the default rule, with no extra words or negation.
pub fn is_bump_queue(raw: &str) -> bool {
    let analyzed = analyze(raw);
    if !analyzed.fully_local || analyzed.invalid || !analyzed.text.is_empty() {
        return false;
    }
    let Ok(tokens) = github_query::scan(raw) else {
        return false;
    };
    let mut seen = Vec::new();
    for token in tokens {
        let Some(key) = token.key.as_deref() else {
            return false;
        };
        if token.negated {
            return false;
        }
        seen.push(format!("{key}:{}", token.value.to_ascii_lowercase()));
    }
    seen.sort();
    seen == ["author:@me", "bump:due", "is:open"]
}

pub fn defaults() -> Vec<SidebarView> {
    vec![
        builtin("mine", "My PRs", MINE_QUERY, 0),
        builtin("to_review", "To review", REVIEW_QUERY, 1),
        builtin("bumps_due", "Bumps due", BUMP_QUERY, 2),
    ]
}

/// Saved views, with the three built-ins always present. A missing or broken
/// catalog is the defaults. Custom views and edited built-in names and queries
/// are kept. A clashing name is made unique so an old blob still loads.
pub fn load_views(store: &Store) -> Result<Vec<SidebarView>, CoreError> {
    let Some(entry) = store.get_setting(CATALOG_KEY.to_owned())? else {
        return Ok(defaults());
    };
    let parsed: Vec<SidebarView> = serde_json::from_str(&entry.value_json).unwrap_or_default();
    Ok(repair(parsed))
}

/// Replaces the catalog. Built-in views that were omitted come back at their
/// default name and query. Returns the list that was stored.
pub fn save_views(store: &Store, views: Vec<SidebarView>) -> Result<Vec<SidebarView>, CoreError> {
    let stored = validate(views)?;
    let value_json = serde_json::to_string(&stored)
        .map_err(|err| CoreError::Store(format!("encode {CATALOG_KEY}: {err}")))?;
    store.put_setting(SettingEntry {
        key: CATALOG_KEY.to_owned(),
        value_json,
    })?;
    Ok(stored)
}

fn builtin(key: &str, name: &str, query: &str, sort_order: i32) -> SidebarView {
    SidebarView {
        key: key.to_owned(),
        name: name.to_owned(),
        query: query.to_owned(),
        builtin: true,
        sort_order,
    }
}

fn local_pair(key: &str, value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    matches!(
        (key, value.as_str()),
        ("author", "@me")
            | ("review-requested", "@me")
            | ("is", "open" | "closed" | "merged" | "draft" | "pr")
            | ("state", "open" | "closed")
            | ("draft", "true" | "false")
            | ("bump", "due")
    )
}

fn finished_token(token: &Scanned) -> bool {
    let Some(key) = token.key.as_deref() else {
        return false;
    };
    let value = token.value.to_ascii_lowercase();
    if token.negated {
        return matches!((key, value.as_str()), ("is", "open") | ("state", "open"));
    }
    matches!(
        (key, value.as_str()),
        ("is", "closed" | "merged") | ("state", "closed")
    )
}

fn match_token(
    token: &Scanned,
    involvement: u8,
    state: &str,
    is_draft: bool,
    due: bool,
    strict: bool,
) -> bool {
    let Some(key) = token.key.as_deref() else {
        return true;
    };
    let value = token.value.to_ascii_lowercase();
    let open = state.eq_ignore_ascii_case("open");
    let positive = match (key, value.as_str()) {
        ("author", "@me") => involvement & AUTHOR != 0,
        ("review-requested", "@me") => involvement & REVIEW_REQUESTED != 0,
        ("is", "open") | ("state", "open") | ("is", "pr") => true,
        ("is", "closed") => state.eq_ignore_ascii_case("closed"),
        ("is", "merged") => state.eq_ignore_ascii_case("merged"),
        ("state", "closed") => !open,
        ("is", "draft") | ("draft", "true") => is_draft,
        ("draft", "false") => !is_draft,
        ("bump", "due") => due,
        ("bump", _) => false,
        _ => !strict,
    };
    if token.negated && matches!((key, value.as_str()), ("is", "open") | ("state", "open")) {
        !open
    } else if token.negated {
        !positive
    } else {
        positive
    }
}

fn render_qualifier(token: &Scanned) -> String {
    let key = token.key.clone().unwrap_or_default();
    let value = if token.value.chars().any(char::is_whitespace) {
        format!("\"{}\"", token.value.replace('"', "\\\""))
    } else {
        token.value.clone()
    };
    let body = format!("{key}:{value}");
    if token.negated {
        format!("-{body}")
    } else {
        body
    }
}

fn repair(input: Vec<SidebarView>) -> Vec<SidebarView> {
    let mut customs = Vec::new();
    let mut seen_keys = std::collections::HashSet::new();
    for view in input.iter().filter(|view| valid_custom_key(&view.key)) {
        if !seen_keys.insert(view.key.clone()) {
            continue;
        }
        customs.push(view.clone());
    }
    customs.sort_by(|left, right| {
        left.sort_order
            .cmp(&right.sort_order)
            .then(left.key.cmp(&right.key))
    });
    let mut out = Vec::new();
    for (index, default) in defaults().into_iter().enumerate() {
        let saved = input.iter().find(|view| view.key == default.key);
        let mut name = saved
            .map(|view| clean_name(&view.name))
            .filter(|name| !name.is_empty())
            .unwrap_or(default.name);
        if name_taken(&out, &name) {
            name = default_name_for(&default.key).to_owned();
        }
        let query = saved
            .map(|view| cap_query(&view.query))
            .filter(|query| github_query::scan(query).is_ok())
            .unwrap_or(default.query);
        out.push(SidebarView {
            key: default.key,
            name,
            query,
            builtin: true,
            sort_order: i32::try_from(index).unwrap_or(0),
        });
    }
    for (offset, view) in customs.into_iter().enumerate() {
        let mut name = clean_name(&view.name);
        if name.is_empty() {
            name = format!("View {}", offset + 1);
        }
        name = unique_name(&out, name);
        let query = cap_query(&view.query);
        let query = if github_query::scan(&query).is_ok() {
            query
        } else {
            String::new()
        };
        out.push(SidebarView {
            key: view.key,
            name,
            query,
            builtin: false,
            sort_order: i32::try_from(out.len()).unwrap_or(0),
        });
    }
    out
}

fn validate(input: Vec<SidebarView>) -> Result<Vec<SidebarView>, CoreError> {
    let mut names = std::collections::HashSet::new();
    let mut customs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for view in &input {
        if defaults().iter().any(|builtin| builtin.key == view.key) {
            continue;
        }
        if !valid_custom_key(&view.key) {
            return Err(CoreError::Validation(format!("unknown view {}", view.key)));
        }
        if !seen.insert(view.key.clone()) {
            return Err(CoreError::Validation(format!(
                "duplicate view {}",
                view.key
            )));
        }
        customs.push(view.clone());
    }
    customs.sort_by(|left, right| {
        left.sort_order
            .cmp(&right.sort_order)
            .then(left.key.cmp(&right.key))
    });
    let mut out = Vec::new();
    for (index, default) in defaults().into_iter().enumerate() {
        let saved = input.iter().find(|view| view.key == default.key);
        let name = match saved {
            Some(view) => require_name(&view.name)?,
            None => default.name,
        };
        let folded = name.to_lowercase();
        if !names.insert(folded) {
            return Err(CoreError::Validation(
                "A view already has that name.".to_owned(),
            ));
        }
        let query = match saved {
            Some(view) => require_query(&view.query)?,
            None => default.query,
        };
        out.push(SidebarView {
            key: default.key,
            name,
            query,
            builtin: true,
            sort_order: i32::try_from(index).unwrap_or(0),
        });
    }
    for view in customs {
        let name = require_name(&view.name)?;
        let folded = name.to_lowercase();
        if !names.insert(folded) {
            return Err(CoreError::Validation(
                "A view already has that name.".to_owned(),
            ));
        }
        out.push(SidebarView {
            key: view.key,
            name,
            query: require_query(&view.query)?,
            builtin: false,
            sort_order: i32::try_from(out.len()).unwrap_or(0),
        });
    }
    Ok(out)
}

fn require_name(raw: &str) -> Result<String, CoreError> {
    let name = clean_name(raw);
    if name.is_empty() {
        return Err(CoreError::Validation("Enter a name.".to_owned()));
    }
    if name.chars().count() > NAME_MAX {
        return Err(CoreError::Validation("That name is too long.".to_owned()));
    }
    Ok(name)
}

fn require_query(raw: &str) -> Result<String, CoreError> {
    let query = cap_query(raw);
    if query.chars().count() > QUERY_MAX {
        return Err(CoreError::Validation("That query is too long.".to_owned()));
    }
    if github_query::scan(&query).is_err() {
        return Err(CoreError::Validation(
            "The query has an unfinished quote.".to_owned(),
        ));
    }
    Ok(query)
}

fn clean_name(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cap_query(raw: &str) -> String {
    raw.trim().chars().take(QUERY_MAX).collect()
}

fn name_taken(views: &[SidebarView], name: &str) -> bool {
    let folded = name.to_lowercase();
    views.iter().any(|view| view.name.to_lowercase() == folded)
}

fn unique_name(views: &[SidebarView], name: String) -> String {
    if !name_taken(views, &name) {
        return name;
    }
    for index in 2..1000 {
        let next = format!("{name} {index}");
        if !name_taken(views, &next) {
            return next;
        }
    }
    name
}

fn default_name_for(key: &str) -> &'static str {
    match key {
        "mine" => "My PRs",
        "to_review" => "To review",
        "bumps_due" => "Bumps due",
        _ => "View",
    }
}

fn valid_custom_key(key: &str) -> bool {
    let Some(rest) = key.strip_prefix("custom:") else {
        return false;
    };
    let mut chars = rest.chars();
    match chars.next() {
        Some(first) if first.is_ascii_digit() && first != '0' => {}
        _ => return false,
    }
    rest.len() <= 8 && chars.all(|ch| ch.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rules_match_the_old_membership() {
        let author = AUTHOR;
        let review = REVIEW_REQUESTED;
        assert!(matches_local(author, "open", false, false, MINE_QUERY));
        assert!(matches_local(author, "closed", false, false, MINE_QUERY));
        assert!(!matches_local(review, "open", false, false, MINE_QUERY));
        assert!(matches_local(
            author | review,
            "open",
            true,
            false,
            MINE_QUERY
        ));

        assert!(matches_local(review, "open", false, false, REVIEW_QUERY));
        assert!(matches_local(review, "merged", false, false, REVIEW_QUERY));
        assert!(!matches_local(author, "open", false, false, REVIEW_QUERY));

        assert!(matches_local(author, "open", false, true, BUMP_QUERY));
        assert!(!matches_local(author, "open", false, false, BUMP_QUERY));
        assert!(!matches_local(review, "open", false, true, BUMP_QUERY));
        assert!(is_bump_queue(BUMP_QUERY));
        assert!(is_bump_queue("  is:open   author:@me  bump:due "));
        assert!(!is_bump_queue("is:open author:@me bump:due label:bug"));
    }

    #[test]
    fn other_qualifiers_go_to_github_without_bump() {
        let rule = analyze(r#"is:open author:@me label:"needs review" bump:due login"#);
        assert!(!rule.fully_local);
        assert_eq!(rule.github, r#"is:open author:@me label:"needs review""#);
        assert_eq!(rule.text, "login");
        assert!(matches_bump_only(true, r#"label:bug bump:due"#));
        assert!(!matches_bump_only(false, r#"label:bug bump:due"#));
        assert!(analyze("is:closed author:@me").shows_finished);
        assert!(!analyze(MINE_QUERY).shows_finished);
        assert!(analyze("-is:open author:@me").shows_finished);
        assert!(!matches_local(
            author_bit(),
            "open",
            false,
            false,
            "-is:open author:@me"
        ));
        assert!(matches_local(
            author_bit(),
            "closed",
            false,
            false,
            "-is:open author:@me"
        ));
    }

    fn author_bit() -> u8 {
        AUTHOR
    }

    #[test]
    fn a_saved_catalog_keeps_edits_and_custom_views_and_restores_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("pr-nexus.db")).unwrap();
        assert_eq!(load_views(&store).unwrap(), defaults());

        let mut views = defaults();
        views[0].name = "Mine".to_owned();
        views.push(SidebarView {
            key: "custom:1".to_owned(),
            name: "Needs me".to_owned(),
            query: REVIEW_QUERY.to_owned(),
            builtin: false,
            sort_order: 9,
        });
        let saved = save_views(&store, views.clone()).unwrap();
        assert_eq!(saved[0].name, "Mine");
        assert_eq!(saved[3].key, "custom:1");

        let mut dropped = saved.clone();
        dropped.retain(|view| view.key != "to_review");
        let restored = save_views(&store, dropped).unwrap();
        assert_eq!(restored[1].name, "To review");
        assert_eq!(restored[1].query, REVIEW_QUERY);

        let again = load_views(&store).unwrap();
        assert_eq!(again[0].name, "Mine");
        assert!(again.iter().any(|view| view.key == "custom:1"));
    }

    #[test]
    fn a_broken_catalog_falls_back_without_dropping_a_readable_custom_view() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("pr-nexus.db")).unwrap();
        store
            .put_setting(SettingEntry {
                key: CATALOG_KEY.to_owned(),
                value_json: "not-json".to_owned(),
            })
            .unwrap();
        assert_eq!(load_views(&store).unwrap(), defaults());

        store
            .put_setting(SettingEntry {
                key: CATALOG_KEY.to_owned(),
                value_json: r#"[{"key":"custom:2","name":"Drafts","query":"is:draft","builtin":false,"sort_order":4}]"#
                    .to_owned(),
            })
            .unwrap();
        let loaded = load_views(&store).unwrap();
        assert_eq!(loaded.len(), 4);
        assert_eq!(loaded[3].name, "Drafts");
        assert_eq!(loaded[0].query, MINE_QUERY);
    }

    #[test]
    fn save_rejects_a_duplicate_name_and_an_unfinished_quote() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("pr-nexus.db")).unwrap();
        let mut views = defaults();
        views.push(SidebarView {
            key: "custom:1".to_owned(),
            name: "My PRs".to_owned(),
            query: "is:open".to_owned(),
            builtin: false,
            sort_order: 3,
        });
        assert!(
            save_views(&store, views)
                .unwrap_err()
                .to_string()
                .contains("name")
        );

        let mut views = defaults();
        views.push(SidebarView {
            key: "custom:1".to_owned(),
            name: "Needs me".to_owned(),
            query: "label:\"bug".to_owned(),
            builtin: false,
            sort_order: 3,
        });
        assert!(
            save_views(&store, views)
                .unwrap_err()
                .to_string()
                .contains("quote")
        );
    }
}
