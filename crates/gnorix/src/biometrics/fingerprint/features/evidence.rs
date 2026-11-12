//! Feature-specific evidence from the unchanged original capture.

use super::super::{LocalPalette, SourceDecision, cleanup};
use super::geometry::{self, ArmPath};
use super::sampling::{Sampler, Work, intervals};
use super::{
    Capture, FeatureArm, FeatureKind, FeaturePolicy, FeatureReason, FeatureStrength,
    FingerprintError, Point, RidgeGraph,
};

struct Context<'a> {
    capture: &'a Capture,
    palette: &'a LocalPalette,
    graph: &'a RidgeGraph,
    policy: &'a FeaturePolicy,
    arms: &'a [Vec<ArmPath>],
    components: &'a [usize],
}

pub(super) fn assess(
    capture: &Capture,
    palette: &LocalPalette,
    graph: &RidgeGraph,
    policy: &FeaturePolicy,
) -> Result<Vec<FeatureStrength>, FingerprintError> {
    let count = capture.pixels().len();
    if palette.usable.len() != count
        || palette.ridge_gray.len() != count
        || palette.valley_gray.len() != count
    {
        return Err(FingerprintError::InvalidMap(
            "feature source dimensions differ",
        ));
    }
    if !policy.reference_width_px.is_finite() || policy.reference_width_px <= 0.0 {
        return Err(FingerprintError::InvalidConfig(
            "feature evidence requires a finite positive width",
        ));
    }
    let mut work = Work::default();
    let arms = geometry::arms(graph, policy, &mut work)?;
    let components = components(graph);
    let context = Context {
        capture,
        palette,
        graph,
        policy,
        arms: &arms,
        components: &components,
    };
    let mut guard_work = cleanup::Work::default();
    let mut result = Vec::with_capacity(graph.nodes.len() + graph.cycles.len());
    for id in 0..graph.nodes.len() {
        result.push(node(&context, id, &mut work, &mut guard_work)?);
    }
    for id in 0..graph.cycles.len() {
        result.push(cycle(&context, id, &mut work)?);
    }
    Ok(result)
}

fn node(
    context: &Context<'_>,
    id: usize,
    work: &mut Work,
    guard_work: &mut cleanup::Work,
) -> Result<FeatureStrength, FingerprintError> {
    let (graph, policy) = (context.graph, context.policy);
    let paths = &context.arms[id];
    let node = &graph.nodes[id];
    let mut feature = empty(
        id,
        super::kind(node.kind, paths.len()),
        Some(id),
        None,
        node.position,
    );
    feature.geometry_coverage = geometry::extent(paths, policy.branch_horizon_px);
    let mut sampler = Sampler::new(context.capture, context.palette, work);
    let (mut source, mut contrast) = measure_arms(&mut feature, paths, &mut sampler, policy)?;
    if !paths.is_empty() && feature.geometry_coverage < 1.0 {
        reason(&mut feature, FeatureReason::InsufficientArmExtent);
    }
    match feature.kind {
        FeatureKind::Ending => {
            source = minimum(
                source,
                ending(context, id, &mut feature, &mut sampler, guard_work)?,
            )
        }
        FeatureKind::Bifurcation | FeatureKind::Junction => {
            feature.geometry_separation = geometry::separation(paths, policy, sampler.work)?;
            feature.geometry_support = feature
                .geometry_separation
                .map(|value| value * feature.geometry_coverage);
            feature.separation = separation(&mut sampler, paths, policy)?;
            if feature.separation == Some(0.0) {
                reason(&mut feature, FeatureReason::NoSeparatedBranches);
            }
            source = minimum(source, feature.separation);
        }
        FeatureKind::Isolated => {
            let sample = sampler.sample(node.position)?;
            contrast = sample.as_ref().map(|sample| sample.contrast);
            source = sample.map(|sample| sample.dark);
            feature.geometry_support = Some(0.0);
            reason(&mut feature, FeatureReason::IsolatedGeometry);
            if source == Some(0.0) {
                reason(&mut feature, FeatureReason::LightRidge);
            }
        }
        FeatureKind::Cycle => unreachable!("cycles are assessed separately"),
    }
    feature.source_coverage = sampler.coverage();
    feature.source_support = if sampler.complete() {
        attenuate(source, contrast, policy)
    } else {
        None
    };
    feature.source_contrast = contrast;
    if !sampler.complete() {
        reason(&mut feature, FeatureReason::MissingSourceSupport);
    }
    Ok(feature)
}

