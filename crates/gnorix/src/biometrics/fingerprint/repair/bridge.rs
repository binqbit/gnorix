//! Tangent-compatible cubic bridge construction with bounded curvature.

use super::super::NodeKind;
use super::{FingerprintError, GapPolicy, Point, RidgeGraph};

const MAX_STEP_PX: f64 = 0.5;

pub(super) struct Endpoint {
    pub node: usize,
    pub edge: usize,
    pub position: Point,
    pub(super) tangent: (f64, f64),
    stable_tangent: (f64, f64),
}

pub(super) fn endpoints(
    graph: &RidgeGraph,
    incidence: &[Vec<usize>],
    policy: GapPolicy,
) -> Vec<Endpoint> {
    graph
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(node, item)| {
            if item.kind != NodeKind::Ending || incidence[node].len() != 1 {
                return None;
            }
            let edge_index = incidence[node][0];
            let edge = &graph.edges[edge_index];
            let reverse = edge.end == node;
            let position = *if reverse {
                edge.points.last()?
            } else {
                edge.points.first()?
            };
            let near = point_at(&edge.points, reverse, policy.tangent_support_px / 2.0)?;
            let far = point_at(&edge.points, reverse, policy.tangent_support_px)?;
            let stable_tangent = direction(far, position)?;
            let neighbor = if reverse {
                edge.points.iter().rev().skip(1).find(|&&p| p != position)
            } else {
                edge.points.iter().skip(1).find(|&&p| p != position)
            }?;
            let tangent = direction(*neighbor, position)?;
            let inner = direction(far, near)?;
            let outer = direction(near, position)?;
            if angle(inner, outer) > policy.max_tangent_error_radians
                || angle(tangent, stable_tangent) > policy.max_tangent_error_radians
            {
                return None;
            }
            Some(Endpoint {
                node,
                edge: edge_index,
                position,
                tangent,
                stable_tangent,
            })
        })
        .collect()
}

fn point_at(points: &[Point], reverse: bool, target: f64) -> Option<Point> {
    let point = |index| {
        points[if reverse {
            points.len() - 1 - index
        } else {
            index
        }]
    };
    let mut traversed = 0.0;
    for index in 1..points.len() {
        let (a, b) = (point(index - 1), point(index));
        let length = distance(a, b);
        if length > 0.0 && traversed + length >= target {
            let fraction = (target - traversed) / length;
            return Point::new(
                a.x() + fraction * (b.x() - a.x()),
                a.y() + fraction * (b.y() - a.y()),
            )
            .ok();
        }
        traversed += length;
    }
    None
}

pub(super) fn continuation(
    a: &Endpoint,
    b: &Endpoint,
    policy: GapPolicy,
) -> Result<Option<(Vec<Point>, f64)>, FingerprintError> {
    let gap = distance(a.position, b.position);
    if gap <= 0.0 || gap > policy.max_gap_px {
        return Ok(None);
    }
    let chord = direction(a.position, b.position).unwrap();
    let facing = angle(a.tangent, chord)
        .max(angle(b.tangent, (-chord.0, -chord.1)))
        .max(angle(a.stable_tangent, chord))
        .max(angle(b.stable_tangent, (-chord.0, -chord.1)));
    if facing > policy.max_tangent_error_radians {
        return Ok(None);
    }
    let controls = [
        a.position,
        Point::new(
            a.position.x() + a.tangent.0 * gap / 3.0,
            a.position.y() + a.tangent.1 * gap / 3.0,
        )
        .map_err(|_| FingerprintError::InvalidCurve("bridge control overflow"))?,
        Point::new(
            b.position.x() + b.tangent.0 * gap / 3.0,
            b.position.y() + b.tangent.1 * gap / 3.0,
        )
        .map_err(|_| FingerprintError::InvalidCurve("bridge control overflow"))?,
        b.position,
    ];
    if curvature_bound(controls, chord).is_none_or(|bound| bound > policy.max_curvature_per_px) {
        return Ok(None);
    }
    // A cubic derivative is a convex combination of three control differences.
    // This upper speed bound guarantees at most half a source pixel per step.
    let max_speed = controls
        .windows(2)
        .map(|pair| 3.0 * distance(pair[0], pair[1]))
        .fold(0.0, f64::max);
    let count = (max_speed / MAX_STEP_PX).ceil().max(2.0) as usize;
    let mut points = Vec::with_capacity(count + 1);
    for index in 0..=count {
        let t = index as f64 / count as f64;
        let u = 1.0 - t;
        let mut value = (0.0, 0.0);
        for (point, weight) in
            controls
                .into_iter()
                .zip([u.powi(3), 3.0 * u * u * t, 3.0 * u * t * t, t.powi(3)])
        {
            value.0 += point.x() * weight;
            value.1 += point.y() * weight;
        }
        let first = derivative(controls, t);
        let second = second_derivative(controls, t);
        let speed = first.0.hypot(first.1);
        let curvature = cross(first, second).abs() / speed.powi(3);
        // Positive chord projection rules out reversal and overshoot. Curvature
        // is bounded in measured-width units, including the attachment ends.
        if speed <= f64::EPSILON
            || dot(first, chord) <= 0.0
            || !curvature.is_finite()
            || curvature > policy.max_curvature_per_px
        {
            return Ok(None);
        }
        points.push(
            Point::new(value.0, value.1)
                .map_err(|_| FingerprintError::InvalidCurve("bridge point overflow"))?,
        );
    }
    points[0] = a.position;
    points[count] = b.position;
    Ok(Some((points, facing)))
}

