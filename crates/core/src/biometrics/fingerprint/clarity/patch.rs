//! Oriented raw patches, spatial class labels and scale-aware repetition.

use super::{Capture, FingerprintError, bilinear};

const MAX_WORK: usize = 1_000_000_000;
const VARIANCE_FLOOR: f64 = 1e-10;

#[derive(Clone, Copy, Default)]
pub(super) struct Components {
    pub contrast: f64,
    pub separation: f64,
    pub repetition: f64,
}

pub(super) struct Sampler<'a> {
    source: Source<'a>,
    period: f64,
    across: Vec<f64>,
    along: Vec<f64>,
    kernel: Vec<f64>,
    lags: Vec<f64>,
    tensor_radius: usize,
}

impl<'a> Sampler<'a> {
    pub(super) fn new(
        capture: &'a Capture,
        period: f64,
        anchors: usize,
    ) -> Result<Self, FingerprintError> {
        let across = coordinates(3.0 * period);
        let along = coordinates(period / 2.0);
        let across_step = across[1] - across[0];
        let sigma = period / 32.0;
        let radius = (3.0 * sigma / across_step).ceil() as usize;
        let kernel = (0..=radius)
            .map(|i| (-0.5 * (i as f64 * across_step / sigma).powi(2)).exp())
            .collect::<Vec<_>>();
        let intervals = (0.4 * period / across_step).ceil() as usize;
        let mut lags: Vec<_> = (0..=intervals)
            .map(|i| (0.8 + 0.4 * i as f64 / intervals as f64) * period / across_step)
            .collect();
        lags.push(period / across_step);
        lags.sort_by(f64::total_cmp);
        lags.dedup();
        let tensor_radius = (period / 2.0).ceil() as usize;
        let tensor_side = 2 * tensor_radius + 1;
        let work_per_anchor = 5 * tensor_side * tensor_side
            + 4 * across.len() * along.len()
            + across.len() * (2 * radius + 1 + 2 * lags.len());
        if anchors
            .checked_mul(work_per_anchor)
            .is_none_or(|work| work > MAX_WORK)
        {
            return Err(FingerprintError::WorkLimit);
        }
        Ok(Self {
            source: Source::new(capture)?,
            period,
            across,
            along,
            kernel,
            lags,
            tensor_radius,
        })
    }

    pub(super) fn measure(&self, x: usize, y: usize) -> Option<Components> {
        let angle = self.normal(x, y)?;
        let (ny, nx) = angle.sin_cos();
        let (tx, ty) = (-ny, nx);
        let extent_x = 1.5 * self.period * nx.abs() + self.period / 4.0 * tx.abs();
        let extent_y = 1.5 * self.period * ny.abs() + self.period / 4.0 * ty.abs();
        // A conservative axis-aligned envelope checks every source cell the
        // rotated patch can touch, including unsampled corner clips.
        if !self.source.observed_box(
            x as f64 - extent_x,
            y as f64 - extent_y,
            x as f64 + extent_x,
            y as f64 + extent_y,
        ) {
            return None;
        }
        let mut values = Vec::with_capacity(self.across.len() * self.along.len());
        for &v in &self.along {
            for &u in &self.across {
                values.push(
                    self.source
                        .sample(x as f64 + u * nx + v * tx, y as f64 + u * ny + v * ty),
                );
            }
        }
        detrend(&mut values, &self.across, &self.along);
        let profile = transverse_mean(&values, self.across.len(), self.along.len());
        let profile = smooth(&profile, &self.kernel);
        let mut components = class_components(
            &values,
            &profile,
            &self.across,
            self.along.len(),
            self.period,
        );
        components.repetition = self
            .lags
            .iter()
            .map(|&lag| correlation(&profile, lag))
            .fold(0.0, f64::max)
            .clamp(0.0, 1.0);
        Some(components)
    }

