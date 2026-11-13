use std::{error::Error, fmt};

/// A finite point in a caller-defined two-dimensional coordinate frame.
///
/// Units and axis directions must agree with those of the consuming algorithm.
/// Coordinates are not implicitly pixels or physical lengths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    x: f64,
    y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Result<Self, GeometryError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(GeometryError::NonFinitePoint);
        }

        Ok(Self { x, y })
    }

    pub fn x(self) -> f64 {
        self.x
    }

    pub fn y(self) -> f64 {
        self.y
    }
}

/// Integer coordinates in a triangular lattice's two-axis basis.
///
/// This prerelease mathematical representation does not select a canonical
/// biometric feature encoding or a policy for image boundaries.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GridPoint {
    pub q: i32,
    pub r: i32,
}

impl GridPoint {
    /// Returns all six nearest lattice neighbors in positive angular order.
    ///
    /// The first offset is `(1, 0)`, followed by `(0, 1)`, `(-1, 1)`, `(-1, 0)`,
    /// `(0, -1)` and `(1, -1)`. The complete neighborhood must fit in `i32`;
    /// boundary overflow returns an error rather than a partial neighborhood.
    /// Image-domain clipping is the caller's separate responsibility.
    pub fn neighbors(self) -> Result<[Self; 6], GeometryError> {
        Ok([
            self.checked_offset(1, 0)?,
            self.checked_offset(0, 1)?,
            self.checked_offset(-1, 1)?,
            self.checked_offset(-1, 0)?,
            self.checked_offset(0, -1)?,
            self.checked_offset(1, -1)?,
        ])
    }

    fn checked_offset(self, q: i32, r: i32) -> Result<Self, GeometryError> {
        Ok(Self {
            q: self
                .q
                .checked_add(q)
                .ok_or(GeometryError::CoordinateOverflow)?,
            r: self
                .r
                .checked_add(r)
                .ok_or(GeometryError::CoordinateOverflow)?,
        })
    }
}

/// A triangular point lattice with an explicit origin, spacing and rotation.
///
/// The unrotated basis is `e1 = (d, 0)`, `e2 = (d/2, sqrt(3)*d/2)` for spacing
/// `d`. A position is `origin + R(rotation) * (q*e1 + r*e2)`. Positive rotation
/// turns the positive x axis toward the positive y axis. Thus it appears
/// counterclockwise with y pointing up and clockwise in a conventional image
/// frame with y pointing down.
///
/// Origin and spacing use the same caller-selected units. These are prerelease
/// mathematical conventions; no spacing, phase, sensor scale, image domain,
/// registration algorithm or tolerance is selected by this type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangularGrid {
    origin: Point,
    spacing: f64,
    rotation_radians: f64,
}

impl TriangularGrid {
    pub fn new(origin: Point, spacing: f64, rotation_radians: f64) -> Result<Self, GeometryError> {
        if !spacing.is_finite() || spacing <= 0.0 {
            return Err(GeometryError::InvalidSpacing);
        }

        if !rotation_radians.is_finite() {
            return Err(GeometryError::InvalidRotation);
        }

        Ok(Self {
            origin,
            spacing,
            rotation_radians,
        })
    }

    pub fn origin(self) -> Point {
        self.origin
    }

    pub fn spacing(self) -> f64 {
        self.spacing
    }

    pub fn rotation_radians(self) -> f64 {
        self.rotation_radians
    }

    /// Computes the point position, rejecting nonfinite floating-point results.
    ///
    /// Normal floating-point rounding applies; the type does not guarantee
    /// distinguishable positions when the spacing is tiny relative to origin.
    pub fn position(self, point: GridPoint) -> Result<Point, GeometryError> {
        let local = lattice_position(point, self.spacing)?;

        rotate_and_translate(local, self.rotation_radians, self.origin)
    }

    /// Finds the Euclidean-nearest node of the unbounded lattice.
    ///
    /// Negative coordinates are retained. Exact floating-point distance ties
    /// choose the lexicographically smallest `(q, r)`. As with `position`,
    /// floating-point rounding applies, including near Voronoi boundaries.
    /// A nearest node outside `i32` returns an error; it is never clipped to a
    /// farther representable node.
    pub fn nearest(self, point: Point) -> Result<GridPoint, GeometryError> {
        let local = inverse_grid_frame(point, self)?;

        nearest_lattice_node(local)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    NonFinitePoint,
    InvalidSpacing,
    InvalidRotation,
    CoordinateOverflow,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinitePoint => formatter.write_str("point coordinates must be finite"),
            Self::InvalidSpacing => formatter.write_str("grid spacing must be finite and positive"),
            Self::InvalidRotation => formatter.write_str("grid rotation must be finite"),
            Self::CoordinateOverflow => {
                formatter.write_str("lattice coordinates exceed the integer coordinate range")
            }
        }
    }
}

