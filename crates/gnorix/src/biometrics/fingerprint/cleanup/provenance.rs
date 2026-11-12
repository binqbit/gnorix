//! Links from cleaned paths back to unchanged observed paths and junction pixels.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    CleanupOrigin, FingerprintError, JunctionRoute, NO_SOURCE, Point, RidgeCleanup, RidgeGraph,
    RidgeMap, pixel, segments,
};

pub(super) fn node_ownership(skeleton: &RidgeMap, groups: &[Vec<usize>]) -> Vec<usize> {
    let mut ownership = vec![NO_SOURCE; skeleton.ridges.len()];
    for (node, pixels) in groups.iter().enumerate() {
        for &pixel in pixels {
            ownership[pixel] = node;
        }
    }
    ownership
}

pub(super) fn map_nodes(
    groups: &[Vec<usize>],
    ownership: &[usize],
) -> Result<Vec<usize>, FingerprintError> {
    groups
        .iter()
        .map(|pixels| {
            let source = ownership[pixels[0]];
            if source == NO_SOURCE || pixels.iter().any(|&pixel| ownership[pixel] != source) {
                return Err(FingerprintError::InvalidTopology(
                    "cleaned node lost its source cluster",
                ));
            }
            Ok(source)
        })
        .collect()
}

#[derive(Clone, Copy)]
enum LinkOrigin {
    Edge(usize),
    Cycle(usize),
}
type Link = (usize, usize);

pub(super) fn retain_origins(
    skeleton: &RidgeMap,
    source: &RidgeGraph,
    ownership: &[usize],
    result: &mut RidgeCleanup,
) -> Result<(), FingerprintError> {
    let mut links = BTreeMap::new();
    for (id, edge) in source.edges.iter().enumerate() {
        for pair in edge.points.windows(2) {
            links.insert(link(skeleton, pair[0], pair[1]), LinkOrigin::Edge(id));
        }
    }
    for (id, cycle) in source.cycles.iter().enumerate() {
        for (a, b) in segments(&cycle.points, true) {
            links.insert(link(skeleton, a, b), LinkOrigin::Cycle(id));
        }
    }
    for edge in &result.graph.edges {
        result.edge_origins.push(path_origin(
            skeleton,
            &edge.points,
            false,
            &links,
            ownership,
            &mut result.junction_routes,
        )?);
    }
    for cycle in &result.graph.cycles {
        result.cycle_origins.push(path_origin(
            skeleton,
            &cycle.points,
            true,
            &links,
            ownership,
            &mut result.junction_routes,
        )?);
    }
    let mut edge_counts = vec![0; source.edges.len()];
    let mut cycle_counts = vec![0; source.cycles.len()];
    for origin in result.edge_origins.iter().chain(&result.cycle_origins) {
        for &id in &origin.source_edges {
            edge_counts[id] += 1;
        }
        for &id in &origin.source_cycles {
            cycle_counts[id] += 1;
        }
    }
    for removal in &result.removed_spurs {
        edge_counts[removal.source_edge] += 1;
    }
    for id in result.loops.iter().filter_map(|item| item.removed_edge) {
        edge_counts[id] += 1;
    }
    if edge_counts
        .iter()
        .chain(&cycle_counts)
        .any(|&count| count != 1)
    {
        return Err(FingerprintError::InvalidTopology(
            "cleanup did not partition original paths",
        ));
    }
    Ok(())
}

fn path_origin(
    skeleton: &RidgeMap,
    points: &[Point],
    closed: bool,
    links: &BTreeMap<Link, LinkOrigin>,
    ownership: &[usize],
    routes: &mut Vec<JunctionRoute>,
) -> Result<CleanupOrigin, FingerprintError> {
    let mut result = CleanupOrigin::default();
    let (mut edges, mut cycles) = (BTreeSet::new(), BTreeSet::new());
    let mut active_route: Option<usize> = None;
    for (a, b) in segments(points, closed) {
        match links.get(&link(skeleton, a, b)) {
            Some(LinkOrigin::Edge(id)) => {
                edges.insert(*id);
                active_route = None;
            }
            Some(LinkOrigin::Cycle(id)) => {
                cycles.insert(*id);
                active_route = None;
            }
            None => {
                let node = ownership[pixel(skeleton, a)];
                if node == NO_SOURCE || ownership[pixel(skeleton, b)] != node {
                    return Err(FingerprintError::InvalidTopology(
                        "cleanup introduced an unobserved connector",
                    ));
                }
                if let Some(id) = active_route.filter(|&id| routes[id].source_node == node) {
                    routes[id].points.push(b);
                } else {
                    let id = routes.len();
                    routes.push(JunctionRoute {
                        source_node: node,
                        points: vec![a, b],
                    });
                    result.source_routes.push(id);
                    active_route = Some(id);
                }
            }
        }
    }
    result.source_edges = edges.into_iter().collect();
    result.source_cycles = cycles.into_iter().collect();
    Ok(result)
}

pub(super) fn point_groups(
    skeleton: &RidgeMap,
    groups: &[Vec<usize>],
) -> Result<Vec<Vec<Point>>, FingerprintError> {
    groups
        .iter()
        .map(|pixels| {
            pixels
                .iter()
                .map(|&index| {
                    Point::new(
                        (index % skeleton.width) as f64,
                        (index / skeleton.width) as f64,
                    )
                    .map_err(|_| FingerprintError::InvalidTopology("invalid source pixel"))
                })
                .collect()
        })
        .collect()
}

fn link(skeleton: &RidgeMap, a: Point, b: Point) -> Link {
    let (a, b) = (pixel(skeleton, a), pixel(skeleton, b));
    (a.min(b), a.max(b))
}
