//! Constrained geometric smoothing of observed paths, without gap repair.
//!
//! A Gaussian target is evaluated at uniform arc-length stations, then mapped
//! back to the original vertices. A single blend fraction keeps each path
//! coherent. Swept source support and intersection checks constrain the blend;
//! failure retains the exact observed path and records why.

mod constraints;

use constraints::{
    SegmentIndex, preserves_intersections, preserves_support, preserves_swept_separation,
};

use super::{FingerprintError, NodeKind, RidgeGraph, RidgeMap};
use crate::biometrics::geometry::Point;

const MAX_POINTS: usize = 100_000;
const MAX_WORK: usize = 100_000_000;
const MAX_BACKOFFS: usize = 8;
const NUMERICAL_PX: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmoothingRejection {
    Support,
    Intersection,
    Degenerate,
}

/// Adjustment of one observed edge or cycle; no feature confidence is implied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmoothingAdjustment {
    pub max_displacement_px: f64,
    pub smoothing_applied: bool,
    /// Fraction of the unconstrained Gaussian displacement that was applied.
    pub applied_fraction: f64,
    /// Present only when the exact original path had to be retained.
    pub retained_reason: Option<SmoothingRejection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SmoothedGraph {
    pub graph: RidgeGraph,
    pub edge_adjustments: Vec<SmoothingAdjustment>,
    pub cycle_adjustments: Vec<SmoothingAdjustment>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmoothingControls {
    pub sample_spacing_px: f64,
    pub sigma_px: f64,
    pub max_displacement_px: f64,
}

impl SmoothingControls {
    pub fn for_unit(unit_px: f64) -> Result<Self, FingerprintError> {
        if !unit_px.is_finite() || unit_px <= 0.0 {
            return Err(FingerprintError::InvalidConfig(
                "smoothing unit must be finite and positive",
            ));
        }
        let controls = Self {
            sample_spacing_px: (unit_px / 8.0).min(1.0),
            sigma_px: unit_px / 2.0,
            max_displacement_px: unit_px / 4.0,
        };
        if controls.sample_spacing_px == 0.0 {
            return Err(FingerprintError::InvalidConfig(
                "smoothing unit is below numerical range",
            ));
        }
        Ok(controls)
    }
}

pub(super) fn smooth(
    graph: &RidgeGraph,
    support: &RidgeMap,
    unit_px: f64,
) -> Result<SmoothedGraph, FingerprintError> {
    let controls = SmoothingControls::for_unit(unit_px)?;
    let mut paths = graph
        .edges
        .iter()
        .map(|edge| {
            ObservedPath::new(
                &edge.points,
                edge.points.first() == edge.points.last(),
                support,
            )
        })
        .chain(
            graph
                .cycles
                .iter()
                .map(|cycle| ObservedPath::new(&cycle.points, true, support)),
        )
        .collect::<Result<Vec<_>, _>>()?;
    let mut work = Work::default();
    let mut index = SegmentIndex::default();
    for (id, edge) in graph.edges.iter().enumerate() {
        if let (Some(&first), Some(&last)) = (edge.points.first(), edge.points.last()) {
            index.ports.insert(id, [first, last]);
        }
    }
    for (id, path) in paths.iter().enumerate() {
        index.add_path(id, &path.points, path.closed, &mut work)?;
    }
    for (node, item) in graph.nodes.iter().enumerate() {
        if item.kind == NodeKind::Isolated {
            // A retained isolated source pixel is still an observed component.
            // It must not be swallowed by movement of a neighboring contour.
            index.add_path(paths.len() + node, &[item.position], true, &mut work)?;
        }
    }

    let mut adjustments = Vec::with_capacity(paths.len());
    for (id, path) in paths.iter_mut().enumerate() {
        let (points, adjustment) = smooth_path(path, id, support, controls, &index, &mut work)?;
        index.replace_path(id, &points, path.closed, &mut work)?;
        path.points = points;
        adjustments.push(adjustment);
    }

    let mut result = graph.clone();
    for (edge, path) in result.edges.iter_mut().zip(&paths) {
        edge.points = path.output_points();
    }
    for (cycle, path) in result.cycles.iter_mut().zip(&paths[graph.edges.len()..]) {
        cycle.points = path.output_points();
    }
    let cycle_adjustments = adjustments.split_off(graph.edges.len());
    Ok(SmoothedGraph {
        graph: result,
        edge_adjustments: adjustments,
        cycle_adjustments,
    })
}

struct ObservedPath {
    points: Vec<Point>,
    cumulative: Vec<f64>,
    length: f64,
    closed: bool,
    repeated_seam: bool,
}

impl ObservedPath {
    fn new(input: &[Point], closed: bool, support: &RidgeMap) -> Result<Self, FingerprintError> {
        if input.len() > MAX_POINTS {
            return Err(FingerprintError::WorkLimit);
        }
        if input.iter().any(|point| {
            point.x() < 0.0
                || point.y() < 0.0
                || point.x() > (support.width() - 1) as f64
                || point.y() > (support.height() - 1) as f64
        }) {
            return Err(FingerprintError::InvalidCurve(
                "smoothing path lies outside source",
            ));
        }
        let repeated_seam = closed && input.len() > 1 && input.first() == input.last();
        let points = input[..input.len() - usize::from(repeated_seam)].to_vec();
        let mut cumulative = vec![0.0];
        let mut length = 0.0;
        for (a, b) in segments(&points, closed) {
            let segment_length = distance(a, b);
            if segment_length == 0.0 || !segment_length.is_finite() {
                return Err(FingerprintError::InvalidCurve(
                    "smoothing path has unresolved segments",
                ));
            }
            length += segment_length;
            cumulative.push(length);
        }
        Ok(Self {
            points,
            cumulative,
            length,
            closed,
            repeated_seam,
        })
    }

    fn output_points(&self) -> Vec<Point> {
        let mut points = self.points.clone();
        if self.repeated_seam {
            points.push(points[0]);
        }
        points
    }

    fn at(&self, position: f64) -> Point {
        let segment = self
            .cumulative
            .partition_point(|&value| value <= position)
            .saturating_sub(1)
            .min(self.cumulative.len() - 2);
        let fraction = (position - self.cumulative[segment])
            / (self.cumulative[segment + 1] - self.cumulative[segment]);
        interpolate(
            self.points[segment],
            self.points[(segment + 1) % self.points.len()],
            fraction,
        )
    }
}

fn smooth_path(
    path: &ObservedPath,
    id: usize,
    support: &RidgeMap,
    controls: SmoothingControls,
    index: &SegmentIndex,
    work: &mut Work,
) -> Result<(Vec<Point>, SmoothingAdjustment), FingerprintError> {
    if path.points.len() < if path.closed { 3 } else { 2 } || path.length <= NUMERICAL_PX {
        return Ok(retain(path, Some(SmoothingRejection::Degenerate)));
    }
    let target = gaussian_target(path, controls, work)?;
    let movement = max_movement(&path.points, &target);
    if movement <= NUMERICAL_PX {
        return Ok(retain(path, None));
    }
    let mut fraction = (controls.max_displacement_px / movement).min(1.0);
    let mut reason = SmoothingRejection::Degenerate;
    for _ in 0..=MAX_BACKOFFS {
        let candidate = path
            .points
            .iter()
            .zip(&target)
            .map(|(&raw, &target)| interpolate(raw, target, fraction))
            .collect::<Vec<_>>();
        reason = if !preserves_shape(path, &candidate) {
            SmoothingRejection::Degenerate
        } else if !preserves_support(&path.points, &candidate, path.closed, support, work)? {
            SmoothingRejection::Support
        } else if !preserves_swept_separation(
            &path.points,
            &candidate,
            path.closed,
            id,
            index,
            work,
        )? || !preserves_intersections(&candidate, path.closed, id, index, work)?
        {
            SmoothingRejection::Intersection
        } else {
            return Ok((
                candidate,
                SmoothingAdjustment {
                    max_displacement_px: movement * fraction,
                    smoothing_applied: true,
                    applied_fraction: fraction,
                    retained_reason: None,
                },
            ));
        };
        fraction *= 0.5;
    }
    Ok(retain(path, Some(reason)))
}

fn retain(
    path: &ObservedPath,
    reason: Option<SmoothingRejection>,
) -> (Vec<Point>, SmoothingAdjustment) {
    (
        path.points.clone(),
        SmoothingAdjustment {
            max_displacement_px: 0.0,
            smoothing_applied: false,
            applied_fraction: 0.0,
            retained_reason: reason,
        },
    )
}

fn gaussian_target(
    path: &ObservedPath,
    controls: SmoothingControls,
    work: &mut Work,
) -> Result<Vec<Point>, FingerprintError> {
    let intervals = (path.length / controls.sample_spacing_px)
        .ceil()
        .max(if path.closed { 3.0 } else { 1.0 });
    if intervals > (MAX_POINTS - 1) as f64 {
        return Err(FingerprintError::WorkLimit);
    }
    let intervals = intervals as usize;
    let count = intervals + usize::from(!path.closed);
    let spacing = path.length / intervals as f64;
    let sigma_samples = controls.sigma_px / spacing;
    let radius = (4.0 * sigma_samples).ceil();
    if !radius.is_finite() || radius > MAX_WORK as f64 {
        return Err(FingerprintError::WorkLimit);
    }
    let radius = radius as usize;
    work.spend(
        count
            .checked_mul(2 * radius + 1)
            .ok_or(FingerprintError::WorkLimit)?,
    )?;
    let samples = (0..count)
        .map(|i| path.at(i as f64 * spacing))
        .collect::<Vec<_>>();
    let weights = (0..=radius)
        .map(|i| (-0.5 * (i as f64 / sigma_samples).powi(2)).exp())
        .collect::<Vec<_>>();
    let mass = 1.0 + 2.0 * weights.iter().skip(1).sum::<f64>();
    let origin = samples[0];
    let mut smoothed = Vec::with_capacity(count);
    for center in 0..count {
        let mut x = 0.0;
        let mut y = 0.0;
        for offset in -(radius as isize)..=radius as isize {
            let sample = extended_sample(&samples, center as isize + offset, path.closed);
            let weight = weights[offset.unsigned_abs()];
            x += weight * (sample.x() - origin.x());
            y += weight * (sample.y() - origin.y());
        }
        smoothed.push(point(origin.x() + x / mass, origin.y() + y / mass));
    }
    let mut target = path
        .cumulative
        .iter()
        .take(path.points.len())
        .map(|&position| {
            let station = (position / spacing).min(intervals as f64);
            let left = (station.floor() as usize).min(count - 1);
            let right = if path.closed {
                (left + 1) % count
            } else {
                (left + 1).min(count - 1)
            };
            interpolate(smoothed[left], smoothed[right], station - left as f64)
        })
        .collect::<Vec<_>>();
    if path.repeated_seam {
        // A self-edge attached to a junction is periodic geometrically, but its
        // shared port must stay attached to the unchanged graph node.
        let dx = path.points[0].x() - target[0].x();
        let dy = path.points[0].y() - target[0].y();
        for value in &mut target {
            *value = point(value.x() + dx, value.y() + dy);
        }
        target[0] = path.points[0];
    } else if !path.closed {
        target[0] = path.points[0];
        *target.last_mut().unwrap() = *path.points.last().unwrap();
    }
    Ok(target)
}

fn extended_sample(samples: &[Point], index: isize, closed: bool) -> Point {
    if closed {
        return samples[index.rem_euclid(samples.len() as isize) as usize];
    }
    let last = samples.len() - 1;
    let period = (2 * last) as isize;
    let repetitions = index.div_euclid(period) as f64;
    let phase = index.rem_euclid(period) as usize;
    let first_point = samples[0];
    let last_point = samples[last];
    let base = if phase <= last {
        samples[phase]
    } else {
        let reflected = samples[2 * last - phase];
        point(
            2.0 * last_point.x() - reflected.x(),
            2.0 * last_point.y() - reflected.y(),
        )
    };
    point(
        base.x() + repetitions * 2.0 * (last_point.x() - first_point.x()),
        base.y() + repetitions * 2.0 * (last_point.y() - first_point.y()),
    )
}

fn preserves_shape(path: &ObservedPath, candidate: &[Point]) -> bool {
    if segments(candidate, path.closed).any(|(a, b)| distance(a, b) <= NUMERICAL_PX) {
        return false;
    }
    if path.closed {
        let before = signed_area(&path.points);
        let after = signed_area(candidate);
        if before.abs() <= NUMERICAL_PX || after * before <= 0.0 || after.abs() < before.abs() / 2.0
        {
            return false;
        }
    }
    true
}

#[derive(Default)]
struct Work {
    used: usize,
}
impl Work {
    fn spend(&mut self, amount: usize) -> Result<(), FingerprintError> {
        self.used = self
            .used
            .checked_add(amount)
            .filter(|&sum| sum <= MAX_WORK)
            .ok_or(FingerprintError::WorkLimit)?;
        Ok(())
    }
}

fn segments(points: &[Point], closed: bool) -> impl Iterator<Item = (Point, Point)> + '_ {
    let count = points.len().saturating_sub(usize::from(!closed));
    (0..count).map(|i| (points[i], points[(i + 1) % points.len()]))
}
fn point(x: f64, y: f64) -> Point {
    Point::new(x, y).expect("bounded source-coordinate arithmetic")
}
fn interpolate(a: Point, b: Point, t: f64) -> Point {
    point(a.x() + t * (b.x() - a.x()), a.y() + t * (b.y() - a.y()))
}
fn distance(a: Point, b: Point) -> f64 {
    (a.x() - b.x()).hypot(a.y() - b.y())
}
fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b.x() - a.x()) * (c.y() - a.y()) - (b.y() - a.y()) * (c.x() - a.x())
}
fn signed_area(points: &[Point]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let origin = points[0];
    segments(points, true)
        .map(|(a, b)| cross(origin, a, b))
        .sum::<f64>()
        / 2.0
}
fn max_movement(a: &[Point], b: &[Point]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(&a, &b)| distance(a, b))
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests;
