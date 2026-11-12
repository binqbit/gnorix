//! Assess original candidates once; never reclassify newly exposed branches.

use std::collections::{BTreeMap, BTreeSet};

use super::super::photometry;
use super::{
    Capture, FingerprintError, LocalPalette, LoopAssessment, LoopEvidence, NO_SOURCE, NodeKind,
    Point, RidgeGraph, RidgeMap, SourceDecision, SpurAssessment, SpurEvidence, SpurRemoval, Work,
    continuation, distance, graph, pixel, source,
};

#[derive(Default)]
pub(super) struct Selection {
    pub removed_spurs: Vec<SpurRemoval>,
    pub spur_assessments: Vec<SpurAssessment>,
    pub loops: Vec<LoopAssessment>,
    pub ambiguous: Vec<usize>,
    pub unobserved: Vec<usize>,
    pub unconfirmed: Vec<usize>,
}

impl Selection {
    pub(super) fn removed_edges(&self, count: usize) -> Vec<bool> {
        let mut removed = vec![false; count];
        for spur in &self.removed_spurs {
            removed[spur.source_edge] = true;
        }
        for edge in self.loops.iter().filter_map(|item| item.removed_edge) {
            removed[edge] = true;
        }
        removed
    }
}

pub(super) fn assess(
    capture: &Capture,
    palette: &LocalPalette,
    skeleton: &RidgeMap,
    observed: &graph::TracedGraph,
    unit: f64,
) -> Result<Selection, FingerprintError> {
    let inputs = Inputs {
        capture,
        palette,
        skeleton,
        observed,
        unit,
        incidence: incidence(&observed.graph),
    };
    let mut work = Work::default();
    let loops = assess_loops(&inputs, &mut work)?;
    let maximum = source::SourcePolicy::for_unit(unit).max_ridge_width_px;
    let mut excluded = vec![false; observed.graph.edges.len()];
    for (id, edge) in observed.graph.edges.iter().enumerate() {
        let kinds = (
            observed.graph.nodes[edge.start].kind,
            observed.graph.nodes[edge.end].kind,
        );
        let terminal = matches!(
            kinds,
            (NodeKind::Junction, NodeKind::Ending) | (NodeKind::Ending, NodeKind::Junction)
        );
        let length: f64 = edge
            .points
            .windows(2)
            .map(|pair| distance(pair[0], pair[1]))
            .sum();
        excluded[id] = terminal && length < maximum;
    }
    for edge in loops.iter().filter_map(|item| item.removed_edge) {
        excluded[edge] = true;
    }
    let mut result = assess_spurs(&inputs, &excluded, &mut work)?;
    result.loops = loops;
    Ok(result)
}

struct Inputs<'a> {
    capture: &'a Capture,
    palette: &'a LocalPalette,
    skeleton: &'a RidgeMap,
    observed: &'a graph::TracedGraph,
    unit: f64,
    incidence: Vec<Vec<usize>>,
}

