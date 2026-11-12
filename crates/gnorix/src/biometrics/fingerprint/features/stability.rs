//! Threshold-only repeatability of the same observed graph features.

mod matching;

use super::super::{ExtractionConfig, FingerprintError, Preprocessed, RidgeGraph, graph, thinning};

/// One actual stress graph; feature IDs are its nodes followed by its cycles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StressCase {
    /// Fraction of the full local dark/light reference spread.
    pub directional_margin: f64,
    pub min_coherence: f64,
    pub node_count: usize,
    pub cycle_count: usize,
}

/// Diagnostic correspondence tolerances, not biometric matching guarantees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StabilityPolicy {
    pub max_position_shift_px: f64,
    pub arm_radius_px: f64,
    pub max_arm_angle_radians: f64,
    pub max_arm_shape_error_px: f64,
    pub max_isolated_shift_px: f64,
    pub cycle_relative_tolerance: f64,
}

impl StabilityPolicy {
    fn for_unit(unit: f64) -> Result<Self, FingerprintError> {
        if !unit.is_finite() || unit <= 0.0 {
            return Err(FingerprintError::InvalidConfig(
                "feature stability requires a positive scale",
            ));
        }
        Ok(Self {
            max_position_shift_px: unit,
            arm_radius_px: unit,
            max_arm_angle_radians: std::f64::consts::PI / 6.0,
            max_arm_shape_error_px: unit / 4.0,
            max_isolated_shift_px: unit / 4.0,
            cycle_relative_tolerance: 0.2,
        })
    }
}

pub(super) struct StabilityAssessment {
    pub cases: [StressCase; 8],
    pub matches: Vec<[Option<usize>; 8]>,
    pub policy: StabilityPolicy,
}

pub(super) fn assess(
    prepared: &Preprocessed,
    observed: &RidgeGraph,
    unit: f64,
    config: &ExtractionConfig,
) -> Result<StabilityAssessment, FingerprintError> {
    config.validate()?;
    let policy = StabilityPolicy::for_unit(unit)?;
    let mut work = matching::Work::default();
    let reference = matching::describe(observed, policy, &mut work)?;
    let mut cases = stress_cases(config.min_coherence);
    let mut matches = vec![[None; 8]; reference.len()];
    for (index, case) in cases.iter_mut().enumerate() {
        let binary = prepared
            .response
            .binarize(case.directional_margin, case.min_coherence)?;
        let skeleton = thinning::thin(&binary)?;
        let variant = graph::trace(&skeleton)?;
        case.node_count = variant.nodes.len();
        case.cycle_count = variant.cycles.len();
        let descriptors = matching::describe(&variant, policy, &mut work)?;
        let assignments = matching::correspond(&reference, &descriptors, policy, &mut work)?;
        for (candidate, assignment) in matches.iter_mut().zip(assignments) {
            candidate[index] = assignment;
        }
    }
    Ok(StabilityAssessment {
        cases,
        matches,
        policy,
    })
}

fn stress_cases(coherence: f64) -> [StressCase; 8] {
    // The default is exactly ±10%. Near bounds a minimum 0.02 step and
    // one-sided substitute retain three distinct, valid actual thresholds.
    let step = (0.1 * coherence).max(0.02);
    let mut low = (coherence - step).max(0.0);
    let mut high = (coherence + step).min(1.0);
    if low == coherence {
        low = (coherence + 2.0 * step).min(1.0);
    }
    if high == coherence {
        high = (coherence - 2.0 * step).max(0.0);
    }
    let mut levels = [low, coherence, high];
    levels.sort_by(f64::total_cmp);
    let empty = StressCase {
        directional_margin: 0.0,
        min_coherence: 0.0,
        node_count: 0,
        cycle_count: 0,
    };
    let mut cases = [empty; 8];
    let mut index = 0;
    for margin in [-0.05, 0.0, 0.05] {
        for threshold in levels {
            if margin == 0.0 && threshold == coherence {
                continue;
            }
            cases[index] = StressCase {
                directional_margin: margin,
                min_coherence: threshold,
                ..empty
            };
            index += 1;
        }
    }
    cases
}

#[cfg(test)]
mod tests;