fn measure_arms(
    feature: &mut FeatureStrength,
    paths: &[ArmPath],
    sampler: &mut Sampler<'_>,
    policy: &FeaturePolicy,
) -> Result<(Option<f64>, Option<f64>), FingerprintError> {
    let (mut source, mut contrast) = (Some(1.0_f64), Some(1.0_f64));
    for arm in paths {
        let image = sampler.path(&arm.points, false, policy.sample_spacing_px)?;
        source = minimum(source, image.support(arm.length));
        contrast = minimum(contrast, image.contrast);
        if image.dark_fraction.is_some_and(|fraction| fraction < 1.0) {
            reason(feature, FeatureReason::LightRidge);
        }
        feature.arms.push(FeatureArm {
            edge: arm.edge,
            length_px: arm.length,
            source_darkness: image.darkness,
            dark_fraction: image.dark_fraction,
            source_contrast: image.contrast,
            longest_light_gap_px: image.longest_light.map(|gap| gap.min(arm.length)),
        });
    }
    Ok((source, contrast))
}

fn ending(
    context: &Context<'_>,
    id: usize,
    feature: &mut FeatureStrength,
    sampler: &mut Sampler<'_>,
    guard_work: &mut cleanup::Work,
) -> Result<Option<f64>, FingerprintError> {
    let (graph, policy) = (context.graph, context.policy);
    let paths = &context.arms[id];
    if paths.len() != 1 {
        return Err(FingerprintError::InvalidTopology(
            "ending must have one incident arm",
        ));
    }
    let arm = &paths[0];
    let Some(direction) = arm.direction else {
        reason(feature, FeatureReason::UnresolvedDirection);
        return Ok(None);
    };
    sampler.work.charge(graph.nodes.len())?;
    sampler.work.charge(graph.edges.len())?;
    sampler.work.charge(graph.cycles.len())?;
    for edge in &graph.edges {
        sampler.work.charge(edge.points.len())?;
    }
    for cycle in &graph.cycles {
        sampler.work.charge(cycle.points.len())?;
    }
    let backbone = ending_backbone(graph, id, arm.edge, policy.branch_horizon_px);
    let continuation = geometry::continuation(id, context.arms, &backbone, policy, sampler.work)?;
    feature.continuation = continuation;
    feature.geometry_support = continuation.map(|value| feature.geometry_coverage * (1.0 - value));
    if continuation.is_some_and(|value| value > 0.0) {
        reason(feature, FeatureReason::PossibleContinuation);
    }
    if !super::sampling::forward_observed(
        context.capture,
        arm.points[0],
        direction,
        policy,
        sampler.work,
    )? {
        feature.geometry_support = None;
        reason(feature, FeatureReason::MissingSourceSupport);
    }
    let points: Vec<_> = arm.points.iter().copied().rev().collect();
    let cap = cleanup::feature_ending(
        context.capture,
        context.palette,
        &points,
        &backbone,
        context.components,
        context.components[id],
        policy.reference_width_px,
        guard_work,
    )?;
    sampler.external_support(cap.decision != SourceDecision::MissingSupport)?;
    let termination = match cap.decision {
        SourceDecision::MissingSupport => None,
        SourceDecision::AmbiguousGeometry => {
            reason(feature, FeatureReason::UnresolvedDirection);
            None
        }
        SourceDecision::Confirmed => cap
            .min_barrier_lightness
            .map(|light| (2.0 * light - 1.0).clamp(0.0, 1.0)),
        _ => Some(0.0),
    };
    if cap.decision != SourceDecision::Confirmed {
        reason(feature, FeatureReason::UnconfirmedTermination);
    }
    if cap.decision == SourceDecision::PossibleContinuation {
        reason(feature, FeatureReason::PossibleContinuation);
    }
    feature.termination = termination;
    Ok(termination)
}

