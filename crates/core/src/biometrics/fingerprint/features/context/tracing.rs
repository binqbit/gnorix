//! Bounded observation through unambiguous, source-supported incident edges.

use super::super::geometry::{self, distance};
use super::super::sampling::{Sampler, intervals};
use super::super::{FeaturePolicy, FingerprintError, Point, RidgeGraph};
use super::{ContextStop, probes};

pub(super) struct Trace {
    pub points: Vec<Point>,
    pub steps: Vec<TraceStep>,
    pub length: f64,
    pub stop: ContextStop,
}

/// Arc positions distinguish the source-confirmed port connector from the
/// original edge itself. Only edges with a nonempty traversed arc are recorded.
pub(super) struct TraceStep {
    pub edge: usize,
    pub bridge_start_px: f64,
    pub entry_px: f64,
    pub reversed: bool,
}

pub(super) fn arms(
    graph: &RidgeGraph,
    incidence: &[Vec<usize>],
    node: usize,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Vec<Trace>, FingerprintError> {
    sampler
        .work
        .charge(incidence[node].len().saturating_mul(incidence[node].len()))?;
    incidence[node]
        .iter()
        .enumerate()
        .map(|(position, &edge)| {
            let item = &graph.edges[edge];
            // A self edge occurs twice in incidence: its two outward directions
            // are separate arms, not two copies of the same traversal.
            let reverse = item.start != node
                || (item.start == item.end && incidence[node][..position].contains(&edge));
            trace(graph, incidence, node, edge, reverse, policy, sampler)
        })
        .collect()
}

fn trace(
    graph: &RidgeGraph,
    incidence: &[Vec<usize>],
    origin: usize,
    first_edge: usize,
    reverse: bool,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Trace, FingerprintError> {
    let first = &graph.edges[first_edge];
    sampler.work.charge(first.points.len())?;
    let points = geometry::prefix(&first.points, reverse, policy.context_horizon_px);
    let mut result = Trace {
        length: geometry::arc_length(&points, false),
        points,
        steps: vec![TraceStep {
            edge: first_edge,
            bridge_start_px: 0.0,
            entry_px: 0.0,
            reversed: reverse,
        }],
        stop: ContextStop::GraphEnding,
    };
    let mut node = if reverse { first.start } else { first.end };
    loop {
        if result.length >= policy.context_horizon_px - 1e-9 {
            result.stop = ContextStop::Horizon;
            break;
        }
        if node == origin {
            result.stop = ContextStop::ClosedPath;
            break;
        }
        sampler.work.charge(incidence[node].len())?;
        let outgoing: Vec<_> = incidence[node]
            .iter()
            .copied()
            .filter(|&edge| result.steps.last().is_none_or(|step| step.edge != edge))
            .collect();
        if outgoing.is_empty() {
            break;
        }
        sampler.work.charge(result.points.len().saturating_mul(3))?;
        let Some(incoming) =
            probes::tangent(&result.points, result.length, policy.tangent_support_px)
        else {
            result.stop = ContextStop::AmbiguousJunction;
            break;
        };
        let mut selected = None;
        let mut missing = false;
        let mut ambiguous = false;
        for edge in outgoing {
            sampler.work.charge(graph.edges[edge].points.len())?;
            let reverse = graph.edges[edge].start != node;
            let path = geometry::prefix(
                &graph.edges[edge].points,
                reverse,
                policy.context_horizon_px - result.length,
            );
            sampler.work.charge(path.len().saturating_mul(3))?;
            let Some(direction) = probes::tangent(&path, 0.0, policy.tangent_support_px) else {
                continue;
            };
            let aligned = incoming[0] * direction[0] + incoming[1] * direction[1];
            if aligned < policy.stability.max_arm_angle_radians.cos() {
                continue;
            }
            let Some(&port) = result.points.last() else {
                continue;
            };
            if distance(port, path[0]) > policy.reference_width_px / 2.0 {
                continue;
            }
            sampler
                .work
                .charge(result.points.len().saturating_add(path.len()))?;
            let mut inspection = result.points.clone();
            append(&mut inspection, &path);
            sampler.work.charge(inspection.len())?;
            inspection = geometry::prefix(&inspection, false, policy.context_horizon_px);
            let complete_length = geometry::arc_length(&inspection, false);
            let span = (complete_length - result.length).min(policy.reference_width_px);
            let support = continuation(&inspection, result.length, span, policy, sampler)?;
            match support {
                None => missing = true,
                Some(value) if value > 0.0 => {
                    if selected.is_some() {
                        ambiguous = true;
                    } else {
                        selected = Some((edge, reverse, path));
                    }
                }
                _ => {}
            }
        }
        if missing {
            result.stop = ContextStop::MissingSource;
            break;
        }
        if ambiguous {
            result.stop = ContextStop::AmbiguousJunction;
            break;
        }
        let Some((edge, reverse, path)) = selected else {
            result.stop = ContextStop::UnsupportedContinuation;
            break;
        };
        sampler.work.charge(result.steps.len())?;
        if result.steps.iter().any(|step| step.edge == edge) {
            result.stop = ContextStop::ClosedPath;
            break;
        }
        let bridge_start = result.length;
        let entry =
            bridge_start + distance(*result.points.last().expect("observed arm port"), path[0]);
        if policy.context_horizon_px - entry > 1e-9 {
            result.steps.push(TraceStep {
                edge,
                bridge_start_px: bridge_start,
                entry_px: entry,
                reversed: reverse,
            });
        }
        sampler.work.charge(
            result
                .points
                .len()
                .saturating_add(path.len())
                .saturating_mul(3),
        )?;
        append(&mut result.points, &path);
        result.points = geometry::prefix(&result.points, false, policy.context_horizon_px);
        result.length = geometry::arc_length(&result.points, false);
        node = if reverse {
            graph.edges[edge].start
        } else {
            graph.edges[edge].end
        };
    }
    Ok(result)
}

fn continuation(
    path: &[Point],
    start: f64,
    span: f64,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<f64>, FingerprintError> {
    if span <= 0.0 {
        return Ok(Some(0.0));
    }
    let count = intervals(span, policy.sample_spacing_px)?;
    let mut weakest = 1.0_f64;
    let mut complete = true;
    for index in 0..count {
        let station = start + span * (index as f64 + 0.5) / count as f64;
        if let Some(value) = probes::cross_section(path, station, policy, sampler)? {
            weakest = weakest.min(value);
        } else {
            complete = false;
        }
    }
    Ok(complete.then_some(weakest))
}

fn append(target: &mut Vec<Point>, source: &[Point]) {
    let skip = usize::from(target.last() == source.first());
    target.extend_from_slice(&source[skip..]);
}

/// Two branches may form a valid enclosure. Their shared outgoing trunk is not
/// independent evidence for either branch: retain the observed enclosure and
/// let its actual source valley determine quality, without a categorical veto.
pub(super) fn limit_shared(
    traces: &mut [Trace],
    work: &mut super::super::sampling::Work,
) -> Result<(), FingerprintError> {
    let mut cuts: Vec<_> = traces.iter().map(|trace| trace.steps.len()).collect();
    for (a, first) in traces.iter().enumerate() {
        for (b, second) in traces.iter().enumerate().skip(a + 1) {
            work.charge(first.steps.len().saturating_mul(second.steps.len()))?;
            for (i, step) in first.steps.iter().enumerate() {
                if let Some(j) = second
                    .steps
                    .iter()
                    .position(|other| other.edge == step.edge)
                {
                    // The two directions of one initial self edge remain valid
                    // observed arms. Only reused continuation edges are clipped.
                    if i > 0 {
                        cuts[a] = cuts[a].min(i);
                    }
                    if j > 0 {
                        cuts[b] = cuts[b].min(j);
                    }
                }
            }
        }
    }
    for (trace, cut) in traces.iter_mut().zip(cuts) {
        if cut == trace.steps.len() {
            continue;
        }
        let length = trace.steps[cut].bridge_start_px;
        work.charge(trace.points.len())?;
        trace.points = geometry::prefix(&trace.points, false, length);
        trace.length = geometry::arc_length(&trace.points, false);
        trace.steps.truncate(cut);
        trace.stop = ContextStop::ClosedPath;
    }
    Ok(())
}
