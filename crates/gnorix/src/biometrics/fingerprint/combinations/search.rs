//! Lazy signing search over overlapping circles of a fresh capture.

use std::{
    cmp::Ordering,
    collections::{BTreeSet, BinaryHeap},
    num::NonZeroUsize,
};

use super::{
    CombinationError, CombinationPoint, Point, RegionBounds, WorkBudget, collect_points,
    enumeration::RankedSubsets, regions,
};
use crate::{
    biometrics::{
        Candidate, Candidates,
        fingerprint::{Fingerprint, FingerprintError, shapes},
    },
    error::AlgorithmError,
};

/// Signing searches every retained, scored point inside each adaptive circle.
/// Applications supply the enrolled minimum, circle radius and frame padding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SigningSearchConfig {
    pub min_points: usize,
    /// Initial circle radius measured in the fresh capture's mean ridge width R.
    pub radius_units: f64,
    /// Fixed frame padding used during enrollment, not a new neighbor search.
    pub enrollment_padding: u32,
    /// Cumulative projection, regions, frontier, deduplication and geometry work.
    /// The caller's authorization limits separately bound candidate attempts.
    pub max_work: usize,
    pub max_descriptor_bytes: NonZeroUsize,
}

impl SigningSearchConfig {
    pub fn validate(&self) -> Result<(), CombinationError> {
        if self.min_points == 0 || self.min_points.checked_mul(2).is_none() {
            return Err(CombinationError::InvalidConfig(
                "signing subset size must be nonzero and its double must fit usize",
            ));
        }
        if !self.radius_units.is_finite() || self.radius_units <= 0.0 {
            return Err(CombinationError::InvalidConfig(
                "circle radius must be finite and positive in ridge units",
            ));
        }
        if self.enrollment_padding > i32::MAX as u32 {
            return Err(CombinationError::InvalidConfig(
                "enrollment padding must fit the signed grid neighborhood",
            ));
        }
        if self.max_work == 0 {
            return Err(CombinationError::InvalidConfig(
                "work limit must be nonzero",
            ));
        }
        Ok(())
    }
}

/// Produces one canonical baseline for each distinct regional point subset of
/// size `min_points..=min(2 * min_points, points_in_region)`.
///
/// Adaptive circles cover the confirmed pattern and grow to include twice the
/// minimum, or all available points when fewer exist. Every scored point inside
/// a circle remains eligible. Overlapping circles never repeat the same subset.
/// Subsets are merged by mean `0.1 + 0.9 * priority²`, then global point rank;
/// equal point priorities use ascending source IDs. Known zero scores remain
/// eligible, while unknown scores do not. There is no global-search fallback.
/// No coverage seeds, regional pool cap or positional perturbations are added.
/// The same observed subset retains enrollment's frame, phase and fixed padding.
///
/// Preparation owns the projected points. Each request generates only the next
/// candidate, so the caller can stop immediately at a Merkle membership match.
/// Natural exhaustion covers only the configured regional space. Work or
/// geometry errors are yielded once, then the stream is exhausted.
pub fn signing_candidates(
    fingerprint: &Fingerprint,
    config: &SigningSearchConfig,
) -> Result<Candidates<'static>, CombinationError> {
    Ok(Box::new(prepare_signing_search(fingerprint, config)?))
}

/// Prepare the same stream as [`signing_candidates`], retaining its observed
/// points, actual circles and most recently yielded feature identities.
/// Diagnostics do not change selection, ranking, geometry or candidate bytes.
pub fn prepare_signing_search(
    fingerprint: &Fingerprint,
    config: &SigningSearchConfig,
) -> Result<SigningSearch, CombinationError> {
    config.validate()?;
    let spacing = fingerprint.scale.unit_px;
    if !spacing.is_finite() || spacing <= 0.0 {
        return Err(CombinationError::InvalidFingerprint(
            FingerprintError::ScaleUnavailable("ridge width must be finite and positive"),
        ));
    }
    let mut work = WorkBudget::new(config.max_work);
    let (points, _) = collect_points(fingerprint, &mut work)?;
    let bounds = regions::pattern_bounds(fingerprint, &points, &mut work)?;
    SigningSearch::new(points, bounds, spacing, *config, work)
}

