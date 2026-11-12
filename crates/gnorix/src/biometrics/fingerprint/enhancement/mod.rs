//! Native, deterministic image enhancement using the measured ridge period.
//!
//! Local normalization and gradient structure tensors follow the conventional
//! orientation-driven fingerprint enhancement approach, e.g. Hong, Wan & Jain
//! (1998), doi:10.1109/34.709565. The directional three-line contrast below is
//! our own simple filter, not their Gabor filter or a SourceAFIS reproduction.

mod normalization;
mod response;
mod window;

use std::f64::consts::{FRAC_PI_2, PI};

pub use normalization::LocalPalette;
pub(super) use response::DirectionalResponse;
use window::{Integral, Window};

use super::{Capture, ExtractionConfig, FingerprintError, Preprocessed, RidgeMap};

const NUMERICAL_VARIANCE_FLOOR: f64 = 1e-12;

/// Dense source support for raw measurements, before ridge enhancement.
pub(super) struct MeasurementInput {
    pub(super) map: RidgeMap,
    pub(super) normal_angles: Vec<f32>,
    pub(super) normal_usable: Vec<bool>,
}

pub(super) fn measurement_input(
    capture: &Capture,
    config: &ExtractionConfig,
    bootstrap_period: f64,
) -> Result<MeasurementInput, FingerprintError> {
    config.validate()?;
    if capture.pixels().len() > config.max_pixels {
        return Err(FingerprintError::WorkLimit);
    }
    if !bootstrap_period.is_finite() || !(4.0..=128.0).contains(&bootstrap_period) {
        return Err(FingerprintError::ScaleUnavailable(
            "bootstrap period exceeds measurement support",
        ));
    }
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    let radius = (bootstrap_period / 2.0).ceil() as usize;
    let standardized = normalization::standardize(capture, radius, config.min_contrast)?;
    let tensor = orientation(&standardized, width, height, radius, config.min_coherence)?;
    let palette = normalization::estimate_palette(capture, &standardized, radius)?;
    let ridges = capture
        .pixels()
        .iter()
        .enumerate()
        .map(|(index, &gray)| {
            palette.usable[index]
                && f64::from(gray)
                    < (f64::from(palette.ridge_gray[index]) + f64::from(palette.valley_gray[index]))
                        / 2.0
        })
        .collect();
    let map = RidgeMap::new(width, height, ridges, palette.usable)?;
    let normal_angles = tensor
        .angles
        .into_iter()
        .map(|angle| (f64::from(angle) + FRAC_PI_2).rem_euclid(PI) as f32)
        .collect();
    Ok(MeasurementInput {
        map,
        normal_angles,
        normal_usable: tensor.usable,
    })
}

pub(super) fn preprocess(
    capture: &Capture,
    config: &ExtractionConfig,
    ridge_period_px: f64,
) -> Result<Preprocessed, FingerprintError> {
    config.validate()?;
    if capture.pixels().len() > config.max_pixels {
        return Err(FingerprintError::WorkLimit);
    }

    let width = capture.width() as usize;
    let height = capture.height() as usize;
    if !ridge_period_px.is_finite() || !(4.0..=128.0).contains(&ridge_period_px) {
        return Err(FingerprintError::ScaleUnavailable(
            "measured period exceeds filter support",
        ));
    }
    let radius = (ridge_period_px / 2.0).ceil() as usize;
    let orientation_input = normalization::standardize(capture, radius, config.min_contrast)?;
    let tensor = orientation(
        &orientation_input,
        width,
        height,
        radius,
        config.min_coherence,
    )?;
    let palette = normalization::estimate_palette(capture, &orientation_input, radius)?;
    drop(orientation_input);
    let local = normalization::apply_palette(capture, &palette);
    let filtered = denoise_intensity(&local, width, height, ridge_period_px / 20.0);
    let response = response::collect(&filtered, &tensor, width, height, ridge_period_px)?;
    let map = response.binarize(0.0, config.min_coherence)?;

    Ok(Preprocessed {
        palette,
        normalized: local.values,
        denoised: filtered.values,
        coherence: tensor.coherence,
        map,
        response,
    })
}

struct Normalized {
    values: Vec<f32>,
    usable: Vec<bool>,
}

