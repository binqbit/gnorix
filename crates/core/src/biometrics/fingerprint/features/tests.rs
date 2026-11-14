use super::*;

#[test]
fn aggregation_requires_context_and_retains_unknown_evidence() {
    let stable = combine(Some(0.8), Some(0.6), Some(0.9), 1.0).unwrap();
    let unstable = combine(Some(0.8), Some(0.6), Some(0.9), 0.0).unwrap();
    assert!(stable > unstable);
    assert!(
        stable <= 0.9,
        "local evidence cannot exceed physical context"
    );
    assert_eq!(
        combine(Some(1.0), Some(1.0), Some(0.0), 1.0),
        Some(0.0),
        "perfect extraction stability cannot rescue absent physical support",
    );
    assert_eq!(combine(None, Some(1.0), Some(1.0), 1.0), None);
    assert_eq!(combine(Some(1.0), None, Some(1.0), 0.0), None);
    assert_eq!(combine(Some(1.0), Some(1.0), None, 1.0), None);
}

use super::super::{LocalPalette, RidgeCycle, RidgeEdge, RidgeNode};
use geometry::{ArmPath, EndingBackbone};
use sampling::{Sampler, Work};

fn point(x: f64, y: f64) -> Point {
    Point::new(x, y).unwrap()
}

fn policy(unit: f64) -> FeaturePolicy {
    let source = super::super::SourcePolicy::for_unit(unit);
    FeaturePolicy {
        reference_width_px: unit,
        branch_horizon_px: 2.0 * unit,
        tangent_support_px: source.tangent_radius_px,
        sample_spacing_px: source.sample_spacing_px,
        source_search_px: source.terminal_lookahead_px,
        continuation_search_px: 3.0 * unit,
        terminal_barrier_width_px: source.terminal_barrier_width_px,
        terminal_barrier_depth_px: source.terminal_barrier_depth_px,
        context_horizon_px: 4.0 * unit,
        neighborhood_radius_px: 3.0 * unit,
        contrast_reference: 0.01,
        stability: StabilityPolicy {
            max_position_shift_px: unit,
            arm_radius_px: unit,
            max_arm_angle_radians: std::f64::consts::PI / 6.0,
            max_arm_shape_error_px: unit / 4.0,
            max_isolated_shift_px: unit / 4.0,
            cycle_relative_tolerance: 0.2,
        },
        stress_cases: [StressCase {
            directional_margin: 0.0,
            min_coherence: 0.2,
            node_count: 0,
            cycle_count: 0,
        }; 8],
    }
}

fn graph(paths: Vec<Vec<Point>>) -> RidgeGraph {
    let mut graph = RidgeGraph {
        nodes: Vec::new(),
        edges: Vec::new(),
        cycles: Vec::new(),
    };
    for points in paths {
        let start = graph.nodes.len();
        for position in [points[0], *points.last().unwrap()] {
            graph.nodes.push(RidgeNode {
                position,
                kind: NodeKind::Ending,
                touches_unobserved: false,
            });
        }
        graph.edges.push(RidgeEdge {
            start,
            end: start + 1,
            points,
        });
    }
    graph
}

fn graph_paths(graph: &RidgeGraph) -> impl Iterator<Item = (&[Point], bool)> {
    graph
        .edges
        .iter()
        .map(|edge| (&edge.points[..], false))
        .chain(graph.cycles.iter().map(|cycle| (&cycle.points[..], true)))
}

#[test]
fn geometric_separation_grows_continuously_from_the_required_outer_window() {
    let policy = policy(8.0);
    for length in [8.0 - 1e-6, 8.0, 8.0 + 1e-6, 12.0, 16.0] {
        let arms: Vec<_> = [-1.0, 1.0]
            .into_iter()
            .enumerate()
            .map(|(edge, sign)| ArmPath {
                edge,
                points: vec![point(0.0, 0.0), point(sign * length, 0.0)],
                length,
                direction: Some([sign, 0.0]),
            })
            .collect();
        let value = geometry::separation(&arms, &policy, &mut Work::default())
            .unwrap()
            .unwrap();
        let expected = ((length - 8.0) / 8.0).clamp(0.0, 1.0);
        assert!(
            (value - expected).abs() < 1e-12,
            "length={length}, separation={value}"
        );
    }
}

