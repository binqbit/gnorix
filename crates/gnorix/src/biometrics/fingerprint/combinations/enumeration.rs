//! Bounded distinct subsets with deterministic priority-weighted incidence.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::collections::BinaryHeap;

use super::{CombinationError, Point, WorkBudget, spatial};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RankedSubset {
    pub ranks: Vec<usize>,
    pub score: f64,
}

/// Lazy best-first traversal over an explicit subset-size range.
///
/// Priorities are borrowed only during each operation, so callers can own their
/// point collection. Expanding the previous tuple is deferred until the next
/// request; stopping after a match performs no work on its unused descendants.
pub(super) struct RankedSubsets {
    pending: BinaryHeap<Pending>,
    deferred: Option<Pending>,
    finished: bool,
}

impl RankedSubsets {
    pub(super) fn new(
        priorities: &[f64],
        minimum: usize,
        maximum: usize,
        work: &mut WorkBudget,
    ) -> Result<Self, CombinationError> {
        if minimum == 0 || minimum > maximum || maximum > priorities.len() {
            return Err(CombinationError::InvalidConfig(
                "subset size range must fit the candidate pool",
            ));
        }
        Ok(Self {
            pending: search_roots(priorities, minimum, maximum, work)?,
            deferred: None,
            finished: false,
        })
    }

    pub(super) fn next(
        &mut self,
        priorities: &[f64],
        work: &mut WorkBudget,
    ) -> Result<Option<RankedSubset>, CombinationError> {
        if self.finished {
            return Ok(None);
        }
        let result = self.advance(priorities, work);
        if result.is_err() || matches!(result, Ok(None)) {
            self.finished = true;
            self.pending.clear();
            self.deferred = None;
        }
        result
    }

