//! Provisional bootstrap and diagnostics from complete grayscale ridge sequences.
//!
//! The fixed-pixel pyramid bootstraps orientation without a known ridge period.
//! Perpendicular projections follow the general approach used in Hong, Wan &
//! Jain (1998), doi:10.1109/34.709565; this implementation, its scale search and
//! half-local-contrast widths are our own adaptation, not a paper reproduction.

mod sampling;

pub(super) use sampling::WidthProfile;
use sampling::{Span, dark_spans, profile, smooth_runs};

use super::{Capture, ExtractionConfig, FingerprintError, photometry};
use crate::biometrics::geometry::Point;

const MIN_PERIOD: f64 = 4.0;
const MAX_PERIOD: f64 = 128.0;
const MAX_PROFILE_SAMPLES: usize = 1_153;

pub(super) fn sample_width(
    capture: &Capture,
    x: f64,
    y: f64,
    normal_angle: f64,
    period: f64,
    min_contrast: f64,
) -> Result<Option<WidthProfile>, FingerprintError> {
    sampling::sample_width(capture, x, y, normal_angle, period, min_contrast)
}

/// A dark ridge bounded by two half-local-contrast grayscale crossings.
#[derive(Clone, Debug, PartialEq)]
pub struct RidgeSpan {
    pub start: Point,
    pub end: Point,
}

/// Exactly three complete dark ridges separated by two observed light gaps.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleProfile {
    pub ridges: [RidgeSpan; 3],
    /// Raw source grayscale line means, in the same order as `ridges`.
    pub ridge_mean_gray: [f64; 3],
    /// Raw source means between ridges 0→1 and 1→2, excluding outer background.
    pub valley_mean_gray: [f64; 2],
}

/// Scale and raw grayscale photometry of all confirmed source profiles.
/// Width and period medians use every observation in source pixels. Dispersion
/// does not reject measurements or imply recognition probability.
#[derive(Clone, Debug, PartialEq)]
pub struct BootstrapScale {
    pub ridge_width_px: f64,
    pub valley_width_px: f64,
    pub ridge_period_px: f64,
    pub ridge_width_mad_px: f64,
    pub ridge_period_mad_px: f64,
    /// Arithmetic mean of all three ridge widths per confirmed profile.
    pub mean_ridge_width_px: f64,
    /// Arithmetic mean of both observed internal gaps per confirmed profile.
    pub mean_valley_width_px: f64,
    /// Length-weighted mean raw grayscale over all confirmed ridge spans, 0–255.
    pub mean_ridge_gray: f64,
    /// Length-weighted mean raw grayscale over internal gaps only, 0–255.
    pub mean_valley_gray: f64,
    pub examined_locations: usize,
    pub profiles: Vec<ScaleProfile>,
}

struct Image {
    width: usize,
    height: usize,
    factor: usize,
    values: Vec<f64>,
    valid: Vec<bool>,
}

impl Image {
    fn downsample(&self) -> Option<Self> {
        let (width, height) = (self.width / 2, self.height / 2);
        if width < 3 || height < 3 {
            return None;
        }
        let mut image = Self {
            width,
            height,
            factor: self.factor * 2,
            values: vec![0.0; width * height],
            valid: vec![false; width * height],
        };
        for y in 0..height {
            for x in 0..width {
                let indices = [
                    2 * y * self.width + 2 * x,
                    2 * y * self.width + 2 * x + 1,
                    (2 * y + 1) * self.width + 2 * x,
                    (2 * y + 1) * self.width + 2 * x + 1,
                ];
                if indices.iter().all(|&i| self.valid[i]) {
                    image.values[y * width + x] =
                        indices.iter().map(|&i| self.values[i]).sum::<f64>() / 4.0;
                    image.valid[y * width + x] = true;
                }
            }
        }
        Some(image)
    }