fn cycle(
    context: &Context<'_>,
    cycle_id: usize,
    work: &mut Work,
) -> Result<FeatureStrength, FingerprintError> {
    let (graph, policy) = (context.graph, context.policy);
    let cycle = &graph.cycles[cycle_id];
    let center = geometry::centroid(&cycle.points)?;
    let length = geometry::arc_length(&cycle.points, true);
    let mut feature = empty(
        graph.nodes.len() + cycle_id,
        FeatureKind::Cycle,
        None,
        Some(cycle_id),
        center,
    );
    let mut sampler = Sampler::new(context.capture, context.palette, work);
    let ring = sampler.path(&cycle.points, true, policy.sample_spacing_px)?;
    let interior = interior_light(&mut sampler, &cycle.points, policy)?;
    feature.source_support = attenuate(
        minimum(ring.support(length), interior),
        ring.contrast,
        policy,
    );
    feature.source_contrast = ring.contrast;
    feature.separation = interior;
    feature.geometry_coverage = (length / policy.branch_horizon_px).min(1.0);
    feature.geometry_support = Some(feature.geometry_coverage);
    feature.source_coverage = sampler.coverage();
    if !sampler.complete() {
        feature.source_support = None;
        reason(&mut feature, FeatureReason::MissingSourceSupport);
    }
    if ring.dark_fraction.is_some_and(|fraction| fraction < 1.0) {
        reason(&mut feature, FeatureReason::LightRidge);
    }
    if interior.is_some_and(|value| value <= 0.5) {
        reason(&mut feature, FeatureReason::NoInteriorLight);
    }
    reason(&mut feature, FeatureReason::IsolatedGeometry);
    Ok(feature)
}

/// Mean light margin across each pair's transverse connector, integrated over
/// the outer R of both branches. A single bright pixel contributes only its
/// represented width and length. The strongest separated pair is useful for a
/// fork; the weakest arm still limits both overall source and geometry support.
fn separation(
    sampler: &mut Sampler<'_>,
    arms: &[ArmPath],
    policy: &FeaturePolicy,
) -> Result<Option<f64>, FingerprintError> {
    let mut strongest = 0.0_f64;
    let mut complete = true;
    for (i, a) in arms.iter().enumerate() {
        for b in &arms[i + 1..] {
            let upper = a.length.min(b.length).min(policy.branch_horizon_px);
            if upper <= policy.reference_width_px {
                continue;
            }
            let span = upper - policy.reference_width_px;
            let count = intervals(span, policy.sample_spacing_px)?;
            sampler
                .work
                .charge(count.saturating_mul(a.points.len() + b.points.len()))?;
            let mut sum = 0.0;
            for index in 0..count {
                let along = policy.reference_width_px + span * (index as f64 + 0.5) / count as f64;
                let first = geometry::at(&a.points, along);
                let second = geometry::at(&b.points, along);
                complete &= sampler.observed_segment(first, second)?;
                let endpoints = (sampler.sample(first)?, sampler.sample(second)?);
                let width = geometry::distance(first, second);
                if width == 0.0 {
                    continue;
                }
                let transverse = intervals(width, policy.sample_spacing_px)?;
                let mut light = 0.0;
                for column in 0..transverse {
                    let point =
                        geometry::lerp(first, second, (column as f64 + 0.5) / transverse as f64);
                    if let Some(sample) = sampler.sample(point)? {
                        light += (1.0 - 2.0 * sample.dark).max(0.0);
                    } else {
                        complete = false;
                    }
                }
                if let (Some(a), Some(b)) = endpoints {
                    let dark_ends = (2.0 * a.dark - 1.0)
                        .max(0.0)
                        .min((2.0 * b.dark - 1.0).max(0.0));
                    sum += dark_ends * light / transverse as f64;
                } else {
                    complete = false;
                }
            }
            strongest = strongest.max(sum / count as f64 * span / policy.reference_width_px);
        }
    }
    Ok(complete.then_some(strongest.clamp(0.0, 1.0)))
}

