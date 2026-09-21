//! Conservative analysis for compacting merged history lanes.
//!
//! Git does not retain deleted branch names, so compact mode only folds side
//! history when the loaded commit DAG proves that it has one unambiguous
//! external merge owner. Ambiguous, live-branch, octopus, and truncated cases
//! stay expanded.

use std::collections::{HashMap, HashSet};

use git2::Oid;

use super::{BranchInfo, CommitInfo};

#[derive(Debug)]
struct MergeCandidate {
    merge_oid: Oid,
    side_commits: HashSet<Oid>,
}

/// Precomputed ownership for side-history commits that can safely be rendered
/// on their merge target's lane.
#[derive(Debug, Default)]
pub(crate) struct MergeFoldPlan {
    /// side commit OID -> root merge commit OID
    pub(crate) owner_by_commit: HashMap<Oid, Oid>,
    /// Root merge commits whose side parent lane can be suppressed.
    pub(crate) foldable_merges: HashSet<Oid>,
}

impl MergeFoldPlan {
    pub(crate) fn analyze(commits: &[CommitInfo], branches: &[BranchInfo]) -> Self {
        let commits_by_oid: HashMap<Oid, &CommitInfo> =
            commits.iter().map(|commit| (commit.oid, commit)).collect();
        let branch_tips: HashSet<Oid> = branches.iter().map(|branch| branch.tip_oid).collect();

        let mut candidates = Vec::new();

        for commit in commits {
            // Octopus merges are intentionally left expanded in the first
            // implementation. They do not have one obvious side-history owner.
            if commit.parent_oids.len() != 2 {
                continue;
            }

            let first_parent = commit.parent_oids[0];
            let side_parent = commit.parent_oids[1];
            if !commits_by_oid.contains_key(&first_parent)
                || !commits_by_oid.contains_key(&side_parent)
            {
                continue;
            }

            let Some(first_ancestors) = loaded_ancestors(first_parent, &commits_by_oid) else {
                continue;
            };
            let Some(side_ancestors) = loaded_ancestors(side_parent, &commits_by_oid) else {
                continue;
            };

            let side_commits: HashSet<Oid> = side_ancestors
                .difference(&first_ancestors)
                .copied()
                .collect();

            if side_commits.is_empty() {
                continue;
            }

            // A surviving local or remote ref means this lineage still has an
            // explicit branch identity, so keep its lane visible.
            if branch_tips
                .iter()
                .any(|tip| side_commits.contains(tip))
            {
                continue;
            }

            candidates.push(MergeCandidate {
                merge_oid: commit.oid,
                side_commits,
            });
        }

        // A nested merge inside another side region is internal topology, not
        // a second external consumer. Only outermost valid regions compete for
        // ownership.
        let root_indices: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter_map(|(idx, candidate)| {
                let nested = candidates.iter().enumerate().any(|(other_idx, outer)| {
                    other_idx != idx && outer.side_commits.contains(&candidate.merge_oid)
                });
                (!nested).then_some(idx)
            })
            .collect();

        // If two external merge regions overlap, the shared history has more
        // than one plausible target. Keep both expanded rather than guessing.
        let mut ambiguous_roots = HashSet::new();
        for (pos, &left_idx) in root_indices.iter().enumerate() {
            for &right_idx in root_indices.iter().skip(pos + 1) {
                if !candidates[left_idx]
                    .side_commits
                    .is_disjoint(&candidates[right_idx].side_commits)
                {
                    ambiguous_roots.insert(left_idx);
                    ambiguous_roots.insert(right_idx);
                }
            }
        }

        let mut plan = Self::default();
        for root_idx in root_indices {
            if ambiguous_roots.contains(&root_idx) {
                continue;
            }

            let candidate = &candidates[root_idx];
            plan.foldable_merges.insert(candidate.merge_oid);
            for &oid in &candidate.side_commits {
                plan.owner_by_commit.insert(oid, candidate.merge_oid);
            }
        }

        plan
    }
}