    fn sample(&self, x: f64, y: f64) -> Option<f64> {
        if x < 0.0 || y < 0.0 || x > (self.width - 1) as f64 || y > (self.height - 1) as f64 {
            return None;
        }
        let (left, top) = (x.floor() as usize, y.floor() as usize);
        let (right, bottom) = (
            (left + 1).min(self.width - 1),
            (top + 1).min(self.height - 1),
        );
        let indices = [
            top * self.width + left,
            top * self.width + right,
            bottom * self.width + left,
            bottom * self.width + right,
        ];
        if indices.iter().any(|&i| !self.valid[i]) {
            return None;
        }
        let (dx, dy) = (x - left as f64, y - top as f64);
        Some(
            (self.values[indices[0]] * (1.0 - dx) + self.values[indices[1]] * dx) * (1.0 - dy)
                + (self.values[indices[2]] * (1.0 - dx) + self.values[indices[3]] * dx) * dy,
        )
    }
}

struct Candidate {
    angle: f64,
    period: f64,
    coherence: f64,
}

pub(super) fn estimate(
    capture: &Capture,
    config: &ExtractionConfig,
) -> Result<BootstrapScale, FingerprintError> {
    config.validate()?;
    let count = capture.pixels().len();
    if count > config.max_pixels {
        return Err(FingerprintError::WorkLimit);
    }
    if config.bootstrap_grid_locations == 0 {
        return Err(FingerprintError::InvalidConfig(
            "bootstrap grid must contain at least one location",
        ));
    }
    let source = Image {
        width: capture.width() as usize,
        height: capture.height() as usize,
        factor: 1,
        values: capture
            .pixels()
            .iter()
            .map(|&v| f64::from(v) / 255.0)
            .collect(),
        valid: capture
            .valid_mask()
            .map_or_else(|| vec![true; count], <[bool]>::to_vec),
    };
    let mut pyramid = vec![source];
    for _ in 0..4 {
        let Some(next) = pyramid.last().unwrap().downsample() else {
            break;
        };
        pyramid.push(next);
    }
    let source = &pyramid[0];
    let budget = config.bootstrap_grid_locations.min(count);
    let columns = ((budget as f64 * source.width as f64 / source.height as f64)
        .sqrt()
        .round() as usize)
        .clamp(1, source.width.min(budget));
    let rows = (budget / columns).clamp(1, source.height);
    let mut profiles = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            let x = (column as f64 + 0.5) * source.width as f64 / columns as f64 - 0.5;
            let y = (row as f64 + 0.5) * source.height as f64 / rows as f64 - 0.5;
            if let Some(profile) = find_profile(capture, &pyramid, x, y, config)? {
                profiles.push(profile);
            }
        }
    }
    let mut examined = columns * rows;
    if profiles.is_empty() {
        // A sparse initialization grid may miss a small valid ridge patch.
        // Exhaustive fallback only initializes support: its first success does
        // not become the final full-traversal width unit. Constant neighborhoods
        // cannot contain a crossing and need no expensive profile search.
        for index in 0..count {
            if !has_local_gradient(source, index) {
                continue;
            }
            examined = examined.checked_add(1).ok_or(FingerprintError::WorkLimit)?;
            if let Some(profile) = find_profile(
                capture,
                &pyramid,
                (index % source.width) as f64,
                (index / source.width) as f64,
                config,
            )? {
                profiles.push(profile);
                break;
            }
        }
    }
    summarize(profiles, examined)
}