/// Weak isotropic smoothing removes subridge grain before directional sampling.
/// Raw contrast, orientation and coherence decisions remain unchanged. Convolve
/// both values and supported weight: invalid values never enter the average,
/// and the original local usability mask is retained exactly.
fn denoise_intensity(local: &Normalized, width: usize, height: usize, sigma: f64) -> Normalized {
    let radius = (3.0 * sigma).ceil() as isize;
    let mut kernel: Vec<_> = (-radius..=radius)
        .map(|offset| (offset, (-0.5 * (offset as f64 / sigma).powi(2)).exp()))
        .collect();
    let total: f64 = kernel.iter().map(|&(_, weight)| weight).sum();
    for (_, weight) in &mut kernel {
        *weight /= total;
    }
    let mut horizontal = vec![0.0; local.values.len()];
    let mut supported = vec![0.0; local.values.len()];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            for &(offset, weight) in &kernel {
                let source = y * width + reflected_offset(x, offset, width);
                if local.usable[source] {
                    horizontal[index] += weight * f64::from(local.values[source]);
                    supported[index] += weight;
                }
            }
        }
    }
    let mut values = vec![0.0; local.values.len()];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if !local.usable[index] {
                continue;
            }
            let (mut numerator, mut denominator) = (0.0, 0.0);
            for &(offset, weight) in &kernel {
                let source = reflected_offset(y, offset, height) * width + x;
                numerator += weight * horizontal[source];
                denominator += weight * supported[source];
            }
            // A usable center contributes strictly positive weight.
            values[index] = (numerator / denominator) as f32;
        }
    }
    Normalized {
        values,
        usable: local.usable.clone(),
    }
}

fn reflected_offset(center: usize, offset: isize, length: usize) -> usize {
    if let Some(index) = center.checked_add_signed(offset)
        && index < length
    {
        return index;
    }
    // Half-sample reflection: -1 -> 0, length -> length-1. A wide integer also
    // covers a kernel reflecting multiple times over a one-pixel image.
    let folded = (center as i128 + offset as i128).rem_euclid(2 * length as i128);
    if folded < length as i128 {
        folded as usize
    } else {
        (2 * length as i128 - 1 - folded) as usize
    }
}

fn gradient(local: &Normalized, width: usize, height: usize, index: usize) -> Option<(f64, f64)> {
    let x = index % width;
    let y = index / width;
    if x == 0 || y == 0 || x + 1 == width || y + 1 == height {
        return None;
    }
    let neighbors = [index, index - 1, index + 1, index - width, index + width];
    if neighbors.iter().any(|&i| !local.usable[i]) {
        return None;
    }
    Some((
        f64::from(local.values[index + 1] - local.values[index - 1]) / 2.0,
        f64::from(local.values[index + width] - local.values[index - width]) / 2.0,
    ))
}

struct Orientation {
    angles: Vec<f32>,
    coherence: Vec<f32>,
    exact_coherence: Vec<f64>,
    observed: Vec<bool>,
    usable: Vec<bool>,
}

fn orientation(
    local: &Normalized,
    width: usize,
    height: usize,
    radius: usize,
    min_coherence: f64,
) -> Result<Orientation, FingerprintError> {
    let count = Integral::new(width, height, |i| {
        f64::from(gradient(local, width, height, i).is_some())
    })?;
    let xx = Integral::new(width, height, |i| {
        gradient(local, width, height, i).map_or(0.0, |(x, _)| x * x)
    })?;
    let xy = Integral::new(width, height, |i| {
        gradient(local, width, height, i).map_or(0.0, |(x, y)| x * y)
    })?;
    let yy = Integral::new(width, height, |i| {
        gradient(local, width, height, i).map_or(0.0, |(_, y)| y * y)
    })?;
    let mut field = Orientation {
        angles: vec![0.0; local.values.len()],
        coherence: vec![0.0; local.values.len()],
        exact_coherence: vec![0.0; local.values.len()],
        observed: vec![false; local.values.len()],
        usable: vec![false; local.values.len()],
    };
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let window = Window::around(x, y, width, height, radius);
            if count.sum(window) != window.count() as f64 {
                continue;
            }
            let (xx, xy, yy) = (xx.sum(window), xy.sum(window), yy.sum(window));
            let energy = xx + yy;
            if energy <= NUMERICAL_VARIANCE_FLOOR {
                continue;
            }
            let coherence = ((xx - yy).hypot(2.0 * xy) / energy).clamp(0.0, 1.0);
            field.coherence[index] = coherence as f32;
            field.exact_coherence[index] = coherence;
            field.observed[index] = true;
            // Retain the observed orientation before applying confidence so
            // threshold stress cases reuse the same source tensor and filter.
            field.angles[index] =
                ((2.0 * xy).atan2(xx - yy) / 2.0 + FRAC_PI_2).rem_euclid(PI) as f32;
            if coherence >= min_coherence {
                field.usable[index] = true;
            }
        }
    }
    Ok(field)
}

