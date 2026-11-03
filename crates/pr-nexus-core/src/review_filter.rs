//! To review filters that GitHub search can answer (§6, §8.2).
//!
//! The list filters run locally over stored rows. Review requests are only
//! loaded in a window, so a filter over the window alone misses older
//! matches. The part of a filter that has a GitHub search qualifier is sent
//! with the review query instead; the rest (projects, Slack, starred, multi
//! value choices, readiness) keeps running locally on what comes back.
//!
//! Every qualifier here must return a superset of what the local filter
//! keeps, so the local pass never loses a row GitHub would have matched.

use serde::{Deserialize, Serialize};

use crate::sync::TrackedScope;

/// `review:` values with an exact local counterpart. "No review" is left
/// out: GitHub's `review:none` drops PRs with comment-only reviews, which
/// the local filter keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewQualifier {
    Approved,
    ChangesRequested,
    Required,
}

/// `status:` values (the head commit's combined check state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiQualifier {
    Success,
    Failure,
    Pending,
}

/// The GitHub-side part of a To review filter. Empty means "no filter".
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RemoteReviewFilter {
    /// Owners to search, a subset of the tracked scope. Empty: the whole
    /// scope.
    #[serde(default)]
    pub orgs: Vec<String>,
    #[serde(default)]
    pub draft: Option<bool>,
    #[serde(default)]
    pub review: Option<ReviewQualifier>,
    #[serde(default)]
    pub ci: Option<CiQualifier>,
}

impl RemoteReviewFilter {
    pub fn is_empty(&self) -> bool {
        self.orgs.is_empty() && self.draft.is_none() && self.review.is_none() && self.ci.is_none()
    }

    /// The qualifiers appended to the review query, in a fixed order, with
    /// a leading space per qualifier. Owners go through [`Self::narrow`].
    pub fn qualifiers(&self) -> String {
        let mut out = String::new();
        if let Some(draft) = self.draft {
            out.push_str(if draft { " draft:true" } else { " draft:false" });
        }
        if let Some(review) = self.review {
            out.push_str(match review {
                ReviewQualifier::Approved => " review:approved",
                ReviewQualifier::ChangesRequested => " review:changes_requested",
                ReviewQualifier::Required => " review:required",
            });
        }
        if let Some(ci) = self.ci {
            out.push_str(match ci {
                CiQualifier::Success => " status:success",
                CiQualifier::Failure => " status:failure",
                CiQualifier::Pending => " status:pending",
            });
        }
        out
    }

    /// The tracked scope cut down to the chosen owners. `None` when no
    /// chosen owner is in scope: there is nothing to search.
    pub fn narrow(&self, scope: &TrackedScope) -> Option<TrackedScope> {
        if self.orgs.is_empty() {
            return Some(scope.clone());
        }
        let chosen = |name: &str| self.orgs.iter().any(|org| org.eq_ignore_ascii_case(name));
        let narrowed = TrackedScope {
            orgs: scope
                .orgs
                .iter()
                .filter(|org| chosen(org))
                .cloned()
                .collect(),
            include_personal: scope.include_personal && chosen(&scope.viewer_login),
            viewer_login: scope.viewer_login.clone(),
        };
        (!narrowed.orgs.is_empty() || narrowed.include_personal).then_some(narrowed)
    }

    /// A stable identity for caching: owners folded and sorted.
    pub fn key(&self) -> String {
        let mut orgs: Vec<String> = self
            .orgs
            .iter()
            .map(|org| org.to_ascii_lowercase())
            .collect();
        orgs.sort();
        orgs.dedup();
        format!("{}|{}", orgs.join(","), self.qualifiers().trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(orgs: &[&str], personal: bool) -> TrackedScope {
        TrackedScope {
            orgs: orgs.iter().map(|org| (*org).to_owned()).collect(),
            include_personal: personal,
            viewer_login: "octo".into(),
        }
    }

    #[test]
    fn an_empty_filter_adds_nothing_and_keeps_the_scope() {
        let filter = RemoteReviewFilter::default();
        assert!(filter.is_empty());
        assert_eq!(filter.qualifiers(), "");
        assert_eq!(
            filter.narrow(&scope(&["acme"], true)),
            Some(scope(&["acme"], true))
        );
    }

    #[test]
    fn each_field_maps_to_its_qualifier_in_a_fixed_order() {
        let filter = RemoteReviewFilter {
            orgs: Vec::new(),
            draft: Some(false),
            review: Some(ReviewQualifier::Approved),
            ci: Some(CiQualifier::Success),
        };
        assert_eq!(
            filter.qualifiers(),
            " draft:false review:approved status:success"
        );
        let cases = [
            (
                ReviewQualifier::ChangesRequested,
                " review:changes_requested",
            ),
            (ReviewQualifier::Required, " review:required"),
        ];
        for (review, text) in cases {
            let filter = RemoteReviewFilter {
                review: Some(review),
                ..Default::default()
            };
            assert_eq!(filter.qualifiers(), text);
        }
        for (ci, text) in [
            (CiQualifier::Failure, " status:failure"),
            (CiQualifier::Pending, " status:pending"),
        ] {
            let filter = RemoteReviewFilter {
                ci: Some(ci),
                ..Default::default()
            };
            assert_eq!(filter.qualifiers(), text);
        }
        let draft = RemoteReviewFilter {
            draft: Some(true),
            ..Default::default()
        };
        assert_eq!(draft.qualifiers(), " draft:true");
    }

    #[test]
    fn owners_narrow_the_tracked_scope_case_insensitively() {
        let filter = RemoteReviewFilter {
            orgs: vec!["ACME-labs".into()],
            ..Default::default()
        };
        assert_eq!(
            filter.narrow(&scope(&["acme", "acme-labs"], true)),
            Some(scope(&["acme-labs"], false))
        );
        let mine = RemoteReviewFilter {
            orgs: vec!["octo".into()],
            ..Default::default()
        };
        assert_eq!(mine.narrow(&scope(&["acme"], true)), Some(scope(&[], true)));
    }

    #[test]
    fn owners_outside_the_scope_search_nothing() {
        let filter = RemoteReviewFilter {
            orgs: vec!["other".into()],
            ..Default::default()
        };
        assert_eq!(filter.narrow(&scope(&["acme"], false)), None);
        let mine = RemoteReviewFilter {
            orgs: vec!["octo".into()],
            ..Default::default()
        };
        assert_eq!(mine.narrow(&scope(&["acme"], false)), None);
    }

    #[test]
    fn the_key_ignores_owner_order_and_case() {
        let left = RemoteReviewFilter {
            orgs: vec!["acme-labs".into(), "Acme".into()],
            ci: Some(CiQualifier::Failure),
            ..Default::default()
        };
        let right = RemoteReviewFilter {
            orgs: vec!["acme".into(), "ACME-LABS".into()],
            ci: Some(CiQualifier::Failure),
            ..Default::default()
        };
        assert_eq!(left.key(), right.key());
        assert_eq!(left.key(), "acme,acme-labs|status:failure");
    }

    #[test]
    fn it_reads_the_frontend_shape() {
        let filter: RemoteReviewFilter =
            serde_json::from_str(r#"{"orgs":["acme"],"review":"changes_requested","ci":null}"#)
                .unwrap();
        assert_eq!(filter.review, Some(ReviewQualifier::ChangesRequested));
        assert_eq!(filter.draft, None);
    }
}
