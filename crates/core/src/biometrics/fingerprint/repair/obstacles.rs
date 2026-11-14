//! Spatial indexing and observed-ridge obstruction checks for proposed joins.

use std::collections::{BTreeMap, BTreeSet};

use super::super::NodeKind;
use super::bridge::{Endpoint, cross, distance, dot};
use super::{FingerprintError, GapPolicy, Point, RidgeGraph, RidgeMap};

const MAX_WORK: usize = 20_000_000;

struct Segment {
    a: Point,
    b: Point,
    edge: Option<usize>,
}

pub(super) struct SegmentIndex {
    segments: Vec<Segment>,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
    size: f64,
}

impl SegmentIndex {
    pub(super) fn new(graph: &RidgeGraph, size: f64) -> Result<Self, FingerprintError> {
        let mut result = Self {
            segments: Vec::new(),
            cells: BTreeMap::new(),
            size,
        };
        let mut work = 0;
        for (edge, item) in graph.edges.iter().enumerate() {
            for pair in item.points.windows(2) {
                result.insert(pair[0], pair[1], Some(edge), &mut work)?;
            }
        }
        for cycle in &graph.cycles {
            for index in 0..cycle.points.len() {
                result.insert(
                    cycle.points[index],
                    cycle.points[(index + 1) % cycle.points.len()],
                    None,
                    &mut work,
                )?;
            }
        }
        // Isolated retained points obstruct a bridge as much as a line does.
        for node in &graph.nodes {
            if node.kind == NodeKind::Isolated {
                result.insert(node.position, node.position, None, &mut work)?;
            }
        }
        Ok(result)
    }

    fn insert(
        &mut self,
        a: Point,
        b: Point,
        edge: Option<usize>,
        work: &mut usize,
    ) -> Result<(), FingerprintError> {
        let index = self.segments.len();
        self.segments.push(Segment { a, b, edge });
        let (start, end) = bounds(&[a, b], 0.0, self.size);
        for y in start.1..=end.1 {
            for x in start.0..=end.0 {
                spend(work, 1)?;
                self.cells.entry((x, y)).or_default().push(index);
            }
        }
        Ok(())
    }