#[test]
fn clipped_diagonal_arms_have_exact_requested_extent_and_start_at_observed_port() {
    let policy = policy(16.820789);
    let mut graph = graph(vec![
        (2..80).map(|i| point(f64::from(i), f64::from(i))).collect(),
    ]);
    // A representative cluster pixel is not a segment between that pixel and a port.
    graph.nodes[0].position = point(0.0, 0.0);
    let arms = geometry::arms(&graph, &policy, &mut Work::default()).unwrap();
    for paths in &arms {
        assert_eq!(paths[0].length, policy.branch_horizon_px);
        assert_eq!(geometry::extent(paths, policy.branch_horizon_px), 1.0);
    }
    assert_eq!(arms[0][0].points[0], point(2.0, 2.0));
}

#[test]
fn nearby_continuation_uses_a_path_tangent_instead_of_each_raster_step() {
    let policy = policy(8.0);
    let incoming = vec![point(50.0, 50.0), point(30.0, 30.0)];
    let diagonal = vec![point(55.0, 55.0), point(80.0, 80.0)];
    let mut staircase = vec![point(55.0, 55.0)];
    for i in 55..80 {
        staircase.push(point(f64::from(i + 1), f64::from(i)));
        staircase.push(point(f64::from(i + 1), f64::from(i + 1)));
    }
    let measure = |path: Vec<Point>| {
        let graph = graph(vec![incoming.clone(), path]);
        let mut work = Work::default();
        let arms = geometry::arms(&graph, &policy, &mut work).unwrap();
        geometry::continuation(0, &arms, graph_paths(&graph), &policy, &mut work)
            .unwrap()
            .unwrap()
    };
    let straight = measure(diagonal);
    assert!(straight > 0.7);
    assert!((measure(staircase.clone()) - straight).abs() < 1e-12);
    staircase.reverse();
    assert!((measure(staircase) - straight).abs() < 1e-12);
}

#[test]
fn closed_light_run_is_invariant_to_cycle_seam_and_reversal() {
    let side = 32;
    let capture = Capture::new(
        side,
        side,
        (0..side * side)
            .map(|i| if i % side <= 13 { 220 } else { 30 })
            .collect(),
        None,
        None,
    )
    .unwrap();
    let palette = LocalPalette {
        radius_px: 1,
        ridge_gray: vec![30.0; 1024],
        valley_gray: vec![220.0; 1024],
        usable: vec![true; 1024],
    };
    let mut points = vec![
        point(10.0, 10.0),
        point(20.0, 10.0),
        point(20.0, 20.0),
        point(10.0, 20.0),
    ];
    let measure = |points: &[Point]| {
        let mut work = Work::default();
        Sampler::new(&capture, &palette, &mut work)
            .path(points, true, 0.5)
            .unwrap()
    };
    let expected = measure(&points);
    assert!(expected.longest_light.unwrap() > 10.0);
    for _ in 0..points.len() {
        points.rotate_left(1);
        for reverse in [false, true] {
            let mut reordered = points.clone();
            if reverse {
                reordered.reverse();
            }
            let actual = measure(&reordered);
            assert!(
                (actual.longest_light.unwrap() - expected.longest_light.unwrap()).abs() < 1e-12
            );
            assert!(
                (actual.support(40.0).unwrap() - expected.support(40.0).unwrap()).abs() < 1e-12
            );
        }
    }
}

#[test]
fn source_contrast_zero_is_finite_even_with_zero_reference() {
    assert_eq!(sampling::contrast_factor(0.0, 0.0), 0.0);
    assert_eq!(sampling::contrast_factor(0.25, 0.0), 1.0);
    assert!(sampling::contrast_factor(0.01, 0.02) < sampling::contrast_factor(0.25, 0.02));
}