/// One actual search circle, expressed in the current capture's pixel frame.
#[derive(Clone, Debug, PartialEq)]
pub struct SigningRegion {
    pub id: usize,
    pub center: Point,
    pub initial_radius: f64,
    pub radius: f64,
    /// Every eligible source ID inside this circle, in descending priority order.
    /// Equal priorities use ascending source IDs. This is not a truncated pool.
    pub feature_ids: Vec<usize>,
}

/// Owned, lazy candidate stream with read-only source diagnostics.
/// Work and geometry failures yield one error and then terminate the stream.
pub struct SigningSearch {
    points: Vec<CombinationPoint>,
    regions: Vec<SigningRegion>,
    last_feature_ids: Vec<usize>,
    subsets: RegionalSubsets,
    spacing: f64,
    config: SigningSearchConfig,
    work: WorkBudget,
    finished: bool,
}

impl SigningSearch {
    /// All eligible points in descending priority order, with source IDs as ties.
    pub fn points(&self) -> &[CombinationPoint] {
        &self.points
    }

    /// The circles used by this stream, including sparse-circle expansion.
    pub fn regions(&self) -> &[SigningRegion] {
        &self.regions
    }

    /// Ascending source IDs of the most recently yielded successful candidate.
    /// Empty before the first success; an error or exhaustion retains that last
    /// successful set. Pending lookahead never changes these identities.
    pub fn last_feature_ids(&self) -> &[usize] {
        &self.last_feature_ids
    }

    fn new(
        mut points: Vec<CombinationPoint>,
        bounds: RegionBounds,
        spacing: f64,
        config: SigningSearchConfig,
        mut work: WorkBudget,
    ) -> Result<Self, CombinationError> {
        if points.len() < config.min_points {
            return Err(CombinationError::InsufficientPoints {
                required: config.min_points,
                available: points.len(),
            });
        }
        work.spend(points.len())?;
        points.sort_unstable_by(|a, b| b.priority.total_cmp(&a.priority).then(a.id.cmp(&b.id)));
        let circles = regions::circular_regions(
            &points,
            bounds,
            spacing,
            config.radius_units,
            2 * config.min_points,
            &mut work,
        )?;
        let regions = describe_regions(&points, &circles, &mut work)?;
        let subsets = RegionalSubsets::new(&points, circles, config.min_points, &mut work)?;
        Ok(Self {
            points,
            regions,
            last_feature_ids: Vec::new(),
            subsets,
            spacing,
            config,
            work,
            finished: false,
        })
    }

    fn advance(&mut self) -> Result<Option<Candidate>, AlgorithmError> {
        let Some(ranks) = self.subsets.next(&mut self.work)? else {
            return Ok(None);
        };
        self.work.spend(ranks.len())?;
        let mut selected = ranks
            .iter()
            .map(|&rank| &self.points[rank])
            .collect::<Vec<_>>();
        selected.sort_unstable_by_key(|point| point.id);
        self.work.spend(selected.len())?;
        let positions = selected
            .iter()
            .map(|point| point.position)
            .collect::<Vec<_>>();
        self.work.spend(selected.len())?;
        let (candidate, used) = shapes::baseline_candidate(
            &positions,
            self.spacing,
            self.config.enrollment_padding,
            self.work.remaining,
            self.config.max_descriptor_bytes,
        )?;
        self.work.spend(used)?;
        self.last_feature_ids.clear();
        self.last_feature_ids
            .extend(selected.iter().map(|point| point.id));
        Ok(Some(candidate))
    }
}

