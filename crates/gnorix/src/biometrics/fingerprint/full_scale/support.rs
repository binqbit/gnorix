//! Rasterized union of source-confirmed three-ridge regions.

use super::{Capture, FingerprintError, ScaleProfile, distance};

/// Rotated ribbons contribute row intervals to a difference image. Union work
/// is O(ribbon height), followed by one O(pixels) scan, rather than repeatedly
/// painting O(period²) pixels for every adjacent confirmed link.
pub(super) struct RegionRaster {
    width: usize,
    height: usize,
    stride: usize,
    difference: Vec<i32>,
}

impl RegionRaster {
    pub(super) fn new(width: usize, height: usize) -> Result<Self, FingerprintError> {
        let stride = width.checked_add(1).ok_or(FingerprintError::WorkLimit)?;
        let count = stride
            .checked_mul(height)
            .ok_or(FingerprintError::WorkLimit)?;
        Ok(Self {
            width,
            height,
            stride,
            difference: vec![0; count],
        })
    }

    pub(super) fn add(&mut self, profile: &ScaleProfile) -> Result<(), FingerprintError> {
        let (start, end) = (profile.ridges[0].start, profile.ridges[2].end);
        let extent = distance(start, end);
        let centers = profile.ridges.each_ref().map(|span| {
            (
                (span.start.x() + span.end.x()) / 2.0,
                (span.start.y() + span.end.y()) / 2.0,
            )
        });
        let period = ((centers[1].0 - centers[0].0).hypot(centers[1].1 - centers[0].1)
            + (centers[2].0 - centers[1].0).hypot(centers[2].1 - centers[1].1))
            / 2.0;
        if !extent.is_finite() || extent <= 0.0 || !period.is_finite() || period <= 0.0 {
            return Err(FingerprintError::InvalidCurve(
                "confirmation ribbon must have finite positive extent",
            ));
        }
        let (nx, ny) = (
            (end.x() - start.x()) / extent,
            (end.y() - start.y()) / extent,
        );
        let (tx, ty) = (-ny, nx);
        let (cx, cy) = ((start.x() + end.x()) / 2.0, (start.y() + end.y()) / 2.0);
        let (half_normal, half_tangent) = (extent / 2.0, period / 2.0);
        let epsilon = 32.0 * f64::EPSILON * (cx.abs() + cy.abs() + extent + period + 1.0);
        let vertical = ny.abs() * half_normal + ty.abs() * half_tangent;
        let low_y = (cy - vertical - epsilon).ceil().max(0.0);
        let high_y = (cy + vertical + epsilon)
            .floor()
            .min((self.height - 1) as f64);
        if high_y < low_y {
            return Ok(());
        }
        for y in low_y as usize..=high_y as usize {
            let dy = y as f64 - cy;
            let Some((a, b)) = axis_interval(nx, ny, half_normal, dy, epsilon) else {
                continue;
            };
            let Some((c, d)) = axis_interval(tx, ty, half_tangent, dy, epsilon) else {
                continue;
            };
            let low = (cx + a.max(c) - epsilon).ceil().max(0.0);
            let high = (cx + b.min(d) + epsilon)
                .floor()
                .min((self.width - 1) as f64);
            if high < low {
                continue;
            }
            let start = y * self.stride + low as usize;
            let end = y * self.stride + high as usize + 1;
            self.difference[start] = self.difference[start]
                .checked_add(1)
                .ok_or(FingerprintError::WorkLimit)?;
            self.difference[end] = self.difference[end]
                .checked_sub(1)
                .ok_or(FingerprintError::WorkLimit)?;
        }
        Ok(())
    }

    pub(super) fn finish(self, capture: &Capture) -> Vec<bool> {
        let mut support = vec![false; self.width * self.height];
        for y in 0..self.height {
            let mut active = 0_i64;
            for x in 0..self.width {
                active += i64::from(self.difference[y * self.stride + x]);
                let index = y * self.width + x;
                support[index] = active > 0 && capture.valid_mask().is_none_or(|mask| mask[index]);
            }
        }
        support
    }
}

fn axis_interval(a: f64, b: f64, half: f64, dy: f64, epsilon: f64) -> Option<(f64, f64)> {
    if a.abs() <= 8.0 * f64::EPSILON {
        return ((b * dy).abs() <= half + epsilon).then_some((f64::NEG_INFINITY, f64::INFINITY));
    }
    let (first, last) = ((-half - b * dy) / a, (half - b * dy) / a);
    Some((first.min(last), first.max(last)))
}