    pub(super) fn obstructs(
        &self,
        points: &[Point],
        a: &Endpoint,
        b: &Endpoint,
        policy: GapPolicy,
        work: &mut usize,
    ) -> Result<bool, FingerprintError> {
        let (start, end) = bounds(points, policy.clearance_px, self.size);
        let mut segments = BTreeSet::new();
        for y in start.1..=end.1 {
            for x in start.0..=end.0 {
                segments.extend(self.cells.get(&(x, y)).into_iter().flatten().copied());
            }
        }
        for index in segments {
            let obstacle = &self.segments[index];
            for bridge in points.windows(2) {
                spend(work, 1)?;
                let contact_a = obstacle.edge == Some(a.edge)
                    && (obstacle.a == a.position || obstacle.b == a.position)
                    && bridge[0] == a.position;
                let contact_b = obstacle.edge == Some(b.edge)
                    && (obstacle.a == b.position || obstacle.b == b.position)
                    && bridge[1] == b.position;
                if intersects(bridge[0], bridge[1], obstacle.a, obstacle.b)
                    && !contact_a
                    && !contact_b
                {
                    return Ok(true);
                }
                // Attachment neighborhoods need zero separation from their own
                // incident edge. Other edges never receive this exemption.
                let attached = [a, b].into_iter().any(|end| {
                    obstacle.edge == Some(end.edge)
                        && distance(obstacle.a, end.position) <= policy.tangent_support_px
                        && distance(obstacle.b, end.position) <= policy.tangent_support_px
                });
                if !attached
                    && segment_distance(bridge[0], bridge[1], obstacle.a, obstacle.b)
                        < policy.clearance_px
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

pub(super) fn paths_conflict(
    a: &[Point],
    b: &[Point],
    clearance: f64,
    work: &mut usize,
) -> Result<bool, FingerprintError> {
    let (a0, a1) = bounds(a, clearance, 1.0);
    let (b0, b1) = bounds(b, 0.0, 1.0);
    if a1.0 < b0.0 || b1.0 < a0.0 || a1.1 < b0.1 || b1.1 < a0.1 {
        return Ok(false);
    }
    for left in a.windows(2) {
        for right in b.windows(2) {
            spend(work, 1)?;
            if segment_distance(left[0], left[1], right[0], right[1]) < clearance {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn raw_obstructs(
    map: &RidgeMap,
    points: &[Point],
    a: &Endpoint,
    b: &Endpoint,
    policy: GapPolicy,
    work: &mut usize,
) -> Result<bool, FingerprintError> {
    let radius = policy.clearance_px.ceil() as usize;
    for &point in points {
        let (x, y) = (point.x().round() as usize, point.y().round() as usize);
        for row in y.saturating_sub(radius)..=y.saturating_add(radius).min(map.height - 1) {
            for column in x.saturating_sub(radius)..=x.saturating_add(radius).min(map.width - 1) {
                spend(work, 1)?;
                if !map.ridges[row * map.width + column] {
                    continue;
                }
                let pixel = Point::new(column as f64, row as f64).map_err(|_| {
                    FingerprintError::InvalidMap("raw obstacle coordinate overflow")
                })?;
                if distance(point, pixel) >= policy.clearance_px {
                    continue;
                }
                // The observed tails lie behind their fixed attachment ends.
                // Smoothed-curve intersection checks independently constrain
                // these exempt neighborhoods; interior raw pixels remain walls.
                let tail = [a, b].into_iter().any(|end| {
                    distance(pixel, end.position) <= policy.tangent_support_px
                        && dot(
                            (pixel.x() - end.position.x(), pixel.y() - end.position.y()),
                            end.tangent,
                        ) <= 1e-9
                });
                if !tail {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn bounds(points: &[Point], padding: f64, size: f64) -> ((i64, i64), (i64, i64)) {
    let min_x = points.iter().map(|p| p.x()).fold(f64::INFINITY, f64::min) - padding;
    let min_y = points.iter().map(|p| p.y()).fold(f64::INFINITY, f64::min) - padding;
    let max_x = points
        .iter()
        .map(|p| p.x())
        .fold(f64::NEG_INFINITY, f64::max)
        + padding;
    let max_y = points
        .iter()
        .map(|p| p.y())
        .fold(f64::NEG_INFINITY, f64::max)
        + padding;
    (
        ((min_x / size).floor() as i64, (min_y / size).floor() as i64),
        ((max_x / size).floor() as i64, (max_y / size).floor() as i64),
    )
}

pub(super) fn cell(point: Point, size: f64) -> (i64, i64) {
    (
        (point.x() / size).floor() as i64,
        (point.y() / size).floor() as i64,
    )
}
pub(super) fn spend(work: &mut usize, amount: usize) -> Result<(), FingerprintError> {
    *work = work
        .checked_add(amount)
        .filter(|&value| value <= MAX_WORK)
        .ok_or(FingerprintError::WorkLimit)?;
    Ok(())
}
fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    if intersects(a, b, c, d) {
        return 0.0;
    }
    point_distance(a, c, d)
        .min(point_distance(b, c, d))
        .min(point_distance(c, a, b))
        .min(point_distance(d, a, b))
}
fn point_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = (b.x() - a.x(), b.y() - a.y());
    let squared = dot(ab, ab);
    if squared == 0.0 {
        return distance(p, a);
    }
    let t = (dot((p.x() - a.x(), p.y() - a.y()), ab) / squared).clamp(0.0, 1.0);
    (p.x() - a.x() - t * ab.0).hypot(p.y() - a.y() - t * ab.1)
}
fn intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    let ab = (b.x() - a.x(), b.y() - a.y());
    let cd = (d.x() - c.x(), d.y() - c.y());
    let ac = (c.x() - a.x(), c.y() - a.y());
    let denominator = cross(ab, cd);
    if denominator.abs() <= 64.0 * f64::EPSILON * distance(a, b).max(1.0) * distance(c, d).max(1.0)
    {
        // Parallel/collinear and isolated-point contacts.
        return point_distance(a, c, d) <= 1e-9
            || point_distance(b, c, d) <= 1e-9
            || point_distance(c, a, b) <= 1e-9
            || point_distance(d, a, b) <= 1e-9;
    }
    let t = cross(ac, cd) / denominator;
    let u = cross(ac, ab) / denominator;
    (-1e-12..=1.0 + 1e-12).contains(&t) && (-1e-12..=1.0 + 1e-12).contains(&u)
}
