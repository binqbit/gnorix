//! Termination evidence and source-confirmed contradictions beyond the tip.

use super::super::super::cleanup;
use super::super::{geometry, sampling};
use super::{
    Capture, FeaturePolicy, FingerprintError, LocalPalette, RidgeGraph, Sampler, Trace,
    clipped_backbone, probes,
};

#[derive(Default)]
pub(super) struct EndingEvidence {
    pub termination: Option<f64>,
    pub continuation: Option<f64>,
    pub forward_continuation: Option<f64>,
    pub curved_continuation: Option<f64>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn assess(
    capture: &Capture,
    palette: &LocalPalette,
    graph: &RidgeGraph,
    components: &[usize],
    node: usize,
    trace: &Trace,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
    guard_work: &mut cleanup::Work,
) -> Result<EndingEvidence, FingerprintError> {
    sampler.work.charge(trace.points.len().saturating_mul(4))?;
    let Some(inward) = probes::tangent(&trace.points, 0.0, policy.tangent_support_px) else {
        return Ok(EndingEvidence::default());
    };
    let origin = trace.points[0];
    // Availability is checked on the original capture. Empty light background
    // in this corridor does not need an artificial usable normalization palette.
    let observed = sampling::forward_observed(capture, origin, inward, policy, sampler.work)?;
    sampler.external_support(observed)?;

    let cap = probes::cap(trace, policy, sampler)?;
    let termination = cap.separation;

    // Clip the same physical terminal arc across all traversed graph edges.
    // The incoming ridge beyond that arc remains explanatory source backbone.
    let clipped = clipped_backbone(graph, trace, policy.branch_horizon_px, sampler.work)?;
    let mut points = geometry::prefix(&trace.points, false, policy.branch_horizon_px);
    points.reverse();
    let reference = cap.reference_margin.filter(|value| *value > 0.0);
    let curved_continuation = if let Some(reference) = reference {
        cleanup::feature_continuation(
            capture,
            palette,
            &points,
            graph.edges.iter().enumerate().map(|(id, edge)| {
                (
                    clipped[id].as_deref().unwrap_or(&edge.points),
                    components[edge.start],
                )
            }),
            components[node],
            policy.reference_width_px,
            reference,
            &mut |point| {
                // A strict backbone proof can independently explain this escape.
                // Optional missing ribbon samples must not invalidate that proof;
                // the combined curved result records unavailable required evidence.
                let mut incoming = Sampler::new(capture, palette, sampler.work);
                probes::incoming_footprint(trace, point, policy, &mut incoming)
            },
            guard_work,
        )?
    } else {
        None
    };
    if reference.is_some() {
        sampler.external_support(curved_continuation.is_some())?;
    }
    let paths = graph
        .edges
        .iter()
        .enumerate()
        .map(|(id, edge)| (clipped[id].as_deref().unwrap_or(&edge.points), false))
        .chain(graph.cycles.iter().map(|cycle| (&cycle.points[..], true)));
    let forward_continuation = if observed {
        forward(paths, origin, inward, policy, sampler)?
    } else {
        None
    };
    let continuation = match (forward_continuation, curved_continuation) {
        (Some(forward), Some(curved)) => Some(forward.max(curved)),
        _ => None,
    };
    Ok(EndingEvidence {
        termination,
        continuation,
        forward_continuation,
        curved_continuation,
    })
}

fn forward<'a>(
    paths: impl Iterator<Item = (&'a [super::super::Point], bool)>,
    origin: super::super::Point,
    inward: [f64; 2],
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<f64>, FingerprintError> {
    let normal = [-inward[1], inward[0]];
    let mut strongest = 0.0_f64;
    let mut complete = true;
    for (points, closed) in paths {
        let mut targets =
            geometry::continuation_targets(points, closed, origin, inward, policy, sampler.work)?;
        sampler.work.charge(
            targets
                .len()
                .saturating_mul((usize::BITS - targets.len().leading_zeros()) as usize),
        )?;
        targets.sort_by(|a, b| {
            b.support
                .total_cmp(&a.support)
                .then(a.station.total_cmp(&b.station))
        });
        for target in targets {
            // A source score cannot exceed one. Lower geometric upper bounds
            // cannot change the strongest confirmed contradiction.
            if target.support <= strongest {
                continue;
            }
            let sideways = (target.position.x() - origin.x()) * normal[0]
                + (target.position.y() - origin.y()) * normal[1];
            if sideways.abs() > policy.reference_width_px / 2.0 {
                continue;
            }
            if let Some(source) =
                probes::continuation_window(points, closed, target.station, policy, sampler)?
            {
                strongest = strongest.max(target.support * source);
            } else {
                complete = false;
            }
        }
    }
    Ok(complete.then_some(strongest))
}
