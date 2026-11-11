//! A GitHub pull-request search the user typed, turned into a query the
//! search API can run.
//!
//! GitHub's website accepts a few tokens the API does not: `@today` and
//! `@today-1d` / `-1w` / `-1m` / `-1y` (with `>`, `>=`, `<`, `<=` and `..`),
//! `sort:updated-desc`, and sometimes `@me`. This module resolves those and
//! always sends `is:pr`, so a search never returns issues. Everything else
//! (`archived:`, `state:`, `author:`, `review-requested:`, `repo:`, `org:`,
//! `label:`, `draft:`, `involves:`, negation, quotes) is passed through.
//!
//! The list search box also holds free text. [`partition`] splits that off
//! so only qualifier tokens reach [`prepare`]. The app mirrors the same split
//! and filters the free text locally.
//!
//! The repository picker is ANDed by adding `repo:` terms. GitHub ORs several
//! `repo:` qualifiers, so when the query already names repositories only the
//! overlap is searched.

use std::collections::HashSet;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};

use crate::sync::SEARCH_QUERY_MAX;

/// When the dates are resolved. `offset_east_minutes` is the user's zone,
/// minutes east of UTC (Pacific time is `-480`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryClock {
    pub now: DateTime<Utc>,
    pub offset_east_minutes: i32,
}

/// How GitHub should order the search. Absent means GitHub's own order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOrder {
    /// REST `sort` value: `updated`, `created`, `comments`, …
    pub sort: String,
    pub descending: bool,
}

/// A query ready to send, still without the repository picker's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedQuery {
    /// Search string. Dates are absolute, `@me` is the login, `is:pr` is set,
    /// and `sort:` has been removed.
    pub q: String,
    pub order: Option<SearchOrder>,
    /// The query asks only for open pull requests, so the open list (and
    /// removing a pull request the moment it is merged or closed) still applies.
    pub open_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    UnfinishedQuote,
    Sort(String),
    TooLong,
    Repos,
    Login,
}

