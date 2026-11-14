//! One seeded partial capture, selected independently of feature strength or
//! matching success. Coverage counts the enrollment's confirmed pattern pixels.

use anyhow::{Context, Result, ensure};
use gnorix::core::biometrics::Capture;
use gnorix_example_support::fingerprint::{Circle, select};
use image::{GrayImage, ImageEncoder};
use serde::Serialize;

pub struct PatternGeometry {
    width: u32,
    height: u32,
    pixels: Vec<[u32; 2]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaptureMetadata {
    pub circle: Circle,
    /// Crop origin in the full source image, for fresh-feature overlays.
    pub offset: [f64; 2],
    pub pattern_pixels: usize,
    pub covered_pattern_pixels: usize,
    pub coverage_fraction: f64,
}

pub struct SampledCapture {
    pub capture: Capture,
    /// The unchanged full frame from which `capture` was selected. The UI
    /// blurs outside `metadata.circle`; analysis receives only the masked crop.
    pub png: Vec<u8>,
    pub metadata: CaptureMetadata,
}

impl PatternGeometry {
    pub fn new(width: u32, height: u32, support: &[bool]) -> Result<Self> {
        let count = usize::try_from(width)?
            .checked_mul(usize::try_from(height)?)
            .context("pattern dimensions overflow")?;
        ensure!(
            count > 0 && support.len() == count,
            "pattern support dimensions differ from the image"
        );
        let pixels: Vec<_> = support
            .iter()
            .enumerate()
            .filter_map(|(index, supported)| {
                supported.then_some([
                    (index % width as usize) as u32,
                    (index / width as usize) as u32,
                ])
            })
            .collect();
        ensure!(!pixels.is_empty(), "confirmed pattern has no pixels");
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// The caller supplies a fresh random seed for each user-approved attempt.
    /// A fixed seed exists only to reproduce a test. No outcome-based resampling
    /// occurs: one center and one radius select unchanged source pixels.
    pub fn sample_with_seed(&self, source: &GrayImage, seed: u64) -> Result<SampledCapture> {
        ensure!(
            source.dimensions() == (self.width, self.height),
            "capture source differs from enrolled image dimensions"
        );
        let mut random = SamplingRandom(seed);
        let center = self.pixels[random.index(self.pixels.len())];
        let circle = self.covering_circle(center);
        let selected = select(source, Some(circle)).map_err(anyhow::Error::from_boxed)?;
        let covered_pattern_pixels = self
            .pixels
            .iter()
            .filter(|&&point| contains(circle, point))
            .count();
        ensure!(
            covered_pattern_pixels >= self.pixels.len().div_ceil(3),
            "sample does not cover a third of the confirmed pattern"
        );
        let metadata = CaptureMetadata {
            circle,
            offset: selected.offset,
            pattern_pixels: self.pixels.len(),
            covered_pattern_pixels,
            coverage_fraction: covered_pattern_pixels as f64 / self.pixels.len() as f64,
        };
        let capture = sanitize(selected.capture)?;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(
            source.as_raw(),
            source.width(),
            source.height(),
            image::ExtendedColorType::L8,
        )?;
        Ok(SampledCapture {
            capture,
            png,
            metadata,
        })
    }

    fn covering_circle(&self, center: [u32; 2]) -> Circle {
        let mut distances: Vec<_> = self
            .pixels
            .iter()
            .map(|&point| distance(center, point))
            .collect();
        let target = self.pixels.len().div_ceil(3) - 1;
        let (_, radius, _) = distances.select_nth_unstable_by(target, f64::total_cmp);
        Circle {
            x: f64::from(center[0]),
            y: f64::from(center[1]),
            // Preserve the shared crop adapter's minimum supported geometry.
            // Exact coverage below reports ties and any extra minimum-radius pixels.
            radius: radius.max(8.0),
        }
    }
}

fn distance(first: [u32; 2], second: [u32; 2]) -> f64 {
    (f64::from(first[0]) - f64::from(second[0])).hypot(f64::from(first[1]) - f64::from(second[1]))
}

fn contains(circle: Circle, point: [u32; 2]) -> bool {
    (f64::from(point[0]) - circle.x).hypot(f64::from(point[1]) - circle.y) <= circle.radius
}

fn sanitize(selected: Capture) -> Result<Capture> {
    let mask = selected
        .valid_mask()
        .context("circular capture has no validity mask")?;
    let mut pixels = selected.pixels().to_vec();
    for (pixel, valid) in pixels.iter_mut().zip(mask) {
        if !valid {
            *pixel = 255;
        }
    }
    Ok(Capture::new(
        selected.width(),
        selected.height(),
        pixels,
        Some(mask.to_vec()),
        selected.pixels_per_inch(),
    )?)
}

/// SplitMix64 is used for reproducible public capture variation, never keys.
struct SamplingRandom(u64);

impl SamplingRandom {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn index(&mut self, count: usize) -> usize {
        let count = count as u64;
        let excluded = count.wrapping_neg() % count;
        loop {
            let value = self.next();
            // Reject only the incomplete integer bucket to avoid modulo bias;
            // this does not retry a circle, extraction or signing result.
            if value >= excluded {
                return (value % count) as usize;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use image::Luma;

    use super::*;

    fn fixture() -> (GrayImage, Vec<bool>) {
        let source = GrayImage::from_fn(120, 160, |x, _| Luma([if x % 12 < 6 { 40 } else { 210 }]));
        let support = (0..120 * 160)
            .map(|index| {
                let x = (index % 120) as f64 - 60.0;
                let y = (index / 120) as f64 - 80.0;
                x * x / 45_f64.powi(2) + y * y / 60_f64.powi(2) <= 1.0
            })
            .collect();
        (source, support)
    }

    #[test]
    fn seeded_selection_preserves_source_pixels_and_excludes_outside_data() -> Result<()> {
        let (source, support) = fixture();
        let pattern = PatternGeometry::new(120, 160, &support)?;
        let first = pattern.sample_with_seed(&source, 17)?;
        let same = pattern.sample_with_seed(&source, 17)?;
        let other = pattern.sample_with_seed(&source, 18)?;
        assert_eq!(first.png, same.png);
        assert_eq!(first.metadata.circle, same.metadata.circle);
        assert_ne!(first.metadata.circle, other.metadata.circle);
        assert_eq!(first.png, other.png);
        let preview = image::load_from_memory(&first.png)?.to_luma8();
        assert_eq!(preview, source);
        let metadata = &first.metadata;
        assert!(metadata.coverage_fraction >= 1.0 / 3.0 && metadata.coverage_fraction < 0.35);
        assert_eq!(
            metadata.pattern_pixels,
            support.iter().filter(|&&value| value).count()
        );
        assert!(support[metadata.circle.y as usize * 120 + metadata.circle.x as usize]);

        let mask = first.capture.valid_mask().unwrap();
        assert!(mask.iter().any(|&valid| !valid));
        let mut observed_supported = 0;
        for (index, &valid) in mask.iter().enumerate() {
            let x = metadata.offset[0] as u32 + index as u32 % first.capture.width();
            let y = metadata.offset[1] as u32 + index as u32 / first.capture.width();
            if valid {
                assert_eq!(first.capture.pixels()[index], source.get_pixel(x, y).0[0]);
                observed_supported += usize::from(support[(y * 120 + x) as usize]);
            } else {
                assert_eq!(first.capture.pixels()[index], 255);
            }
        }
        assert_eq!(metadata.covered_pattern_pixels, observed_supported);
        Ok(())
    }

    #[test]
    fn distance_quantile_uses_the_pattern_instead_of_the_frame_area() -> Result<()> {
        let (source, support) = fixture();
        let pattern = PatternGeometry::new(120, 160, &support)?;
        for seed in 0..24 {
            let sample = pattern.sample_with_seed(&source, seed)?;
            assert!(sample.metadata.covered_pattern_pixels >= pattern.pixels.len().div_ceil(3));
            assert!(sample.metadata.coverage_fraction < 0.35);
        }
        assert!(PatternGeometry::new(0, 1, &[]).is_err());
        assert!(PatternGeometry::new(120, 160, &support[..1]).is_err());
        assert!(PatternGeometry::new(120, 160, &vec![false; support.len()]).is_err());
        assert!(pattern.sample_with_seed(&GrayImage::new(1, 1), 1).is_err());
        Ok(())
    }
}
