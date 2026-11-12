//! Assemble accepted continuations while retaining source-path provenance.

use super::super::{RidgeCycle, RidgeEdge};
use super::{
    FingerprintError, GapJoin, GapPolicy, GapRepair, GapStatus, PathOrigin, Point, RidgeGraph,
};

pub(super) fn assemble(
    graph: &RidgeGraph,
    incidence: &[Vec<usize>],
    joins: Vec<GapJoin>,
    policy: GapPolicy,
) -> Result<GapRepair, FingerprintError> {
    let mut partners = vec![None; graph.nodes.len()];
    for (index, join) in joins
        .iter()
        .enumerate()
        .filter(|(_, join)| join.status == GapStatus::Accepted)
    {
        partners[join.start_node] = Some((join.end_node, index));
        partners[join.end_node] = Some((join.start_node, index));
    }
    let node_sources: Vec<_> = (0..graph.nodes.len())
        .filter(|&node| partners[node].is_none())
        .collect();
    let mut remap = vec![usize::MAX; graph.nodes.len()];
    for (new, &old) in node_sources.iter().enumerate() {
        remap[old] = new;
    }
    let mut result = GapRepair {
        graph: RidgeGraph {
            nodes: node_sources
                .iter()
                .map(|&i| graph.nodes[i].clone())
                .collect(),
            edges: Vec::new(),
            cycles: graph.cycles.clone(),
        },
        node_sources,
        edge_origins: Vec::new(),
        cycle_origins: (0..graph.cycles.len())
            .map(|i| PathOrigin {
                source_cycles: vec![i],
                ..PathOrigin::default()
            })
            .collect(),
        joins,
        policy,
    };
    let mut visited = vec![false; graph.edges.len()];
    for &start in &result.node_sources {
        for &edge in &incidence[start] {
            if visited[edge] {
                continue;
            }
            let (end, points, origin) = walk_chain(
                graph,
                incidence,
                &partners,
                &result.joins,
                &mut visited,
                start,
                edge,
            )?;
            result.graph.edges.push(RidgeEdge {
                start: remap[start],
                end: remap[end],
                points,
            });
            result.edge_origins.push(origin);
        }
    }
    for edge in 0..graph.edges.len() {
        if visited[edge] {
            continue;
        }
        let start = graph.edges[edge].start;
        let (end, mut points, origin) = walk_chain(
            graph,
            incidence,
            &partners,
            &result.joins,
            &mut visited,
            start,
            edge,
        )?;
        if end != start {
            return Err(FingerprintError::InvalidTopology(
                "continued cycle did not close",
            ));
        }
        if points.first() == points.last() {
            points.pop();
        }
        result.graph.cycles.push(RidgeCycle { points });
        result.cycle_origins.push(origin);
    }
    Ok(result)
}

fn walk_chain(
    graph: &RidgeGraph,
    incidence: &[Vec<usize>],
    partners: &[Option<(usize, usize)>],
    joins: &[GapJoin],
    visited: &mut [bool],
    start: usize,
    mut edge_index: usize,
) -> Result<(usize, Vec<Point>, PathOrigin), FingerprintError> {
    let mut current = start;
    let mut points = Vec::new();
    let mut origin = PathOrigin::default();
    loop {
        if visited[edge_index] {
            return Err(FingerprintError::InvalidTopology(
                "continued path reuses an edge",
            ));
        }
        visited[edge_index] = true;
        let edge = &graph.edges[edge_index];
        let reverse = edge.end == current && edge.start != current;
        append(&mut points, &edge.points, reverse);
        origin.source_edges.push(edge_index);
        current = if reverse { edge.start } else { edge.end };
        let Some((other, join_index)) = partners[current] else {
            break;
        };
        let join = &joins[join_index];
        append(&mut points, &join.points, join.end_node == current);
        origin.source_joins.push(join_index);
        current = other;
        if current == start {
            break;
        }
        edge_index = *incidence[current]
            .first()
            .ok_or(FingerprintError::InvalidTopology(
                "continued ending has no incident edge",
            ))?;
    }
    Ok((current, points, origin))
}

fn append(target: &mut Vec<Point>, source: &[Point], reverse: bool) {
    for index in 0..source.len() {
        let point = source[if reverse {
            source.len() - 1 - index
        } else {
            index
        }];
        if target.last() != Some(&point) {
            target.push(point);
        }
    }
}