fn assess_spurs(
    input: &Inputs<'_>,
    excluded: &[bool],
    work: &mut Work,
) -> Result<Selection, FingerprintError> {
    let (capture, palette, skeleton, observed, incidence, unit) = (
        input.capture,
        input.palette,
        input.skeleton,
        input.observed,
        &input.incidence,
        input.unit,
    );
    let graph = &observed.graph;
    let maximum = source::SourcePolicy::for_unit(unit).max_ridge_width_px;
    let components = components(graph, incidence);
    let mut total = vec![0; graph.nodes.len()];
    let mut eligible = vec![0; graph.nodes.len()];
    let mut candidates = Vec::new();
    let mut pending = Vec::new();
    let mut result = Selection::default();
    for (id, edge) in graph.edges.iter().enumerate() {
        total[components[edge.start]] += 1;
        let (attachment, terminal) =
            match (graph.nodes[edge.start].kind, graph.nodes[edge.end].kind) {
                (NodeKind::Junction, NodeKind::Ending) => (edge.start, edge.end),
                (NodeKind::Ending, NodeKind::Junction) => (edge.end, edge.start),
                _ => continue,
            };
        if incidence[terminal].len() != 1 {
            continue;
        }
        let length: f64 = edge
            .points
            .windows(2)
            .map(|pair| distance(pair[0], pair[1]))
            .sum();
        if length >= maximum {
            continue;
        }
        charge_source(work, &[&edge.points], unit)?;
        let points = oriented(&edge.points, edge.start != attachment);
        let evidence = if clusters_observed(
            capture,
            palette,
            skeleton,
            observed,
            &[attachment, terminal],
        ) {
            source::verify_spur(capture, palette, &points, unit)?
        } else {
            SpurEvidence::missing_support()
        };
        pending.push((
            SpurRemoval {
                source_edge: id,
                attachment_node: attachment,
                terminal_node: terminal,
                arc_length_px: length,
            },
            points,
            evidence,
        ));
    }
    let confirmed: Vec<_> = pending
        .iter()
        .filter_map(|(candidate, _, evidence)| {
            (evidence.decision == SourceDecision::Confirmed).then_some(candidate.source_edge)
        })
        .collect();
    let backbone = continuation::Backbone::new(graph, excluded, &components, &confirmed);
    for (candidate, points, mut evidence) in pending {
        if evidence.decision == SourceDecision::Confirmed {
            evidence.decision = continuation::check(
                capture,
                palette,
                &points,
                unit,
                &backbone,
                components[candidate.attachment_node],
                work,
            )?;
        }
        let id = candidate.source_edge;
        match evidence.decision {
            SourceDecision::Confirmed => {
                eligible[components[candidate.attachment_node]] += 1;
                candidates.push(candidate);
            }
            SourceDecision::MissingSupport => result.unobserved.push(id),
            _ => result.unconfirmed.push(id),
        }
        result.spur_assessments.push(SpurAssessment {
            source_edge: id,
            evidence,
        });
    }
    for candidate in candidates {
        let component = components[candidate.attachment_node];
        if eligible[component] == total[component] {
            result.ambiguous.push(candidate.source_edge);
        } else {
            result.removed_spurs.push(candidate);
        }
    }
    Ok(result)
}

fn assess_loops(
    input: &Inputs<'_>,
    work: &mut Work,
) -> Result<Vec<LoopAssessment>, FingerprintError> {
    let (capture, palette, skeleton, observed, incidence, unit) = (
        input.capture,
        input.palette,
        input.skeleton,
        input.observed,
        &input.incidence,
        input.unit,
    );
    let mut pairs: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (id, edge) in observed.graph.edges.iter().enumerate() {
        if edge.start == edge.end
            || incidence[edge.start].len() != 3
            || incidence[edge.end].len() != 3
        {
            continue;
        }
        if observed.graph.nodes[edge.start].kind != NodeKind::Junction
            || observed.graph.nodes[edge.end].kind != NodeKind::Junction
        {
            continue;
        }
        pairs
            .entry((edge.start.min(edge.end), edge.start.max(edge.end)))
            .or_default()
            .push(id);
    }
    let mut assessments = Vec::new();
    for ((start, end), edges) in pairs {
        if edges.len() != 2 {
            continue;
        }
        let first = &observed.graph.edges[edges[0]];
        let second = &observed.graph.edges[edges[1]];
        if !disjoint_interiors(skeleton, &first.points, &second.points) {
            continue;
        }
        charge_source(work, &[&first.points, &second.points], unit)?;
        let a = oriented(&first.points, first.start != start);
        let b = oriented(&second.points, second.start != start);
        let mut evidence = if clusters_observed(capture, palette, skeleton, observed, &[start, end])
        {
            source::verify_loop(capture, palette, &a, &b, unit)?
        } else {
            LoopEvidence::missing_support()
        };
        let removed_edge = removable_arm(&observed.graph, [edges[0], edges[1]], &mut evidence);
        assessments.push(LoopAssessment {
            source_edges: [edges[0], edges[1]],
            source_nodes: [start, end],
            evidence,
            removed_edge,
        });
    }
    Ok(assessments)
}

