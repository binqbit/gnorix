//! Supported intensity smoothing and bilinear sampling.

use super::Normalized;

/// Weak isotropic smoothing removes subridge grain before directional sampling.
/// Raw contrast, orientation and coherence decisions remain unchanged. Convolve
/// both values and supported weight: invalid values never enter the average,
/// and the original local usability mask is retained exactly.
pub(super) fn denoise_intensity(
    local: &Normalized,
    width: usize,
    height: usize,
    sigma: f64,
) -> Normalized {
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

pub(super) fn interpolate(
    local: &Normalized,
    width: usize,
    height: usize,
    x: f64,
    y: f64,
) -> Option<f64> {
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
