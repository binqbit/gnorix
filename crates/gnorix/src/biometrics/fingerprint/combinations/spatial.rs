//! Source-space diameter and separated-anchor envelope requirements.

use super::{CombinationError, FingerprintError, Point, RegionBounds, WorkBudget};
use crate::biometrics::geometry::hull::{self, HullError};

#[cfg(test)]
mod tests;

/// Source-figure policy resolved once from the enrolled bounds' shorter side.
/// At least `min_anchor_points` anchors must be pairwise separated; every other
/// feature must lie inside or on their convex hull. More anchors are allowed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialPolicy {
    /// Minimum figure diameter as a fraction of the enrolled shorter side.
    pub min_diameter_fraction: f64,
    /// Minimum anchor count, not an exact hull-vertex count.
    pub min_anchor_points: usize,
    /// Minimum distance between every anchor pair, as an enrolled-side fraction.
    pub min_anchor_separation_fraction: f64,
}

impl SpatialPolicy {
    pub fn validate(&self) -> Result<(), CombinationError> {
        for fraction in [
            self.min_diameter_fraction,
            self.min_anchor_separation_fraction,
        ] {
            if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
                return Err(CombinationError::InvalidConfig(
                    "spatial fractions must be finite and in [0, 1]",
                ));
            }
        }
        anchor_count(self.min_anchor_points)
    }
}

/// Fixed enrolled requirements in mean dark-ridge widths R.
/// Fresh captures apply their own measured R to these retained distances; they
/// do not derive smaller requirements from a crop's bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialRequirements {
    pub min_diameter_units: f64,
    pub min_anchor_points: usize,
    pub min_anchor_separation_units: f64,
}

impl SpatialRequirements {
    pub fn validate(&self) -> Result<(), CombinationError> {
        for units in [self.min_diameter_units, self.min_anchor_separation_units] {
            if !units.is_finite() || units < 0.0 {
                return Err(CombinationError::InvalidConfig(
                    "spatial distances must be finite and nonnegative",
                ));
            }
        }
        anchor_count(self.min_anchor_points)
    }

    pub(super) fn pixels(&self, unit: f64) -> Result<SpatialPixels, CombinationError> {
        self.validate()?;
        validate_unit(unit)?;
        let diameter = self.min_diameter_units * unit;
        let separation = self.min_anchor_separation_units * unit;
        if !diameter.is_finite() || !separation.is_finite() {
            return Err(CombinationError::InvalidConfig(
                "spatial distances in source pixels overflow",
            ));
        }
        if (self.min_diameter_units > 0.0 && diameter == 0.0)
            || (self.min_anchor_separation_units > 0.0 && separation == 0.0)
        {
            return Err(CombinationError::InvalidConfig(
                "positive spatial distances underflow in source pixels",
            ));
        }
        Ok(SpatialPixels {
            diameter,
            anchors: self.min_anchor_points,
            separation,
        })
    }
}

/// Resolve both fractions from the same enrolled pattern/eligible-point bounds.
pub fn derive_spatial_requirements(
    bounds: RegionBounds,
    unit: f64,
    policy: &SpatialPolicy,
) -> Result<SpatialRequirements, CombinationError> {
    policy.validate()?;
    validate_unit(unit)?;
    if bounds.max.x() < bounds.min.x() || bounds.max.y() < bounds.min.y() {
        return Err(CombinationError::InvalidConfig(
            "spatial bounds must not be inverted",
        ));
    }
    let shorter = (bounds.max.x() - bounds.min.x()).min(bounds.max.y() - bounds.min.y());
    let requirements = SpatialRequirements {
        min_diameter_units: policy.min_diameter_fraction * shorter / unit,
        min_anchor_points: policy.min_anchor_points,
        min_anchor_separation_units: policy.min_anchor_separation_fraction * shorter / unit,
    };
    requirements.validate()?;
    if shorter > 0.0
        && ((policy.min_diameter_fraction > 0.0 && requirements.min_diameter_units == 0.0)
            || (policy.min_anchor_separation_fraction > 0.0
                && requirements.min_anchor_separation_units == 0.0))
    {
        return Err(CombinationError::InvalidConfig(
            "positive spatial fractions underflow in enrolled ridge units",
        ));
    }
    Ok(requirements)
}

/// Check source positions before snapping. Returns admission and consumed work.
/// Every anchor pair must meet the separation; all other points must lie in/on
/// their convex hull. The anchor count is a minimum, not an exact count.
pub fn satisfies_spatial_requirements(
    positions: &[Point],
    unit: f64,
    requirements: &SpatialRequirements,
    max_work: usize,
) -> Result<(bool, usize), CombinationError> {
    if max_work == 0 {
        return Err(CombinationError::InvalidConfig(
            "work limit must be nonzero",
        ));
    }
    let policy = requirements.pixels(unit)?;
    let mut work = WorkBudget::new(max_work);
    let admitted = check(positions, policy, &mut work)?;
    Ok((admitted, max_work - work.remaining))
}

