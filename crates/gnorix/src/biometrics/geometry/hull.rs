//! Shared bounded convex hulls and inclusive orientation predicates.

use std::cmp::Ordering;

use super::{GeometryError, Point};

pub(crate) enum HullError {
    Geometry(GeometryError),
    WorkLimit,
}

pub(crate) fn convex_hull(
    points: &[Point],
    max_work: usize,
) -> Result<(Vec<usize>, usize), HullError> {
    let mut remaining = max_work;
    spend(&mut remaining, points.len())?;
    let mut ordered: Vec<_> = (0..points.len()).collect();
    ordered.sort_unstable_by(|&a, &b| {
        canonical(points[a].x())
            .total_cmp(&canonical(points[b].x()))
            .then(canonical(points[a].y()).total_cmp(&canonical(points[b].y())))
            .then(a.cmp(&b))
    });
    ordered.dedup_by(|a, b| points[*a] == points[*b]);
    if ordered.len() <= 2 {
        return Ok((ordered, max_work - remaining));
    }
    let mut lower = half(points, ordered.iter().copied(), &mut remaining)?;
    let mut upper = half(points, ordered.iter().rev().copied(), &mut remaining)?;
    lower.pop();
    upper.pop();
    lower.extend(upper);
    Ok((lower, max_work - remaining))
}

fn half(
    points: &[Point],
    ordered: impl Iterator<Item = usize>,
    remaining: &mut usize,
) -> Result<Vec<usize>, HullError> {
    let mut hull = Vec::new();
    for next in ordered {
        spend(remaining, 1)?;
        while hull.len() >= 2 {
            spend(remaining, 1)?;
            if orientation(
                points[hull[hull.len() - 2]],
                points[hull[hull.len() - 1]],
                points[next],
            )
            .map_err(HullError::Geometry)?
                == Ordering::Greater
            {
                break;
            }
            hull.pop();
        }
        hull.push(next);
    }
    Ok(hull)
}

/// The collinearity guard covers floating arithmetic, not a geometric tolerance.
pub(crate) fn orientation(a: Point, b: Point, c: Point) -> Result<Ordering, GeometryError> {
    let ab = Point::new(b.x() - a.x(), b.y() - a.y())?;
    let ac = Point::new(c.x() - a.x(), c.y() - a.y())?;
    let ab_scale = ab.x().abs().max(ab.y().abs());
    let ac_scale = ac.x().abs().max(ac.y().abs());
    if ab_scale == 0.0 || ac_scale == 0.0 {
        return Ok(Ordering::Equal);
    }
    let left = (ab.x() / ab_scale) * (ac.y() / ac_scale);
    let right = (ab.y() / ab_scale) * (ac.x() / ac_scale);
    let turn = left - right;
    let uncertainty = 8.0 * f64::EPSILON * left.abs().max(right.abs());
    Ok(if turn > uncertainty {
        Ordering::Greater
    } else if turn < -uncertainty {
        Ordering::Less
    } else {
        Ordering::Equal
    })
}

fn canonical(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}
fn spend(remaining: &mut usize, count: usize) -> Result<(), HullError> {
    *remaining = remaining.checked_sub(count).ok_or(HullError::WorkLimit)?;
    Ok(())
}