/// Return all ancestors reachable inside the currently loaded history window.
/// If traversal reaches a parent outside the window, ownership cannot be
/// proven complete, so return None and leave that region expanded.
fn loaded_ancestors<'a>(
    start: Oid,
    commits: &HashMap<Oid, &'a CommitInfo>,
) -> Option<HashSet<Oid>> {
    let mut seen = HashSet::new();
    let mut stack = vec![start];

    while let Some(oid) = stack.pop() {
        if !seen.insert(oid) {
            continue;
        }

        let commit = commits.get(&oid)?;
        for parent in &commit.parent_oids {
            if !commits.contains_key(parent) {
                return None;
            }
            stack.push(*parent);
        }
    }

    Some(seen)
}

#[cfg(test)]
mod tests {
    use chrono::Local;

    use super::*;

    fn oid(hex: char) -> Oid {
        Oid::from_str(&hex.to_string().repeat(40)).unwrap()
    }

    fn commit(id: char, parents: &[char]) -> CommitInfo {
        let oid = oid(id);
        CommitInfo {
            oid,
            short_id: oid.to_string()[..7].to_string(),
            author_name: "Test".to_string(),
            author_email: "test@example.com".to_string(),
            timestamp: Local::now(),
            message: id.to_string(),
            full_message: id.to_string(),
            parent_oids: parents.iter().map(|parent| oid(*parent)).collect(),
        }
    }

    fn branch(name: &str, tip: char) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_head: false,
            is_remote: false,
            upstream: None,
            tip_oid: oid(tip),
        }
    }

    #[test]
    fn folds_single_owner_side_history() {
        // M(B, D), B(A), D(C), C(A)
        let commits = vec![
            commit('e', &['b', 'd']),
            commit('d', &['c']),
            commit('c', &['a']),
            commit('b', &['a']),
            commit('a', &[]),
        ];

        let plan = MergeFoldPlan::analyze(&commits, &[branch("main", 'e')]);

        assert!(plan.foldable_merges.contains(&oid('e')));
        assert_eq!(plan.owner_by_commit.get(&oid('d')), Some(&oid('e')));
        assert_eq!(plan.owner_by_commit.get(&oid('c')), Some(&oid('e')));
        assert!(!plan.owner_by_commit.contains_key(&oid('a')));
    }

    #[test]
    fn keeps_live_side_branch_expanded() {
        let commits = vec![
            commit('e', &['b', 'd']),
            commit('d', &['c']),
            commit('c', &['a']),
            commit('b', &['a']),
            commit('a', &[]),
        ];
        let branches = vec![branch("main", 'e'), branch("feature", 'd')];

        let plan = MergeFoldPlan::analyze(&commits, &branches);

        assert!(plan.foldable_merges.is_empty());
        assert!(plan.owner_by_commit.is_empty());
    }

    #[test]
    fn keeps_history_with_multiple_external_merge_targets_expanded() {
        // E(B, D) and F(G, D) both consume D/C.
        let commits = vec![
            commit('e', &['b', 'd']),
            commit('f', &['g', 'd']),
            commit('d', &['c']),
            commit('c', &['a']),
            commit('b', &['a']),
            commit('g', &['a']),
            commit('a', &[]),
        ];
        let branches = vec![branch("main", 'e'), branch("release", 'f')];

        let plan = MergeFoldPlan::analyze(&commits, &branches);

        assert!(plan.foldable_merges.is_empty());
        assert!(plan.owner_by_commit.is_empty());
    }

    #[test]
    fn treats_nested_merge_as_internal_to_single_external_region() {
        // Outer E(B, D); inside side history D(C, H).
        let commits = vec![
            commit('e', &['b', 'd']),
            commit('d', &['c', 'h']),
            commit('h', &['a']),
            commit('c', &['a']),
            commit('b', &['a']),
            commit('a', &[]),
        ];

        let plan = MergeFoldPlan::analyze(&commits, &[branch("main", 'e')]);

        assert!(plan.foldable_merges.contains(&oid('e')));
        assert!(!plan.foldable_merges.contains(&oid('d')));
        for id in ['d', 'c', 'h'] {
            assert_eq!(plan.owner_by_commit.get(&oid(id)), Some(&oid('e')));
        }
    }

    #[test]
    fn truncated_history_is_not_folded() {
        // A is intentionally missing from the loaded window.
        let commits = vec![
            commit('e', &['b', 'd']),
            commit('d', &['c']),
            commit('c', &['a']),
            commit('b', &['a']),
        ];

        let plan = MergeFoldPlan::analyze(&commits, &[branch("main", 'e')]);

        assert!(plan.foldable_merges.is_empty());
    }
}