fn curvature_bound(p: [Point; 4], chord: (f64, f64)) -> Option<f64> {
    let velocities = [0, 1, 2].map(|i| {
        (
            3.0 * (p[i + 1].x() - p[i].x()),
            3.0 * (p[i + 1].y() - p[i].y()),
        )
    });
    let accelerations = [0, 1].map(|i| {
        (
            2.0 * (velocities[i + 1].0 - velocities[i].0),
            2.0 * (velocities[i + 1].1 - velocities[i].1),
        )
    });
    let min_speed = velocities
        .into_iter()
        .map(|v| dot(v, chord))
        .fold(f64::INFINITY, f64::min);
    if min_speed <= 0.0 {
        return None;
    }
    // Both derivative curves are convex combinations of their control vectors.
    // The cross-product numerator is bounded by all control-vector pairs;
    // positive chord projection supplies a global lower bound on speed.
    let numerator = velocities
        .into_iter()
        .flat_map(|v| accelerations.into_iter().map(move |a| cross(v, a).abs()))
        .fold(0.0, f64::max);
    Some(numerator / min_speed.powi(3))
}

fn derivative(p: [Point; 4], t: f64) -> (f64, f64) {
    let weights = [3.0 * (1.0 - t).powi(2), 6.0 * (1.0 - t) * t, 3.0 * t * t];
    p.windows(2)
        .zip(weights)
        .fold((0.0, 0.0), |sum, (pair, weight)| {
            (
                sum.0 + weight * (pair[1].x() - pair[0].x()),
                sum.1 + weight * (pair[1].y() - pair[0].y()),
            )
        })
}

fn second_derivative(p: [Point; 4], t: f64) -> (f64, f64) {
    let dimension = |a: f64, b: f64, c: f64, d: f64| {
        6.0 * ((1.0 - t) * (c - 2.0 * b + a) + t * (d - 2.0 * c + b))
    };
    (
        dimension(p[0].x(), p[1].x(), p[2].x(), p[3].x()),
        dimension(p[0].y(), p[1].y(), p[2].y(), p[3].y()),
    )
}

pub(super) fn distance(a: Point, b: Point) -> f64 {
    (a.x() - b.x()).hypot(a.y() - b.y())
}
fn direction(a: Point, b: Point) -> Option<(f64, f64)> {
    let length = distance(a, b);
    (length > 0.0).then_some(((b.x() - a.x()) / length, (b.y() - a.y()) / length))
}
pub(super) fn dot(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
pub(super) fn cross(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.1 - a.1 * b.0
}
fn angle(a: (f64, f64), b: (f64, f64)) -> f64 {
    dot(a, b).clamp(-1.0, 1.0).acos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }

    #[test]
    fn cubic_attachment_follows_local_direction_instead_of_the_long_secant() {
        let angle = 0.1_f64;
        let a = Endpoint {
            node: 0,
            edge: 0,
            position: point(20.0, 20.0),
            tangent: (angle.cos(), angle.sin()),
            stable_tangent: (1.0, 0.0),
        };
        let b = Endpoint {
            node: 1,
            edge: 1,
            position: point(28.0, 20.0),
            tangent: (-angle.cos(), angle.sin()),
            stable_tangent: (-1.0, 0.0),
        };
        let (points, _) = continuation(&a, &b, GapPolicy::for_unit(8.0).unwrap())
            .unwrap()
            .unwrap();
        let first = direction(points[0], points[1]).unwrap();
        let last = direction(*points.last().unwrap(), points[points.len() - 2]).unwrap();
        assert!(super::angle(first, a.tangent) < 0.01);
        assert!(super::angle(last, b.tangent) < 0.01);
        assert!(super::angle(first, a.stable_tangent) > 0.08);
        assert!(
            points
                .windows(2)
                .all(|p| distance(p[0], p[1]) <= MAX_STEP_PX * (1.0 + 1e-12))
        );
        assert!(points.windows(2).all(|p| p[1].x() > p[0].x()));
    }

    #[test]
    fn rejects_a_short_cubic_that_requires_excessive_bending() {
        let angle = 0.3_f64;
        let a = Endpoint {
            node: 0,
            edge: 0,
            position: point(20.0, 20.0),
            tangent: (angle.cos(), angle.sin()),
            stable_tangent: (1.0, 0.0),
        };
        let b = Endpoint {
            node: 1,
            edge: 1,
            position: point(21.0, 20.0),
            tangent: (-angle.cos(), angle.sin()),
            stable_tangent: (-1.0, 0.0),
        };
        assert!(
            continuation(&a, &b, GapPolicy::for_unit(8.0).unwrap())
                .unwrap()
                .is_none()
        );
    }
}
