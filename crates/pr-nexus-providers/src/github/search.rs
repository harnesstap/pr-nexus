//! Exhaustive search: every page of every query string, split by creation
//! date when GitHub has more matches than one search lets through.
//!
//! GitHub search answers at most [`SEARCH_RESULT_LIMIT`] results per query
//! string, however many pages are followed. The first page of each query
//! carries `issueCount`; above the limit the query is split into two
//! `created:` ranges, and each half is checked again. Up to
//! [`SEARCH_WORKERS`] ranges are paged at once. Pages reach the caller in
//! arrival order on the calling thread, so it can store each page right away.

use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::{Condvar, Mutex};

use chrono::{DateTime, Duration as Span, Utc};
use pr_nexus_core::domain::Involvement;
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::providers::SyncedPullRequest;
use serde_json::{Value, json};

use super::{GithubClient, SEARCH, parse};

/// The most results GitHub returns for one search query string.
pub const SEARCH_RESULT_LIMIT: u64 = 1000;

/// Date ranges paged at once. The client's gate still caps requests at two.
pub const SEARCH_WORKERS: usize = 2;

/// GitHub has no pull requests created before it opened.
const FIRST_SECOND: i64 = 1_199_145_600; // 2008-01-01T00:00:00Z

/// An inclusive `created:` range, to the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Created {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl Created {
    /// Every moment a pull request can have been created at, through a day
    /// from now so a clock behind GitHub's still covers the newest.
    pub fn everything() -> Self {
        Self {
            from: DateTime::from_timestamp(FIRST_SECOND, 0).expect("first second"),
            to: Utc::now() + Span::days(1),
        }
    }

    /// Two halves that together cover `self`, or `None` for a single second.
    pub fn split(self) -> Option<(Self, Self)> {
        let seconds = (self.to - self.from).num_seconds();
        if seconds < 1 {
            return None;
        }
        let mid = self.from + Span::seconds(seconds / 2);
        Some((
            Self {
                from: self.from,
                to: mid,
            },
            Self {
                from: mid + Span::seconds(1),
                to: self.to,
            },
        ))
    }

    fn qualifier(self) -> String {
        const FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";
        format!(
            " created:{}..{}",
            self.from.format(FORMAT),
            self.to.format(FORMAT)
        )
    }
}

/// `query` narrowed to `range`; the whole query when `range` is `None`.
pub fn ranged(query: &str, range: Option<Created>) -> String {
    match range {
        None => query.to_owned(),
        Some(range) => format!("{query}{}", range.qualifier()),
    }
}

/// Ranges waiting to be paged, shared by the workers.
struct Work {
    state: Mutex<WorkState>,
    changed: Condvar,
}

struct WorkState {
    /// `None` is the query without a `created:` qualifier.
    pending: Vec<Option<Created>>,
    active: usize,
    stopped: bool,
}

impl Work {
    fn new() -> Self {
        Self {
            state: Mutex::new(WorkState {
                pending: vec![None],
                active: 0,
                stopped: false,
            }),
            changed: Condvar::new(),
        }
    }

    /// The next range, waiting while another worker may still split one.
    /// `None` once everything is paged or a worker failed.
    fn take(&self) -> Option<Option<Created>> {
        let mut state = self.state.lock().expect("search work");
        loop {
            if state.stopped {
                return None;
            }
            if let Some(range) = state.pending.pop() {
                state.active += 1;
                return Some(range);
            }
            if state.active == 0 {
                return None;
            }
            state = self.changed.wait(state).expect("search work");
        }
    }

    fn push(&self, range: Created) {
        self.state
            .lock()
            .expect("search work")
            .pending
            .push(Some(range));
        self.changed.notify_one();
    }

    fn finish(&self, failed: bool) {
        let mut state = self.state.lock().expect("search work");
        state.active -= 1;
        state.stopped |= failed;
        self.changed.notify_all();
    }
}

type PageMessage = Result<Vec<SyncedPullRequest>, ProviderError>;

