//! Native, deterministic image enhancement using the measured ridge period.
//!
//! Local normalization and gradient structure tensors follow the conventional
//! orientation-driven fingerprint enhancement approach, e.g. Hong, Wan & Jain
//! (1998), doi:10.1109/34.709565. The directional three-line contrast below is
//! our own simple filter, not their Gabor filter or a SourceAFIS reproduction.

mod intensity;
mod normalization;
mod orientation;
mod response;
mod window;

use std::f64::consts::{FRAC_PI_2, PI};

use intensity::denoise_intensity;
pub use normalization::LocalPalette;
use orientation::orientation;
pub(super) use response::DirectionalResponse;

use super::{Capture, ExtractionConfig, FingerprintError, RidgeMap};

const NUMERICAL_VARIANCE_FLOOR: f64 = 1e-12;

pub(super) struct Preprocessed {
    pub(super) palette: LocalPalette,
    pub(super) normalized: Vec<f32>,
    pub(super) denoised: Vec<f32>,
    pub(super) coherence: Vec<f32>,
    pub(super) map: RidgeMap,
    pub(super) response: DirectionalResponse,
}

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

#[cfg(test)]
mod tests;
