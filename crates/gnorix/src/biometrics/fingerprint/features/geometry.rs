//! Observed arc extent and branch geometry; no source pixels are invented.

use super::sampling::Work;
use super::{FeaturePolicy, FingerprintError, Point, RidgeGraph};

pub(super) struct ArmPath {
    pub edge: usize,
    pub points: Vec<Point>,
    pub length: f64,
    pub direction: Option<[f64; 2]>,
}

pub(super) fn arms(
    graph: &RidgeGraph,
    policy: &FeaturePolicy,
    work: &mut Work,
) -> Result<Vec<Vec<ArmPath>>, FingerprintError> {
    let mut output: Vec<Vec<ArmPath>> = (0..graph.nodes.len()).map(|_| Vec::new()).collect();
    for (id, edge) in graph.edges.iter().enumerate() {
        if edge.start >= output.len() || edge.end >= output.len() || edge.points.len() < 2 {
            return Err(FingerprintError::InvalidTopology(
                "invalid feature incident path",
            ));
        }
        work.charge(edge.points.len().saturating_mul(2))?;
        let length = arc_length(&edge.points, false).min(policy.branch_horizon_px);
        for (node, reverse) in [(edge.start, false), (edge.end, true)] {
            let points = prefix(&edge.points, reverse, policy.branch_horizon_px);
            let direction = points.first().and_then(|&first| {
                direction(first, at(&points, policy.tangent_support_px.min(length)))
            });
            output[node].push(ArmPath {
                edge: id,
                points,
                length,
                direction,
            });
        }
    }
    Ok(output)
}

pub(super) fn prefix(points: &[Point], reverse: bool, limit: f64) -> Vec<Point> {
    let point = |i: usize| points[if reverse { points.len() - 1 - i } else { i }];
    if points.is_empty() {
        return Vec::new();
    }
    let mut result = vec![point(0)];
    let mut remaining = limit;
    for index in 1..points.len() {
        let (a, b) = (point(index - 1), point(index));
        let length = distance(a, b);
        if length == 0.0 {
            continue;
        }
        if length >= remaining {
            result.push(lerp(a, b, remaining / length));
            break;
        }
        result.push(b);
        remaining -= length;
    }
    result
}

pub(super) fn at(points: &[Point], target: f64) -> Point {
    let mut remaining = target;
    for pair in points.windows(2) {
        let length = distance(pair[0], pair[1]);
        if length > 0.0 && length >= remaining {
            return lerp(pair[0], pair[1], remaining / length);
        }
        remaining -= length;
    }
    *points.last().expect("an observed path has a source point")
}

pub(super) fn extent(arms: &[ArmPath], horizon: f64) -> f64 {
    arms.iter()
        .map(|arm| (arm.length / horizon).clamp(0.0, 1.0))
        .reduce(f64::min)
        .unwrap_or(0.0)
}

/// Distinct source paths must stay spatially separated over their outer arc.
/// This uses source-pixel distance rather than preferring an ideal Y angle.
pub(super) fn separation(
    arms: &[ArmPath],
    policy: &FeaturePolicy,
    work: &mut Work,
) -> Result<Option<f64>, FingerprintError> {
    if arms.len() < 2 {
        return Ok(Some(0.0));
    }
    let mut smallest = 1.0_f64;
    for (i, a) in arms.iter().enumerate() {
        for b in &arms[i + 1..] {
            let available = a.length.min(b.length).min(policy.branch_horizon_px);
            if available <= policy.reference_width_px {
                return Ok(Some(0.0));
            }
            let span = available - policy.reference_width_px;
            let count = (span / policy.sample_spacing_px).ceil().max(1.0) as usize;
            work.charge(count.saturating_mul(a.points.len() + b.points.len()))?;
            let mut total = 0.0;
            for index in 0..count {
                let station =
                    policy.reference_width_px + span * (index as f64 + 0.5) / count as f64;
                total += (distance(at(&a.points, station), at(&b.points, station))
                    / policy.reference_width_px)
                    .min(1.0);
            }
            smallest = smallest.min(total / count as f64 * span / policy.reference_width_px);
        }
    }
    Ok(Some(smallest))
}

