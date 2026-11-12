//! Sliding-window statistics shared by normalization and orientation.

use super::FingerprintError;

/// Summed-area statistics keep neighborhood work independent of ridge period.
pub(super) struct Integral {
    stride: usize,
    sums: Vec<f64>,
}

impl Integral {
    pub(super) fn new(
        width: usize,
        height: usize,
        mut value: impl FnMut(usize) -> f64,
    ) -> Result<Self, FingerprintError> {
        let stride = width.checked_add(1).ok_or(FingerprintError::WorkLimit)?;
        let count = height
            .checked_add(1)
            .and_then(|rows| rows.checked_mul(stride))
            .ok_or(FingerprintError::WorkLimit)?;
        let mut sums = vec![0.0; count];
        for y in 0..height {
            let mut row_sum = 0.0;
            for x in 0..width {
                row_sum += value(y * width + x);
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + row_sum;
            }
        }
        Ok(Self { stride, sums })
    }

    pub(super) fn sum(&self, area: Window) -> f64 {
        self.sums[area.bottom * self.stride + area.right]
            - self.sums[area.top * self.stride + area.right]
            - self.sums[area.bottom * self.stride + area.left]
            + self.sums[area.top * self.stride + area.left]
    }
}

#[derive(Clone, Copy)]
pub(super) struct Window {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

impl Window {
    pub(super) fn around(x: usize, y: usize, width: usize, height: usize, radius: usize) -> Self {
        Self {
            left: x.saturating_sub(radius),
            top: y.saturating_sub(radius),
            right: x.saturating_add(radius).saturating_add(1).min(width),
            bottom: y.saturating_add(radius).saturating_add(1).min(height),
        }
    }

    pub(super) fn count(self) -> usize {
        (self.right - self.left) * (self.bottom - self.top)
    }
}