impl Error for GeometryError {}

fn inverse_grid_frame(point: Point, grid: TriangularGrid) -> Result<Point, GeometryError> {
    let delta = Point::new(
        (point.x - grid.origin.x) / grid.spacing,
        (point.y - grid.origin.y) / grid.spacing,
    )?;
    let (sin, cos) = grid.rotation_radians.sin_cos();

    Point::new(
        cos * delta.x + sin * delta.y,
        -sin * delta.x + cos * delta.y,
    )
}

fn nearest_lattice_node(local: Point) -> Result<GridPoint, GeometryError> {
    let row_height = 3.0_f64.sqrt() / 2.0;
    let fractional_r = local.y / row_height;
    let fractional_q = local.x - fractional_r / 2.0;
    // The nearest node differs by less than one in either axial coordinate.
    // Keep a whole-node margin to compare the true unbounded nearest node,
    // including candidates just outside the representable integer domain.
    let lower = f64::from(i32::MIN) - 1.0;
    let upper = f64::from(i32::MAX) + 1.0;
    if !(lower..=upper).contains(&fractional_q) || !(lower..=upper).contains(&fractional_r) {
        return Err(GeometryError::CoordinateOverflow);
    }

    // The covering radius is 1/sqrt(3), smaller than the row height. Thus only
    // the two adjacent rows can contain the nearest node; each needs only its
    // two adjacent horizontal nodes. Integer rounding of q and r separately
    // would not respect Euclidean distance in the oblique lattice basis.
    let first_row = fractional_r.floor() as i64;
    let mut best = (i64::MAX, i64::MAX);
    let mut best_distance = f64::INFINITY;
    for r in [first_row, first_row + 1] {
        let row_x = local.x - r as f64 / 2.0;
        let first_column = row_x.floor() as i64;
        for q in [first_column, first_column + 1] {
            let dx = row_x - q as f64;
            let dy = local.y - row_height * r as f64;
            let distance = dx * dx + dy * dy;
            if distance < best_distance || (distance == best_distance && (q, r) < best) {
                best = (q, r);
                best_distance = distance;
            }
        }
    }

    Ok(GridPoint {
        q: i32::try_from(best.0).map_err(|_| GeometryError::CoordinateOverflow)?,
        r: i32::try_from(best.1).map_err(|_| GeometryError::CoordinateOverflow)?,
    })
}

fn lattice_position(point: GridPoint, spacing: f64) -> Result<Point, GeometryError> {
    let q = f64::from(point.q);
    let r = f64::from(point.r);

    Point::new(
        spacing * (q + r / 2.0),
        spacing * ((3.0_f64.sqrt() / 2.0) * r),
    )
}

