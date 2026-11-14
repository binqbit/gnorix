//! Deterministic regional subsets of assessed, surviving fingerprint features.

mod allocation;
mod enumeration;
mod local;
mod regions;
mod search;
mod spatial;

use std::{collections::BTreeMap, error::Error, fmt};

use super::{FeatureKind, FingerprintError, FingerprintFeatures, Point};

pub use search::{
    SigningRegion, SigningSearch, SigningSearchConfig, prepare_signing_search, signing_candidates,
};
pub use spatial::{SpatialPolicy, satisfies_spatial_requirements};

/// Selection controls. Applications choose these explicitly; they are not
/// biometric matching tolerances or a cryptographic security estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombinationConfig {
    /// Inclusive subset size range, shared with signing.
    pub min_points: usize,
    pub max_points: usize,
    /// Inclusive minimum final feature priority, between zero and one.
    pub min_priority: f64,
    pub spatial: SpatialPolicy,
    /// Initial circle radius in measured mean dark-ridge widths R.
    pub radius_units: f64,
    /// Cumulative feature validation and selection/search work entries.
    pub max_work: usize,
}

impl CombinationConfig {
    pub fn validate(&self) -> Result<(), CombinationError> {
        validate_min_priority(self.min_priority)?;
        self.spatial.validate()?;
        validate_point_range(
            self.min_points,
            self.max_points,
            self.spatial.min_anchor_points,
        )?;
        if !self.radius_units.is_finite() || self.radius_units <= 0.0 {
            return Err(CombinationError::InvalidConfig(
                "circle radius in ridge units must be finite and positive",
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

#[derive(Clone, Debug, PartialEq)]
pub struct CombinationPoint {
    /// Original feature ID within this capture, not a cross-capture identity.
    pub id: usize,
    pub kind: FeatureKind,
    pub position: Point,
    pub priority: f64,
}

/// Confirmed pattern pixel-edge bounds, extended to cover eligible point centers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionBounds {
    pub min: Point,
    pub max: Point,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CombinationRegion {
    /// Row-major position in the capture's adaptive rectangular center grid.
    pub id: usize,
    pub center: Point,
    pub initial_radius: f64,
    pub radius: f64,
    /// Original feature IDs ordered by descending priority, then ascending ID.
    pub feature_ids: Vec<usize>,
    pub combination_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureCombination {
    /// Ascending original IDs. A point can occur only once in a subset.
    pub feature_ids: Vec<usize>,
    /// Mean of 0.1 + 0.9 * priority² over the subset.
    pub score: f64,
    /// All regions selecting this same point set, in ascending order.
    pub regions: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureCombinations {
    pub bounds: RegionBounds,
    /// Surviving candidates whose known priority meets the configured minimum.
    pub points: Vec<CombinationPoint>,
    /// Type/degree-preserved source identities whose priority is unavailable.
    pub unscored_features: usize,
    pub regions: Vec<CombinationRegion>,
    /// Unique subsets, descending score with ascending feature IDs as ties.
    pub combinations: Vec<FeatureCombination>,
    pub work_used: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombinationError {
    InvalidConfig(&'static str),
    InvalidFingerprint(FingerprintError),
    PatternUnavailable,
    InsufficientPoints { required: usize, available: usize },
    UncoverablePoint { region: usize, feature_id: usize },
    WorkLimit,
}

impl fmt::Display for CombinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(reason) => write!(f, "invalid combination config: {reason}"),
            Self::InvalidFingerprint(error) => write!(f, "invalid combination input: {error}"),
            Self::PatternUnavailable => f.write_str("no confirmed pattern for regional centers"),
            Self::InsufficientPoints {
                required,
                available,
            } => {
                write!(
                    f,
                    "need at least {required} eligible features; only {available} available"
                )
            }
            Self::WorkLimit => f.write_str("feature combination work limit exceeded"),
            Self::UncoverablePoint { region, feature_id } => write!(
                f,
                "feature {feature_id} in region {region} cannot enter any valid spatial combination"
            ),
        }
    }
}

impl Error for CombinationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidFingerprint(error) => Some(error),
            _ => None,
        }
    }
}

/// Selects automatically bounded, quality-ranked subsets in adaptive ridge-scale circles.
///
/// Unknown scores and priorities below the configured minimum are excluded.
/// Each region targets three times its eligible point count in valid subsets.
/// Their strongest included point assigns high (>0.9), middle (0.5..=0.9) or
/// low (<0.5) priority, allocated in a 2:1:1 cycle with quality ordering within
/// each band. Empty or exhausted bands redistribute their share. Additional
/// point-coverage witnesses can exceed the target and change those proportions.
/// Compact witnesses around each regional feature supplement that selection so
/// partial observations can retain a local group without the strongest distant
/// features. They use the same subset-size and separated-anchor requirements.
/// Every eligible regional point must occur in at least one valid subset; otherwise
/// the complete operation fails with its region and source ID. Spatial anchors
/// can occur more often than stronger, compact neighbors. Valid subsets retain
/// their quality ordering. Regions can overlap. Identical input and controls
/// reproduce the result; registration and stable candidate encoding are separate.
/// Exceeding work limits returns an error, never a partial successful result.
pub fn generate_combinations(
    fingerprint: &FingerprintFeatures,
    config: &CombinationConfig,
) -> Result<FeatureCombinations, CombinationError> {
    config.validate()?;
    let mut work = WorkBudget::new(config.max_work);
    let (points, unscored_features) = collect_points(fingerprint, config.min_priority, &mut work)?;
    if points.len() < config.min_points.max(config.spatial.min_anchor_points) {
        return Err(CombinationError::InsufficientPoints {
            required: config.min_points.max(config.spatial.min_anchor_points),
            available: points.len(),
        });
    }
    let bounds = regions::pattern_bounds(fingerprint, &points, &mut work)?;
    let spatial_pixels = config.spatial.pixels(fingerprint.unit_px)?;
    let selected = regions::select(
        &points,
        bounds,
        fingerprint.unit_px,
        config,
        spatial_pixels,
        &mut work,
    )?;
    let (regions, combinations) = collect_combinations(selected, &mut work)?;

    Ok(FeatureCombinations {
        bounds,
        points,
        unscored_features,
        regions,
        combinations,
        work_used: config.max_work - work.remaining,
    })
}

fn collect_points(
    fingerprint: &FingerprintFeatures,
    min_priority: f64,
    work: &mut WorkBudget,
) -> Result<(Vec<CombinationPoint>, usize), CombinationError> {
    work.spend(fingerprint.features.len())?;
    fingerprint
        .validate()
        .map_err(CombinationError::InvalidFingerprint)?;
    let mut unscored = 0;
    let mut points = Vec::new();
    for feature in &fingerprint.features {
        let Some(priority) = feature.priority else {
            unscored += 1;
            continue;
        };
        if priority >= min_priority {
            points.push(CombinationPoint {
                id: feature.id,
                kind: feature.kind,
                position: feature.position,
                priority,
            });
        }
    }
    work.spend(points.len())?;
    points.sort_unstable_by_key(|point| point.id);
    Ok((points, unscored))
}

fn validate_point_range(
    minimum: usize,
    maximum: usize,
    anchors: usize,
) -> Result<(), CombinationError> {
    if minimum == 0 || minimum > maximum {
        return Err(CombinationError::InvalidConfig(
            "point range must be nonzero and ordered",
        ));
    }
    if anchors > maximum {
        return Err(CombinationError::InvalidConfig(
            "minimum anchor count exceeds maximum points per combination",
        ));
    }
    Ok(())
}

fn validate_min_priority(min_priority: f64) -> Result<(), CombinationError> {
    if !min_priority.is_finite() || !(0.0..=1.0).contains(&min_priority) {
        return Err(CombinationError::InvalidConfig(
            "minimum feature priority must be finite and in [0, 1]",
        ));
    }
    Ok(())
}

fn collect_combinations(
    selected: Vec<regions::RegionSelection>,
    work: &mut WorkBudget,
) -> Result<(Vec<CombinationRegion>, Vec<FeatureCombination>), CombinationError> {
    let mut regions = Vec::new();
    let mut unique: BTreeMap<Vec<usize>, FeatureCombination> = BTreeMap::new();
    for selection in selected {
        let region = selection.region;
        for subset in selection.subsets {
            work.spend(subset.ranks.len())?;
            let mut ids: Vec<_> = subset
                .ranks
                .iter()
                .map(|&rank| region.feature_ids[rank])
                .collect();
            ids.sort_unstable();
            if let Some(existing) = unique.get_mut(&ids) {
                existing.regions.push(region.id);
            } else {
                unique.insert(
                    ids.clone(),
                    FeatureCombination {
                        feature_ids: ids,
                        score: subset.score,
                        regions: vec![region.id],
                    },
                );
            }
        }
        regions.push(region);
    }
    let mut combinations: Vec<_> = unique.into_values().collect();
    combinations.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.feature_ids.cmp(&b.feature_ids))
    });
    Ok((regions, combinations))
}

