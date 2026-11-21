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
const COLLAPSE_KEY: &str = "sidebar.collapse";
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
/// `parent` is `mine` or `to_review` for a custom view nested under that default.
/// Older catalogs omit it; those views stay at the top level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarView {
    pub key: String,
    pub name: String,
    pub query: String,
    pub builtin: bool,
    pub sort_order: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// Which sidebar sections and parent views are collapsed. Missing fields stay open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SidebarCollapse {
    #[serde(default)]
    pub views: bool,
    #[serde(default)]
    pub projects: bool,
    /// `mine` and `to_review` whose children are hidden.
    #[serde(default)]
    pub parents: Vec<String>,
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

/// Facts a fully local rule can see. Teams are `org/slug`. Assignees are logins.
pub struct LocalFacts<'a> {
    pub involvement: u8,
    pub state: &'a str,
    pub is_draft: bool,
    pub due: bool,
    pub teams: &'a [String],
    pub assignees: &'a [String],
    pub viewer: &'a str,
}

/// True when a fully local rule keeps this row. Non-local qualifiers are not
/// expected here; an unknown one does not match. Team and assignee facts are
/// empty, so a rule that names them does not match.
pub fn matches_local(involvement: u8, state: &str, is_draft: bool, due: bool, raw: &str) -> bool {
    matches_facts(
        &LocalFacts {
            involvement,
            state,
            is_draft,
            due,
            teams: &[],
            assignees: &[],
            viewer: "",
        },
        raw,
    )
}

/// Like [`matches_local`], with pending reviewer teams and assignees.
/// Repeated `team-review-requested:` or `assignee:` values match if any one
/// does. `no:assignee` joins that same group, so a rule with both keeps a row
/// that is unassigned or assigned to one of the people.
pub fn matches_facts(facts: &LocalFacts<'_>, raw: &str) -> bool {
    let Ok(tokens) = github_query::scan(raw) else {
        return false;
    };
    let mut teams = Vec::new();
    let mut people = Vec::new();
    let mut unassigned = false;
    let mut rest = Vec::new();
    for token in tokens {
        let key = token.key.as_deref();
        if !token.negated && key == Some("team-review-requested") && !token.value.is_empty() {
            teams.push(token);
        } else if !token.negated && key == Some("assignee") && !token.value.is_empty() {
            people.push(token);
        } else if !token.negated
            && key == Some("no")
            && token.value.eq_ignore_ascii_case("assignee")
        {
            unassigned = true;
        } else {
            rest.push(token);
        }
    }
    if !teams.is_empty() && !teams.iter().any(|token| team_matches(facts, &token.value)) {
        return false;
    }
    if (!people.is_empty() || unassigned)
        && !((unassigned && facts.assignees.is_empty())
            || people
                .iter()
                .any(|token| assignee_matches(facts, &token.value)))
    {
        return false;
    }
    rest.iter().all(|token| match_token(token, facts, true))
}

fn team_matches(facts: &LocalFacts<'_>, value: &str) -> bool {
    facts
        .teams
        .iter()
        .any(|team| team.eq_ignore_ascii_case(value))
}

fn assignee_matches(facts: &LocalFacts<'_>, value: &str) -> bool {
    let login = if value.eq_ignore_ascii_case("@me") {
        facts.viewer
    } else {
        value
    };
    !login.is_empty()
        && facts
            .assignees
            .iter()
            .any(|have| have.eq_ignore_ascii_case(login))
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
        match_token(
            token,
            &LocalFacts {
                involvement: 0,
                state: "open",
                is_draft: false,
                due,
                teams: &[],
                assignees: &[],
                viewer: "",
            },
            false,
        )
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
    // A deleted view's key can be handed out again (`custom:N` is the highest
    // plus one), so its sort, grouping and filters must not outlive it.
    store.delete_custom_view_states_except(stored.iter().map(|view| view.key.clone()).collect())?;
    Ok(stored)
}

/// Collapsed sidebar sections. A missing or broken value is everything open.
pub fn load_collapse(store: &Store) -> Result<SidebarCollapse, CoreError> {
    let Some(entry) = store.get_setting(COLLAPSE_KEY.to_owned())? else {
        return Ok(SidebarCollapse::default());
    };
    let parsed: SidebarCollapse = serde_json::from_str(&entry.value_json).unwrap_or_default();
    Ok(normalize_collapse(parsed))
}