impl Iterator for SigningSearch {
    type Item = Result<Candidate, AlgorithmError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match self.advance() {
            Ok(Some(candidate)) => Some(Ok(candidate)),
            Ok(None) => {
                self.finished = true;
                None
            }
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}

fn describe_regions(
    points: &[CombinationPoint],
    circles: &[regions::RegionCandidates],
    work: &mut WorkBudget,
) -> Result<Vec<SigningRegion>, CombinationError> {
    work.spend(circles.len())?;
    circles
        .iter()
        .map(|circle| {
            work.spend(circle.indices.len())?;
            Ok(SigningRegion {
                id: circle.id,
                center: circle.center,
                initial_radius: circle.initial_radius,
                radius: circle.radius,
                feature_ids: circle
                    .indices
                    .iter()
                    .map(|&index| points[index].id)
                    .collect(),
            })
        })
        .collect()
}

struct RegionCursor {
    indices: Vec<usize>,
    priorities: Vec<f64>,
    subsets: RankedSubsets,
}

/// Keep one head per regional stream; expand a consumed head only on demand.
struct RegionalSubsets {
    regions: Vec<RegionCursor>,
    pending: BinaryHeap<RegionHead>,
    deferred: Option<usize>,
    seen: BTreeSet<Vec<usize>>,
}

impl RegionalSubsets {
    fn new(
        points: &[CombinationPoint],
        circles: Vec<regions::RegionCandidates>,
        minimum: usize,
        work: &mut WorkBudget,
    ) -> Result<Self, CombinationError> {
        let mut search = Self {
            regions: Vec::new(),
            pending: BinaryHeap::new(),
            deferred: None,
            seen: BTreeSet::new(),
        };
        for circle in circles {
            work.spend(1)?;
            work.spend(circle.indices.len())?;
            let priorities: Vec<_> = circle
                .indices
                .iter()
                .map(|&index| points[index].priority)
                .collect();
            let maximum = (2 * minimum).min(priorities.len());
            let subsets = RankedSubsets::new(&priorities, minimum, maximum, work)?;
            let region = search.regions.len();
            search.regions.push(RegionCursor {
                indices: circle.indices,
                priorities,
                subsets,
            });
            search.replenish(region, work)?;
        }
        Ok(search)
    }

    fn next(&mut self, work: &mut WorkBudget) -> Result<Option<Vec<usize>>, CombinationError> {
        loop {
            if let Some(region) = self.deferred.take() {
                self.replenish(region, work)?;
            }
            let Some(head) = self.pending.pop() else {
                return Ok(None);
            };
            work.spend(1)?;
            self.deferred = Some(head.region);
            // Global ranks identify the same source IDs in every circle. Charge
            // the lookup/copy even when an overlapping circle repeats a subset.
            work.spend(head.ranks.len())?;
            if self.seen.insert(head.ranks.clone()) {
                return Ok(Some(head.ranks));
            }
        }
    }

    fn replenish(&mut self, region: usize, work: &mut WorkBudget) -> Result<(), CombinationError> {
        let cursor = &mut self.regions[region];
        let Some(subset) = cursor.subsets.next(&cursor.priorities, work)? else {
            return Ok(());
        };
        work.spend(1)?;
        work.spend(subset.ranks.len())?;
        self.pending.push(RegionHead {
            region,
            ranks: subset
                .ranks
                .iter()
                .map(|&rank| cursor.indices[rank])
                .collect(),
            score: subset.score,
        });
        Ok(())
    }
}

struct RegionHead {
    region: usize,
    ranks: Vec<usize>,
    score: f64,
}

impl PartialEq for RegionHead {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for RegionHead {}

impl PartialOrd for RegionHead {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RegionHead {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.ranks.cmp(&self.ranks))
            .then_with(|| other.region.cmp(&self.region))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Error,
        biometrics::fingerprint::{
            FeatureCombination, FeatureKind, GridShapeConfig, GridShapeError, Point,
            grid_candidate, prepare_grid_shapes,
        },
        error::Resource,
    };

    fn config(min_points: usize, enrollment_padding: u32) -> SigningSearchConfig {
        SigningSearchConfig {
            min_points,
            radius_units: 100_000.0,
            enrollment_padding,
            max_work: 1_000_000,
            max_descriptor_bytes: NonZeroUsize::new(4096).unwrap(),
        }
    }

