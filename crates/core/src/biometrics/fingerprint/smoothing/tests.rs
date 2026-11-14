use super::constraints::forbidden_intersection;
use super::*;
use crate::biometrics::fingerprint::{NodeKind, RidgeCycle, RidgeEdge, RidgeNode};

fn all_support() -> RidgeMap {
    RidgeMap::new(160, 160, vec![true; 160 * 160], vec![true; 160 * 160]).unwrap()
}

fn graph(points: Vec<Point>) -> RidgeGraph {
    RidgeGraph {
        nodes: vec![
            RidgeNode {
                position: points[0],
                kind: NodeKind::Ending,
                touches_unobserved: false,
            },
            RidgeNode {
                position: *points.last().unwrap(),
                kind: NodeKind::Ending,
                touches_unobserved: false,
            },
        ],
        edges: vec![RidgeEdge {
            start: 0,
            end: 1,
            points,
        }],
        cycles: vec![],
    }
}

fn roughness(points: &[Point]) -> f64 {
    points
        .windows(3)
        .map(|p| {
            let dx = p[2].x() - 2.0 * p[1].x() + p[0].x();
            let dy = p[2].y() - 2.0 * p[1].y() + p[0].y();
            dx * dx + dy * dy
        })
        .sum()
}

#[test]
fn straight_path_and_open_endpoints_are_exactly_preserved() {
    let original = graph((10..=110).map(|x| point(x as f64, 40.0)).collect());
    let output = smooth(&original, &all_support(), 12.0).unwrap();
    assert_eq!(output.graph, original);
    assert!(!output.edge_adjustments[0].smoothing_applied);
    assert_eq!(output.edge_adjustments[0].retained_reason, None);
}

#[test]
fn removes_pixel_zigzags_without_moving_endpoints_or_graph_nodes() {
    let original = graph(
        (10..=110)
            .map(|x| point(x as f64, 40.0 + f64::from(x % 2)))
            .collect(),
    );
    let output = smooth(&original, &all_support(), 12.0).unwrap();
    let before = &original.edges[0].points;
    let after = &output.graph.edges[0].points;
    assert_eq!(before.len(), after.len());
    assert_eq!(before.first(), after.first());
    assert_eq!(before.last(), after.last());
    assert_eq!(output.graph.nodes, original.nodes);
    assert!(roughness(after) < roughness(before) / 50.0);
    assert!(output.edge_adjustments[0].smoothing_applied);
    assert!(
        (max_movement(before, after) - output.edge_adjustments[0].max_displacement_px).abs()
            < 1e-10
    );
    assert!(output.edge_adjustments[0].max_displacement_px <= 3.0);
}

#[test]
fn periodic_smoothing_preserves_winding_and_does_not_collapse_small_holes() {
    let points = (0..80)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / 80.0;
            let radius = 3.0 + if i % 2 == 0 { 0.2 } else { -0.2 };
            point(60.0 + radius * angle.cos(), 60.0 + radius * angle.sin())
        })
        .collect::<Vec<_>>();
    let original = RidgeGraph {
        nodes: vec![],
        edges: vec![],
        cycles: vec![RidgeCycle {
            points: points.clone(),
        }],
    };
    let output = smooth(&original, &all_support(), 16.0).unwrap();
    let smoothed = &output.graph.cycles[0].points;
    assert_eq!(smoothed.len(), points.len());
    assert!(signed_area(smoothed) >= signed_area(&points) / 2.0);
    assert!(output.cycle_adjustments[0].smoothing_applied);
    assert!(output.cycle_adjustments[0].applied_fraction < 1.0);
    assert!(roughness(smoothed) < roughness(&points));
}

#[test]
fn closed_edge_keeps_its_exact_junction_port() {
    let mut points = (0..80)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / 80.0;
            let radius = 10.0 + f64::from(i % 2) / 2.0;
            point(60.0 + radius * angle.cos(), 60.0 + radius * angle.sin())
        })
        .collect::<Vec<_>>();
    points.push(points[0]);
    let mut original = graph(points);
    original.edges[0].end = 0;
    original.nodes.truncate(1);
    original.nodes[0].kind = NodeKind::Junction;
    let output = smooth(&original, &all_support(), 8.0).unwrap();
    assert_eq!(
        output.graph.edges[0].points.first(),
        original.edges[0].points.first()
    );
    assert_eq!(
        output.graph.edges[0].points.first(),
        output.graph.edges[0].points.last()
    );
    assert_eq!(output.graph.nodes, original.nodes);
    assert!(output.edge_adjustments[0].smoothing_applied);
}