/// Nearby forward paths aligned over a supported tangent span. Their presence
/// weakens termination evidence; this measurement never proposes a new join.
/// The supplied evidence graph excludes only this ending's inspected terminal
/// arc; the remaining own edge can contain a nearby facing end of a broken ring.
pub(super) fn continuation(
    node: usize,
    all: &[Vec<ArmPath>],
    graph: &RidgeGraph,
    policy: &FeaturePolicy,
    work: &mut Work,
) -> Result<Option<f64>, FingerprintError> {
    let arm = &all[node][0];
    let Some(tangent) = arm.direction else {
        return Ok(None);
    };
    let origin = arm.points[0];
    let mut maximum = 0.0_f64;
    for (points, closed) in graph
        .edges
        .iter()
        .map(|edge| (&edge.points[..], false))
        .chain(graph.cycles.iter().map(|cycle| (&cycle.points[..], true)))
    {
        work.charge(points.len().saturating_mul(12))?;
        let mut distances = vec![0.0];
        for (a, b) in segments(points, closed) {
            distances.push(distances.last().unwrap() + distance(a, b));
        }
        for (index, (a, b)) in segments(points, closed).enumerate() {
            let Some(segment_direction) = direction(a, b) else {
                continue;
            };
            let length = distance(a, b);
            let projection = ((origin.x() - a.x()) * segment_direction[0]
                + (origin.y() - a.y()) * segment_direction[1])
                .clamp(0.0, length);
            let target = lerp(a, b, projection / length);
            let gap = distance(origin, target);
            if gap <= 0.0 || gap >= policy.continuation_search_px {
                continue;
            }
            let station = distances[index] + projection;
            let Some(other_tangent) = tangent_at(
                points,
                &distances,
                closed,
                station,
                policy.tangent_support_px,
            ) else {
                continue;
            };
            let Some(chord) = direction(origin, target) else {
                continue;
            };
            let facing = (-tangent[0] * chord[0] - tangent[1] * chord[1]).clamp(0.0, 1.0);
            let aligned = (other_tangent[0] * tangent[0] + other_tangent[1] * tangent[1]).abs();
            let reach = 1.0 - gap / policy.continuation_search_px;
            maximum = maximum.max(facing * aligned * reach);
        }
    }
    Ok(Some(maximum))
}

fn tangent_at(
    points: &[Point],
    distances: &[f64],
    closed: bool,
    station: f64,
    span: f64,
) -> Option<[f64; 2]> {
    let total = *distances.last()?;
    if total <= 0.0 {
        return None;
    }
    let span = span.min(total);
    let (from, to) = if closed {
        (
            (station - span / 2.0).rem_euclid(total),
            (station + span / 2.0).rem_euclid(total),
        )
    } else {
        let from = (station - span / 2.0).clamp(0.0, total - span);
        (from, from + span)
    };
    let point_at = |station: f64| {
        let index = distances
            .partition_point(|&distance| distance <= station)
            .saturating_sub(1)
            .min(distances.len() - 2);
        let length = distances[index + 1] - distances[index];
        if length <= 0.0 {
            return points[index % points.len()];
        }
        lerp(
            points[index % points.len()],
            points[(index + 1) % points.len()],
            (station - distances[index]) / length,
        )
    };
    direction(point_at(from), point_at(to))
}

pub(super) fn centroid(points: &[Point]) -> Result<Point, FingerprintError> {
    if points.len() < 3 {
        return Err(FingerprintError::InvalidTopology(
            "invalid isolated feature cycle",
        ));
    }
    let (mut x, mut y, mut total) = (0.0, 0.0, 0.0);
    for (a, b) in segments(points, true) {
        let length = distance(a, b);
        x += (a.x() + b.x()) / 2.0 * length;
        y += (a.y() + b.y()) / 2.0 * length;
        total += length;
    }
    if !total.is_finite() || total <= 0.0 {
        return Err(FingerprintError::InvalidTopology(
            "degenerate feature cycle",
        ));
    }
    Point::new(x / total, y / total)
        .map_err(|_| FingerprintError::InvalidCurve("feature centroid overflow"))
}

pub(super) fn arc_length(points: &[Point], closed: bool) -> f64 {
    segments(points, closed).map(|(a, b)| distance(a, b)).sum()
}

pub(super) fn segments(
    points: &[Point],
    closed: bool,
) -> impl Iterator<Item = (Point, Point)> + '_ {
    points
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .chain((closed && points.len() > 1).then(|| (*points.last().unwrap(), points[0])))
}

pub(super) fn direction(a: Point, b: Point) -> Option<[f64; 2]> {
    let length = distance(a, b);
    (length > 0.0).then_some([(b.x() - a.x()) / length, (b.y() - a.y()) / length])
}
pub(super) fn distance(a: Point, b: Point) -> f64 {
    (a.x() - b.x()).hypot(a.y() - b.y())
}
pub(super) fn lerp(a: Point, b: Point, fraction: f64) -> Point {
    Point::new(
        a.x() + (b.x() - a.x()) * fraction,
        a.y() + (b.y() - a.y()) * fraction,
    )
    .expect("bounded source interpolation")
}
