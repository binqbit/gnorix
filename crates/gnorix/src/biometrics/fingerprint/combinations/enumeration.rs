//! Bounded distinct subsets with deterministic priority-weighted incidence.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::{CombinationError, WorkBudget};

#[derive(Debug, PartialEq)]
pub(super) struct RankedSubset {
    pub ranks: Vec<usize>,
    pub score: f64,
}

/// Priorities are validated and sorted descending by the caller. Rank order
/// also supplies the canonical tie break between equally weighted points.
pub(super) fn enumerate(
    priorities: &[f64],
    minimum: usize,
    budget: usize,
    work: &mut WorkBudget,
) -> Result<Vec<RankedSubset>, CombinationError> {
    validate_limits(priorities.len(), minimum, budget)?;
    let mut selected = coverage_subsets(priorities, minimum, work)?;

    if selected.len() < budget {
        fill_by_score(&mut selected, priorities, minimum, budget, work)?;
    }

    selected.sort_unstable_by(compare_subsets);
    Ok(selected)
}

fn validate_limits(
    pool_size: usize,
    minimum: usize,
    budget: usize,
) -> Result<(), CombinationError> {
    if minimum == 0 || minimum > pool_size {
        return Err(CombinationError::InvalidConfig(
            "minimum subset size must fit the candidate pool",
        ));
    }
    if budget < pool_size - minimum + 1 {
        return Err(CombinationError::InvalidConfig(
            "subset budget must cover every pool point",
        ));
    }
    Ok(())
}

fn coverage_subsets(
    priorities: &[f64],
    minimum: usize,
    work: &mut WorkBudget,
) -> Result<Vec<RankedSubset>, CombinationError> {
    let mut selected = Vec::new();
    for last in minimum - 1..priorities.len() {
        work.spend(minimum)?;
        let ranks = (0..minimum - 1).chain(std::iter::once(last)).collect();
        selected.push(ranked_subset(priorities, ranks));
    }
    Ok(selected)
}

fn fill_by_score(
    selected: &mut Vec<RankedSubset>,
    priorities: &[f64],
    minimum: usize,
    budget: usize,
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    let mut pending = search_roots(priorities, minimum, work)?;
    while let Some(current) = pending.pop() {
        let already_selected = is_coverage_seed(&current.subset.ranks, minimum);
        if !already_selected && selected.len() + 1 == budget {
            selected.push(current.subset);
            break;
        }

        extend_frontier(&mut pending, &current, priorities, work)?;
        if !already_selected {
            selected.push(current.subset);
        }
    }
    Ok(())
}

fn search_roots(
    priorities: &[f64],
    minimum: usize,
    work: &mut WorkBudget,
) -> Result<BinaryHeap<Pending>, CombinationError> {
    let mut pending = BinaryHeap::new();
    for size in minimum..=priorities.len() {
        work.spend(size)?;
        pending.push(Pending {
            subset: ranked_subset(priorities, (0..size).collect()),
            first_nonminimal: size,
        });
    }
    Ok(pending)
}

/// Decrementing a tuple's first nonminimal index gives its unique parent.
/// Therefore each tuple has at most these two children; no visited set is
/// needed, and the frontier grows with the output budget rather than 2^n.
fn extend_frontier(
    pending: &mut BinaryHeap<Pending>,
    current: &Pending,
    priorities: &[f64],
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    let size = current.subset.ranks.len();
    let first = current.first_nonminimal;
    if first == size {
        push_child(pending, current, size - 1, priorities, work)?;
    } else {
        push_child(pending, current, first, priorities, work)?;
        if first > 0 {
            push_child(pending, current, first - 1, priorities, work)?;
        }
    }
    Ok(())
}

fn push_child(
    pending: &mut BinaryHeap<Pending>,
    parent: &Pending,
    position: usize,
    priorities: &[f64],
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    let ranks = &parent.subset.ranks;
    let limit = ranks.get(position + 1).copied().unwrap_or(priorities.len());
    if ranks[position] + 1 >= limit {
        return Ok(());
    }

    work.spend(ranks.len())?;
    let mut child = ranks.clone();
    child[position] += 1;
    pending.push(Pending {
        subset: ranked_subset(priorities, child),
        first_nonminimal: position,
    });
    Ok(())
}

