use super::assembly::assemble;
use super::*;
use crate::biometrics::fingerprint::{NodeKind, RidgeCycle, RidgeEdge, RidgeNode};

fn point(x: f64, y: f64) -> Point {
    Point::new(x, y).unwrap()
}

fn horizontal(start: usize, end: usize, y: usize) -> Vec<Point> {
    (start..=end).map(|x| point(x as f64, y as f64)).collect()
}

fn graph(paths: Vec<Vec<Point>>) -> RidgeGraph {
    let mut graph = RidgeGraph {
        nodes: Vec::new(),
        edges: Vec::new(),
        cycles: Vec::new(),
    };
    for path in paths {
        let start = graph.nodes.len();
        for position in [path[0], *path.last().unwrap()] {
            graph.nodes.push(RidgeNode {
                position,
                kind: NodeKind::Ending,
                touches_unobserved: true,
            });
        }
        graph.edges.push(RidgeEdge {
            start,
            end: start + 1,
            points: path,
        });
    }
    graph
}

fn fixture() -> (Capture, LocalPalette, RidgeGraph, RidgeMap) {
    let (width, height) = (80, 50);
    let mut pixels = vec![220; width * height];
    for y in 22..=28 {
        for x in 5..=65 {
            pixels[y * width + x] = 20;
        }
    }
    let capture = Capture::new(width as u32, height as u32, pixels, None, None).unwrap();
    let palette = LocalPalette {
        radius_px: 8,
        ridge_gray: vec![20.0; width * height],
        valley_gray: vec![220.0; width * height],
        usable: vec![true; width * height],
    };
    let graph = graph(vec![horizontal(5, 30, 25), horizontal(36, 65, 25)]);
    let mut ridges = vec![false; width * height];
    for edge in &graph.edges {
        for p in &edge.points {
            ridges[p.y() as usize * width + p.x() as usize] = true;
        }
    }
    let mut usable = vec![true; width * height];
    for x in 31..36 {
        usable[25 * width + x] = false;
    }
    let skeleton = RidgeMap::new(width, height, ridges, usable).unwrap();
    (capture, palette, graph, skeleton)
}

#[test]
fn restores_a_processing_gap_from_source_despite_enhancement_boundary_flags() {
    let (capture, palette, graph, skeleton) = fixture();
    let result = reconstruct(&capture, &palette, &graph, &skeleton, 8.0).unwrap();
    assert_eq!(result.joins.len(), 1);
    let join = &result.joins[0];
    assert_eq!(join.status, GapStatus::Accepted);
    assert_eq!((join.start_node, join.end_node), (1, 2));
    assert_eq!(join.gap_px, 6.0);
    let evidence = join.evidence.unwrap();
    assert!((evidence.mean_darkness - 1.0).abs() < 1e-12);
    assert!((evidence.dark_fraction - 1.0).abs() < 1e-12);
    assert!((evidence.mean_flank_contrast - 1.0).abs() < 1e-12);
    assert_eq!(result.node_sources, [0, 3]);
    assert_eq!(result.graph.edges.len(), 1);
    assert_eq!(
        result.graph.edges[0].points.first(),
        Some(&point(5.0, 25.0))
    );
    assert_eq!(
        result.graph.edges[0].points.last(),
        Some(&point(65.0, 25.0))
    );
    assert_eq!(
        result.edge_origins[0],
        PathOrigin {
            source_edges: vec![0, 1],
            source_cycles: vec![],
            source_joins: vec![0]
        }
    );
    assert_eq!(graph.edges.len(), 2);
}