    fn normal(&self, x: usize, y: usize) -> Option<f64> {
        let radius = self.tensor_radius;
        if x <= radius
            || y <= radius
            || x + radius + 1 >= self.source.width
            || y + radius + 1 >= self.source.height
        {
            return None;
        }
        if !self.source.observed_box(
            (x - radius - 1) as f64,
            (y - radius - 1) as f64,
            (x + radius + 1) as f64,
            (y + radius + 1) as f64,
        ) {
            return None;
        }
        let (mut sx, mut sy, mut xx, mut xy, mut yy, mut count) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for row in y - radius..=y + radius {
            for column in x - radius..=x + radius {
                let index = row * self.source.width + column;
                let gx = f64::from(self.source.pixels[index + 1])
                    - f64::from(self.source.pixels[index - 1]);
                let gy = f64::from(self.source.pixels[index + self.source.width])
                    - f64::from(self.source.pixels[index - self.source.width]);
                sx += gx;
                sy += gy;
                xx += gx * gx;
                xy += gx * gy;
                yy += gy * gy;
                count += 1.0;
            }
        }
        // Remove the mean gradient so an additive illumination plane does not
        // rotate the ridge normal. No coherence gate or quality multiplier is used.
        xx = (xx - sx * sx / count).max(0.0);
        xy -= sx * sy / count;
        yy = (yy - sy * sy / count).max(0.0);
        Some(if xx + yy <= VARIANCE_FLOOR {
            0.0
        } else {
            0.5 * (2.0 * xy).atan2(xx - yy)
        })
    }
}

fn coordinates(length: f64) -> Vec<f64> {
    let intervals = length.ceil() as usize;
    (0..=intervals)
        .map(|i| -length / 2.0 + length * i as f64 / intervals as f64)
        .collect()
}

fn detrend(values: &mut [f64], across: &[f64], along: &[f64]) {
    let (mut sum, mut ux, mut vy) = (0.0, 0.0, 0.0);
    for (&v, row) in along.iter().zip(values.chunks_exact(across.len())) {
        for (&u, &value) in across.iter().zip(row) {
            sum += value;
            ux += u * value;
            vy += v * value;
        }
    }
    let mean = sum / values.len() as f64;
    let slope_u = ux / (along.len() as f64 * across.iter().map(|u| u * u).sum::<f64>());
    let slope_v = vy / (across.len() as f64 * along.iter().map(|v| v * v).sum::<f64>());
    for (&v, row) in along.iter().zip(values.chunks_exact_mut(across.len())) {
        for (&u, value) in across.iter().zip(row) {
            *value -= mean + slope_u * u + slope_v * v;
        }
    }
}

fn transverse_mean(values: &[f64], columns: usize, rows: usize) -> Vec<f64> {
    let mut profile = vec![0.0; columns];
    for row in values.chunks_exact(columns) {
        for (sum, value) in profile.iter_mut().zip(row) {
            *sum += value / rows as f64;
        }
    }
    profile
}

fn smooth(profile: &[f64], kernel: &[f64]) -> Vec<f64> {
    let radius = kernel.len() - 1;
    (0..profile.len())
        .map(|index| {
            let (mut sum, mut weight) = (0.0, 0.0);
            for (neighbor, &value) in profile
                .iter()
                .enumerate()
                .take((index + radius + 1).min(profile.len()))
                .skip(index.saturating_sub(radius))
            {
                let k = kernel[index.abs_diff(neighbor)];
                sum += k * value;
                weight += k;
            }
            sum / weight
        })
        .collect()
}

fn class_components(
    values: &[f64],
    profile: &[f64],
    across: &[f64],
    rows: usize,
    period: f64,
) -> Components {
    let columns: Vec<_> = across
        .iter()
        .enumerate()
        .filter_map(|(i, u)| (u.abs() <= period / 2.0 + 1e-9).then_some(i))
        .collect();
    let low = columns
        .iter()
        .map(|&i| profile[i])
        .fold(f64::INFINITY, f64::min);
    let high = columns
        .iter()
        .map(|&i| profile[i])
        .fold(f64::NEG_INFINITY, f64::max);
    if high - low <= 1e-6 {
        return Components::default();
    }
    let threshold = (low + high) / 2.0;
    let (mut sums, mut counts, mut squared) = ([0.0; 2], [0_usize; 2], 0.0);
    for row in values.chunks_exact(across.len()) {
        for &column in &columns {
            // Spatial column labels come from the projected/smoothed profile;
            // raw pixels are never classified by their own intensity value.
            let class = usize::from(profile[column] >= threshold);
            sums[class] += row[column];
            counts[class] += 1;
            squared += row[column].powi(2);
        }
    }
    if counts.contains(&0) {
        return Components::default();
    }
    let count = (rows * columns.len()) as f64;
    let variance = (squared / count - ((sums[0] + sums[1]) / count).powi(2)).max(0.0);
    if variance <= VARIANCE_FLOOR {
        return Components::default();
    }
    let difference = sums[1] / counts[1] as f64 - sums[0] / counts[0] as f64;
    let between = (counts[0] as f64 / count) * (counts[1] as f64 / count) * difference.powi(2);
    Components {
        contrast: (difference / 255.0).clamp(0.0, 1.0),
        separation: (between / variance).clamp(0.0, 1.0),
        repetition: 0.0,
    }
}

