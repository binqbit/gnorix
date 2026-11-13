use super::super::FeatureKind;
use super::*;

fn point(id: usize, x: f64, y: f64, priority: f64) -> CombinationPoint {
    CombinationPoint {
        id,
        kind: FeatureKind::Ending,
        position: Point::new(x, y).unwrap(),
        priority,
    }
}

fn bounds(width: f64, height: f64) -> RegionBounds {
    RegionBounds {
        min: Point::new(0.0, 0.0).unwrap(),
        max: Point::new(width, height).unwrap(),
    }
}

#[test]
fn grows_sparse_circle_and_retains_all_inclusive_boundary_points() {
    let points = vec![
        point(5, -3.0, 0.0, 0.9),
        point(3, 1.0, 0.0, 0.0),
        point(2, 3.0, 0.0, 0.8),
    ];
    let circle = circle_candidates(
        &points,
        0,
        Point::new(0.0, 0.0).unwrap(),
        1.0,
        2,
        &mut WorkBudget::new(100),
    )
    .unwrap();
    assert_eq!(circle.radius, 3.0);
    assert_eq!(circle.indices, [0, 2, 1]);
}

#[test]
fn enrollment_caps_pools_while_shared_circles_retain_every_point() {
    let points = vec![
        point(8, 1.0, 1.0, 0.2),
        point(2, 2.0, 1.0, 0.2),
        point(3, 4.0, 1.0, 1.0),
    ];
    let config = CombinationConfig {
        min_points: 1,
        min_priority: 0.0,
        min_span_fraction: 0.0,
        pool_multiplier: 2,
        radius_units: 5.0,
        combinations_per_region: 2,
        max_work: 1000,
    };
    let regions = select(
        &points,
        bounds(6.0, 2.0),
        1.0,
        &config,
        0.0,
        &mut WorkBudget::new(1000),
    )
    .unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].pool, [3, 2]);
    assert_eq!(regions[0].available_points, 3);
    let all = circular_regions(
        &points,
        bounds(6.0, 2.0),
        1.0,
        5.0,
        2,
        &mut WorkBudget::new(1000),
    )
    .unwrap();
    assert_eq!(all[0].indices, [2, 1, 0]);
}

#[test]
fn preserves_confirmed_pixel_edges_and_extends_only_to_uncovered_feature_centers() {
    let mut mask = vec![false; 100];
    mask[22] = true;
    mask[77] = true;
    let measured = support_bounds(&mask, 10, &mut WorkBudget::new(100)).unwrap();
    assert_eq!(measured.min, Point::new(1.5, 1.5).unwrap());
    assert_eq!(measured.max, Point::new(7.5, 7.5).unwrap());
    let extended = cover_points(
        measured,
        &[point(1, 0.0, 4.0, 1.0), point(2, 3.0, 9.0, 1.0)],
        &mut WorkBudget::new(10),
    )
    .unwrap();
    assert_eq!(extended.min, Point::new(0.0, 1.5).unwrap());
    assert_eq!(extended.max, Point::new(7.5, 9.0).unwrap());
    assert_eq!(
        cover_points(
            measured,
            &[point(1, 3.0, 4.0, 1.0)],
            &mut WorkBudget::new(10)
        )
        .unwrap(),
        measured
    );
}

#[test]
fn all_edges_corners_and_interiors_are_covered_without_a_nine_region_cap() {
    let points = (0..=14)
        .flat_map(|y| (0..=10).map(move |x| point(y * 11 + x, x as f64, y as f64, 0.5)))
        .collect::<Vec<_>>();
    let circles = circular_regions(
        &points,
        bounds(10.0, 14.0),
        1.0,
        2.0,
        1,
        &mut WorkBudget::new(100_000),
    )
    .unwrap();
    assert!(circles.len() > 9);
    for (index, point) in points.iter().enumerate() {
        assert!(
            circles.iter().any(|circle| circle.indices.contains(&index)),
            "uncovered {:?}",
            point.position
        );
    }
    for (id, circle) in circles.iter().enumerate() {
        assert_eq!(circle.id, id);
        assert_eq!(circle.initial_radius, 2.0);
        assert_eq!(circle.radius, 2.0);
    }
}