fn find_profile(
    capture: &Capture,
    pyramid: &[Image],
    x: f64,
    y: f64,
    config: &ExtractionConfig,
) -> Result<Option<ScaleProfile>, FingerprintError> {
    let source = &pyramid[0];
    let mut candidates = Vec::new();
    for image in pyramid {
        let factor = image.factor as f64;
        let (lx, ly) = ((x + 0.5) / factor - 0.5, (y + 0.5) / factor - 0.5);
        let Some((angle, coherence)) = orientation(image, lx, ly, config.min_coherence) else {
            continue;
        };
        let samples = profile(|x, y| image.sample(x, y), lx, ly, angle, 32.0, 0.5, 2.0)?;
        if let Some(triple) = choose_triple(&samples, 0.5, 4.0, 16.0, config.min_contrast) {
            let period = (triple[2].center() - triple[0].center()) / 2.0 * factor;
            if (MIN_PERIOD..=MAX_PERIOD).contains(&period) {
                candidates.push(Candidate {
                    angle,
                    period,
                    coherence,
                });
            }
        }
    }
    // Multiple pyramid detections at one source site are support alternatives,
    // not independent measurements or a reason to select a width group.
    candidates.sort_by(|a, b| b.coherence.total_cmp(&a.coherence));
    for candidate in candidates {
        let half = 2.25 * candidate.period;
        let samples = profile(
            |x, y| source.sample(x, y),
            x,
            y,
            candidate.angle,
            half,
            0.5,
            (candidate.period / 8.0).min(4.0),
        )?;
        let smooth = smooth_runs(&samples, candidate.period / 32.0 / 0.5);
        if let Some(triple) =
            choose_triple(&smooth, 0.5, MIN_PERIOD, MAX_PERIOD, config.min_contrast)
        {
            match sampling::source_profile(capture, x, y, candidate.angle, half, triple) {
                Ok(profile) => return Ok(Some(profile)),
                Err(FingerprintError::ScaleUnavailable(_)) => continue,
                Err(error) => return Err(error),
            }
        }
    }
    Ok(None)
}

fn has_local_gradient(image: &Image, index: usize) -> bool {
    if !image.valid[index] {
        return false;
    }
    let (x, y) = (index % image.width, index / image.width);
    if x == 0 || y == 0 || x + 1 == image.width || y + 1 == image.height {
        return false;
    }
    let adjacent = [
        index - 1,
        index + 1,
        index - image.width,
        index + image.width,
    ];
    adjacent.iter().all(|&i| image.valid[i])
        && (image.values[index - 1] != image.values[index + 1]
            || image.values[index - image.width] != image.values[index + image.width])
}

fn orientation(image: &Image, x: f64, y: f64, minimum: f64) -> Option<(f64, f64)> {
    if image.width < 3 || image.height < 3 || x < 0.0 || y < 0.0 {
        return None;
    }
    let (cx, cy) = (x.round() as usize, y.round() as usize);
    let (left, right) = (
        cx.saturating_sub(8).max(1),
        cx.saturating_add(8).min(image.width - 2),
    );
    let (top, bottom) = (
        cy.saturating_sub(8).max(1),
        cy.saturating_add(8).min(image.height - 2),
    );
    let (mut xx, mut xy, mut yy, mut count) = (0.0, 0.0, 0.0, 0);
    for row in top..=bottom {
        for column in left..=right {
            let i = row * image.width + column;
            if [i - 1, i + 1, i - image.width, i + image.width]
                .iter()
                .any(|&i| !image.valid[i])
            {
                return None;
            }
            let gx = (image.values[i + 1] - image.values[i - 1]) / 2.0;
            let gy = (image.values[i + image.width] - image.values[i - image.width]) / 2.0;
            xx += gx * gx;
            xy += gx * gy;
            yy += gy * gy;
            count += 1;
        }
    }
    let energy = xx + yy;
    if count < 9 || energy <= 1e-12 {
        return None;
    }
    let coherence = ((xx - yy).hypot(2.0 * xy) / energy).clamp(0.0, 1.0);
    (coherence >= minimum).then_some(((2.0 * xy).atan2(xx - yy) / 2.0, coherence))
}

