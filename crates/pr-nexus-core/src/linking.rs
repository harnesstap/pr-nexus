//! Project auto-link rules (§8.1).
//!
//! Rules are compiled once per evaluation with the `regex` crate (linear time,
//! safe on untrusted titles). Evaluation is pure: [`plan`] turns pull requests
//! and rules into the wanted `rule` links, and [`relink`] hands that plan to the
//! store, which leaves `manual` and `excluded` rows alone.

use regex::{Regex, RegexBuilder};

use crate::domain::{PrId, PrState, ProjectId, ProjectRule, PullRequest, RuleKind};
use crate::error::CoreError;
use crate::settings;
use crate::store::Store;

/// Default `projects.ticket_key_regex`.
pub const DEFAULT_TICKET_KEY_REGEX: &str = r"([A-Z][A-Z0-9]+-\d+)";

/// Longest accepted pattern; keeps compiled regexes small.
const MAX_PATTERN_LEN: usize = 256;

enum Matcher {
    TicketKey(Regex),
    Branch(Regex),
    Label(String),
    Repo(Regex),
    Title(Regex),
}

/// One rule, compiled.
pub struct CompiledRule {
    pub rule: ProjectRule,
    matcher: Matcher,
}

/// Compiled rules plus the ticket-key extractor, ordered by priority (desc), then id.
pub struct RuleSet {
    rules: Vec<CompiledRule>,
    ticket: Regex,
}

/// A wanted `source='rule'` link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleLink {
    pub pr: PrId,
    pub project: ProjectId,
    pub rule_id: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelinkReport {
    pub added: usize,
    pub removed: usize,
    pub skipped_rules: usize,
}

/// Checks a pattern for `kind` and returns it trimmed.
pub fn validate(kind: RuleKind, pattern: &str) -> Result<String, CoreError> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(CoreError::Validation("Pattern cannot be empty".into()));
    }
    if pattern.len() > MAX_PATTERN_LEN {
        return Err(CoreError::Validation(format!(
            "Pattern is longer than {MAX_PATTERN_LEN} characters"
        )));
    }
    compile_matcher(kind, pattern)?;
    Ok(pattern.to_owned())
}

fn compile_matcher(kind: RuleKind, pattern: &str) -> Result<Matcher, CoreError> {
    Ok(match kind {
        RuleKind::TicketKeyInBranch => Matcher::TicketKey(glob(pattern)?),
        RuleKind::BranchRegex => Matcher::Branch(user_regex(pattern)?),
        RuleKind::Label => Matcher::Label(pattern.to_owned()),
        RuleKind::Repo => Matcher::Repo(glob(pattern)?),
        RuleKind::TitleRegex => Matcher::Title(user_regex(pattern)?),
    })
}

fn user_regex(pattern: &str) -> Result<Regex, CoreError> {
    RegexBuilder::new(pattern)
        .size_limit(1 << 20)
        .build()
        .map_err(|err| CoreError::Validation(format!("Invalid regex: {err}")))
}

/// `*` matches any run of characters; everything else is literal. Case-insensitive, anchored.
fn glob(pattern: &str) -> Result<Regex, CoreError> {
    let body = pattern
        .split('*')
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(".*");
    RegexBuilder::new(&format!("^{body}$"))
        .case_insensitive(true)
        .build()
        .map_err(|err| CoreError::Validation(format!("Invalid pattern: {err}")))
}

fn ticket_regex(pattern: &str) -> Result<Regex, CoreError> {
    // Branch names are often lower-case (`feat/proj-123-retry`), so keys are
    // extracted case-insensitively and then compared case-insensitively (§8.1).
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .map_err(|err| CoreError::Validation(format!("Invalid projects.ticket_key_regex: {err}")))
}

impl CompiledRule {
    pub fn new(rule: ProjectRule) -> Result<Self, CoreError> {
        let matcher = compile_matcher(rule.kind, rule.pattern.trim())?;
        Ok(Self { rule, matcher })
    }

    pub fn matches(&self, pr: &PullRequest, ticket: &Regex) -> bool {
        match &self.matcher {
            Matcher::TicketKey(pattern) => ticket_keys(ticket, pr)
                .iter()
                .any(|key| pattern.is_match(key)),
            Matcher::Branch(regex) => regex.is_match(&pr.head_ref),
            Matcher::Label(label) => pr.labels.iter().any(|value| value == label),
            Matcher::Repo(pattern) => {
                pattern.is_match(&format!("{}/{}", pr.repo.owner, pr.repo.name))
            }
            Matcher::Title(regex) => regex.is_match(&pr.title),
        }
    }
}