#[test]
fn exact_circle_corners_are_inclusive_and_region_ids_can_exceed_a_byte() {
    let corners = [(0.0, 0.0), (6.0, 0.0), (0.0, 8.0), (6.0, 8.0)]
        .into_iter()
        .enumerate()
        .map(|(id, (x, y))| point(id, x, y, 1.0))
        .collect::<Vec<_>>();
    let circle = circular_regions(
        &corners,
        bounds(6.0, 8.0),
        1.0,
        5.0,
        1,
        &mut WorkBudget::new(100),
    )
    .unwrap();
    assert_eq!(circle.len(), 1);
    assert_eq!(circle[0].radius, 5.0);
    assert_eq!(circle[0].indices, [0, 1, 2, 3]);

    let many = circular_regions(
        &[point(0, 0.0, 0.0, 1.0)],
        bounds(40.0, 40.0),
        1.0,
        1.0,
        1,
        &mut WorkBudget::new(10_000),
    )
    .unwrap();
    assert!(many.len() > 255);
    assert_eq!(many.last().unwrap().id, many.len() - 1);
}

#[test]
fn scaled_inputs_and_ridge_width_preserve_regions_and_membership() {
    let original = (0..12)
        .map(|i| {
            point(
                i,
                (i % 3) as f64 * 7.0,
                (i / 3) as f64 * 7.0,
                i as f64 / 12.0,
            )
        })
        .collect::<Vec<_>>();
    let base = circular_regions(
        &original,
        bounds(15.0, 25.0),
        1.7,
        5.0,
        4,
        &mut WorkBudget::new(10_000),
    )
    .unwrap();
    for scale in [0.1, 3.7, 1000.0] {
        let moved = original
            .iter()
            .map(|point| CombinationPoint {
                position: Point::new(
                    31.0 + scale * point.position.x(),
                    72.0 + scale * point.position.y(),
                )
                .unwrap(),
                ..point.clone()
            })
            .collect::<Vec<_>>();
        let extent = RegionBounds {
            min: Point::new(31.0, 72.0).unwrap(),
            max: Point::new(31.0 + 15.0 * scale, 72.0 + 25.0 * scale).unwrap(),
        };
        let result = circular_regions(
            &moved,
            extent,
            1.7 * scale,
            5.0,
            4,
            &mut WorkBudget::new(10_000),
        )
        .unwrap();
        assert_eq!(result.len(), base.len());
        for (actual, expected) in result.iter().zip(&base) {
            assert_eq!(actual.indices, expected.indices);
            assert!((actual.center.x() - (31.0 + scale * expected.center.x())).abs() < 1e-8);
            assert!((actual.center.y() - (72.0 + scale * expected.center.y())).abs() < 1e-8);
            assert!((actual.radius - scale * expected.radius).abs() < 1e-8);
        }
    }
}

#[test]
fn missing_pattern_invalid_units_and_resource_exhaustion_are_explicit() {
    assert_eq!(
        support_bounds(&[false; 4], 2, &mut WorkBudget::new(4)),
        Err(CombinationError::PatternUnavailable)
    );
    assert_eq!(
        support_bounds(&[true; 4], 2, &mut WorkBudget::new(3)),
        Err(CombinationError::WorkLimit)
    );
    let points = [point(1, 0.0, 0.0, 1.0)];
    for (unit, radius) in [
        (0.0, 1.0),
        (f64::NAN, 1.0),
        (1.0, 0.0),
        (1.0, f64::INFINITY),
        (f64::MAX, 2.0),
    ] {
        assert!(
            circular_regions(
                &points,
                bounds(1.0, 1.0),
                unit,
                radius,
                1,
                &mut WorkBudget::new(100)
            )
            .is_err()
        );
    }
    for limit in [0, 1, 5, 25] {
        assert!(matches!(
            circular_regions(
                &points,
                bounds(100.0, 100.0),
                1.0,
                1.0,
                1,
                &mut WorkBudget::new(limit)
            ),
            Err(CombinationError::WorkLimit)
        ));
    }
    assert!(matches!(
        circular_regions(
            &points,
            bounds(1.0, 1.0),
            1.0,
            f64::MIN_POSITIVE,
            1,
            &mut WorkBudget::new(100)
        ),
        Err(CombinationError::WorkLimit)
    ));
    // Region list itself fits, but retaining repeated memberships exhausts the same budget.
    let dense = (0..30)
        .map(|id| point(id, 0.0, 0.0, 1.0))
        .collect::<Vec<_>>();
    assert!(matches!(
        circular_regions(
            &dense,
            bounds(1.0, 1.0),
            1.0,
            2.0,
            1,
            &mut WorkBudget::new(40)
        ),
        Err(CombinationError::WorkLimit)
    ));
}