/// Stores which sections and parent views are collapsed. Unknown parent keys are dropped.
pub fn save_collapse(
    store: &Store,
    collapse: SidebarCollapse,
) -> Result<SidebarCollapse, CoreError> {
    let stored = normalize_collapse(collapse);
    let value_json = serde_json::to_string(&stored)
        .map_err(|err| CoreError::Store(format!("encode {COLLAPSE_KEY}: {err}")))?;
    store.put_setting(SettingEntry {
        key: COLLAPSE_KEY.to_owned(),
        value_json,
    })?;
    Ok(stored)
}

fn normalize_collapse(collapse: SidebarCollapse) -> SidebarCollapse {
    let mut parents = Vec::new();
    for key in collapse.parents {
        if (key == "mine" || key == "to_review") && !parents.contains(&key) {
            parents.push(key);
        }
    }
    SidebarCollapse {
        views: collapse.views,
        projects: collapse.projects,
        parents,
    }
}

fn builtin(key: &str, name: &str, query: &str, sort_order: i32) -> SidebarView {
    SidebarView {
        key: key.to_owned(),
        name: name.to_owned(),
        query: query.to_owned(),
        builtin: true,
        sort_order,
        parent: None,
    }
}

fn parent_key(parent: &Option<String>) -> Option<String> {
    match parent.as_deref() {
        Some("mine") => Some("mine".to_owned()),
        Some("to_review") => Some("to_review".to_owned()),
        _ => None,
    }
}

