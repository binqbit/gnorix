//! Seeded full-frame capture defects for the local test stand.

use image::{GrayImage, Luma};

use crate::Result;

mod streaks;

/// Smooth displacement, blur and exposure fields cover the whole image.
/// Their spatial scales come from clean ridge width; severity controls strength.
pub fn apply(source: &GrayImage, unit_px: f64, level: u8, seed: u64) -> Result<GrayImage> {
    validate(source, unit_px, level)?;
    if level == 0 {
        return Ok(source.clone());
    }

    let unit = unit_px.min((f64::from(source.width().min(source.height())) / 12.0).max(1.0));
    let strength = f64::from(level) / 100.0;
    let intensity = strength * (0.3 + 0.7 * strength);
    let fields = CaptureFields::new(source, unit, seed);
    let warped = warp(source, &fields, unit * 4.0 * intensity);
    let mild = image::imageops::blur(&warped, (unit * 0.4 * intensity) as f32);
    let strong = image::imageops::blur(&warped, (unit * 1.4 * intensity) as f32);

    let mut output = render_capture(&warped, &mild, &strong, &fields, intensity);
    streaks::apply(&mut output, unit, strength, seed)?;
    Ok(output)
}

fn validate(source: &GrayImage, unit_px: f64, level: u8) -> Result<()> {
    if source.width() == 0 || source.height() == 0 {
        return Err("artifact input must have nonzero dimensions".into());
    }
    if !unit_px.is_finite() || unit_px <= 0.0 {
        return Err("artifact ridge width must be finite and positive".into());
    }
    if level > 100 {
        return Err("artifact level must be between 0 and 100".into());
    }
    Ok(())
}

struct CaptureFields {
    horizontal: LayeredField,
    vertical: LayeredField,
    blur: SmoothField,
    exposure: LayeredField,
}

impl CaptureFields {
    fn new(source: &GrayImage, unit: f64, seed: u64) -> Self {
        let mut random = ScenarioRandom(seed);
        Self {
            horizontal: LayeredField::new(source, unit * 14.0, unit * 4.0, &mut random),
            vertical: LayeredField::new(source, unit * 14.0, unit * 4.0, &mut random),
            blur: SmoothField::new(source, unit * 20.0, &mut random),
            exposure: LayeredField::new(source, unit * 28.0, unit * 7.0, &mut random),
        }
    }
}

/// Broad defects with irregular edges and no discontinuities at control nodes.
struct LayeredField {
    coarse: SmoothField,
    fine: SmoothField,
}

impl LayeredField {
    fn new(source: &GrayImage, coarse: f64, fine: f64, random: &mut ScenarioRandom) -> Self {
        Self {
            coarse: SmoothField::new(source, coarse, random),
            fine: SmoothField::new(source, fine, random),
        }
    }

    fn at(&self, x: f64, y: f64) -> f64 {
        0.72 * self.coarse.at(x, y) + 0.28 * self.fine.at(x, y)
    }
}

/// Smooth random control fields span the full capture. A shifted origin avoids
/// anchoring defects to the image's top-left pixel.
struct SmoothField {
    columns: usize,
    spacing: f64,
    offset: [f64; 2],
    values: Vec<f64>,
}

impl SmoothField {
    fn new(source: &GrayImage, spacing: f64, random: &mut ScenarioRandom) -> Self {
        let spacing = spacing.max(8.0);
        let columns = (f64::from(source.width()) / spacing).ceil() as usize + 3;
        let rows = (f64::from(source.height()) / spacing).ceil() as usize + 3;
        let offset = [random.unit(), random.unit()];
        let values = (0..columns * rows)
            .map(|_| 2.0 * random.unit() - 1.0)
            .collect();
        Self {
            columns,
            spacing,
            offset,
            values,
        }
    }