#[test]
fn unbounded_separation_probe_returns_work_limit_before_sampling() {
    let policy = policy(1e9);
    let arms: Vec<_> = [-1.0, 1.0]
        .into_iter()
        .enumerate()
        .map(|(edge, sign)| ArmPath {
            edge,
            points: vec![point(0.0, 0.0), point(sign * 2e9, 0.0)],
            length: 2e9,
            direction: Some([sign, 0.0]),
        })
        .collect();
    assert!(matches!(
        geometry::separation(&arms, &policy, &mut Work::default()),
        Err(FingerprintError::WorkLimit)
    ));
}

#[test]
fn nearby_facing_end_of_the_same_long_edge_counts_as_continuation() {
    let policy = policy(8.0);
    let graph = graph(vec![vec![
        point(50.0, 50.0),
        point(30.0, 50.0),
        point(30.0, 80.0),
        point(80.0, 80.0),
        point(80.0, 50.0),
        point(60.0, 50.0),
    ]]);
    let mut work = Work::default();
    let arms = geometry::arms(&graph, &policy, &mut work).unwrap();
    // Evidence starts after the first ending's inspected 2R. The other end is
    // geometrically nearby even though both ends belong to the same graph edge.
    let backbone = EndingBackbone::new(&graph, 0, 0, policy.branch_horizon_px);
    let value = geometry::continuation(0, &arms, backbone.paths(), &policy, &mut work)
        .unwrap()
        .unwrap();
    assert!((value - (1.0 - 10.0 / 24.0)).abs() < 1e-12);
}

#[test]
fn ending_backbone_borrows_other_paths_and_clips_either_terminal_arc() {
    let mut graph = graph(vec![
        vec![point(10.0, 10.0), point(30.0, 10.0), point(30.0, 30.0)],
        vec![point(5.0, 10.0), point(0.0, 10.0)],
    ]);
    graph.cycles.push(RidgeCycle {
        points: vec![point(40.0, 40.0), point(50.0, 40.0), point(45.0, 50.0)],
    });
    let original = graph.clone();
    let policy = policy(8.0);
    let arms = geometry::arms(&graph, &policy, &mut Work::default()).unwrap();

    for (node, horizon, expected) in [
        (
            0,
            16.0,
            vec![point(26.0, 10.0), point(30.0, 10.0), point(30.0, 30.0)],
        ),
        (
            1,
            16.0,
            vec![point(10.0, 10.0), point(30.0, 10.0), point(30.0, 14.0)],
        ),
        (0, 40.0, Vec::new()),
    ] {
        let backbone = EndingBackbone::new(&graph, node, 0, horizon);
        let edges: Vec<_> = backbone.edges().collect();
        assert_eq!(edges.len(), graph.edges.len());
        assert_eq!(edges[0], (expected.as_slice(), graph.edges[0].start));
        assert_eq!(edges[1].1, graph.edges[1].start);
        assert!(std::ptr::eq(edges[1].0, graph.edges[1].points.as_slice()));

        let paths: Vec<_> = backbone.paths().collect();
        assert!(!paths[0].1);
        assert!(!paths[1].1);
        assert!(paths[2].1);
        assert!(std::ptr::eq(paths[2].0, graph.cycles[0].points.as_slice()));

        let mut owned = original.clone();
        owned.edges[0].points = expected;
        assert_eq!(paths, graph_paths(&owned).collect::<Vec<_>>());
        assert_eq!(
            geometry::continuation(node, &arms, backbone.paths(), &policy, &mut Work::default())
                .unwrap(),
            geometry::continuation(
                node,
                &arms,
                graph_paths(&owned),
                &policy,
                &mut Work::default()
            )
            .unwrap()
        );
        assert_eq!(graph, original);
    }
}
