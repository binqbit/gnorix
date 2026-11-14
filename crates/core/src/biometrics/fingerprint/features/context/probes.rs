//! Spatially labelled raw-source profiles and feature-specific separation.

use super::super::geometry::{self, at, distance, lerp};
use super::super::sampling::{Sampler, intervals};
use super::super::{FeaturePolicy, FingerprintError, Point};
use super::tracing::Trace;

#[derive(Clone, Copy, Default)]
struct Class {
    count: usize,
    sum: f64,
    squared: f64,
    quantization: f64,
}

impl Class {
    fn mean(self) -> f64 {
        self.sum / self.count as f64
    }
    fn variance(self) -> f64 {
        (self.squared / self.count as f64 - self.mean().powi(2)).max(0.0)
    }
    fn noise(self) -> f64 {
        self.quantization / self.count as f64
    }
    fn include(&mut self, sample: super::super::sampling::Sample) {
        self.count += 1;
        self.sum += sample.raw_dark;
        self.squared += sample.raw_dark.powi(2);
        self.quantization += sample.quantization_variance;
    }
    fn merge(&mut self, other: Self) {
        self.count += other.count;
        self.sum += other.sum;
        self.squared += other.squared;
        self.quantization += other.quantization;
    }
}

/// Contrast-to-noise separation, without a second absolute-contrast ceiling.
/// A flat patch has zero difference. Quantization variance prevents arbitrarily
/// tiny deterministic differences from being treated as perfect evidence.
fn separation(dark: Class, light: Class) -> f64 {
    let difference = dark.mean() - light.mean();
    if difference <= 0.0 {
        return 0.0;
    }
    let signal = difference.powi(2);
    (signal / (signal + dark.variance() + light.variance() + dark.noise() + light.noise()))
        .clamp(0.0, 1.0)
}

pub(super) struct ArmEvidence {
    pub separation: Option<f64>,
    pub continuity: Option<f64>,
    pub supported_length: Option<f64>,
    pub longest_unsupported: Option<f64>,
    length: f64,
}

impl ArmEvidence {
    pub fn support(&self, horizon: f64) -> Option<f64> {
        Some(self.separation? * self.continuity? * (self.length / horizon).min(1.0))
    }
}

