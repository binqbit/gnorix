//! Local feature geometry and deterministic one-to-one correspondence.

use std::collections::{BTreeMap, VecDeque};

use super::super::super::{FingerprintError, NodeKind, RidgeGraph};
use super::StabilityPolicy;

const MAX_WORK: usize = 100_000_000;

#[derive(Default)]
pub(super) struct Work(usize);

impl Work {
    fn charge(&mut self, count: usize) -> Result<(), FingerprintError> {
        self.0 = self
            .0
            .checked_add(count)
            .filter(|&value| value <= MAX_WORK)
            .ok_or(FingerprintError::WorkLimit)?;
        Ok(())
    }
}

type Vector = [f64; 2];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Node(NodeKind, usize),
    Cycle,
}

struct Arm {
    direction: Vector,
    samples: [Vector; 4],
}

struct Cycle {
    length: f64,
    radius: f64,
    points: Vec<Vector>,
    samples: Vec<Vector>,
}

pub(super) struct Descriptor {
    kind: Kind,
    position: Vector,
    arms: Vec<Arm>,
    complete: bool,
    cycle: Option<Cycle>,
}

pub(super) fn describe(
    graph: &RidgeGraph,
    policy: StabilityPolicy,
    work: &mut Work,
) -> Result<Vec<Descriptor>, FingerprintError> {
    work.charge(graph.nodes.len().saturating_add(graph.cycles.len()))?;
    let mut result: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| Descriptor {
            kind: Kind::Node(node.kind, 0),
            position: [node.position.x(), node.position.y()],
            arms: vec![],
            complete: true,
            cycle: None,
        })
        .collect();
    for edge in &graph.edges {
        for (node, reverse) in [(edge.start, false), (edge.end, true)] {
            let descriptor = result
                .get_mut(node)
                .ok_or(FingerprintError::InvalidTopology(
                    "feature edge node is missing",
                ))?;
            if let Kind::Node(kind, degree) = descriptor.kind {
                descriptor.kind = Kind::Node(kind, degree + 1);
            }
            if let Some(arm) = describe_arm(
                &edge.points,
                reverse,
                descriptor.position,
                policy.arm_radius_px,
                work,
            )? {
                descriptor.arms.push(arm);
            } else {
                descriptor.complete = false;
            }
        }
    }
    for cycle in &graph.cycles {
        let (position, geometry) = describe_cycle(&cycle.points, policy, work)?;
        result.push(Descriptor {
            kind: Kind::Cycle,
            position,
            arms: vec![],
            complete: geometry.is_some(),
            cycle: geometry,
        });
    }
    Ok(result)
}

fn describe_arm(
    points: &[crate::biometrics::geometry::Point],
    reverse: bool,
    position: Vector,
    radius: f64,
    work: &mut Work,
) -> Result<Option<Arm>, FingerprintError> {
    let mut observed = Vec::new();
    let mut lengths = vec![0.0];
    for offset in 0..points.len() {
        work.charge(1)?;
        let point = points[if reverse {
            points.len() - 1 - offset
        } else {
            offset
        }];
        let point = [point.x(), point.y()];
        if let Some(&previous) = observed.last() {
            lengths.push(lengths.last().unwrap() + distance(previous, point));
        }
        observed.push(point);
        if *lengths.last().unwrap() >= radius {
            break;
        }
    }
    if observed.len() < 2 || *lengths.last().unwrap() <= 0.0 {
        return Ok(None);
    }
    let target = interpolate(
        &observed,
        &lengths,
        (radius / 2.0).min(lengths.last().unwrap() / 2.0),
    );
    let outward = sub(target, observed[0]);
    let norm = length(outward);
    if norm <= 0.0 {
        return Ok(None);
    }
    let samples = std::array::from_fn(|i| {
        sub(
            interpolate(&observed, &lengths, radius * (i + 1) as f64 / 4.0),
            position,
        )
    });
    Ok(Some(Arm {
        direction: [outward[0] / norm, outward[1] / norm],
        samples,
    }))
}

