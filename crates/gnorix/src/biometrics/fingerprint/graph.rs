//! Deterministic graph extraction from usable centerline pixels.
//!
//! Diagonal neighbors connect only when neither orthogonal corner pixel is a
//! ridge. A fully occupied 2x2 cell has no background hole, so its top horizontal
//! link is redundant and omitted. This deterministic orientation preserves all
//! pixels and connectivity without counting filled cells as graph cycles.
//! Acyclic clusters of adjacent junction pixels form one node; cyclic clusters
//! retain individual nodes so their loops remain in the graph. Incident paths
//! retain actual pixel ports instead of artificial segments.

use super::{FingerprintError, NodeKind, RidgeCycle, RidgeEdge, RidgeGraph, RidgeMap, RidgeNode};
use crate::biometrics::geometry::Point;

const NO_NODE: usize = usize::MAX;
const OFFSETS: [(isize, isize); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

pub(super) fn trace(map: &RidgeMap) -> Result<RidgeGraph, FingerprintError> {
    validate_map(map)?;
    let degrees = pixel_degrees(map);
    let (nodes, node_pixels, ownership) = collect_nodes(map, &degrees)?;
    let mut visited = vec![0_u8; map.ridges.len()];
    let edges = trace_edges(map, &node_pixels, &ownership, &mut visited)?;
    let cycles = trace_cycles(map, &ownership, &mut visited)?;

    Ok(RidgeGraph {
        nodes,
        edges,
        cycles,
    })
}

fn validate_map(map: &RidgeMap) -> Result<(), FingerprintError> {
    if map.width == 0
        || map.height == 0
        || map.width.checked_mul(map.height) != Some(map.ridges.len())
        || map.usable.len() != map.ridges.len()
    {
        return Err(FingerprintError::InvalidMap(
            "inconsistent ridge-map dimensions",
        ));
    }
    Ok(())
}

fn pixel_degrees(map: &RidgeMap) -> Vec<u8> {
    (0..map.ridges.len())
        .map(|pixel| {
            if is_ridge(map, pixel) {
                neighbors(map, pixel).iter().flatten().count() as u8
            } else {
                0
            }
        })
        .collect()
}

type CollectedNodes = (Vec<RidgeNode>, Vec<Vec<usize>>, Vec<usize>);

fn collect_nodes(map: &RidgeMap, degrees: &[u8]) -> Result<CollectedNodes, FingerprintError> {
    let mut nodes = Vec::new();
    let mut node_pixels = Vec::new();
    let mut ownership = vec![NO_NODE; map.ridges.len()];

    for pixel in 0..map.ridges.len() {
        if !is_ridge(map, pixel) || degrees[pixel] == 2 || ownership[pixel] != NO_NODE {
            continue;
        }
        let node_index = nodes.len();
        let mut pixels = grow_node(map, degrees, &mut ownership, pixel, node_index);
        if is_cyclic_junction_cluster(map, degrees, &pixels) {
            // Contracting a cyclic cluster would silently erase its holes.
            // Retain its actual vertices and links; no connecting geometry is
            // inferred between the representative positions of other nodes.
            pixels.sort_unstable();
            for pixel in pixels {
                ownership[pixel] = nodes.len();
                nodes.push(make_node(map, degrees[pixel], &[pixel])?);
                node_pixels.push(vec![pixel]);
            }
        } else {
            nodes.push(make_node(map, degrees[pixel], &pixels)?);
            node_pixels.push(pixels);
        }
    }
    Ok((nodes, node_pixels, ownership))
}

fn make_node(map: &RidgeMap, degree: u8, pixels: &[usize]) -> Result<RidgeNode, FingerprintError> {
    Ok(RidgeNode {
        position: point(map, pixels[0])?,
        kind: match degree {
            0 => NodeKind::Isolated,
            1 => NodeKind::Ending,
            _ => NodeKind::Junction,
        },
        touches_unobserved: pixels.iter().any(|&pixel| touches_unknown(map, pixel)),
    })
}

fn is_cyclic_junction_cluster(map: &RidgeMap, degrees: &[u8], pixels: &[usize]) -> bool {
    if degrees[pixels[0]] < 3 {
        return false;
    }
    let mut links = 0;
    for &pixel in pixels {
        for next in neighbors(map, pixel).into_iter().flatten() {
            if next > pixel && degrees[next] >= 3 {
                links += 1;
                // grow_node returns a connected cluster. It contains a cycle
                // precisely when it has at least as many links as vertices.
                if links >= pixels.len() {
                    return true;
                }
            }
        }
    }
    false
}

fn grow_node(
    map: &RidgeMap,
    degrees: &[u8],
    ownership: &mut [usize],
    seed: usize,
    node: usize,
) -> Vec<usize> {
    let mut pixels = vec![seed];
    ownership[seed] = node;
    if degrees[seed] < 3 {
        return pixels;
    }

    // A queue cursor avoids recursion and visits every junction pixel once.
    let mut cursor = 0;
    while cursor < pixels.len() {
        for neighbor in neighbors(map, pixels[cursor]).into_iter().flatten() {
            if degrees[neighbor] >= 3 && ownership[neighbor] == NO_NODE {
                ownership[neighbor] = node;
                pixels.push(neighbor);
            }
        }
        cursor += 1;
    }
    pixels
}

fn trace_edges(
    map: &RidgeMap,
    node_pixels: &[Vec<usize>],
    ownership: &[usize],
    visited: &mut [u8],
) -> Result<Vec<RidgeEdge>, FingerprintError> {
    let mut edges = Vec::new();
    for (start, pixels) in node_pixels.iter().enumerate() {
        for &pixel in pixels {
            for (direction, neighbor) in neighbors(map, pixel).into_iter().enumerate() {
                let Some(neighbor) = neighbor else { continue };
                if ownership[neighbor] == start || is_visited(visited, pixel, direction) {
                    continue;
                }
                let (end, points) = walk_edge(map, ownership, visited, pixel, direction, neighbor)?;
                edges.push(RidgeEdge { start, end, points });
            }
        }
    }
    Ok(edges)
}

fn walk_edge(
    map: &RidgeMap,
    ownership: &[usize],
    visited: &mut [u8],
    start: usize,
    direction: usize,
    first: usize,
) -> Result<(usize, Vec<Point>), FingerprintError> {
    let mut points = vec![point(map, start)?, point(map, first)?];
    mark_step(visited, start, direction, first);
    let (mut previous, mut current) = (start, first);

    while ownership[current] == NO_NODE {
        let (direction, next) = continuation(map, current, previous)?;
        if is_visited(visited, current, direction) {
            return Err(FingerprintError::InvalidTopology(
                "edge encountered an already traced step",
            ));
        }
        mark_step(visited, current, direction, next);
        points.push(point(map, next)?);
        previous = current;
        current = next;
    }
    Ok((ownership[current], points))
}

fn trace_cycles(
    map: &RidgeMap,
    ownership: &[usize],
    visited: &mut [u8],
) -> Result<Vec<RidgeCycle>, FingerprintError> {
    let mut cycles = Vec::new();
    for seed in 0..map.ridges.len() {
        if !is_ridge(map, seed) || ownership[seed] != NO_NODE || visited[seed] != 0 {
            continue;
        }
        cycles.push(RidgeCycle {
            points: walk_cycle(map, ownership, visited, seed)?,
        });
    }
    Ok(cycles)
}

fn walk_cycle(
    map: &RidgeMap,
    ownership: &[usize],
    visited: &mut [u8],
    seed: usize,
) -> Result<Vec<Point>, FingerprintError> {
    let mut points = vec![point(map, seed)?];
    let (direction, first) = neighbors(map, seed)
        .into_iter()
        .enumerate()
        .find_map(|(direction, pixel)| pixel.map(|pixel| (direction, pixel)))
        .ok_or(FingerprintError::InvalidTopology(
            "cycle has no adjacent pixel",
        ))?;
    mark_step(visited, seed, direction, first);
    let (mut previous, mut current) = (seed, first);

    while current != seed {
        if ownership[current] != NO_NODE {
            return Err(FingerprintError::InvalidTopology(
                "cycle encountered a graph node",
            ));
        }
        points.push(point(map, current)?);
        let (direction, next) = continuation(map, current, previous)?;
        if is_visited(visited, current, direction) {
            return Err(FingerprintError::InvalidTopology(
                "cycle encountered an already traced step",
            ));
        }
        mark_step(visited, current, direction, next);
        previous = current;
        current = next;
    }
    // RidgeCycle is implicitly closed; do not duplicate the first vertex.
    Ok(points)
}

fn continuation(
    map: &RidgeMap,
    current: usize,
    previous: usize,
) -> Result<(usize, usize), FingerprintError> {
    let adjacent = neighbors(map, current);
    let mut onward = adjacent
        .into_iter()
        .enumerate()
        .filter_map(|(direction, pixel)| {
            pixel
                .filter(|&pixel| pixel != previous)
                .map(|pixel| (direction, pixel))
        });
    let next = onward.next().ok_or(FingerprintError::InvalidTopology(
        "path terminated without a graph node",
    ))?;
    if onward.next().is_some() {
        return Err(FingerprintError::InvalidTopology(
            "path branched outside a graph node",
        ));
    }
    Ok(next)
}

pub(super) fn neighbors(map: &RidgeMap, pixel: usize) -> [Option<usize>; 8] {
    let (x, y) = (pixel % map.width, pixel / map.width);
    OFFSETS.map(|(dx, dy)| {
        let (nx, ny) = (x.checked_add_signed(dx)?, y.checked_add_signed(dy)?);
        if nx >= map.width || ny >= map.height {
            return None;
        }
        let neighbor = ny * map.width + nx;
        if !is_ridge(map, neighbor) {
            return None;
        }
        if dx != 0
            && dy != 0
            && (is_ridge(map, y * map.width + nx) || is_ridge(map, ny * map.width + x))
        {
            return None;
        }
        if dy == 0
            && y + 1 < map.height
            && is_ridge(map, (y + 1) * map.width + x)
            && is_ridge(map, (y + 1) * map.width + nx)
        {
            // A filled pixel cell is not a hole. Remove only its top link;
            // both vertical sides remain. If the bottom link is also removed,
            // following those sides downward reaches a retained bottom link.
            return None;
        }
        Some(neighbor)
    })
}

fn is_ridge(map: &RidgeMap, pixel: usize) -> bool {
    map.usable[pixel] && map.ridges[pixel]
}

fn touches_unknown(map: &RidgeMap, pixel: usize) -> bool {
    if map.boundary_origin[pixel] {
        return true;
    }
    let (x, y) = (pixel % map.width, pixel / map.width);
    OFFSETS.iter().any(|&(dx, dy)| {
        let (Some(nx), Some(ny)) = (x.checked_add_signed(dx), y.checked_add_signed(dy)) else {
            return true;
        };
        nx >= map.width || ny >= map.height || !map.usable[ny * map.width + nx]
    })
}

fn is_visited(visited: &[u8], pixel: usize, direction: usize) -> bool {
    visited[pixel] & (1 << direction) != 0
}

fn mark_step(visited: &mut [u8], from: usize, direction: usize, to: usize) {
    visited[from] |= 1 << direction;
    visited[to] |= 1 << (7 - direction);
}

fn point(map: &RidgeMap, pixel: usize) -> Result<Point, FingerprintError> {
    Point::new((pixel % map.width) as f64, (pixel / map.width) as f64)
        .map_err(|_| FingerprintError::InvalidTopology("nonfinite graph point"))
}

#[cfg(test)]
mod tests {
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
                || (e.end == junction
                    && *e.points.last().unwrap() != graph.nodes[junction].position)
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
}
