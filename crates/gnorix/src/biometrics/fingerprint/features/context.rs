//! Source-confirmed feature structure over bounded two- and four-width contexts.
//!
//! Context evidence is a diagnostic engineering score, not a probability. Paths
//! follow observed graph edges; an incidental node is crossed only when one
//! continuation is supported by the original image. No graph is changed.

mod ending;
mod probes;
mod tracing;

use super::super::{LocalPalette, cleanup, graph};
use super::geometry;
use super::sampling::{Sampler, Work};
use super::{Capture, FeaturePolicy, FingerprintError, NodeKind, RidgeGraph};
use probes::ArmEvidence;
use tracing::Trace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextStop {
    Horizon,
    GraphEnding,
    AmbiguousJunction,
    UnsupportedContinuation,
    MissingSource,
    ClosedPath,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextArm {
    pub edge: usize,
    /// Original edges in traversal order, each at most once.
    pub edges: Vec<usize>,
    /// Observed path extent, capped at the context horizon.
    pub length_px: f64,
    /// Mean spatial ridge/flank separation, independently of path length.
    pub ridge_separation: Option<f64>,
    /// One minus the longest unsupported interval divided by measured length.
    pub continuity: Option<f64>,
    /// Length-weighted source evidence; never exceeds the observed path extent.
    pub supported_length_px: Option<f64>,
    pub longest_unsupported_px: Option<f64>,
    pub stop: ContextStop,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeatureContext {
    /// Source-confirmed structure at the full context horizon (currently 4R).
    pub support: Option<f64>,
    /// The same context probes truncated at the local diagnostic horizon (2R).
    pub support_2r: Option<f64>,
    pub source_coverage: f64,
    pub separation: Option<f64>,
    pub termination: Option<f64>,
    /// Strongest observed continuation, either aligned or source-connected.
    pub continuation: Option<f64>,
    pub forward_continuation: Option<f64>,
    pub curved_continuation: Option<f64>,
    /// One outward arm for each original incident edge; isolated cycles have none.
    pub arms: Vec<ContextArm>,
}

pub(super) fn assess(
    capture: &Capture,
    palette: &LocalPalette,
    graph: &RidgeGraph,
    policy: &FeaturePolicy,
) -> Result<Vec<FeatureContext>, FingerprintError> {
    let mut work = Work::default();
    work.charge(graph.nodes.len().saturating_add(graph.edges.len()))?;
    let incidence = graph::incidence(graph);
    let components = graph::components(graph, &incidence);
    let mut guard_work = cleanup::Work::default();
    let mut result = Vec::with_capacity(graph.nodes.len() + graph.cycles.len());
    for node in 0..graph.nodes.len() {
        let mut sampler = Sampler::new(capture, palette, &mut work);
        let mut traces = tracing::arms(graph, &incidence, node, policy, &mut sampler)?;
        tracing::limit_shared(&mut traces, sampler.work)?;
        let mut feature = describe(&traces, policy, &mut sampler)?;
        match graph.nodes[node].kind {
            NodeKind::Ending if traces.len() == 1 => {
                let ending = ending::assess(
                    capture,
                    palette,
                    graph,
                    &components,
                    node,
                    &traces[0],
                    policy,
                    &mut sampler,
                    &mut guard_work,
                )?;
                feature.termination = ending.termination;
                feature.continuation = ending.continuation;
                feature.forward_continuation = ending.forward_continuation;
                feature.curved_continuation = ending.curved_continuation;
                let support = minimum(
                    ending.termination,
                    ending.continuation.map(|value| 1.0 - value),
                );
                feature.support = minimum(feature.support, support);
                feature.support_2r = minimum(feature.support_2r, support);
            }
            NodeKind::Junction if traces.len() >= 2 => {
                let separation =
                    probes::fork(&traces, policy.context_horizon_px, policy, &mut sampler)?;
                feature.separation = separation;
                feature.support = minimum(feature.support, separation);
                feature.support_2r = minimum(
                    feature.support_2r,
                    probes::fork(&traces, policy.branch_horizon_px, policy, &mut sampler)?,
                );
            }
            _ => {
                // A bare point or malformed degree has no resolved structure.
                if let Some(point) = graph.nodes.get(node).map(|node| node.position) {
                    let _ = sampler.sample(point)?;
                }
                feature.support = Some(0.0);
                feature.support_2r = Some(0.0);
            }
        }
        feature.source_coverage = sampler.coverage();
        if !sampler.complete()
            || traces
                .iter()
                .any(|trace| trace.stop == ContextStop::MissingSource)
        {
            feature.support = None;
            feature.support_2r = None;
        }
        result.push(feature);
    }
    for cycle in &graph.cycles {
        let mut sampler = Sampler::new(capture, palette, &mut work);
        result.push(cycle_context(&cycle.points, policy, &mut sampler)?);
    }
    Ok(result)
}

fn describe(
    traces: &[Trace],
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<FeatureContext, FingerprintError> {
    let mut feature = FeatureContext {
        support: Some(1.0),
        support_2r: Some(1.0),
        source_coverage: 0.0,
        separation: None,
        termination: None,
        continuation: None,
        forward_continuation: None,
        curved_continuation: None,
        arms: Vec::with_capacity(traces.len()),
    };
    for trace in traces {
        let broad = probes::arm(trace, policy.context_horizon_px, policy, sampler)?;
        let local = probes::arm(trace, policy.branch_horizon_px, policy, sampler)?;
        feature.support = minimum(feature.support, broad.support(policy.context_horizon_px));
        feature.support_2r = minimum(feature.support_2r, local.support(policy.branch_horizon_px));
        feature.arms.push(arm_description(trace, broad));
    }
    Ok(feature)
}

fn arm_description(trace: &Trace, evidence: ArmEvidence) -> ContextArm {
    ContextArm {
        edge: trace.steps[0].edge,
        edges: trace.steps.iter().map(|step| step.edge).collect(),
        length_px: trace.length,
        ridge_separation: evidence.separation,
        continuity: evidence.continuity,
        supported_length_px: evidence.supported_length,
        longest_unsupported_px: evidence.longest_unsupported,
        stop: trace.stop,
    }
}

fn clipped_backbone(
    graph: &RidgeGraph,
    trace: &Trace,
    horizon: f64,
    work: &mut Work,
) -> Result<Vec<Option<Vec<super::Point>>>, FingerprintError> {
    work.charge(graph.edges.len())?;
    let mut clipped = vec![None; graph.edges.len()];
    for step in &trace.steps {
        let remove = horizon - step.entry_px;
        if remove <= 0.0 {
            break;
        }
        let source = &graph.edges[step.edge].points;
        work.charge(source.len().saturating_mul(2))?;
        let remaining = geometry::arc_length(source, false) - remove;
        let mut points = if remaining <= 0.0 {
            Vec::new()
        } else {
            geometry::prefix(source, !step.reversed, remaining)
        };
        if !step.reversed {
            points.reverse();
        }
        clipped[step.edge] = Some(points);
    }
    Ok(clipped)
}

fn cycle_context(
    points: &[super::Point],
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<FeatureContext, FingerprintError> {
    let interior = super::sampling::interior_light(sampler, points, policy)?;
    let evidence = probes::cycle(points, policy, sampler)?;
    let complete = sampler.complete();
    Ok(FeatureContext {
        support: complete
            .then(|| minimum(evidence.support(policy.context_horizon_px), interior))
            .flatten(),
        support_2r: complete
            .then(|| minimum(evidence.support(policy.branch_horizon_px), interior))
            .flatten(),
        source_coverage: sampler.coverage(),
        separation: interior,
        termination: None,
        continuation: None,
        forward_continuation: None,
        curved_continuation: None,
        arms: Vec::new(),
    })
}

fn minimum(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    Some(a?.min(b?))
}

#[cfg(test)]
mod tests;