fn describe_cycle(
    points: &[crate::biometrics::geometry::Point],
    policy: StabilityPolicy,
    work: &mut Work,
) -> Result<(Vector, Option<Cycle>), FingerprintError> {
    work.charge(points.len())?;
    if points.is_empty() {
        return Ok(([0.0, 0.0], None));
    }
    let mut vertices: Vec<_> = points.iter().map(|p| [p.x(), p.y()]).collect();
    vertices.push(vertices[0]);
    let mut total = 0.0;
    let mut center = [0.0, 0.0];
    let mut positions = vec![0.0];
    for pair in vertices.windows(2) {
        let span = distance(pair[0], pair[1]);
        total += span;
        positions.push(total);
        for (axis, value) in center.iter_mut().enumerate() {
            *value += span * (pair[0][axis] + pair[1][axis]) / 2.0;
        }
    }
    if !total.is_finite() || total <= 0.0 {
        return Ok((vertices[0], None));
    }
    for value in &mut center {
        *value /= total;
    }
    let relative: Vec<_> = vertices.iter().map(|&p| sub(p, center)).collect();
    let mut second_moment = 0.0;
    for pair in relative.windows(2) {
        second_moment += distance(pair[0], pair[1])
            * (dot(pair[0], pair[0]) + dot(pair[0], pair[1]) + dot(pair[1], pair[1]))
            / 3.0;
    }
    let radius = (second_moment / total).sqrt();
    if !radius.is_finite() || radius <= 0.0 {
        return Ok((center, None));
    }
    let spacing = (policy.arm_radius_px / 8.0).min(radius / 4.0);
    let count = (total / spacing).ceil();
    if !count.is_finite() || count > MAX_WORK as f64 {
        return Err(FingerprintError::WorkLimit);
    }
    let count = (count as usize).max(3);
    work.charge(count)?;
    let mut samples = relative.clone();
    samples.extend((0..count).map(|i| {
        sub(
            interpolate(&vertices, &positions, total * i as f64 / count as f64),
            center,
        )
    }));
    Ok((
        center,
        Some(Cycle {
            length: total,
            radius,
            points: relative,
            samples,
        }),
    ))
}

pub(super) fn correspond(
    reference: &[Descriptor],
    variant: &[Descriptor],
    policy: StabilityPolicy,
    work: &mut Work,
) -> Result<Vec<Option<usize>>, FingerprintError> {
    let mut cells: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    let cell = |p: Vector| {
        (
            (p[0] / policy.max_position_shift_px).floor() as i64,
            (p[1] / policy.max_position_shift_px).floor() as i64,
        )
    };
    for (id, item) in variant.iter().enumerate() {
        cells.entry(cell(item.position)).or_default().push(id);
    }
    let mut adjacency = Vec::with_capacity(reference.len());
    for candidate in reference {
        let (x, y) = cell(candidate.position);
        let mut compatible = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let (Some(cx), Some(cy)) = (x.checked_add(dx), y.checked_add(dy)) else {
                    continue;
                };
                if let Some(ids) = cells.get(&(cx, cy)) {
                    for &id in ids {
                        work.charge(1)?;
                        if let Some(cost) = compatibility(candidate, &variant[id], policy, work)? {
                            compatible.push((cost, id));
                        }
                    }
                }
            }
        }
        compatible.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        adjacency.push(compatible.into_iter().map(|(_, id)| id).collect());
    }
    assign(&adjacency, variant.len(), work)
}

