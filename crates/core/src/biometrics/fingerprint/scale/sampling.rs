//! Raw transverse grayscale sampling shared by bootstrap and full measurement.

use super::{
    Capture, FingerprintError, MAX_PERIOD, MAX_PROFILE_SAMPLES, MIN_PERIOD, Point, RidgeSpan,
    ScaleProfile, photometry,
};

pub(in super::super) struct WidthProfile {
    pub(in super::super) ridge: RidgeSpan,
    pub(in super::super) confirmation: Option<ScaleProfile>,
}

/// Measures the complete dark span containing the requested source point.
/// Nearby ridges may confirm its region, but never replace an absent own span.
pub(in super::super) fn sample_width(
    capture: &Capture,
    x: f64,
    y: f64,
    normal_angle: f64,
    period: f64,
    min_contrast: f64,
) -> Result<Option<WidthProfile>, FingerprintError> {
    if !x.is_finite()
        || !y.is_finite()
        || !normal_angle.is_finite()
        || !period.is_finite()
        || !(MIN_PERIOD..=MAX_PERIOD).contains(&period)
        || !min_contrast.is_finite()
        || !(0.0..=1.0).contains(&min_contrast)
    {
        return Err(FingerprintError::InvalidConfig(
            "invalid raw-width sampling controls",
        ));
    }
    if source_sample(capture, x, y).is_none() {
        return Ok(None);
    }
    let half = 2.25 * period;
    let spacing = 0.5;
    let samples = profile(
        |x, y| source_sample(capture, x, y),
        x,
        y,
        normal_angle,
        half,
        spacing,
        (period / 8.0).min(4.0),
    )?;
    let smoothed = smooth_runs(&samples, period / 32.0 / spacing);
    let target = half / spacing;
    let Some((start, end)) = containing_run(&smoothed, target) else {
        return Ok(None);
    };
    let values: Vec<_> = smoothed[start..end]
        .iter()
        .map(|value| value.unwrap())
        .collect();
    let relative_target = target - start as f64;
    let confirmed = confirming_triple(&dark_spans(&values, min_contrast), relative_target, spacing);

    if let Some(triple) = confirmed {
        let own_index = triple
            .iter()
            .position(|span| contains(*span, relative_target))
            .unwrap();
        let triple = triple.map(|span| offset_span(span, start, spacing));
        let ridge = source_span(x, y, normal_angle, half, triple[own_index])?;
        let confirmation = match source_profile(capture, x, y, normal_angle, half, triple) {
            Ok(profile) => Some(profile),
            Err(FingerprintError::ScaleUnavailable(_)) => None,
            Err(error) => return Err(error),
        };
        return Ok(Some(WidthProfile {
            ridge,
            confirmation,
        }));
    }

    // A lone visible ridge may occupy less than a tenth of this window. Its
    // actual extrema can support its own width without confirming three ridges.
    let contrast = values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - values.iter().copied().fold(f64::INFINITY, f64::min);
    let own = spans_with_contrast(&values, min_contrast, contrast)
        .into_iter()
        .find(|&span| contains(span, relative_target));
    let Some(own) = own else {
        return Ok(None);
    };
    Ok(Some(WidthProfile {
        ridge: source_span(x, y, normal_angle, half, offset_span(own, start, spacing))?,
        confirmation: None,
    }))
}

fn contains(span: Span, target: f64) -> bool {
    span.left <= target && target <= span.right
}

fn containing_run(samples: &[Option<f64>], target: f64) -> Option<(usize, usize)> {
    let (left, right) = (target.floor() as usize, target.ceil() as usize);
    samples.get(left)?.as_ref()?;
    samples.get(right)?.as_ref()?;
    let mut start = left;
    while start > 0 && samples[start - 1].is_some() {
        start -= 1;
    }
    let mut end = right + 1;
    while end < samples.len() && samples[end].is_some() {
        end += 1;
    }
    (end - start >= 3).then_some((start, end))
}

