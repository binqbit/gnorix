//! Per-candidate original-source evidence and extraction stability.
//!
//! Scores describe observed support, not biometric probabilities. Assessment
//! never removes a candidate or changes the graph used by reconstruction.

mod evidence;
mod geometry;
mod sampling;
mod stability;

pub use stability::{StabilityPolicy, StressCase};

use super::enhancement::Preprocessed;
use super::{Capture, ExtractionConfig, FingerprintError, NodeKind, RidgeGraph};
use crate::biometrics::geometry::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureKind {
    Ending,
    Bifurcation,
    Junction,
    Isolated,
    Cycle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureReason {
    MissingSourceSupport,
    InsufficientArmExtent,
    UnresolvedDirection,
    NoSeparatedBranches,
    UnconfirmedTermination,
    PossibleContinuation,
    IsolatedGeometry,
    LightRidge,
    NoInteriorLight,
    StressSensitive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureArm {
    pub edge: usize,
    /// Available outward arc, capped at the assessment horizon.
    pub length_px: f64,
    pub source_darkness: Option<f64>,
    pub dark_fraction: Option<f64>,
    /// Original local dark-to-light reference difference, divided by 255.
    pub source_contrast: Option<f64>,
    pub longest_light_gap_px: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureStrength {
    /// Original nodes first, followed by isolated cycles; never a filtered ID.
    pub id: usize,
    pub kind: FeatureKind,
    pub source_node: Option<usize>,
    pub source_cycle: Option<usize>,
    pub position: Point,
    /// Feature-specific source evidence, attenuated once by original contrast.
    /// Missing required capture or palette support is unknown, not zero.
    pub source_support: Option<f64>,
    /// Observed extent and separation/continuation. An ending additionally
    /// requires capture availability throughout its forward inspection corridor.
    pub geometry_support: Option<f64>,
    pub stability: f64,
    pub strength: Option<f64>,
    /// Fraction of required source point/cell checks with observed support.
    pub source_coverage: f64,
    /// Fraction of the requested horizon available on the weakest arm, or on
    /// an isolated cycle's perimeter; zero for an isolated point. This measures
    /// arc extent, independently of capture availability and geometry support.
    pub geometry_coverage: f64,
    /// Weakest arm's mean original contrast; ring mean or isolated point otherwise.
    pub source_contrast: Option<f64>,
    pub arms: Vec<FeatureArm>,
    pub separation: Option<f64>,
    pub geometry_separation: Option<f64>,
    pub termination: Option<f64>,
    pub continuation: Option<f64>,
    pub reasons: Vec<FeatureReason>,
    /// Variant-local IDs; None means no compatible one-to-one assignment.
    pub stress_matches: [Option<usize>; 8],
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaturePolicy {
    pub reference_width_px: f64,
    pub branch_horizon_px: f64,
    pub tangent_support_px: f64,
    pub sample_spacing_px: f64,
    pub source_search_px: f64,
    pub continuation_search_px: f64,
    pub terminal_barrier_width_px: f64,
    pub terminal_barrier_depth_px: f64,
    pub contrast_reference: f64,
    pub stability: StabilityPolicy,
    pub stress_cases: [StressCase; 8],
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureAssessment {
    pub policy: FeaturePolicy,
    pub candidates: Vec<FeatureStrength>,
}

pub(super) fn assess(
    capture: &Capture,
    prepared: &Preprocessed,
    graph: &RidgeGraph,
    unit_px: f64,
    config: &ExtractionConfig,
) -> Result<FeatureAssessment, FingerprintError> {
    let stress = stability::assess(prepared, graph, unit_px, config)?;
    let policy = assessment_policy(unit_px, config.min_contrast, &stress);
    let mut candidates = evidence::assess(capture, &prepared.palette, graph, &policy)?;
    apply_stability(&mut candidates, stress.matches)?;

    Ok(FeatureAssessment { policy, candidates })
}

fn assessment_policy(
    unit_px: f64,
    contrast_reference: f64,
    stress: &stability::StabilityAssessment,
) -> FeaturePolicy {
    let source = super::SourcePolicy::for_unit(unit_px);
    FeaturePolicy {
        reference_width_px: unit_px,
        branch_horizon_px: 2.0 * unit_px,
        tangent_support_px: source.tangent_radius_px,
        sample_spacing_px: source.sample_spacing_px,
        source_search_px: source.terminal_lookahead_px,
        continuation_search_px: 3.0 * unit_px,
        terminal_barrier_width_px: source.terminal_barrier_width_px,
        terminal_barrier_depth_px: source.terminal_barrier_depth_px,
        contrast_reference,
        stability: stress.policy,
        stress_cases: stress.cases,
    }
}

fn apply_stability(
    candidates: &mut [FeatureStrength],
    matches: Vec<[Option<usize>; 8]>,
) -> Result<(), FingerprintError> {
    if candidates.len() != matches.len() {
        return Err(FingerprintError::InvalidTopology(
            "feature stability population mismatch",
        ));
    }
    for (candidate, matches) in candidates.iter_mut().zip(matches) {
        candidate.stress_matches = matches;
        candidate.stability = matches.iter().filter(|item| item.is_some()).count() as f64 / 8.0;
        if candidate.stability < 1.0 {
            candidate.reasons.push(FeatureReason::StressSensitive);
        }
        candidate.strength = combine(
            candidate.source_support,
            candidate.geometry_support,
            candidate.stability,
        );
    }
    Ok(())
}

fn combine(source: Option<f64>, geometry: Option<f64>, stability: f64) -> Option<f64> {
    Some(source?.min(geometry?) * (0.5 + 0.5 * stability))
}

fn kind(node: NodeKind, degree: usize) -> FeatureKind {
    match node {
        NodeKind::Ending => FeatureKind::Ending,
        NodeKind::Isolated => FeatureKind::Isolated,
        NodeKind::Junction if degree == 3 => FeatureKind::Bifurcation,
        NodeKind::Junction => FeatureKind::Junction,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod control_tests;
