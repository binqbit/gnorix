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
                formatter.write_str("lattice neighbor exceeds the integer coordinate range")
            }
        }
    }
}

impl Error for GeometryError {}

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
