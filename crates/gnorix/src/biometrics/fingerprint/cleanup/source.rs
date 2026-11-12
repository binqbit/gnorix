//! Conservative source checks for short branches and narrow split/rejoin paths.
//!
//! These are explicit image-processing rules, not calibrated biometric scores.
//! Source support is independent of orientation/coherence and clarity filtering.

use super::super::{Capture, FingerprintError, LocalPalette, photometry::sample_darkness};
use crate::biometrics::geometry::Point;

mod profiles;
mod sampling;

use profiles::{common_normal, section, summarize_loop};
use sampling::{
    Bounds, Path, check_work, observed_area, offset, section_fractions, steps, validate,
};

#[cfg(test)]
use profiles::Section;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDecision {
    Confirmed,
    MissingSupport,
    PossibleContinuation,
    SeparateRidges,
    ImageUnsupported,
    AmbiguousGeometry,
}

/// Source-pixel sampling controls derived from the one measured ridge width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourcePolicy {
    pub dark_midpoint: f64,
    pub sample_spacing_px: f64,
    pub tangent_radius_px: f64,
    pub relative_width_tolerance: f64,
    pub max_ridge_width_px: f64,
    pub max_backbone_extension_px: f64,
    pub terminal_lookahead_px: f64,
    pub terminal_barrier_width_px: f64,
    pub terminal_barrier_depth_px: f64,
    pub loop_flank_search_px: f64,
    pub loop_min_valley_length_px: f64,
}