#[test]
fn reconstruction_accepts_a_measured_width_above_the_bootstrap_period_limit() {
    // Width and period are independent dimensions. This prepared source has a
    // 129-pixel dark ridge and an eight-pixel processing gap in its skeleton.
    // Reconstruction must use that supplied measured width, not reject it using
    // the bootstrap search's unrelated 128-pixel period limit.
    let (width, height) = (360, 300);
    let mut pixels = vec![220; width * height];
    for y in 86..=214 {
        for x in 20..=340 {
            pixels[y * width + x] = 20;
        }
    }
    let capture = Capture::new(width as u32, height as u32, pixels, None, None).unwrap();
    let palette = LocalPalette {
        radius_px: 64,
        ridge_gray: vec![20.0; width * height],
        valley_gray: vec![220.0; width * height],
        usable: vec![true; width * height],
    };
    let input = graph(vec![horizontal(20, 170, 150), horizontal(178, 340, 150)]);
    let mut ridges = vec![false; width * height];
    for edge in &input.edges {
        for point in &edge.points {
            ridges[point.y() as usize * width + point.x() as usize] = true;
        }
    }
    let skeleton = RidgeMap::new(width, height, ridges, vec![true; width * height]).unwrap();

    let result = reconstruct(&capture, &palette, &input, &skeleton, 129.0).unwrap();
    assert_eq!(result.policy.max_gap_px, 129.0);
    assert_eq!(result.policy.flank_offset_px, 96.75);
    assert_eq!(result.joins.len(), 1);
    assert_eq!(result.joins[0].status, GapStatus::Accepted);
    assert_eq!(result.joins[0].gap_px, 8.0);
    assert_eq!(result.graph.edges.len(), 1);
    assert_eq!(result.node_sources, [0, 3]);
    assert_eq!(result.edge_origins[0].source_edges, [0, 1]);
    assert_eq!(result.edge_origins[0].source_joins, [0]);
}

#[test]
fn source_white_gap_remains_a_proposal_and_keeps_true_endings() {
    let (capture, palette, graph, skeleton) = fixture();
    let mut pixels = capture.pixels().to_vec();
    for y in 22..=28 {
        for x in 31..36 {
            pixels[y * 80 + x] = 220;
        }
    }
    let capture = Capture::new(80, 50, pixels, None, None).unwrap();
    let result = reconstruct(&capture, &palette, &graph, &skeleton, 8.0).unwrap();
    assert_eq!(result.joins[0].status, GapStatus::ImageUnsupported);
    assert!(result.joins[0].evidence.unwrap().dark_fraction < 0.3);
    assert_eq!(result.graph, graph);
}

#[test]
fn an_acquisition_gap_or_missing_local_palette_is_never_reconstructed() {
    let (capture, mut palette, graph, skeleton) = fixture();
    let mut mask = vec![true; capture.pixels().len()];
    mask[25 * 80 + 33] = false;
    let source = Capture::new(80, 50, capture.pixels().to_vec(), Some(mask.clone()), None).unwrap();
    let first = reconstruct(&source, &palette, &graph, &skeleton, 8.0).unwrap();
    assert_eq!(first.joins[0].status, GapStatus::Unobserved);
    assert_eq!(first.graph, graph);
    let mut pixels = source.pixels().to_vec();
    pixels[25 * 80 + 33] = 255;
    let hidden = Capture::new(80, 50, pixels, Some(mask), None).unwrap();
    assert_eq!(
        first.joins,
        reconstruct(&hidden, &palette, &graph, &skeleton, 8.0)
            .unwrap()
            .joins
    );
    palette.usable[19 * 80 + 33] = false;
    let missing_flank = reconstruct(&capture, &palette, &graph, &skeleton, 8.0).unwrap();
    assert_eq!(missing_flank.joins[0].status, GapStatus::Unobserved);
}

#[test]
fn rejects_crossing_another_ridge_and_retains_it_in_output() {
    let (capture, palette, mut graph, skeleton) = fixture();
    let start = graph.nodes.len();
    for position in [point(33.0, 10.0), point(33.0, 40.0)] {
        graph.nodes.push(RidgeNode {
            position,
            kind: NodeKind::Ending,
            touches_unobserved: false,
        });
    }
    graph.edges.push(RidgeEdge {
        start,
        end: start + 1,
        points: (10..=40).map(|y| point(33.0, y as f64)).collect(),
    });
    let result = reconstruct(&capture, &palette, &graph, &skeleton, 8.0).unwrap();
    assert_eq!(result.joins.len(), 1);
    assert_eq!(result.joins[0].status, GapStatus::Obstructed);
    assert_eq!(result.graph, graph);
}

fn join(start_node: usize, end_node: usize, points: Vec<Point>) -> GapJoin {
    GapJoin {
        start_node,
        end_node,
        gap_px: distance(points[0], *points.last().unwrap()),
        points,
        max_tangent_error_radians: 0.0,
        evidence: None,
        status: GapStatus::Accepted,
    }
}