pub(super) struct WorkBudget {
    remaining: usize,
}

impl WorkBudget {
    pub(super) fn new(limit: usize) -> Self {
        Self { remaining: limit }
    }

    pub(super) fn spend(&mut self, amount: usize) -> Result<(), CombinationError> {
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or(CombinationError::WorkLimit)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permissive_policy() -> SpatialPolicy {
        SpatialPolicy {
            min_anchor_points: 1,
            min_anchor_distance_units: 0.0,
        }
    }

    fn threshold_fingerprint() -> (FingerprintFeatures, Vec<usize>) {
        let features = [
            Some(0.0),
            Some(0.49),
            Some(0.5),
            Some(0.75),
            Some(1.0),
            None,
        ]
        .into_iter()
        .enumerate()
        .map(|(id, priority)| super::super::AnalyzedFeature {
            id,
            kind: FeatureKind::Ending,
            position: Point::new(10.0 + id as f64 * 12.0, 40.0).unwrap(),
            priority,
        })
        .collect();
        let fingerprint = FingerprintFeatures::new(
            96,
            96,
            6.0,
            Some(RegionBounds {
                min: Point::new(-0.5, -0.5).unwrap(),
                max: Point::new(95.5, 95.5).unwrap(),
            }),
            features,
        )
        .unwrap();
        (fingerprint, (0..5).collect())
    }

    fn threshold_config(min_priority: f64) -> CombinationConfig {
        CombinationConfig {
            min_points: 1,
            max_points: 2,
            min_priority,
            spatial: permissive_policy(),
            radius_units: 1000.0,
            max_work: 1_000_000,
        }
    }

    fn signing_config(controls: CombinationConfig) -> SigningSearchConfig {
        SigningSearchConfig {
            min_points: controls.min_points,
            max_points: controls.max_points,
            min_priority: controls.min_priority,
            spatial: permissive_policy(),
            radius_units: controls.radius_units,
            uncertainty_steps: 0,
            max_work: controls.max_work,
            max_descriptor_bytes: std::num::NonZeroUsize::new(4096).unwrap(),
        }
    }

    #[test]
    fn anchor_count_must_fit_the_explicit_subset_maximum() {
        let mut config = threshold_config(0.0);
        config.spatial.min_anchor_points = 3;
        assert!(matches!(
            config.validate(),
            Err(CombinationError::InvalidConfig(_))
        ));
        config.max_points = 3;
        assert!(config.validate().is_ok());
        for distance in [-0.1, f64::NAN, f64::INFINITY] {
            let mut invalid = config;
            invalid.spatial.min_anchor_distance_units = distance;
            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    fn insufficient_anchor_points_are_reported_before_pattern_layout() {
        let (mut fingerprint, _) = threshold_fingerprint();
        fingerprint.pattern_bounds = None;
        let mut config = threshold_config(1.0);
        config.min_points = 2;
        config.max_points = 3;
        config.spatial.min_anchor_points = 3;
        let expected = CombinationError::InsufficientPoints {
            required: 3,
            available: 1,
        };
        assert_eq!(
            generate_combinations(&fingerprint, &config).unwrap_err(),
            expected
        );
        let mut signing = signing_config(config);
        signing.spatial.min_anchor_points = 3;
        assert!(
            matches!(prepare_signing_search(&fingerprint,&signing),Err(error) if error==expected)
        );
    }

    #[test]
    fn priority_cutoff_is_inclusive_for_enrollment_and_signing() {
        let (fingerprint, ids) = threshold_fingerprint();
        let mut unscored = None;
        for (minimum, expected) in [(0.0, &ids[..]), (0.5, &ids[2..]), (1.0, &ids[4..])] {
            let controls = threshold_config(minimum);
            let selected = generate_combinations(&fingerprint, &controls).unwrap();
            assert_eq!(
                selected
                    .points
                    .iter()
                    .map(|point| point.id)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                *unscored.get_or_insert(selected.unscored_features),
                selected.unscored_features
            );
            assert!(selected.combinations.iter().all(|combination| {
                combination
                    .feature_ids
                    .iter()
                    .all(|id| expected.contains(id))
            }));
            let search = prepare_signing_search(&fingerprint, &signing_config(controls)).unwrap();
            let mut eligible = search
                .points()
                .iter()
                .map(|point| point.id)
                .collect::<Vec<_>>();
            eligible.sort_unstable();
            assert_eq!(eligible, expected);
            assert!(
                search
                    .regions()
                    .iter()
                    .all(|region| { region.feature_ids.iter().all(|id| expected.contains(id)) })
            );
        }
    }

    #[test]
    fn thresholded_regions_never_refill_with_weaker_points_and_search_preserves_the_set() {
        let (mut fingerprint, ids) = threshold_fingerprint();
        let mut controls = threshold_config(0.5);
        controls.min_points = 2;
        controls.max_points = 4;
        controls.radius_units = 1.0;
        let selected = generate_combinations(&fingerprint, &controls).unwrap();
        let eligible = &ids[2..];
        assert!(
            selected
                .regions
                .iter()
                .any(|region| region.radius > region.initial_radius)
        );
        assert!(selected.regions.iter().all(|region| {
            region.feature_ids.len() == eligible.len()
                && region.feature_ids.iter().all(|id| eligible.contains(id))
        }));
        let mut search = prepare_signing_search(&fingerprint, &signing_config(controls)).unwrap();
        let mut searched = std::collections::BTreeSet::new();
        while let Some(candidate) = search.next() {
            candidate.unwrap();
            assert!(searched.insert(search.last_feature_ids().to_vec()));
        }
        assert_eq!(
            searched,
            selected
                .combinations
                .iter()
                .map(|c| c.feature_ids.clone())
                .collect()
        );
        assert!(
            selected
                .combinations
                .windows(2)
                .all(|pair| pair[0].score >= pair[1].score)
        );

        controls.min_priority = 1.0;
        let expected = CombinationError::InsufficientPoints {
            required: 2,
            available: 1,
        };
        assert_eq!(
            generate_combinations(&fingerprint, &controls).unwrap_err(),
            expected
        );
        assert!(matches!(
            prepare_signing_search(&fingerprint, &signing_config(controls)),
            Err(error) if error == expected
        ));

        for feature in &mut fingerprint.features {
            feature.priority = Some(0.0);
        }
        fingerprint.pattern_bounds = None;
        // Insufficient eligible data is known before any attempt to lay out
        // circles, even when the capture also lacks usable pattern bounds.
        assert!(matches!(
            prepare_signing_search(&fingerprint, &signing_config(controls)),
            Err(CombinationError::InsufficientPoints {
                required: 2,
                available: 0
            })
        ));
    }

    #[test]
    fn invalid_thresholds_and_invalid_low_priority_data_are_rejected() {
        for minimum in [-0.01, 1.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let controls = threshold_config(minimum);
            assert!(matches!(
                controls.validate(),
                Err(CombinationError::InvalidConfig(_))
            ));
            assert!(matches!(
                signing_config(controls).validate(),
                Err(CombinationError::InvalidConfig(_))
            ));
        }
        let (mut fingerprint, ids) = threshold_fingerprint();
        let controls = threshold_config(1.0);
        let original = fingerprint.features[ids[0]].position;
        fingerprint.features[ids[0]].position = Point::new(-1.0, 0.0).unwrap();
        assert!(matches!(
            generate_combinations(&fingerprint, &controls),
            Err(CombinationError::InvalidFingerprint(_))
        ));
        fingerprint.features[ids[0]].position = original;
        fingerprint.features[ids[0]].priority = Some(f64::NAN);
        assert!(matches!(
            prepare_signing_search(&fingerprint, &signing_config(controls)),
            Err(CombinationError::InvalidFingerprint(_))
        ));
    }

    #[test]
    fn fewer_points_than_maximum_retains_every_available_point() {
        let points: Vec<_> = (0..4)
            .map(|id| CombinationPoint {
                id,
                kind: FeatureKind::Ending,
                position: Point::new(id as f64, 0.0).unwrap(),
                priority: 1.0 - id as f64 / 3.0,
            })
            .collect();
        let bounds = RegionBounds {
            min: Point::new(-0.5, -0.5).unwrap(),
            max: Point::new(5.5, 5.5).unwrap(),
        };
        let config = CombinationConfig {
            min_points: 3,
            max_points: 6,
            min_priority: 0.0,
            spatial: permissive_policy(),
            radius_units: 1.0,
            max_work: 10_000,
        };
        config.validate().unwrap();
        let mut work = WorkBudget::new(config.max_work);
        let selected = regions::select(
            &points,
            bounds,
            1.0,
            &config,
            permissive_policy().pixels(1.0).unwrap(),
            &mut work,
        )
        .unwrap();
        let (regions, combinations) = collect_combinations(selected, &mut work).unwrap();
        let region_count = regions.len();
        assert!(
            regions
                .iter()
                .all(|region| region.feature_ids == [0, 1, 2, 3]
                    && region.radius > region.initial_radius)
        );
        assert_eq!(combinations.len(), 5);
        assert!(regions.iter().all(|region| region.combination_count == 5));
        assert!(
            combinations
                .iter()
                .all(|item| item.regions == (0..region_count).collect::<Vec<_>>())
        );
        assert!(
            combinations
                .iter()
                .any(|item| item.feature_ids == [0, 1, 2, 3])
        );
    }
}