    fn at(&self, x: f64, y: f64) -> f64 {
        let column = x / self.spacing + self.offset[0];
        let row = y / self.spacing + self.offset[1];
        let left = column.floor() as usize;
        let top = row.floor() as usize;
        let across = smoothstep(column.fract());
        let down = smoothstep(row.fract());
        let upper = mix(
            self.values[top * self.columns + left],
            self.values[top * self.columns + left + 1],
            across,
        );
        let lower = mix(
            self.values[(top + 1) * self.columns + left],
            self.values[(top + 1) * self.columns + left + 1],
            across,
        );
        mix(upper, lower, down)
    }
}

fn warp(source: &GrayImage, fields: &CaptureFields, distance: f64) -> GrayImage {
    GrayImage::from_fn(source.width(), source.height(), |x, y| {
        let x = f64::from(x);
        let y = f64::from(y);
        pixel(sample(
            source,
            x + distance * fields.horizontal.at(x, y),
            y + distance * fields.vertical.at(x, y),
        ))
    })
}

fn render_capture(
    warped: &GrayImage,
    mild: &GrayImage,
    strong: &GrayImage,
    fields: &CaptureFields,
    strength: f64,
) -> GrayImage {
    GrayImage::from_fn(warped.width(), warped.height(), |x, y| {
        let position = [f64::from(x), f64::from(y)];
        let broad_blur = smoothstep((fields.blur.at(position[0], position[1]) + 0.65) / 1.25);
        let softened = mix(
            f64::from(warped.get_pixel(x, y).0[0]),
            f64::from(mild.get_pixel(x, y).0[0]),
            0.55 * strength,
        );
        let blurred = mix(
            softened,
            f64::from(strong.get_pixel(x, y).0[0]),
            strength * (0.35 + 0.64 * broad_blur),
        );
        let exposure = fields.exposure.at(position[0], position[1]);
        let bright = smoothstep((exposure - 0.05) / 0.55);
        let dark = smoothstep((-exposure - 0.05) / 0.55);
        let faded = mix(blurred, 255.0, 0.94 * strength * bright);
        pixel(mix(faded, 0.0, 0.94 * strength * dark))
    })
}

fn pixel(value: f64) -> Luma<u8> {
    Luma([value.round().clamp(0.0, 255.0) as u8])
}

fn mix(first: f64, second: f64, amount: f64) -> f64 {
    first + (second - first) * amount
}