fn local_pair(key: &str, value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if key == "team-review-requested" || key == "assignee" {
        return true;
    }
    let value = value.to_ascii_lowercase();
    matches!(
        (key, value.as_str()),
        ("author", "@me")
            | ("review-requested", "@me")
            | ("is", "open" | "closed" | "merged" | "draft" | "pr")
            | ("state", "open" | "closed")
            | ("draft", "true" | "false")
            | ("bump", "due")
            | ("no", "assignee")
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

fn match_token(token: &Scanned, facts: &LocalFacts<'_>, strict: bool) -> bool {
    let Some(key) = token.key.as_deref() else {
        return true;
    };
    let value = token.value.to_ascii_lowercase();
    let open = facts.state.eq_ignore_ascii_case("open");
    let positive = match (key, value.as_str()) {
        ("author", "@me") => facts.involvement & AUTHOR != 0,
        ("review-requested", "@me") => facts.involvement & REVIEW_REQUESTED != 0,
        ("is", "open") | ("state", "open") | ("is", "pr") => true,
        // GitHub's `is:closed` includes merged pull requests.
        ("is", "closed") | ("state", "closed") => !open,
        ("is", "merged") => facts.state.eq_ignore_ascii_case("merged"),
        ("is", "draft") | ("draft", "true") => facts.is_draft,
        ("draft", "false") => !facts.is_draft,
        ("bump", "due") => facts.due,
        ("bump", _) => false,
        ("team-review-requested", _) if !token.value.is_empty() => {
            team_matches(facts, &token.value)
        }
        ("assignee", _) if !token.value.is_empty() => assignee_matches(facts, &token.value),
        ("no", "assignee") => facts.assignees.is_empty(),
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
            parent: None,
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
            parent: parent_key(&view.parent),
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
            parent: None,
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
            parent: parent_key(&view.parent),
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

    #[test]
    fn is_closed_includes_merged_like_github() {
        let rule = "is:closed author:@me";
        assert!(matches_local(AUTHOR, "closed", false, false, rule));
        assert!(matches_local(AUTHOR, "merged", false, false, rule));
        assert!(!matches_local(AUTHOR, "open", false, false, rule));
        assert!(matches_local(AUTHOR, "open", false, false, "-is:closed"));
        assert!(!matches_local(AUTHOR, "merged", false, false, "-is:closed"));
        assert!(matches_local(AUTHOR, "merged", false, false, "is:merged"));
        assert!(!matches_local(AUTHOR, "closed", false, false, "is:merged"));
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
            parent: Some("to_review".to_owned()),
        });
        let saved = save_views(&store, views.clone()).unwrap();
        assert_eq!(saved[0].name, "Mine");
        assert_eq!(saved[3].key, "custom:1");
        assert_eq!(saved[3].parent.as_deref(), Some("to_review"));

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
    fn deleting_a_custom_view_drops_its_saved_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("pr-nexus.db")).unwrap();
        let custom = |key: &str| SidebarView {
            key: key.to_owned(),
            name: key.to_owned(),
            query: String::new(),
            builtin: false,
            sort_order: 9,
            parent: None,
        };
        let mut views = defaults();
        views.extend([custom("custom:1"), custom("custom:2")]);
        save_views(&store, views.clone()).unwrap();
        for key in ["mine", "custom:1", "custom:2"] {
            store
                .put_view_state(crate::domain::ViewState {
                    view_key: key.to_owned(),
                    state_json: r#"{"filters":{"search":"old"}}"#.to_owned(),
                    updated_at: chrono::Utc::now(),
                })
                .unwrap();
        }
        views.retain(|view| view.key != "custom:2");
        save_views(&store, views).unwrap();
        // custom:2 is the next key handed out again: it must start clean.
        assert!(
            store
                .get_view_state("custom:2".to_owned())
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_view_state("custom:1".to_owned())
                .unwrap()
                .is_some()
        );
        assert!(store.get_view_state("mine".to_owned()).unwrap().is_some());
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
        assert_eq!(loaded[3].parent, None);
        assert_eq!(loaded[0].query, MINE_QUERY);
    }

    #[test]
    fn a_parent_is_kept_only_for_my_prs_and_to_review() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("pr-nexus.db")).unwrap();
        store
            .put_setting(SettingEntry {
                key: CATALOG_KEY.to_owned(),
                value_json: r#"[{"key":"custom:1","name":"Drafts","query":"is:draft","builtin":false,"sort_order":3,"parent":"mine"},{"key":"custom:2","name":"Elsewhere","query":"is:open","builtin":false,"sort_order":4,"parent":"bumps_due"}]"#.to_owned(),
            })
            .unwrap();
        let loaded = load_views(&store).unwrap();
        assert_eq!(
            loaded
                .iter()
                .find(|view| view.key == "custom:1")
                .unwrap()
                .parent
                .as_deref(),
            Some("mine")
        );
        assert_eq!(
            loaded
                .iter()
                .find(|view| view.key == "custom:2")
                .unwrap()
                .parent,
            None
        );

        let saved = save_collapse(
            &store,
            SidebarCollapse {
                views: true,
                projects: false,
                parents: vec!["mine".into(), "bumps_due".into(), "mine".into()],
            },
        )
        .unwrap();
        assert!(saved.views);
        assert!(!saved.projects);
        assert_eq!(saved.parents, vec!["mine".to_owned()]);
        assert_eq!(load_collapse(&store).unwrap(), saved);
        assert_eq!(
            load_collapse(&Store::open(dir.path().join("other.db")).unwrap()).unwrap(),
            SidebarCollapse::default()
        );
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
            parent: None,
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
            parent: None,
        });
        assert!(
            save_views(&store, views)
                .unwrap_err()
                .to_string()
                .contains("quote")
        );
    }

    fn facts<'a>(teams: &'a [String], assignees: &'a [String], viewer: &'a str) -> LocalFacts<'a> {
        LocalFacts {
            involvement: REVIEW_REQUESTED,
            state: "open",
            is_draft: false,
            due: false,
            teams,
            assignees,
            viewer,
        }
    }

    #[test]
    fn team_and_assignee_rules_stay_local_and_match_any_of_each() {
        let rule = "is:open team-review-requested:acme/web-platform team-review-requested:acme/payments assignee:@me no:assignee";
        let analyzed = analyze(rule);
        assert!(analyzed.fully_local);
        assert!(analyzed.github.is_empty());
        let web = "acme/web-platform".to_owned();
        let pay = "acme/payments".to_owned();
        let ada = "ada".to_owned();
        let sam = "sam".to_owned();
        assert!(matches_facts(
            &facts(std::slice::from_ref(&web), &[], "ada"),
            rule
        ));
        assert!(matches_facts(
            &facts(
                std::slice::from_ref(&pay),
                std::slice::from_ref(&ada),
                "ada"
            ),
            rule
        ));
        assert!(!matches_facts(
            &facts(
                std::slice::from_ref(&sam),
                std::slice::from_ref(&sam),
                "ada"
            ),
            rule
        ));
        let only_team = "is:open team-review-requested:acme/web-platform";
        assert!(matches_facts(
            &facts(
                std::slice::from_ref(&web),
                std::slice::from_ref(&sam),
                "ada"
            ),
            only_team
        ));
        assert!(!matches_facts(
            &facts(
                std::slice::from_ref(&pay),
                std::slice::from_ref(&ada),
                "ada"
            ),
            only_team
        ));
        assert!(matches_facts(
            &facts(&[], std::slice::from_ref(&ada), "ada"),
            "assignee:@me"
        ));
        assert!(!matches_facts(
            &facts(&[], std::slice::from_ref(&sam), "ada"),
            "assignee:@me"
        ));
        assert!(matches_facts(&facts(&[], &[], "ada"), "no:assignee"));
        assert!(!matches_facts(
            &facts(&[], std::slice::from_ref(&ada), "ada"),
            "no:assignee"
        ));
        assert!(!matches_facts(
            &facts(&[web], &[], "ada"),
            "-team-review-requested:acme/web-platform"
        ));
    }
}
