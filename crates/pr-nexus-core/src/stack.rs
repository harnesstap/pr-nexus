//! Stacked pull requests, derived from head and base refs already on each row.
//!
//! A pull request is stacked on another when its base branch is the head
//! branch of an open pull request in the same repository. Merged parents are
//! not links: the child's base is treated as retargeted onto that parent's
//! base, which is what GitHub does after the parent merges. The result is a
//! forest. A cycle drops the parent link of the lowest-numbered pull request
//! in the cycle so that one becomes the bottom and the walk stops.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

/// Open, closed, or merged. Only open pull requests are stack members.
/// Merged ones are walked when a base still names their head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackState {
    Open,
    Closed,
    Merged,
}

/// The fields the graph needs. Callers keep titles and status beside this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackRef {
    pub id: i64,
    /// `owner/name`. Branch names only match inside one repository.
    pub repo: String,
    pub number: u64,
    pub state: StackState,
    pub head_ref: String,
    pub base_ref: String,
}

/// Where one open pull request sits in its stack. Absent when it is alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackPlace {
    /// 1-based index in merge order (bottom is 1).
    pub position: u32,
    pub size: u32,
    pub root_id: i64,
    pub root_number: u64,
    pub parent_id: Option<i64>,
    pub parent_number: Option<u64>,
    /// Head branch of the parent. The child merges into this until the parent lands.
    pub parent_head: Option<String>,
    /// 0 at the bottom of the stack.
    pub depth: u32,
    /// The lowest unmerged member: the bottom, which has to merge before the rest.
    pub blocking: bool,
}

/// One stack, members from bottom to top in merge order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackChain {
    pub root_id: i64,
    pub blocking_id: i64,
    pub member_ids: Vec<i64>,
}

/// Places for every member of a stack of two or more, plus those chains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackIndex {
    places: HashMap<i64, StackPlace>,
    chains: Vec<StackChain>,
}

impl StackIndex {
    pub fn place(&self, id: i64) -> Option<&StackPlace> {
        self.places.get(&id)
    }

    pub fn places(&self) -> &HashMap<i64, StackPlace> {
        &self.places
    }

    pub fn chains(&self) -> &[StackChain] {
        &self.chains
    }

    /// The chain that contains `id`, if it is in a stack of two or more.
    pub fn chain_of(&self, id: i64) -> Option<&StackChain> {
        let root = self.places.get(&id)?.root_id;
        self.chains.iter().find(|chain| chain.root_id == root)
    }
}

