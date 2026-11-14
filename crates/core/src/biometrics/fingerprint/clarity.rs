//! Diagnostic source-image ridge/valley separability, without extraction gates.

mod patch;

use std::fmt;

use super::{Capture, FingerprintError, raster};
use crate::biometrics::geometry::Point;
use patch::{Components, Sampler};

/// Local source evidence in [0, 1], not recognition confidence or anatomy.
/// Unsupported slots contain zero; `supported` distinguishes them from measured
/// zero clarity. Values interpolate the products measured at grid anchors;
/// components are interpolated independently for inspection.
#[derive(Clone, PartialEq)]
pub struct ClarityMap {
    width: usize,
    height: usize,
    pub values: Vec<f32>,
    pub contrast: Vec<f32>,
    pub separation: Vec<f32>,
    pub repetition: Vec<f32>,
    pub supported: Vec<bool>,
    pub period_px: f64,
    pub grid_spacing_px: usize,
}

impl fmt::Debug for ClarityMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClarityMap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("period_px", &self.period_px)
            .field("grid_spacing_px", &self.grid_spacing_px)
            .field("rasters", &"[REDACTED]")
            .finish()
    }
}

impl ClarityMap {
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }

    /// Bilinear clarity at a source coordinate. Every participating corner must
    /// be supported; missing evidence is never replaced by zero or extrapolated.
    pub fn sample(&self, x: f64, y: f64) -> Option<f64> {
        let cell = self.cell(x, y)?;
        let (tx, ty) = (x - cell.x as f64, y - cell.y as f64);
        let weights = [
            (1.0 - tx) * (1.0 - ty),
            tx * (1.0 - ty),
            (1.0 - tx) * ty,
            tx * ty,
        ];
        let mut value = 0.0;
        for (index, weight) in cell.indices.into_iter().zip(weights) {
            if weight != 0.0 {
                value += weight * self.observed_value(index)?;
            }
        }
        Some(value.clamp(0.0, 1.0))
    }

    /// Exact mean of the map's piecewise-bilinear interpolant along a segment.
    /// Each intersected cell must be supported, including tiny oblique corner
    /// clips. This integrates the diagnostic map, not original-image brightness.
    pub fn segment_mean(&self, start: Point, end: Point) -> Option<f64> {
        self.cell(start.x(), start.y())?;
        self.cell(end.x(), end.y())?;
        if start == end {
            return self.sample(start.x(), start.y());
        }
        let (dx, dy) = (end.x() - start.x(), end.y() - start.y());
        let (mut mean, mut correction) = (0.0, 0.0);
        raster::visit_cells(start, end, self.width, self.height, |interval| {
            let area = self.integrate_cell(
                self.grid_cell(interval.x, interval.y),
                start,
                (dx, dy),
                (interval.start, interval.end),
            )?;
            let adjusted = area - correction;
            let next = mean + adjusted;
            correction = (next - mean) - adjusted;
            mean = next;
            Some(())
        })?;
        Some(mean.clamp(0.0, 1.0))
    }

    fn integrate_cell(
        &self,
        cell: Cell,
        start: Point,
        delta: (f64, f64),
        interval: (f64, f64),
    ) -> Option<f64> {
        let values = self.cell_values(cell)?;
        let sample = |t: f64| {
            let x = (start.x() + t * delta.0 - cell.x as f64).clamp(0.0, 1.0);
            let y = (start.y() + t * delta.1 - cell.y as f64).clamp(0.0, 1.0);
            bilinear(values, x, y)
        };
        let middle = interval.0 + (interval.1 - interval.0) / 2.0;
        Some(
            (interval.1 - interval.0)
                * (sample(interval.0) + 4.0 * sample(middle) + sample(interval.1))
                / 6.0,
        )
    }

    fn cell_values(&self, cell: Cell) -> Option<[f64; 4]> {
        let values = cell.indices.map(|index| self.observed_value(index));
        Some([values[0]?, values[1]?, values[2]?, values[3]?])
    }

    fn observed_value(&self, index: usize) -> Option<f64> {
        if !self.supported.get(index).copied()? {
            return None;
        }
        let value = f64::from(*self.values.get(index)?);
        (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
    }

    fn cell(&self, x: f64, y: f64) -> Option<Cell> {
        if self.width == 0
            || self.height == 0
            || !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x > (self.width - 1) as f64
            || y > (self.height - 1) as f64
        {
            return None;
        }
        let (left, top) = (x.floor() as usize, y.floor() as usize);
        Some(self.grid_cell(left, top))
    }

    fn grid_cell(&self, left: usize, top: usize) -> Cell {
        let (right, bottom) = (
            (left + 1).min(self.width - 1),
            (top + 1).min(self.height - 1),
        );
        Cell {
            x: left,
            y: top,
            indices: [
                top * self.width + left,
                top * self.width + right,
                bottom * self.width + left,
                bottom * self.width + right,
            ],
        }
    }
}

