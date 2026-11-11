//! Which comment authors count as bots.
//!
//! GitHub Bot actors and logins that look like apps (`[bot]`, `-bot`,
//! `copilot`) are bots. Settings can add more logins. The detail panel and
//! the list row use [`split`] so human comment counts leave those authors out.

use serde::{Deserialize, Serialize};

use crate::stats::is_bot_login;

/// Where a note came from on the pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    /// A conversation comment on the pull request.
    Issue,
    /// A comment inside a review thread.
    Thread,
    /// The body of a submitted review.
    Review,
}

/// One fetched comment or review, bot or not.
///
/// Stored on the overview cache so a later change to the extra-login list
/// can reclassify it without another GitHub call. Snippets are already short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentNote {
    pub login: String,
    /// GraphQL actor type `Bot`, or a login [`is_bot_login`] already matches.
    pub actor_bot: bool,
    pub kind: NoteKind,
    /// Review-thread index in the overview query. `None` for other kinds.
    #[serde(default)]
    pub thread_id: Option<u32>,
    pub snippet: String,
    pub url: String,
    /// The review thread is unresolved. Issue comments and reviews are false.
    pub unresolved: bool,
    /// RFC 3339. Empty when GitHub omitted the time.
    pub at: String,
}

/// One bot comment or review, as the detail panel lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotNote {
    pub login: String,
    pub kind: NoteKind,
    pub snippet: String,
    pub url: String,
    pub unresolved: bool,
    pub at: String,
}

/// Bot comments and reviews for one pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BotReviews {
    pub count: u32,
    pub unresolved: u32,
    pub latest_snippet: Option<String>,
    pub notes: Vec<BotNote>,
}

impl BotReviews {
    pub fn is_empty(&self) -> bool {
        self.count == 0 && self.notes.is_empty()
    }
}

/// Human totals plus the bot section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    pub human_issue: u32,
    pub human_review: u32,
    pub bots: BotReviews,
}

/// `login` is a bot: the actor was a Bot, the login matches the built-in
/// rule, or it is on the extra list (case-insensitive, exact).
pub fn is_bot(login: &str, actor_bot: bool, extra: &[String]) -> bool {
    actor_bot || is_bot_login(login) || extra.iter().any(|item| item.eq_ignore_ascii_case(login))
}

/// `issue_total` and `review_total` are GitHub's counts, bots included.
/// Bot review bodies are listed but not subtracted: they are not thread
/// comments. Unresolved counts each unresolved bot thread once.
pub fn split(
    issue_total: u32,
    review_total: u32,
    notes: &[CommentNote],
    extra: &[String],
) -> Split {
    let mut bot_issue = 0u32;
    let mut bot_threads = 0u32;
    let mut unresolved = std::collections::BTreeSet::new();
    let mut kept = Vec::new();
    for note in notes {
        if !is_bot(&note.login, note.actor_bot, extra) {
            continue;
        }
        match note.kind {
            NoteKind::Issue => bot_issue += 1,
            NoteKind::Thread => {
                bot_threads += 1;
                if note.unresolved
                    && let Some(id) = note.thread_id
                {
                    unresolved.insert(id);
                }
            }
            NoteKind::Review => {}
        }
        kept.push(BotNote {
            login: note.login.clone(),
            kind: note.kind,
            snippet: note.snippet.clone(),
            url: note.url.clone(),
            unresolved: note.unresolved,
            at: note.at.clone(),
        });
    }
    kept.sort_by(|a, b| b.at.cmp(&a.at).then(a.login.cmp(&b.login)));
    let latest_snippet = kept
        .iter()
        .find(|note| !note.snippet.is_empty())
        .map(|note| note.snippet.clone());
    let count = u32::try_from(kept.len()).unwrap_or(u32::MAX);
    Split {
        human_issue: issue_total.saturating_sub(bot_issue),
        human_review: review_total.saturating_sub(bot_threads),
        bots: BotReviews {
            count,
            unresolved: u32::try_from(unresolved.len()).unwrap_or(u32::MAX),
            latest_snippet,
            notes: kept,
        },
    }
}

