//! Reproducible local capture defects for the test stand, not biometric augmentation.

use image::{GrayImage, Luma};

use crate::Result;

const COLUMNS: usize = 4;
const ROWS: usize = 6;

/// Apply one seeded defect scenario without changing the image coordinate frame.
///
/// The level is 0..=100. Zero preserves every byte; higher levels increase the
/// same patches' deformation and opacity, rather than covering more of the image.
/// Displacement is measured relative to the original image's ridge width R.
pub fn apply(source: &GrayImage, unit_px: f64, level: u8, seed: u64) -> Result<GrayImage> {
    validate(source, unit_px, level)?;
    if level == 0 {
        return Ok(source.clone());
    }

    let mut output = source.clone();
    let strength = f64::from(level) / 100.0;
    for patch in scenario(source, unit_px, seed) {
        render_patch(source, &mut output, patch, strength);
    }
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Defect {
    Bend {
        angle: f64,
        drift: [f64; 2],
        blur: f64,
    },
    Dropout,
    Smudge,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Patch {
    center: [f64; 2],
    radius: f64,
    defect: Defect,
}

fn scenario(source: &GrayImage, unit_px: f64, seed: u64) -> Vec<Patch> {
    let mut random = ScenarioRandom(seed);
    let cell = [
        f64::from(source.width()) / COLUMNS as f64,
        f64::from(source.height()) / ROWS as f64,
    ];
    // Every support disk stays in its own cell. Even an oversized supplied R
    // cannot create one image-wide defect or allow patches to compound.
    let radius_limit = 0.32 * cell[0].min(cell[1]);
    let unit = unit_px.min(radius_limit / 2.0);
    let mut patches = Vec::with_capacity(COLUMNS * ROWS);
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let center = [
                (column as f64 + random.between(0.38, 0.62)) * cell[0],
                (row as f64 + random.between(0.38, 0.62)) * cell[1],
            ];
            let kind = (row * COLUMNS + column) % 3;
            let radius = (unit * random.between(2.0, 4.0)).min(radius_limit);
            let defect = match kind {
                0 => Defect::Bend {
                    angle: random.between(-0.65, 0.65),
                    drift: [
                        unit * random.between(-0.8, 0.8),
                        unit * random.between(-0.8, 0.8),
                    ],
                    blur: unit * random.between(0.15, 0.3),
                },
                1 => Defect::Dropout,
                _ => Defect::Smudge,
            };
            patches.push(Patch {
                center,
                radius,
                defect,
            });
        }
    }
    patches
}

fn render_patch(source: &GrayImage, output: &mut GrayImage, patch: Patch, strength: f64) {
    let left = (patch.center[0] - patch.radius).floor().max(0.0) as u32;
    let top = (patch.center[1] - patch.radius).floor().max(0.0) as u32;
    let right = ((patch.center[0] + patch.radius).ceil() as u32).min(source.width());
    let bottom = ((patch.center[1] + patch.radius).ceil() as u32).min(source.height());
    for y in top..bottom {
        for x in left..right {
            let position = [f64::from(x), f64::from(y)];
            let amount = strength * support(patch, position);
            if amount == 0.0 {
                continue;
            }
            let original = f64::from(source.get_pixel(x, y).0[0]);
            let value = match patch.defect {
                Defect::Bend { angle, drift, blur } => {
                    bend(source, patch.center, position, amount, angle, drift, blur)
                }
                Defect::Dropout => original + amount * 0.95 * (255.0 - original),
                Defect::Smudge => original * (1.0 - amount * 0.95),
            };
            output.put_pixel(x, y, Luma([value.round().clamp(0.0, 255.0) as u8]));
        }
    }
}

fn support(patch: Patch, position: [f64; 2]) -> f64 {
    let dx = (position[0] - patch.center[0]) / patch.radius;
    let dy = (position[1] - patch.center[1]) / patch.radius;
    (1.0 - dx * dx - dy * dy).max(0.0).powi(3)
}

fn bend(
    source: &GrayImage,
    center: [f64; 2],
    position: [f64; 2],
    amount: f64,
    angle: f64,
    drift: [f64; 2],
    blur: f64,
) -> f64 {
    let (sin, cos) = (angle * amount).sin_cos();
    let dx = position[0] - center[0];
    let dy = position[1] - center[1];
    let x = center[0] + dx * cos - dy * sin + drift[0] * amount;
    let y = center[1] + dx * sin + dy * cos + drift[1] * amount;
    let radius = blur * amount;

    0.5 * sample(source, x, y)
        + 0.125
            * (sample(source, x - radius, y)
                + sample(source, x + radius, y)
                + sample(source, x, y - radius)
                + sample(source, x, y + radius))
}

