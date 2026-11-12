use super::selection::components;
use super::*;
use std::collections::BTreeSet;

const PADDING: usize = 32;

fn map(rows: &[&str]) -> RidgeMap {
    let width = rows[0].len() + 2 * PADDING;
    let height = rows.len() + 2 * PADDING;
    let mut pixels = vec![false; width * height];
    for (y, row) in rows.iter().enumerate() {
        assert_eq!(row.len() + 2 * PADDING, width);
        for (x, cell) in row.bytes().enumerate() {
            pixels[(y + PADDING) * width + x + PADDING] = cell == b'#';
        }
    }
    RidgeMap::new(width, height, pixels, vec![true; width * height]).unwrap()
}

fn source_image(map: &RidgeMap) -> (Capture, LocalPalette) {
    let capture = Capture::new(
        map.width as u32,
        map.height as u32,
        map.ridges
            .iter()
            .map(|&ridge| if ridge { 30 } else { 220 })
            .collect(),
        None,
        None,
    )
    .unwrap();
    let palette = LocalPalette {
        radius_px: 1,
        ridge_gray: vec![30.0; map.ridges.len()],
        valley_gray: vec![220.0; map.ridges.len()],
        usable: vec![true; map.ridges.len()],
    };
    (capture, palette)
}

fn run(map: &RidgeMap, unit: f64) -> (graph::TracedGraph, Cleaned) {
    let observed = graph::trace_with_pixels(map).unwrap();
    let (capture, palette) = source_image(map);
    let cleaned = clean(&capture, &palette, map, &observed, unit).unwrap();
    assert_topology(&observed.graph, &cleaned.result);
    assert_partition(&observed.graph, &cleaned.result);
    (observed, cleaned)
}

fn counts(graph: &RidgeGraph) -> (usize, isize) {
    let mut incidence = vec![Vec::new(); graph.nodes.len()];
    for (id, edge) in graph.edges.iter().enumerate() {
        incidence[edge.start].push(id);
        incidence[edge.end].push(id);
    }
    for (node, edges) in graph.nodes.iter().zip(&incidence) {
        assert_eq!(
            node.kind,
            match edges.len() {
                0 => NodeKind::Isolated,
                1 => NodeKind::Ending,
                _ => NodeKind::Junction,
            }
        );
        assert_ne!(
            edges.len(),
            2,
            "degree-two vertex should be traced as a path"
        );
    }
    let parts = components(graph, &incidence)
        .into_iter()
        .collect::<BTreeSet<_>>()
        .len();
    let count = parts + graph.cycles.len();
    let rank = graph.edges.len() as isize - graph.nodes.len() as isize
        + parts as isize
        + graph.cycles.len() as isize;
    (count, rank)
}

fn assert_topology(before: &RidgeGraph, after: &RidgeCleanup) {
    let expected = counts(before);
    let removed_cycles = after
        .loops
        .iter()
        .filter(|item| item.removed_edge.is_some())
        .count() as isize;
    assert_eq!(
        (expected.0, expected.1 - removed_cycles),
        counts(&after.graph)
    );
}

fn assert_partition(before: &RidgeGraph, after: &RidgeCleanup) {
    let mut edges = vec![0; before.edges.len()];
    let mut cycles = vec![0; before.cycles.len()];
    let mut routes = vec![0; after.junction_routes.len()];
    for path in after.edge_origins.iter().chain(&after.cycle_origins) {
        for &id in &path.source_edges {
            edges[id] += 1;
        }
        for &id in &path.source_cycles {
            cycles[id] += 1;
        }
        for &id in &path.source_routes {
            routes[id] += 1;
        }
    }
    for spur in &after.removed_spurs {
        edges[spur.source_edge] += 1;
    }
    for edge in after.loops.iter().filter_map(|item| item.removed_edge) {
        edges[edge] += 1;
    }
    assert!(
        edges
            .iter()
            .chain(&cycles)
            .chain(&routes)
            .all(|&count| count == 1)
    );
}

