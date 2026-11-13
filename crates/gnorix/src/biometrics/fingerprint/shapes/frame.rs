//! Hull-derived rotation frames with a virtual top-right bounding-box origin.

use crate::biometrics::geometry::{GeometryError, GridPoint, Point, TriangularGrid};

use super::{GridShapeError, WorkBudget};

/// All geometrically tied frames remain available for canonical selection.
pub(super) struct FrameCandidates {
    /// Input point indexes on the exterior, without repeated closing vertex.
    pub(super) hull: Vec<usize>,
    pub(super) candidates: Vec<FrameCandidate>,
}

pub(super) struct FrameCandidate {
    /// The exterior edge placed above the figure; absent for a single location.
    pub(super) edge: Option<[usize; 2]>,
    /// Virtual top-right corner in the input coordinate system.
    pub(super) origin: Point,
    /// Unit direction of increasing aligned x, without reflection.
    pub(super) x_axis: Point,
    /// Every input point, in input order, before grid rounding.
    pub(super) aligned: Vec<Point>,
    /// Every input point, in input order, on the unbounded signed lattice.
    pub(super) snapped: Vec<GridPoint>,
}

pub(super) fn candidate_frames(
    points: &[Point],
    spacing: f64,
    work: &mut WorkBudget,
) -> Result<FrameCandidates, GridShapeError> {
    if points.is_empty() {
        return Err(GridShapeError::InvalidInput(
            "a shape needs at least one point",
        ));
    }
    let grid =
        TriangularGrid::new(point(0.0, 0.0)?, spacing, 0.0).map_err(GridShapeError::Geometry)?;
    let hull = convex_hull(points, work)?;
    let edges = longest_edges(points, &hull, work)?;
    let mut candidates = Vec::new();
    if edges.is_empty() {
        candidates.push(align(points, None, grid, work)?);
    } else {
        for edge in edges {
            candidates.push(align(points, Some(edge), grid, work)?);
        }
    }
    Ok(FrameCandidates { hull, candidates })
}

fn convex_hull(points: &[Point], work: &mut WorkBudget) -> Result<Vec<usize>, GridShapeError> {
    use crate::biometrics::geometry::hull::{self, HullError};
    let (hull, used) = hull::convex_hull(points, work.remaining).map_err(|error| match error {
        HullError::Geometry(error) => GridShapeError::Geometry(error),
        HullError::WorkLimit => GridShapeError::WorkLimit,
    })?;
    work.spend(used)?;
    Ok(hull)
}

fn longest_edges(
    points: &[Point],
    hull: &[usize],
    work: &mut WorkBudget,
) -> Result<Vec<[usize; 2]>, GridShapeError> {
    if hull.len() == 1 {
        return Ok(Vec::new());
    }
    work.spend(hull.len())?;
    let mut edges = Vec::new();
    let mut longest = 0.0_f64;
    for position in 0..hull.len() {
        let edge = [hull[position], hull[(position + 1) % hull.len()]];
        let delta = difference(points[edge[1]], points[edge[0]])?;
        let length = finite(delta.x().hypot(delta.y()))?;
        longest = longest.max(length);
        edges.push((edge, length));
    }
    // Rigid transforms can perturb an exactly equal length by a few ulps.
    // This tie guard is arithmetic only; near-equal biological edges do not
    // acquire an additional matching tolerance here.
    let uncertainty = 32.0 * f64::EPSILON * longest;
    Ok(edges
        .into_iter()
        .filter_map(|(edge, length)| (longest - length <= uncertainty).then_some(edge))
        .collect())
}

fn align(
    points: &[Point],
    edge: Option<[usize; 2]>,
    grid: TriangularGrid,
    work: &mut WorkBudget,
) -> Result<FrameCandidate, GridShapeError> {
    let reference = points[edge.map_or(0, |[a, _]| a)];
    let x_axis = match edge {
        Some([a, b]) => {
            let delta = difference(points[b], points[a])?;
            let length = finite(delta.x().hypot(delta.y()))?;
            point(delta.x() / length, delta.y() / length)?
        }
        None => point(1.0, 0.0)?,
    };
    let y_axis = point(-x_axis.y(), x_axis.x())?;
    let mut aligned = project(points, reference, x_axis, y_axis, work)?;
    let right = aligned
        .iter()
        .map(|p| p.x())
        .fold(f64::NEG_INFINITY, f64::max);
    let top = aligned.iter().map(|p| p.y()).fold(f64::INFINITY, f64::min);
    let origin = point(
        reference.x() + x_axis.x() * right + y_axis.x() * top,
        reference.y() + x_axis.y() * right + y_axis.y() * top,
    )?;
    for position in &mut aligned {
        *position = point(position.x() - right, position.y() - top)?;
    }
    work.spend(points.len())?;
    let snapped = aligned
        .iter()
        .map(|&position| grid.nearest(position).map_err(GridShapeError::Geometry))
        .collect::<Result<_, _>>()?;
    Ok(FrameCandidate {
        edge,
        origin,
        x_axis,
        aligned,
        snapped,
    })
}

