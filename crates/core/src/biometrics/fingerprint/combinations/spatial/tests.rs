use super::*;

fn points(values: &[(f64, f64)]) -> Vec<Point> {
    values
        .iter()
        .map(|&(x, y)| Point::new(x, y).unwrap())
        .collect()
}
fn policy(anchors: usize, separation: f64) -> SpatialPolicy {
    SpatialPolicy {
        min_anchor_points: anchors,
        min_anchor_distance_units: separation,
    }
}
fn admitted(positions: &[Point], requirements: SpatialPolicy) -> bool {
    satisfies_spatial_requirements(positions, 1.0, &requirements, 100_000)
        .unwrap()
        .0
}

fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b.x() - a.x()) * (c.y() - a.y()) - (b.y() - a.y()) * (c.x() - a.x())
}

/// Independent small-coordinate oracle using segments and triangles, rather
/// than the production hull construction or mandatory-vertex reduction.
fn in_envelope(point: Point, anchors: &[Point]) -> bool {
    if anchors.contains(&point) {
        return true;
    }
    for first in 0..anchors.len() {
        for second in first + 1..anchors.len() {
            let (a, b) = (anchors[first], anchors[second]);
            if cross(a, b, point) == 0.0
                && point.x() >= a.x().min(b.x())
                && point.x() <= a.x().max(b.x())
                && point.y() >= a.y().min(b.y())
                && point.y() <= a.y().max(b.y())
            {
                return true;
            }
            for &c in anchors.iter().skip(second + 1) {
                if cross(a, b, c) == 0.0 {
                    continue;
                }
                let turns = [cross(a, b, point), cross(b, c, point), cross(c, a, point)];
                if turns.iter().all(|&v| v >= 0.0) || turns.iter().all(|&v| v <= 0.0) {
                    return true;
                }
            }
        }
    }
    false
}

fn oracle(positions: &[Point], requirements: SpatialPolicy) -> bool {
    let square = |a: Point, b: Point| (a.x() - b.x()).powi(2) + (a.y() - b.y()).powi(2);
    for mask in 1_usize..1 << positions.len() {
        if (mask.count_ones() as usize) < requirements.min_anchor_points {
            continue;
        }
        let anchors: Vec<_> = positions
            .iter()
            .enumerate()
            .filter_map(|(i, &point)| (mask & (1 << i) != 0).then_some(point))
            .collect();
        if (0..anchors.len()).any(|i| {
            (i + 1..anchors.len()).any(|j| {
                square(anchors[i], anchors[j]) < requirements.min_anchor_distance_units.powi(2)
            })
        }) {
            continue;
        }
        if positions.iter().all(|&point| in_envelope(point, &anchors)) {
            return true;
        }
    }
    false
}

#[test]
fn predicate_matches_an_independent_exhaustive_anchor_oracle() {
    let locations = points(&[(0., 0.), (4., 0.), (0., 4.), (1., 1.), (2., 0.), (4., 4.)]);
    for mask in 1_usize..1 << locations.len() {
        let selected: Vec<_> = locations
            .iter()
            .enumerate()
            .filter_map(|(i, &point)| (mask & (1 << i) != 0).then_some(point))
            .collect();
        for count in 1..=selected.len().min(4) + 1 {
            for separation in [0., 1., 3., 4.] {
                let requirements = policy(count, separation);
                assert_eq!(
                    admitted(&selected, requirements),
                    oracle(&selected, requirements),
                    "mask={mask}, k={count}, d={separation}"
                );
            }
        }
    }
}

#[test]
fn hull_boundaries_collinearity_duplicate_coordinates_and_minimum_anchor_counts() {
    let line = points(&[(0., 0.), (2., 0.), (4., 0.)]);
    assert!(admitted(&line, policy(3, 2.)));
    assert!(!admitted(&line, policy(3, 2.000_001)));
    let duplicate = points(&[(0., 0.), (2., 0.), (2., 0.), (4., 0.)]);
    assert!(!admitted(&duplicate, policy(4, 2.)));
    assert!(admitted(&duplicate, policy(4, 0.)));
    let square = points(&[(0., 0.), (4., 0.), (4., 4.), (0., 4.), (2., 0.), (2., 2.)]);
    assert!(admitted(&square, policy(2, 4.))); // four mandatory anchors allowed for k=2
    assert!(admitted(&square, policy(5, 2.))); // an interior anchor supplements the hull
    let outside = points(&[(0., 0.), (10., 0.), (0., 10.), (-1., 1.)]);
    assert!(admitted(&outside[..3], policy(3, 10.)));
    assert!(!admitted(&outside, policy(3, 10.)));
}

#[test]
fn work_exhaustion_is_an_error_instead_of_false_and_requirements_are_validated() {
    let positions = points(&[(0., 0.), (2., 0.), (4., 0.)]);
    let requirements = policy(3, 2.);
    let (valid, used) =
        satisfies_spatial_requirements(&positions, 1.0, &requirements, 1000).unwrap();
    assert!(valid && used > 0);
    assert!(matches!(
        satisfies_spatial_requirements(&positions, 1.0, &requirements, used - 1),
        Err(CombinationError::WorkLimit)
    ));
    for invalid in [
        policy(3, -1.),
        policy(0, 1.),
        policy(3, f64::NAN),
        policy(3, f64::INFINITY),
    ] {
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn positive_spatial_thresholds_cannot_underflow_into_disabled_constraints() {
    let singleton = points(&[(0., 0.)]);
    let coincident = points(&[(0., 0.), (0., 0.)]);
    assert!(matches!(
        satisfies_spatial_requirements(&coincident, 1e-308, &policy(2, 1e-308), 100),
        Err(CombinationError::InvalidConfig(_))
    ));
    // Explicit zeros retain their documented permissive meaning.
    assert!(
        satisfies_spatial_requirements(&singleton, 1e-308, &policy(1, 0.), 100)
            .unwrap()
            .0
    );
}

#[test]
fn direct_ridge_units_preserve_admission_across_rigid_motion_and_resizing() {
    let requirements = policy(3, 2.5);
    let original = points(&[(0., 0.), (12., 0.), (0., 12.), (3., 3.)]);
    assert!(
        satisfies_spatial_requirements(&original, 2., &requirements, 1000)
            .unwrap()
            .0
    );
    let (sin, cos) = 0.7_f64.sin_cos();
    let moved: Vec<_> = original
        .iter()
        .map(|p| {
            Point::new(
                100. + 7. * (cos * p.x() - sin * p.y()),
                200. + 7. * (sin * p.x() + cos * p.y()),
            )
            .unwrap()
        })
        .collect();
    assert!(
        satisfies_spatial_requirements(&moved, 14., &requirements, 1000)
            .unwrap()
            .0
    );
    assert!(
        !satisfies_spatial_requirements(&original, 20., &requirements, 1000)
            .unwrap()
            .0
    );
}
