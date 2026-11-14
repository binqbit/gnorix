//! Bounded distinct subsets with deterministic priority-weighted incidence.

use std::cmp::Ordering;
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
    first_mutable: usize,
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
                "subset size range must fit the candidate points",
            ));
        }
        Ok(Self {
            pending: search_roots(priorities, minimum, maximum, 0, work)?,
            deferred: None,
            finished: false,
            first_mutable: 0,
        })
    }

    /// One disjoint band: the first rank is fixed and every remaining rank is
    /// weaker. The full subset score determines ordering across subset sizes.
    fn with_leading(
        priorities: &[f64],
        minimum: usize,
        maximum: usize,
        leading: usize,
        work: &mut WorkBudget,
    ) -> Result<Self, CombinationError> {
        Ok(Self {
            pending: search_roots(
                priorities,
                minimum,
                maximum.min(priorities.len() - leading),
                leading,
                work,
            )?,
            deferred: None,
            finished: false,
            first_mutable: 1,
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
            extend_frontier(
                &mut self.pending,
                &previous,
                priorities,
                self.first_mutable,
                work,
            )?;
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

pub(super) struct SpatialCoverage {
    pub subsets: Vec<RankedSubset>,
    pub uncovered: Vec<usize>,
}

/// Partition by strongest included rank. The first band receives N selections;
/// each next receives ceil(previous / 2), with a floor of one. Quotas count valid
/// subsets, not rejected geometry. Coverage witnesses supplement those quotas.
/// Every source point must occur, or the caller fails the complete enrollment.
pub(super) fn enumerate_spatial(
    priorities: &[f64],
    positions: &[Point],
    minimum: usize,
    maximum: usize,
    policy: spatial::SpatialPixels,
    work: &mut WorkBudget,
) -> Result<SpatialCoverage, CombinationError> {
    let minimum = minimum.max(policy.anchors);
    if minimum == 0
        || minimum > maximum
        || maximum > priorities.len()
        || positions.len() != priorities.len()
    {
        return Err(CombinationError::InvalidConfig(
            "subset size range must fit the regional features",
        ));
    }
    work.spend(priorities.len())?;
    let mut selected = EnrollmentSelection {
        subsets: Vec::new(),
        covered: vec![false; priorities.len()],
        remaining: priorities.len(),
    };
    let mut quota = priorities.len();
    for leading in 0..=priorities.len() - minimum {
        let mut band = RankedSubsets::with_leading(priorities, minimum, maximum, leading, work)?;
        let mut accepted = 0;
        while accepted < quota {
            let Some(subset) = band.next(priorities, work)? else {
                break;
            };
            if spatial::subset(positions, &subset.ranks, policy, work)? {
                selected.add(subset, work)?;
                accepted += 1;
            }
        }
        quota = quota.div_ceil(2);
    }
    add_coverage_witnesses(
        &mut selected,
        priorities,
        positions,
        minimum,
        maximum,
        policy,
        work,
    )?;
    selected.subsets.sort_unstable_by(compare_subsets);
    work.spend(selected.covered.len())?;
    let uncovered = selected
        .covered
        .into_iter()
        .enumerate()
        .filter_map(|(rank, present)| (!present).then_some(rank))
        .collect();
    Ok(SpatialCoverage {
        subsets: selected.subsets,
        uncovered,
    })
}

struct EnrollmentSelection {
    subsets: Vec<RankedSubset>,
    covered: Vec<bool>,
    remaining: usize,
}

impl EnrollmentSelection {
    fn add(&mut self, subset: RankedSubset, work: &mut WorkBudget) -> Result<(), CombinationError> {
        work.spend(subset.ranks.len())?;
        for &rank in &subset.ranks {
            if !self.covered[rank] {
                self.covered[rank] = true;
                self.remaining -= 1;
            }
        }
        self.subsets.push(subset);
        Ok(())
    }
}

fn add_coverage_witnesses(
    selected: &mut EnrollmentSelection,
    priorities: &[f64],
    positions: &[Point],
    minimum: usize,
    maximum: usize,
    policy: spatial::SpatialPixels,
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    if selected.remaining == 0 {
        return Ok(());
    }
    let mut ranked = RankedSubsets::new(priorities, minimum, maximum, work)?;
    while let Some(subset) = ranked.next(priorities, work)? {
        work.spend(subset.ranks.len())?;
        if subset.ranks.iter().all(|&rank| selected.covered[rank]) {
            continue;
        }
        if spatial::subset(positions, &subset.ranks, policy, work)? {
            selected.add(subset, work)?;
            if selected.remaining == 0 {
                break;
            }
        }
    }
    Ok(())
}

fn search_roots(
    priorities: &[f64],
    minimum: usize,
    maximum: usize,
    start: usize,
    work: &mut WorkBudget,
) -> Result<BinaryHeap<Pending>, CombinationError> {
    let mut pending = BinaryHeap::new();
    for size in minimum..=maximum {
        work.spend(size)?;
        pending.push(Pending {
            subset: ranked_subset(priorities, (start..start + size).collect()),
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
    first_mutable: usize,
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    let size = current.subset.ranks.len();
    if size <= first_mutable {
        return Ok(());
    }
    let first = current.first_nonminimal;
    if first == size {
        push_child(pending, current, size - 1, priorities, work)?;
    } else {
        push_child(pending, current, first, priorities, work)?;
        if first > first_mutable {
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

fn ranked_subset(priorities: &[f64], ranks: Vec<usize>) -> RankedSubset {
    let squared_sum: f64 = ranks
        .iter()
        .map(|&rank| priorities[rank] * priorities[rank])
        .sum();
    // Pull the common affine terms outside the average. In particular, a
    // zero-priority set has the exact same score at every subset size.
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
        maximum: usize,
        policy: spatial::SpatialPixels,
    ) -> SpatialCoverage {
        let mut valid = (1_usize..1 << priorities.len())
            .filter_map(|mask| {
                let ranks: Vec<_> = (0..priorities.len())
                    .filter(|rank| mask & (1 << rank) != 0)
                    .collect();
                if !(minimum.max(policy.anchors)..=maximum).contains(&ranks.len()) {
                    return None;
                }
                let selected: Vec<_> = ranks.iter().map(|&rank| positions[rank]).collect();
                spatial::check(&selected, policy, &mut WorkBudget::new(100_000))
                    .unwrap()
                    .then(|| ranked_subset(priorities, ranks))
            })
            .collect::<Vec<_>>();
        valid.sort_unstable_by(compare_subsets);
        let mut selected = Vec::new();
        let mut quota = priorities.len();
        for rank in 0..priorities.len() {
            selected.extend(
                valid
                    .iter()
                    .filter(|s| s.ranks[0] == rank)
                    .take(quota)
                    .cloned(),
            );
            quota = quota.div_ceil(2);
        }
        let mut covered: BTreeSet<_> = selected
            .iter()
            .flat_map(|s| s.ranks.iter().copied())
            .collect();
        for subset in valid {
            if subset.ranks.iter().any(|rank| !covered.contains(rank)) {
                covered.extend(subset.ranks.iter().copied());
                selected.push(subset);
            }
        }
        selected.sort_unstable_by(compare_subsets);
        SpatialCoverage {
            subsets: selected,
            uncovered: (0..priorities.len())
                .filter(|rank| !covered.contains(rank))
                .collect(),
        }
    }

    #[test]
    fn automatic_bands_and_coverage_match_exhaustive_reference() {
        let layouts = [
            [
                (0., 0.),
                (10., 0.),
                (0., 10.),
                (-1., 1.),
                (3., 3.),
                (0., 0.),
            ],
            [(0., 0.), (2., 0.), (4., 0.), (6., 0.), (8., 0.), (10., 0.)],
            [
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
            for priorities in [
                vec![0.5; 6],
                vec![1., 0.9, 0.8, 0.5, 0.3, 0.],
                vec![
                    1.,
                    f64::from_bits(1.0_f64.to_bits() - 1),
                    0.5,
                    f64::from_bits(0.5_f64.to_bits() - 1),
                    f64::MIN_POSITIVE,
                    0.,
                ],
            ] {
                for (anchors, separation) in [(1, 0.), (3, 5.), (4, 3.), (3, 100.)] {
                    let policy = spatial::SpatialPixels {
                        anchors,
                        separation,
                    };
                    for minimum in 1..=6 {
                        for maximum in minimum.max(anchors)..=6 {
                            let expected = spatial_reference(
                                &priorities,
                                &positions,
                                minimum,
                                maximum,
                                policy,
                            );
                            let actual = enumerate_spatial(
                                &priorities,
                                &positions,
                                minimum,
                                maximum,
                                policy,
                                &mut WorkBudget::new(100_000),
                            )
                            .unwrap();
                            assert_eq!(actual.subsets, expected.subsets);
                            assert_eq!(actual.uncovered, expected.uncovered);
                            let distinct: BTreeSet<_> =
                                actual.subsets.iter().map(|s| &s.ranks).collect();
                            assert_eq!(distinct.len(), actual.subsets.len());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn decreasing_band_quotas_allow_omitting_the_strongest_features() {
        let priorities: Vec<_> = (0..12).map(|rank| 1. - rank as f64 / 12.).collect();
        let positions: Vec<_> = (0..12)
            .map(|rank| Point::new(rank as f64, 0.).unwrap())
            .collect();
        let selection = enumerate_spatial(
            &priorities,
            &positions,
            3,
            3,
            spatial::SpatialPixels {
                anchors: 1,
                separation: 0.,
            },
            &mut WorkBudget::new(100_000),
        )
        .unwrap();
        assert!(selection.uncovered.is_empty());
        let counts: Vec<_> = (0..10)
            .map(|rank| {
                selection
                    .subsets
                    .iter()
                    .filter(|s| s.ranks[0] == rank)
                    .count()
            })
            .collect();
        assert_eq!(counts, [12, 6, 3, 2, 1, 1, 1, 1, 1, 1]);
        assert!(selection.subsets.iter().any(|s| s.ranks == [9, 10, 11]));
    }

    #[test]
    fn coverage_witnesses_retain_points_missed_by_band_quotas() {
        let priorities = [1., 0.9, 0.8, 0.7, 0.6, 0.];
        let positions: Vec<_> = [
            (0., 0.),
            (10., 0.),
            (0., 10.),
            (9., 9.),
            (11., 11.),
            (0.1, 0.1),
        ]
        .into_iter()
        .map(|(x, y)| Point::new(x, y).unwrap())
        .collect();
        let policy = spatial::SpatialPixels {
            anchors: 3,
            separation: 5.,
        };
        let expected = spatial_reference(&priorities, &positions, 3, 4, policy);
        let mut work = WorkBudget::new(100_000);
        let result = enumerate_spatial(&priorities, &positions, 3, 4, policy, &mut work).unwrap();
        assert_eq!(result.subsets, expected.subsets);
        assert!(result.uncovered.is_empty());
        let used = 100_000 - work.remaining;
        assert!(matches!(
            enumerate_spatial(
                &priorities,
                &positions,
                3,
                4,
                policy,
                &mut WorkBudget::new(used - 1)
            ),
            Err(CombinationError::WorkLimit)
        ));
    }

    #[test]
    fn large_conceptual_space_has_bounded_selection_work() {
        let priorities: Vec<_> = (0..100).map(|rank| 1. - rank as f64 / 100.).collect();
        let positions: Vec<_> = (0..100)
            .map(|rank| Point::new(rank as f64, 0.).unwrap())
            .collect();
        let result = enumerate_spatial(
            &priorities,
            &positions,
            3,
            6,
            spatial::SpatialPixels {
                anchors: 1,
                separation: 0.,
            },
            &mut WorkBudget::new(100_000),
        )
        .unwrap();
        assert!(result.uncovered.is_empty());
        assert!(result.subsets.len() < 3 * priorities.len());
        assert!(
            result
                .subsets
                .iter()
                .all(|s| (3..=6).contains(&s.ranks.len()))
        );
    }

    #[test]
    fn impossible_spatial_selection_reports_uncovered_points() {
        let priorities = [1.; 5];
        let positions: Vec<_> = (0..5)
            .map(|rank| Point::new(rank as f64, 0.).unwrap())
            .collect();
        let result = enumerate_spatial(
            &priorities,
            &positions,
            3,
            4,
            spatial::SpatialPixels {
                anchors: 3,
                separation: 100.,
            },
            &mut WorkBudget::new(100_000),
        )
        .unwrap();
        assert!(result.subsets.is_empty());
        assert_eq!(result.uncovered, [0, 1, 2, 3, 4]);
    }
}
