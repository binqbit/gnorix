use super::*;

fn map(rows: &[&str]) -> RidgeMap {
    let width = rows[0].len();
    assert!(rows.iter().all(|row| row.len() == width));
    RidgeMap::new(
        width,
        rows.len(),
        rows.iter()
            .flat_map(|row| row.bytes().map(|b| b == b'#'))
            .collect(),
        rows.iter()
            .flat_map(|row| row.bytes().map(|b| b != b'?'))
            .collect(),
    )
    .unwrap()
}

fn coordinates(points: &[Point]) -> Vec<(u32, u32)> {
    points
        .iter()
        .map(|p| (p.x() as u32, p.y() as u32))
        .collect()
}

#[test]
fn empty_and_isolated_components_are_preserved() {
    let empty = trace(&map(&["...", "...", "..."])).unwrap();
    assert!(empty.nodes.is_empty() && empty.edges.is_empty() && empty.cycles.is_empty());
    let isolated = trace(&map(&[".....", ".#.#.", "....."])).unwrap();
    assert_eq!(isolated.nodes.len(), 2);
    assert!(
        isolated
            .nodes
            .iter()
            .all(|n| n.kind == NodeKind::Isolated && !n.touches_unobserved)
    );
    assert!(isolated.edges.is_empty() && isolated.cycles.is_empty());
}

#[test]
fn orthogonal_corner_does_not_create_a_diagonal_shortcut() {
    let graph = trace(&map(&["....", ".##.", "..#.", "...."])).unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert!(graph.cycles.is_empty());
    assert_eq!(
        coordinates(&graph.edges[0].points),
        [(1, 1), (2, 1), (2, 2)]
    );
}

#[test]
fn diagonal_ridges_remain_connected() {
    let graph = trace(&map(&[".....", ".#...", "..#..", "...#.", "....."])).unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(
        coordinates(&graph.edges[0].points),
        [(1, 1), (2, 2), (3, 3)]
    );
}

#[test]
fn adjacent_junction_pixels_become_one_node_with_real_ports() {
    let graph = trace(&map(&[
        ".......", "..#....", ".####..", "...#...", ".......",
    ]))
    .unwrap();
    assert_eq!(graph.nodes.len(), 5);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Junction)
            .count(),
        1
    );
    assert_eq!(graph.edges.len(), 4);
    assert!(graph.edges.iter().all(|e| e.points.len() == 2));
    assert!(graph.cycles.is_empty());
    let junction = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Junction)
        .unwrap();
    assert_eq!(coordinates(&[graph.nodes[junction].position]), [(2, 2)]);
    assert!(graph.edges.iter().any(|e| {
        (e.start == junction && e.points[0] != graph.nodes[junction].position)
            || (e.end == junction && *e.points.last().unwrap() != graph.nodes[junction].position)
    }));
}

#[test]
fn degree_two_component_is_an_implicit_closed_cycle() {
    let graph = trace(&map(&[".....", ".###.", ".#.#.", ".###.", "....."])).unwrap();
    assert!(graph.nodes.is_empty() && graph.edges.is_empty());
    assert_eq!(graph.cycles.len(), 1);
    assert_eq!(
        coordinates(&graph.cycles[0].points),
        [
            (1, 1),
            (2, 1),
            (3, 1),
            (3, 2),
            (3, 3),
            (2, 3),
            (1, 3),
            (1, 2)
        ]
    );
}

#[test]
fn loop_attached_to_a_junction_is_a_self_edge() {
    let graph = trace(&map(&[
        ".......", ".#####.", ".#.#...", ".###...", ".......",
    ]))
    .unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 2);
    assert_eq!(graph.edges.iter().filter(|e| e.start == e.end).count(), 1);
    assert!(graph.cycles.is_empty());
}