#[test]
fn one_short_spur_becomes_continuation_and_two_short_arms_become_an_ending() {
    let source = map(&["....#....", "....#....", "#########"]);
    let (_, cleaned) = run(&source, 2.5);
    assert_eq!(cleaned.result.removed_spurs.len(), 1);
    assert_eq!(cleaned.result.graph.edges.len(), 1);
    assert_eq!(cleaned.result.graph.nodes.len(), 2);
    assert_eq!(cleaned.skeleton.ridges.iter().filter(|&&v| v).count(), 9);

    let source = map(&["....#.", "....#.", "######"]);
    let (_, literal) = run(&source, 2.5);
    assert_eq!(literal.result.removed_spurs.len(), 2);
    assert_eq!(literal.result.graph.nodes.len(), 2);
    let observed = graph::trace_with_pixels(&source).unwrap();
    // The observed vertical arm is a processing artifact on the end of the
    // original horizontal ridge. Both short arms have supported terminations.
    let capture = grayscale(&source, |x, y| {
        if y == 2 && (0..=5).contains(&x) {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&source);
    let cleaned = clean(&capture, &palette, &source, &observed, 2.5).unwrap();
    assert_topology(&observed.graph, &cleaned.result);
    assert_partition(&observed.graph, &cleaned.result);
    assert_eq!(cleaned.result.removed_spurs.len(), 2);
    assert_eq!(cleaned.result.graph.edges.len(), 1);
    assert_eq!(cleaned.result.graph.nodes.len(), 2);
}

#[test]
fn strict_tolerant_width_limit_keeps_equality_and_removes_only_below() {
    let mut source = map(&["........................................."; 15]);
    for x in 0..=40 {
        put(&mut source, x, 12);
    }
    for y in 0..12 {
        put(&mut source, 20, y);
    }
    for (nominal, removed) in [
        (f64::from_bits(10.0_f64.to_bits() - 1), 0),
        (10.0, 0),
        (f64::from_bits(10.0_f64.to_bits() + 1), 1),
    ] {
        let (_, cleaned) = run(&source, nominal);
        assert_eq!(cleaned.result.cursor_diameter_px, nominal);
        assert_eq!(
            cleaned.result.source_policy.max_ridge_width_px,
            1.2 * nominal
        );
        assert_eq!(cleaned.result.removed_spurs.len(), removed);
    }
}

#[test]
fn all_short_components_and_isolated_fragments_remain_unchanged() {
    for source in [map(&[".#.", "###"]), map(&["###"]), map(&["#"])] {
        let (observed, cleaned) = run(&source, 10.0);
        assert_eq!(cleaned.result.graph, observed.graph);
        assert!(cleaned.result.removed_spurs.is_empty());
    }
    let (_, cleaned) = run(&map(&[".#.", "###"]), 10.0);
    assert_eq!(cleaned.result.retained_ambiguous_edges.len(), 3);
}

#[test]
fn processing_unknown_is_resolved_but_missing_source_is_preserved() {
    let source = map(&["....#....", "....#....", "#########"]);
    let traced = graph::trace_with_pixels(&source).unwrap();
    let (short_id, short) = traced
        .graph
        .edges
        .iter()
        .enumerate()
        .find(|(_, edge)| edge.points.len() == 3)
        .unwrap();
    for point in &short.points {
        let mut masked = source.clone();
        let index = pixel(&masked, *point);
        masked.boundary_origin[index] = true;
        let (_, cleaned) = run(&masked, 2.5);
        assert_eq!(cleaned.result.removed_spurs.len(), 1);
        assert_eq!(cleaned.result.removed_spurs[0].source_edge, short_id);
        assert!(cleaned.result.retained_unobserved_edges.is_empty());
        let (original, palette) = source_image(&masked);
        let mut valid = vec![true; original.pixels().len()];
        valid[index] = false;
        let capture = Capture::new(
            original.width(),
            original.height(),
            original.pixels().to_vec(),
            Some(valid),
            None,
        )
        .unwrap();
        let traced = graph::trace_with_pixels(&masked).unwrap();
        let unresolved = clean(&capture, &palette, &masked, &traced, 2.5).unwrap();
        assert!(unresolved.result.removed_spurs.is_empty());
        assert_eq!(unresolved.result.retained_unobserved_edges, [short_id]);
        let (_, long_unknown) = run(&masked, 1.5);
        assert!(long_unknown.result.retained_unobserved_edges.is_empty());
    }
}

#[test]
fn adjacent_junction_ports_follow_observed_cluster_links() {
    let source = map(&["...##....", "...##....", "#########"]);
    let (observed, cleaned) = run(&source, 2.5);
    assert!(!cleaned.result.removed_spurs.is_empty());
    assert!(!cleaned.result.junction_routes.is_empty());
    for route in &cleaned.result.junction_routes {
        let cluster = &observed.node_pixels[route.source_node];
        for (a, b) in segments(&route.points, false) {
            let (a, b) = (pixel(&source, a), pixel(&source, b));
            assert!(cluster.contains(&a) && cluster.contains(&b));
            assert!(graph::neighbors(&source, a).contains(&Some(b)));
        }
    }
}

#[test]
fn removing_a_tail_retains_an_attached_loop() {
    let source = map(&["..#..", "..#..", ".###.", ".#.#.", ".###."]);
    let (_, cleaned) = run(&source, 2.5);
    assert_eq!(cleaned.result.removed_spurs.len(), 1);
    assert_eq!(counts(&cleaned.result.graph), (1, 1));
    assert_eq!(cleaned.result.graph.cycles.len(), 1);
}

#[test]
fn cyclic_junction_vertices_keep_their_sources_after_one_arm_is_removed() {
    let source = map(&[
        "...........",
        "...........",
        "...........",
        ".....#.....",
        ".....#.....",
        ".####.####.",
        ".....#.....",
        ".....#.....",
        ".....#.....",
        ".....#.....",
        "...........",
    ]);
    let (_, cleaned) = run(&source, 1.1);
    assert_eq!(cleaned.result.removed_spurs.len(), 1);
    assert_eq!(counts(&cleaned.result.graph), (1, 1));
    assert_eq!(
        cleaned
            .result
            .graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Junction)
            .count(),
        3
    );
}

#[test]
fn short_new_terminal_paths_are_not_removed_in_a_second_pass() {
    let mut source = map(&["............."; 13]);
    for x in 2..=12 {
        source.ridges[(PADDING + 6) * source.width + x + PADDING] = true;
    }
    for y in 0..=12 {
        source.ridges[(y + PADDING) * source.width + PADDING + 6] = true;
    }
    source.ridges[(PADDING + 5) * source.width + PADDING + 3] = true;
    let (observed, cleaned) = run(&source, 4.0);
    let original_middle = observed
        .graph
        .edges
        .iter()
        .position(|edge| {
            observed.graph.nodes[edge.start].kind == NodeKind::Junction
                && observed.graph.nodes[edge.end].kind == NodeKind::Junction
        })
        .unwrap();
    assert_eq!(cleaned.result.removed_spurs.len(), 2);
    let retained = cleaned
        .result
        .edge_origins
        .iter()
        .position(|origin| origin.source_edges.contains(&original_middle))
        .unwrap();
    let edge = &cleaned.result.graph.edges[retained];
    assert!(
        edge.points
            .windows(2)
            .map(|pair| distance(pair[0], pair[1]))
            .sum::<f64>()
            < 4.0
    );
    assert!(
        cleaned.result.graph.nodes[edge.start].kind == NodeKind::Ending
            || cleaned.result.graph.nodes[edge.end].kind == NodeKind::Ending
    );
}

#[test]
fn a_curved_terminal_inside_a_small_radius_is_kept_when_its_arc_is_long() {
    let source = map(&[
        "#.....", "#.....", "#.....", "#.###.", "#...#.", "#####.", "#.....", "#.....", "#.....",
        "#.....", "#.....",
    ]);
    let (observed, cleaned) = run(&source, 5.0);
    let (curved_id, curved) = observed
        .graph
        .edges
        .iter()
        .enumerate()
        .find(|(_, edge)| {
            distance(edge.points[0], *edge.points.last().unwrap()) < 3.0
                && edge
                    .points
                    .windows(2)
                    .map(|pair| distance(pair[0], pair[1]))
                    .sum::<f64>()
                    >= cleaned.result.source_policy.max_ridge_width_px
        })
        .unwrap();
    assert!(curved.points.len() > 5);
    assert!(
        !cleaned
            .result
            .removed_spurs
            .iter()
            .any(|spur| spur.source_edge == curved_id)
    );
}

#[test]
fn every_small_raster_preserves_components_cycles_and_path_partition() {
    for pattern in 0..512 {
        let mut source = map(&["...", "...", "..."]);
        for bit in 0..9 {
            source.ridges[(bit / 3 + PADDING) * source.width + bit % 3 + PADDING] =
                pattern & (1 << bit) != 0;
        }
        for diameter in [1.0, 2.0, 5.0] {
            run(&source, diameter);
        }
    }
}

#[test]
fn invalid_diameters_are_rejected_without_partial_cleanup() {
    let source = map(&["###"]);
    let observed = graph::trace_with_pixels(&source).unwrap();
    let (capture, palette) = source_image(&source);
    for diameter in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(clean(&capture, &palette, &source, &observed, diameter).is_err());
    }
}