fn interpolate(local: &Normalized, width: usize, height: usize, x: f64, y: f64) -> Option<f64> {
    if x < 0.0 || y < 0.0 || x > (width - 1) as f64 || y > (height - 1) as f64 {
        return None;
    }
    let left = x.floor() as usize;
    let top = y.floor() as usize;
    let right = (left + 1).min(width - 1);
    let bottom = (top + 1).min(height - 1);
    let indices = [
        top * width + left,
        top * width + right,
        bottom * width + left,
        bottom * width + right,
    ];
    if indices.iter().any(|&i| !local.usable[i]) {
        return None;
    }
    let (dx, dy) = (x - left as f64, y - top as f64);
    let upper =
        f64::from(local.values[indices[0]]) * (1.0 - dx) + f64::from(local.values[indices[1]]) * dx;
    let lower =
        f64::from(local.values[indices[2]]) * (1.0 - dx) + f64::from(local.values[indices[3]]) * dx;
    Some(upper * (1.0 - dy) + lower * dy)
}

#[cfg(test)]
fn directional_ridges(
    local: &Normalized,
    field: &Orientation,
    width: usize,
    height: usize,
    period: f64,
) -> Result<RidgeMap, FingerprintError> {
    let mut ridges = vec![false; local.values.len()];
    let mut usable = vec![false; local.values.len()];
    let intervals = period.ceil() as usize;
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if !field.usable[index] {
                continue;
            }
            let (sin, cos) = f64::from(field.angles[index]).sin_cos();
            let mut lines = [0.0; 3];
            let mut complete = true;
            // Uniform samples at most one source pixel apart, repeated at the
            // center and half a period to each side. This avoids subridge
            // aliasing from a fixed sparse sample count. The supplied period
            // bounds work; no fitting, explicit gap repair, or resize occurs.
            for (line, offset) in [-0.5, 0.0, 0.5].into_iter().enumerate() {
                for sample in 0..=intervals {
                    let along = -period / 2.0 + sample as f64 * period / intervals as f64;
                    let across = offset * period;
                    let sx = x as f64 + along * cos - across * sin;
                    let sy = y as f64 + along * sin + across * cos;
                    if let Some(value) = interpolate(local, width, height, sx, sy) {
                        lines[line] += value;
                    } else {
                        complete = false;
                        break;
                    }
                }
                if !complete {
                    break;
                }
            }
            if complete {
                usable[index] = true;
                // Intensities are dark-negative: a ridge is darker than the
                // two neighboring half-period lines and darker than local mean.
                ridges[index] = lines[1] < 0.0 && lines[1] < (lines[0] + lines[2]) / 2.0;
            }
        }
    }
    RidgeMap::new(width, height, ridges, usable)
}

#[cfg(test)]
mod tests {
    use super::super::{NodeKind, graph, thinning::thin};
    use super::*;

    fn stripes(width: usize, height: usize, angle: f64) -> Vec<u8> {
        let (sin, cos) = angle.sin_cos();
        (0..width * height)
            .map(|i| {
                let phase = (i % width) as f64 * cos + (i / width) as f64 * sin;
                (128.0 - 90.0 * (2.0 * PI * phase / 12.0).cos()).round() as u8
            })
            .collect()
    }

    fn capture(width: usize, height: usize, pixels: Vec<u8>, mask: Option<Vec<bool>>) -> Capture {
        Capture::new(width as u32, height as u32, pixels, mask, None).unwrap()
    }

    #[test]
    fn blank_low_contrast_invalid_and_small_images_have_no_ridges() {
        let config = ExtractionConfig::default();
        let cases = [
            capture(80, 80, vec![127; 6400], None),
            capture(
                80,
                80,
                (0..6400).map(|i| 127 + (i % 2) as u8).collect(),
                None,
            ),
            capture(80, 80, stripes(80, 80, 0.0), Some(vec![false; 6400])),
            capture(1, 1, vec![255], None),
            capture(2, 2, vec![0, 255, 255, 0], None),
        ];
        for input in cases {
            let result = preprocess(&input, &config, 12.0).unwrap();
            assert!(result.map.ridges.iter().all(|&v| !v));
            assert!(result.normalized.iter().all(|v| v.is_finite()));
            assert!(result.denoised.iter().all(|v| v.is_finite()));
            assert!(result.coherence.iter().all(|v| v.is_finite()));
            assert!(thin(&result.map).unwrap().ridges.iter().all(|&v| !v));
        }
    }