impl QueryError {
    pub fn message(&self) -> String {
        match self {
            Self::UnfinishedQuote => "The query has an unfinished quote.".to_owned(),
            Self::Sort(name) => format!(
                "GitHub search cannot sort by \"{name}\". Use updated, created, comments, reactions, or interactions."
            ),
            Self::TooLong => "That query is longer than GitHub allows (256 characters).".to_owned(),
            Self::Repos => {
                "No repository is in both the query and the repository filter.".to_owned()
            }
            Self::Login => "This GitHub account has no login to use for @me.".to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
struct Piece {
    negated: bool,
    key: Option<String>,
    value: String,
    quoted: bool,
}

/// Qualifier tokens and the free text that was typed beside them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchParts {
    /// `key:value` tokens, in order, as typed. This is what [`prepare`] runs.
    pub github: String,
    /// Words that are not qualifiers. They stay on the machine that typed them.
    pub text: String,
}

/// Splits a search field into GitHub qualifiers and free text.
///
/// A qualifier is `key:value` (optional leading `-`, quotes allowed) whose key
/// starts with a letter and then letters, digits, `-`, or `_`. A pasted
/// `https://…` link is free text. An unfinished quote is
/// [`QueryError::UnfinishedQuote`], as in [`prepare`].
pub fn partition(raw: &str) -> Result<SearchParts, QueryError> {
    let mut github = Vec::new();
    let mut text = Vec::new();
    for token in raw_tokens(raw)? {
        if is_qualifier_token(&token) {
            github.push(token);
        } else {
            text.push(token);
        }
    }
    Ok(SearchParts {
        github: github.join(" "),
        text: text.join(" "),
    })
}

/// Resolves dates, `@me`, `sort:`, and `is:pr`. `me` is that account's login;
/// it is only checked when the query uses `@me`.
pub fn prepare(raw: &str, me: &str, clock: &QueryClock) -> Result<PreparedQuery, QueryError> {
    let today = local_date(clock);
    let pieces = tokenize(raw)?;
    let open_only = is_open_only(&pieces);
    let mut order = None;
    let mut kept = Vec::new();
    let mut saw_pr = false;
    for mut piece in pieces {
        if piece.key.as_deref() == Some("sort") && !piece.negated {
            order = Some(parse_sort(&piece.value)?);
            continue;
        }
        if piece.key.as_deref() == Some("sort") && piece.negated {
            continue;
        }
        if piece.key.as_deref() == Some("is")
            && !piece.negated
            && piece.value.eq_ignore_ascii_case("issue")
        {
            continue;
        }
        if piece.key.as_deref() == Some("is") && piece.value.eq_ignore_ascii_case("pr") {
            if piece.negated {
                continue;
            }
            if saw_pr {
                continue;
            }
            saw_pr = true;
        }
        if let Some(resolved) = resolve_date_value(&piece.value, today) {
            piece.value = resolved;
            piece.quoted = piece.value.chars().any(char::is_whitespace);
        }
        if piece.value.eq_ignore_ascii_case("@me") {
            if !login_ok(me) {
                return Err(QueryError::Login);
            }
            piece.value = me.to_owned();
            piece.quoted = false;
        }
        kept.push(piece);
    }
    if !saw_pr {
        kept.insert(
            0,
            Piece {
                negated: false,
                key: Some("is".to_owned()),
                value: "pr".to_owned(),
                quoted: false,
            },
        );
    }
    let q = kept.iter().map(render).collect::<Vec<_>>().join(" ");
    if q.len() > SEARCH_QUERY_MAX {
        return Err(QueryError::TooLong);
    }
    Ok(PreparedQuery {
        q,
        order,
        open_only,
    })
}

/// ANDs the repository picker onto `query`.
///
/// An empty picker leaves the query unchanged (one string). Otherwise the
/// picker's repositories are added as `repo:` terms, which GitHub ORs with
/// each other and ANDs with the rest. A query that already names repositories
/// is limited to the overlap. The result may be several strings when one
/// would pass GitHub's 256-character cap; together they cover the same set.
pub fn and_repos(query: &str, repos: &[String]) -> Result<Vec<String>, QueryError> {
    let wanted = valid_repo_names(repos);
    if wanted.is_empty() {
        if query.len() > SEARCH_QUERY_MAX {
            return Err(QueryError::TooLong);
        }
        return Ok(vec![query.to_owned()]);
    }
    let pieces = tokenize(query)?;
    let mut named = Vec::new();
    let mut stem = Vec::new();
    for piece in pieces {
        if !piece.negated
            && piece.key.as_deref() == Some("repo")
            && let Some(name) = normalize_repo(&piece.value)
        {
            if !named.iter().any(|known: &String| known == &name) {
                named.push(name);
            }
            continue;
        }
        stem.push(render(&piece));
    }
    let effective = if named.is_empty() {
        wanted
    } else {
        let chosen: HashSet<&str> = wanted.iter().map(String::as_str).collect();
        let overlap: Vec<String> = named
            .into_iter()
            .filter(|name| chosen.contains(name.as_str()))
            .collect();
        if overlap.is_empty() {
            return Err(QueryError::Repos);
        }
        overlap
    };
    pack(&stem.join(" "), &effective)
}

fn pack(stem: &str, repos: &[String]) -> Result<Vec<String>, QueryError> {
    if repos.is_empty() {
        if stem.len() > SEARCH_QUERY_MAX {
            return Err(QueryError::TooLong);
        }
        return Ok(vec![stem.to_owned()]);
    }
    let mut out = Vec::new();
    let mut current = stem.to_owned();
    for repo in repos {
        let piece = format!(" repo:{repo}");
        if stem.len() + piece.len() > SEARCH_QUERY_MAX {
            return Err(QueryError::TooLong);
        }
        if current.len() + piece.len() > SEARCH_QUERY_MAX {
            out.push(std::mem::take(&mut current));
            current = format!("{stem}{piece}");
        } else {
            current.push_str(&piece);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    Ok(out)
}

fn is_open_only(pieces: &[Piece]) -> bool {
    let mut open = false;
    let mut finished = false;
    for piece in pieces {
        if piece.negated {
            continue;
        }
        let Some(key) = piece.key.as_deref() else {
            continue;
        };
        let value = piece.value.as_str();
        if (key == "is" || key == "state") && value.eq_ignore_ascii_case("open") {
            open = true;
        }
        if key == "is"
            && (value.eq_ignore_ascii_case("closed") || value.eq_ignore_ascii_case("merged"))
        {
            finished = true;
        }
        if key == "state"
            && (value.eq_ignore_ascii_case("closed") || value.eq_ignore_ascii_case("merged"))
        {
            finished = true;
        }
    }
    open && !finished
}

fn parse_sort(value: &str) -> Result<SearchOrder, QueryError> {
    let (name, descending) = if let Some(name) = value.strip_suffix("-desc") {
        (name, true)
    } else if let Some(name) = value.strip_suffix("-asc") {
        (name, false)
    } else {
        (value, true)
    };
    let sort = match name {
        "updated"
        | "created"
        | "comments"
        | "interactions"
        | "reactions"
        | "reactions-+1"
        | "reactions--1"
        | "reactions-smile"
        | "reactions-thinking_face"
        | "reactions-heart"
        | "reactions-tada" => name.to_owned(),
        _ => return Err(QueryError::Sort(value.to_owned())),
    };
    Ok(SearchOrder { sort, descending })
}

fn login_ok(login: &str) -> bool {
    !login.is_empty()
        && login
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

fn local_date(clock: &QueryClock) -> NaiveDate {
    let minutes = clock.offset_east_minutes.clamp(-14 * 60, 14 * 60);
    let offset = chrono::FixedOffset::east_opt(minutes * 60)
        .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("utc"));
    clock.now.with_timezone(&offset).date_naive()
}

/// `Some` when `value` is a relative date (optionally compared or a range).
fn resolve_date_value(value: &str, today: NaiveDate) -> Option<String> {
    if let Some((left, right)) = value.split_once("..") {
        let left = resolve_side(left, today)?;
        let right = resolve_side(right, today)?;
        return Some(format!("{left}..{right}"));
    }
    resolve_side(value, today)
}

fn resolve_side(side: &str, today: NaiveDate) -> Option<String> {
    let (op, rest) = strip_op(side);
    let date = relative_date(rest, today)?;
    Some(format!("{op}{}", date.format("%Y-%m-%d")))
}

fn strip_op(side: &str) -> (&str, &str) {
    if let Some(rest) = side.strip_prefix(">=") {
        (">=", rest)
    } else if let Some(rest) = side.strip_prefix("<=") {
        ("<=", rest)
    } else if let Some(rest) = side.strip_prefix('>') {
        (">", rest)
    } else if let Some(rest) = side.strip_prefix('<') {
        ("<", rest)
    } else {
        ("", side)
    }
}

/// `@today` or `@today-<n><d|w|m|y>`.
fn relative_date(token: &str, today: NaiveDate) -> Option<NaiveDate> {
    let rest = token.strip_prefix("@today")?;
    if rest.is_empty() {
        return Some(today);
    }
    let rest = rest.strip_prefix('-')?;
    let split = rest.find(|ch: char| !ch.is_ascii_digit())?;
    if split == 0 {
        return None;
    }
    let (count, unit) = rest.split_at(split);
    if unit.chars().count() != 1 {
        return None;
    }
    let count: u32 = count.parse().ok()?;
    let unit = unit.chars().next()?.to_ascii_lowercase();
    if !matches!(unit, 'd' | 'w' | 'm' | 'y') {
        return None;
    }
    Some(shift(today, count, unit))
}

fn shift(today: NaiveDate, count: u32, unit: char) -> NaiveDate {
    match unit {
        'd' => today - Duration::days(i64::from(count)),
        'w' => today - Duration::days(i64::from(count) * 7),
        'm' => sub_months(today, count),
        'y' => sub_months(today, count.saturating_mul(12)),
        _ => today,
    }
}

fn sub_months(date: NaiveDate, months: u32) -> NaiveDate {
    let index = date.year() * 12 + i32::try_from(date.month()).unwrap_or(1)
        - 1
        - i32::try_from(months).unwrap_or(i32::MAX);
    let year = index.div_euclid(12);
    let month = u32::try_from(index.rem_euclid(12) + 1).unwrap_or(1);
    let day = date.day();
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or_else(|| {
        let (next_year, next_month) = if month == 12 {
            (year + 1, 1)
        } else {
            (year, month + 1)
        };
        NaiveDate::from_ymd_opt(next_year, next_month, 1)
            .and_then(|first| first.pred_opt())
            .unwrap_or(date)
    })
}

fn raw_tokens(input: &str) -> Result<Vec<String>, QueryError> {
    let mut out = Vec::new();
    let mut rest = input;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let (token, next) = take_token(rest)?;
        rest = next;
        if !token.is_empty() {
            out.push(token);
        }
    }
    Ok(out)
}

/// True when a non-negated `sort:` qualifier sets the result order.
///
/// Other qualifiers still search GitHub. The list then uses the chosen sort.
pub fn sets_order(raw: &str) -> bool {
    let Ok(tokens) = raw_tokens(raw) else {
        return false;
    };
    tokens.iter().any(|token| is_sort_token(token))
}

/// A non-negated `sort:` qualifier. `-sort:` does not set the order.
fn is_sort_token(token: &str) -> bool {
    if !is_qualifier_token(token) || token.starts_with('-') {
        return false;
    }
    colon_outside_quotes(token).is_some_and(|at| at > 0 && &token[..at] == "sort")
}

/// `key:value`, with an optional leading `-`. `https://…` is not a qualifier.
fn is_qualifier_token(token: &str) -> bool {
    let body = match token.strip_prefix('-') {
        Some(stripped)
            if !stripped.is_empty() && !stripped.starts_with(|ch: char| ch.is_whitespace()) =>
        {
            stripped
        }
        _ => token,
    };
    let Some(at) = colon_outside_quotes(body) else {
        return false;
    };
    if at == 0 || !is_key(&body[..at]) {
        return false;
    }
    !body[at + 1..].starts_with("//")
}

fn tokenize(input: &str) -> Result<Vec<Piece>, QueryError> {
    let mut out = Vec::new();
    let mut rest = input;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let (negated, body) = if let Some(stripped) = rest.strip_prefix('-') {
            if stripped.starts_with(|ch: char| ch.is_whitespace()) || stripped.is_empty() {
                (false, rest)
            } else {
                (true, stripped)
            }
        } else {
            (false, rest)
        };
        let (token, next) = take_token(body)?;
        rest = next;
        if token.is_empty() {
            continue;
        }
        out.push(parse_piece(negated, &token));
    }
    Ok(out)
}

/// The next token and what follows it. A token ends at unquoted whitespace.
fn take_token(input: &str) -> Result<(String, &str), QueryError> {
    let mut in_quote = false;
    let mut escape = false;
    for (index, ch) in input.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && in_quote {
            escape = true;
            continue;
        }
        if ch == '"' {
            in_quote = !in_quote;
            continue;
        }
        if ch.is_whitespace() && !in_quote {
            return Ok((input[..index].to_owned(), &input[index..]));
        }
    }
    if in_quote || escape {
        return Err(QueryError::UnfinishedQuote);
    }
    Ok((input.to_owned(), ""))
}

fn parse_piece(negated: bool, raw: &str) -> Piece {
    let colon = colon_outside_quotes(raw);
    let Some(at) = colon else {
        return Piece {
            negated,
            key: None,
            value: raw.to_owned(),
            quoted: false,
        };
    };
    let key = &raw[..at];
    if !is_key(key) {
        return Piece {
            negated,
            key: None,
            value: raw.to_owned(),
            quoted: false,
        };
    }
    let (value, quoted) = unwrap_quotes(&raw[at + 1..]);
    Piece {
        negated,
        key: Some(key.to_ascii_lowercase()),
        value,
        quoted,
    }
}

fn colon_outside_quotes(raw: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut escape = false;
    for (index, ch) in raw.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && in_quote {
            escape = true;
            continue;
        }
        if ch == '"' {
            in_quote = !in_quote;
            continue;
        }
        if ch == ':' && !in_quote {
            return Some(index);
        }
    }
    None
}