    fn points(count: usize) -> Vec<CombinationPoint> {
        (0..count)
            .map(|index| CombinationPoint {
                id: count - index,
                kind: FeatureKind::Ending,
                position: Point::new(index as f64 * 13.7, (index * index % 17) as f64 * 5.3)
                    .unwrap(),
                priority: 1.0 - index as f64 / (count - 1).max(1) as f64,
            })
            .collect()
    }

    fn search(points: Vec<CombinationPoint>, config: SigningSearchConfig) -> SigningSearch {
        config.validate().unwrap();
        let bounds = bounds(&points);
        SigningSearch::new(
            points,
            bounds,
            2.3,
            config,
            WorkBudget::new(config.max_work),
        )
        .unwrap()
    }

    fn bounds(points: &[CombinationPoint]) -> RegionBounds {
        let extent = |coordinate: fn(&CombinationPoint) -> f64| {
            points
                .iter()
                .map(coordinate)
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
                    (min.min(value), max.max(value))
                })
        };
        let (left, right) = extent(|point| point.position.x());
        let (top, bottom) = extent(|point| point.position.y());
        RegionBounds {
            min: Point::new(left - 0.5, top - 0.5).unwrap(),
            max: Point::new(right + 0.5, bottom + 0.5).unwrap(),
        }
    }

    fn baseline(points: &[CombinationPoint], ids: Vec<usize>, padding: u32) -> Candidate {
        let shapes = prepare_grid_shapes(
            points,
            &[FeatureCombination {
                feature_ids: ids,
                score: 1.0,
                regions: vec![0],
            }],
            2.3,
            &GridShapeConfig {
                uncertainty_steps: padding,
                max_variants: u64::MAX,
                max_work: 1_000_000,
            },
        )
        .unwrap();
        let shape = &shapes.shapes()[0];
        grid_candidate(
            &shape.baseline,
            shape.phase,
            config(1, padding).max_descriptor_bytes,
        )
        .unwrap()
    }

    #[test]
    fn a_single_circle_searches_every_subset_in_the_exact_size_range_and_enrolled_frame() {
        let mut source = points(6);
        // Equal priorities exercise source-ID ties; zero remains searchable.
        source[0].priority = source[1].priority;
        let mut ranked = source.clone();
        ranked.sort_unstable_by(|a, b| b.priority.total_cmp(&a.priority).then(a.id.cmp(&b.id)));
        let mut expected = (1_usize..1 << source.len())
            .filter_map(|mask| {
                let ranks = (0..source.len())
                    .filter(|&i| mask & (1 << i) != 0)
                    .collect::<Vec<_>>();
                if !(2..=4).contains(&ranks.len()) {
                    return None;
                }
                let score = 0.1
                    + 0.9
                        * ranks
                            .iter()
                            .map(|&rank| ranked[rank].priority.powi(2))
                            .sum::<f64>()
                        / ranks.len() as f64;
                Some((ranks, score))
            })
            .collect::<Vec<_>>();
        expected.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        assert_eq!(expected.len(), 50);
        assert!(expected.iter().any(|(ranks, _)| ranks == &[4, 5]));

        for padding in 0..=3 {
            source.reverse();
            let mut actual = search(source.clone(), config(2, padding));
            assert_eq!(actual.subsets.regions.len(), 1);
            assert!(actual.last_feature_ids().is_empty());
            for (ranks, _) in &expected {
                let mut ids: Vec<_> = ranks.iter().map(|&rank| ranked[rank].id).collect();
                ids.sort_unstable();
                let candidate = actual.next().unwrap().unwrap();
                assert_eq!(actual.last_feature_ids(), ids);
                assert_eq!(
                    candidate.as_bytes(),
                    baseline(&source, ids, padding).as_bytes()
                );
            }
            let last = actual.last_feature_ids().to_vec();
            assert!(actual.next().is_none());
            assert!(actual.next().is_none());
            assert_eq!(actual.last_feature_ids(), last);
        }
    }

    #[test]
    fn merges_the_exhaustive_regional_union_in_priority_order_without_global_fallback() {
        for priority_case in 0..3 {
            let mut source = points(8);
            for (index, point) in source.iter_mut().enumerate() {
                point.priority = match priority_case {
                    0 => point.priority,
                    1 => (index / 2) as f64 / 4.0,
                    _ => 0.0,
                };
            }
            source.sort_unstable_by(|a, b| b.priority.total_cmp(&a.priority).then(a.id.cmp(&b.id)));
            let controls = SigningSearchConfig {
                radius_units: 10.0,
                ..config(2, 1)
            };
            let circles = regions::circular_regions(
                &source,
                bounds(&source),
                2.3,
                controls.radius_units,
                4,
                &mut WorkBudget::new(1_000_000),
            )
            .unwrap();
            assert!(circles.len() > 1);
            let mut expected = BTreeSet::new();
            let mut occurrences = 0;
            for circle in &circles {
                for mask in 1_usize..1 << circle.indices.len() {
                    if !(2..=4).contains(&(mask.count_ones() as usize)) {
                        continue;
                    }
                    let ranks: Vec<_> = circle
                        .indices
                        .iter()
                        .enumerate()
                        .filter_map(|(offset, &rank)| (mask & (1 << offset) != 0).then_some(rank))
                        .collect();
                    expected.insert(ranks);
                    occurrences += 1;
                }
            }
            let score = |ranks: &[usize]| {
                0.1 + 0.9
                    * (ranks
                        .iter()
                        .map(|&rank| source[rank].priority.powi(2))
                        .sum::<f64>()
                        / ranks.len() as f64)
            };
            let mut expected: Vec<_> = expected.into_iter().collect();
            expected.sort_unstable_by(|a, b| score(b).total_cmp(&score(a)).then(a.cmp(b)));
            assert!(
                expected.len() < occurrences,
                "overlap must exercise deduplication"
            );
            // A global search of 8 points would yield C(8,2)+C(8,3)+C(8,4)=154.
            assert!(
                expected.len() < 154,
                "remote-only subsets must stay excluded"
            );
            let mut shuffled = source.clone();
            shuffled.reverse();
            let mut actual = search(shuffled, controls);
            assert_eq!(actual.points(), source);
            assert_eq!(actual.regions().len(), circles.len());
            for (diagnostic, circle) in actual.regions().iter().zip(&circles) {
                assert_eq!(diagnostic.id, circle.id);
                assert_eq!(diagnostic.center, circle.center);
                assert_eq!(diagnostic.initial_radius, circle.initial_radius);
                assert_eq!(diagnostic.radius, circle.radius);
                assert_eq!(
                    diagnostic.feature_ids,
                    circle
                        .indices
                        .iter()
                        .map(|&index| source[index].id)
                        .collect::<Vec<_>>()
                );
            }
            for ranks in &expected {
                let mut ids: Vec<_> = ranks.iter().map(|&rank| source[rank].id).collect();
                ids.sort_unstable();
                let candidate = actual.next().unwrap().unwrap();
                assert_eq!(actual.last_feature_ids(), ids);
                assert_eq!(candidate.as_bytes(), baseline(&source, ids, 1).as_bytes());
            }
            assert!(actual.next().is_none());
            assert_eq!(actual.subsets.seen.len(), expected.len());
            assert!(actual.next().is_none());
        }
    }

    #[test]
    fn available_points_cap_the_maximum_without_removing_coordinate_collisions() {
        let mut source = points(3);
        source[1].position = source[0].position;
        source[2].position = source[0].position;
        let mut actual = search(source.clone(), config(2, 1));
        let candidates = std::iter::from_fn(|| actual.next())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(candidates.len(), 4); // C(3,2) + C(3,3), not positional variants.
        assert_eq!(
            candidates[0].as_bytes(),
            baseline(&source, vec![3, 2], 1).as_bytes()
        );
        assert!(candidates.iter().any(
            |candidate| candidate.as_bytes() == baseline(&source, vec![1, 2, 3], 1).as_bytes()
        ));
    }

    #[test]
    fn first_result_does_not_enumerate_the_regional_space_or_enrollment_neighbors() {
        let mut large = search(
            points(299),
            SigningSearchConfig {
                max_work: 20_000,
                ..config(3, 1)
            },
        );
        assert!(large.next().unwrap().is_ok());

        // Enrollment's 7^24 neighbor assignments exceed u64. Signing only
        // prepares the one baseline, retaining the same one-row/column padding.
        let source = points(24);
        assert!(matches!(
            prepare_grid_shapes(
                &source,
                &[FeatureCombination {
                    feature_ids: source.iter().map(|point| point.id).collect(),
                    score: 1.0,
                    regions: vec![0],
                }],
                2.3,
                &GridShapeConfig {
                    uncertainty_steps: 1,
                    max_variants: u64::MAX,
                    max_work: 1_000_000,
                }
            ),
            Err(GridShapeError::CountOverflow)
        ));
        let mut actual = search(source, config(24, 1));
        assert!(actual.next().unwrap().is_ok());
        assert!(actual.next().is_none());
    }

    #[test]
    fn cumulative_work_limit_and_late_descriptor_failure_terminate_the_stream() {
        let source = points(8);
        let controls = SigningSearchConfig {
            radius_units: 10.0,
            ..config(2, 1)
        };
        let mut measured = search(source.clone(), controls);
        assert!(measured.subsets.regions.len() > 1);
        measured.next().unwrap().unwrap();
        let first_work = controls.max_work - measured.work.remaining;
        let mut bounded = search(
            source,
            SigningSearchConfig {
                max_work: first_work,
                ..controls
            },
        );
        assert!(bounded.next().unwrap().is_ok());
        let last = bounded.last_feature_ids().to_vec();
        assert!(matches!(
            bounded
                .next()
                .unwrap()
                .unwrap_err()
                .downcast_ref::<CombinationError>(),
            Some(CombinationError::WorkLimit)
        ));
        assert!(bounded.next().is_none());
        assert_eq!(bounded.last_feature_ids(), last);

        let mut source = points(3);
        for point in &mut source {
            point.priority = 1.0;
        }
        let mut limited = search(
            source,
            SigningSearchConfig {
                max_descriptor_bytes: NonZeroUsize::new(97).unwrap(),
                ..config(2, 1)
            },
        );
        assert!(limited.next().unwrap().is_ok()); // Two records fit exactly.
        let last = limited.last_feature_ids().to_vec();
        assert!(matches!(
            limited.next().unwrap().unwrap_err().downcast_ref::<Error>(),
            Some(Error::LimitExceeded(Resource::CandidateBytes))
        ));
        assert!(limited.next().is_none());
        assert_eq!(limited.last_feature_ids(), last);
    }

    #[test]
    fn rejects_invalid_controls_and_insufficient_observed_points() {
        for invalid in [
            SigningSearchConfig {
                min_points: 0,
                ..config(2, 1)
            },
            SigningSearchConfig {
                min_points: usize::MAX,
                ..config(2, 1)
            },
            SigningSearchConfig {
                max_work: 0,
                ..config(2, 1)
            },
            SigningSearchConfig {
                enrollment_padding: u32::MAX,
                ..config(2, 1)
            },
            SigningSearchConfig {
                radius_units: 0.0,
                ..config(2, 1)
            },
            SigningSearchConfig {
                radius_units: f64::NAN,
                ..config(2, 1)
            },
            SigningSearchConfig {
                radius_units: f64::INFINITY,
                ..config(2, 1)
            },
        ] {
            assert!(matches!(
                invalid.validate(),
                Err(CombinationError::InvalidConfig(_))
            ));
        }
        assert!(matches!(
            SigningSearch::new(
                points(2),
                bounds(&points(2)),
                2.3,
                config(3, 1),
                WorkBudget::new(100)
            ),
            Err(CombinationError::InsufficientPoints {
                required: 3,
                available: 2
            })
        ));
    }
}