fn interior_light(
    sampler: &mut Sampler<'_>,
    points: &[Point],
    policy: &FeaturePolicy,
) -> Result<Option<f64>, FingerprintError> {
    let (mut left, mut top, mut right, mut bottom) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for point in points {
        left = left.min(point.x());
        top = top.min(point.y());
        right = right.max(point.x());
        bottom = bottom.max(point.y());
    }
    let columns = intervals(right - left, policy.sample_spacing_px)?;
    let rows = intervals(bottom - top, policy.sample_spacing_px)?;
    sampler.work.charge(
        columns
            .checked_mul(rows)
            .and_then(|n| n.checked_mul(points.len()))
            .ok_or(FingerprintError::WorkLimit)?,
    )?;
    let (mut total, mut sum, mut complete) = (0_usize, 0.0, true);
    for row in 0..rows {
        for column in 0..columns {
            let p = Point::new(
                left + (right - left) * (column as f64 + 0.5) / columns as f64,
                top + (bottom - top) * (row as f64 + 0.5) / rows as f64,
            )
            .unwrap();
            if !inside(points, p) {
                continue;
            }
            total += 1;
            if let Some(sample) = sampler.sample(p)? {
                sum += 1.0 - sample.dark;
            } else {
                complete = false;
            }
        }
    }
    if total == 0 {
        return Ok(Some(0.0));
    }
    Ok(complete.then_some(sum / total as f64))
}

fn inside(points: &[Point], p: Point) -> bool {
    let mut inside = false;
    for (a, b) in geometry::segments(points, true) {
        if (a.y() > p.y()) != (b.y() > p.y())
            && p.x() < (b.x() - a.x()) * (p.y() - a.y()) / (b.y() - a.y()) + a.x()
        {
            inside = !inside;
        }
    }
    inside
}

fn ending_backbone(graph: &RidgeGraph, node: usize, edge: usize, horizon: f64) -> RidgeGraph {
    let mut backbone = graph.clone();
    let original = &graph.edges[edge];
    let remaining = geometry::arc_length(&original.points, false) - horizon;
    backbone.edges[edge].points = if remaining <= 0.0 {
        Vec::new()
    } else {
        let reverse = original.start == node;
        let mut points = geometry::prefix(&original.points, reverse, remaining);
        if reverse {
            points.reverse();
        }
        points
    };
    backbone
}

fn components(graph: &RidgeGraph) -> Vec<usize> {
    let mut adjacent = vec![Vec::new(); graph.nodes.len()];
    for edge in &graph.edges {
        adjacent[edge.start].push(edge.end);
        adjacent[edge.end].push(edge.start);
    }
    let mut labels = vec![usize::MAX; graph.nodes.len()];
    for seed in 0..labels.len() {
        if labels[seed] != usize::MAX {
            continue;
        }
        labels[seed] = seed;
        let mut queue = vec![seed];
        while let Some(node) = queue.pop() {
            for &other in &adjacent[node] {
                if labels[other] == usize::MAX {
                    labels[other] = seed;
                    queue.push(other);
                }
            }
        }
    }
    labels
}

fn minimum(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    Some(a?.min(b?))
}
fn attenuate(source: Option<f64>, contrast: Option<f64>, policy: &FeaturePolicy) -> Option<f64> {
    Some(source? * super::sampling::contrast_factor(contrast?, policy.contrast_reference))
}
fn reason(feature: &mut FeatureStrength, reason: FeatureReason) {
    if !feature.reasons.contains(&reason) {
        feature.reasons.push(reason);
    }
}

fn empty(
    id: usize,
    kind: FeatureKind,
    node: Option<usize>,
    cycle: Option<usize>,
    position: Point,
) -> FeatureStrength {
    FeatureStrength {
        id,
        kind,
        source_node: node,
        source_cycle: cycle,
        position,
        source_support: None,
        geometry_support: None,
        stability: 0.0,
        strength: None,
        source_coverage: 0.0,
        geometry_coverage: 0.0,
        source_contrast: None,
        arms: Vec::new(),
        separation: None,
        geometry_separation: None,
        termination: None,
        continuation: None,
        reasons: Vec::new(),
        stress_matches: [None; 8],
    }
}