#[derive(Clone, Copy)]
struct Cell {
    x: usize,
    y: usize,
    indices: [usize; 4],
}

pub(super) fn assess(capture: &Capture, period_px: f64) -> Result<ClarityMap, FingerprintError> {
    if !period_px.is_finite() || !(4.0..=128.0).contains(&period_px) {
        return Err(FingerprintError::InvalidConfig(
            "clarity period must be between 4 and 128 pixels",
        ));
    }
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    let spacing = (period_px / 4.0).floor().max(1.0) as usize;
    let xs = anchors(width, spacing);
    let ys = anchors(height, spacing);
    let sampler = Sampler::new(capture, period_px, xs.len() * ys.len())?;
    let mut measured = Vec::with_capacity(xs.len() * ys.len());
    for &y in &ys {
        for &x in &xs {
            measured.push(sampler.measure(x, y));
        }
    }
    let count = capture.pixels().len();
    let mut map = ClarityMap {
        width,
        height,
        values: vec![0.0; count],
        contrast: vec![0.0; count],
        separation: vec![0.0; count],
        repetition: vec![0.0; count],
        supported: vec![false; count],
        period_px,
        grid_spacing_px: spacing,
    };
    interpolate_anchors(&mut map, capture, &xs, &ys, &measured);
    Ok(map)
}

fn anchors(length: usize, step: usize) -> Vec<usize> {
    let mut positions: Vec<_> = (0..length).step_by(step).collect();
    if positions.last() != Some(&(length - 1)) {
        positions.push(length - 1);
    }
    positions
}

#[derive(Clone, Copy)]
struct Blend {
    left: usize,
    right: usize,
    fraction: f64,
}

fn blends(length: usize, positions: &[usize]) -> Vec<Blend> {
    let mut left = 0;
    (0..length)
        .map(|position| {
            while left + 1 < positions.len() && position >= positions[left + 1] {
                left += 1;
            }
            let right = (left + 1).min(positions.len() - 1);
            let fraction = if left == right {
                0.0
            } else {
                (position - positions[left]) as f64 / (positions[right] - positions[left]) as f64
            };
            Blend {
                left,
                right,
                fraction,
            }
        })
        .collect()
}

fn interpolate_anchors(
    map: &mut ClarityMap,
    capture: &Capture,
    xs: &[usize],
    ys: &[usize],
    measured: &[Option<Components>],
) {
    let x_blends = blends(map.width, xs);
    let y_blends = blends(map.height, ys);
    for (y, vertical) in y_blends.iter().enumerate() {
        for (x, horizontal) in x_blends.iter().enumerate() {
            let index = y * map.width + x;
            if capture.valid_mask().is_some_and(|mask| !mask[index]) {
                continue;
            }
            let corners = [
                vertical.left * xs.len() + horizontal.left,
                vertical.left * xs.len() + horizontal.right,
                vertical.right * xs.len() + horizontal.left,
                vertical.right * xs.len() + horizontal.right,
            ];
            let (dx, dy) = (horizontal.fraction, vertical.fraction);
            let weights = [
                (1.0 - dx) * (1.0 - dy),
                dx * (1.0 - dy),
                (1.0 - dx) * dy,
                dx * dy,
            ];
            let mut value = Components::default();
            let mut clarity = 0.0;
            let mut supported = true;
            for (corner, weight) in corners.into_iter().zip(weights) {
                if weight == 0.0 {
                    continue;
                }
                let Some(part) = measured[corner] else {
                    supported = false;
                    break;
                };
                value.contrast += weight * part.contrast;
                value.separation += weight * part.separation;
                value.repetition += weight * part.repetition;
                clarity += weight * part.contrast * part.separation * part.repetition;
            }
            if supported {
                map.contrast[index] = value.contrast.clamp(0.0, 1.0) as f32;
                map.separation[index] = value.separation.clamp(0.0, 1.0) as f32;
                map.repetition[index] = value.repetition.clamp(0.0, 1.0) as f32;
                map.values[index] = clarity.clamp(0.0, 1.0) as f32;
                map.supported[index] = true;
            }
        }
    }
}

fn bilinear(values: [f64; 4], x: f64, y: f64) -> f64 {
    (values[0] * (1.0 - x) + values[1] * x) * (1.0 - y)
        + (values[2] * (1.0 - x) + values[3] * x) * y
}

#[cfg(test)]
mod tests;
