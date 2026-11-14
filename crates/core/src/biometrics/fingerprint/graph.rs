//! Deterministic graph extraction from usable centerline pixels.
//!
//! Diagonal neighbors connect only when neither orthogonal corner pixel is a
//! ridge. A fully occupied 2x2 cell has no background hole, so its top horizontal
//! link is redundant and omitted. This deterministic orientation preserves all
//! pixels and connectivity without counting filled cells as graph cycles.
//! Acyclic clusters of adjacent junction pixels form one node; cyclic clusters
//! retain individual nodes so their loops remain in the graph. Incident paths
//! retain actual pixel ports instead of artificial segments.

use super::{FingerprintError, RidgeMap};
use crate::biometrics::geometry::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Ending,
    Junction,
    Isolated,
}

/// A representative pixel of an extracted graph node, not an anatomical claim.
/// Acyclic junction clusters use an actual pixel; incident edge ports may differ.
/// Cyclic clusters retain individual vertices to preserve their genuine holes.
#[derive(Clone, Debug, PartialEq)]
pub struct RidgeNode {
    pub position: Point,
    pub kind: NodeKind,
    /// The node touches, or inherited thinning support from, the frame or an
    /// unusable region. Continuation is unknown; false is not proof of anatomy.
    pub touches_unobserved: bool,
}

/// An observed path between node clusters. No anchor-to-port line is invented.
#[derive(Clone, Debug, PartialEq)]
pub struct RidgeEdge {
    pub start: usize,
    pub end: usize,
    pub points: Vec<Point>,
}

/// An isolated closed path without a privileged biological starting point.
#[derive(Clone, Debug, PartialEq)]
pub struct RidgeCycle {
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RidgeGraph {
    pub nodes: Vec<RidgeNode>,
    pub edges: Vec<RidgeEdge>,
    pub cycles: Vec<RidgeCycle>,
}

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

pub(super) struct TracedGraph {
    pub graph: RidgeGraph,
    pub node_pixels: Vec<Vec<usize>>,
}

pub(super) fn trace(map: &RidgeMap) -> Result<RidgeGraph, FingerprintError> {
    Ok(trace_with_pixels(map)?.graph)
}

pub(super) fn trace_with_pixels(map: &RidgeMap) -> Result<TracedGraph, FingerprintError> {
    trace_view(TraceView {
        source: map,
        retained: None,
        source_nodes: None,
    })
}

/// Trace a subset without inventing diagonals when removed pixels change corner
/// occupancy. All retained links are links of the original source skeleton.
pub(super) fn trace_subset(
    map: &RidgeMap,
    retained: &[bool],
    source_nodes: &[usize],
) -> Result<TracedGraph, FingerprintError> {
    if retained.len() != map.ridges.len()
        || source_nodes.len() != map.ridges.len()
        || retained
            .iter()
            .zip(&map.ridges)
            .any(|(&keep, &ridge)| keep && !ridge)
    {
        return Err(FingerprintError::InvalidMap(
            "retained skeleton is not a source subset",
        ));
    }
    trace_view(TraceView {
        source: map,
        retained: Some(retained),
        source_nodes: Some(source_nodes),
    })
}

struct TraceView<'a> {
    source: &'a RidgeMap,
    retained: Option<&'a [bool]>,
    source_nodes: Option<&'a [usize]>,
}

impl std::ops::Deref for TraceView<'_> {
    type Target = RidgeMap;
    fn deref(&self) -> &Self::Target {
        self.source
    }
}

impl TraceView<'_> {
    fn contains(&self, pixel: usize) -> bool {
        is_ridge(self.source, pixel) && self.retained.is_none_or(|kept| kept[pixel])
    }

    fn neighbors(&self, pixel: usize) -> [Option<usize>; 8] {
        neighbors(self.source, pixel).map(|next| next.filter(|&next| self.contains(next)))
    }

    fn cluster_neighbors(&self, pixel: usize) -> [Option<usize>; 8] {
        self.neighbors(pixel).map(|next| {
            next.filter(|&next| {
                self.source_nodes
                    .is_none_or(|sources| sources[pixel] == sources[next])
            })
        })
    }
}