fn correlation(profile: &[f64], lag: f64) -> f64 {
    let count = ((profile.len() - 1) as f64 - lag).floor() as usize + 1;
    let (mut a, mut b, mut aa, mut bb, mut ab) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for index in 0..count {
        let shifted = index as f64 + lag;
        let left = shifted.floor() as usize;
        let fraction = shifted - left as f64;
        let x = profile[index];
        let y = profile[left] * (1.0 - fraction)
            + profile[(left + 1).min(profile.len() - 1)] * fraction;
        a += x;
        b += y;
        aa += x * x;
        bb += y * y;
        ab += x * y;
    }
    let count = count as f64;
    let va = (aa - a * a / count).max(0.0);
    let vb = (bb - b * b / count).max(0.0);
    if va <= VARIANCE_FLOOR || vb <= VARIANCE_FLOOR {
        return 0.0;
    }
    ((ab - a * b / count) / (va * vb).sqrt()).clamp(-1.0, 1.0)
}

struct Source<'a> {
    width: usize,
    height: usize,
    pixels: &'a [u8],
    invalid_prefix: Option<Vec<u32>>,
}

impl<'a> Source<'a> {
    fn new(capture: &'a Capture) -> Result<Self, FingerprintError> {
        let (width, height) = (capture.width() as usize, capture.height() as usize);
        let invalid_prefix = if let Some(mask) = capture.valid_mask() {
            let stride = width.checked_add(1).ok_or(FingerprintError::WorkLimit)?;
            let count = height
                .checked_add(1)
                .and_then(|rows| stride.checked_mul(rows))
                .ok_or(FingerprintError::WorkLimit)?;
            if mask.len() > u32::MAX as usize {
                return Err(FingerprintError::WorkLimit);
            }
            let mut prefix = vec![0_u32; count];
            for y in 0..height {
                let mut row = 0;
                for x in 0..width {
                    row += u32::from(!mask[y * width + x]);
                    prefix[(y + 1) * stride + x + 1] = prefix[y * stride + x + 1] + row;
                }
            }
            Some(prefix)
        } else {
            None
        };
        Ok(Self {
            width,
            height,
            pixels: capture.pixels(),
            invalid_prefix,
        })
    }

    fn observed_box(&self, left: f64, top: f64, right: f64, bottom: f64) -> bool {
        if left < 0.0
            || top < 0.0
            || right > (self.width - 1) as f64
            || bottom > (self.height - 1) as f64
        {
            return false;
        }
        let Some(prefix) = &self.invalid_prefix else {
            return true;
        };
        let (left, top) = (left.floor() as usize, top.floor() as usize);
        let (right, bottom) = (
            (right.floor() as usize + 1).min(self.width - 1) + 1,
            (bottom.floor() as usize + 1).min(self.height - 1) + 1,
        );
        let stride = self.width + 1;
        // Rearranged unsigned sums cannot underflow for a rectangle count.
        prefix[bottom * stride + right] + prefix[top * stride + left]
            == prefix[bottom * stride + left] + prefix[top * stride + right]
    }

    fn sample(&self, x: f64, y: f64) -> f64 {
        let x = x.clamp(0.0, (self.width - 1) as f64);
        let y = y.clamp(0.0, (self.height - 1) as f64);
        let (left, top) = (x.floor() as usize, y.floor() as usize);
        let (right, bottom) = (
            (left + 1).min(self.width - 1),
            (top + 1).min(self.height - 1),
        );
        let values = [
            top * self.width + left,
            top * self.width + right,
            bottom * self.width + left,
            bottom * self.width + right,
        ]
        .map(|i| f64::from(self.pixels[i]));
        bilinear(values, x - left as f64, y - top as f64)
    }
}
