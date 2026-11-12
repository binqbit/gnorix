//! Per-capture ridge extraction and geometric measurements.
//!
//! This module exposes observed structure, not stable identities or signing
//! candidates. Endings can result from missing signal. All positions and lengths
//! are in source-image pixels; measured ridge spacing is not an inferred DPI.

mod curve;
mod enhancement;
mod full_scale;
mod graph;
mod photometry;
mod quality;
mod scale;
mod thinning;

use std::{error::Error, fmt};

use super::{Capture, geometry::Point};

pub use enhancement::LocalPalette;
pub use full_scale::{FullScale, MeasurementCoverage, RidgeObservation};
pub use quality::WidthConsistency;
pub use scale::{BootstrapScale, RidgeSpan, ScaleProfile};

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

/// A binary ridge image with an explicit usable region.
#[derive(Clone, PartialEq, Eq)]
pub struct RidgeMap {
    width: usize,
    height: usize,
    ridges: Vec<bool>,
    usable: Vec<bool>,
    boundary_origin: Vec<bool>,
}

impl RidgeMap {
    pub fn new(
        width: usize,
        height: usize,
        ridges: Vec<bool>,
        usable: Vec<bool>,
    ) -> Result<Self, FingerprintError> {
        let count =
            width
                .checked_mul(height)
                .filter(|&n| n > 0)
                .ok_or(FingerprintError::InvalidMap(
                    "dimensions must be nonzero and addressable",
                ))?;
        if ridges.len() != count || usable.len() != count {
            return Err(FingerprintError::InvalidMap(
                "pixel buffers do not match dimensions",
            ));
        }
        if ridges
            .iter()
            .zip(&usable)
            .any(|(&ridge, &valid)| ridge && !valid)
        {
            return Err(FingerprintError::InvalidMap("ridge pixels must be usable"));
        }
        let boundary_origin = boundary_contacts(width, height, &ridges, &usable);
        Ok(Self {
            width,
            height,
            ridges,
            usable,
            boundary_origin,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn ridges(&self) -> &[bool] {
        &self.ridges
    }
    pub fn usable(&self) -> &[bool] {
        &self.usable
    }
}

fn boundary_contacts(width: usize, height: usize, ridges: &[bool], usable: &[bool]) -> Vec<bool> {
    (0..ridges.len())
        .map(|index| {
            if !ridges[index] {
                return false;
            }
            let (x, y) = (index % width, index / width);
            x == 0
                || y == 0
                || x + 1 == width
                || y + 1 == height
                || ((y - 1)..=(y + 1))
                    .any(|row| ((x - 1)..=(x + 1)).any(|column| !usable[row * width + column]))
        })
        .collect()
}

impl fmt::Debug for RidgeMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RidgeMap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pixels", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Ending,
    Junction,
    Isolated,
}

/// A representative pixel of an extracted graph node, not an anatomical claim.
/// Acyclic junction clusters use an actual pixel; incident edge ports may differ.
/// Cyclic clusters retain individual vertices to preserve their genuine holes.
#[derive(Clone, Debug, PartialEq)]
pub struct RidgeNode {
    pub position: Point,
    pub kind: NodeKind,
    /// The node touches, or inherited thinning support from, the frame or an
    /// unusable region. Continuation is unknown; false is not proof of anatomy.
    pub touches_unobserved: bool,
}

/// An observed path between node clusters. No anchor-to-port line is invented.
#[derive(Clone, Debug, PartialEq)]
pub struct RidgeEdge {
    pub start: usize,
    pub end: usize,
    pub points: Vec<Point>,
}

/// An isolated closed path without a privileged biological starting point.
#[derive(Clone, Debug, PartialEq)]
pub struct RidgeCycle {
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RidgeGraph {
    pub nodes: Vec<RidgeNode>,
    pub edges: Vec<RidgeEdge>,
    pub cycles: Vec<RidgeCycle>,
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

/// Sensitive diagnostics and structure from one capture; no match score.
pub struct Fingerprint {
    /// Full-source ridge-width measurement; `unit_px` is the common length unit.
    pub scale: FullScale,
    /// Provisional pattern search used to initialize raw measurement windows.
    pub bootstrap: BootstrapScale,
    /// Thickness consistency around the full-pass arithmetic mean.
    pub width_consistency: [WidthConsistency; 3],
    /// Local dark/light reference levels measured on original grayscale.
    pub palette: LocalPalette,
    /// Intensity relative to the local dark/light midpoint, clipped to [-3, 3].
    pub normalized: Vec<f32>,
    /// Spatially denoised normalized intensity used for ridge detection.
    pub denoised: Vec<f32>,
    /// Orientation coherence in [0, 1], not a confidence probability.
    pub coherence: Vec<f32>,
    pub binary: RidgeMap,
    pub skeleton: RidgeMap,
    pub graph: RidgeGraph,
    /// One entry per graph edge, in the same order. An unresolved descriptor
    /// retains the graph path and its error rather than silently dropping it.
    pub edge_geometry: Vec<Result<CurveGeometry, FingerprintError>>,
    /// One entry per isolated cycle, in the same order.
    pub cycle_geometry: Vec<Result<CurveGeometry, FingerprintError>>,
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fingerprint")
            .field("width", &self.binary.width)
            .field("height", &self.binary.height)
            .field("biometrics", &"[REDACTED]")
            .finish()
    }
}

struct Preprocessed {
    palette: LocalPalette,
    normalized: Vec<f32>,
    denoised: Vec<f32>,
    coherence: Vec<f32>,
    map: RidgeMap,
}

/// Extracts ridge structure without a separate gap-repair or pore-removal stage.
///
/// The image scale is estimated before any scale-dependent filtering. This is a transparent native
/// processing pipeline, not an exact port of SourceAFIS or a calibrated matcher.
/// Filtering and thresholding can change connectivity; returned topology is
/// observed in the processed image. Explicitly invalid capture pixels are never
/// bridged, and the tracer preserves the resulting closed components.
pub fn extract(
    capture: &Capture,
    config: &ExtractionConfig,
) -> Result<Fingerprint, FingerprintError> {
    let bootstrap = scale::estimate(capture, config)?;
    let scale = full_scale::measure(capture, config, &bootstrap)?;
    let width_consistency = scale.width_consistency()?;
    let prepared = enhancement::preprocess(capture, config, bootstrap.ridge_period_px)?;
    let skeleton = thinning::thin(&prepared.map)?;
    let graph = graph::trace(&skeleton)?;
    let curve_config = CurveConfig {
        sample_spacing: scale.unit_px / 4.0,
        smoothing_sigma: scale.unit_px / 2.0,
    };
    let (edge_geometry, cycle_geometry) = describe_graph(&graph, curve_config);

    Ok(Fingerprint {
        scale,
        bootstrap,
        width_consistency,
        palette: prepared.palette,
        normalized: prepared.normalized,
        denoised: prepared.denoised,
        coherence: prepared.coherence,
        binary: prepared.map,
        skeleton,
        graph,
        edge_geometry,
        cycle_geometry,
    })
}

/// Traverses the raw measurement centerlines and measures original ridge widths.
/// The arithmetic mean weights each portion by its represented source length.
/// Three-ridge profiles confirm local regions; unmeasurable portions remain in
/// coverage instead of receiving an inferred width.
pub fn estimate_scale(
    capture: &Capture,
    config: &ExtractionConfig,
) -> Result<FullScale, FingerprintError> {
    config.validate()?;
    if capture.pixels().len() > config.max_pixels {
        return Err(FingerprintError::WorkLimit);
    }
    let bootstrap = scale::estimate(capture, config)?;
    full_scale::measure(capture, config, &bootstrap)
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

type CurveMeasurements = Vec<Result<CurveGeometry, FingerprintError>>;

fn describe_graph(
    graph: &RidgeGraph,
    config: CurveConfig,
) -> (CurveMeasurements, CurveMeasurements) {
    let edges = graph
        .edges
        .iter()
        .map(|edge| {
            curve::describe(
                &edge.points,
                edge.points.first() == edge.points.last(),
                config,
            )
        })
        .collect();
    let cycles = graph
        .cycles
        .iter()
        .map(|cycle| curve::describe(&cycle.points, true, config))
        .collect();
    (edges, cycles)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returning_to_a_junction_does_not_close_distinct_edge_ports() {
        let points = [(1.0, 0.0), (2.0, 1.0), (3.0, 0.0)]
            .into_iter()
            .map(|(x, y)| Point::new(x, y).unwrap())
            .collect();
        let graph = RidgeGraph {
            nodes: vec![RidgeNode {
                position: Point::new(2.0, 0.0).unwrap(),
                kind: NodeKind::Junction,
                touches_unobserved: false,
            }],
            edges: vec![RidgeEdge {
                start: 0,
                end: 0,
                points,
            }],
            cycles: vec![],
        };
        let (edges, _) = describe_graph(
            &graph,
            CurveConfig {
                sample_spacing: 0.2,
                smoothing_sigma: 0.1,
            },
        );
        assert_eq!(edges[0].as_ref().unwrap().chord_length, 2.0);
        assert!((edges[0].as_ref().unwrap().arc_length - 2.0 * 2.0_f64.sqrt()).abs() < 1e-12);
    }
}