fn compatibility(
    a: &Descriptor,
    b: &Descriptor,
    policy: StabilityPolicy,
    work: &mut Work,
) -> Result<Option<f64>, FingerprintError> {
    if a.kind != b.kind || !a.complete || !b.complete {
        return Ok(None);
    }
    let shift = distance(a.position, b.position);
    let limit = if matches!(a.kind, Kind::Node(NodeKind::Isolated, 0)) {
        policy.max_isolated_shift_px
    } else {
        policy.max_position_shift_px
    };
    if shift > limit {
        return Ok(None);
    }
    match (&a.cycle, &b.cycle) {
        (Some(a), Some(b)) => {
            let relative = policy.cycle_relative_tolerance;
            if (a.length - b.length).abs() > relative * a.length.min(b.length)
                || (a.radius - b.radius).abs() > relative * a.radius.min(b.radius)
            {
                return Ok(None);
            }
            let limit = policy
                .max_arm_shape_error_px
                .min(relative * a.radius.min(b.radius));
            if !shape_within(a, b, limit, work)? || !shape_within(b, a, limit, work)? {
                return Ok(None);
            }
        }
        (None, None) => {
            let mut arms = Vec::with_capacity(a.arms.len());
            for first in &a.arms {
                let mut compatible = Vec::new();
                for (id, second) in b.arms.iter().enumerate() {
                    work.charge(4)?;
                    if dot(first.direction, second.direction) < policy.max_arm_angle_radians.cos() {
                        continue;
                    }
                    let error = first
                        .samples
                        .iter()
                        .zip(&second.samples)
                        .map(|(&one, &two)| distance(one, two))
                        .fold(0.0, f64::max);
                    if error <= policy.max_arm_shape_error_px {
                        compatible.push((error, id));
                    }
                }
                compatible.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                arms.push(compatible.into_iter().map(|(_, id)| id).collect());
            }
            if assign(&arms, b.arms.len(), work)?
                .iter()
                .any(Option::is_none)
            {
                return Ok(None);
            }
        }
        _ => return Ok(None),
    }
    Ok(Some(shift))
}

fn shape_within(
    a: &Cycle,
    b: &Cycle,
    limit: f64,
    work: &mut Work,
) -> Result<bool, FingerprintError> {
    for &point in &a.samples {
        work.charge(b.points.len())?;
        if !b
            .points
            .windows(2)
            .any(|pair| point_segment_distance(point, pair[0], pair[1]) <= limit)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Deterministic augmenting paths maximize the number of compatible matches.
/// A variant feature can be consumed only once; no recursive depth is needed.
fn assign(
    adjacency: &[Vec<usize>],
    right_count: usize,
    work: &mut Work,
) -> Result<Vec<Option<usize>>, FingerprintError> {
    let mut matched: Vec<Option<usize>> = vec![None; adjacency.len()];
    let mut owner: Vec<Option<usize>> = vec![None; right_count];
    for root in 0..adjacency.len() {
        work.charge(adjacency.len().saturating_add(right_count))?;
        let mut visited = vec![false; adjacency.len()];
        let mut predecessor = vec![None; right_count];
        let mut queue = VecDeque::from([root]);
        visited[root] = true;
        let mut free = None;
        'search: while let Some(left) = queue.pop_front() {
            for &right in &adjacency[left] {
                work.charge(1)?;
                if predecessor[right].is_some() {
                    continue;
                }
                predecessor[right] = Some(left);
                if let Some(next) = owner[right] {
                    if !visited[next] {
                        visited[next] = true;
                        queue.push_back(next);
                    }
                } else {
                    free = Some(right);
                    break 'search;
                }
            }
        }
        while let Some(right) = free {
            let left = predecessor[right].expect("visited augmenting link");
            free = matched[left];
            matched[left] = Some(right);
            owner[right] = Some(left);
        }
    }
    Ok(matched)
}

fn interpolate(points: &[Vector], positions: &[f64], target: f64) -> Vector {
    let index = positions.partition_point(|&value| value <= target);
    if index == 0 {
        return points[0];
    }
    if index == points.len() {
        return *points.last().unwrap();
    }
    let span = positions[index] - positions[index - 1];
    let t = (target - positions[index - 1]) / span;
    [
        points[index - 1][0] + t * (points[index][0] - points[index - 1][0]),
        points[index - 1][1] + t * (points[index][1] - points[index - 1][1]),
    ]
}

fn point_segment_distance(point: Vector, start: Vector, end: Vector) -> f64 {
    let axis = sub(end, start);
    let norm = dot(axis, axis);
    let t = if norm > 0.0 {
        (dot(sub(point, start), axis) / norm).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(point, [start[0] + t * axis[0], start[1] + t * axis[1]])
}
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: Vector, b: Vector) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn length(a: Vector) -> f64 {
    a[0].hypot(a[1])
}
fn distance(a: Vector, b: Vector) -> f64 {
    length(sub(a, b))
}