fn is_coverage_seed(ranks: &[usize], minimum: usize) -> bool {
    ranks.len() == minimum && ranks[..minimum - 1].iter().copied().eq(0..minimum - 1)
}

fn ranked_subset(priorities: &[f64], ranks: Vec<usize>) -> RankedSubset {
    let squared_sum: f64 = ranks
        .iter()
        .map(|&rank| priorities[rank] * priorities[rank])
        .sum();
    // Pull the common affine terms outside the average. In particular, a
    // zero-priority pool has the exact same score at every subset size.
    let score = 0.1 + 0.9 * (squared_sum / ranks.len() as f64);
    RankedSubset { ranks, score }
}

fn compare_subsets(first: &RankedSubset, second: &RankedSubset) -> Ordering {
    second
        .score
        .total_cmp(&first.score)
        .then_with(|| first.ranks.cmp(&second.ranks))
}

struct Pending {
    subset: RankedSubset,
    first_nonminimal: usize,
}

impl PartialEq for Pending {
    fn eq(&self, other: &Self) -> bool {
        compare_subsets(&self.subset, &other.subset) == Ordering::Equal
    }
}

impl Eq for Pending {}

impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_subsets(&self.subset, &other.subset).reverse()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn reference(priorities: &[f64], minimum: usize, budget: usize) -> Vec<RankedSubset> {
        let mut all = Vec::new();
        for mask in 1usize..1usize << priorities.len() {
            if (mask.count_ones() as usize) < minimum {
                continue;
            }
            let ranks: Vec<_> = (0..priorities.len())
                .filter(|rank| mask & (1 << rank) != 0)
                .collect();
            let mean_squared = ranks
                .iter()
                .map(|&rank| priorities[rank] * priorities[rank])
                .sum::<f64>()
                / ranks.len() as f64;
            let score = 0.1 + 0.9 * mean_squared;
            all.push(RankedSubset { ranks, score });
        }
        all.sort_by(compare_subsets);
        let seeds = |item: &RankedSubset| {
            item.ranks.len() == minimum && (0..minimum - 1).all(|index| item.ranks[index] == index)
        };
        let mut selected = Vec::new();
        let mut remaining = budget - (priorities.len() - minimum + 1);
        for subset in all {
            if seeds(&subset) {
                selected.push(subset);
            } else if remaining > 0 {
                selected.push(subset);
                remaining -= 1;
            }
        }
        selected.sort_by(compare_subsets);
        selected
    }

    fn assert_incidence(subsets: &[RankedSubset], pool_size: usize) {
        let mut counts = vec![0; pool_size];
        let mut seen = BTreeSet::new();
        for subset in subsets {
            assert!(seen.insert(&subset.ranks));
            for &rank in &subset.ranks {
                counts[rank] += 1;
            }
        }
        assert!(counts.iter().all(|&count| count > 0));
        assert!(counts.windows(2).all(|pair| pair[0] >= pair[1]));
    }

    #[test]
    fn agrees_with_exhaustive_order_for_variable_sizes_ties_and_budgets() {
        for size in 1..=7 {
            let cases = [
                vec![0.0; size],
                vec![1.0; size],
                (0..size)
                    .map(|rank| (size - rank) as f64 / size as f64)
                    .collect(),
                (0..size)
                    .map(|rank| ((size - rank) / 2) as f64 / size as f64)
                    .collect(),
            ];
            for priorities in cases {
                for minimum in 1..=size {
                    let seed_count = size - minimum + 1;
                    let total = (1usize..1usize << size)
                        .filter(|mask| mask.count_ones() as usize >= minimum)
                        .count();
                    for budget in seed_count..=total + 1 {
                        let actual = enumerate(
                            &priorities,
                            minimum,
                            budget,
                            &mut WorkBudget::new(1_000_000),
                        )
                        .unwrap();
                        assert_eq!(actual, reference(&priorities, minimum, budget));
                        assert_eq!(actual.len(), budget.min(total));
                        assert_incidence(&actual, size);
                    }
                }
            }
        }
    }

    #[test]
    fn scores_mean_weights_and_returns_canonical_ties() {
        let result = enumerate(&[1.0, 0.5, 0.0], 2, 4, &mut WorkBudget::new(1_000)).unwrap();
        assert_eq!(
            result.iter().map(|item| &item.ranks).collect::<Vec<_>>(),
            [&vec![0, 1], &vec![0, 2], &vec![0, 1, 2], &vec![1, 2]]
        );
        assert!((result[0].score - 0.6625).abs() < 1e-12);
        assert!((result[2].score - 0.475).abs() < 1e-12);

        for priority in [0.0, 1.0] {
            let equal = enumerate(&[priority; 3], 1, 7, &mut WorkBudget::new(1_000)).unwrap();
            assert!(equal.windows(2).all(|pair| pair[0].ranks < pair[1].ranks));
            assert!(equal.iter().all(|item| item.score == equal[0].score));
        }
    }

    #[test]
    fn adjacent_float_priorities_preserve_incidence_order() {
        let priorities = [
            1.0,
            f64::from_bits(1.0_f64.to_bits() - 1),
            0.5,
            f64::from_bits(0.5_f64.to_bits() - 1),
            f64::MIN_POSITIVE,
            0.0,
        ];
        for budget in 5..=57 {
            let actual = enumerate(&priorities, 2, budget, &mut WorkBudget::new(10_000)).unwrap();
            assert_eq!(actual, reference(&priorities, 2, budget));
            assert_incidence(&actual, priorities.len());
        }
    }

    #[test]
    fn seed_only_budget_covers_lowest_points_without_requiring_every_size() {
        let result = enumerate(&[1.0, 0.8, 0.5, 0.0], 2, 3, &mut WorkBudget::new(6)).unwrap();
        assert_eq!(
            result
                .iter()
                .map(|item| item.ranks.clone())
                .collect::<Vec<_>>(),
            [vec![0, 1], vec![0, 2], vec![0, 3]]
        );
        assert_incidence(&result, 4);
        assert!(result.iter().all(|item| item.ranks.len() == 2));
    }

    #[test]
    fn huge_conceptual_space_uses_work_proportional_to_budget() {
        let priorities: Vec<_> = (0..40).map(|rank| (40 - rank) as f64 / 40.0).collect();
        let minimum = 20;
        let budget = 256;
        // At most seeds + roots + two children per popped tuple, charging at
        // most n rank entries each. The full space contains > 2^39 subsets.
        let bound = 40 * (2 * (40 - minimum + 1) + 2 * budget);
        let result = enumerate(&priorities, minimum, budget, &mut WorkBudget::new(bound)).unwrap();
        assert_eq!(result.len(), budget);
        assert_incidence(&result, priorities.len());
    }

    #[test]
    fn budget_limits_charge_seeds_roots_and_child_copies() {
        for limit in [0, 5, 14] {
            assert!(matches!(
                enumerate(&[1.0, 0.8, 0.5, 0.0], 2, 10, &mut WorkBudget::new(limit)),
                Err(CombinationError::WorkLimit)
            ));
        }
        // Seeds use 6 entries; roots use 2+3+4 = 9. Any expansion exceeds 15.
        assert!(matches!(
            enumerate(&[1.0, 0.8, 0.5, 0.0], 2, 10, &mut WorkBudget::new(15)),
            Err(CombinationError::WorkLimit)
        ));
    }

    #[test]
    fn rejects_invalid_subset_limits() {
        for (minimum, budget) in [(0, 4), (4, 4), (1, 2), (2, 1)] {
            assert!(matches!(
                enumerate(
                    &[1.0, 0.5, 0.0],
                    minimum,
                    budget,
                    &mut WorkBudget::new(1_000)
                ),
                Err(CombinationError::InvalidConfig(_))
            ));
        }
        assert!(matches!(
            enumerate(&[], 1, 1, &mut WorkBudget::new(1_000)),
            Err(CombinationError::InvalidConfig(_))
        ));
    }
}
