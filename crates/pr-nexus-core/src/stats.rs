//! Review statistics for Settings → Agents → Activity → Stats (§8.2).
//!
//! Pure computation over [`StatsPr`] facts (fetched from GitHub on demand)
//! and [`SlackRequestFact`] rows (the app's Slack review requests). Nothing
//! here touches the network or the store, so the same code can later run
//! per profile with a different input set.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::sync::{TrackedScope, scoped_queries};

#[cfg(test)]
mod tests;

/// Periods the Stats view offers, in days.
pub const PERIODS: [u32; 3] = [7, 30, 90];

/// Base search for the user's own pull requests, any state.
const STATS_QUERY: &str = "is:pr author:@me archived:false sort:created-desc";

/// The search strings for pull requests the viewer opened since `since`,
/// scoped to the tracked orgs (and personal repositories when on).
pub fn stats_queries(scope: &TrackedScope, since: DateTime<Utc>) -> Vec<String> {
    let base = format!("{STATS_QUERY} created:>={}", since.format("%Y-%m-%d"));
    scoped_queries(&base, scope)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsPrState {
    Open,
    Merged,
    Closed,
}

/// Who a review was requested from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum RequestTarget {
    /// A GitHub user login.
    User(String),
    /// A GitHub team, as `org/slug`.
    Team(String),
    /// A Slack channel (or DM) the app posted a review request in.
    SlackChannel(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsRequest {
    pub target: RequestTarget,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsReview {
    pub author: String,
    pub is_bot: bool,
    pub at: DateTime<Utc>,
}

/// One pull request the viewer opened, with its review timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsPr {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub author: String,
    pub created_at: DateTime<Utc>,
    pub is_draft: bool,
    pub state: StatsPrState,
    pub merged_at: Option<DateTime<Utc>>,
    /// Every `ReadyForReviewEvent`, any order.
    pub ready_events: Vec<DateTime<Utc>>,
    /// GitHub `ReviewRequestedEvent`s (users and teams), any order.
    pub requests: Vec<StatsRequest>,
    /// Submitted reviews, any order.
    pub reviews: Vec<StatsReview>,
}

/// What [`crate::providers::VcsProvider::review_timelines`] read. `partial`
/// when it stopped early (rate budget or size cap).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsFetch {
    pub prs: Vec<StatsPr>,
    pub partial: bool,
}

/// One Slack review request the app tracks (`review_requests`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackRequestFact {
    pub repo: String,
    pub number: u64,
    pub channel: String,
    pub requested_at: DateTime<Utc>,
    pub bump_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsFilter {
    /// `owner/name`; `None` for every repository.
    pub repo: Option<String>,
    /// An org (repository owner); `None` for every org.
    pub org: Option<String>,
}

impl StatsFilter {
    fn keeps(&self, repo: &str) -> bool {
        let owner = repo.split('/').next().unwrap_or_default();
        self.repo
            .as_deref()
            .is_none_or(|want| want.eq_ignore_ascii_case(repo))
            && self
                .org
                .as_deref()
                .is_none_or(|want| want.eq_ignore_ascii_case(owner))
    }
}

/// Median and mean of a set of durations, in seconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DurationSummary {
    pub count: u32,
    pub median_secs: Option<i64>,
    pub average_secs: Option<i64>,
}