#[test]
fn self_edge_with_distinct_ports_remains_open() {
    let mut original = graph(vec![
        point(20.0, 20.0),
        point(30.0, 30.0),
        point(40.0, 20.0),
    ]);
    original.edges[0].end = 0;
    original.nodes.truncate(1);
    original.nodes[0].kind = NodeKind::Junction;
    let output = smooth(&original, &all_support(), 10.0).unwrap();
    assert_eq!(
        output.graph.edges[0].points.first(),
        original.edges[0].points.first()
    );
    assert_eq!(
        output.graph.edges[0].points.last(),
        original.edges[0].points.last()
    );
    assert_ne!(
        output.graph.edges[0].points.first(),
        output.graph.edges[0].points.last()
    );
}

#[test]
fn swept_corridor_cannot_jump_across_a_missing_source_pixel() {
    let mut support = all_support();
    support.usable[40 * 160 + 40] = false;
    support.ridges[40 * 160 + 40] = false;
    let raw = [point(30.0, 39.0), point(50.0, 39.0)];
    let target = [point(30.0, 41.0), point(50.0, 41.0)];
    assert!(!preserves_support(&raw, &target, false, &support, &mut Work::default()).unwrap());
    assert!(preserves_support(&target, &target, false, &support, &mut Work::default()).unwrap());
}

#[test]
fn accepted_paths_do_not_cross_other_paths_or_themselves() {
    let mut index = SegmentIndex::default();
    index
        .add_path(
            0,
            &[point(10.0, 20.0), point(40.0, 20.0)],
            false,
            &mut Work::default(),
        )
        .unwrap();
    assert!(
        !preserves_intersections(
            &[point(25.0, 10.0), point(25.0, 30.0)],
            false,
            1,
            &index,
            &mut Work::default()
        )
        .unwrap()
    );
    assert!(
        !preserves_intersections(
            &[
                point(10.0, 10.0),
                point(30.0, 30.0),
                point(10.0, 30.0),
                point(30.0, 10.0)
            ],
            false,
            0,
            &SegmentIndex::default(),
            &mut Work::default()
        )
        .unwrap()
    );
    index
        .ports
        .insert(1, [point(10.0, 20.0), point(10.0, 30.0)]);
    assert!(
        preserves_intersections(
            &[point(10.0, 20.0), point(10.0, 30.0)],
            false,
            1,
            &index,
            &mut Work::default()
        )
        .unwrap()
    );
}

#[test]
fn shared_endpoints_are_allowed_but_overlapping_segments_are_not() {
    let a = point(10.0, 10.0);
    let b = point(20.0, 10.0);
    assert!(!forbidden_intersection(a, b, b, point(30.0, 10.0), true));
    assert!(!forbidden_intersection(a, b, b, point(20.0, 20.0), true));
    assert!(forbidden_intersection(a, b, a, point(15.0, 10.0), false));
    assert!(forbidden_intersection(
        a,
        b,
        point(15.0, 10.0),
        point(25.0, 10.0),
        false
    ));
}

#[test]
fn sampling_density_does_not_set_gaussian_smoothing_weight() {
    let sparse = ObservedPath::new(
        &[point(10.0, 10.0), point(30.0, 10.0), point(30.0, 40.0)],
        false,
        &all_support(),
    )
    .unwrap();
    let dense = ObservedPath::new(
        &[
            point(10.0, 10.0),
            point(10.1, 10.0),
            point(10.2, 10.0),
            point(30.0, 10.0),
            point(30.0, 39.0),
            point(30.0, 40.0),
        ],
        false,
        &all_support(),
    )
    .unwrap();
    let controls = SmoothingControls::for_unit(12.0).unwrap();
    let a = gaussian_target(&sparse, controls, &mut Work::default()).unwrap();
    let b = gaussian_target(&dense, controls, &mut Work::default()).unwrap();
    assert!(distance(a[1], b[3]) < 1e-10);
}

#[test]
fn reversing_an_open_path_reverses_its_smoothed_geometry() {
    let original = graph(
        (10..=110)
            .map(|x| point(x as f64, 40.0 + f64::from(x % 2)))
            .collect(),
    );
    let forward = smooth(&original, &all_support(), 12.0).unwrap();
    let reversed = graph(original.edges[0].points.iter().rev().copied().collect());
    let backward = smooth(&reversed, &all_support(), 12.0).unwrap();
    for (&a, &b) in forward.graph.edges[0]
        .points
        .iter()
        .zip(backward.graph.edges[0].points.iter().rev())
    {
        assert!(distance(a, b) < 1e-10);
    }
}