#[derive(Clone, Copy)]
pub(super) struct SpatialPixels {
    pub diameter: f64,
    pub anchors: usize,
    pub separation: f64,
}

impl SpatialPixels {
    pub(super) fn unconstrained(self) -> bool {
        self.diameter == 0.0 && self.anchors == 1 && self.separation == 0.0
    }
}

pub(super) fn check(
    positions: &[Point],
    policy: SpatialPixels,
    work: &mut WorkBudget,
) -> Result<bool, CombinationError> {
    if policy.unconstrained() {
        return Ok(!positions.is_empty());
    }
    if positions.len() < policy.anchors {
        return Ok(false);
    }
    if policy.diameter > 0.0 {
        let mut wide = false;
        'pairs: for first in 0..positions.len() {
            for second in first + 1..positions.len() {
                if distance(positions[first], positions[second], work)? >= policy.diameter {
                    wide = true;
                    break 'pairs;
                }
            }
        }
        if !wide {
            return Ok(false);
        }
    }
    let (boundary, used) =
        hull::convex_hull(positions, work.remaining).map_err(|error| match error {
            HullError::WorkLimit => CombinationError::WorkLimit,
            HullError::Geometry(_) => unrepresentable(),
        })?;
    work.spend(used)?;
    // Any anchor envelope containing the complete figure must contain all its
    // extreme vertices. Those mandatory anchors already prove containment.
    for (offset, &first) in boundary.iter().enumerate() {
        for &second in &boundary[offset + 1..] {
            if distance(positions[first], positions[second], work)? < policy.separation {
                return Ok(false);
            }
        }
    }
    if boundary.len() >= policy.anchors {
        return Ok(true);
    }
    let mut interior = Vec::new();
    for (index, &position) in positions.iter().enumerate() {
        work.spend(boundary.len() + 1)?;
        if boundary.contains(&index) {
            continue;
        }
        let mut compatible = true;
        for &anchor in &boundary {
            if distance(position, positions[anchor], work)? < policy.separation {
                compatible = false;
                break;
            }
        }
        if compatible {
            interior.push(index);
        }
    }
    clique(
        positions,
        &interior,
        policy.anchors - boundary.len(),
        policy.separation,
        work,
    )
}

fn clique(
    positions: &[Point],
    candidates: &[usize],
    required: usize,
    separation: f64,
    work: &mut WorkBudget,
) -> Result<bool, CombinationError> {
    let mut selected = Vec::new();
    let mut next = 0;
    loop {
        if selected.len() == required {
            return Ok(true);
        }
        if candidates.len().saturating_sub(next) < required - selected.len() {
            let Some(previous) = selected.pop() else {
                return Ok(false);
            };
            next = previous + 1;
            continue;
        }
        work.spend(1)?;
        let candidate = next;
        next += 1;
        let mut compatible = true;
        for &chosen in &selected {
            if distance(
                positions[candidates[candidate]],
                positions[candidates[chosen]],
                work,
            )? < separation
            {
                compatible = false;
                break;
            }
        }
        if compatible {
            selected.push(candidate);
        }
    }
}

pub(super) fn subset(
    positions: &[Point],
    ranks: &[usize],
    policy: SpatialPixels,
    work: &mut WorkBudget,
) -> Result<bool, CombinationError> {
    work.spend(ranks.len())?;
    let selected: Vec<_> = ranks.iter().map(|&rank| positions[rank]).collect();
    check(&selected, policy, work)
}

pub(super) fn point_subset(
    points: &[super::CombinationPoint],
    ranks: &[usize],
    policy: SpatialPixels,
    work: &mut WorkBudget,
) -> Result<bool, CombinationError> {
    work.spend(ranks.len())?;
    let selected: Vec<_> = ranks.iter().map(|&rank| points[rank].position).collect();
    check(&selected, policy, work)
}

fn distance(a: Point, b: Point, work: &mut WorkBudget) -> Result<f64, CombinationError> {
    work.spend(1)?;
    let distance = (a.x() - b.x()).hypot(a.y() - b.y());
    if distance.is_finite() {
        Ok(distance)
    } else {
        Err(unrepresentable())
    }
}
fn unrepresentable() -> CombinationError {
    CombinationError::InvalidFingerprint(FingerprintError::InvalidTopology(
        "source spatial geometry is not representable",
    ))
}
fn anchor_count(count: usize) -> Result<(), CombinationError> {
    if count == 0 {
        Err(CombinationError::InvalidConfig(
            "minimum anchor count must be nonzero",
        ))
    } else {
        Ok(())
    }
}
fn validate_unit(unit: f64) -> Result<(), CombinationError> {
    if !unit.is_finite() || unit <= 0.0 {
        Err(CombinationError::InvalidFingerprint(
            FingerprintError::ScaleUnavailable("ridge width must be finite and positive"),
        ))
    } else {
        Ok(())
    }
}
