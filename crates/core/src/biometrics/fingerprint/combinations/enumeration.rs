//! Lazy ranked frontiers and bounded enrollment across absolute-priority bands.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::{
    CombinationError, Point, WorkBudget,
    allocation::{BandSchedule, PriorityBand},
    spatial,
};

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
    leading_end: usize,
}

impl RankedSubsets {
    #[cfg(test)]
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
            leading_end: priorities.len(),
        })
    }

    /// Disjoint absolute-score bands, determined by the strongest included
    /// point. Weaker points from any following band remain eligible partners.
    pub(super) fn bands(
        priorities: &[f64],
        minimum: usize,
        maximum: usize,
        work: &mut WorkBudget,
    ) -> Result<[Self; 3], CombinationError> {
        let [high, middle, low] = PriorityBand::ALL.map(|band| {
            let ranks = band.leading_ranks(priorities);
            Ok(Self {
                pending: if ranks.is_empty() {
                    BinaryHeap::new()
                } else {
                    search_roots(
                        priorities,
                        minimum,
                        maximum.min(priorities.len() - ranks.start),
                        ranks.start,
                        work,
                    )?
                },
                deferred: None,
                finished: false,
                leading_end: ranks.end,
            })
        });
        Ok([high?, middle?, low?])
    }

    /// Search only subsets containing a required rank. One root per possible
    /// position of that rank keeps the frontiers disjoint and score-ordered.
    fn including(
        priorities: &[f64],
        minimum: usize,
        maximum: usize,
        required: usize,
        work: &mut WorkBudget,
    ) -> Result<Self, CombinationError> {
        let mut pending = BinaryHeap::new();
        for size in minimum..=maximum {
            for position in 0..size {
                let after = size - position - 1;
                if position > required || after > priorities.len() - required - 1 {
                    continue;
                }
                work.spend(size)?;
                let ranks = (0..position)
                    .chain(std::iter::once(required))
                    .chain(required + 1..required + 1 + after)
                    .collect();
                pending.push(Pending {
                    subset: ranked_subset(priorities, ranks),
                    first_nonminimal: size,
                    fixed_position: Some(position),
                });
            }
        }
        Ok(Self {
            pending,
            deferred: None,
            finished: false,
            leading_end: priorities.len(),
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
                self.leading_end,
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

/// Target 3N selections across absolute-priority bands with 2:1:1 shares.
/// Quotas count valid subsets, not rejected geometry. Exhausted bands release
/// their share; coverage witnesses supplement the target and can alter shares.
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
    select_priority_bands(
        &mut selected,
        priorities,
        positions,
        minimum,
        maximum,
        policy,
        work,
    )?;
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

fn select_priority_bands(
    selected: &mut EnrollmentSelection,
    priorities: &[f64],
    positions: &[Point],
    minimum: usize,
    maximum: usize,
    policy: spatial::SpatialPixels,
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    let target = priorities
        .len()
        .checked_mul(3)
        .ok_or(CombinationError::WorkLimit)?;
    let mut streams = RankedSubsets::bands(priorities, minimum, maximum, work)?;
    let mut schedule = BandSchedule::new();
    while selected.subsets.len() < target {
        let Some(band) = schedule.current() else {
            break;
        };
        let Some(subset) = streams[band.index()].next(priorities, work)? else {
            schedule.exhaust(band);
            continue;
        };
        if spatial::subset(positions, &subset.ranks, policy, work)? {
            selected.add(subset, work)?;
            schedule.accepted();
        }
    }
    Ok(())
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
    let mut witnesses = Vec::new();
    for (required, &covered) in selected.covered.iter().enumerate() {
        work.spend(1)?;
        if covered {
            continue;
        }
        let mut ranked = RankedSubsets::including(priorities, minimum, maximum, required, work)?;
        while let Some(subset) = ranked.next(priorities, work)? {
            if spatial::subset(positions, &subset.ranks, policy, work)? {
                witnesses.push(subset);
                break;
            }
        }
    }
    // The best valid subset for each uncovered point contains the best possible
    // next witness globally. Sorting those heads preserves the original greedy
    // coverage rule without enumerating unrelated subsets ahead of weak points.
    witnesses.sort_unstable_by(compare_subsets);
    for subset in witnesses {
        work.spend(subset.ranks.len())?;
        if subset.ranks.iter().all(|&rank| selected.covered[rank]) {
            continue;
        }
        selected.add(subset, work)?;
        if selected.remaining == 0 {
            break;
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
            fixed_position: None,
        });
    }
    Ok(pending)
}

/// Decrementing a tuple's first nonminimal mutable rank gives its unique
/// parent. Only earlier ranks can introduce a new first nonminimal position;
/// adjacent-rank bounds reject blocked increments. A fixed witness point can
/// create an additional gap. No visited set or exhaustive space is needed.
fn extend_frontier(
    pending: &mut BinaryHeap<Pending>,
    current: &Pending,
    priorities: &[f64],
    leading_end: usize,
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    let last = current.first_nonminimal.min(current.subset.ranks.len() - 1);
    work.spend(last + 1)?;
    for position in 0..=last {
        if current.fixed_position != Some(position) {
            push_child(pending, current, position, priorities, leading_end, work)?;
        }
    }
    Ok(())
}

fn push_child(
    pending: &mut BinaryHeap<Pending>,
    parent: &Pending,
    position: usize,
    priorities: &[f64],
    leading_end: usize,
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    let ranks = &parent.subset.ranks;
    let mut limit = ranks.get(position + 1).copied().unwrap_or(priorities.len());
    if position == 0 {
        limit = limit.min(leading_end);
    }
    if ranks[position] + 1 >= limit {
        return Ok(());
    }

    work.spend(ranks.len())?;
    let mut child = ranks.clone();
    child[position] += 1;
    pending.push(Pending {
        subset: ranked_subset(priorities, child),
        first_nonminimal: position,
        fixed_position: parent.fixed_position,
    });
    Ok(())
}

pub(super) fn ranked_subset(priorities: &[f64], ranks: Vec<usize>) -> RankedSubset {
    let squared_sum: f64 = ranks
        .iter()
        .map(|&rank| priorities[rank] * priorities[rank])
        .sum();
    // Pull the common affine terms outside the average. In particular, a
    // zero-priority set has the exact same score at every subset size.
    let score = 0.1 + 0.9 * (squared_sum / ranks.len() as f64);
    RankedSubset { ranks, score }
}

pub(super) fn compare_subsets(first: &RankedSubset, second: &RankedSubset) -> Ordering {
    second
        .score
        .total_cmp(&first.score)
        .then_with(|| first.ranks.cmp(&second.ranks))
}

struct Pending {
    subset: RankedSubset,
    first_nonminimal: usize,
    fixed_position: Option<usize>,
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

    #[test]
    fn band_and_required_point_frontiers_match_independent_exhaustive_subsets() {
        let priorities = [1.0, 0.95, 0.9, 0.5, 0.49, 0.2, 0.0];
        for minimum in 1..=priorities.len() {
            for maximum in minimum..=priorities.len() {
                let mut all: Vec<_> = (1_usize..1 << priorities.len())
                    .filter(|mask| (minimum..=maximum).contains(&(mask.count_ones() as usize)))
                    .map(|mask| {
                        ranked_subset(
                            &priorities,
                            (0..priorities.len())
                                .filter(|i| mask & (1 << i) != 0)
                                .collect(),
                        )
                    })
                    .collect();
                all.sort_unstable_by(compare_subsets);
                let mut work = WorkBudget::new(1_000_000);
                let bands = RankedSubsets::bands(&priorities, minimum, maximum, &mut work).unwrap();
                for (band, mut stream) in PriorityBand::ALL.into_iter().zip(bands) {
                    let expected: Vec<_> = all
                        .iter()
                        .filter(|s| PriorityBand::containing(priorities[s.ranks[0]]) == band)
                        .cloned()
                        .collect();
                    let mut actual = Vec::new();
                    while let Some(subset) = stream.next(&priorities, &mut work).unwrap() {
                        actual.push(subset);
                    }
                    assert_eq!(actual, expected);
                }
                for required in 0..priorities.len() {
                    let expected: Vec<_> = all
                        .iter()
                        .filter(|s| s.ranks.contains(&required))
                        .cloned()
                        .collect();
                    let mut stream = RankedSubsets::including(
                        &priorities,
                        minimum,
                        maximum,
                        required,
                        &mut work,
                    )
                    .unwrap();
                    let mut actual = Vec::new();
                    while let Some(subset) = stream.next(&priorities, &mut work).unwrap() {
                        actual.push(subset);
                    }
                    assert_eq!(actual, expected);
                }
            }
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
        let band = |subset: &RankedSubset| {
            let strongest = priorities[subset.ranks[0]];
            if strongest > 0.9 {
                0
            } else if strongest >= 0.5 {
                1
            } else {
                2
            }
        };
        let mut bands = [0, 1, 2].map(|group| valid.iter().filter(move |s| band(s) == group));
        while selected.len() < 3 * priorities.len() {
            let before = selected.len();
            for group in [0, 1, 0, 2] {
                if selected.len() == 3 * priorities.len() {
                    break;
                }
                if let Some(subset) = bands[group].next() {
                    selected.push(subset.clone());
                }
            }
            if selected.len() == before {
                break;
            }
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
    fn absolute_bands_allocate_half_quarter_quarter_without_stronger_point_leakage() {
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
        let counts: Vec<_> = PriorityBand::ALL
            .into_iter()
            .map(|band| {
                selection
                    .subsets
                    .iter()
                    .filter(|s| PriorityBand::containing(priorities[s.ranks[0]]) == band)
                    .count()
            })
            .collect();
        assert_eq!(counts, [18, 9, 9]);
        assert!(
            selection
                .subsets
                .iter()
                .any(|s| s.ranks.iter().all(|&rank| priorities[rank] < 0.5))
        );
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
    fn targeted_witnesses_preserve_exhaustive_greedy_order_for_every_initial_coverage_mask() {
        let priorities = [1., 0.95, 0.9, 0.5, 0.49, 0.1];
        let positions: Vec<_> = [
            (0., 0.),
            (1., 1.),
            (12., 0.),
            (0., 12.),
            (11., 11.),
            (4., 4.),
        ]
        .into_iter()
        .map(|(x, y)| Point::new(x, y).unwrap())
        .collect();
        for separation in [0., 5., 100.] {
            let policy = spatial::SpatialPixels {
                anchors: 3,
                separation,
            };
            let mut valid = Vec::new();
            for mask in 1_usize..1 << priorities.len() {
                if !(3..=5).contains(&(mask.count_ones() as usize)) {
                    continue;
                }
                let ranks: Vec<_> = (0..priorities.len())
                    .filter(|rank| mask & (1 << rank) != 0)
                    .collect();
                if spatial::subset(&positions, &ranks, policy, &mut WorkBudget::new(100_000))
                    .unwrap()
                {
                    valid.push(ranked_subset(&priorities, ranks));
                }
            }
            valid.sort_unstable_by(compare_subsets);
            for initial in 0_usize..1 << priorities.len() {
                let covered: Vec<_> = (0..priorities.len())
                    .map(|rank| initial & (1 << rank) != 0)
                    .collect();
                let mut expected_covered = covered.clone();
                let mut expected = Vec::new();
                for subset in &valid {
                    if subset.ranks.iter().any(|&rank| !expected_covered[rank]) {
                        for &rank in &subset.ranks {
                            expected_covered[rank] = true;
                        }
                        expected.push(subset.clone());
                    }
                }
                let mut actual = EnrollmentSelection {
                    subsets: Vec::new(),
                    remaining: covered.iter().filter(|&&covered| !covered).count(),
                    covered,
                };
                add_coverage_witnesses(
                    &mut actual,
                    &priorities,
                    &positions,
                    3,
                    5,
                    policy,
                    &mut WorkBudget::new(100_000),
                )
                .unwrap();
                assert_eq!(
                    actual.subsets, expected,
                    "initial mask {initial}, separation {separation}"
                );
                assert_eq!(actual.covered, expected_covered);
                assert_eq!(
                    actual
                        .subsets
                        .iter()
                        .map(|s| &s.ranks)
                        .collect::<BTreeSet<_>>()
                        .len(),
                    actual.subsets.len()
                );
            }
        }
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
        assert!((3 * priorities.len()..=4 * priorities.len()).contains(&result.subsets.len()));
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