    #[test]
    fn extracts_both_axis_directions_and_diagonal_dark_stripes() {
        let config = ExtractionConfig::default();
        for angle in [0.0, FRAC_PI_2, PI / 4.0] {
            let pixels = stripes(96, 96, angle);
            let result = preprocess(&capture(96, 96, pixels.clone(), None), &config, 12.0).unwrap();
            let accepted: Vec<_> = result
                .map
                .ridges
                .iter()
                .enumerate()
                .filter_map(|(i, &ridge)| ridge.then_some(i))
                .collect();
            assert!(
                accepted.len() > 100,
                "angle {angle}: {} ridges",
                accepted.len()
            );
            let average =
                accepted.iter().map(|&i| f64::from(pixels[i])).sum::<f64>() / accepted.len() as f64;
            assert!(
                average < 110.0,
                "accepted ridges must follow dark lines: {average}"
            );
            assert!(accepted.iter().all(|&i| result.coherence[i] > 0.8));
            let skeleton = thin(&result.map).unwrap();
            assert!(skeleton.ridges.iter().any(|&v| v));
            assert!(skeleton.ridges.iter().filter(|&&v| v).count() < accepted.len());
        }
    }

    #[test]
    fn mask_hole_values_cannot_influence_any_output_or_be_bridged() {
        let (width, height) = (100, 100);
        let mut mask = vec![true; width * height];
        for y in 30..70 {
            for x in 45..55 {
                mask[y * width + x] = false;
            }
        }
        let mut first = stripes(width, height, 0.0);
        let mut second = first.clone();
        for i in 0..first.len() {
            if !mask[i] {
                first[i] = 0;
                second[i] = 255;
            }
        }
        let config = ExtractionConfig::default();
        let a = preprocess(
            &capture(width, height, first, Some(mask.clone())),
            &config,
            12.0,
        )
        .unwrap();
        let b = preprocess(
            &capture(width, height, second, Some(mask.clone())),
            &config,
            12.0,
        )
        .unwrap();
        assert_eq!(a.normalized, b.normalized);
        assert_eq!(a.denoised, b.denoised);
        assert_eq!(a.coherence, b.coherence);
        assert_eq!(a.palette, b.palette);
        assert_eq!(a.map.ridges, b.map.ridges);
        assert_eq!(a.map.usable, b.map.usable);
        assert_eq!(a.map.boundary_origin, b.map.boundary_origin);
        let skeleton = thin(&a.map).unwrap();
        for (index, &valid) in mask.iter().enumerate() {
            if !valid {
                assert!(!a.map.usable[index]);
                assert!(!a.map.ridges[index]);
                assert!(!skeleton.ridges[index]);
            }
        }
    }

    #[test]
    fn masked_gaussian_preserves_constants_and_ignores_unsupported_values() {
        let mut first = Normalized {
            values: vec![2.0; 9],
            usable: vec![true; 9],
        };
        first.usable[4] = false;
        first.values[4] = -1_000_000.0;
        let mut second = Normalized {
            values: first.values.clone(),
            usable: first.usable.clone(),
        };
        second.values[4] = 1_000_000.0;
        // This kernel reflects repeatedly over the tiny image, exercising both
        // the boundary convention and supported-weight normalization.
        let a = denoise_intensity(&first, 3, 3, 6.4);
        let b = denoise_intensity(&second, 3, 3, 6.4);
        assert_eq!(a.values, b.values);
        assert_eq!(a.usable, first.usable);
        assert_eq!(a.values[4], 0.0);
        assert!(
            a.values
                .iter()
                .enumerate()
                .all(|(i, &v)| i == 4 || (v - 2.0).abs() < 1e-6)
        );
    }

    #[test]
    fn local_palette_changes_actual_ridge_detection_for_identical_gray() {
        let (width, height) = (96, 48);
        let pixels = (0..width * height)
            .map(|index| {
                let x = index % width;
                match (x < 48, x % 12 < 4) {
                    (true, true) => 20,
                    (true, false) | (false, true) => 100,
                    (false, false) => 220,
                }
            })
            .collect();
        let input = capture(width, height, pixels, None);
        let output = preprocess(&input, &ExtractionConfig::default(), 12.0).unwrap();
        let (light, dark) = (24 * width + 20, 24 * width + 60);

        assert_eq!(input.pixels()[light], input.pixels()[dark]);
        assert!(output.map.usable[light] && output.map.usable[dark]);
        assert!(!output.map.ridges[light]);
        assert!(output.map.ridges[dark]);
        assert!(output.normalized[light] > 0.0 && output.normalized[dark] < 0.0);
        assert!(output.coherence[light] > 0.99 && output.coherence[dark] > 0.99);
    }

