//! Deterministic regional subsets of assessed, surviving fingerprint features.

mod enumeration;
mod projection;
mod regions;
mod search;

use std::{collections::BTreeMap, error::Error, fmt};

use super::{FeatureKind, Fingerprint, FingerprintError, Point};

pub use search::{SigningSearchConfig, signing_candidates};

/// Selection controls. Applications choose these explicitly; they are not
/// biometric matching tolerances or a cryptographic security estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombinationConfig {
    pub min_points: usize,
    pub pool_multiplier: usize,
    /// Initial circle radius in measured mean dark-ridge widths R.
    pub radius_units: f64,
    /// Maximum distinct subsets selected in each region, including coverage.
    pub combinations_per_region: usize,
    /// Cumulative source-mask visits and selection/search work entries.
    pub max_work: usize,
}

impl CombinationConfig {
    pub fn validate(&self) -> Result<(), CombinationError> {
        if self.min_points == 0 || self.pool_multiplier == 0 {
            return Err(CombinationError::InvalidConfig(
                "point counts must be nonzero",
            ));
        }
        let target = self
            .min_points
            .checked_mul(self.pool_multiplier)
            .ok_or(CombinationError::InvalidConfig("point pool size overflows"))?;
        if !self.radius_units.is_finite() || self.radius_units <= 0.0 {
            return Err(CombinationError::InvalidConfig(
                "circle radius in ridge units must be finite and positive",
            ));
        }
        if self.combinations_per_region < target - self.min_points + 1 {
            return Err(CombinationError::InvalidConfig(
                "combination budget must cover every point in the configured pool",
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
    /// Scored features within the final inclusive circle, before pool ranking.
    pub available_points: usize,
    /// Original feature IDs ordered by descending priority, then ascending ID.
    pub pool: Vec<usize>,
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
    /// All surviving candidates with known priority, including measured zero.
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
                    "need at least {required} scored features; only {available} available"
                )
            }
            Self::WorkLimit => f.write_str("feature combination work limit exceeded"),
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

/// Selects bounded, quality-ranked subsets in adaptive ridge-scale circles.
///
/// Unknown scores are excluded; known zero remains eligible. Every selected
/// pool point occurs in at least one subset. Stronger pool points occur no less
/// often than weaker ones within that region. Regions can overlap; their union
/// does not imply this incidence ordering globally. Identical input and controls
/// reproduce the result; registration and stable candidate encoding are separate.
/// Exceeding work limits returns an error, never a partial successful result.
pub fn generate_combinations(
    fingerprint: &Fingerprint,
    config: &CombinationConfig,
) -> Result<FeatureCombinations, CombinationError> {
    config.validate()?;
    let mut work = WorkBudget::new(config.max_work);
    let (points, unscored_features) = collect_points(fingerprint, &mut work)?;
    if points.len() < config.min_points {
        return Err(CombinationError::InsufficientPoints {
            required: config.min_points,
            available: points.len(),
        });
    }
    let bounds = regions::pattern_bounds(fingerprint, &points, &mut work)?;
    let mut regions = regions::select(
        &points,
        bounds,
        fingerprint.scale.unit_px,
        config,
        &mut work,
    )?;
    let combinations = collect_combinations(&points, &mut regions, config, &mut work)?;

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
    fingerprint: &Fingerprint,
    work: &mut WorkBudget,
) -> Result<(Vec<CombinationPoint>, usize), CombinationError> {
    charge_projection(fingerprint, work)?;
    let retained = projection::retained_candidates(fingerprint)
        .map_err(CombinationError::InvalidFingerprint)?;
    let unscored = retained
        .iter()
        .filter(|feature| feature.priority.is_none())
        .count();
    let mut points = Vec::new();
    for feature in retained {
        let Some(priority) = feature.priority else {
            continue;
        };
        if !priority.is_finite() || !(0.0..=1.0).contains(&priority) {
            return Err(CombinationError::InvalidFingerprint(
                FingerprintError::InvalidTopology("feature priority must be in [0, 1]"),
            ));
        }
        let position = feature.position;
        if position.x() < 0.0
            || position.y() < 0.0
            || position.x() >= fingerprint.binary.width() as f64
            || position.y() >= fingerprint.binary.height() as f64
        {
            return Err(CombinationError::InvalidFingerprint(
                FingerprintError::InvalidTopology("feature lies outside the source image"),
            ));
        }
        points.push(CombinationPoint {
            id: feature.id,
            kind: feature.kind,
            position: feature.position,
            priority,
        });
    }
    Ok((points, unscored))
}

fn charge_projection(
    fingerprint: &Fingerprint,
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    work.spend(fingerprint.feature_strength.candidates.len())?;
    let contours = &fingerprint.contours;
    for graph in [&fingerprint.graph, &contours.cleanup.graph, &contours.graph] {
        work.spend(graph.nodes.len())?;
        work.spend(graph.edges.len())?;
        work.spend(graph.cycles.len())?;
    }
    work.spend(contours.cleanup.node_sources.len())?;
    work.spend(contours.node_sources.len())?;
    for origin in &contours.cleanup.cycle_origins {
        work.spend(1)?;
        work.spend(origin.source_cycles.len())?;
    }
    for origin in &contours.cycle_origins {
        work.spend(1)?;
        work.spend(origin.source_cycles.len())?;
    }
    Ok(())
}

fn collect_combinations(
    points: &[CombinationPoint],
    regions: &mut [CombinationRegion],
    config: &CombinationConfig,
    work: &mut WorkBudget,
) -> Result<Vec<FeatureCombination>, CombinationError> {
    let priorities: BTreeMap<_, _> = points
        .iter()
        .map(|point| (point.id, point.priority))
        .collect();
    let mut unique: BTreeMap<Vec<usize>, FeatureCombination> = BTreeMap::new();
    for region in regions {
        let scores: Vec<_> = region.pool.iter().map(|id| priorities[id]).collect();
        let subsets = enumeration::enumerate(
            &scores,
            config.min_points,
            config.combinations_per_region,
            work,
        )?;
        region.combination_count = subsets.len();
        for subset in subsets {
            work.spend(subset.ranks.len())?;
            let mut ids: Vec<_> = subset.ranks.iter().map(|&rank| region.pool[rank]).collect();
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
    }
    let mut combinations: Vec<_> = unique.into_values().collect();
    combinations.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.feature_ids.cmp(&b.feature_ids))
    });
    Ok(combinations)
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

    #[test]
    fn smaller_global_pool_uses_every_available_point_and_exhausts_subsets() {
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
            pool_multiplier: 2,
            radius_units: 1.0,
            combinations_per_region: 12,
            max_work: 10_000,
        };
        config.validate().unwrap();
        let mut work = WorkBudget::new(config.max_work);
        let mut regions = regions::select(&points, bounds, 1.0, &config, &mut work).unwrap();
        let region_count = regions.len();
        assert!(
            regions
                .iter()
                .all(|region| region.pool == [0, 1, 2, 3] && region.radius > region.initial_radius)
        );
        let combinations = collect_combinations(&points, &mut regions, &config, &mut work).unwrap();
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