    fn advance(
        &mut self,
        priorities: &[f64],
        work: &mut WorkBudget,
    ) -> Result<Option<RankedSubset>, CombinationError> {
        if let Some(previous) = self.deferred.take() {
            extend_frontier(&mut self.pending, &previous, priorities, work)?;
        }
        let Some(current) = self.pending.pop() else {
            return Ok(None);
        };
        // The current parent remains available for deferred expansion. Charge
        // the separate owned rank list returned to the consumer.
        work.spend(current.subset.ranks.len())?;
        let subset = current.subset.clone();
        self.deferred = Some(current);
        Ok(Some(subset))
    }
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

pub(super) struct SpatialCoverage {
    pub subsets: Vec<RankedSubset>,
    pub uncovered: Vec<usize>,
}

/// Keep coverage witnesses and the best valid subsets in one ranked traversal.
/// A bounded best-candidate buffer avoids replaying the stream after coverage.
pub(super) fn enumerate_spatial(
    priorities: &[f64],
    positions: &[Point],
    minimum: usize,
    budget: usize,
    policy: spatial::SpatialPixels,
    work: &mut WorkBudget,
) -> Result<SpatialCoverage, CombinationError> {
    validate_limits(priorities.len(), minimum, budget)?;
    if policy.unconstrained() {
        return Ok(SpatialCoverage {
            subsets: enumerate(priorities, minimum, budget, work)?,
            uncovered: Vec::new(),
        });
    }
    work.spend(priorities.len())?;
    let mut covered = vec![false; priorities.len()];
    let mut remaining = priorities.len();
    let mut witnesses = Vec::new();
    let mut best = Vec::new();
    let minimum = minimum.max(policy.anchors);
    if minimum <= priorities.len() {
        let mut ranked = RankedSubsets::new(priorities, minimum, priorities.len(), work)?;
        while let Some(subset) = ranked.next(priorities, work)? {
            if !spatial::subset(positions, &subset.ranks, policy, work)? {
                continue;
            }
            let mut adds_coverage = false;
            for &rank in &subset.ranks {
                work.spend(1)?;
                if !covered[rank] {
                    covered[rank] = true;
                    remaining -= 1;
                    adds_coverage = true;
                }
            }
            if best.len() < budget {
                work.spend(subset.ranks.len())?;
                best.push(subset.clone());
            }
            if adds_coverage {
                witnesses.push(subset);
            }
            if remaining == 0 && best.len() == budget {
                break;
            }
        }
    }
    work.spend(covered.len())?;
    let uncovered: Vec<_> = covered
        .into_iter()
        .enumerate()
        .filter_map(|(rank, present)| (!present).then_some(rank))
        .collect();
    if !uncovered.is_empty() {
        return Ok(SpatialCoverage {
            subsets: witnesses,
            uncovered,
        });
    }
    let subsets = complete_coverage(witnesses, best, budget, work)?;
    Ok(SpatialCoverage { subsets, uncovered })
}

fn complete_coverage(
    mut selected: Vec<RankedSubset>,
    best: Vec<RankedSubset>,
    budget: usize,
    work: &mut WorkBudget,
) -> Result<Vec<RankedSubset>, CombinationError> {
    let mut witnesses = BTreeSet::new();
    for subset in &selected {
        work.spend(subset.ranks.len())?;
        witnesses.insert(subset.ranks.clone());
    }
    for subset in best {
        if selected.len() == budget {
            break;
        }
        work.spend(subset.ranks.len())?;
        if !witnesses.contains(&subset.ranks) {
            selected.push(subset);
        }
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
    let mut ranked = RankedSubsets::new(priorities, minimum, priorities.len(), work)?;
    while selected.len() < budget {
        let Some(subset) = ranked.next(priorities, work)? else {
            break;
        };
        if !is_coverage_seed(&subset.ranks, minimum) {
            selected.push(subset);
        }
    }
    Ok(())
}

fn search_roots(
    priorities: &[f64],
    minimum: usize,
    maximum: usize,
    work: &mut WorkBudget,
) -> Result<BinaryHeap<Pending>, CombinationError> {
    let mut pending = BinaryHeap::new();
    for size in minimum..=maximum {
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
    fn lazy_size_ranges_match_exhaustive_order_without_duplicates() {
        for size in 1..=7 {
            for priorities in [
                vec![0.0; size],
                vec![1.0; size],
                (0..size)
                    .map(|rank| (size - rank) as f64 / size as f64)
                    .collect(),
            ] {
                for minimum in 1..=size {
                    for maximum in minimum..=size {
                        let mut expected: Vec<_> = (1usize..1usize << size)
                            .filter(|mask| {
                                (minimum..=maximum).contains(&(mask.count_ones() as usize))
                            })
                            .map(|mask| {
                                ranked_subset(
                                    &priorities,
                                    (0..size).filter(|rank| mask & (1 << rank) != 0).collect(),
                                )
                            })
                            .collect();
                        expected.sort_by(compare_subsets);
                        let mut work = WorkBudget::new(100_000);
                        let mut cursor =
                            RankedSubsets::new(&priorities, minimum, maximum, &mut work).unwrap();
                        let mut actual = Vec::new();
                        while let Some(subset) = cursor.next(&priorities, &mut work).unwrap() {
                            actual.push(subset);
                        }
                        assert_eq!(actual, expected);
                        let distinct: BTreeSet<_> = actual.iter().map(|item| &item.ranks).collect();
                        assert_eq!(distinct.len(), actual.len());
                        assert!(cursor.next(&priorities, &mut work).unwrap().is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn lazy_expansion_waits_for_another_request_and_failure_terminates_search() {
        let priorities = [1.0, 0.8, 0.6, 0.4];
        // Two entries construct the first root and two copy its yielded ranks.
        // No budget remains for the unused children until another request.
        let mut work = WorkBudget::new(4);
        let mut cursor = RankedSubsets::new(&priorities, 2, 2, &mut work).unwrap();
        assert_eq!(
            cursor.next(&priorities, &mut work).unwrap().unwrap().ranks,
            [0, 1]
        );
        assert!(matches!(
            cursor.next(&priorities, &mut work),
            Err(CombinationError::WorkLimit)
        ));
        assert!(cursor.next(&priorities, &mut work).unwrap().is_none());
        for (minimum, maximum) in [(0, 2), (3, 2), (1, 5)] {
            assert!(
                RankedSubsets::new(&priorities, minimum, maximum, &mut WorkBudget::new(100))
                    .is_err()
            );
        }
    }

    fn spatial_reference(
        priorities: &[f64],
        positions: &[Point],
        minimum: usize,
        budget: usize,
        policy: spatial::SpatialPixels,
    ) -> SpatialCoverage {
        let mut valid = (1_usize..1 << priorities.len())
            .filter_map(|mask| {
                let ranks: Vec<_> = (0..priorities.len())
                    .filter(|rank| mask & (1 << rank) != 0)
                    .collect();
                if ranks.len() < minimum.max(policy.anchors) {
                    return None;
                }
                let selected: Vec<_> = ranks.iter().map(|&rank| positions[rank]).collect();
                spatial::check(&selected, policy, &mut WorkBudget::new(100_000))
                    .unwrap()
                    .then(|| ranked_subset(priorities, ranks))
            })
            .collect::<Vec<_>>();
        valid.sort_unstable_by(compare_subsets);
        let mut covered = BTreeSet::<usize>::new();
        let mut selected = Vec::new();
        for subset in &valid {
            if subset.ranks.iter().any(|rank| !covered.contains(rank)) {
                covered.extend(subset.ranks.iter().copied());
                selected.push(subset.clone());
            }
        }
        let uncovered: Vec<_> = (0..priorities.len())
            .filter(|rank| !covered.contains(rank))
            .collect();
        if uncovered.is_empty() {
            for subset in valid {
                if selected.len() == budget {
                    break;
                }
                if !selected.iter().any(|seed| seed.ranks == subset.ranks) {
                    selected.push(subset);
                }
            }
            selected.sort_unstable_by(compare_subsets);
        }
        SpatialCoverage {
            subsets: selected,
            uncovered,
        }
    }

    #[test]
    fn spatial_selection_matches_exhaustive_coverage_and_ranking() {
        let layouts = [
            vec![
                (0., 0.),
                (10., 0.),
                (0., 10.),
                (-1., 1.),
                (3., 3.),
                (0., 0.),
            ],
            vec![(0., 0.), (2., 0.), (4., 0.), (6., 0.), (8., 0.), (10., 0.)],
            vec![
                (0., 0.),
                (10., 0.),
                (12., 7.),
                (7., 12.),
                (0., 10.),
                (5., 5.),
            ],
        ];
        for layout in layouts {
            let positions: Vec<_> = layout
                .into_iter()
                .map(|(x, y)| Point::new(x, y).unwrap())
                .collect();
            for priorities in [vec![0.5; 6], vec![1., 0.9, 0.8, 0.5, 0.3, 0.]] {
                for (diameter, anchors, separation) in
                    [(0., 2, 0.), (8., 3, 5.), (10., 4, 3.), (20., 3, 5.)]
                {
                    let policy = spatial::SpatialPixels {
                        diameter,
                        anchors,
                        separation,
                    };
                    for minimum in 1..=3 {
                        for budget in [7 - minimum, 12, 64] {
                            let expected =
                                spatial_reference(&priorities, &positions, minimum, budget, policy);
                            let actual = enumerate_spatial(
                                &priorities,
                                &positions,
                                minimum,
                                budget,
                                policy,
                                &mut WorkBudget::new(100_000),
                            )
                            .unwrap();
                            assert_eq!(actual.subsets, expected.subsets);
                            assert_eq!(actual.uncovered, expected.uncovered);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn spatial_selection_keeps_late_coverage_without_losing_best_valid_sets() {
        let positions: Vec<_> = [(0., 0.), (10., 0.), (0., 10.), (9., 9.), (4., 4.)]
            .into_iter()
            .map(|(x, y)| Point::new(x, y).unwrap())
            .collect();
        let priorities = [1., 0.9, 0.8, 0.7, 0.];
        let policy = spatial::SpatialPixels {
            diameter: 8.,
            anchors: 3,
            separation: 5.,
        };
        let expected = spatial_reference(&priorities, &positions, 3, 3, policy);
        assert!(expected.uncovered.is_empty());
        assert!(
            expected
                .subsets
                .iter()
                .any(|subset| subset.ranks.contains(&4))
        );
        let mut succeeded = false;
        for limit in 1..=10_000 {
            match enumerate_spatial(
                &priorities,
                &positions,
                3,
                3,
                policy,
                &mut WorkBudget::new(limit),
            ) {
                Ok(actual) => {
                    assert_eq!(actual.subsets, expected.subsets);
                    assert!(actual.uncovered.is_empty());
                    succeeded = true;
                    break;
                }
                Err(error) => assert_eq!(error, CombinationError::WorkLimit),
            }
        }
        assert!(succeeded);
    }

    #[test]
    fn impossible_large_spatial_pool_stops_at_work_limit() {
        let positions: Vec<_> = (0..32).map(|x| Point::new(x as f64, 0.).unwrap()).collect();
        let priorities = vec![1.; 32];
        let policy = spatial::SpatialPixels {
            diameter: 100.,
            anchors: 3,
            separation: 10.,
        };
        assert!(matches!(
            enumerate_spatial(
                &priorities,
                &positions,
                3,
                32,
                policy,
                &mut WorkBudget::new(1000)
            ),
            Err(CombinationError::WorkLimit)
        ));
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
        // At most seeds + roots + two children and one owned result per tuple, charging at
        // most n rank entries each. The full space contains > 2^39 subsets.
        let bound = 40 * (2 * (40 - minimum + 1) + 3 * budget);
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