    #[test]
    fn raw_measurement_input_keeps_a_bright_interruption_and_source_normals() {
        let (width, height) = (96, 64);
        let mut pixels: Vec<_> = (0..width * height)
            .map(|index| if index % width % 12 < 4 { 20 } else { 230 })
            .collect();
        let interruption = 32 * width + 25;
        pixels[interruption] = 230;
        let input = capture(width, height, pixels, None);
        let measured = measurement_input(&input, &ExtractionConfig::default(), 12.0).unwrap();
        assert!(measured.map.usable[interruption]);
        assert!(!measured.map.ridges[interruption]);
        assert!(measured.map.ridges[interruption - width]);
        let intact = 32 * width + 49;
        assert!(measured.normal_usable[intact]);
        let normal = f64::from(measured.normal_angles[intact]);
        assert!(normal.sin().abs() < 1e-5);
        assert_eq!(measured.normal_angles.len(), width * height);
        assert_eq!(measured.normal_usable.len(), width * height);
    }

    fn noisy_pixels(
        width: usize,
        height: usize,
        intensity: impl Fn(f64, f64) -> f64,
        sigma: f64,
    ) -> Vec<u8> {
        let mut random = 42_u64;
        let mut uniform = || {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((random >> 11) as f64 + 0.5) / ((1_u64 << 53) as f64)
        };
        (0..width * height)
            .map(|i| {
                let grain = (-2.0 * uniform().ln()).sqrt() * (2.0 * PI * uniform()).cos();
                (intensity((i % width) as f64, (i / width) as f64) + sigma * grain)
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
            .collect()
    }

    fn distance_to_segment(x: f64, y: f64, a: (f64, f64), b: (f64, f64)) -> f64 {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let fraction = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - a.0 - fraction * dx).hypot(y - a.1 - fraction * dy)
    }

    fn phantom(distance: impl Fn(f64, f64) -> f64) -> Capture {
        capture(
            240,
            240,
            noisy_pixels(
                240,
                240,
                |x, y| 215.0 - 170.0 * (-0.5 * (distance(x, y) / 1.8).powi(2)).exp(),
                15.0,
            ),
            None,
        )
    }

    fn probes_connected(map: &RidgeMap, probes: &[(usize, usize)]) -> bool {
        let near = |index: usize, point: (usize, usize)| {
            ((index % map.width) as f64 - point.0 as f64)
                .hypot((index / map.width) as f64 - point.1 as f64)
                <= 6.0
        };
        let Some(start) = (0..map.ridges.len()).find(|&i| map.ridges[i] && near(i, probes[0]))
        else {
            return false;
        };
        let mut visited = vec![false; map.ridges.len()];
        let mut pending = vec![start];
        visited[start] = true;
        while let Some(index) = pending.pop() {
            for next in
                neighbor_indices(map.width, map.height, index % map.width, index / map.width)
                    .into_iter()
                    .flatten()
            {
                if map.ridges[next] && !visited[next] {
                    visited[next] = true;
                    pending.push(next);
                }
            }
        }
        probes
            .iter()
            .all(|&p| (0..map.ridges.len()).any(|i| visited[i] && near(i, p)))
    }

    #[test]
    fn noisy_parallel_ridges_do_not_acquire_interior_bifurcations() {
        for angle in [0.0, PI / 4.0] {
            let (sin, cos) = angle.sin_cos();
            let pixels = noisy_pixels(
                240,
                240,
                |x, y| 128.0 - 85.0 * (2.0 * PI * (x * cos + y * sin + 4.3) / 12.0).cos(),
                18.0,
            );
            let prepared = preprocess(
                &capture(240, 240, pixels, None),
                &ExtractionConfig::default(),
                12.0,
            )
            .unwrap();
            let skeleton = thin(&prepared.map).unwrap();
            let graph = graph::trace(&skeleton).unwrap();
            let false_junctions = graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == NodeKind::Junction
                        && (24.0..216.0).contains(&node.position.x())
                        && (24.0..216.0).contains(&node.position.y())
                })
                .count();
            assert_eq!(false_junctions, 0, "angle {angle}");
        }
    }

    #[test]
    fn noisy_planted_y_retains_one_central_junction_and_three_connected_arms() {
        let input = phantom(|x, y| {
            [
                ((120.0, 200.0), (120.0, 120.0)),
                ((120.0, 120.0), (72.0, 40.0)),
                ((120.0, 120.0), (168.0, 40.0)),
            ]
            .iter()
            .map(|&(a, b)| distance_to_segment(x, y, a, b))
            .fold(f64::INFINITY, f64::min)
        });
        let prepared = preprocess(&input, &ExtractionConfig::default(), 12.0).unwrap();
        let skeleton = thin(&prepared.map).unwrap();
        assert!(probes_connected(
            &skeleton,
            &[(120, 175), (88, 67), (152, 67)]
        ));
        let graph = graph::trace(&skeleton).unwrap();
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Junction
                    && (n.position.x() - 120.0).hypot(n.position.y() - 120.0) <= 12.0)
                .count(),
            1
        );
    }

    #[test]
    fn noisy_large_u_retains_its_curve_and_both_observed_endings() {
        let input = phantom(|x, y| {
            let arms = distance_to_segment(x, y, (75.0, 40.0), (75.0, 120.0))
                .min(distance_to_segment(x, y, (165.0, 40.0), (165.0, 120.0)));
            if y >= 120.0 {
                arms.min(((x - 120.0).hypot(y - 120.0) - 45.0).abs())
            } else {
                arms
            }
        });
        let prepared = preprocess(&input, &ExtractionConfig::default(), 12.0).unwrap();
        let skeleton = thin(&prepared.map).unwrap();
        assert!(probes_connected(
            &skeleton,
            &[(75, 60), (120, 165), (165, 60)]
        ));
        let graph = graph::trace(&skeleton).unwrap();
        for x in [75.0, 165.0] {
            assert!(graph.nodes.iter().any(|n| n.kind == NodeKind::Ending
                && (n.position.x() - x).hypot(n.position.y() - 40.0) <= 12.0));
        }
    }

    #[test]
    fn noisy_ring_retains_its_true_bounded_hole() {
        let input = phantom(|x, y| ((x - 120.0).hypot(y - 120.0) - 56.0).abs());
        let prepared = preprocess(&input, &ExtractionConfig::default(), 12.0).unwrap();
        let skeleton = thin(&prepared.map).unwrap();
        assert!(probes_connected(
            &skeleton,
            &[(64, 120), (120, 64), (176, 120), (120, 176)]
        ));
        let mut visited = vec![false; 240 * 240];
        let mut pending = vec![120 * 240 + 120];
        visited[120 * 240 + 120] = true;
        while let Some(index) = pending.pop() {
            let (x, y) = (index % 240, index / 240);
            assert!(x > 0 && y > 0 && x < 239 && y < 239, "true ring was opened");
            for (direction, next) in neighbor_indices(240, 240, x, y).into_iter().enumerate() {
                if direction % 2 == 0
                    && let Some(next) = next
                    && !skeleton.ridges[next]
                    && !visited[next]
                {
                    visited[next] = true;
                    pending.push(next);
                }
            }
        }
    }

    #[test]
    fn validates_config_and_work_limit_before_processing() {
        let input = capture(20, 20, stripes(20, 20, 0.0), None);
        let mut config = ExtractionConfig {
            max_pixels: 399,
            ..ExtractionConfig::default()
        };
        assert!(matches!(
            preprocess(&input, &config, 12.0),
            Err(FingerprintError::WorkLimit)
        ));
        config.max_pixels = 400;
        config.min_contrast = f64::NAN;
        assert!(matches!(
            preprocess(&input, &config, 12.0),
            Err(FingerprintError::InvalidConfig(_))
        ));
    }

    // Independent eight-neighbor traversal for connectivity assertions.
    fn neighbor_indices(width: usize, height: usize, x: usize, y: usize) -> [Option<usize>; 8] {
        // Clockwise from north, matching the Zhang-Suen p2..p9 convention.
        [
            (0, -1),
            (1, -1),
            (1, 0),
            (1, 1),
            (0, 1),
            (-1, 1),
            (-1, 0),
            (-1, -1),
        ]
        .map(|(dx, dy)| {
            let nx = x.checked_add_signed(dx)?;
            let ny = y.checked_add_signed(dy)?;
            (nx < width && ny < height).then_some(ny * width + nx)
        })
    }
}