impl GithubClient {
    /// Every result of `query`, page by page. `on_page` gets each page's rows
    /// not seen before (by node id, across calls sharing `seen`).
    pub(crate) fn search_all(
        &self,
        what: &str,
        query: &str,
        involvement: Involvement,
        seen: &mut HashSet<String>,
        on_page: &mut dyn FnMut(Vec<SyncedPullRequest>),
    ) -> Result<(), ProviderError> {
        let work = Work::new();
        let (tx, rx) = mpsc::channel::<PageMessage>();
        std::thread::scope(|scope| {
            for _ in 0..SEARCH_WORKERS {
                let tx = tx.clone();
                let work = &work;
                scope.spawn(move || {
                    while let Some(range) = work.take() {
                        let outcome = self.search_range(what, query, range, involvement, work, &tx);
                        let failed = outcome.is_err();
                        if let Err(err) = outcome {
                            let _ = tx.send(Err(err));
                        }
                        work.finish(failed);
                    }
                });
            }
            drop(tx);
            let mut first_error = None;
            for message in rx {
                match message {
                    Ok(items) => {
                        let fresh: Vec<_> = items
                            .into_iter()
                            .filter(|item| seen.insert(item.pull_request.remote.id.clone()))
                            .collect();
                        if !fresh.is_empty() {
                            on_page(fresh);
                        }
                    }
                    Err(err) => {
                        first_error.get_or_insert(err);
                    }
                }
            }
            first_error.map_or(Ok(()), Err)
        })
    }

    /// Pages `query` within `range`. When the first page reports more matches
    /// than GitHub lets through, queues both halves of the range instead.
    fn search_range(
        &self,
        what: &str,
        query: &str,
        range: Option<Created>,
        involvement: Involvement,
        work: &Work,
        tx: &mpsc::Sender<PageMessage>,
    ) -> Result<(), ProviderError> {
        let q = ranged(query, range);
        let mut level = 0;
        let mut cursor = Value::Null;
        let mut first = true;
        loop {
            let body = self.paged(what, &mut level, |size| {
                self.graphql(
                    SEARCH,
                    json!({ "query": q, "first": size, "cursor": cursor }),
                )
            })?;
            let page = parse::parse_search(&body, self.account, involvement)?;
            if first && page.total > SEARCH_RESULT_LIMIT {
                let whole = range.unwrap_or_else(Created::everything);
                if let Some((older, newer)) = whole.split() {
                    work.push(older);
                    work.push(newer);
                    return Ok(());
                }
                // One second with more matches than GitHub lets through:
                // nothing narrower exists, so page what GitHub returns.
            }
            first = false;
            if tx.send(Ok(page.items)).is_err() {
                return Ok(());
            }
            if !page.has_next {
                return Ok(());
            }
            cursor = page.cursor.map(Value::String).unwrap_or(Value::Null);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_600_000_000 + seconds, 0).unwrap()
    }

    #[test]
    fn split_halves_cover_the_range_without_overlap() {
        let range = Created {
            from: at(0),
            to: at(9),
        };
        let (older, newer) = range.split().unwrap();
        assert_eq!(older.from, range.from);
        assert_eq!(newer.to, range.to);
        assert_eq!(older.to + Span::seconds(1), newer.from);
        assert_eq!(
            Created {
                from: at(5),
                to: at(5)
            }
            .split(),
            None
        );
        let two = Created {
            from: at(0),
            to: at(1),
        };
        let (a, b) = two.split().unwrap();
        assert_eq!((a.from, a.to, b.from, b.to), (at(0), at(0), at(1), at(1)));
    }

    #[test]
    fn everything_starts_before_the_first_pull_request() {
        let all = Created::everything();
        assert_eq!(all.from.format("%Y-%m-%d").to_string(), "2008-01-01");
        assert!(all.to > Utc::now());
    }

    #[test]
    fn ranged_adds_a_created_qualifier() {
        assert_eq!(ranged("is:pr org:acme", None), "is:pr org:acme");
        let range = Created {
            from: DateTime::from_timestamp(1_614_816_000, 0).unwrap(),
            to: DateTime::from_timestamp(1_620_259_199, 0).unwrap(),
        };
        assert_eq!(
            ranged("is:pr org:acme", Some(range)),
            "is:pr org:acme created:2021-03-04T00:00:00Z..2021-05-05T23:59:59Z"
        );
    }
}
