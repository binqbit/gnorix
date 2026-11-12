//! Source-supported structure tensors and axial ridge orientation.

use std::f64::consts::{FRAC_PI_2, PI};

use super::window::{Integral, Window};
use super::{FingerprintError, NUMERICAL_VARIANCE_FLOOR, Normalized};

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

pub(super) struct Orientation {
    pub(super) angles: Vec<f32>,
    pub(super) coherence: Vec<f32>,
    pub(super) exact_coherence: Vec<f64>,
    pub(super) observed: Vec<bool>,
    pub(super) usable: Vec<bool>,
}

pub(super) fn orientation(
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
