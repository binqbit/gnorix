//! Source-verified short branches and narrow loops, with observed raster provenance.

mod continuation;
mod footprint;
mod provenance;
mod selection;
mod source;

pub use source::{LoopEvidence, SourceDecision, SourcePolicy, SpurEvidence};

use std::collections::VecDeque;

use provenance::{map_nodes, node_ownership, point_groups, retain_origins};

use super::{Capture, FingerprintError, LocalPalette, NodeKind, RidgeGraph, RidgeMap, graph};
use crate::biometrics::geometry::Point;

const NO_SOURCE: usize = usize::MAX;
const MAX_WORK: usize = 200_000_000;

#[derive(Default)]
pub(super) struct Work {
    estimated: f64,
}

impl Work {
    pub(super) fn charge(&mut self, amount: f64) -> Result<(), FingerprintError> {
        self.estimated += amount;
        if !self.estimated.is_finite() || self.estimated > MAX_WORK as f64 {
            return Err(FingerprintError::WorkLimit);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanupOrigin {
    pub source_edges: Vec<usize>,
    pub source_cycles: Vec<usize>,
    pub source_routes: Vec<usize>,
}

/// An actually observed route inside one original contracted junction cluster.
#[derive(Clone, Debug, PartialEq)]
pub struct JunctionRoute {
    pub source_node: usize,
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpurRemoval {
    pub source_edge: usize,
    pub attachment_node: usize,
    pub terminal_node: usize,
    pub arc_length_px: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpurAssessment {
    pub source_edge: usize,
    pub evidence: SpurEvidence,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoopAssessment {
    pub source_edges: [usize; 2],
    pub source_nodes: [usize; 2],
    pub evidence: LoopEvidence,
    /// The discarded original arm. The other arm remains observed geometry.
    pub removed_edge: Option<usize>,
}

/// The input graph for smoothing and gap repair. All source IDs in this object
/// refer to the unchanged observed graph; later contour IDs refer to `graph`.
#[derive(Clone, Debug, PartialEq)]
pub struct RidgeCleanup {
    pub cursor_diameter_px: f64,
    pub source_policy: SourcePolicy,
    pub graph: RidgeGraph,
    /// A split original junction may supply more than one retained node.
    pub node_sources: Vec<usize>,
    pub node_pixels: Vec<Vec<Point>>,
    pub source_node_pixels: Vec<Vec<Point>>,
    pub edge_origins: Vec<CleanupOrigin>,
    pub cycle_origins: Vec<CleanupOrigin>,
    pub removed_spurs: Vec<SpurRemoval>,
    pub spur_assessments: Vec<SpurAssessment>,
    pub loops: Vec<LoopAssessment>,
    /// Original eligible edges retained because they comprise the whole component.
    pub retained_ambiguous_edges: Vec<usize>,
    /// Short original terminal branches with missing original source/palette support.
    pub retained_unobserved_edges: Vec<usize>,
    /// Short branches whose original source does not confirm a termination.
    pub retained_unconfirmed_edges: Vec<usize>,
    pub junction_routes: Vec<JunctionRoute>,
}

pub(super) struct Cleaned {
    pub result: RidgeCleanup,
    pub skeleton: RidgeMap,
}

pub(super) fn clean(
    capture: &Capture,
    palette: &LocalPalette,
    skeleton: &RidgeMap,
    observed: &graph::TracedGraph,
    unit_px: f64,
) -> Result<Cleaned, FingerprintError> {
    validate_work(skeleton, observed, unit_px)?;
    validate_source(capture, palette, skeleton)?;
    let source_policy = SourcePolicy::for_unit(unit_px);
    let selected = selection::assess(capture, palette, skeleton, observed, unit_px)?;
    rebuild(skeleton, observed, selected, source_policy, unit_px)
}

/// The feature assessor supplies the original component labels and an evidence
/// backbone with this ending's inspected terminal arc excluded. This reuses the
/// same source cap and curved-continuation guard without changing cleanup.
#[allow(clippy::too_many_arguments)]
pub(super) fn feature_ending(
    capture: &Capture,
    palette: &LocalPalette,
    points: &[Point],
    backbone_graph: &RidgeGraph,
    components: &[usize],
    component: usize,
    unit: f64,
    work: &mut Work,
) -> Result<SpurEvidence, FingerprintError> {
    selection::charge_source(work, &[points], unit)?;
    let mut evidence = source::verify_spur(capture, palette, points, unit)?;
    if evidence.decision == SourceDecision::Confirmed {
        work.charge(backbone_graph.edges.len() as f64)?;
        let excluded: Vec<_> = backbone_graph
            .edges
            .iter()
            .map(|edge| edge.points.len() < 2)
            .collect();
        let backbone = continuation::Backbone::new(backbone_graph, &excluded, components, &[]);
        evidence.decision =
            continuation::check(capture, palette, points, unit, &backbone, component, work)?;
    }
    Ok(evidence)
}

fn rebuild(
    skeleton: &RidgeMap,
    observed: &graph::TracedGraph,
    selected: selection::Selection,
    source_policy: SourcePolicy,
    unit_px: f64,
) -> Result<Cleaned, FingerprintError> {
    let source_node_pixels = point_groups(skeleton, &observed.node_pixels)?;
    let removed = selected.removed_edges(observed.graph.edges.len());
    if !removed.iter().any(|&removed| removed) {
        return Ok(identity(
            skeleton,
            observed,
            source_node_pixels,
            selected,
            source_policy,
            unit_px,
        ));
    }

    let ownership = node_ownership(skeleton, &observed.node_pixels);
    let retained = retained_pixels(skeleton, observed, &ownership, &removed);
    let traced = graph::trace_subset(skeleton, &retained, &ownership)?;
    let mut result = RidgeCleanup {
        cursor_diameter_px: unit_px,
        source_policy,
        node_sources: map_nodes(&traced.node_pixels, &ownership)?,
        node_pixels: point_groups(skeleton, &traced.node_pixels)?,
        source_node_pixels,
        graph: traced.graph,
        edge_origins: Vec::new(),
        cycle_origins: Vec::new(),
        removed_spurs: selected.removed_spurs,
        spur_assessments: selected.spur_assessments,
        loops: selected.loops,
        retained_ambiguous_edges: selected.ambiguous,
        retained_unobserved_edges: selected.unobserved,
        retained_unconfirmed_edges: selected.unconfirmed,
        junction_routes: Vec::new(),
    };
    retain_origins(skeleton, &observed.graph, &ownership, &mut result)?;
    let mut cleaned_skeleton = skeleton.clone();
    cleaned_skeleton.ridges = retained;
    Ok(Cleaned {
        result,
        skeleton: cleaned_skeleton,
    })
}

fn validate_source(
    capture: &Capture,
    palette: &LocalPalette,
    skeleton: &RidgeMap,
) -> Result<(), FingerprintError> {
    let count = capture.pixels().len();
    if capture.width() as usize != skeleton.width
        || capture.height() as usize != skeleton.height
        || palette.usable.len() != count
        || palette.ridge_gray.len() != count
        || palette.valley_gray.len() != count
    {
        return Err(FingerprintError::InvalidMap(
            "cleanup source dimensions do not match skeleton",
        ));
    }
    Ok(())
}

fn validate_work(
    skeleton: &RidgeMap,
    observed: &graph::TracedGraph,
    unit: f64,
) -> Result<(), FingerprintError> {
    if !unit.is_finite() || unit <= 0.0 {
        return Err(FingerprintError::InvalidConfig(
            "spur cleanup needs a finite positive ridge width",
        ));
    }
    let points = observed
        .graph
        .edges
        .iter()
        .map(|edge| edge.points.len())
        .chain(observed.graph.cycles.iter().map(|cycle| cycle.points.len()))
        .try_fold(0_usize, usize::checked_add)
        .ok_or(FingerprintError::WorkLimit)?;
    // Each source pixel and original path station participates in bounded scans;
    // cluster trimming visits each link at most twice, never once per spur.
    skeleton
        .ridges
        .len()
        .checked_add(points)
        .and_then(|count| count.checked_mul(16))
        .filter(|&work| work <= MAX_WORK)
        .ok_or(FingerprintError::WorkLimit)?;
    Ok(())
}

fn retained_pixels(
    skeleton: &RidgeMap,
    observed: &graph::TracedGraph,
    ownership: &[usize],
    removed_edges: &[bool],
) -> Vec<bool> {
    let mut retained = skeleton.ridges.clone();
    for (id, edge) in observed.graph.edges.iter().enumerate() {
        if !removed_edges[id] {
            continue;
        }
        for &point in &edge.points {
            let index = pixel(skeleton, point);
            // Node-cluster pixels belong to the surviving observed tree. Trim
            // unused cluster twigs separately after protecting retained ports.
            if ownership[index] == NO_SOURCE
                || observed.graph.nodes[ownership[index]].kind != NodeKind::Junction
            {
                retained[index] = false;
            }
        }
    }
    let mut protected = vec![false; retained.len()];
    for (id, edge) in observed.graph.edges.iter().enumerate() {
        if !removed_edges[id] {
            protected[pixel(skeleton, edge.points[0])] = true;
            protected[pixel(skeleton, *edge.points.last().unwrap())] = true;
        }
    }
    let mut degree = vec![0_u8; retained.len()];
    let mut leaves = VecDeque::new();
    for (node, pixels) in observed.node_pixels.iter().enumerate() {
        if observed.graph.nodes[node].kind != NodeKind::Junction {
            continue;
        }
        for &index in pixels {
            degree[index] = graph::neighbors(skeleton, index)
                .into_iter()
                .flatten()
                .filter(|&next| retained[next] && ownership[next] == node)
                .count() as u8;
            if !protected[index] && degree[index] <= 1 {
                leaves.push_back(index);
            }
        }
    }
    // Trim unused internal tree twigs only. Terminal edges are selected once
    // from the original graph; newly shortened paths never become candidates.
    while let Some(index) = leaves.pop_front() {
        if !retained[index] || protected[index] {
            continue;
        }
        retained[index] = false;
        for next in graph::neighbors(skeleton, index).into_iter().flatten() {
            if retained[next] && ownership[next] == ownership[index] {
                degree[next] -= 1;
                if !protected[next] && degree[next] <= 1 {
                    leaves.push_back(next);
                }
            }
        }
    }
    retained
}

fn identity(
    skeleton: &RidgeMap,
    observed: &graph::TracedGraph,
    pixels: Vec<Vec<Point>>,
    selected: selection::Selection,
    source_policy: SourcePolicy,
    unit: f64,
) -> Cleaned {
    Cleaned {
        result: RidgeCleanup {
            cursor_diameter_px: unit,
            source_policy,
            graph: observed.graph.clone(),
            node_sources: (0..observed.graph.nodes.len()).collect(),
            node_pixels: pixels.clone(),
            source_node_pixels: pixels,
            edge_origins: (0..observed.graph.edges.len())
                .map(|id| CleanupOrigin {
                    source_edges: vec![id],
                    ..CleanupOrigin::default()
                })
                .collect(),
            cycle_origins: (0..observed.graph.cycles.len())
                .map(|id| CleanupOrigin {
                    source_cycles: vec![id],
                    ..CleanupOrigin::default()
                })
                .collect(),
            removed_spurs: selected.removed_spurs,
            spur_assessments: selected.spur_assessments,
            loops: selected.loops,
            retained_ambiguous_edges: selected.ambiguous,
            retained_unobserved_edges: selected.unobserved,
            retained_unconfirmed_edges: selected.unconfirmed,
            junction_routes: Vec::new(),
        },
        skeleton: skeleton.clone(),
    }
}

fn pixel(skeleton: &RidgeMap, point: Point) -> usize {
    point.y() as usize * skeleton.width + point.x() as usize
}
fn segments(points: &[Point], closed: bool) -> impl Iterator<Item = (Point, Point)> + '_ {
    points
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .chain(closed.then(|| (*points.last().unwrap(), points[0])))
}
fn distance(a: Point, b: Point) -> f64 {
    (a.x() - b.x()).hypot(a.y() - b.y())
}

#[cfg(test)]
mod tests;