fn confirming_triple(spans: &[Span], target: f64, spacing: f64) -> Option<[Span; 3]> {
    spans
        .windows(3)
        .filter(|triple| {
            triple.iter().any(|&span| contains(span, target))
                && triple.windows(2).all(|pair| {
                    pair[0].right < pair[1].left
                        && (MIN_PERIOD..=MAX_PERIOD)
                            .contains(&((pair[1].center() - pair[0].center()) * spacing))
                })
        })
        .min_by(|a, b| {
            (a[1].center() - target)
                .abs()
                .total_cmp(&(b[1].center() - target).abs())
        })
        .map(|triple| [triple[0], triple[1], triple[2]])
}

fn offset_span(span: Span, start: usize, spacing: f64) -> Span {
    Span {
        left: (span.left + start as f64) * spacing,
        right: (span.right + start as f64) * spacing,
    }
}

fn source_span(
    x: f64,
    y: f64,
    angle: f64,
    half: f64,
    span: Span,
) -> Result<RidgeSpan, FingerprintError> {
    let (sin, cos) = angle.sin_cos();
    let point = |distance: f64| {
        Point::new(x + (distance - half) * cos, y + (distance - half) * sin)
            .map_err(|_| FingerprintError::ScaleUnavailable("nonfinite scale coordinates"))
    };
    Ok(RidgeSpan {
        start: point(span.left)?,
        end: point(span.right)?,
    })
}

pub(super) fn source_profile(
    capture: &Capture,
    x: f64,
    y: f64,
    angle: f64,
    half: f64,
    triple: [Span; 3],
) -> Result<ScaleProfile, FingerprintError> {
    let [a, b, c] = triple.map(|span| source_span(x, y, angle, half, span));
    let ridges = [a?, b?, c?];
    let [a, b, c] = ridges
        .each_ref()
        .map(|ridge| photometry::mean_gray(capture, ridge.start, ridge.end));
    let [left, right] = [
        (ridges[0].end, ridges[1].start),
        (ridges[1].end, ridges[2].start),
    ]
    .map(|(start, end)| photometry::mean_gray(capture, start, end));
    Ok(ScaleProfile {
        ridges,
        ridge_mean_gray: [a?, b?, c?],
        valley_mean_gray: [left?, right?],
    })
}

fn source_sample(capture: &Capture, x: f64, y: f64) -> Option<f64> {
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    if x < 0.0 || y < 0.0 || x > (width - 1) as f64 || y > (height - 1) as f64 {
        return None;
    }
    let (left, top) = (x.floor() as usize, y.floor() as usize);
    let (right, bottom) = ((left + 1).min(width - 1), (top + 1).min(height - 1));
    let indices = [
        top * width + left,
        top * width + right,
        bottom * width + left,
        bottom * width + right,
    ];
    if capture
        .valid_mask()
        .is_some_and(|mask| indices.iter().any(|&index| !mask[index]))
    {
        return None;
    }
    let (dx, dy) = (x - left as f64, y - top as f64);
    let value = |i: usize| f64::from(capture.pixels()[indices[i]]) / 255.0;
    Some(
        (value(0) * (1.0 - dx) + value(1) * dx) * (1.0 - dy)
            + (value(2) * (1.0 - dx) + value(3) * dx) * dy,
    )
}

#[derive(Clone, Copy)]
pub(super) struct Span {
    pub(super) left: f64,
    pub(super) right: f64,
}

impl Span {
    pub(super) fn center(self) -> f64 {
        (self.left + self.right) / 2.0
    }
}