#[test]
fn a_cycle_made_entirely_of_junction_pixels_is_not_contracted() {
    let input = map(&[
        ".......", "...#...", "...#.#.", "..#.#..", ".#.#...", "....#..", ".......",
    ]);
    let skeleton = crate::biometrics::fingerprint::thinning::thin(&input).unwrap();
    assert_eq!(skeleton.ridges, input.ridges);
    assert_eq!(background_holes(&skeleton), 1);

    let graph = trace(&skeleton).unwrap();
    assert_eq!(graph.nodes.len(), 8);
    assert_eq!(graph.edges.len(), 8);
    assert_eq!(graph_components(&graph), 1);
    assert_eq!(graph_cycle_rank(&graph), 1);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Junction)
            .count(),
        4
    );
    assert!(graph.edges.iter().all(|edge| edge.points.len() == 2));
    assert_eq!(trace(&skeleton).unwrap(), graph);
}

#[test]
fn a_filled_two_by_two_junction_is_not_a_false_cycle() {
    let input = map(&["......", ".#..#.", "..##..", "..##..", ".#..#.", "......"]);
    let skeleton = crate::biometrics::fingerprint::thinning::thin(&input).unwrap();
    assert_eq!(skeleton.ridges, input.ridges);
    assert_eq!(background_holes(&skeleton), 0);

    let graph = trace(&skeleton).unwrap();
    assert_eq!(graph_components(&graph), 1);
    assert_eq!(graph_cycle_rank(&graph), 0);
    assert_eq!(graph.nodes.len(), 5);
    assert_eq!(graph.edges.len(), 4);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Ending)
            .count(),
        4
    );
}

#[test]
fn a_filled_junction_and_a_real_hole_keep_only_the_real_cycle() {
    let input = map(&[
        "........", "....#...", "..#.#...", "...##...", "...###..", "...#..#.", "....#.#.",
        ".....#..",
    ]);
    let skeleton = crate::biometrics::fingerprint::thinning::thin(&input).unwrap();
    assert_eq!(skeleton.ridges, input.ridges);
    assert_eq!(background_holes(&skeleton), 1);

    let graph = trace(&skeleton).unwrap();
    assert_eq!(graph_components(&graph), 1);
    assert_eq!(graph_cycle_rank(&graph), 1);
}

#[test]
fn borders_and_missing_pixels_mark_uncertain_endings() {
    let border = trace(&map(&[".....", "###..", "....."])).unwrap();
    assert!(border.nodes[0].touches_unobserved);
    assert!(!border.nodes[1].touches_unobserved);
    let missing = trace(&map(&[".....", ".###?", "....."])).unwrap();
    assert!(!missing.nodes[0].touches_unobserved);
    assert!(missing.nodes[1].touches_unobserved);
    assert_eq!(missing.edges.len(), 1);
}

#[test]
fn missing_regions_are_not_bridged() {
    let graph = trace(&map(&[".........", ".##?##...", "........."])).unwrap();
    assert_eq!(graph.nodes.len(), 4);
    assert_eq!(graph.edges.len(), 2);
    assert_eq!(
        graph.nodes.iter().filter(|n| n.touches_unobserved).count(),
        2
    );
    assert!(graph.edges.iter().all(|edge| edge.points.len() == 2));
}

#[test]
fn crossing_retains_all_four_branches() {
    let graph = trace(&map(&[".....", "..#..", ".###.", "..#..", "....."])).unwrap();
    assert_eq!(graph.edges.len(), 4);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Ending)
            .count(),
        4
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Junction)
            .count(),
        1
    );
}

#[test]
fn long_paths_do_not_require_recursion() {
    let width = 20_002;
    let mut ridges = vec![false; width * 3];
    ridges[width + 1..2 * width - 1].fill(true);
    let input = RidgeMap::new(width, 3, ridges, vec![true; width * 3]).unwrap();
    let graph = trace(&input).unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].points.len(), 20_000);
    assert_eq!(coordinates(&[graph.edges[0].points[19_999]]), [(20_000, 1)]);
}

#[test]
fn every_three_by_three_pattern_preserves_connected_components() {
    // Suppressing a redundant diagonal must not change eight-connected
    // components. This independently checks all small junction/cycle cases.
    for pattern in 0..512 {
        let ridges: Vec<bool> = (0..9).map(|bit| pattern & (1 << bit) != 0).collect();
        let expected = eight_connected_components(&ridges);
        let input = RidgeMap::new(3, 3, ridges, vec![true; 9]).unwrap();
        let graph = trace(&input).unwrap();
        assert_eq!(graph_components(&graph), expected, "pattern {pattern:09b}");
    }
}

