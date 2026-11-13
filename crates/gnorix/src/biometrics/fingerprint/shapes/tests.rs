use super::*;
use crate::biometrics::fingerprint::FeatureKind;

fn points(positions: &[(f64, f64)]) -> Vec<CombinationPoint> {
    positions
        .iter()
        .enumerate()
        .map(|(id, &(x, y))| CombinationPoint {
            id,
            kind: FeatureKind::Ending,
            position: Point::new(x, y).unwrap(),
            priority: 1.0 - id as f64 / positions.len() as f64,
        })
        .collect()
}

fn combination(ids: &[usize]) -> FeatureCombination {
    FeatureCombination {
        feature_ids: ids.to_vec(),
        score: 1.0,
        regions: vec![0],
    }
}

fn config(level: u32) -> GridShapeConfig {
    GridShapeConfig {
        uncertainty_steps: level,
        max_variants: 1_000_000,
        max_work: 1_000_000,
    }
}

#[test]
fn keeps_every_point_and_enumerates_all_assignments_in_one_padded_frame() {
    let points = points(&[(0.0, 0.0), (12.0, 0.0), (4.0, 5.0), (5.0, 1.0)]);
    let result =
        prepare_grid_shapes(&points, &[combination(&[0, 1, 2, 3])], 1.0, &config(1)).unwrap();
    let shape = &result.shapes()[0];
    assert_eq!(shape.frame.boundary.len(), 3);
    assert_eq!(shape.feature_ids, [0, 1, 2, 3]);
    assert_eq!(shape.baseline.len(), 4);
    assert_eq!(result.variant_count(), 2401);
    let variants = result.variants().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(variants.len(), 2401);
    assert_eq!(variants[0].positions, shape.baseline);
    assert!(variants.iter().all(|v| v.positions.len() == 4));
    // All common translations survive as distinct placements in the fixed frame.
    let translated: Vec<_> = shape
        .baseline
        .iter()
        .map(|p| GridCell {
            column: p.column + 1,
            row: p.row,
        })
        .collect();
    assert!(variants.iter().any(|v| v.positions == translated));
}

#[test]
fn rigid_motion_and_source_order_do_not_change_grid_geometry_away_from_boundaries() {
    let original = points(&[(2.13, 1.74), (29.7, 1.74), (10.1, 13.2), (12.2, 7.1)]);
    let base =
        prepare_grid_shapes(&original, &[combination(&[0, 1, 2, 3])], 2.3, &config(1)).unwrap();
    for angle in [0.0_f64, 0.3, 1.7, 3.4, 5.1] {
        let (sin, cos) = angle.sin_cos();
        let mut moved = original.clone();
        for point in &mut moved {
            let p = point.position;
            point.position = Point::new(
                73.0 + cos * p.x() - sin * p.y(),
                -31.0 + sin * p.x() + cos * p.y(),
            )
            .unwrap();
            point.priority = 0.0;
        }
        moved.reverse();
        let result =
            prepare_grid_shapes(&moved, &[combination(&[3, 1, 0, 2])], 2.3, &config(1)).unwrap();
        assert_eq!(result.shapes()[0].phase, base.shapes()[0].phase);
        assert_eq!(result.shapes()[0].baseline, base.shapes()[0].baseline);
        assert_eq!(result.variant_count(), base.variant_count());
    }
}

#[test]
fn scaling_points_and_measured_cursor_together_preserves_the_figure() {
    let original = points(&[(0.0, 0.0), (22.0, 0.0), (8.0, 5.0)]);
    let combos = [combination(&[0, 1, 2])];
    let base = prepare_grid_shapes(&original, &combos, 1.2, &config(0)).unwrap();
    let mut scaled = original.clone();
    for point in &mut scaled {
        point.position = Point::new(point.position.x() * 3.7, point.position.y() * 3.7).unwrap();
    }
    let result = prepare_grid_shapes(&scaled, &combos, 1.2 * 3.7, &config(0)).unwrap();
    assert_eq!(base.shapes()[0].baseline, result.shapes()[0].baseline);
    assert_eq!(base.shapes()[0].phase, result.shapes()[0].phase);
}

#[test]
fn coincident_features_are_preserved_with_their_ids_even_at_level_zero() {
    let points = points(&[(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]);
    let result = prepare_grid_shapes(&points, &[combination(&[0, 1, 2])], 1.0, &config(0)).unwrap();
    let variant = result.variants().next().unwrap().unwrap();
    assert_eq!(variant.positions, vec![GridCell { column: 0, row: 0 }; 3]);
    assert_eq!(variant.collisions.len(), 1);
    assert_eq!(variant.collisions[0].feature_ids, [0, 1, 2]);
    assert_eq!(result.variant_count(), 1);
}

#[test]
fn rejects_complete_count_excess_instead_of_returning_a_truncated_set() {
    let points = points(&[(0.0, 0.0), (12.0, 0.0), (4.0, 5.0)]);
    let combos = [combination(&[0, 1, 2]), combination(&[0, 1])];
    let too_small = GridShapeConfig {
        max_variants: 391,
        ..config(1)
    };
    assert!(matches!(
        prepare_grid_shapes(&points, &combos, 1.0, &too_small),
        Err(GridShapeError::VariantLimit {
            required: 392,
            limit: 391
        })
    ));
    let exact = GridShapeConfig {
        max_variants: 392,
        ..config(1)
    };
    let result = prepare_grid_shapes(&points, &combos, 1.0, &exact).unwrap();
    assert_eq!(
        result
            .variants()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len(),
        392
    );
}

#[test]
fn malformed_inputs_and_resource_exhaustion_are_explicit() {
    let points = points(&[(0.0, 0.0), (12.0, 0.0), (4.0, 5.0)]);
    for ids in [vec![], vec![0, 0], vec![0, 99]] {
        assert!(matches!(
            prepare_grid_shapes(&points, &[combination(&ids)], 1.0, &config(0)),
            Err(GridShapeError::InvalidInput(_))
        ));
    }
    for unit in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            prepare_grid_shapes(&points, &[combination(&[0, 1, 2])], unit, &config(0)),
            Err(GridShapeError::Geometry(GeometryError::InvalidSpacing))
        ));
    }
    assert!(matches!(
        prepare_grid_shapes(
            &points,
            &[combination(&[0, 1, 2])],
            1.0,
            &GridShapeConfig {
                max_work: 1,
                ..config(1)
            }
        ),
        Err(GridShapeError::WorkLimit)
    ));
    let repeated = vec![points[0].clone(), points[0].clone()];
    assert!(matches!(
        prepare_grid_shapes(&repeated, &[combination(&[0])], 1.0, &config(0)),
        Err(GridShapeError::InvalidInput(_))
    ));
}