pub(super) fn profile(
    sample: impl Fn(f64, f64) -> Option<f64>,
    x: f64,
    y: f64,
    angle: f64,
    half: f64,
    spacing: f64,
    strip: f64,
) -> Result<Vec<Option<f64>>, FingerprintError> {
    let intervals = (2.0 * half / spacing).ceil() as usize;
    if intervals + 1 > MAX_PROFILE_SAMPLES {
        return Err(FingerprintError::WorkLimit);
    }
    let (sin, cos) = angle.sin_cos();
    let strip_steps = strip.ceil().max(1.0) as i32;
    Ok((0..=intervals)
        .map(|i| {
            let along = i as f64 * spacing - half;
            let mut sum = 0.0;
            for step in -strip_steps..=strip_steps {
                let across = f64::from(step) * strip / f64::from(strip_steps);
                sum += sample(
                    x + along * cos - across * sin,
                    y + along * sin + across * cos,
                )?;
            }
            Some(sum / f64::from(2 * strip_steps + 1))
        })
        .collect())
}

pub(super) fn smooth_runs(samples: &[Option<f64>], sigma: f64) -> Vec<Option<f64>> {
    let radius = (3.0 * sigma).ceil() as usize;
    let weights: Vec<_> = (0..=radius)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .collect();
    let mut output = vec![None; samples.len()];
    let mut start = 0;
    while start < samples.len() {
        while start < samples.len() && samples[start].is_none() {
            start += 1;
        }
        let mut end = start;
        while end < samples.len() && samples[end].is_some() {
            end += 1;
        }
        for (i, target) in output.iter_mut().enumerate().take(end).skip(start) {
            let (mut sum, mut weight) = (0.0, 0.0);
            for (j, value) in samples
                .iter()
                .enumerate()
                .take((i + radius + 1).min(end))
                .skip(i.saturating_sub(radius).max(start))
            {
                let w = weights[i.abs_diff(j)];
                sum += w * value.unwrap();
                weight += w;
            }
            *target = Some(sum / weight);
        }
        start = end.saturating_add(1);
    }
    output
}

pub(super) fn dark_spans(values: &[f64], minimum_contrast: f64) -> Vec<Span> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let contrast = sorted[sorted.len() * 9 / 10] - sorted[sorted.len() / 10];
    spans_with_contrast(values, minimum_contrast, contrast)
}