fn trace_view(map: TraceView<'_>) -> Result<TracedGraph, FingerprintError> {
    validate_map(&map)?;
    let degrees = pixel_degrees(&map);
    let (nodes, node_pixels, ownership) = collect_nodes(&map, &degrees)?;
    let mut visited = vec![0_u8; map.ridges.len()];
    let edges = trace_edges(&map, &node_pixels, &ownership, &mut visited)?;
    let cycles = trace_cycles(&map, &ownership, &mut visited)?;

    Ok(TracedGraph {
        graph: RidgeGraph {
            nodes,
            edges,
            cycles,
        },
        node_pixels,
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

fn pixel_degrees(map: &TraceView<'_>) -> Vec<u8> {
    (0..map.ridges.len())
        .map(|pixel| {
            if map.contains(pixel) {
                map.neighbors(pixel).iter().flatten().count() as u8
            } else {
                0
            }
        })
        .collect()
}

type CollectedNodes = (Vec<RidgeNode>, Vec<Vec<usize>>, Vec<usize>);

fn collect_nodes(map: &TraceView<'_>, degrees: &[u8]) -> Result<CollectedNodes, FingerprintError> {
    let mut nodes = Vec::new();
    let mut node_pixels = Vec::new();
    let mut ownership = vec![NO_NODE; map.ridges.len()];

    for pixel in 0..map.ridges.len() {
        if !map.contains(pixel) || degrees[pixel] == 2 || ownership[pixel] != NO_NODE {
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

fn make_node(
    map: &TraceView<'_>,
    degree: u8,
    pixels: &[usize],
) -> Result<RidgeNode, FingerprintError> {
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

fn is_cyclic_junction_cluster(map: &TraceView<'_>, degrees: &[u8], pixels: &[usize]) -> bool {
    if degrees[pixels[0]] < 3 {
        return false;
    }
    let mut links = 0;
    for &pixel in pixels {
        for next in map.cluster_neighbors(pixel).into_iter().flatten() {
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
    map: &TraceView<'_>,
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
        for neighbor in map.cluster_neighbors(pixels[cursor]).into_iter().flatten() {
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
    map: &TraceView<'_>,
    node_pixels: &[Vec<usize>],
    ownership: &[usize],
    visited: &mut [u8],
) -> Result<Vec<RidgeEdge>, FingerprintError> {
    let mut edges = Vec::new();
    for (start, pixels) in node_pixels.iter().enumerate() {
        for &pixel in pixels {
            for (direction, neighbor) in map.neighbors(pixel).into_iter().enumerate() {
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
    map: &TraceView<'_>,
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
    map: &TraceView<'_>,
    ownership: &[usize],
    visited: &mut [u8],
) -> Result<Vec<RidgeCycle>, FingerprintError> {
    let mut cycles = Vec::new();
    for seed in 0..map.ridges.len() {
        if !map.contains(seed) || ownership[seed] != NO_NODE || visited[seed] != 0 {
            continue;
        }
        cycles.push(RidgeCycle {
            points: walk_cycle(map, ownership, visited, seed)?,
        });
    }
    Ok(cycles)
}

fn walk_cycle(
    map: &TraceView<'_>,
    ownership: &[usize],
    visited: &mut [u8],
    seed: usize,
) -> Result<Vec<Point>, FingerprintError> {
    let mut points = vec![point(map, seed)?];
    let (direction, first) = map
        .neighbors(seed)
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
    map: &TraceView<'_>,
    current: usize,
    previous: usize,
) -> Result<(usize, usize), FingerprintError> {
    let adjacent = map.neighbors(current);
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

pub(super) fn touches_unknown(map: &RidgeMap, pixel: usize) -> bool {
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

/// Incident edge IDs for each node; a self-edge contributes twice.
pub(super) fn incidence(graph: &RidgeGraph) -> Vec<Vec<usize>> {
    let mut result = vec![Vec::new(); graph.nodes.len()];
    for (id, edge) in graph.edges.iter().enumerate() {
        result[edge.start].push(id);
        result[edge.end].push(id);
    }
    result
}

pub(super) fn components(source: &RidgeGraph, incidence: &[Vec<usize>]) -> Vec<usize> {
    let mut labels = vec![NO_NODE; source.nodes.len()];
    for seed in 0..labels.len() {
        if labels[seed] != NO_NODE {
            continue;
        }
        labels[seed] = seed;
        let mut queue = vec![seed];
        let mut cursor = 0;
        while cursor < queue.len() {
            let node = queue[cursor];
            for &id in &incidence[node] {
                let edge = &source.edges[id];
                let other = if edge.start == node {
                    edge.end
                } else {
                    edge.start
                };
                if labels[other] == NO_NODE {
                    labels[other] = seed;
                    queue.push(other);
                }
            }
            cursor += 1;
        }
    }
    labels
}

#[cfg(test)]
mod tests;