#[test]
fn ambiguous_supported_partners_are_all_rejected_without_greedy_ordering() {
    let policy = GapPolicy::for_unit(8.0).unwrap();
    let a = join(0, 1, vec![point(0.0, 0.0), point(6.0, 1.0)]);
    let b = join(0, 2, vec![point(0.0, 0.0), point(6.0, -1.0)]);
    for mut joins in [vec![a.clone(), b.clone()], vec![b.clone(), a.clone()]] {
        resolve_conflicts(&mut joins, 3, policy).unwrap();
        assert!(joins.iter().all(|join| join.status == GapStatus::Ambiguous));
    }
}

#[test]
fn crossing_bridges_are_both_rejected_without_ordering_preference() {
    let mut joins = vec![
        join(0, 1, vec![point(0.0, 4.0), point(8.0, 4.0)]),
        join(2, 3, vec![point(4.0, 0.0), point(4.0, 8.0)]),
    ];
    resolve_conflicts(&mut joins, 4, GapPolicy::for_unit(8.0).unwrap()).unwrap();
    assert!(
        joins
            .iter()
            .all(|join| join.status == GapStatus::Obstructed)
    );
}

#[test]
fn assembly_handles_reversed_paths_multiple_joins_and_closed_components() {
    let mut input = graph(vec![
        horizontal(5, 20, 25),
        horizontal(25, 40, 25),
        horizontal(45, 60, 25),
    ]);
    input.edges[1].points.reverse();
    let edge = &mut input.edges[1];
    std::mem::swap(&mut edge.start, &mut edge.end);
    let joins = vec![
        join(1, 2, horizontal(20, 25, 25)),
        join(3, 4, horizontal(40, 45, 25)),
    ];
    let result = assemble(
        &input,
        &incidence(&input).unwrap(),
        joins,
        GapPolicy::for_unit(8.0).unwrap(),
    )
    .unwrap();
    assert_eq!(result.graph.edges.len(), 1);
    assert_eq!(result.node_sources, [0, 5]);
    assert_eq!(result.graph.edges[0].points, horizontal(5, 60, 25));
    assert_eq!(result.edge_origins[0].source_edges, [0, 1, 2]);
    assert_eq!(result.edge_origins[0].source_joins, [0, 1]);

    let graph = graph(vec![vec![
        point(10.0, 10.0),
        point(10.0, 30.0),
        point(30.0, 30.0),
        point(30.0, 10.0),
    ]]);
    let closure = join(0, 1, vec![point(10.0, 10.0), point(30.0, 10.0)]);
    let result = assemble(
        &graph,
        &incidence(&graph).unwrap(),
        vec![closure],
        GapPolicy::for_unit(8.0).unwrap(),
    )
    .unwrap();
    assert!(result.graph.nodes.is_empty());
    assert!(result.graph.edges.is_empty());
    assert_eq!(result.graph.cycles.len(), 1);
    assert_eq!(result.graph.cycles[0].points, graph.edges[0].points);
    assert_eq!(result.cycle_origins[0].source_edges, [0]);
    assert_eq!(result.cycle_origins[0].source_joins, [0]);
}

#[test]
fn short_edges_long_gaps_and_nonfacing_endings_are_not_fabricated() {
    let (capture, palette, _, skeleton) = fixture();
    for graph in [
        graph(vec![horizontal(25, 30, 25), horizontal(36, 40, 25)]),
        graph(vec![horizontal(5, 25, 25), horizontal(36, 65, 25)]),
        graph(vec![
            horizontal(5, 30, 25),
            (30..=45).map(|y| point(36.0, y as f64)).collect(),
        ]),
    ] {
        let result = reconstruct(&capture, &palette, &graph, &skeleton, 8.0).unwrap();
        assert!(result.joins.is_empty());
        assert_eq!(result.graph, graph);
    }
}