fn rotate_and_translate(
    point: Point,
    rotation_radians: f64,
    origin: Point,
) -> Result<Point, GeometryError> {
    let (sin, cos) = rotation_radians.sin_cos();

    Point::new(
        origin.x + cos * point.x - sin * point.y,
        origin.y + sin * point.x + cos * point.y,
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, f64::consts::FRAC_PI_2};

    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
    }

    #[test]
    fn rejects_nonfinite_points() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(Point::new(value, 0.0), Err(GeometryError::NonFinitePoint));
            assert_eq!(Point::new(0.0, value), Err(GeometryError::NonFinitePoint));
        }
    }

    #[test]
    fn rejects_invalid_grid_parameters() {
        let origin = Point::new(0.0, 0.0).unwrap();

        for spacing in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                TriangularGrid::new(origin, spacing, 0.0),
                Err(GeometryError::InvalidSpacing)
            );
        }

        for rotation in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                TriangularGrid::new(origin, 1.0, rotation),
                Err(GeometryError::InvalidRotation)
            );
        }
    }

    #[test]
    fn six_distinct_neighbors_are_equidistant() {
        let grid = TriangularGrid::new(Point::new(7.0, -3.0).unwrap(), 2.5, 0.37).unwrap();
        let center = GridPoint { q: 3, r: -2 };
        let center_position = grid.position(center).unwrap();
        let neighbors = center.neighbors().unwrap();

        assert_eq!(neighbors.into_iter().collect::<HashSet<_>>().len(), 6);

        for neighbor in neighbors {
            let position = grid.position(neighbor).unwrap();
            let distance =
                (position.x() - center_position.x()).hypot(position.y() - center_position.y());

            assert_close(distance, grid.spacing());
            assert!(neighbor.neighbors().unwrap().contains(&center));
        }
    }

    #[test]
    fn neighbor_order_follows_the_documented_basis() {
        assert_eq!(
            GridPoint { q: 0, r: 0 }.neighbors().unwrap(),
            [
                GridPoint { q: 1, r: 0 },
                GridPoint { q: 0, r: 1 },
                GridPoint { q: -1, r: 1 },
                GridPoint { q: -1, r: 0 },
                GridPoint { q: 0, r: -1 },
                GridPoint { q: 1, r: -1 },
            ]
        );
    }

    #[test]
    fn maps_the_rotated_translated_basis() {
        let origin = Point::new(10.0, -4.0).unwrap();
        let grid = TriangularGrid::new(origin, 2.0, FRAC_PI_2).unwrap();
        let first_axis = grid.position(GridPoint { q: 1, r: 0 }).unwrap();
        let second_axis = grid.position(GridPoint { q: 0, r: 1 }).unwrap();

        assert_eq!(grid.origin(), origin);
        assert_eq!(grid.rotation_radians(), FRAC_PI_2);
        assert_eq!(grid.position(GridPoint { q: 0, r: 0 }).unwrap(), origin);
        assert_close(first_axis.x(), 10.0);
        assert_close(first_axis.y(), -2.0);
        assert_close(second_axis.x(), 10.0 - 3.0_f64.sqrt());
        assert_close(second_axis.y(), -3.0);
    }

    #[test]
    fn snaps_by_euclidean_distance_instead_of_rounding_each_axial_coordinate() {
        let grid = TriangularGrid::new(Point::new(0.0, 0.0).unwrap(), 1.0, 0.0).unwrap();
        let y = (3.0_f64.sqrt() / 2.0) * 0.49;

        assert_eq!(
            grid.nearest(Point::new(0.48 + 0.49 / 2.0, y).unwrap()),
            Ok(GridPoint { q: 0, r: 1 })
        );
        assert_eq!(
            grid.nearest(Point::new(-0.48 - 0.49 / 2.0, -y).unwrap()),
            Ok(GridPoint { q: 0, r: -1 })
        );
    }

    #[test]
    fn exact_boundary_ties_choose_lexicographic_coordinates_without_an_epsilon() {
        let grid = TriangularGrid::new(Point::new(0.0, 0.0).unwrap(), 1.0, 0.0).unwrap();
        let center = GridPoint { q: 0, r: 0 };
        for neighbor in center.neighbors().unwrap() {
            let position = grid.position(neighbor).unwrap();
            let midpoint = Point::new(position.x() / 2.0, position.y() / 2.0).unwrap();
            assert_eq!(grid.nearest(midpoint), Ok(center.min(neighbor)));
        }

        assert_eq!(
            grid.nearest(Point::new(0.5_f64.next_up(), 0.0).unwrap()),
            Ok(GridPoint { q: 1, r: 0 })
        );
        assert_eq!(
            grid.nearest(Point::new(0.5_f64.next_down(), 0.0).unwrap()),
            Ok(center)
        );
    }

    #[test]
    fn snapping_preserves_signed_nodes_in_rotated_translated_scaled_frames() {
        for rotation in [0.0, 0.37, -1.1, FRAC_PI_2] {
            let grid =
                TriangularGrid::new(Point::new(17.25, -12.5).unwrap(), 2.5, rotation).unwrap();
            for node in [
                GridPoint { q: -7, r: -3 },
                GridPoint { q: -2, r: 5 },
                GridPoint { q: 4, r: -8 },
                GridPoint { q: 0, r: 0 },
            ] {
                let position = grid.position(node).unwrap();
                assert_eq!(grid.nearest(position), Ok(node));
                for (dx, dy) in [(0.2, 0.3), (-0.3, 0.2), (-0.2, -0.3), (0.3, -0.2)] {
                    let nearby = Point::new(position.x() + dx, position.y() + dy).unwrap();
                    assert_eq!(grid.nearest(nearby), Ok(node));
                }
            }
        }
    }

    #[test]
    fn nearest_matches_exhaustive_euclidean_search_across_negative_and_positive_cells() {
        let grid = TriangularGrid::new(Point::new(0.0, 0.0).unwrap(), 1.0, 0.0).unwrap();
        let nodes: Vec<_> = (-8..=8)
            .flat_map(|q| (-8..=8).map(move |r| GridPoint { q, r }))
            .map(|node| (node, grid.position(node).unwrap()))
            .collect();
        for x in -32..=32 {
            for y in -32..=32 {
                let source = Point::new(f64::from(x) / 8.0, f64::from(y) / 8.0).unwrap();
                let expected = nodes
                    .iter()
                    .map(|&(node, position)| {
                        let dx = source.x() - position.x();
                        let dy = source.y() - position.y();
                        (node, dx * dx + dy * dy)
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
                    .unwrap()
                    .0;
                assert_eq!(grid.nearest(source), Ok(expected), "source: {source:?}");
            }
        }
    }

    #[test]
    fn snapping_checks_the_true_nearest_node_before_integer_range_conversion() {
        let grid = TriangularGrid::new(Point::new(0.0, 0.0).unwrap(), 1.0, 0.0).unwrap();
        for node in [
            GridPoint {
                q: i32::MIN,
                r: i32::MIN,
            },
            GridPoint {
                q: i32::MAX,
                r: i32::MAX,
            },
            GridPoint {
                q: i32::MIN,
                r: i32::MAX,
            },
            GridPoint {
                q: i32::MAX,
                r: i32::MIN,
            },
        ] {
            assert_eq!(grid.nearest(grid.position(node).unwrap()), Ok(node));
        }
        for (boundary, direction) in [(i32::MIN, -1.0), (i32::MAX, 1.0)] {
            assert_eq!(
                grid.nearest(Point::new(f64::from(boundary) + direction * 0.25, 0.0).unwrap()),
                Ok(GridPoint { q: boundary, r: 0 })
            );
            assert_eq!(
                grid.nearest(Point::new(f64::from(boundary) + direction * 0.75, 0.0).unwrap()),
                Err(GeometryError::CoordinateOverflow)
            );
        }
        assert_eq!(
            grid.nearest(Point::new(f64::MAX, 0.0).unwrap()),
            Err(GeometryError::CoordinateOverflow)
        );
    }

    #[test]
    fn snapping_rejects_nonfinite_inverse_transforms() {
        let tiny =
            TriangularGrid::new(Point::new(0.0, 0.0).unwrap(), f64::MIN_POSITIVE, 0.0).unwrap();
        assert_eq!(
            tiny.nearest(Point::new(f64::MAX, 0.0).unwrap()),
            Err(GeometryError::NonFinitePoint)
        );
        let distant = TriangularGrid::new(Point::new(f64::MAX, 0.0).unwrap(), 1.0, 0.0).unwrap();
        assert_eq!(
            distant.nearest(Point::new(-f64::MAX, 0.0).unwrap()),
            Err(GeometryError::NonFinitePoint)
        );
    }

    #[test]
    fn rejects_any_neighbor_coordinate_overflow() {
        for point in [
            GridPoint { q: i32::MIN, r: 0 },
            GridPoint { q: i32::MAX, r: 0 },
            GridPoint { q: 0, r: i32::MIN },
            GridPoint { q: 0, r: i32::MAX },
        ] {
            assert_eq!(point.neighbors(), Err(GeometryError::CoordinateOverflow));
        }
    }

    #[test]
    fn maps_extreme_integer_coordinates_without_integer_arithmetic_overflow() {
        let grid = TriangularGrid::new(Point::new(0.0, 0.0).unwrap(), 1.0, 0.0).unwrap();

        for point in [
            GridPoint {
                q: i32::MAX,
                r: i32::MAX,
            },
            GridPoint {
                q: i32::MIN,
                r: i32::MIN,
            },
            GridPoint {
                q: i32::MAX,
                r: i32::MIN,
            },
        ] {
            let position = grid.position(point).unwrap();

            assert_eq!(position.x(), f64::from(point.q) + f64::from(point.r) / 2.0);
            assert!(position.y().is_finite());
        }
    }

    #[test]
    fn rejects_floating_point_overflow() {
        let origin = Point::new(0.0, 0.0).unwrap();
        let huge_spacing = TriangularGrid::new(origin, f64::MAX, 0.0).unwrap();
        let huge_origin =
            TriangularGrid::new(Point::new(f64::MAX, 0.0).unwrap(), f64::MAX, 0.0).unwrap();

        assert_eq!(
            huge_spacing.position(GridPoint { q: 2, r: 0 }),
            Err(GeometryError::NonFinitePoint)
        );
        assert_eq!(
            huge_origin.position(GridPoint { q: 1, r: 0 }),
            Err(GeometryError::NonFinitePoint)
        );
    }
}
