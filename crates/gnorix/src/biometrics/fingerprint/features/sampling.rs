//! Bounded original-source probes with explicit missing-support accounting.

use super::super::{LocalPalette, photometry};
use super::geometry::{self, ArmPath, distance, lerp, segments};
use super::{Capture, FeaturePolicy, FingerprintError, Point};

const MAX_WORK: usize = 200_000_000;

#[derive(Default)]
pub(super) struct Work {
    spent: usize,
}

impl Work {
    pub fn charge(&mut self, amount: usize) -> Result<(), FingerprintError> {
        self.spent = self
            .spent
            .checked_add(amount)
            .filter(|&work| work <= MAX_WORK)
            .ok_or(FingerprintError::WorkLimit)?;
        Ok(())
    }
}

pub(super) struct Sample {
    pub dark: f64,
    pub contrast: f64,
}

pub(super) struct Sampler<'a> {
    capture: &'a Capture,
    palette: &'a LocalPalette,
    pub work: &'a mut Work,
    pub checks: usize,
    pub observed: usize,
}

pub(super) struct PathEvidence {
    pub darkness: Option<f64>,
    pub dark_fraction: Option<f64>,
    pub contrast: Option<f64>,
    pub longest_light: Option<f64>,
}

impl PathEvidence {
    pub fn support(&self, length: f64) -> Option<f64> {
        if length <= 0.0 {
            return None;
        }
        if self.dark_fraction? == 0.0 {
            return Some(0.0);
        }
        let continuity = (1.0 - self.longest_light? / length).clamp(0.0, 1.0);
        Some((self.darkness? * continuity).clamp(0.0, 1.0))
    }
}

impl<'a> Sampler<'a> {
    pub fn new(capture: &'a Capture, palette: &'a LocalPalette, work: &'a mut Work) -> Self {
        Self {
            capture,
            palette,
            work,
            checks: 0,
            observed: 0,
        }
    }

    pub fn sample(&mut self, point: Point) -> Result<Option<Sample>, FingerprintError> {
        self.work.charge(8)?;
        self.checks += 1;
        let Some(darkness) = photometry::sample_darkness(self.capture, self.palette, point) else {
            return Ok(None);
        };
        let (width, height) = (
            self.capture.width() as usize,
            self.capture.height() as usize,
        );
        let (x, y) = (point.x().floor() as usize, point.y().floor() as usize);
        let (tx, ty) = (point.x() - x as f64, point.y() - y as f64);
        let indices = [
            y * width + x,
            y * width + (x + 1).min(width - 1),
            (y + 1).min(height - 1) * width + x,
            (y + 1).min(height - 1) * width + (x + 1).min(width - 1),
        ];
        let values = indices
            .map(|i| f64::from(self.palette.valley_gray[i] - self.palette.ridge_gray[i]) / 255.0);
        let contrast = ((values[0] * (1.0 - tx) + values[1] * tx) * (1.0 - ty)
            + (values[2] * (1.0 - tx) + values[3] * tx) * ty)
            .clamp(0.0, 1.0);
        self.observed += 1;
        Ok(Some(Sample {
            dark: darkness.clamp(0.0, 1.0),
            contrast,
        }))
    }

    pub fn observed_segment(&mut self, a: Point, b: Point) -> Result<bool, FingerprintError> {
        let cells = ((b.x() - a.x()).abs().ceil() + (b.y() - a.y()).abs().ceil() + 4.0) * 16.0;
        if !cells.is_finite() || cells > MAX_WORK as f64 {
            return Err(FingerprintError::WorkLimit);
        }
        self.work.charge(cells as usize)?;
        self.checks += 1;
        let valid = photometry::segment_observed(self.capture, self.palette, a, b).is_some();
        self.observed += usize::from(valid);
        Ok(valid)
    }

    pub fn external_support(&mut self, observed: bool) -> Result<(), FingerprintError> {
        self.work.charge(1)?;
        self.checks += 1;
        self.observed += usize::from(observed);
        Ok(())
    }

