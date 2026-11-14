//! Exact swept-cell support and spatially indexed curve intersections.

use std::collections::HashMap;

use super::{
    FingerprintError, Point, RidgeMap, Work, cross, distance, interpolate, point, segments,
    signed_area,
};

const GRID_CELL_PX: f64 = 8.0;

pub(super) fn preserves_support(
    original: &[Point],
    candidate: &[Point],
    closed: bool,
    support: &RidgeMap,
    work: &mut Work,
) -> Result<bool, FingerprintError> {
    for ((a, b), (c, d)) in segments(original, closed).zip(segments(candidate, closed)) {
        if [c, d].iter().any(|p| {
            p.x() < 0.0
                || p.y() < 0.0
                || p.x() > (support.width() - 1) as f64
                || p.y() > (support.height() - 1) as f64
        }) {
            return Ok(false);
        }
        let hull = convex_hull([a, b, c, d]);
        let bounds = Bounds::of(&hull);
        let left = (bounds.left - 0.5).ceil().max(0.0) as usize;
        let right = (bounds.right + 0.5)
            .floor()
            .min((support.width() - 1) as f64) as usize;
        let top = (bounds.top - 0.5).ceil().max(0.0) as usize;
        let bottom = (bounds.bottom + 0.5)
            .floor()
            .min((support.height() - 1) as f64) as usize;
        for y in top..=bottom {
            for x in left..=right {
                work.spend(1)?;
                let index = y * support.width() + x;
                if support.usable()[index] && support.ridges()[index] {
                    continue;
                }
                let cell = Bounds {
                    left: x as f64 - 0.5,
                    right: x as f64 + 0.5,
                    top: y as f64 - 0.5,
                    bottom: y as f64 + 0.5,
                };
                if segment_enters_box(c, d, cell) || polygon_overlaps_box(&hull, cell) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

pub(super) fn preserves_swept_separation(
    original: &[Point],
    candidate: &[Point],
    closed: bool,
    id: usize,
    obstacles: &SegmentIndex,
    work: &mut Work,
) -> Result<bool, FingerprintError> {
    for ((a, b), (c, d)) in segments(original, closed).zip(segments(candidate, closed)) {
        let hull = convex_hull([a, b, c, d]);
        if hull.len() < 3 || signed_area(&hull).abs() <= 1e-12 {
            continue;
        }
        let bounds = Bounds::of(&hull);
        for other in obstacles.query(
            point(bounds.left, bounds.top),
            point(bounds.right, bounds.bottom),
            work,
        )? {
            if other.path != id && segment_enters_polygon(other.a, other.b, &hull) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn segment_enters_polygon(a: Point, b: Point, polygon: &[Point]) -> bool {
    let mut low: f64 = 0.0;
    let mut high: f64 = 1.0;
    for (start, end) in segments(polygon, true) {
        let first = cross(start, end, a);
        let change = cross(start, end, b) - first;
        if change == 0.0 {
            if first <= 1e-12 {
                return false;
            }
        } else {
            let crossing = (1e-12 - first) / change;
            if change > 0.0 {
                low = low.max(crossing);
            } else {
                high = high.min(crossing);
            }
        }
    }
    high - low > 1e-12
}

pub(super) fn preserves_intersections(
    points: &[Point],
    closed: bool,
    id: usize,
    obstacles: &SegmentIndex,
    work: &mut Work,
) -> Result<bool, FingerprintError> {
    let mut own = SegmentIndex::default();
    let segment_count = points.len() - usize::from(!closed);
    for (ordinal, (a, b)) in segments(points, closed).enumerate() {
        for other in obstacles.query(a, b, work)? {
            let allowed_port = [a, b].into_iter().any(|p| {
                (p == other.a || p == other.b)
                    && obstacles
                        .ports
                        .get(&id)
                        .is_some_and(|ports| ports.contains(&p))
                    && obstacles
                        .ports
                        .get(&other.path)
                        .is_some_and(|ports| ports.contains(&p))
            });
            if other.path != id && forbidden_intersection(a, b, other.a, other.b, allowed_port) {
                return Ok(false);
            }
        }
        for other in own.query(a, b, work)? {
            let adjacent = other.ordinal + 1 == ordinal
                || (closed && other.ordinal == 0 && ordinal + 1 == segment_count);
            if forbidden_intersection(a, b, other.a, other.b, adjacent) {
                return Ok(false);
            }
        }
        own.add_segment(
            Segment {
                a,
                b,
                path: id,
                ordinal,
                active: true,
            },
            work,
        )?;
    }
    Ok(true)
}

#[derive(Clone, Copy)]
struct Segment {
    a: Point,
    b: Point,
    path: usize,
    ordinal: usize,
    active: bool,
}

#[derive(Default)]
pub(super) struct SegmentIndex {
    segments: Vec<Segment>,
    cells: HashMap<(i64, i64), Vec<usize>>,
    by_path: HashMap<usize, Vec<usize>>,
    pub(super) ports: HashMap<usize, [Point; 2]>,
}

impl SegmentIndex {
    pub(super) fn add_path(
        &mut self,
        id: usize,
        points: &[Point],
        closed: bool,
        work: &mut Work,
    ) -> Result<(), FingerprintError> {
        if !closed && let (Some(&first), Some(&last)) = (points.first(), points.last()) {
            self.ports.insert(id, [first, last]);
        }
        for (ordinal, (a, b)) in segments(points, closed).enumerate() {
            self.add_segment(
                Segment {
                    a,
                    b,
                    path: id,
                    ordinal,
                    active: true,
                },
                work,
            )?;
        }
        Ok(())
    }

    pub(super) fn replace_path(
        &mut self,
        id: usize,
        points: &[Point],
        closed: bool,
        work: &mut Work,
    ) -> Result<(), FingerprintError> {
        if let Some(old) = self.by_path.remove(&id) {
            for index in old {
                self.segments[index].active = false;
            }
        }
        self.add_path(id, points, closed, work)
    }

    fn add_segment(&mut self, segment: Segment, work: &mut Work) -> Result<(), FingerprintError> {
        let id = self.segments.len();
        for cell in grid_cells(segment.a, segment.b, work)? {
            self.cells.entry(cell).or_default().push(id);
        }
        self.by_path.entry(segment.path).or_default().push(id);
        self.segments.push(segment);
        Ok(())
    }

    fn query(&self, a: Point, b: Point, work: &mut Work) -> Result<Vec<Segment>, FingerprintError> {
        let mut indices: Vec<usize> = Vec::new();
        for cell in grid_cells(a, b, work)? {
            if let Some(bucket) = self.cells.get(&cell) {
                indices.extend(bucket.iter().copied());
            }
        }
        work.spend(indices.len())?;
        indices.sort_unstable();
        indices.dedup();
        Ok(indices
            .into_iter()
            .filter_map(|index| {
                let segment = self.segments[index];
                segment.active.then_some(segment)
            })
            .collect())
    }
}

fn grid_cells(a: Point, b: Point, work: &mut Work) -> Result<Vec<(i64, i64)>, FingerprintError> {
    let bounds = Bounds::of(&[a, b]);
    let left = (bounds.left / GRID_CELL_PX).floor() as i64;
    let right = (bounds.right / GRID_CELL_PX).floor() as i64;
    let top = (bounds.top / GRID_CELL_PX).floor() as i64;
    let bottom = (bounds.bottom / GRID_CELL_PX).floor() as i64;
    let count = (right - left + 1)
        .checked_mul(bottom - top + 1)
        .ok_or(FingerprintError::WorkLimit)?;
    work.spend(usize::try_from(count).map_err(|_| FingerprintError::WorkLimit)?)?;
    Ok((top..=bottom)
        .flat_map(|y| (left..=right).map(move |x| (x, y)))
        .collect())
}

#[derive(Clone, Copy)]
struct Bounds {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}
impl Bounds {
    fn of(points: &[Point]) -> Self {
        Self {
            left: points.iter().map(|p| p.x()).fold(f64::INFINITY, f64::min),
            right: points
                .iter()
                .map(|p| p.x())
                .fold(f64::NEG_INFINITY, f64::max),
            top: points.iter().map(|p| p.y()).fold(f64::INFINITY, f64::min),
            bottom: points
                .iter()
                .map(|p| p.y())
                .fold(f64::NEG_INFINITY, f64::max),
        }
    }
}

fn segment_enters_box(a: Point, b: Point, bounds: Bounds) -> bool {
    let mut low: f64 = 0.0;
    let mut high: f64 = 1.0;
    for (start, delta, min, max) in [
        (a.x(), b.x() - a.x(), bounds.left, bounds.right),
        (a.y(), b.y() - a.y(), bounds.top, bounds.bottom),
    ] {
        if delta == 0.0 {
            if start < min || start > max {
                return false;
            }
        } else {
            let first = (min - start) / delta;
            let second = (max - start) / delta;
            low = low.max(first.min(second));
            high = high.min(first.max(second));
        }
    }
    high - low > 1e-12
}

fn polygon_overlaps_box(polygon: &[Point], bounds: Bounds) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut clipped = polygon.to_vec();
    for (axis, limit, greater) in [
        (0, bounds.left, true),
        (0, bounds.right, false),
        (1, bounds.top, true),
        (1, bounds.bottom, false),
    ] {
        if clipped.is_empty() {
            return false;
        }
        let mut output = Vec::new();
        for index in 0..clipped.len() {
            let a = clipped[index];
            let b = clipped[(index + 1) % clipped.len()];
            let coord = |p: Point| if axis == 0 { p.x() } else { p.y() };
            let inside = |p: Point| {
                if greater {
                    coord(p) >= limit
                } else {
                    coord(p) <= limit
                }
            };
            if inside(a) {
                output.push(a);
            }
            if inside(a) != inside(b) {
                output.push(interpolate(
                    a,
                    b,
                    (limit - coord(a)) / (coord(b) - coord(a)),
                ));
            }
        }
        clipped = output;
    }
    signed_area(&clipped).abs() > 1e-12
}

fn convex_hull(mut points: [Point; 4]) -> Vec<Point> {
    points.sort_by(|a, b| a.x().total_cmp(&b.x()).then(a.y().total_cmp(&b.y())));
    let mut hull = Vec::new();
    for &p in &points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len();
    for &p in points[..3].iter().rev() {
        while hull.len() > lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    hull.dedup();
    hull
}

pub(super) fn forbidden_intersection(
    a: Point,
    b: Point,
    c: Point,
    d: Point,
    allow_shared: bool,
) -> bool {
    let ab_c = orientation(a, b, c);
    let ab_d = orientation(a, b, d);
    let cd_a = orientation(c, d, a);
    let cd_b = orientation(c, d, b);
    if ab_c * ab_d == -1 && cd_a * cd_b == -1 {
        return true;
    }
    let shared = a == c || a == d || b == c || b == d;
    if shared && (ab_c != 0 || ab_d != 0) {
        return !allow_shared;
    }
    if shared {
        // A shared endpoint is allowed, but a collinear overlap is not. The
        // allowance covers arithmetic in these distances, not a pixel margin.
        let sum = distance(a, b) + distance(c, d);
        let extent = [
            distance(a, c),
            distance(a, d),
            distance(b, c),
            distance(b, d),
        ]
        .into_iter()
        .fold(0.0, f64::max);
        let error = 64.0 * f64::EPSILON * (sum + extent);
        return !allow_shared || extent < sum - error;
    }
    on_segment(a, c, d) || on_segment(b, c, d) || on_segment(c, a, b) || on_segment(d, a, b)
}

fn orientation(a: Point, b: Point, c: Point) -> i8 {
    let left = (b.x() - a.x()) * (c.y() - a.y());
    let right = (b.y() - a.y()) * (c.x() - a.x());
    let determinant = left - right;
    // Error scales with the determinant's product terms, so a resolved crossing
    // of short subpixel segments is not erased by an absolute area threshold.
    // Near-collinear uncertainty is treated as possible contact conservatively.
    let error = 16.0 * f64::EPSILON * (left.abs() + right.abs());
    if determinant > error {
        1
    } else if determinant < -error {
        -1
    } else {
        0
    }
}

fn on_segment(p: Point, start: Point, end: Point) -> bool {
    if orientation(start, end, p) != 0 {
        return false;
    }
    let scale = [p.x(), p.y(), start.x(), start.y(), end.x(), end.y()]
        .into_iter()
        .map(f64::abs)
        .fold(0.0, f64::max);
    let error = 32.0 * f64::EPSILON * scale;
    p.x() >= start.x().min(end.x()) - error
        && p.x() <= start.x().max(end.x()) + error
        && p.y() >= start.y().min(end.y()) - error
        && p.y() <= start.y().max(end.y()) + error
}
