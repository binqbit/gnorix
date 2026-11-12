//! Provisional bootstrap and diagnostics from complete grayscale ridge sequences.
//!
//! The fixed-pixel pyramid bootstraps orientation without a known ridge period.
//! Perpendicular projections follow the general approach used in Hong, Wan &
//! Jain (1998), doi:10.1109/34.709565; this implementation, its scale search and
//! half-local-contrast widths are our own adaptation, not a paper reproduction.

mod pyramid;
mod sampling;

use pyramid::Image;

pub(super) use sampling::sample_width;
use sampling::{Span, dark_spans, profile, smooth_runs};

use super::{Capture, ExtractionConfig, FingerprintError, photometry};
use crate::biometrics::geometry::Point;

const MIN_PERIOD: f64 = 4.0;
const MAX_PERIOD: f64 = 128.0;
const MAX_PROFILE_SAMPLES: usize = 1_153;

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
    if capture.pixels().len() > config.max_pixels {
        return Err(FingerprintError::WorkLimit);
    }

    let pyramid = pyramid::build(capture);
    let (mut profiles, mut examined) = scan_grid(capture, &pyramid, config)?;
    if profiles.is_empty() {
        let (fallback, searched) = find_initial_support(capture, &pyramid, config, examined)?;
        profiles.extend(fallback);
        examined = searched;
    }
    summarize(profiles, examined)
}

fn scan_grid(
    capture: &Capture,
    pyramid: &[Image],
    config: &ExtractionConfig,
) -> Result<(Vec<ScaleProfile>, usize), FingerprintError> {
    let source = &pyramid[0];
    let budget = config.bootstrap_grid_locations.min(capture.pixels().len());
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
            if let Some(profile) = find_profile(capture, pyramid, x, y, config)? {
                profiles.push(profile);
            }
        }
    }
    Ok((profiles, columns * rows))
}

/// Sparse initialization can miss a small valid patch. The exhaustive fallback
/// finds initial support only; it never replaces the full traversal's width unit.
fn find_initial_support(
    capture: &Capture,
    pyramid: &[Image],
    config: &ExtractionConfig,
    mut examined: usize,
) -> Result<(Option<ScaleProfile>, usize), FingerprintError> {
    let source = &pyramid[0];
    for index in 0..capture.pixels().len() {
        if !has_local_gradient(source, index) {
            continue;
        }
        examined = examined.checked_add(1).ok_or(FingerprintError::WorkLimit)?;
        if let Some(profile) = find_profile(
            capture,
            pyramid,
            (index % source.width) as f64,
            (index / source.width) as f64,
            config,
        )? {
            return Ok((Some(profile), examined));
        }
    }
    Ok((None, examined))
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
mod tests;