    pub fn path(
        &mut self,
        points: &[Point],
        closed: bool,
        spacing: f64,
    ) -> Result<PathEvidence, FingerprintError> {
        let (mut length, mut darkness, mut dark_length, mut contrast) = (0.0, 0.0, 0.0, 0.0);
        let (mut light_run, mut longest_light) = (0.0_f64, 0.0_f64);
        let (mut first_light, mut seen_dark) = (0.0, false);
        let mut complete = true;
        for (a, b) in segments(points, closed) {
            complete &= self.observed_segment(a, b)?;
            let span = distance(a, b);
            let count = intervals(span, spacing)?;
            let weight = span / count as f64;
            for index in 0..count {
                length += weight;
                let sample = self.sample(lerp(a, b, (index as f64 + 0.5) / count as f64))?;
                let Some(sample) = sample else {
                    complete = false;
                    continue;
                };
                darkness += sample.dark * weight;
                contrast += sample.contrast * weight;
                if sample.dark >= 0.5 {
                    seen_dark = true;
                    dark_length += weight;
                    light_run = 0.0;
                } else {
                    light_run += weight;
                    if !seen_dark {
                        first_light += weight;
                    }
                    longest_light = longest_light.max(light_run);
                }
            }
        }
        complete &= length > 0.0;
        if closed {
            longest_light = longest_light.max((first_light + light_run).min(length));
        }
        let measured = |value: f64| if complete { Some(value / length) } else { None };
        Ok(PathEvidence {
            darkness: measured(darkness),
            dark_fraction: measured(dark_length),
            contrast: measured(contrast),
            longest_light: complete.then_some(longest_light),
        })
    }

    pub fn coverage(&self) -> f64 {
        if self.checks == 0 {
            0.0
        } else {
            self.observed as f64 / self.checks as f64
        }
    }
    pub fn complete(&self) -> bool {
        self.checks > 0 && self.observed == self.checks
    }
}

pub(super) fn intervals(length: f64, spacing: f64) -> Result<usize, FingerprintError> {
    let count = (length / spacing).ceil().max(1.0);
    if !count.is_finite() || count > MAX_WORK as f64 {
        return Err(FingerprintError::WorkLimit);
    }
    Ok(count as usize)
}

pub(super) fn contrast_factor(contrast: f64, reference: f64) -> f64 {
    if contrast <= 0.0 {
        0.0
    } else {
        contrast / (contrast + reference)
    }
}

/// Capture availability over the entire forward corridor, independent of local
/// normalization. Uniform observed background can have no usable palette.
/// Each intersected interpolation cell contributes all of its source corners.
pub(super) fn forward_observed(
    capture: &Capture,
    origin: Point,
    inward: [f64; 2],
    policy: &FeaturePolicy,
    work: &mut Work,
) -> Result<bool, FingerprintError> {
    let axis = [-inward[0], -inward[1]];
    let normal = [-axis[1], axis[0]];
    let half_length = policy.continuation_search_px / 2.0;
    let half_width = policy.reference_width_px / 2.0;
    let center = [
        origin.x() + axis[0] * half_length,
        origin.y() + axis[1] * half_length,
    ];
    let extent = [
        axis[0].abs() * half_length + normal[0].abs() * half_width,
        axis[1].abs() * half_length + normal[1].abs() * half_width,
    ];
    let min = [center[0] - extent[0], center[1] - extent[1]];
    let max = [center[0] + extent[0], center[1] + extent[1]];
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    if min[0] < 0.0 || min[1] < 0.0 || max[0] > (width - 1) as f64 || max[1] > (height - 1) as f64 {
        return Ok(false);
    }
    let (left, top, right, bottom) = (
        min[0].floor() as usize,
        min[1].floor() as usize,
        max[0].floor() as usize,
        max[1].floor() as usize,
    );
    work.charge(
        (right - left + 1)
            .checked_mul(bottom - top + 1)
            .and_then(|n| n.checked_mul(8))
            .ok_or(FingerprintError::WorkLimit)?,
    )?;
    let Some(mask) = capture.valid_mask() else {
        return Ok(true);
    };
    let cell_axis = (axis[0].abs() + axis[1].abs()) / 2.0;
    let cell_normal = (normal[0].abs() + normal[1].abs()) / 2.0;
    for y in top..=bottom {
        for x in left..=right {
            let delta = [x as f64 + 0.5 - center[0], y as f64 + 0.5 - center[1]];
            if (delta[0] * axis[0] + delta[1] * axis[1]).abs() > half_length + cell_axis
                || (delta[0] * normal[0] + delta[1] * normal[1]).abs() > half_width + cell_normal
            {
                continue;
            }
            let (x1, y1) = ((x + 1).min(width - 1), (y + 1).min(height - 1));
            if [
                y * width + x,
                y * width + x1,
                y1 * width + x,
                y1 * width + x1,
            ]
            .iter()
            .any(|&index| !mask[index])
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Mean light margin across each pair's transverse connector, integrated over
/// the outer R of both branches. A single bright pixel contributes only its
/// represented width and length. The strongest separated pair is useful for a
/// fork; the weakest arm still limits both overall source and geometry support.
pub(super) fn separation(
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

pub(super) fn interior_light(
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