pub(super) fn removable_arm(
    graph: &RidgeGraph,
    edges: [usize; 2],
    evidence: &mut LoopEvidence,
) -> Option<usize> {
    let discarded = match (evidence.decision, evidence.preferred_arm) {
        (SourceDecision::Confirmed, Some(0)) => edges[1],
        (SourceDecision::Confirmed, Some(1)) => edges[0],
        _ => return None,
    };
    // A direct port-to-port link has no removable interior source pixel. The
    // retained-pixel representation must not claim to remove a link it still has.
    if graph.edges[discarded].points.len() <= 2 {
        evidence.decision = SourceDecision::AmbiguousGeometry;
        evidence.preferred_arm = None;
        return None;
    }
    Some(discarded)
}

pub(super) fn charge_source(
    work: &mut Work,
    paths: &[&[Point]],
    unit: f64,
) -> Result<(), FingerprintError> {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut stations = 0.0;
    for points in paths {
        let length: f64 = points
            .windows(2)
            .map(|pair| distance(pair[0], pair[1]))
            .sum();
        stations += points.len() as f64 + (2.0 * length).ceil() + 3.0;
        for point in *points {
            min[0] = min[0].min(point.x());
            min[1] = min[1].min(point.y());
            max[0] = max[0].max(point.x());
            max[1] = max[1].max(point.y());
        }
    }
    let policy = source::SourcePolicy::for_unit(unit);
    // Full support envelopes plus conservative sampling bounds. The same
    // support rectangle covers the short arm's forward cap and loop normals.
    let margin = policy.max_ridge_width_px + 2.0 * policy.loop_flank_search_px;
    let envelope =
        (max[0] - min[0] + margin + 4.0).ceil() * (max[1] - min[1] + margin + 4.0).ceil();
    let rows = (policy.terminal_lookahead_px / policy.sample_spacing_px).ceil() + 2.0;
    // Interior + outward scans and the resulting complete center profile each
    // cover at most `margin`, with endpoints and arm samples counted separately.
    let samples = stations.max(rows) * (2.0 * margin / policy.sample_spacing_px + 12.0).ceil();
    work.charge(envelope + samples)
}

fn clusters_observed(
    capture: &Capture,
    palette: &LocalPalette,
    skeleton: &RidgeMap,
    observed: &graph::TracedGraph,
    nodes: &[usize],
) -> bool {
    nodes.iter().all(|&node| {
        observed.node_pixels[node].iter().all(|&index| {
            let point = Point::new(
                (index % skeleton.width) as f64,
                (index / skeleton.width) as f64,
            )
            .unwrap();
            photometry::sample_darkness(capture, palette, point).is_some()
        })
    })
}

fn disjoint_interiors(skeleton: &RidgeMap, first: &[Point], second: &[Point]) -> bool {
    let inside: BTreeSet<_> = first[1..first.len() - 1]
        .iter()
        .map(|&point| pixel(skeleton, point))
        .collect();
    second[1..second.len() - 1]
        .iter()
        .all(|&point| !inside.contains(&pixel(skeleton, point)))
}

fn oriented(points: &[Point], reverse: bool) -> Vec<Point> {
    if reverse {
        points.iter().rev().copied().collect()
    } else {
        points.to_vec()
    }
}

fn incidence(graph: &RidgeGraph) -> Vec<Vec<usize>> {
    let mut result = vec![Vec::new(); graph.nodes.len()];
    for (id, edge) in graph.edges.iter().enumerate() {
        result[edge.start].push(id);
        result[edge.end].push(id);
    }
    result
}

pub(super) fn components(source: &RidgeGraph, incidence: &[Vec<usize>]) -> Vec<usize> {
    let mut labels = vec![NO_SOURCE; source.nodes.len()];
    for seed in 0..labels.len() {
        if labels[seed] != NO_SOURCE {
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
                if labels[other] == NO_SOURCE {
                    labels[other] = seed;
                    queue.push(other);
                }
            }
            cursor += 1;
        }
    }
    labels
}