fn project(
    points: &[Point],
    reference: Point,
    x_axis: Point,
    y_axis: Point,
    work: &mut WorkBudget,
) -> Result<Vec<Point>, GridShapeError> {
    work.spend(points.len())?;
    points
        .iter()
        .map(|&position| {
            let relative = difference(position, reference)?;
            point(
                relative.x() * x_axis.x() + relative.y() * x_axis.y(),
                relative.x() * y_axis.x() + relative.y() * y_axis.y(),
            )
        })
        .collect()
}

fn coordinate(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

fn difference(a: Point, b: Point) -> Result<Point, GridShapeError> {
    point(a.x() - b.x(), a.y() - b.y())
}

fn point(x: f64, y: f64) -> Result<Point, GridShapeError> {
    Point::new(coordinate(x), coordinate(y)).map_err(GridShapeError::Geometry)
}

fn finite(value: f64) -> Result<f64, GridShapeError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(GridShapeError::Geometry(GeometryError::NonFinitePoint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(values: &[(f64, f64)]) -> Vec<Point> {
        values
            .iter()
            .map(|&(x, y)| Point::new(x, y).unwrap())
            .collect()
    }

    fn asymmetric() -> Vec<Point> {
        points(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (12.0, 5.0),
            (8.0, 9.0),
            (1.0, 7.0),
            (5.0, 4.0),
        ])
    }

    fn frames(points: &[Point], spacing: f64) -> FrameCandidates {
        candidate_frames(points, spacing, &mut WorkBudget::new(100_000)).unwrap()
    }

    fn key(candidate: &FrameCandidate) -> Vec<(i64, i64)> {
        let right = candidate
            .snapped
            .iter()
            .map(|p| 2 * i64::from(p.q) + i64::from(p.r))
            .max()
            .unwrap();
        let top = candidate
            .snapped
            .iter()
            .map(|p| i64::from(p.r))
            .min()
            .unwrap();
        let mut result: Vec<_> = candidate
            .snapped
            .iter()
            .map(|p| {
                (
                    2 * i64::from(p.q) + i64::from(p.r) - right,
                    i64::from(p.r) - top,
                )
            })
            .collect();
        result.sort_unstable();
        result
    }

    fn canonical(points: &[Point], spacing: f64) -> Vec<(i64, i64)> {
        frames(points, spacing)
            .candidates
            .iter()
            .map(key)
            .min()
            .unwrap()
    }

    fn transform(points: &[Point], angle: f64, shift: (f64, f64)) -> Vec<Point> {
        let (sin, cos) = angle.sin_cos();
        points
            .iter()
            .map(|p| {
                Point::new(
                    cos * p.x() - sin * p.y() + shift.0,
                    sin * p.x() + cos * p.y() + shift.1,
                )
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn rightmost_point_can_lie_below_the_longest_top_edge() {
        let source = asymmetric();
        let result = frames(&source, 1.3);
        assert_eq!(result.hull, vec![0, 1, 2, 3, 4]);
        assert_eq!(result.candidates.len(), 1);
        let frame = &result.candidates[0];
        assert_eq!(frame.edge, Some([0, 1]));
        assert_eq!(frame.origin, Point::new(12.0, 0.0).unwrap());
        assert!(!source.contains(&frame.origin));
        assert_eq!(frame.x_axis, Point::new(1.0, 0.0).unwrap());
        assert_eq!(frame.aligned[2], Point::new(0.0, 5.0).unwrap());
        assert_eq!(frame.aligned[1], Point::new(-2.0, 0.0).unwrap());
        assert_eq!(frame.aligned[5], Point::new(-7.0, 4.0).unwrap());
        assert_eq!(frame.snapped.len(), source.len());
        assert!(frame.aligned.iter().all(|p| p.x() <= 0.0 && p.y() >= 0.0));
    }

    #[test]
    fn rotated_translated_and_permuted_inputs_have_the_same_canonical_geometry() {
        let source = asymmetric();
        let expected = canonical(&source, 1.3);
        for angle in [0.17, 0.61, 1.3, 2.4, -0.83] {
            let mut moved = transform(&source, angle, (113.5, -87.25));
            assert_eq!(canonical(&moved, 1.3), expected);
            moved.reverse();
            assert_eq!(canonical(&moved, 1.3), expected);
        }
    }

    #[test]
    fn equal_longest_edges_remain_candidates_for_geometric_tie_breaking() {
        let source = points(&[(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0), (2.1, 3.3)]);
        let expected = canonical(&source, 1.7);
        assert_eq!(frames(&source, 1.7).candidates.len(), 4);
        for angle in [0.31, -1.7] {
            let moved = transform(&source, angle, (41.0, -53.0));
            assert_eq!(frames(&moved, 1.7).candidates.len(), 4);
            assert_eq!(canonical(&moved, 1.7), expected);
        }
    }

    #[test]
    fn normalization_does_not_reflect_asymmetric_geometry() {
        let source = asymmetric();
        let mirrored: Vec<_> = source
            .iter()
            .map(|p| Point::new(-p.x(), p.y()).unwrap())
            .collect();
        assert_ne!(canonical(&source, 1.3), canonical(&mirrored, 1.3));
    }

    #[test]
    fn one_location_keeps_every_feature_record_without_inventing_an_edge() {
        let source = points(&[(7.0, -3.0), (7.0, -3.0)]);
        let result = frames(&source, 2.0);
        assert_eq!(result.hull.len(), 1);
        assert_eq!(result.candidates.len(), 1);
        let frame = &result.candidates[0];
        assert_eq!(frame.edge, None);
        assert_eq!(frame.origin, source[0]);
        assert_eq!(frame.snapped, vec![GridPoint { q: 0, r: 0 }; 2]);
    }

    #[test]
    fn collinear_geometry_keeps_both_axis_directions_and_interior_points() {
        let source = points(&[(0.0, 0.0), (2.0, 0.0), (9.0, 0.0), (2.0, 0.0)]);
        let expected = canonical(&source, 1.3);
        for angle in [0.0, 0.37, -1.23] {
            let moved = transform(&source, angle, (14.0, -3.0));
            let result = frames(&moved, 1.3);
            assert_eq!(result.hull.len(), 2);
            assert_eq!(result.candidates.len(), 2);
            assert!(
                result
                    .candidates
                    .iter()
                    .all(|frame| frame.snapped.len() == 4)
            );
            assert_eq!(canonical(&moved, 1.3), expected);
        }
    }

    #[test]
    fn changing_pixel_units_with_cursor_size_preserves_grid_geometry() {
        let source = asymmetric();
        let scaled: Vec<_> = source
            .iter()
            .map(|p| Point::new(p.x() * 3.0, p.y() * 3.0).unwrap())
            .collect();
        assert_eq!(canonical(&source, 1.3), canonical(&scaled, 3.9));
    }

    #[test]
    fn hull_orientation_does_not_depend_on_area_product_representability() {
        for unit in [1e-200, 1e200] {
            let source = points(&[(0.0, 0.0), (unit, 0.0), (0.0, unit)]);
            let result = frames(&source, unit);
            assert_eq!(result.hull.len(), 3);
            assert_eq!(result.candidates.len(), 1);
        }
    }

    #[test]
    fn work_limits_fail_instead_of_returning_a_partial_frame_set() {
        let source = asymmetric();
        assert!(matches!(
            candidate_frames(&source, 1.3, &mut WorkBudget::new(0)),
            Err(GridShapeError::WorkLimit)
        ));
        assert!(matches!(
            candidate_frames(&source, 1.3, &mut WorkBudget::new(source.len())),
            Err(GridShapeError::WorkLimit)
        ));
    }

    #[test]
    fn invalid_input_and_numeric_overflow_are_explicit_errors() {
        assert!(matches!(
            candidate_frames(&[], 1.0, &mut WorkBudget::new(100)),
            Err(GridShapeError::InvalidInput(_))
        ));
        assert!(matches!(
            candidate_frames(&asymmetric(), 0.0, &mut WorkBudget::new(1000)),
            Err(GridShapeError::Geometry(GeometryError::InvalidSpacing))
        ));
        let extreme = points(&[(-f64::MAX, 0.0), (f64::MAX, 0.0)]);
        assert!(matches!(
            candidate_frames(&extreme, 1.0, &mut WorkBudget::new(1000)),
            Err(GridShapeError::Geometry(GeometryError::NonFinitePoint))
        ));
    }
}