/// Build the forest. Pull requests that are not in a stack of two or more
/// have no [`StackPlace`].
pub fn index(prs: &[StackRef]) -> StackIndex {
    let by_id: HashMap<i64, &StackRef> = prs.iter().map(|pr| (pr.id, pr)).collect();
    let mut by_repo: HashMap<&str, Vec<&StackRef>> = HashMap::new();
    for pr in prs {
        by_repo.entry(pr.repo.as_str()).or_default().push(pr);
    }

    let mut parent_of: HashMap<i64, i64> = HashMap::new();
    for group in by_repo.values() {
        let open_by_head = by_head(group, StackState::Open);
        let merged_by_head = by_head(group, StackState::Merged);
        for pr in group {
            if pr.state != StackState::Open {
                continue;
            }
            let base = effective_base(pr, &open_by_head, &merged_by_head);
            let Some(parent) = open_by_head.get(base.as_str()) else {
                continue;
            };
            if parent.id == pr.id {
                continue;
            }
            parent_of.insert(pr.id, parent.id);
        }
    }
    break_cycles(&mut parent_of, &by_id);

    let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut has_parent: HashSet<i64> = HashSet::new();
    for (child, parent) in &parent_of {
        children.entry(*parent).or_default().push(*child);
        has_parent.insert(*child);
    }
    for kids in children.values_mut() {
        kids.sort_by(|&left, &right| cmp_pr(left, right, &by_id));
    }

    let mut roots: Vec<i64> = by_id
        .values()
        .filter(|pr| pr.state == StackState::Open && !has_parent.contains(&pr.id))
        .map(|pr| pr.id)
        .collect();
    roots.sort_by(|&left, &right| cmp_pr(left, right, &by_id));

    let mut places = HashMap::new();
    let mut chains = Vec::new();
    for root in roots {
        let mut ordered: Vec<(i64, u32, Option<i64>)> = Vec::new();
        walk(root, 0, None, &children, &mut ordered);
        if ordered.len() < 2 {
            continue;
        }
        let size = u32::try_from(ordered.len()).unwrap_or(u32::MAX);
        let root_number = by_id.get(&root).map(|pr| pr.number).unwrap_or(0);
        let mut member_ids = Vec::with_capacity(ordered.len());
        for (index, (id, depth, parent_id)) in ordered.iter().enumerate() {
            let parent = parent_id.and_then(|parent| by_id.get(&parent).copied());
            let position = u32::try_from(index + 1).unwrap_or(u32::MAX);
            places.insert(
                *id,
                StackPlace {
                    position,
                    size,
                    root_id: root,
                    root_number,
                    parent_id: parent.map(|pr| pr.id),
                    parent_number: parent.map(|pr| pr.number),
                    parent_head: parent.map(|pr| pr.head_ref.clone()),
                    depth: *depth,
                    blocking: *id == root,
                },
            );
            member_ids.push(*id);
        }
        chains.push(StackChain {
            root_id: root,
            blocking_id: root,
            member_ids,
        });
    }

    StackIndex { places, chains }
}

/// The open pull request whose head is this base, after walking merged parents.
fn effective_base(
    pr: &StackRef,
    open_by_head: &HashMap<&str, &StackRef>,
    merged_by_head: &HashMap<&str, &StackRef>,
) -> String {
    let mut base = pr.base_ref.clone();
    let mut seen = HashSet::new();
    loop {
        if base.is_empty() || !seen.insert(base.clone()) {
            break;
        }
        if open_by_head.contains_key(base.as_str()) {
            break;
        }
        let Some(via) = merged_by_head.get(base.as_str()) else {
            break;
        };
        if via.id == pr.id {
            break;
        }
        base = via.base_ref.clone();
    }
    base
}

/// One head can match several pull requests. The lowest number wins, then the
/// lowest id, so the parent does not change between runs.
fn by_head<'a>(group: &[&'a StackRef], state: StackState) -> HashMap<&'a str, &'a StackRef> {
    let mut out: HashMap<&str, &'a StackRef> = HashMap::new();
    for pr in group {
        if pr.state != state || pr.head_ref.is_empty() {
            continue;
        }
        let keep = out
            .get(pr.head_ref.as_str())
            .is_some_and(|current| (current.number, current.id) <= (pr.number, pr.id));
        if !keep {
            out.insert(pr.head_ref.as_str(), *pr);
        }
    }
    out
}

/// Drop parent links until nothing loops. Each cycle loses the link of its
/// lowest-numbered pull request, which becomes the bottom of what remains.
fn break_cycles(parent_of: &mut HashMap<i64, i64>, by_id: &HashMap<i64, &StackRef>) {
    let limit = parent_of.len().saturating_add(1);
    for _ in 0..limit {
        let Some(cycle) = find_cycle(parent_of) else {
            return;
        };
        let Some(victim) = cycle
            .iter()
            .copied()
            .min_by(|&left, &right| cmp_pr(left, right, by_id))
        else {
            return;
        };
        parent_of.remove(&victim);
    }
}

fn find_cycle(parent_of: &HashMap<i64, i64>) -> Option<Vec<i64>> {
    let mut starts: Vec<i64> = parent_of.keys().copied().collect();
    starts.sort_unstable();
    for start in starts {
        let mut path: Vec<i64> = Vec::new();
        let mut at: HashMap<i64, usize> = HashMap::new();
        let mut cursor = Some(start);
        while let Some(id) = cursor {
            if let Some(index) = at.get(&id).copied() {
                return Some(path[index..].to_vec());
            }
            at.insert(id, path.len());
            path.push(id);
            cursor = parent_of.get(&id).copied();
        }
    }
    None
}