fn put(map: &mut RidgeMap, x: usize, y: usize) {
    map.ridges[(y + PADDING) * map.width + x + PADDING] = true;
}

fn grayscale(map: &RidgeMap, gray: impl Fn(isize, isize) -> u8) -> Capture {
    Capture::new(
        map.width as u32,
        map.height as u32,
        (0..map.ridges.len())
            .map(|index| {
                gray(
                    (index % map.width) as isize - PADDING as isize,
                    (index / map.width) as isize - PADDING as isize,
                )
            })
            .collect(),
        None,
        None,
    )
    .unwrap()
}

fn loop_fixture(with_spur: bool) -> (RidgeMap, Capture, LocalPalette) {
    let mut map = map(&["..................................................."; 20]);
    for x in 0..=48 {
        put(&mut map, x, 10);
    }
    for x in 20..=40 {
        put(&mut map, x, 12);
    }
    put(&mut map, 20, 11);
    put(&mut map, 40, 11);
    if with_spur {
        put(&mut map, 14, 8);
        put(&mut map, 14, 9);
    }
    let capture = grayscale(&map, |_, y| (30 + 35 * (y - 10).abs()).min(220) as u8);
    let (_, palette) = source_image(&map);
    (map, capture, palette)
}

#[test]
fn a_supported_narrow_loop_keeps_the_stronger_original_arm_and_drops_one_cycle() {
    let (map, capture, palette) = loop_fixture(false);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let cleaned = clean(&capture, &palette, &map, &observed, 8.0).unwrap();
    assert_topology(&observed.graph, &cleaned.result);
    assert_partition(&observed.graph, &cleaned.result);
    assert_eq!(cleaned.result.loops.len(), 1);
    let loop_result = &cleaned.result.loops[0];
    assert_eq!(loop_result.evidence.decision, SourceDecision::Confirmed);
    assert!(loop_result.removed_edge.is_some());
    for &edge in &loop_result.source_edges {
        assert!(
            observed.graph.edges[edge]
                .points
                .windows(2)
                .map(|pair| distance(pair[0], pair[1]))
                .sum::<f64>()
                > 8.0
        );
    }
    assert_eq!(counts(&cleaned.result.graph), (1, 0));
    assert_eq!(cleaned.result.graph.edges.len(), 1);
    assert!(
        cleaned.result.graph.edges[0]
            .points
            .iter()
            .all(|point| point.y() == (PADDING + 10) as f64)
    );
}