fn spans_with_contrast(values: &[f64], minimum_contrast: f64, contrast: f64) -> Vec<Span> {
    let prominence = (0.2 * contrast)
        .max(2.0 * minimum_contrast)
        .max(1.0 / 255.0);
    if contrast < 2.0 * minimum_contrast || contrast < 1.0 / 255.0 {
        return Vec::new();
    }
    let mut minima = Vec::new();
    let mut i = 1;
    while i + 1 < values.len() {
        let mut end = i;
        while end + 1 < values.len() && (values[end + 1] - values[i]).abs() < 1e-12 {
            end += 1;
        }
        if end + 1 < values.len() && values[i] < values[i - 1] && values[end] < values[end + 1] {
            let center = (i + end) / 2;
            let mut left = values[center];
            let mut right = left;
            for &v in values[..center].iter().rev() {
                if v < values[center] - 1e-12 {
                    break;
                }
                left = left.max(v);
            }
            for &v in &values[center + 1..] {
                if v < values[center] - 1e-12 {
                    break;
                }
                right = right.max(v);
            }
            if left.min(right) - values[center] >= prominence {
                minima.push(center);
            }
        }
        i = end + 1;
    }
    let mut spans = Vec::new();
    for (k, &center) in minima.iter().enumerate() {
        let left_limit = if k == 0 { 0 } else { minima[k - 1] };
        let right_limit = if k + 1 == minima.len() {
            values.len() - 1
        } else {
            minima[k + 1]
        };
        let left_peak = values[left_limit..center]
            .iter()
            .copied()
            .fold(values[center], f64::max);
        let right_peak = values[center + 1..=right_limit]
            .iter()
            .copied()
            .fold(values[center], f64::max);
        let left_level = (left_peak + values[center]) / 2.0;
        let right_level = (right_peak + values[center]) / 2.0;
        let mut left = center;
        while left > left_limit && values[left] < left_level {
            left -= 1;
        }
        let mut right = center;
        while right < right_limit && values[right] < right_level {
            right += 1;
        }
        if left < center
            && right > center
            && values[left] >= left_level
            && values[right] >= right_level
        {
            let a = left as f64 + (values[left] - left_level) / (values[left] - values[left + 1]);
            let b = (right - 1) as f64
                + (right_level - values[right - 1]) / (values[right] - values[right - 1]);
            if a < b {
                spans.push(Span { left: a, right: b });
            }
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ridges(centers: &[usize], mask: Option<Vec<bool>>) -> Capture {
        let pixels = (0_usize..160 * 48)
            .map(|index| {
                if centers.iter().any(|&x| (index % 160).abs_diff(x) < 3) {
                    20
                } else {
                    230
                }
            })
            .collect();
        Capture::new(160, 48, pixels, mask, None).unwrap()
    }

    #[test]
    fn confirmation_contains_the_actual_requested_ridge() {
        let capture = ridges(&[30, 50, 70], None);
        let measured = sample_width(&capture, 50.0, 24.0, 0.0, 20.0, 0.035)
            .unwrap()
            .unwrap();
        assert!(measured.ridge.start.x() < 50.0 && measured.ridge.end.x() > 50.0);
        assert_eq!(measured.ridge.start.y(), 24.0);
        assert!(
            measured
                .confirmation
                .unwrap()
                .ridges
                .contains(&measured.ridge)
        );
    }

    #[test]
    fn light_target_does_not_jump_to_a_nearby_confirmed_ridge() {
        let capture = ridges(&[30, 50, 70], None);
        assert!(
            sample_width(&capture, 40.0, 24.0, 0.0, 20.0, 0.035)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_lone_complete_span_is_measurable_without_confirming_a_region() {
        let capture = ridges(&[80], None);
        let measured = sample_width(&capture, 80.0, 24.0, 0.0, 20.0, 0.035)
            .unwrap()
            .unwrap();
        assert!(measured.confirmation.is_none());
        assert!((measured.ridge.end.x() - measured.ridge.start.x() - 5.0).abs() < 0.1);
    }

    #[test]
    fn mask_incomplete_confirmation_preserves_only_the_observed_own_span() {
        let mut mask = vec![true; 160 * 48];
        for y in 0..48 {
            mask[y * 160 + 90] = false;
        }
        let capture = ridges(&[60, 80, 100], Some(mask.clone()));
        let first = sample_width(&capture, 80.0, 24.0, 0.0, 20.0, 0.035)
            .unwrap()
            .unwrap();
        assert!(first.confirmation.is_none());
        let mut pixels = capture.pixels().to_vec();
        for (index, &valid) in mask.iter().enumerate() {
            if !valid {
                pixels[index] = 0;
            }
        }
        let changed = Capture::new(160, 48, pixels, Some(mask), None).unwrap();
        let second = sample_width(&changed, 80.0, 24.0, 0.0, 20.0, 0.035)
            .unwrap()
            .unwrap();
        assert_eq!(first.ridge, second.ridge);
        assert!(second.confirmation.is_none());
        assert!(
            sample_width(&changed, 90.0, 24.0, 0.0, 20.0, 0.035)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn invalid_controls_fail_while_unobserved_targets_remain_local_failures() {
        let capture = ridges(&[30, 50, 70], None);
        assert!(matches!(
            sample_width(&capture, 50.0, 24.0, f64::NAN, 20.0, 0.035),
            Err(FingerprintError::InvalidConfig(_))
        ));
        assert!(matches!(
            sample_width(&capture, 50.0, 24.0, 0.0, 0.0, 0.035),
            Err(FingerprintError::InvalidConfig(_))
        ));
        assert!(
            sample_width(&capture, -1.0, 24.0, 0.0, 20.0, 0.035)
                .unwrap()
                .is_none()
        );
    }
}