impl DurationSummary {
    fn of(mut values: Vec<i64>) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        values.sort_unstable();
        let n = values.len();
        let median = if n % 2 == 1 {
            values[n / 2]
        } else {
            (values[n / 2 - 1] + values[n / 2]) / 2
        };
        let sum: i128 = values.iter().map(|v| i128::from(*v)).sum();
        Self {
            count: n as u32,
            median_secs: Some(median),
            average_secs: Some((sum / n as i128) as i64),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewerStat {
    pub target: RequestTarget,
    /// Pull requests this reviewer was asked on.
    pub requests: u32,
    /// Of those, how many got a review after the request.
    pub responded: u32,
    /// `responded / requests`, 0..=1.
    pub response_rate: f64,
    pub median_response_secs: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewStats {
    pub period_days: u32,
    pub since: DateTime<Utc>,
    pub opened: u32,
    pub merged: u32,
    /// Open, ready for review, and nobody else reviewed yet.
    pub waiting: u32,
    /// Slack reminders (bumps) on those pull requests.
    pub reminders: u32,
    /// Ready for review (or creation) → first review by someone else.
    pub first_review: DurationSummary,
    /// First Slack request → first review after it.
    pub slack_to_first_review: DurationSummary,
    pub reviewers: Vec<ReviewerStat>,
    /// Repositories present in the period, for the filter menu.
    pub repos: Vec<String>,
}

/// Bot accounts never count as reviewers.
pub fn is_bot_login(login: &str) -> bool {
    let lower = login.to_ascii_lowercase();
    lower.ends_with("[bot]") || lower.ends_with("-bot") || lower == "copilot"
}

fn counts(review: &StatsReview, author: &str) -> bool {
    !review.is_bot && !is_bot_login(&review.author) && !review.author.eq_ignore_ascii_case(author)
}

/// When the pull request asked for review: the first ready-for-review event,
/// else creation if it was never a draft. `None` while still a draft.
pub fn review_start(pr: &StatsPr) -> Option<DateTime<Utc>> {
    if let Some(first) = pr.ready_events.iter().min() {
        return Some(*first);
    }
    (!pr.is_draft).then_some(pr.created_at)
}

fn first_review_after(pr: &StatsPr, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    pr.reviews
        .iter()
        .filter(|review| counts(review, &pr.author) && review.at >= after)
        .map(|review| review.at)
        .min()
}

fn response_after(
    pr: &StatsPr,
    target: &RequestTarget,
    at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match target {
        RequestTarget::User(login) => pr
            .reviews
            .iter()
            .filter(|review| review.at >= at && review.author.eq_ignore_ascii_case(login))
            .map(|review| review.at)
            .min(),
        // Team members are not known here: any other person's review answers.
        RequestTarget::Team(_) | RequestTarget::SlackChannel(_) => first_review_after(pr, at),
    }
}

fn excluded_target(target: &RequestTarget, author: &str) -> bool {
    match target {
        RequestTarget::User(login) => is_bot_login(login) || login.eq_ignore_ascii_case(author),
        RequestTarget::Team(_) | RequestTarget::SlackChannel(_) => false,
    }
}

/// Statistics over `prs` the viewer opened in the `period_days` before `now`.
pub fn compute(
    prs: &[StatsPr],
    slack: &[SlackRequestFact],
    viewer: &str,
    period_days: u32,
    now: DateTime<Utc>,
    filter: &StatsFilter,
) -> ReviewStats {
    let since = now - Duration::days(i64::from(period_days));
    let mut repos: Vec<String> = Vec::new();
    let mut opened = 0;
    let mut merged = 0;
    let mut waiting = 0;
    let mut reminders = 0;
    let mut first = Vec::new();
    let mut from_slack = Vec::new();
    // target → (requests, response times in seconds)
    let mut per_target: BTreeMap<RequestTarget, (u32, Vec<i64>)> = BTreeMap::new();

    for pr in prs {
        if !pr.author.eq_ignore_ascii_case(viewer) || pr.created_at < since || pr.created_at > now {
            continue;
        }
        if !repos
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&pr.repo))
        {
            repos.push(pr.repo.clone());
        }
        if !filter.keeps(&pr.repo) {
            continue;
        }
        opened += 1;
        if pr.state == StatsPrState::Merged {
            merged += 1;
        }
        let start = review_start(pr);
        let reviewed = start.and_then(|start| {
            // A review left while still a draft answers at once.
            pr.reviews
                .iter()
                .filter(|review| counts(review, &pr.author))
                .map(|review| review.at)
                .min()
                .map(|at| (at - start).num_seconds().max(0))
        });
        match (start, reviewed) {
            (Some(_), Some(secs)) => first.push(secs),
            (Some(_), None) if pr.state == StatsPrState::Open => waiting += 1,
            _ => {}
        }

        let pr_slack: Vec<&SlackRequestFact> = slack
            .iter()
            .filter(|fact| fact.number == pr.number && fact.repo.eq_ignore_ascii_case(&pr.repo))
            .collect();
        reminders += pr_slack.iter().map(|fact| fact.bump_count).sum::<u32>();
        if let Some(asked) = pr_slack.iter().map(|fact| fact.requested_at).min()
            && let Some(answer) = first_review_after(pr, asked)
        {
            from_slack.push((answer - asked).num_seconds());
        }

        // One request per (PR, target): the earliest ask counts.
        let mut asks: BTreeMap<RequestTarget, DateTime<Utc>> = BTreeMap::new();
        let slack_asks = pr_slack.iter().map(|fact| {
            (
                RequestTarget::SlackChannel(fact.channel.clone()),
                fact.requested_at,
            )
        });
        let github_asks = pr.requests.iter().map(|req| (req.target.clone(), req.at));
        for (target, at) in github_asks.chain(slack_asks) {
            if excluded_target(&target, &pr.author) {
                continue;
            }
            asks.entry(target)
                .and_modify(|known| *known = (*known).min(at))
                .or_insert(at);
        }
        for (target, at) in asks {
            let entry = per_target.entry(target.clone()).or_default();
            entry.0 += 1;
            if let Some(answer) = response_after(pr, &target, at) {
                entry.1.push((answer - at).num_seconds());
            }
        }
    }

    let mut reviewers: Vec<ReviewerStat> = per_target
        .into_iter()
        .map(|(target, (requests, times))| {
            let responded = times.len() as u32;
            ReviewerStat {
                target,
                requests,
                responded,
                response_rate: f64::from(responded) / f64::from(requests.max(1)),
                median_response_secs: DurationSummary::of(times).median_secs,
            }
        })
        .collect();
    reviewers.sort_by(|a, b| {
        b.requests
            .cmp(&a.requests)
            .then(b.responded.cmp(&a.responded))
            .then_with(|| a.target.cmp(&b.target))
    });
    repos.sort_by_key(|repo| repo.to_ascii_lowercase());

    ReviewStats {
        period_days,
        since,
        opened,
        merged,
        waiting,
        reminders,
        first_review: DurationSummary::of(first),
        slack_to_first_review: DurationSummary::of(from_slack),
        reviewers,
        repos,
    }
}