fn smoothstep(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn sample(source: &GrayImage, x: f64, y: f64) -> f64 {
    let x = x.clamp(0.0, f64::from(source.width() - 1));
    let y = y.clamp(0.0, f64::from(source.height() - 1));
    let left = x.floor() as u32;
    let top = y.floor() as u32;
    let right = (left + 1).min(source.width() - 1);
    let bottom = (top + 1).min(source.height() - 1);
    let upper = mix(
        f64::from(source.get_pixel(left, top).0[0]),
        f64::from(source.get_pixel(right, top).0[0]),
        x.fract(),
    );
    let lower = mix(
        f64::from(source.get_pixel(left, bottom).0[0]),
        f64::from(source.get_pixel(right, bottom).0[0]),
        x.fract(),
    );
    mix(upper, lower, y.fract())
}

/// SplitMix64 drives public, reproducible test artifacts, never key material.
struct ScenarioRandom(u64);

impl ScenarioRandom {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^= value >> 31;
        value
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / ((1_u64 << 53) as f64)
    }

    fn between(&mut self, lower: f64, upper: f64) -> f64 {
        lower + self.unit() * (upper - lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ridges() -> GrayImage {
        GrayImage::from_fn(240, 360, |x, y| {
            Luma([if (x + y / 3) % 12 < 6 { 40 } else { 210 }])
        })
    }

    #[test]
    fn zero_is_exact_and_invalid_parameters_are_rejected() {
        let source = ridges();
        assert_eq!(apply(&source, 6.0, 0, 0).unwrap(), source);
        for unit in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(apply(&source, unit, 10, 0).is_err());
        }
        assert!(apply(&source, 6.0, 101, 0).is_err());
        assert!(apply(&GrayImage::new(0, 2), 1.0, 0, 0).is_err());
    }

    #[test]
    fn same_seed_reproduces_the_whole_frame_and_other_seeds_change_it() {
        let source = ridges();
        let first = apply(&source, 6.0, 80, 17).unwrap();
        assert_eq!(first, apply(&source, 6.0, 80, 17).unwrap());
        assert_ne!(first, apply(&source, 6.0, 80, 18).unwrap());
        assert_eq!(first.dimensions(), source.dimensions());
    }

    #[test]
    fn high_severity_changes_every_region_without_flattening_the_scan() {
        let source = ridges();
        let result = apply(&source, 6.0, 100, 19).unwrap();
        for row in 0..4 {
            for column in 0..4 {
                let mut changed = 0;
                let mut minimum = 255;
                let mut maximum = 0;
                for y in row * 90..(row + 1) * 90 {
                    for x in column * 60..(column + 1) * 60 {
                        let value = result.get_pixel(x, y).0[0];
                        changed += usize::from(value.abs_diff(source.get_pixel(x, y).0[0]) >= 3);
                        minimum = minimum.min(value);
                        maximum = maximum.max(value);
                    }
                }
                assert!(
                    changed > 60 * 90 / 2,
                    "region ({column}, {row}) remained mostly untouched"
                );
                assert!(
                    maximum - minimum > 30,
                    "region ({column}, {row}) lost all structure"
                );
            }
        }
    }

    #[test]
    fn maximum_severity_strongly_changes_most_ridge_pixels() {
        let source = ridges();
        for seed in [19, 41, 73] {
            let moderate = apply(&source, 6.0, 25, seed).unwrap();
            let maximum = apply(&source, 6.0, 100, seed).unwrap();
            let difference = |image: &GrayImage| {
                image
                    .as_raw()
                    .iter()
                    .zip(source.as_raw())
                    .map(|(&actual, &original)| u64::from(actual.abs_diff(original)))
                    .sum::<u64>()
            };
            let strongly_changed = maximum
                .as_raw()
                .iter()
                .zip(source.as_raw())
                .filter(|&(&actual, &original)| actual.abs_diff(original) >= 24)
                .count();
            assert!(difference(&maximum) > 2 * difference(&moderate));
            assert!(strongly_changed * 100 >= source.as_raw().len() * 80);
        }
    }

    #[test]
    fn exposure_severity_is_progressive_in_broad_bright_and_dark_areas() {
        let source = GrayImage::from_pixel(240, 360, Luma([128]));
        let levels = [20, 60, 100].map(|level| apply(&source, 6.0, level, 23).unwrap());
        let deviations = levels.each_ref().map(|image| {
            image
                .as_raw()
                .iter()
                .map(|&value| u64::from(value.abs_diff(128)))
                .sum::<u64>()
        });
        assert!(deviations[0] < deviations[1] && deviations[1] < deviations[2]);
        let highest = levels.last().unwrap();
        assert!(
            highest
                .as_raw()
                .iter()
                .filter(|&&value| value > 170)
                .count()
                > 1000
        );
        assert!(highest.as_raw().iter().filter(|&&value| value < 85).count() > 1000);
    }

    #[test]
    fn broad_blur_fields_reduce_ridge_contrast_across_the_capture() {
        let source = ridges();
        let result = apply(&source, 6.0, 100, 41).unwrap();
        let contrast = |image: &GrayImage| {
            (1..image.width())
                .flat_map(|x| {
                    (0..image.height()).map(move |y| {
                        u64::from(
                            image.get_pixel(x, y).0[0].abs_diff(image.get_pixel(x - 1, y).0[0]),
                        )
                    })
                })
                .sum::<u64>()
        };
        assert!(contrast(&result) < contrast(&source) / 2);
    }

    #[test]
    fn sampling_preserves_constant_edges_and_extreme_valid_scales_remain_bounded() {
        let constant = GrayImage::from_pixel(10, 10, Luma([173]));
        for (x, y) in [(-100.0, -20.0), (100.0, 50.0), (3.5, 4.8)] {
            assert!((sample(&constant, x, y) - 173.0).abs() < 1e-10);
        }
        for unit in [1e-100, 1e100] {
            assert_eq!(
                apply(&constant, unit, 100, 11).unwrap().dimensions(),
                constant.dimensions()
            );
        }
    }
}