/// Bot notes in a cached overview JSON. A cache written before this field
/// existed, or one that does not parse, counts as none.
pub fn count_in_json(json: &str, extra: &[String]) -> u32 {
    #[derive(Deserialize)]
    struct Slim {
        #[serde(default)]
        feedback: Vec<CommentNote>,
    }
    serde_json::from_str::<Slim>(json)
        .map(|slim| split(0, 0, &slim.feedback, extra).bots.count)
        .unwrap_or(0)
}

/// Collapse whitespace and keep the first `max` characters.
pub fn snippet(body: &str, max: usize) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max).collect();
    if let Some(space) = cut.rfind(' ')
        && space > max / 2
    {
        cut.truncate(space);
    }
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(
        login: &str,
        actor_bot: bool,
        kind: NoteKind,
        thread: Option<u32>,
        unresolved: bool,
        at: &str,
    ) -> CommentNote {
        CommentNote {
            login: login.into(),
            actor_bot,
            kind,
            thread_id: thread,
            snippet: format!("from {login}"),
            url: format!("https://github.com/acme/widgets/pull/1#{login}"),
            unresolved,
            at: at.into(),
        }
    }

    #[test]
    fn bots_leave_the_human_counts_and_unresolved_threads_count_once() {
        let notes = vec![
            note(
                "ada",
                false,
                NoteKind::Issue,
                None,
                false,
                "2026-10-01T00:00:00Z",
            ),
            note(
                "dependabot[bot]",
                true,
                NoteKind::Issue,
                None,
                false,
                "2026-10-02T00:00:00Z",
            ),
            note(
                "copilot",
                false,
                NoteKind::Thread,
                Some(0),
                true,
                "2026-10-03T00:00:00Z",
            ),
            note(
                "copilot",
                false,
                NoteKind::Thread,
                Some(0),
                true,
                "2026-10-04T00:00:00Z",
            ),
            note(
                "ci-helper[bot]",
                true,
                NoteKind::Review,
                None,
                false,
                "2026-10-05T00:00:00Z",
            ),
            note(
                "sam",
                false,
                NoteKind::Thread,
                Some(1),
                true,
                "2026-10-01T00:00:00Z",
            ),
        ];
        let split = split(4, 6, &notes, &[]);
        assert_eq!(split.human_issue, 3);
        assert_eq!(split.human_review, 4);
        assert_eq!(split.bots.count, 4);
        assert_eq!(split.bots.unresolved, 1);
        assert_eq!(
            split.bots.latest_snippet.as_deref(),
            Some("from ci-helper[bot]")
        );
        assert_eq!(split.bots.notes[0].login, "ci-helper[bot]");
    }

    #[test]
    fn an_extra_login_is_a_bot_and_a_human_login_is_not() {
        let notes = vec![note(
            "review-helper",
            false,
            NoteKind::Issue,
            None,
            false,
            "2026-10-01T00:00:00Z",
        )];
        let extra = vec!["Review-Helper".into()];
        assert_eq!(split(1, 0, &notes, &extra).human_issue, 0);
        assert_eq!(split(1, 0, &notes, &[]).human_issue, 1);
        assert!(is_bot("Copilot", false, &[]));
        assert!(!is_bot("octocat", false, &[]));
    }

    #[test]
    fn snippet_stops_on_a_word_and_count_in_json_reads_feedback() {
        assert_eq!(snippet("one two three four", 10), "one two…");
        let json = serde_json::json!({
            "issue_comments": 2,
            "feedback": [note("dependabot[bot]", true, NoteKind::Issue, None, false, "2026-10-01T00:00:00Z")]
        });
        assert_eq!(count_in_json(&json.to_string(), &[]), 1);
        assert_eq!(count_in_json("{", &[]), 0);
    }
}