#[test]
fn a_bridge_bowing_outside_the_capture_is_not_exported_as_a_proposal() {
    let (capture, palette, _, skeleton) = fixture();
    let input = graph(vec![
        (5..=30)
            .map(|x| point(x as f64, (30 - x) as f64 / 10.0))
            .collect(),
        (36..=65)
            .map(|x| point(x as f64, (x - 36) as f64 / 10.0))
            .collect(),
    ]);
    let result = reconstruct(&capture, &palette, &input, &skeleton, 8.0).unwrap();
    assert!(result.joins.is_empty());
    assert_eq!(result.graph, input);
}

#[test]
fn existing_closed_paths_and_distinct_junction_ports_remain_unchanged() {
    let (capture, palette, _, skeleton) = fixture();
    let input = RidgeGraph {
        nodes: vec![RidgeNode {
            position: point(10.0, 15.0),
            kind: NodeKind::Junction,
            touches_unobserved: false,
        }],
        edges: vec![RidgeEdge {
            start: 0,
            end: 0,
            points: vec![
                point(10.0, 10.0),
                point(20.0, 10.0),
                point(20.0, 20.0),
                point(10.0, 20.0),
            ],
        }],
        cycles: vec![RidgeCycle {
            points: vec![point(40.0, 10.0), point(50.0, 10.0), point(45.0, 20.0)],
        }],
    };
    let result = reconstruct(&capture, &palette, &input, &skeleton, 8.0).unwrap();
    assert_eq!(result.graph, input);
    assert_eq!(result.edge_origins[0].source_edges, [0]);
    assert_eq!(result.cycle_origins[0].source_cycles, [0]);
}

#[test]
fn an_oblique_corner_clip_cannot_skip_an_unobserved_bilinear_cell() {
    let mut mask = vec![true; 9];
    mask[6] = false;
    let capture = Capture::new(3, 3, vec![20; 9], Some(mask), None).unwrap();
    let mut palette = LocalPalette {
        radius_px: 1,
        ridge_gray: vec![20.0; 9],
        valley_gray: vec![220.0; 9],
        usable: vec![true; 9],
    };
    let a = point(0.95, 0.96);
    let b = point(1.15, 1.159);
    let middle = point((a.x() + b.x()) / 2.0, (a.y() + b.y()) / 2.0);
    assert!(distance(a, b) < 0.5);
    for p in [a, middle, b] {
        assert!(sample_darkness(&capture, &palette, p).is_some());
    }
    assert!(segment_observed(&capture, &palette, a, b).is_none());
    let unmasked = Capture::new(3, 3, vec![20; 9], None, None).unwrap();
    palette.usable[6] = false;
    assert!(segment_observed(&unmasked, &palette, a, b).is_none());
}

#[test]
fn adjacent_float_corner_crossings_check_capture_and_palette_support() {
    let mut mask = vec![true; 9];
    mask[6] = false;
    let capture = Capture::new(3, 3, vec![20; 9], Some(mask), None).unwrap();
    let unmasked = Capture::new(3, 3, vec![20; 9], None, None).unwrap();
    let mut palette = LocalPalette {
        radius_px: 1,
        ridge_gray: vec![20.0; 9],
        valley_gray: vec![220.0; 9],
        usable: vec![true; 9],
    };
    let (a, b) = (point(0.0, 0.0), point(1.4999999999999998, 1.5));
    assert!(segment_observed(&unmasked, &palette, a, b).is_some());
    for (start, end) in [(a, b), (b, a)] {
        assert!(sample_darkness(&capture, &palette, start).is_some());
        assert!(sample_darkness(&capture, &palette, end).is_some());
        assert!(segment_observed(&capture, &palette, start, end).is_none());
    }
    palette.usable[6] = false;
    assert!(segment_observed(&unmasked, &palette, a, b).is_none());
    assert!(segment_observed(&unmasked, &palette, b, a).is_none());
}

#[test]
fn raw_junction_interior_pixels_remain_obstacles_even_if_absent_from_graph_paths() {
    let (capture, palette, graph, mut skeleton) = fixture();
    skeleton.ridges[25 * 80 + 33] = true;
    skeleton.usable[25 * 80 + 33] = true;
    let result = reconstruct(&capture, &palette, &graph, &skeleton, 8.0).unwrap();
    assert_eq!(result.joins[0].status, GapStatus::Obstructed);
    assert_eq!(result.graph, graph);
}