fn is_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

/// Quotes around the whole value come off. A value that was not wrapped is
/// returned unchanged, quotes included, so a stray quote is not eaten.
fn unwrap_quotes(value: &str) -> (String, bool) {
    let mut chars = value.chars();
    if chars.next() != Some('"') || !value.ends_with('"') || value.len() < 2 {
        return (value.to_owned(), false);
    }
    let inner = &value[1..value.len() - 1];
    let mut out = String::new();
    let mut escape = false;
    for ch in inner.chars() {
        if escape {
            out.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        out.push(ch);
    }
    if escape {
        out.push('\\');
    }
    (out, true)
}

fn render(piece: &Piece) -> String {
    let mut out = String::new();
    if piece.negated {
        out.push('-');
    }
    if let Some(key) = &piece.key {
        out.push_str(key);
        out.push(':');
    }
    let quote = piece.quoted || piece.value.chars().any(char::is_whitespace);
    if quote {
        out.push('"');
        for ch in piece.value.chars() {
            if ch == '"' || ch == '\\' {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('"');
    } else {
        out.push_str(&piece.value);
    }
    out
}

fn valid_repo_names(repos: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for repo in repos {
        let Some(name) = normalize_repo(repo) else {
            continue;
        };
        if !out.iter().any(|known: &String| known == &name) {
            out.push(name);
        }
    }
    out
}

fn normalize_repo(repo: &str) -> Option<String> {
    let (owner, name) = repo.trim().split_once('/')?;
    let plain = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    };
    (plain(owner) && plain(name)).then(|| {
        format!(
            "{}/{}",
            owner.to_ascii_lowercase(),
            name.to_ascii_lowercase()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> QueryClock {
        QueryClock {
            // 02:00 UTC is still the previous evening in a zone 8 hours west.
            now: DateTime::parse_from_rfc3339("2026-11-09T02:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            offset_east_minutes: -480,
        }
    }

    fn prepared(raw: &str) -> PreparedQuery {
        prepare(raw, "octocat", &clock()).unwrap()
    }

    #[test]
    fn partition_sends_qualifiers_to_github_and_keeps_free_text() {
        let parts = partition(
            r#"author:@me passkeys -label:"bug fix" created:>@today-1w "two words" repo:acme/widgets sort:updated-desc"#,
        )
        .unwrap();
        assert_eq!(
            parts.github,
            r#"author:@me -label:"bug fix" created:>@today-1w repo:acme/widgets sort:updated-desc"#
        );
        assert_eq!(parts.text, r#"passkeys "two words""#);
        let bare = partition("draft:true draft is:draft").unwrap();
        assert_eq!(bare.github, "draft:true is:draft");
        assert_eq!(bare.text, "draft");
        assert!(!sets_order("author:@me is:draft created:>@today-1w"));
        assert!(sets_order("author:@me sort:created-asc passkeys"));
        assert!(sets_order("sort:\"created-asc\""));
        assert!(!sets_order("-sort:updated-desc"));
    }

    #[test]
    fn partition_leaves_urls_and_plain_words_as_free_text() {
        let parts = partition("https://github.com/acme/widgets/pull/1 fix(api): #123").unwrap();
        assert_eq!(parts.github, "");
        assert_eq!(
            parts.text,
            "https://github.com/acme/widgets/pull/1 fix(api): #123"
        );
        assert_eq!(
            partition("label:\"bug").unwrap_err(),
            QueryError::UnfinishedQuote
        );
    }

    #[test]
    fn relative_dates_use_the_local_calendar_day() {
        let query = prepared(
            "created:>@today-1w updated:>=@today closed:<@today-1d merged:<=@today-1m closed:@today-1y",
        );
        assert_eq!(
            query.q,
            "is:pr created:>2026-11-01 updated:>=2026-11-08 closed:<2026-11-07 merged:<=2026-10-08 closed:2025-11-08"
        );
        assert!(!query.open_only);
    }

    #[test]
    fn a_date_range_and_comparisons_resolve_both_ends() {
        let query = prepared("created:@today-1w..@today updated:>@today-1m..<=@today-1d");
        assert_eq!(
            query.q,
            "is:pr created:2026-11-01..2026-11-08 updated:>2026-10-08..<=2026-11-07"
        );
    }

    #[test]
    fn sort_maps_to_the_api_and_leaves_the_query() {
        let updated = prepared("author:octocat sort:updated-desc");
        assert_eq!(updated.q, "is:pr author:octocat");
        assert_eq!(updated.order.as_ref().unwrap().sort, "updated");
        assert!(updated.order.as_ref().unwrap().descending);
        let created = prepared("sort:created-asc is:pr");
        assert_eq!(created.q, "is:pr");
        assert_eq!(created.order.as_ref().unwrap().sort, "created");
        assert!(!created.order.as_ref().unwrap().descending);
        let comments = prepared("sort:comments-desc");
        assert_eq!(comments.order.as_ref().unwrap().sort, "comments");
        assert!(comments.order.as_ref().unwrap().descending);
        let plain = prepared("sort:comments");
        assert!(plain.order.as_ref().unwrap().descending);
        assert_eq!(plain.order.as_ref().unwrap().sort, "comments");
    }

    #[test]
    fn an_unknown_sort_is_a_plain_error() {
        let err = prepare("sort:foo", "octocat", &clock()).unwrap_err();
        assert!(err.message().contains("cannot sort by \"foo\""));
        assert!(!err.message().contains("422"));
    }

    #[test]
    fn is_pr_is_enforced_and_issues_are_dropped() {
        assert_eq!(prepared("author:octocat").q, "is:pr author:octocat");
        assert_eq!(
            prepared("is:issue author:octocat").q,
            "is:pr author:octocat"
        );
        assert_eq!(prepared("-is:pr author:octocat").q, "is:pr author:octocat");
        assert_eq!(prepared("is:pr is:pr").q, "is:pr");
    }

    #[test]
    fn me_quotes_and_negation_pass_through() {
        let query = prepared(
            r#"author:@me -label:wip label:"bug fix" review-requested:@me involves:@me draft:false"#,
        );
        assert_eq!(
            query.q,
            r#"is:pr author:octocat -label:wip label:"bug fix" review-requested:octocat involves:octocat draft:false"#
        );
    }

    #[test]
    fn an_unfinished_quote_is_reported() {
        let err = prepare("label:\"bug", "octocat", &clock()).unwrap_err();
        assert_eq!(err, QueryError::UnfinishedQuote);
        assert_eq!(err.message(), "The query has an unfinished quote.");
    }

    #[test]
    fn me_needs_a_login() {
        let err = prepare("author:@me", "", &clock()).unwrap_err();
        assert_eq!(err, QueryError::Login);
        assert!(prepare("author:octocat", "", &clock()).is_ok());
    }

    #[test]
    fn open_only_is_an_open_qualifier_without_a_finished_one() {
        assert!(prepared("is:pr state:open archived:false").open_only);
        assert!(prepared("is:open -is:closed").open_only);
        assert!(!prepared("author:octocat").open_only);
        assert!(!prepared("is:open is:closed").open_only);
        assert!(!prepared("is:merged").open_only);
        assert!(!prepared("state:closed").open_only);
    }

    #[test]
    fn qualifiers_the_api_already_understands_are_kept() {
        let query = prepared(
            "archived:false state:open author:octocat review-requested:@me repo:acme/widgets org:acme label:bug draft:true involves:octocat -author:ada",
        );
        assert_eq!(
            query.q,
            "is:pr archived:false state:open author:octocat review-requested:octocat repo:acme/widgets org:acme label:bug draft:true involves:octocat -author:ada"
        );
        assert!(query.open_only);
    }

    #[test]
    fn the_repository_filter_is_anded() {
        let base = prepared("author:octocat").q;
        assert_eq!(
            and_repos(&base, &["acme/widgets".into(), "Acme/Widgets".into()]).unwrap(),
            vec!["is:pr author:octocat repo:acme/widgets".to_owned()]
        );
    }

    #[test]
    fn a_query_repo_and_the_picker_intersect() {
        let base = prepared("repo:Acme/Widgets repo:acme/other author:octocat").q;
        assert_eq!(
            and_repos(&base, &["acme/widgets".into(), "acme-labs/widgets".into()]).unwrap(),
            vec!["is:pr author:octocat repo:acme/widgets".to_owned()]
        );
        let err = and_repos(&base, &["acme-labs/widgets".into()]).unwrap_err();
        assert_eq!(err, QueryError::Repos);
        assert_eq!(
            err.message(),
            "No repository is in both the query and the repository filter."
        );
    }

    #[test]
    fn an_empty_picker_leaves_repo_terms_alone() {
        let base = prepared("repo:Acme/Widgets author:octocat").q;
        assert_eq!(
            and_repos(&base, &[]).unwrap(),
            vec!["is:pr repo:Acme/Widgets author:octocat".to_owned()]
        );
    }

    #[test]
    fn negated_repos_stay_while_the_picker_is_added() {
        let queries = and_repos(
            "is:pr -repo:acme/secret author:octocat",
            &["acme/widgets".into()],
        )
        .unwrap();
        assert_eq!(
            queries,
            vec!["is:pr -repo:acme/secret author:octocat repo:acme/widgets".to_owned()]
        );
    }
}