fn choose_triple(
    samples: &[Option<f64>],
    spacing: f64,
    min_period: f64,
    max_period: f64,
    min_contrast: f64,
) -> Option<[Span; 3]> {
    let mut choices = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        while start < samples.len() && samples[start].is_none() {
            start += 1;
        }
        let mut end = start;
        while end < samples.len() && samples[end].is_some() {
            end += 1;
        }
        if end - start >= 9 {
            let values: Vec<_> = samples[start..end].iter().map(|v| v.unwrap()).collect();
            let spans = dark_spans(&values, min_contrast);
            for triple in spans.windows(3) {
                let [a, b, c] = [triple[0], triple[1], triple[2]].map(|s| Span {
                    left: (s.left + start as f64) * spacing,
                    right: (s.right + start as f64) * spacing,
                });
                let periods = [b.center() - a.center(), c.center() - b.center()];
                if periods
                    .iter()
                    .all(|p| (min_period..=max_period).contains(p))
                    && a.right < b.left
                    && b.right < c.left
                {
                    choices.push([a, b, c]);
                }
            }
        }
        start = end.saturating_add(1);
    }
    let center = (samples.len() - 1) as f64 * spacing / 2.0;
    choices.into_iter().min_by(|a, b| {
        (a[1].center() - center)
            .abs()
            .total_cmp(&(b[1].center() - center).abs())
    })
}

fn summarize(
    profiles: Vec<ScaleProfile>,
    examined_locations: usize,
) -> Result<BootstrapScale, FingerprintError> {
    if profiles.is_empty() {
        return Err(FingerprintError::ScaleUnavailable(
            "no complete supported three-ridge grayscale profile",
        ));
    }
    let mut widths = Vec::with_capacity(profiles.len() * 3);
    let mut periods = Vec::with_capacity(profiles.len() * 2);
    let mut valleys = Vec::with_capacity(profiles.len() * 2);
    let (mut ridge_integral, mut valley_integral) = (0.0, 0.0);
    for profile in &profiles {
        for (ridge, &gray) in profile.ridges.iter().zip(&profile.ridge_mean_gray) {
            let width = distance(ridge.start, ridge.end);
            widths.push(width);
            ridge_integral += width * gray;
        }
        for (pair, &gray) in profile.ridges.windows(2).zip(&profile.valley_mean_gray) {
            let midpoint = |ridge: &RidgeSpan| {
                Point::new(
                    (ridge.start.x() + ridge.end.x()) / 2.0,
                    (ridge.start.y() + ridge.end.y()) / 2.0,
                )
                .unwrap()
            };
            periods.push(distance(midpoint(&pair[0]), midpoint(&pair[1])));
            let gap = distance(pair[0].end, pair[1].start);
            valleys.push(gap);
            valley_integral += gap * gray;
        }
    }
    let ridge_length: f64 = widths.iter().sum();
    let valley_length: f64 = valleys.iter().sum();
    let mean_ridge_width_px = ridge_length / widths.len() as f64;
    let mean_valley_width_px = valley_length / valleys.len() as f64;
    let mean_ridge_gray = ridge_integral / ridge_length;
    let mean_valley_gray = valley_integral / valley_length;
    let ridge_width_px = median(&mut widths);
    let ridge_period_px = median(&mut periods);
    let ridge_width_mad_px = median(
        &mut widths
            .iter()
            .map(|&v| (v - ridge_width_px).abs())
            .collect::<Vec<_>>(),
    );
    let ridge_period_mad_px = median(
        &mut periods
            .iter()
            .map(|&v| (v - ridge_period_px).abs())
            .collect::<Vec<_>>(),
    );
    Ok(BootstrapScale {
        ridge_width_px,
        valley_width_px: median(&mut valleys),
        ridge_period_px,
        ridge_width_mad_px,
        ridge_period_mad_px,
        mean_ridge_width_px,
        mean_valley_width_px,
        mean_ridge_gray,
        mean_valley_gray,
        examined_locations,
        profiles,
    })
}