fn sample(source: &GrayImage, x: f64, y: f64) -> f64 {
    let x = x.clamp(0.0, f64::from(source.width() - 1));
    let y = y.clamp(0.0, f64::from(source.height() - 1));
    let left = x.floor() as u32;
    let top = y.floor() as u32;
    let right = (left + 1).min(source.width() - 1);
    let bottom = (top + 1).min(source.height() - 1);
    let horizontal = x - f64::from(left);
    let vertical = y - f64::from(top);
    let upper = f64::from(source.get_pixel(left, top).0[0]) * (1.0 - horizontal)
        + f64::from(source.get_pixel(right, top).0[0]) * horizontal;
    let lower = f64::from(source.get_pixel(left, bottom).0[0]) * (1.0 - horizontal)
        + f64::from(source.get_pixel(right, bottom).0[0]) * horizontal;
    upper * (1.0 - vertical) + lower * vertical
}

/// SplitMix64 supplies reproducible scenario coordinates, never key material.
struct ScenarioRandom(u64);

impl ScenarioRandom {
    fn between(&mut self, lower: f64, upper: f64) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^= value >> 31;
        let fraction = (value >> 11) as f64 / ((1_u64 << 53) as f64);
        lower + fraction * (upper - lower)
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
    fn scenario_is_reproducible_and_seed_changes_local_defects() {
        let source = ridges();
        let first = apply(&source, 6.0, 80, 17).unwrap();
        assert_eq!(first, apply(&source, 6.0, 80, 17).unwrap());
        assert_ne!(first, source);
        assert_ne!(first, apply(&source, 6.0, 80, 18).unwrap());
        assert_eq!(first.dimensions(), source.dimensions());
    }

    #[test]
    fn maximum_strength_preserves_pixels_outside_compact_nonoverlapping_patches() {
        let source = ridges();
        // Also exercise an oversized R: the cell cap still prevents full coverage.
        for unit in [6.0, 1e100] {
            let patches = scenario(&source, unit, 19);
            let result = apply(&source, unit, 100, 19).unwrap();
            let mut outside = 0;
            let mut changed = 0;
            for (x, y, pixel) in source.enumerate_pixels() {
                let count = patches
                    .iter()
                    .filter(|&&patch| support(patch, [f64::from(x), f64::from(y)]) > 0.0)
                    .count();
                assert!(count <= 1, "patches overlap at ({x}, {y})");
                if count == 0 {
                    outside += 1;
                    assert_eq!(result.get_pixel(x, y), pixel);
                }
                changed += usize::from(result.get_pixel(x, y) != pixel);
            }
            assert!(outside > source.as_raw().len() / 2);
            assert!(changed > 0);
        }
    }

    #[test]
    fn dropout_and_smudge_increase_monotonically_in_the_same_scenario() {
        let source = ridges();
        let patches = scenario(&source, 6.0, 23);
        let levels = [0, 20, 50, 100].map(|level| apply(&source, 6.0, level, 23).unwrap());
        for (x, y, original) in source.enumerate_pixels() {
            let Some(patch) = patches
                .iter()
                .find(|&&patch| support(patch, [f64::from(x), f64::from(y)]) > 0.0)
            else {
                continue;
            };
            let values = levels.each_ref().map(|image| image.get_pixel(x, y).0[0]);
            match patch.defect {
                Defect::Dropout => {
                    assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
                    assert!(values[3] >= original.0[0]);
                }
                Defect::Smudge => {
                    assert!(values.windows(2).all(|pair| pair[0] >= pair[1]));
                    assert!(values[3] <= original.0[0]);
                }
                Defect::Bend { .. } => {}
            }
        }
    }

    #[test]
    fn warping_clamps_to_source_edges_without_inventing_black_pixels() {
        let source = GrayImage::from_pixel(10, 10, Luma([173]));
        for position in [[-100.0, -20.0], [100.0, 50.0], [3.5, 4.8]] {
            assert!((sample(&source, position[0], position[1]) - 173.0).abs() < 1e-10);
            assert!(
                (bend(&source, [0.0, 0.0], position, 1.0, 0.6, [-20.0, 40.0], 3.0) - 173.0).abs()
                    < 1e-10
            );
        }
    }
}