fn ticket_keys(ticket: &Regex, pr: &PullRequest) -> Vec<String> {
    [pr.head_ref.as_str(), pr.title.as_str()]
        .into_iter()
        .flat_map(|text| {
            ticket.captures_iter(text).filter_map(|caps| {
                caps.get(1)
                    .or_else(|| caps.get(0))
                    .map(|found| found.as_str().to_owned())
            })
        })
        .collect()
}

impl RuleSet {
    /// Compiles `rules`. Rules that no longer compile are skipped and counted.
    pub fn new(
        rules: Vec<ProjectRule>,
        ticket_key_regex: &str,
    ) -> Result<(Self, usize), CoreError> {
        let ticket = ticket_regex(ticket_key_regex)?;
        let mut skipped = 0;
        let mut compiled: Vec<CompiledRule> = rules
            .into_iter()
            .filter_map(|rule| match CompiledRule::new(rule) {
                Ok(rule) => Some(rule),
                Err(_) => {
                    skipped += 1;
                    None
                }
            })
            .collect();
        compiled.sort_by(|left, right| {
            right
                .rule
                .priority
                .cmp(&left.rule.priority)
                .then(left.rule.id.cmp(&right.rule.id))
        });
        Ok((
            Self {
                rules: compiled,
                ticket,
            },
            skipped,
        ))
    }

    /// The highest-priority rule of `project` that matches `pr`.
    pub fn first_match(&self, pr: &PullRequest, project: ProjectId) -> Option<&ProjectRule> {
        self.rules
            .iter()
            .filter(|rule| rule.rule.project == project)
            .find(|rule| rule.matches(pr, &self.ticket))
            .map(|rule| &rule.rule)
    }

    /// Wanted rule links: one per (PR, project), carrying the winning rule.
    pub fn plan(&self, prs: &[PullRequest]) -> Vec<RuleLink> {
        let mut out = Vec::new();
        for pr in prs {
            let mut seen: Vec<ProjectId> = Vec::new();
            for rule in &self.rules {
                if seen.contains(&rule.rule.project) || !rule.matches(pr, &self.ticket) {
                    continue;
                }
                seen.push(rule.rule.project);
                out.push(RuleLink {
                    pr: pr.id,
                    project: rule.rule.project,
                    rule_id: rule.rule.id,
                });
            }
        }
        out
    }
}

/// Open, visible pull requests that `kind`/`pattern` would match (the live preview).
pub fn preview_count(
    store: &Store,
    prs: &[PullRequest],
    kind: RuleKind,
    pattern: &str,
) -> Result<usize, CoreError> {
    let pattern = validate(kind, pattern)?;
    let ticket = ticket_regex(&settings::ticket_key_regex(store)?)?;
    let rule = CompiledRule::new(ProjectRule {
        id: 0,
        project: ProjectId(0),
        kind,
        pattern,
        priority: 0,
    })?;
    Ok(prs
        .iter()
        .filter(|pr| pr.state == PrState::Open && !pr.hidden)
        .filter(|pr| rule.matches(pr, &ticket))
        .count())
}

/// Re-evaluates every rule against `prs` and stores the result (§8.1 steps 1–3).
pub fn relink(store: &Store, prs: &[PullRequest]) -> Result<RelinkReport, CoreError> {
    let (rules, skipped) = RuleSet::new(
        store.list_all_project_rules()?,
        &settings::ticket_key_regex(store)?,
    )?;
    let wanted = rules.plan(prs);
    let scope = prs.iter().map(|pr| pr.id).collect();
    let (added, removed) = store.apply_rule_links(scope, wanted)?;
    Ok(RelinkReport {
        added,
        removed,
        skipped_rules: skipped,
    })
}

/// Whether any rule of `project` matches `pr` right now.
pub fn rule_matches(
    store: &Store,
    pr: &PullRequest,
    project: ProjectId,
) -> Result<bool, CoreError> {
    let (rules, _) = RuleSet::new(
        store.list_project_rules(project)?,
        &settings::ticket_key_regex(store)?,
    )?;
    Ok(rules.first_match(pr, project).is_some())
}

#[cfg(test)]
mod tests;