impl SourcePolicy {
    pub(in crate::biometrics::fingerprint) fn for_unit(unit: f64) -> Self {
        let relative_width_tolerance = 0.2;
        let max_ridge_width_px = (1.0 + relative_width_tolerance) * unit;
        Self {
            dark_midpoint: 0.5,
            sample_spacing_px: 0.5,
            tangent_radius_px: unit / 2.0,
            relative_width_tolerance,
            max_ridge_width_px,
            max_backbone_extension_px: 2.0 * unit,
            terminal_lookahead_px: max_ridge_width_px,
            terminal_barrier_width_px: unit,
            terminal_barrier_depth_px: unit / 4.0,
            loop_flank_search_px: max_ridge_width_px,
            loop_min_valley_length_px: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpurEvidence {
    pub decision: SourceDecision,
    pub terminal_darkness: Option<f64>,
    pub light_barrier_distance_px: Option<f64>,
    pub min_barrier_lightness: Option<f64>,
}

impl SpurEvidence {
    pub(super) fn missing_support() -> Self {
        Self::empty(SourceDecision::MissingSupport)
    }

    fn empty(decision: SourceDecision) -> Self {
        Self {
            decision,
            terminal_darkness: None,
            light_barrier_distance_px: None,
            min_barrier_lightness: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoopEvidence {
    pub decision: SourceDecision,
    pub max_width_px: Option<f64>,
    pub mean_interior_darkness: Option<f64>,
    pub max_valley_lightness: Option<f64>,
    pub longest_valley_length_px: Option<f64>,
    pub min_flank_lightness: Option<f64>,
    pub arm_darkness: Option<[f64; 2]>,
    pub arm_center_distance_px: Option<[f64; 2]>,
    pub preferred_arm: Option<usize>,
}

impl LoopEvidence {
    pub(super) fn missing_support() -> Self {
        Self::empty(SourceDecision::MissingSupport)
    }

    fn empty(decision: SourceDecision) -> Self {
        Self {
            decision,
            max_width_px: None,
            mean_interior_darkness: None,
            max_valley_lightness: None,
            longest_valley_length_px: None,
            min_flank_lightness: None,
            arm_darkness: None,
            arm_center_distance_px: None,
            preferred_arm: None,
        }
    }
}

/// `points` runs from the attachment port toward the terminal pixel.
pub(super) fn verify_spur(
    capture: &Capture,
    palette: &LocalPalette,
    points: &[Point],
    unit: f64,
) -> Result<SpurEvidence, FingerprintError> {
    validate(capture, palette, unit, points.len())?;
    let Some(path) = Path::new(points) else {
        return Ok(SpurEvidence::empty(SourceDecision::AmbiguousGeometry));
    };
    let policy = SourcePolicy::for_unit(unit);
    let Some((tip, direction)) = terminal_frame_from_path(&path, unit) else {
        return Ok(SpurEvidence::empty(SourceDecision::AmbiguousGeometry));
    };
    let normal = [-direction[1], direction[0]];
    let half_width = policy.terminal_barrier_width_px / 2.0;
    let mut bounds = Bounds::of(points);
    for forward in [0.0, policy.terminal_lookahead_px] {
        for side in [-half_width, half_width] {
            bounds.include(offset(tip, direction, forward, normal, side));
        }
    }
    if !observed_area(capture, palette, bounds)? {
        return Ok(SpurEvidence::missing_support());
    }

    let rows = steps(policy.terminal_lookahead_px, policy.sample_spacing_px)?;
    let columns = steps(policy.terminal_barrier_width_px, policy.sample_spacing_px)?;
    check_work((rows + 1).checked_mul(columns + 1))?;
    let mut lightness = Vec::with_capacity(rows + 1);
    for row in 0..=rows {
        let forward = policy.terminal_lookahead_px * row as f64 / rows as f64;
        let mut darkest = f64::NEG_INFINITY;
        for column in 0..=columns {
            let side =
                policy.terminal_barrier_width_px * column as f64 / columns as f64 - half_width;
            let point = offset(tip, direction, forward, normal, side);
            let Some(darkness) = sample_darkness(capture, palette, point) else {
                return Ok(SpurEvidence::missing_support());
            };
            darkest = darkest.max(darkness);
        }
        lightness.push(1.0 - darkest);
    }

    let terminal_darkness = sample_darkness(capture, palette, tip);
    let step = policy.terminal_lookahead_px / rows as f64;
    let mut run_start = None;
    let mut minimum = f64::INFINITY;
    for (row, &light) in lightness.iter().enumerate() {
        if light <= policy.dark_midpoint {
            run_start = None;
            minimum = f64::INFINITY;
            continue;
        }
        let start = *run_start.get_or_insert(row);
        minimum = minimum.min(light);
        if (row - start) as f64 * step >= policy.terminal_barrier_depth_px {
            return Ok(SpurEvidence {
                decision: SourceDecision::Confirmed,
                terminal_darkness,
                light_barrier_distance_px: Some(start as f64 * step),
                min_barrier_lightness: Some(minimum),
            });
        }
    }
    Ok(SpurEvidence {
        terminal_darkness,
        ..SpurEvidence::empty(SourceDecision::PossibleContinuation)
    })
}

/// Both observed arms must run between the same junctions in the same direction.
/// Correspondence uses equal fractions of traversed arc length, preserving raw
/// paths; acceptance never manufactures a third, interpolated output contour.
pub(super) fn verify_loop(
    capture: &Capture,
    palette: &LocalPalette,
    first: &[Point],
    second: &[Point],
    unit: f64,
) -> Result<LoopEvidence, FingerprintError> {
    validate(
        capture,
        palette,
        unit,
        first.len().saturating_add(second.len()),
    )?;
    let (Some(first_path), Some(second_path)) = (Path::new(first), Path::new(second)) else {
        return Ok(LoopEvidence::empty(SourceDecision::AmbiguousGeometry));
    };
    let policy = SourcePolicy::for_unit(unit);
    let length = (first_path.length() + second_path.length()) / 2.0;
    let fractions = section_fractions(&first_path, &second_path, policy.sample_spacing_px)?;
    let mut bounds = Bounds::of(first);
    for &point in second {
        bounds.include(point);
    }
    // Projection onto the common normal can leave the raw arm bounding box.
    // Half the maximum arm width plus the full outward search covers it.
    bounds.expand(policy.max_ridge_width_px / 2.0 + policy.loop_flank_search_px);
    if !observed_area(capture, palette, bounds)? {
        return Ok(LoopEvidence::missing_support());
    }
    // The allowed width plus both outward searches bound every cross-section.
    let transverse = steps(
        policy.max_ridge_width_px + 2.0 * policy.loop_flank_search_px,
        policy.sample_spacing_px,
    )?;
    check_work(
        fractions
            .len()
            .checked_mul(transverse + 1)
            .and_then(|work| work.checked_mul(3)),
    )?;

    let mut measurements = Vec::with_capacity(fractions.len());
    let mut max_width = 0.0_f64;
    for &fraction in &fractions {
        let a = first_path.at(first_path.length() * fraction);
        let b = second_path.at(second_path.length() * fraction);
        let Some(normal) = common_normal(&first_path, &second_path, fraction, policy) else {
            return Ok(LoopEvidence::empty(SourceDecision::AmbiguousGeometry));
        };
        let width = ((b.x() - a.x()) * normal[0] + (b.y() - a.y()) * normal[1]).abs();
        max_width = max_width.max(width);
        if width >= policy.max_ridge_width_px {
            return Ok(LoopEvidence {
                max_width_px: Some(max_width),
                ..LoopEvidence::empty(SourceDecision::AmbiguousGeometry)
            });
        }
        let Some(section) = section(capture, palette, a, b, normal, policy)? else {
            return Ok(LoopEvidence::missing_support());
        };
        measurements.push(section);
    }
    let positions: Vec<_> = fractions
        .iter()
        .map(|&fraction| fraction * length)
        .collect();
    Ok(summarize_loop(&measurements, &positions, max_width, policy))
}

pub(super) fn terminal_frame(points: &[Point], unit: f64) -> Option<(Point, [f64; 2])> {
    terminal_frame_from_path(&Path::new(points)?, unit)
}

fn terminal_frame_from_path(path: &Path<'_>, unit: f64) -> Option<(Point, [f64; 2])> {
    let tip = path.at(path.length());
    let behind = path.at((path.length() - SourcePolicy::for_unit(unit).tangent_radius_px).max(0.0));
    Some((tip, sampling::direction(behind, tip)?))
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