#[test]
fn invalid_controls_and_resource_exhaustion_are_explicit() {
    for unit in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(SmoothingControls::for_unit(unit).is_err());
    }
    let original = graph(vec![point(1.0, 1.0), point(100.0, 1.0)]);
    assert_eq!(
        smooth(&original, &all_support(), 1e-10),
        Err(FingerprintError::WorkLimit)
    );
    assert_eq!(
        smooth(&original, &all_support(), 1e100),
        Err(FingerprintError::WorkLimit)
    );
}

#[test]
fn adjacent_backtracking_and_nonadjacent_self_contacts_are_rejected() {
    assert!(forbidden_intersection(
        point(10.0, 10.0),
        point(20.0, 10.0),
        point(20.0, 10.0),
        point(15.0, 10.0),
        true,
    ));
    assert!(
        !preserves_intersections(
            &[
                point(10.0, 10.0),
                point(20.0, 10.0),
                point(20.0, 20.0),
                point(10.0, 10.0),
                point(10.0, 20.0),
            ],
            false,
            0,
            &SegmentIndex::default(),
            &mut Work::default()
        )
        .unwrap()
    );
}

#[test]
fn swept_curve_cannot_cross_another_line_then_finish_on_its_far_side() {
    let mut obstacles = SegmentIndex::default();
    obstacles
        .add_path(
            1,
            &[point(10.0, 20.0), point(50.0, 20.0)],
            false,
            &mut Work::default(),
        )
        .unwrap();
    let original = [point(20.0, 19.0), point(40.0, 19.0)];
    let candidate = [point(20.0, 21.0), point(40.0, 21.0)];
    assert!(
        preserves_intersections(&candidate, false, 0, &obstacles, &mut Work::default()).unwrap()
    );
    assert!(
        !preserves_swept_separation(
            &original,
            &candidate,
            false,
            0,
            &obstacles,
            &mut Work::default()
        )
        .unwrap()
    );
}

#[test]
fn an_empty_graph_has_an_empty_smoothing_result() {
    let original = RidgeGraph {
        nodes: vec![],
        edges: vec![],
        cycles: vec![],
    };
    let result = smooth(&original, &all_support(), 12.0).unwrap();
    assert_eq!(result.graph, original);
    assert!(result.edge_adjustments.is_empty());
    assert!(result.cycle_adjustments.is_empty());
}

#[test]
fn an_isolated_observed_point_is_not_swallowed_by_a_moving_curve() {
    let mut index = SegmentIndex::default();
    index
        .add_path(1, &[point(30.0, 20.0)], true, &mut Work::default())
        .unwrap();
    let original = [point(20.0, 19.0), point(40.0, 19.0)];
    let candidate = [point(20.0, 21.0), point(40.0, 21.0)];
    assert!(
        !preserves_swept_separation(
            &original,
            &candidate,
            false,
            0,
            &index,
            &mut Work::default()
        )
        .unwrap()
    );
    assert!(
        !preserves_intersections(
            &[point(20.0, 20.0), point(40.0, 20.0)],
            false,
            0,
            &index,
            &mut Work::default()
        )
        .unwrap()
    );
}

#[test]
fn proper_subpixel_crossings_are_detected_after_rotation_and_translation() {
    for scale in [0.001, 1.0, 1000.0] {
        for origin in [0.0, 1000.0, 10_000_000.0] {
            for angle in [0.0_f64, 0.37, 1.5] {
                let (sin, cos) = angle.sin_cos();
                let transformed = |x, y| {
                    point(
                        origin + scale * (cos * x - sin * y),
                        origin + scale * (sin * x + cos * y),
                    )
                };
                let (a, b) = (transformed(-1.0, 0.0), transformed(1.0, 0.0));
                assert!(
                    forbidden_intersection(
                        a,
                        b,
                        transformed(0.0, -1.0),
                        transformed(0.0, 1.0),
                        false
                    ),
                    "missed crossing: scale={scale}, origin={origin}, angle={angle}"
                );
                assert!(
                    !forbidden_intersection(
                        a,
                        b,
                        transformed(-1.0, 1.0),
                        transformed(1.0, 1.0),
                        false
                    ),
                    "invented crossing: scale={scale}, origin={origin}, angle={angle}"
                );
            }
        }
    }
}