pub(super) fn arm(
    trace: &Trace,
    horizon: f64,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<ArmEvidence, FingerprintError> {
    let length = trace.length.min(horizon);
    // The junction core naturally contains merged ridges. Inspect each arm once
    // it departs; a short arm still contributes only its actual finite extent.
    let start = policy.reference_width_px.min(length / 2.0);
    measure_path(&trace.points, length, start, policy, sampler)
}

fn measure_path(
    points: &[Point],
    length: f64,
    start: f64,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<ArmEvidence, FingerprintError> {
    if points.len() < 2 || length <= 0.0 {
        return Ok(ArmEvidence {
            separation: None,
            continuity: None,
            supported_length: None,
            longest_unsupported: None,
            length,
        });
    }
    let span = length - start;
    let count = intervals(span, policy.sample_spacing_px)?;
    let step = span / count as f64;
    let (mut total, mut run, mut longest) = (0.0, 0.0_f64, 0.0_f64);
    let mut complete = true;
    for index in 0..count {
        let station = start + step * (index as f64 + 0.5);
        let value = cross_section(points, station, policy, sampler)?;
        let Some(value) = value else {
            complete = false;
            continue;
        };
        total += value * step;
        if value <= 0.0 {
            run += step;
            longest = longest.max(run);
        } else {
            run = 0.0;
        }
    }
    Ok(ArmEvidence {
        separation: complete.then_some(total / span),
        continuity: complete.then_some((1.0 - longest / span).clamp(0.0, 1.0)),
        supported_length: complete.then_some(total.min(length)),
        longest_unsupported: complete.then_some(longest.min(length)),
        length,
    })
}

/// Source confirmation of a potential continuation uses a whole one-width arc
/// around its projected location. Open endpoints shift the window inward;
/// closed paths wrap without selecting a privileged cycle seam.
pub(super) fn continuation_window(
    points: &[Point],
    closed: bool,
    station: f64,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<f64>, FingerprintError> {
    sampler.work.charge(points.len().saturating_mul(8))?;
    let length = geometry::arc_length(points, closed);
    if length <= 0.0 {
        return Ok(Some(0.0));
    }
    let span = policy.reference_width_px.min(length);
    let start = if closed {
        (station - span / 2.0).rem_euclid(length)
    } else {
        (station - span / 2.0).clamp(0.0, length - span)
    };
    let wrapped;
    let source = if closed {
        wrapped = points
            .iter()
            .chain(points)
            .chain(points.first())
            .copied()
            .collect::<Vec<_>>();
        &wrapped[..]
    } else {
        points
    };
    let prefix = geometry::prefix(source, false, start + span);
    let mut window = geometry::prefix(&prefix, true, span);
    window.reverse();
    let evidence = measure_path(&window, span, 0.0, policy, sampler)?;
    Ok(evidence.support(policy.reference_width_px))
}

pub(super) fn cross_section(
    points: &[Point],
    station: f64,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<f64>, FingerprintError> {
    sampler.work.charge(points.len().saturating_mul(4))?;
    let Some(direction) = tangent(points, station, policy.tangent_support_px) else {
        return Ok(None);
    };
    let center = at(points, station);
    section(center, direction, policy, sampler)
}

fn section(
    center: Point,
    direction: [f64; 2],
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<f64>, FingerprintError> {
    let normal = [-direction[1], direction[0]];
    let unit = policy.reference_width_px;
    let center_class = band(center, normal, -0.2 * unit, 0.2 * unit, policy, sampler)?;
    let left = band(center, normal, -0.9 * unit, -0.7 * unit, policy, sampler)?;
    let right = band(center, normal, 0.7 * unit, 0.9 * unit, policy, sampler)?;
    Ok(match (center_class, left, right) {
        (Some(center), Some(left), Some(right)) => {
            Some(separation(center, left).min(separation(center, right)))
        }
        _ => None,
    })
}

/// An isolated cycle has no privileged seam. Inspect every original segment
/// and wrap the tangent and longest unsupported run across the closing point.
pub(super) fn cycle(
    points: &[Point],
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<ArmEvidence, FingerprintError> {
    sampler.work.charge(points.len().saturating_mul(2))?;
    let mut closed = points.to_vec();
    if let Some(&first) = points.first() {
        closed.push(first);
    }
    let length = geometry::arc_length(&closed, false);
    if length <= 0.0 {
        return Ok(ArmEvidence {
            separation: None,
            continuity: None,
            supported_length: None,
            longest_unsupported: None,
            length,
        });
    }
    let tangent_span = policy.tangent_support_px.min(length / 2.0);
    let (mut total, mut station, mut run, mut longest) = (0.0, 0.0, 0.0_f64, 0.0_f64);
    let (mut first_run, mut seen_supported, mut complete) = (0.0, false, true);
    for pair in closed.windows(2) {
        let span = distance(pair[0], pair[1]);
        let count = intervals(span, policy.sample_spacing_px)?;
        sampler
            .work
            .charge(count.saturating_mul(closed.len()).saturating_mul(2))?;
        let step = span / count as f64;
        for index in 0..count {
            let along = station + step * (index as f64 + 0.5);
            let start = at(&closed, (along - tangent_span / 2.0).rem_euclid(length));
            let end = at(&closed, (along + tangent_span / 2.0).rem_euclid(length));
            let value = if let Some(direction) = geometry::direction(start, end) {
                section(
                    lerp(pair[0], pair[1], (index as f64 + 0.5) / count as f64),
                    direction,
                    policy,
                    sampler,
                )?
            } else {
                None
            };
            if let Some(value) = value {
                total += value * step;
                if value <= 0.0 {
                    run += step;
                    if !seen_supported {
                        first_run += step;
                    }
                    longest = longest.max(run);
                } else {
                    seen_supported = true;
                    run = 0.0;
                }
            } else {
                complete = false;
            }
        }
        station += span;
    }
    longest = longest.max((first_run + run).min(length));
    Ok(ArmEvidence {
        separation: complete.then_some(total / length),
        continuity: complete.then_some((1.0 - longest / length).clamp(0.0, 1.0)),
        supported_length: complete.then_some(total.min(length)),
        longest_unsupported: complete.then_some(longest),
        length,
    })
}

pub(super) fn tangent(points: &[Point], station: f64, span: f64) -> Option<[f64; 2]> {
    let length = geometry::arc_length(points, false);
    if length <= 0.0 {
        return None;
    }
    let span = span.min(length);
    let start = (station - span / 2.0).clamp(0.0, length - span);
    geometry::direction(at(points, start), at(points, start + span))
}

fn band(
    center: Point,
    direction: [f64; 2],
    start: f64,
    end: f64,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<Class>, FingerprintError> {
    let first = offset(center, direction, start);
    let last = offset(center, direction, end);
    let mut complete = sampler.observed_segment(first, last)?;
    let count = intervals(end - start, policy.sample_spacing_px)?;
    let mut class = Class::default();
    for index in 0..count {
        if let Some(sample) =
            sampler.sample(lerp(first, last, (index as f64 + 0.5) / count as f64))?
        {
            class.include(sample);
        } else {
            complete = false;
        }
    }
    Ok(complete.then_some(class))
}

pub(super) fn fork(
    traces: &[Trace],
    horizon: f64,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<f64>, FingerprintError> {
    let paths: Vec<_> = traces
        .iter()
        .map(|trace| (&trace.points[..], trace.length.min(horizon)))
        .collect();
    let pairs = geometry::confusable_pairs(&paths, policy.reference_width_px, sampler.work)?;
    if pairs.is_empty() {
        return Ok(Some(0.0));
    }
    let mut weakest = 1.0_f64;
    let mut complete = true;
    for (a, b) in pairs {
        let upper = paths[a].1.min(paths[b].1);
        let start = policy.reference_width_px;
        if upper <= start {
            return Ok(Some(0.0));
        }
        let count = intervals(upper - start, policy.sample_spacing_px)?;
        sampler
            .work
            .charge(count.saturating_mul(paths[a].0.len() + paths[b].0.len()))?;
        let mut total = 0.0;
        let mut longest = 0.0_f64;
        let mut run = 0.0_f64;
        let step = (upper - start) / count as f64;
        for index in 0..count {
            let along = start + step * (index as f64 + 0.5);
            let value = valley(
                at(paths[a].0, along),
                at(paths[b].0, along),
                policy,
                sampler,
            )?;
            if let Some(value) = value {
                total += value;
                if value <= 0.0 {
                    run += step;
                    longest = longest.max(run);
                } else {
                    run = 0.0;
                }
            } else {
                complete = false;
            }
        }
        let continuity = (1.0 - longest / (upper - start)).clamp(0.0, 1.0);
        weakest = weakest.min(total / count as f64 * continuity);
    }
    Ok(complete.then_some(weakest))
}

fn valley(
    first: Point,
    second: Point,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<f64>, FingerprintError> {
    let width = distance(first, second);
    let Some(direction) = geometry::direction(first, second) else {
        return Ok(Some(0.0));
    };
    let unit = policy.reference_width_px;
    // The interior starts beyond the largest accepted ridge half-width.
    let interior = width - 1.2 * unit;
    if interior <= 0.0 {
        return Ok(Some(0.0));
    }
    let a = band(first, direction, -0.2 * unit, 0.2 * unit, policy, sampler)?;
    let b = band(second, direction, -0.2 * unit, 0.2 * unit, policy, sampler)?;
    let middle = band(
        first,
        direction,
        0.6 * unit,
        width - 0.6 * unit,
        policy,
        sampler,
    )?;
    Ok(match (a, b, middle) {
        (Some(a), Some(b), Some(middle)) => {
            let resolved_width = (interior / (unit / 3.0)).min(1.0);
            Some(separation(a, middle).min(separation(b, middle)) * resolved_width)
        }
        _ => None,
    })
}

/// Compare complete physical caps with the observed incoming ridge. Source
/// separation is graded; a single lateral dark sample is not a categorical veto.
#[derive(Default)]
pub(super) struct CapEvidence {
    pub separation: Option<f64>,
    pub reference_margin: Option<f64>,
}

pub(super) fn cap(
    trace: &Trace,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<CapEvidence, FingerprintError> {
    sampler.work.charge(trace.points.len().saturating_mul(3))?;
    let Some(inward) = tangent(&trace.points, 0.0, policy.tangent_support_px) else {
        return Ok(CapEvidence::default());
    };
    let outward = [-inward[0], -inward[1]];
    let normal = [-inward[1], inward[0]];
    let origin = trace.points[0];
    let rows = intervals(policy.terminal_barrier_depth_px, policy.sample_spacing_px)?;
    sampler
        .work
        .charge(rows.saturating_mul(trace.points.len()))?;
    let mut dark = Class::default();
    let mut complete = true;
    for row in 0..rows {
        let shift = policy.terminal_barrier_depth_px * (row as f64 + 0.5) / rows as f64;
        let ridge = at(
            &trace.points,
            (policy.reference_width_px / 2.0 + shift).min(trace.length),
        );
        match band(
            ridge,
            normal,
            -0.2 * policy.reference_width_px,
            0.2 * policy.reference_width_px,
            policy,
            sampler,
        )? {
            Some(sampled) => dark.merge(sampled),
            None => complete = false,
        }
    }
    let reference_margin =
        (complete && dark.count > 0).then(|| (2.0 * dark.mean() - 1.0).clamp(0.0, 1.0));
    let range = policy.source_search_px - policy.terminal_barrier_depth_px;
    let windows = intervals(range, policy.sample_spacing_px)?;
    let mut strongest = 0.0_f64;
    for window in 0..=windows {
        let start = range * window as f64 / windows as f64;
        match cap_window(origin, outward, normal, start, policy, sampler)? {
            Some(light) if dark.count > 0 => strongest = strongest.max(separation(dark, light)),
            _ => complete = false,
        }
    }
    Ok(CapEvidence {
        separation: complete.then_some(strongest),
        reference_margin,
    })
}

fn cap_window(
    origin: Point,
    outward: [f64; 2],
    normal: [f64; 2],
    start: f64,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<Class>, FingerprintError> {
    let depth = policy.terminal_barrier_depth_px;
    let half = policy.terminal_barrier_width_px / 2.0;
    let first = offset(origin, outward, start);
    let last = offset(origin, outward, start + depth);
    let corners = [
        offset(first, normal, -half),
        offset(last, normal, -half),
        offset(last, normal, half),
        offset(first, normal, half),
    ];
    let mut complete = true;
    for (a, b) in corners.iter().zip(corners.iter().cycle().skip(1)).take(4) {
        complete &= sampler.observed_segment(*a, *b)?;
    }
    let rows = intervals(depth, policy.sample_spacing_px)?;
    let mut light = Class::default();
    for row in 0..rows {
        let center = offset(
            origin,
            outward,
            start + depth * (row as f64 + 0.5) / rows as f64,
        );
        match band(center, normal, -half, half, policy, sampler)? {
            Some(sampled) => light.merge(sampled),
            None => complete = false,
        }
    }
    Ok(complete.then_some(light))
}

/// Explain source around the actually observed terminal path. No endpoint is
/// extended: a strong bend beyond its endpoint must remain a contradiction.
pub(super) fn incoming_footprint(
    trace: &Trace,
    point: Point,
    policy: &FeaturePolicy,
    sampler: &mut Sampler<'_>,
) -> Result<Option<f64>, FingerprintError> {
    sampler.work.charge(trace.points.len().saturating_mul(8))?;
    let limit = trace.length.min(policy.branch_horizon_px);
    let path = geometry::prefix(&trace.points, false, limit);
    let mut nearest = None;
    let mut position = 0.0;
    for pair in path.windows(2) {
        let span = distance(pair[0], pair[1]);
        if span <= 0.0 {
            continue;
        }
        let direction = [
            (pair[1].x() - pair[0].x()) / span,
            (pair[1].y() - pair[0].y()) / span,
        ];
        let along =
            (point.x() - pair[0].x()) * direction[0] + (point.y() - pair[0].y()) * direction[1];
        let shift = along.clamp(0.0, span);
        let center = offset(pair[0], direction, shift);
        let distance = distance(center, point);
        if nearest.is_none_or(|(_, best, _)| distance < best) {
            nearest = Some((position + shift, distance, center));
        }
        position += span;
    }
    let Some((station, distance, center)) = nearest else {
        return Ok(Some(0.0));
    };
    if center == path[0] || path.last() == Some(&center) {
        let Some(direction) = tangent(&path, station, policy.tangent_support_px) else {
            return Ok(None);
        };
        let along =
            (point.x() - center.x()) * direction[0] + (point.y() - center.y()) * direction[1];
        if (center == path[0] && along < 0.0) || (path.last() == Some(&center) && along > 0.0) {
            return Ok(Some(0.0));
        }
    }
    let half = policy.reference_width_px / 2.0;
    let maximum = 1.2 * half;
    let membership = ((maximum - distance) / (maximum - half)).clamp(0.0, 1.0);
    if membership <= 0.0 {
        return Ok(Some(0.0));
    }
    Ok(cross_section(&path, station, policy, sampler)?.map(|support| support * membership))
}

fn offset(point: Point, direction: [f64; 2], distance: f64) -> Point {
    Point::new(
        point.x() + direction[0] * distance,
        point.y() + direction[1] * distance,
    )
    .expect("finite source-probe coordinates")
}