fn walk(
    id: i64,
    depth: u32,
    parent: Option<i64>,
    children: &HashMap<i64, Vec<i64>>,
    out: &mut Vec<(i64, u32, Option<i64>)>,
) {
    out.push((id, depth, parent));
    let Some(kids) = children.get(&id) else {
        return;
    };
    for child in kids {
        let next_depth = depth.saturating_add(1);
        walk(*child, next_depth, Some(id), children, out);
    }
}

fn cmp_pr(left: i64, right: i64, by_id: &HashMap<i64, &StackRef>) -> std::cmp::Ordering {
    let ln = by_id.get(&left).map(|pr| pr.number).unwrap_or(0);
    let rn = by_id.get(&right).map(|pr| pr.number).unwrap_or(0);
    ln.cmp(&rn).then(left.cmp(&right))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(id: i64, number: u64, state: StackState, head: &str, base: &str) -> StackRef {
        StackRef {
            id,
            repo: "acme/widgets".into(),
            number,
            state,
            head_ref: head.into(),
            base_ref: base.into(),
        }
    }

    fn open(id: i64, number: u64, head: &str, base: &str) -> StackRef {
        pr(id, number, StackState::Open, head, base)
    }

    fn ids(index: &StackIndex, id: i64) -> Option<Vec<i64>> {
        index.chain_of(id).map(|chain| chain.member_ids.clone())
    }

    #[test]
    fn a_linear_stack_is_bottom_to_top_and_the_bottom_blocks() {
        let index = index(&[
            open(1, 10, "feat-a", "main"),
            open(2, 11, "feat-b", "feat-a"),
            open(3, 12, "feat-c", "feat-b"),
            open(4, 13, "feat-d", "feat-c"),
        ]);
        assert_eq!(ids(&index, 3), Some(vec![1, 2, 3, 4]));
        let second = index.place(2).unwrap();
        assert_eq!(second.position, 2);
        assert_eq!(second.size, 4);
        assert_eq!(second.depth, 1);
        assert_eq!(second.parent_id, Some(1));
        assert_eq!(second.parent_number, Some(10));
        assert_eq!(second.parent_head.as_deref(), Some("feat-a"));
        assert!(!second.blocking);
        assert!(index.place(1).unwrap().blocking);
        assert_eq!(index.place(4).unwrap().position, 4);
        assert!(index.place(99).is_none());
    }

    #[test]
    fn a_branch_lists_each_child_after_its_parent_in_number_order() {
        // 1
        // ├─ 2
        // └─ 3
        //    └─ 4
        let index = index(&[
            open(1, 10, "feat-a", "main"),
            open(3, 12, "feat-c", "feat-a"),
            open(2, 11, "feat-b", "feat-a"),
            open(4, 13, "feat-d", "feat-c"),
        ]);
        assert_eq!(ids(&index, 4), Some(vec![1, 2, 3, 4]));
        assert_eq!(index.place(2).unwrap().depth, 1);
        assert_eq!(index.place(3).unwrap().depth, 1);
        assert_eq!(index.place(4).unwrap().depth, 2);
        assert_eq!(index.place(4).unwrap().position, 4);
        assert_eq!(index.chain_of(2).unwrap().blocking_id, 1);
    }

    #[test]
    fn a_cycle_stops_and_the_lowest_number_is_the_bottom() {
        let index = index(&[
            open(1, 20, "feat-a", "feat-b"),
            open(2, 10, "feat-b", "feat-a"),
        ]);
        assert_eq!(ids(&index, 1), Some(vec![2, 1]));
        assert!(index.place(2).unwrap().blocking);
        assert_eq!(index.place(1).unwrap().parent_id, Some(2));
        assert!(index.place(2).unwrap().parent_id.is_none());
    }

    #[test]
    fn a_three_pull_request_cycle_also_stops() {
        let index = index(&[
            open(1, 30, "feat-a", "feat-c"),
            open(2, 10, "feat-b", "feat-a"),
            open(3, 20, "feat-c", "feat-b"),
        ]);
        // 2 is the lowest number, so its link back to 3 is dropped.
        assert_eq!(ids(&index, 1), Some(vec![2, 3, 1]));
        assert!(index.place(2).unwrap().blocking);
        assert!(index.place(2).unwrap().parent_id.is_none());
    }

    #[test]
    fn a_merged_parent_retargets_the_base_onto_the_next_open_pull_request() {
        // B merged between A and C. C's stored base is still B's head.
        // GitHub retargets that base onto B's base, which is A's head.
        let index = index(&[
            open(1, 10, "feat-a", "main"),
            pr(2, 11, StackState::Merged, "feat-b", "feat-a"),
            open(3, 12, "feat-c", "feat-b"),
        ]);
        assert_eq!(ids(&index, 3), Some(vec![1, 3]));
        assert!(index.place(2).is_none());
        assert_eq!(index.place(3).unwrap().parent_id, Some(1));
        assert_eq!(
            index.place(3).unwrap().parent_head.as_deref(),
            Some("feat-a")
        );
        assert_eq!(index.place(3).unwrap().position, 2);
        assert!(index.place(1).unwrap().blocking);
    }

    #[test]
    fn a_merged_parent_whose_child_already_points_at_main_is_not_a_stack() {
        let index = index(&[
            pr(1, 10, StackState::Merged, "feat-a", "main"),
            open(2, 11, "feat-b", "main"),
        ]);
        assert!(index.place(1).is_none());
        assert!(index.place(2).is_none());
        assert!(index.chains().is_empty());
    }

    #[test]
    fn a_merged_parent_with_no_further_open_base_leaves_the_child_unstacked() {
        // The stored base still names the merged head. After retarget it is main.
        let index = index(&[
            pr(1, 10, StackState::Merged, "feat-a", "main"),
            open(2, 11, "feat-b", "feat-a"),
        ]);
        assert!(index.place(2).is_none());
    }

    #[test]
    fn the_same_branch_names_in_another_repository_do_not_link() {
        let mut other = open(2, 11, "feat-b", "feat-a");
        other.repo = "acme-labs/widgets".into();
        let index = index(&[open(1, 10, "feat-a", "main"), other]);
        assert!(index.chains().is_empty());
    }

    #[test]
    fn a_closed_pull_request_is_not_a_parent_and_is_not_retargeted_through() {
        let index = index(&[
            pr(1, 10, StackState::Closed, "feat-a", "main"),
            open(2, 11, "feat-b", "feat-a"),
        ]);
        assert!(index.place(2).is_none());
    }

    #[test]
    fn two_open_pull_requests_with_one_head_pick_the_lower_number_as_parent() {
        let index = index(&[
            open(1, 20, "feat-a", "main"),
            open(2, 10, "feat-a", "main"),
            open(3, 30, "feat-b", "feat-a"),
        ]);
        assert_eq!(index.place(3).unwrap().parent_id, Some(2));
        assert_eq!(ids(&index, 3), Some(vec![2, 3]));
        assert!(index.place(1).is_none());
    }

    #[test]
    fn a_pull_request_on_main_alone_is_not_a_stack() {
        let index = index(&[open(1, 10, "feat-a", "main")]);
        assert!(index.place(1).is_none());
    }

    #[test]
    fn a_retarget_walk_stops_when_merged_heads_loop() {
        let index = index(&[
            pr(1, 10, StackState::Merged, "feat-a", "feat-b"),
            pr(2, 11, StackState::Merged, "feat-b", "feat-a"),
            open(3, 12, "feat-c", "feat-a"),
        ]);
        assert!(index.place(3).is_none());
    }
}
