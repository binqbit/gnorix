//! Global, lazy signing search over the observed points of a fresh capture.

use std::num::NonZeroUsize;

use super::{
    CombinationError, CombinationPoint, WorkBudget, collect_points, enumeration::RankedSubsets,
};
use crate::{
    biometrics::{
        Candidate, Candidates,
        fingerprint::{Fingerprint, FingerprintError, shapes},
    },
    error::AlgorithmError,
};

/// Signing searches all retained, scored points, independent of enrollment's
/// nine regions. Applications must supply the enrolled minimum and padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SigningSearchConfig {
    pub min_points: usize,
    /// Fixed frame padding used during enrollment, not a new neighbor search.
    pub enrollment_padding: u32,
    /// Cumulative projection, frontier, geometry entries and descriptor bytes.
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

/// Produces one canonical baseline candidate for each global point subset of
/// size `min_points..=min(2 * min_points, available_points)`.
///
/// Subsets use the existing mean `0.1 + 0.9 * priority²` ranking. Equal point
/// priorities use ascending source IDs; equally scored subsets use that rank
/// order. Known zero priorities remain eligible, while unknown scores do not.
/// No coverage seeds, regional pool cap or positional perturbations are added.
/// The same observed subset retains enrollment's frame, phase and fixed padding.
///
/// Preparation owns the projected points. Each request generates only the next
/// candidate, so the caller can stop immediately at a Merkle membership match.
/// Work or geometry errors are yielded once, then the stream is exhausted.
pub fn signing_candidates(
    fingerprint: &Fingerprint,
    config: &SigningSearchConfig,
) -> Result<Candidates<'static>, CombinationError> {
    config.validate()?;
    let spacing = fingerprint.scale.unit_px;
    if !spacing.is_finite() || spacing <= 0.0 {
        return Err(CombinationError::InvalidFingerprint(
            FingerprintError::ScaleUnavailable("ridge width must be finite and positive"),
        ));
    }
    let mut work = WorkBudget::new(config.max_work);
    let (points, _) = collect_points(fingerprint, &mut work)?;
    let mut search = SigningSearch::new(points, spacing, *config, work)?;
    Ok(Box::new(std::iter::from_fn(move || search.next())))
}

struct SigningSearch {
    points: Vec<CombinationPoint>,
    priorities: Vec<f64>,
    subsets: RankedSubsets,
    spacing: f64,
    config: SigningSearchConfig,
    work: WorkBudget,
    finished: bool,
}

impl SigningSearch {
    fn new(
        mut points: Vec<CombinationPoint>,
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
        work.spend(points.len())?;
        let priorities = points
            .iter()
            .map(|point| point.priority)
            .collect::<Vec<_>>();
        let maximum = (2 * config.min_points).min(points.len());
        let subsets = RankedSubsets::new(&priorities, config.min_points, maximum, &mut work)?;
        Ok(Self {
            points,
            priorities,
            subsets,
            spacing,
            config,
            work,
            finished: false,
        })
    }

    fn next(&mut self) -> Option<Result<Candidate, AlgorithmError>> {
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

    fn advance(&mut self) -> Result<Option<Candidate>, AlgorithmError> {
        let Some(subset) = self.subsets.next(&self.priorities, &mut self.work)? else {
            return Ok(None);
        };
        self.work.spend(subset.ranks.len())?;
        let mut selected = subset
            .ranks
            .iter()
            .map(|&rank| &self.points[rank])
            .collect::<Vec<_>>();
        selected.sort_unstable_by_key(|point| point.id);
        self.work.spend(selected.len())?;
        let positions = selected
            .iter()
            .map(|point| point.position)
            .collect::<Vec<_>>();
        let (candidate, used) = shapes::baseline_candidate(
            &positions,
            self.spacing,
            self.config.enrollment_padding,
            self.work.remaining,
            self.config.max_descriptor_bytes,
        )?;
        self.work.spend(used)?;
        Ok(Some(candidate))
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
        SigningSearch::new(points, 2.3, config, WorkBudget::new(config.max_work)).unwrap()
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
    fn searches_every_global_subset_in_the_exact_size_range_and_enrolled_frame() {
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
            for (ranks, _) in &expected {
                let ids = ranks.iter().map(|&rank| ranked[rank].id).collect();
                assert_eq!(
                    actual.next().unwrap().unwrap().as_bytes(),
                    baseline(&source, ids, padding).as_bytes()
                );
            }
            assert!(actual.next().is_none());
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
    fn first_result_does_not_enumerate_the_global_space_or_enrollment_neighbors() {
        let mut large = search(
            points(299),
            SigningSearchConfig {
                max_work: 10_000,
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
        let source = points(5);
        let controls = config(2, 1);
        let mut measured = search(source.clone(), controls);
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
        assert!(matches!(
            bounded
                .next()
                .unwrap()
                .unwrap_err()
                .downcast_ref::<CombinationError>(),
            Some(CombinationError::WorkLimit)
        ));
        assert!(bounded.next().is_none());

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
        assert!(matches!(
            limited.next().unwrap().unwrap_err().downcast_ref::<Error>(),
            Some(Error::LimitExceeded(Resource::CandidateBytes))
        ));
        assert!(limited.next().is_none());
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
        ] {
            assert!(matches!(
                invalid.validate(),
                Err(CombinationError::InvalidConfig(_))
            ));
        }
        assert!(matches!(
            SigningSearch::new(points(2), 2.3, config(3, 1), WorkBudget::new(100)),
            Err(CombinationError::InsufficientPoints {
                required: 3,
                available: 2
            })
        ));
    }
}
