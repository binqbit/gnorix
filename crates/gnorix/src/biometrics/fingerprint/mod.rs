//! Per-capture ridge extraction, point subsets and geometric grid variants.
//!
//! This module exposes observed structure and candidate encodings, not validated
//! identities. Endings can result from missing signal. Extraction measurements
//! use source-image pixels; measured ridge spacing is not an inferred DPI.
//! Final grid figures and positional variants use integer node indices.

mod analysis;
mod clarity;
mod cleanup;
mod combinations;
mod curve;
mod enhancement;
mod features;
mod full_scale;
mod graph;
mod map;
mod photometry;
mod pipeline;
mod quality;
mod raster;
mod repair;
mod scale;
mod shapes;
mod smoothing;
mod thinning;

use std::{error::Error, fmt};

use super::{Capture, geometry::Point};

pub use analysis::{AnalyzedFeature, FingerprintFeatures};
pub use clarity::ClarityMap;
pub use cleanup::{
    CleanupOrigin, JunctionRoute, LoopAssessment, LoopEvidence, RidgeCleanup, SourceDecision,
    SourcePolicy, SpurAssessment, SpurEvidence, SpurRemoval,
};
pub use combinations::{
    CombinationConfig, CombinationError, CombinationPoint, CombinationRegion, FeatureCombination,
    FeatureCombinations, RegionBounds, SigningRegion, SigningSearch, SigningSearchConfig,
    SpatialPolicy, SpatialRequirements, derive_spatial_requirements, generate_combinations,
    prepare_signing_search, satisfies_spatial_requirements, signing_candidates,
};
pub use enhancement::LocalPalette;
pub use features::{
    ContextArm, ContextStop, FeatureArm, FeatureAssessment, FeatureContext, FeatureKind,
    FeatureNeighborhood, FeaturePolicy, FeatureReason, FeatureStrength, StabilityPolicy,
    StressCase,
};
pub use full_scale::{FullScale, MeasurementCoverage, RidgeObservation};
pub use graph::{NodeKind, RidgeCycle, RidgeEdge, RidgeGraph, RidgeNode};
pub use map::RidgeMap;
pub use pipeline::{ContourReconstruction, Fingerprint, analyze, estimate_scale, extract};
pub use quality::WidthConsistency;
pub use repair::{GapEvidence, GapJoin, GapPolicy, GapStatus, PathOrigin};
pub use scale::{BootstrapScale, RidgeSpan, ScaleProfile};
pub use shapes::{
    GridBaselineConfig, GridBaselines, GridCell, GridCollision, GridShape, GridShapeConfig,
    GridShapeError, GridShapeFrame, GridShapes, GridVariant, GridVariants, grid_candidate,
    prepare_grid_baselines, prepare_grid_shapes,
};
pub use smoothing::{SmoothingAdjustment, SmoothingControls, SmoothingRejection};

/// Image-processing controls, independent of biometric matching tolerances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtractionConfig {
    /// Minimum local standard deviation on grayscale values divided by 255.
    pub min_contrast: f64,
    /// Minimum structure-tensor coherence, between zero and one.
    pub min_coherence: f64,
    /// Maximum number of source pixels accepted in one extraction.
    pub max_pixels: usize,
    /// Initial bootstrap-grid locations; does not cap fallback or full traversal.
    pub bootstrap_grid_locations: usize,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            min_contrast: 0.035,
            min_coherence: 0.2,
            max_pixels: 4_000_000,
            bootstrap_grid_locations: 4096,
        }
    }
}

impl ExtractionConfig {
    fn validate(&self) -> Result<(), FingerprintError> {
        if !self.min_contrast.is_finite() || !(0.0..=1.0).contains(&self.min_contrast) {
            return Err(FingerprintError::InvalidConfig(
                "contrast must be in [0, 1]",
            ));
        }
        if !self.min_coherence.is_finite() || !(0.0..=1.0).contains(&self.min_coherence) {
            return Err(FingerprintError::InvalidConfig(
                "coherence must be in [0, 1]",
            ));
        }
        if self.max_pixels == 0 {
            return Err(FingerprintError::InvalidConfig(
                "pixel limit must be nonzero",
            ));
        }
        if self.bootstrap_grid_locations == 0 {
            return Err(FingerprintError::InvalidConfig(
                "bootstrap grid must contain at least one location",
            ));
        }
        Ok(())
    }
}

/// Sampling and smoothing in the same units as the input path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveConfig {
    pub sample_spacing: f64,
    pub smoothing_sigma: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnSample {
    pub arc_fraction: f64,
    /// Unwrapped tangent relative to the initial directed tangent.
    pub angle_radians: f64,
}

/// Geometric measurements; the profile retains path traversal direction.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveGeometry {
    pub arc_length: f64,
    pub chord_length: f64,
    /// Halfway along the raw path, not the center of a fitted circle.
    pub midpoint: Point,
    pub turning: Vec<TurnSample>,
}

/// Traces an already one-pixel-wide skeleton, including isolated cycles.
pub fn trace_skeleton(skeleton: &RidgeMap) -> Result<RidgeGraph, FingerprintError> {
    graph::trace(skeleton)
}

/// Measures a path using uniform arc-length sampling and Gaussian smoothing.
///
/// Spacing and sigma must be finite and positive. Sampling spacing is adjusted
/// downward to cover the complete path. The Gaussian is truncated at four sigma,
/// renormalized at open ends and wrapped for closed curves. Length and midpoint
/// use the unsmoothed path. A closed curve includes its last-to-first segment,
/// has zero chord, and retains its supplied seam and directed winding.
/// Resource or numerical failures return errors, not partial measurements.
pub fn describe_curve(
    points: &[Point],
    closed: bool,
    config: CurveConfig,
) -> Result<CurveGeometry, FingerprintError> {
    curve::describe(points, closed, config)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FingerprintError {
    InvalidConfig(&'static str),
    InvalidMap(&'static str),
    InvalidCurve(&'static str),
    InvalidTopology(&'static str),
    ScaleUnavailable(&'static str),
    WorkLimit,
}

impl fmt::Display for FingerprintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(reason) => write!(f, "invalid extraction config: {reason}"),
            Self::InvalidMap(reason) => write!(f, "invalid ridge map: {reason}"),
            Self::InvalidCurve(reason) => write!(f, "invalid ridge curve: {reason}"),
            Self::InvalidTopology(reason) => write!(f, "invalid skeleton topology: {reason}"),
            Self::ScaleUnavailable(reason) => write!(f, "ridge scale unavailable: {reason}"),
            Self::WorkLimit => f.write_str("fingerprint processing work limit exceeded"),
        }
    }
}

impl Error for FingerprintError {}