#[test]
fn every_thinned_three_by_three_pattern_preserves_cycle_rank() {
    for pattern in 0..512 {
        let ridges = (0..9).map(|bit| pattern & (1 << bit) != 0).collect();
        let input = RidgeMap::new(3, 3, ridges, vec![true; 9]).unwrap();
        let skeleton = crate::biometrics::fingerprint::thinning::thin(&input).unwrap();
        let graph = trace(&skeleton).unwrap();
        assert_eq!(
            graph_cycle_rank(&graph),
            background_holes(&skeleton),
            "pattern {pattern:09b}",
        );
    }
}

fn graph_components(graph: &RidgeGraph) -> usize {
    let mut seen = vec![false; graph.nodes.len()];
    let mut count = graph.cycles.len();
    for node in 0..graph.nodes.len() {
        if seen[node] {
            continue;
        }
        count += 1;
        let mut pending = vec![node];
        while let Some(current) = pending.pop() {
            if seen[current] {
                continue;
            }
            seen[current] = true;
            for edge in &graph.edges {
                if edge.start == current && !seen[edge.end] {
                    pending.push(edge.end);
                }
                if edge.end == current && !seen[edge.start] {
                    pending.push(edge.start);
                }
            }
        }
    }
    count
}

fn graph_cycle_rank(graph: &RidgeGraph) -> usize {
    // Each separately represented degree-two cycle adds one component and
    // one independent cycle. Within nodes/edges, use the Euler cycle rank.
    graph.edges.len() + graph_components(graph) - graph.nodes.len()
}

fn background_holes(map: &RidgeMap) -> usize {
    let (width, height) = (map.width + 2, map.height + 2);
    let mut background = vec![true; width * height];
    for y in 0..map.height {
        for x in 0..map.width {
            background[(y + 1) * width + x + 1] = !map.ridges[y * map.width + x];
        }
    }
    let mut count = 0;
    for seed in 0..background.len() {
        if !background[seed] {
            continue;
        }
        count += 1;
        background[seed] = false;
        let mut pending = vec![seed];
        while let Some(pixel) = pending.pop() {
            let (x, y) = (pixel % width, pixel / width);
            for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let (Some(nx), Some(ny)) = (x.checked_add_signed(dx), y.checked_add_signed(dy))
                else {
                    continue;
                };
                if nx < width && ny < height && background[ny * width + nx] {
                    background[ny * width + nx] = false;
                    pending.push(ny * width + nx);
                }
            }
        }
    }
    count - 1 // Exclude the connected padded exterior.
}

fn eight_connected_components(ridges: &[bool]) -> usize {
    let mut seen = [false; 9];
    let mut count = 0;
    for seed in 0..9 {
        if !ridges[seed] || seen[seed] {
            continue;
        }
        count += 1;
        let mut stack = vec![seed];
        while let Some(current) = stack.pop() {
            if seen[current] {
                continue;
            }
            seen[current] = true;
            for next in 0..9 {
                let dx = (current % 3).abs_diff(next % 3);
                let dy = (current / 3).abs_diff(next / 3);
                if ridges[next] && !seen[next] && dx <= 1 && dy <= 1 {
                    stack.push(next);
                }
            }
        }
    }
    count
}

#[test]
fn tracing_is_deterministic_and_retains_disconnected_components() {
    let input = map(&[
        "...........",
        ".###...#...",
        ".#.#....#..",
        ".###.....#.",
        "...........",
        ".#.........",
        "...........",
    ]);
    let first = trace(&input).unwrap();
    let second = trace(&input).unwrap();
    assert_eq!(first.nodes.len(), 3);
    assert_eq!(first.edges.len(), 1);
    assert_eq!(first.cycles.len(), 1);
    assert_eq!(first, second);
}