fn distance(a: Point, b: Point) -> f64 {
    (a.x() - b.x()).hypot(a.y() - b.y())
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        values[middle - 1] + (values[middle] - values[middle - 1]) / 2.0
    } else {
        values[middle]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn config() -> ExtractionConfig {
        ExtractionConfig {
            bootstrap_grid_locations: 128,
            ..ExtractionConfig::default()
        }
    }

    fn stripes(
        dimensions: (usize, usize),
        period: f64,
        dark_width: f64,
        angle: f64,
        phase: f64,
        noise: f64,
        illumination: bool,
    ) -> Capture {
        let (width, height) = dimensions;
        let (sin, cos) = angle.sin_cos();
        let mut random = 7_u64;
        let pixels = (0..width * height)
            .map(|i| {
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let grain = ((random >> 32) as f64 / f64::from(u32::MAX) - 0.5) * noise;
                let (x, y) = ((i % width) as f64, (i / width) as f64);
                let distance = ((x * cos + y * sin + phase + period / 2.0).rem_euclid(period)
                    - period / 2.0)
                    .abs();
                let intensity = 128.0
                    + 105.0 * ((distance - dark_width / 2.0) / 0.4).tanh()
                    + grain
                    + if illumination {
                        30.0 * (x / width as f64 - 0.5) + 15.0 * (y / height as f64 - 0.5)
                    } else {
                        0.0
                    };
                intensity.round().clamp(0.0, 255.0) as u8
            })
            .collect();
        Capture::new(width as u32, height as u32, pixels, None, None).unwrap()
    }

    #[test]
    fn measures_independent_width_and_period_over_angles_phase_noise_and_light() {
        for (angle, phase) in [(0.0, 0.0), (0.31, 0.7), (PI / 4.0, 2.3), (PI / 2.0, 0.4)] {
            let result = estimate(
                &stripes((180, 180), 20.0, 6.0, angle, phase, 6.0, true),
                &config(),
            )
            .unwrap();
            assert!(
                (result.ridge_width_px - 6.0).abs() < 0.7,
                "angle {angle}: width {}",
                result.ridge_width_px
            );
            assert!(
                (result.ridge_period_px - 20.0).abs() < 1.0,
                "angle {angle}: period {}",
                result.ridge_period_px
            );
            assert!((result.valley_width_px - 14.0).abs() < 1.2);
            assert!((result.ridge_period_px - 2.0 * result.ridge_width_px).abs() > 5.0);
            assert!(result.examined_locations <= 128 && !result.profiles.is_empty());
        }
    }

    #[test]
    fn three_complete_dark_ridges_are_required_and_edge_fragments_do_not_count() {
        for (dark_centers, accepted) in [
            (&[30.0, 50.0][..], false),
            (&[30.0, 50.0, 70.0][..], true),
            (&[0.0, 30.0, 50.0][..], false),
        ] {
            let pixels = (0..100 * 40)
                .map(|i| {
                    let distance = dark_centers
                        .iter()
                        .map(|&x| ((i % 100) as f64 - x).abs())
                        .fold(f64::INFINITY, f64::min);
                    if distance < 3.0 { 20 } else { 230 }
                })
                .collect();
            let input = Capture::new(100, 40, pixels, None, None).unwrap();
            let result = estimate(&input, &config());
            assert_eq!(
                result.is_ok(),
                accepted,
                "centers {dark_centers:?}: {result:?}"
            );
        }
    }

    #[test]
    fn unavailable_scale_never_uses_a_default_or_clamps_an_unsupported_period() {
        let blank = Capture::new(128, 128, vec![127; 128 * 128], None, None).unwrap();
        assert!(matches!(
            estimate(&blank, &config()),
            Err(FingerprintError::ScaleUnavailable(_))
        ));
        let masked = Capture::new(
            128,
            128,
            vec![127; 128 * 128],
            Some(vec![false; 128 * 128]),
            None,
        )
        .unwrap();
        assert!(matches!(
            estimate(&masked, &config()),
            Err(FingerprintError::ScaleUnavailable(_))
        ));
        for period in [2.0, 240.0] {
            let input = stripes((900, 128), period, period / 3.0, 0.0, 0.0, 0.0, false);
            assert!(
                matches!(
                    estimate(&input, &config()),
                    Err(FingerprintError::ScaleUnavailable(_))
                ),
                "period {period}"
            );
        }
    }

    #[test]
    fn source_pixel_geometry_and_measurements_scale_under_resizing() {
        let a = estimate(
            &stripes((160, 160), 16.0, 5.0, 0.21, 0.3, 0.0, false),
            &config(),
        )
        .unwrap();
        let b = estimate(
            &stripes((320, 320), 32.0, 10.0, 0.21, 0.6, 0.0, false),
            &config(),
        )
        .unwrap();
        assert!((b.ridge_period_px / a.ridge_period_px - 2.0).abs() < 0.08);
        assert!((b.ridge_width_px / a.ridge_width_px - 2.0).abs() < 0.15);
        for result in [&a, &b] {
            let widths: Vec<_> = result
                .profiles
                .iter()
                .flat_map(|p| p.ridges.iter().map(|r| distance(r.start, r.end)))
                .collect();
            assert_eq!(median(&mut widths.clone()), result.ridge_width_px);
        }
    }

    #[test]
    fn measures_the_supported_period_endpoints_without_a_scale_hint() {
        // At the lower supported period use a resolved, symmetric half-local-
        // contrast width. An underlying sub-two-pixel physical stripe need not
        // reach its continuous dark extremum at the sampled pixel phase.
        for (dimensions, period, width) in [((128, 96), 4.0, 2.0), ((800, 200), 128.0, 32.0)] {
            let result = estimate(
                &stripes(dimensions, period, width, 0.0, 0.3, 0.0, false),
                &config(),
            )
            .unwrap();
            assert!(
                (result.ridge_period_px - period).abs() < 0.03 * period,
                "period {period}: {}",
                result.ridge_period_px
            );
            assert!(
                (result.ridge_width_px - width).abs() < 0.15 * width,
                "width {width}: {}",
                result.ridge_width_px
            );
        }
    }

    #[test]
    fn undersampled_stripe_width_uses_observed_half_contrast_not_latent_thickness() {
        let input = stripes((128, 96), 4.0, 1.5, 0.0, 0.3, 0.0, false);
        // Independently tabulated source pixels for this analytic stripe. The
        // nearest sampled trough is43, not the latent continuous dark extreme.
        assert_eq!(&input.pixels()[..4], &[43, 220, 231, 115]);
        let pixels = [43.0, 220.0, 231.0, 115.0];
        // One centered site is x=63.5, so the refinement's half-step samples
        // begin at x=54.5 (phase2.5 modulo4). Construct one periodic profile
        // directly from those four source samples, without calling any scale
        // profile, smoothing, extrema or crossing implementation helpers.
        let sampled: [f64; 8] = std::array::from_fn(|i| {
            let x = 2.5 + 0.5 * i as f64;
            let left = x.floor() as usize;
            let fraction = x - left as f64;
            pixels[left % 4] * (1.0 - fraction) + pixels[(left + 1) % 4] * fraction
        });
        // σ=P/32=.125px=.25samples: truncate3σ gives the exact3-tap kernel.
        let neighbor = (-8.0_f64).exp();
        let observed: [f64; 8] = std::array::from_fn(|i| {
            (neighbor * sampled[(i + 7) % 8] + sampled[i] + neighbor * sampled[(i + 1) % 8])
                / (1.0 + 2.0 * neighbor)
        });
        let darkest = (0..8)
            .min_by(|&a, &b| observed[a].total_cmp(&observed[b]))
            .unwrap();
        let brightest = observed.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let half = (observed[darkest] + brightest) / 2.0;
        let value = |i: isize| observed[i.rem_euclid(8) as usize];
        let mut left = darkest as isize;
        while value(left) < half {
            left -= 1;
        }
        let mut right = darkest as isize;
        while value(right) < half {
            right += 1;
        }
        let a = left as f64 + (value(left) - half) / (value(left) - value(left + 1));
        let b = (right - 1) as f64 + (half - value(right - 1)) / (value(right) - value(right - 1));
        let expected = (b - a) * 0.5;
        assert!((expected - 1.720844863948994).abs() < 1e-12);
        let one = estimate(
            &input,
            &ExtractionConfig {
                bootstrap_grid_locations: 1,
                ..config()
            },
        )
        .unwrap();
        assert!((one.ridge_width_px - expected).abs() < 1e-8);
        assert!((one.ridge_period_px - 4.0).abs() < 1e-8);
        // Preserve the originally discovered multi-site phase case as well.
        // Its median samples several shifted half-pixel profiles and therefore
        // differs from both this one-site oracle and the unobservable latent1.5.
        let multiple = estimate(&input, &config()).unwrap();
        assert!((multiple.ridge_width_px - 1.8044967114337922).abs() < 1e-8);
        assert!((multiple.ridge_period_px - 4.0).abs() < 1e-8);
    }

    #[test]
    fn multiple_measured_thickness_groups_remain_in_the_output() {
        let source = (0..180 * 180)
            .map(|i| {
                let dark_width = if i / 180 < 90 { 4.0 } else { 10.0 };
                let distance = (((i % 180) as f64 + 10.0).rem_euclid(20.0) - 10.0).abs();
                (128.0 + 105.0 * ((distance - dark_width / 2.0) / 0.4).tanh())
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
            .collect();
        let input = Capture::new(180, 180, source, None, None).unwrap();
        let result = estimate(&input, &config()).unwrap();
        let mut widths: Vec<_> = result
            .profiles
            .iter()
            .flat_map(|p| p.ridges.iter().map(|r| distance(r.start, r.end)))
            .collect();
        assert!(widths.iter().any(|&w| w < 5.0));
        assert!(widths.iter().any(|&w| w > 9.0));
        assert_eq!(median(&mut widths), result.ridge_width_px);
    }

    #[test]
    fn a_mask_cut_is_not_bridged_and_hidden_values_do_not_change_measurements() {
        let original = stripes((160, 120), 20.0, 6.0, 0.0, 1.0, 0.0, false);
        let mut mask = vec![true; 160 * 120];
        for y in 0..120 {
            for x in 74..87 {
                mask[y * 160 + x] = false;
            }
        }
        let mut a = original.pixels().to_vec();
        let mut b = a.clone();
        for i in 0..a.len() {
            if !mask[i] {
                a[i] = 0;
                b[i] = 255;
            }
        }
        let first = Capture::new(160, 120, a, Some(mask.clone()), None).unwrap();
        let second = Capture::new(160, 120, b, Some(mask), None).unwrap();
        let a = estimate(&first, &config()).unwrap();
        let b = estimate(&second, &config()).unwrap();
        assert_eq!(a, b);
        for p in &a.profiles {
            let min = p
                .ridges
                .iter()
                .map(|r| r.start.x().min(r.end.x()))
                .fold(f64::INFINITY, f64::min);
            let max = p
                .ridges
                .iter()
                .map(|r| r.start.x().max(r.end.x()))
                .fold(f64::NEG_INFINITY, f64::max);
            assert!(max < 74.0 || min > 86.0, "triple crosses unobserved mask");
        }
    }

    #[test]
    fn all_confirmed_width_groups_contribute_to_the_median_and_mad() {
        let profile = |widths: [f64; 3]| ScaleProfile {
            ridges: std::array::from_fn(|i| {
                let center = 20.0 * i as f64;
                RidgeSpan {
                    start: Point::new(center - widths[i] / 2.0, 5.0).unwrap(),
                    end: Point::new(center + widths[i] / 2.0, 5.0).unwrap(),
                }
            }),
            ridge_mean_gray: [32.0; 3],
            valley_mean_gray: [224.0; 2],
        };
        let result = summarize(
            vec![profile([2.0, 2.0, 2.0]), profile([10.0, 10.0, 18.0])],
            2,
        )
        .unwrap();
        assert_eq!(result.ridge_width_px, 6.0);
        assert_eq!(result.ridge_width_mad_px, 4.0);
        assert_eq!(result.ridge_period_px, 20.0);
        assert_eq!(result.profiles.len(), 2);
        assert_eq!(result.mean_ridge_width_px, 44.0 / 6.0);
        assert_eq!(result.mean_valley_width_px, 13.0);
        assert_eq!(result.mean_ridge_gray, 32.0);
        assert_eq!(result.mean_valley_gray, 224.0);
    }

    #[test]
    fn photometry_is_length_weighted_while_width_means_count_every_span() {
        let profiles = [(2.0, 0.0, 100.0), (10.0, 200.0, 200.0)]
            .into_iter()
            .map(|(width, ridge_gray, valley_gray)| ScaleProfile {
                ridges: std::array::from_fn(|i| RidgeSpan {
                    start: Point::new(i as f64 * 20.0 - width / 2.0, 5.0).unwrap(),
                    end: Point::new(i as f64 * 20.0 + width / 2.0, 5.0).unwrap(),
                }),
                ridge_mean_gray: [ridge_gray; 3],
                valley_mean_gray: [valley_gray; 2],
            })
            .collect();
        let result = summarize(profiles, 2).unwrap();
        assert_eq!(result.mean_ridge_width_px, 6.0);
        assert_eq!(result.mean_valley_width_px, 14.0);
        assert!((result.mean_ridge_gray - 6000.0 / 36.0).abs() < 1e-12);
        assert!((result.mean_valley_gray - 7600.0 / 56.0).abs() < 1e-12);
        // Different thickness groups remain in both the mean and the median.
        assert_eq!(result.ridge_width_px, 6.0);
        assert_eq!(result.profiles.len(), 2);
    }

    #[test]
    fn budgets_are_validated_before_work_and_locations_cover_both_dimensions() {
        let input = stripes((160, 120), 20.0, 6.0, 0.0, 0.0, 0.0, false);
        let mut settings = config();
        settings.max_pixels = 10;
        assert!(matches!(
            estimate(&input, &settings),
            Err(FingerprintError::WorkLimit)
        ));
        settings = config();
        settings.bootstrap_grid_locations = 0;
        assert!(matches!(
            estimate(&input, &settings),
            Err(FingerprintError::InvalidConfig(_))
        ));
        settings = config();
        settings.bootstrap_grid_locations = 7;
        let result = estimate(&input, &settings).unwrap();
        assert!(result.examined_locations <= 7);
        assert!(result.profiles.iter().any(|p| p.ridges[1].start.y() < 60.0));
        assert!(result.profiles.iter().any(|p| p.ridges[1].start.y() > 60.0));
    }

    #[test]
    fn bootstrap_fallback_finds_a_small_patch_between_grid_sites() {
        let (width, height) = (240, 160);
        let mut pixels = vec![230; width * height];
        for y in 15..65 {
            for x in 12_usize..88 {
                if [30_usize, 50, 70]
                    .iter()
                    .any(|&center| x.abs_diff(center) < 3)
                {
                    pixels[y * width + x] = 20;
                }
            }
        }
        let capture = Capture::new(width as u32, height as u32, pixels, None, None).unwrap();
        let settings = ExtractionConfig {
            bootstrap_grid_locations: 1,
            ..config()
        };
        let result = estimate(&capture, &settings).unwrap();
        assert!(
            result.examined_locations > 1,
            "the one-site grid must miss this patch"
        );
        assert_eq!(result.profiles.len(), 1);
        assert!((result.ridge_period_px - 20.0).abs() < 0.5);
        assert!(
            result.profiles[0]
                .ridges
                .iter()
                .all(|ridge| ridge.start.y() < 65.0)
        );
    }
}
