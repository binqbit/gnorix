//! Conservative source-supported continuation of short ending-to-ending gaps.

mod bridge;

use std::collections::BTreeMap;

use super::{
    Capture, FingerprintError, LocalPalette, NodeKind, RidgeCycle, RidgeEdge, RidgeGraph, RidgeMap,
    raster,
};
use crate::biometrics::geometry::Point;
use bridge::{Endpoint, SegmentIndex, distance};

/// Processing thresholds expressed through the measured common ridge width.
/// These are explicit reconstruction rules, not calibrated biometric tolerances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GapPolicy {
    pub max_gap_px: f64,
    pub tangent_support_px: f64,
    pub max_tangent_error_radians: f64,
    pub max_curvature_per_px: f64,
    pub clearance_px: f64,
    pub flank_offset_px: f64,
    pub min_mean_darkness: f64,
    pub min_dark_fraction: f64,
    pub min_flank_contrast: f64,
    pub max_light_gap_px: f64,
}

impl GapPolicy {
    pub(super) fn for_unit(unit_px: f64) -> Result<Self, FingerprintError> {
        if !unit_px.is_finite() || unit_px <= 0.0 {
            return Err(FingerprintError::InvalidConfig(
                "gap repair needs a finite positive ridge width",
            ));
        }
        Ok(Self {
            max_gap_px: unit_px,
            tangent_support_px: unit_px,
            max_tangent_error_radians: std::f64::consts::PI / 6.0,
            max_curvature_per_px: 1.0 / unit_px,
            clearance_px: unit_px / 4.0,
            flank_offset_px: 0.75 * unit_px,
            min_mean_darkness: 0.5,
            min_dark_fraction: 0.7,
            min_flank_contrast: 0.25,
            max_light_gap_px: unit_px / 4.0,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapStatus {
    Accepted,
    ImageUnsupported,
    Unobserved,
    Ambiguous,
    Obstructed,
}

/// Original-image evidence over the complete sampled bridge, including its ends.
/// Darkness is `(local light - source)/(local light - local dark)`; values may
/// lie outside [0, 1]. Fractions and means weight represented bridge length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GapEvidence {
    pub mean_darkness: f64,
    pub dark_fraction: f64,
    pub mean_flank_contrast: f64,
    pub longest_light_gap_px: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GapJoin {
    /// Node indices in the observed graph, before continuation removes endings.
    pub start_node: usize,
    pub end_node: usize,
    pub points: Vec<Point>,
    /// Straight endpoint distance; bridge arc length can be longer.
    pub gap_px: f64,
    pub max_tangent_error_radians: f64,
    pub evidence: Option<GapEvidence>,
    pub status: GapStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathOrigin {
    pub source_edges: Vec<usize>,
    pub source_cycles: Vec<usize>,
    pub source_joins: Vec<usize>,
}

pub(super) struct GapRepair {
    pub graph: RidgeGraph,
    pub node_sources: Vec<usize>,
    pub edge_origins: Vec<PathOrigin>,
    pub cycle_origins: Vec<PathOrigin>,
    pub joins: Vec<GapJoin>,
    pub policy: GapPolicy,
}

pub(super) fn reconstruct(
    capture: &Capture,
    palette: &LocalPalette,
    graph: &RidgeGraph,
    skeleton: &RidgeMap,
    unit_px: f64,
) -> Result<GapRepair, FingerprintError> {
    let policy = GapPolicy::for_unit(unit_px)?;
    validate_support(capture, palette, skeleton)?;
    let incidence = incidence(graph)?;
    let endpoints = bridge::endpoints(graph, &incidence, policy);
    let obstacles = SegmentIndex::new(graph, unit_px)?;
    let mut joins = propose_joins(capture, palette, skeleton, &endpoints, &obstacles, policy)?;
    resolve_conflicts(&mut joins, graph.nodes.len(), policy)?;
    assemble(graph, &incidence, joins, policy)
}

fn validate_support(
    capture: &Capture,
    palette: &LocalPalette,
    skeleton: &RidgeMap,
) -> Result<(), FingerprintError> {
    let count = capture.pixels().len();
    if palette.ridge_gray.len() != count
        || palette.valley_gray.len() != count
        || palette.usable.len() != count
        || skeleton.width != capture.width() as usize
        || skeleton.height != capture.height() as usize
    {
        return Err(FingerprintError::InvalidMap(
            "repair support dimensions do not match source",
        ));
    }
    Ok(())
}

fn incidence(graph: &RidgeGraph) -> Result<Vec<Vec<usize>>, FingerprintError> {
    let mut result = vec![Vec::new(); graph.nodes.len()];
    for (index, edge) in graph.edges.iter().enumerate() {
        if edge.start >= result.len() || edge.end >= result.len() || edge.points.len() < 2 {
            return Err(FingerprintError::InvalidTopology(
                "repair received an invalid graph edge",
            ));
        }
        result[edge.start].push(index);
        result[edge.end].push(index);
    }
    Ok(result)
}

fn propose_joins(
    capture: &Capture,
    palette: &LocalPalette,
    skeleton: &RidgeMap,
    endpoints: &[Endpoint],
    obstacles: &SegmentIndex,
    policy: GapPolicy,
) -> Result<Vec<GapJoin>, FingerprintError> {
    let mut cells: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    let mut joins = Vec::new();
    let mut work = 0;
    for (index, endpoint) in endpoints.iter().enumerate() {
        let cell = bridge::cell(endpoint.position, policy.max_gap_px);
        for y in cell.1 - 1..=cell.1 + 1 {
            for x in cell.0 - 1..=cell.0 + 1 {
                for &other in cells.get(&(x, y)).into_iter().flatten() {
                    bridge::spend(&mut work, 1)?;
                    let first = &endpoints[other];
                    let Some((points, angle)) = bridge::continuation(first, endpoint, policy)?
                    else {
                        continue;
                    };
                    if points.iter().any(|point| {
                        point.x() < 0.0
                            || point.y() < 0.0
                            || point.x() > f64::from(capture.width() - 1)
                            || point.y() > f64::from(capture.height() - 1)
                    }) {
                        continue;
                    }
                    bridge::spend(&mut work, points.len() * 24)?;
                    let evidence = source_evidence(capture, palette, &points, policy);
                    let status = if obstacles
                        .obstructs(&points, first, endpoint, policy, &mut work)?
                        || bridge::raw_obstructs(
                            skeleton, &points, first, endpoint, policy, &mut work,
                        )? {
                        GapStatus::Obstructed
                    } else if evidence.is_none() {
                        GapStatus::Unobserved
                    } else if supported(evidence.as_ref().unwrap(), policy) {
                        GapStatus::Accepted
                    } else {
                        GapStatus::ImageUnsupported
                    };
                    joins.push(GapJoin {
                        start_node: first.node,
                        end_node: endpoint.node,
                        gap_px: distance(first.position, endpoint.position),
                        max_tangent_error_radians: angle,
                        points,
                        evidence,
                        status,
                    });
                }
            }
        }
        cells.entry(cell).or_default().push(index);
    }
    joins.sort_by_key(|join| (join.start_node, join.end_node));
    Ok(joins)
}

fn supported(evidence: &GapEvidence, policy: GapPolicy) -> bool {
    evidence.mean_darkness >= policy.min_mean_darkness
        && evidence.dark_fraction >= policy.min_dark_fraction
        && evidence.mean_flank_contrast >= policy.min_flank_contrast
        && evidence.longest_light_gap_px <= policy.max_light_gap_px
}

fn source_evidence(
    capture: &Capture,
    palette: &LocalPalette,
    points: &[Point],
    policy: GapPolicy,
) -> Option<GapEvidence> {
    let (mut length, mut darkness, mut dark_length, mut contrast) = (0.0, 0.0, 0.0, 0.0);
    let (mut light_run, mut longest_light_gap_px) = (0.0_f64, 0.0_f64);
    for segment in points.windows(2) {
        let start = segment[0];
        let end = segment[1];
        let span = distance(start, end);
        if span <= 0.0 {
            return None;
        }
        segment_observed(capture, palette, start, end)?;
        let middle = Point::new((start.x() + end.x()) / 2.0, (start.y() + end.y()) / 2.0).ok()?;
        let center = sample_darkness(capture, palette, middle)?;
        let nx = -(end.y() - start.y()) / span * policy.flank_offset_px;
        let ny = (end.x() - start.x()) / span * policy.flank_offset_px;
        let left = Point::new(middle.x() + nx, middle.y() + ny).ok()?;
        let right = Point::new(middle.x() - nx, middle.y() - ny).ok()?;
        for sign in [-1.0, 1.0] {
            segment_observed(
                capture,
                palette,
                Point::new(start.x() + sign * nx, start.y() + sign * ny).ok()?,
                Point::new(end.x() + sign * nx, end.y() + sign * ny).ok()?,
            )?;
        }
        let flank = (sample_darkness(capture, palette, left)?
            + sample_darkness(capture, palette, right)?)
            / 2.0;
        length += span;
        darkness += center * span;
        contrast += (center - flank) * span;
        if center >= policy.min_mean_darkness {
            dark_length += span;
            light_run = 0.0;
        } else {
            light_run += span;
            longest_light_gap_px = longest_light_gap_px.max(light_run);
        }
    }
    (length > 0.0).then_some(GapEvidence {
        mean_darkness: darkness / length,
        dark_fraction: dark_length / length,
        mean_flank_contrast: contrast / length,
        longest_light_gap_px,
    })
}

fn segment_observed(
    capture: &Capture,
    palette: &LocalPalette,
    start: Point,
    end: Point,
) -> Option<()> {
    sample_darkness(capture, palette, start)?;
    sample_darkness(capture, palette, end)?;
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    raster::visit_cells(start, end, width, height, |cell| {
        let (right, bottom) = ((cell.x + 1).min(width - 1), (cell.y + 1).min(height - 1));
        let corners = [
            cell.y * width + cell.x,
            cell.y * width + right,
            bottom * width + cell.x,
            bottom * width + right,
        ];
        corners
            .iter()
            .all(|&index| {
                let dark = palette.ridge_gray[index];
                let light = palette.valley_gray[index];
                palette.usable[index]
                    && capture.valid_mask().is_none_or(|mask| mask[index])
                    && dark.is_finite()
                    && light.is_finite()
                    && (0.0..=255.0).contains(&dark)
                    && (0.0..=255.0).contains(&light)
                    && light > dark
            })
            .then_some(())
    })
}

fn sample_darkness(capture: &Capture, palette: &LocalPalette, point: Point) -> Option<f64> {
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    if point.x() < 0.0
        || point.y() < 0.0
        || point.x() > (width - 1) as f64
        || point.y() > (height - 1) as f64
    {
        return None;
    }
    let (x, y) = (point.x().floor() as usize, point.y().floor() as usize);
    let (tx, ty) = (point.x() - x as f64, point.y() - y as f64);
    let corners = [
        y * width + x,
        y * width + (x + 1).min(width - 1),
        (y + 1).min(height - 1) * width + x,
        (y + 1).min(height - 1) * width + (x + 1).min(width - 1),
    ];
    if corners
        .iter()
        .any(|&i| !palette.usable[i] || capture.valid_mask().is_some_and(|mask| !mask[i]))
    {
        return None;
    }
    let interpolate = |values: [f64; 4]| {
        (values[0] * (1.0 - tx) + values[1] * tx) * (1.0 - ty)
            + (values[2] * (1.0 - tx) + values[3] * tx) * ty
    };
    let dark = interpolate(corners.map(|i| f64::from(palette.ridge_gray[i])));
    let light = interpolate(corners.map(|i| f64::from(palette.valley_gray[i])));
    if !dark.is_finite()
        || !light.is_finite()
        || !(0.0..=255.0).contains(&dark)
        || !(0.0..=255.0).contains(&light)
        || light <= dark
    {
        return None;
    }
    let gray = interpolate(corners.map(|i| f64::from(capture.pixels()[i])));
    Some((light - gray) / (light - dark))
}

fn resolve_conflicts(
    joins: &mut [GapJoin],
    nodes: usize,
    policy: GapPolicy,
) -> Result<(), FingerprintError> {
    let mut counts = vec![0; nodes];
    for join in joins
        .iter()
        .filter(|join| join.status == GapStatus::Accepted)
    {
        counts[join.start_node] += 1;
        counts[join.end_node] += 1;
    }
    for join in joins.iter_mut() {
        if join.status == GapStatus::Accepted
            && (counts[join.start_node] != 1 || counts[join.end_node] != 1)
        {
            join.status = GapStatus::Ambiguous;
        }
    }
    let candidates: Vec<_> = joins
        .iter()
        .enumerate()
        .filter_map(|(index, join)| (join.status == GapStatus::Accepted).then_some(index))
        .collect();
    let mut blocked = vec![false; joins.len()];
    let mut work = 0;
    for (position, &a) in candidates.iter().enumerate() {
        for &b in &candidates[position + 1..] {
            bridge::spend(&mut work, 1)?;
            if bridge::paths_conflict(
                &joins[a].points,
                &joins[b].points,
                policy.clearance_px,
                &mut work,
            )? {
                blocked[a] = true;
                blocked[b] = true;
            }
        }
    }
    for (index, join) in joins.iter_mut().enumerate() {
        if blocked[index] {
            join.status = GapStatus::Obstructed;
        }
    }
    Ok(())
}

fn assemble(
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

#[cfg(test)]
mod tests;