#[test]
fn simultaneous_spur_and_loop_cleanup_preserves_components_and_exact_sources() {
    let (map, capture, palette) = loop_fixture(true);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let cleaned = clean(&capture, &palette, &map, &observed, 8.0).unwrap();
    assert_topology(&observed.graph, &cleaned.result);
    assert_partition(&observed.graph, &cleaned.result);
    assert_eq!(cleaned.result.removed_spurs.len(), 1);
    assert_eq!(
        cleaned
            .result
            .loops
            .iter()
            .filter(|item| item.removed_edge.is_some())
            .count(),
        1
    );
    assert_eq!(cleaned.result.graph.edges.len(), 1);
}

#[test]
fn a_real_valley_or_missing_original_pixels_preserve_a_loop() {
    let (map, single_ridge, palette) = loop_fixture(false);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let valley = grayscale(&map, |x, y| {
        if (24..=36).contains(&x) && y == 11 {
            220
        } else {
            (30 + 35 * (y - 10).abs()).min(220) as u8
        }
    });
    let preserved = clean(&valley, &palette, &map, &observed, 8.0).unwrap();
    assert_eq!(
        preserved.result.loops[0].evidence.decision,
        SourceDecision::SeparateRidges
    );
    assert_eq!(preserved.result.graph, observed.graph);
    let index = (PADDING + 11) * map.width + PADDING + 30;
    let mut valid = vec![true; map.ridges.len()];
    valid[index] = false;
    let mut results = Vec::new();
    for hidden in [0, 255] {
        let mut pixels = single_ridge.pixels().to_vec();
        pixels[index] = hidden;
        let capture = Capture::new(
            map.width as u32,
            map.height as u32,
            pixels,
            Some(valid.clone()),
            None,
        )
        .unwrap();
        let result = clean(&capture, &palette, &map, &observed, 8.0)
            .unwrap()
            .result;
        assert_eq!(
            result.loops[0].evidence.decision,
            SourceDecision::MissingSupport
        );
        assert_eq!(result.graph, observed.graph);
        results.push(result);
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn a_direct_observed_link_without_removable_pixels_is_retained_explicitly() {
    let point = |x, y| Point::new(x, y).unwrap();
    let graph = RidgeGraph {
        nodes: vec![],
        cycles: vec![],
        edges: vec![
            super::super::RidgeEdge {
                start: 0,
                end: 1,
                points: vec![point(10.0, 10.0), point(10.0, 11.0)],
            },
            super::super::RidgeEdge {
                start: 0,
                end: 1,
                points: vec![
                    point(10.0, 10.0),
                    point(12.0, 10.0),
                    point(12.0, 11.0),
                    point(10.0, 11.0),
                ],
            },
        ],
    };
    let mut evidence = LoopEvidence::missing_support();
    evidence.decision = SourceDecision::Confirmed;
    evidence.preferred_arm = Some(1);
    assert_eq!(
        selection::removable_arm(&graph, [0, 1], &mut evidence),
        None
    );
    assert_eq!(evidence.decision, SourceDecision::AmbiguousGeometry);
    assert_eq!(evidence.preferred_arm, None);
}

#[test]
fn a_real_curved_continuation_cannot_escape_the_cap_or_use_its_parent_as_an_excuse() {
    let mut map = map(&["................................................."; 49]);
    for y in 8..=40 {
        put(&mut map, 20, y);
    }
    for x in 21..=24 {
        put(&mut map, x, 24);
    }
    let capture = grayscale(&map, |x, y| {
        if ((18..=22).contains(&x) && (8..=40).contains(&y))
            || ((20..=28).contains(&x) && (22..=26).contains(&y))
            || ((26..=28).contains(&x) && (12..=26).contains(&y))
        {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&map);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let spur = observed
        .graph
        .edges
        .iter()
        .find(|edge| edge.points.len() == 5)
        .unwrap();
    let points: Vec<_> = if observed.graph.nodes[spur.start].kind == NodeKind::Junction {
        spur.points.clone()
    } else {
        spur.points.iter().rev().copied().collect()
    };
    assert_eq!(
        source::verify_spur(&capture, &palette, &points, 8.0)
            .unwrap()
            .decision,
        SourceDecision::Confirmed
    );
    let result = clean(&capture, &palette, &map, &observed, 8.0)
        .unwrap()
        .result;
    assert!(result.removed_spurs.is_empty());
    assert_eq!(result.retained_unconfirmed_edges.len(), 1);
    assert_eq!(
        result.spur_assessments[0].evidence.decision,
        SourceDecision::PossibleContinuation
    );
    assert!(
        result.spur_assessments[0]
            .evidence
            .light_barrier_distance_px
            .is_some()
    );
    assert_eq!(result.graph, observed.graph);

    // This pixel lies outside the original forward cap but inside the side
    // continuation inspection. An earlier cap cannot bypass new missing data.
    let mut mask = vec![true; map.ridges.len()];
    mask[(PADDING + 17) * map.width + PADDING + 25] = false;
    let masked = Capture::new(
        map.width as u32,
        map.height as u32,
        capture.pixels().to_vec(),
        Some(mask),
        None,
    )
    .unwrap();
    let result = clean(&masked, &palette, &map, &observed, 8.0)
        .unwrap()
        .result;
    assert_eq!(result.retained_unobserved_edges.len(), 1);
    assert_eq!(
        result.spur_assessments[0].evidence.decision,
        SourceDecision::MissingSupport
    );
    assert!(
        result.spur_assessments[0]
            .evidence
            .light_barrier_distance_px
            .is_some()
    );
}

#[test]
fn a_light_overshot_tip_does_not_hide_a_real_bend_earlier_on_the_branch() {
    let mut map = map(&["................................................."; 49]);
    for y in 8..=40 {
        put(&mut map, 20, y);
    }
    for x in 21..=26 {
        put(&mut map, x, 20);
    }
    let capture = grayscale(&map, |x, y| {
        if (x == 20 && (8..=40).contains(&y))
            || (y == 20 && (20..=24).contains(&x))
            || (x == 24 && (12..=20).contains(&y))
        {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&map);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let spur = observed
        .graph
        .edges
        .iter()
        .find(|edge| edge.points.len() == 7)
        .unwrap();
    let points: Vec<_> = if observed.graph.nodes[spur.start].kind == NodeKind::Junction {
        spur.points.clone()
    } else {
        spur.points.iter().rev().copied().collect()
    };
    let cap = source::verify_spur(&capture, &palette, &points, 8.0).unwrap();
    assert_eq!(cap.decision, SourceDecision::Confirmed);
    assert_eq!(cap.terminal_darkness, Some(0.0));
    let result = clean(&capture, &palette, &map, &observed, 8.0)
        .unwrap()
        .result;
    assert!(result.removed_spurs.is_empty());
    assert_eq!(result.retained_unconfirmed_edges.len(), 1);
    assert_eq!(
        result.spur_assessments[0].evidence.decision,
        SourceDecision::PossibleContinuation
    );
    assert_eq!(result.graph, observed.graph);
}

#[test]
fn a_short_bent_branch_cannot_return_through_its_attachment_for_support() {
    let mut map = map(&["................................................."; 49]);
    for y in 8..=40 {
        put(&mut map, 20, y);
    }
    for x in 21..=23 {
        put(&mut map, x, 24);
    }
    let capture = grayscale(&map, |x, y| {
        if ((18..=22).contains(&x) && (8..=40).contains(&y))
            || ((20..=27).contains(&x) && (22..=26).contains(&y))
            || ((25..=27).contains(&x) && (8..=26).contains(&y))
        {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&map);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let result = clean(&capture, &palette, &map, &observed, 12.0)
        .unwrap()
        .result;
    assert_eq!(result.spur_assessments.len(), 1);
    assert_eq!(
        result.spur_assessments[0].evidence.decision,
        SourceDecision::PossibleContinuation
    );
    assert!(result.removed_spurs.is_empty());
    assert_eq!(result.graph, observed.graph);
}

#[test]
fn a_continuation_initially_inside_the_backbone_footprint_can_still_peel_away() {
    let mut map = map(&["................................................."; 49]);
    for y in 8..=40 {
        put(&mut map, 20, y);
    }
    for x in 21..=24 {
        put(&mut map, x, 24);
    }
    let capture = grayscale(&map, |x, y| {
        if ((18..=24).contains(&x) && (8..=40).contains(&y))
            || ((24..=30).contains(&x) && (17..=19).contains(&y))
        {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&map);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let result = clean(&capture, &palette, &map, &observed, 8.0)
        .unwrap()
        .result;
    assert_eq!(result.spur_assessments.len(), 1);
    assert_eq!(
        result.spur_assessments[0].evidence.decision,
        SourceDecision::PossibleContinuation
    );
    assert!(result.removed_spurs.is_empty());
    assert_eq!(result.graph, observed.graph);
}

#[test]
fn a_wide_source_ridge_can_continue_past_the_observed_backbone_endpoint() {
    let mut map = map(&["................................................."; 49]);
    for x in 20..=45 {
        put(&mut map, x, 24);
    }
    for y in 22..=26 {
        put(&mut map, 20, y);
    }
    let capture = grayscale(&map, |x, y| {
        if (8..=48).contains(&x) && (21..=27).contains(&y) {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&map);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let result = clean(&capture, &palette, &map, &observed, 8.0)
        .unwrap()
        .result;
    assert_eq!(result.spur_assessments.len(), 2);
    assert_eq!(result.removed_spurs.len(), 2);
    assert_eq!(result.graph.edges.len(), 1);
    assert!(
        result.graph.edges[0]
            .points
            .iter()
            .all(|p| p.y() == (PADDING + 24) as f64)
    );
    assert_topology(&observed.graph, &result);
    assert_partition(&observed.graph, &result);
}

#[test]
fn a_finite_width_bend_behind_a_light_overshot_tip_is_preserved() {
    let mut map = map(&["................................................."; 49]);
    for y in 8..=40 {
        put(&mut map, 20, y);
    }
    for x in 21..=28 {
        put(&mut map, x, 24);
    }
    let capture = grayscale(&map, |x, y| {
        if ((18..=22).contains(&x) && (8..=40).contains(&y))
            || ((20..=27).contains(&x) && (22..=26).contains(&y))
            || ((24..=26).contains(&x) && (10..=26).contains(&y))
        {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&map);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let result = clean(&capture, &palette, &map, &observed, 12.0)
        .unwrap()
        .result;
    assert_eq!(result.spur_assessments.len(), 1);
    assert_eq!(
        result.spur_assessments[0].evidence.decision,
        SourceDecision::PossibleContinuation
    );
    assert!(result.removed_spurs.is_empty());
    assert_eq!(result.graph, observed.graph);
}

#[test]
fn a_dark_tip_cannot_hide_a_real_bend_one_pixel_behind_it() {
    let mut map = map(&["................................................."; 49]);
    for y in 8..=40 {
        put(&mut map, 20, y);
    }
    for x in 21..=24 {
        put(&mut map, x, 24);
    }
    let capture = grayscale(&map, |x, y| {
        if (x == 20 && (8..=40).contains(&y))
            || (y == 24 && (20..=24).contains(&x))
            || (x == 23 && (12..=24).contains(&y))
        {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&map);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let result = clean(&capture, &palette, &map, &observed, 8.0)
        .unwrap()
        .result;
    assert_eq!(result.spur_assessments.len(), 1);
    assert_eq!(
        result.spur_assessments[0].evidence.decision,
        SourceDecision::PossibleContinuation
    );
    assert!(result.removed_spurs.is_empty());
    assert_eq!(result.graph, observed.graph);
}

#[test]
fn a_confirmed_cap_on_another_short_path_does_not_hide_its_bent_continuation() {
    let mut map = map(&["................................................."; 49]);
    for y in 4..=44 {
        put(&mut map, 20, y);
    }
    for x in 16..=24 {
        put(&mut map, x, 24);
    }
    let capture = grayscale(&map, |x, y| {
        if (x == 20 && (4..=44).contains(&y))
            || (y == 24 && (17..=24).contains(&x))
            || (x == 17 && (4..=24).contains(&y))
        {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&map);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let left = observed
        .graph
        .edges
        .iter()
        .enumerate()
        .find(|(_, edge)| edge.points.iter().any(|p| p.x() == (PADDING + 16) as f64))
        .unwrap()
        .0;
    for unit in [8.0, 12.0] {
        // Both original short arms pass their independent forward-cap checks.
        // The source on the left actually bends and crosses the side boundary
        // of both inspection sizes; that crossing must not be silently clipped.
        for edge in observed
            .graph
            .edges
            .iter()
            .filter(|edge| edge.points.len() == 5)
        {
            let points: Vec<_> = if observed.graph.nodes[edge.start].kind == NodeKind::Junction {
                edge.points.clone()
            } else {
                edge.points.iter().rev().copied().collect()
            };
            assert_eq!(
                source::verify_spur(&capture, &palette, &points, unit)
                    .unwrap()
                    .decision,
                SourceDecision::Confirmed
            );
        }
        let result = clean(&capture, &palette, &map, &observed, unit)
            .unwrap()
            .result;
        assert!(result.retained_unconfirmed_edges.contains(&left));
        assert!(
            !result
                .removed_spurs
                .iter()
                .any(|spur| spur.source_edge == left)
        );
        assert_eq!(
            result
                .spur_assessments
                .iter()
                .find(|item| item.source_edge == left)
                .unwrap()
                .evidence
                .decision,
            SourceDecision::PossibleContinuation
        );
        assert_topology(&observed.graph, &result);
        assert_partition(&observed.graph, &result);
    }
    // Missing source at the continuation boundary cannot be excused by either
    // independently observed short path or by the already measured light cap.
    let mut valid = vec![true; map.ridges.len()];
    valid[(PADDING + 16) * map.width + PADDING + 17] = false;
    for hidden in [0, 255] {
        let mut pixels = capture.pixels().to_vec();
        pixels[(PADDING + 16) * map.width + PADDING + 17] = hidden;
        let masked = Capture::new(
            map.width as u32,
            map.height as u32,
            pixels,
            Some(valid.clone()),
            None,
        )
        .unwrap();
        let result = clean(&masked, &palette, &map, &observed, 8.0)
            .unwrap()
            .result;
        assert!(result.retained_unobserved_edges.contains(&left));
        let evidence = &result
            .spur_assessments
            .iter()
            .find(|item| item.source_edge == left)
            .unwrap()
            .evidence;
        assert_eq!(evidence.decision, SourceDecision::MissingSupport);
        assert!(evidence.light_barrier_distance_px.is_some());
    }
}

#[test]
fn a_rotated_guard_window_does_not_shorten_backbone_evidence_coverage() {
    let mut map = map(&["................................................."; 49]);
    for x in 20..=45 {
        put(&mut map, x, 24);
    }
    for (x, y) in [(19, 23), (19, 22), (20, 21), (20, 20), (20, 25), (20, 26)] {
        put(&mut map, x, y);
    }
    let capture = grayscale(&map, |x, y| {
        if (8..=48).contains(&x) && (20..=28).contains(&y) {
            30
        } else {
            220
        }
    });
    let (_, palette) = source_image(&map);
    let observed = graph::trace_with_pixels(&map).unwrap();
    let result = clean(&capture, &palette, &map, &observed, 8.0)
        .unwrap()
        .result;
    assert_eq!(result.spur_assessments.len(), 2);
    assert_eq!(result.removed_spurs.len(), 2);
    assert_eq!(result.graph.edges.len(), 1);
    assert!(
        result.graph.edges[0]
            .points
            .iter()
            .all(|p| p.y() == (PADDING + 24) as f64)
    );
    assert_topology(&observed.graph, &result);
    assert_partition(&observed.graph, &result);
}

#[test]
fn tolerance_changes_only_the_agreed_global_width_controls() {
    let policy = SourcePolicy::for_unit(10.0);
    assert_eq!(policy.relative_width_tolerance, 0.2);
    assert_eq!(policy.max_ridge_width_px, 12.0);
    assert_eq!(policy.terminal_lookahead_px, 12.0);
    assert_eq!(policy.loop_flank_search_px, 12.0);
    assert_eq!(policy.tangent_radius_px, 5.0);
    assert_eq!(policy.terminal_barrier_width_px, 10.0);
    assert_eq!(policy.terminal_barrier_depth_px, 2.5);
    assert_eq!(policy.max_backbone_extension_px, 20.0);
    assert_eq!(policy.dark_midpoint, 0.5);
    assert_eq!(policy.sample_spacing_px, 0.5);
    assert_eq!(policy.loop_min_valley_length_px, 1.0);
}
