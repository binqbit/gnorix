//! Local grayscale references and contrast normalization.

use std::fmt;

use super::window::{Integral, Window};
use super::{Capture, FingerprintError, NUMERICAL_VARIANCE_FLOOR, Normalized};

/// Sliding local means of observed dark and light grayscale classes.
///
/// Both reference arrays use raw grayscale units in [0, 255]. Only entries
/// whose `usable` bit is true are measurements; unsupported slots contain zero.
#[derive(Clone, PartialEq)]
pub struct LocalPalette {
    /// Window radius in source pixels; the unclipped width is twice this plus one.
    pub radius_px: usize,
    pub ridge_gray: Vec<f32>,
    pub valley_gray: Vec<f32>,
    pub usable: Vec<bool>,
}

impl fmt::Debug for LocalPalette {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalPalette")
            .field("radius_px", &self.radius_px)
            .field("pixels", &self.usable.len())
            .field("references", &"[REDACTED]")
            .finish()
    }
}

/// Retain the existing standardized image for orientation and class seeding.
pub(super) fn standardize(
    capture: &Capture,
    radius: usize,
    min_contrast: f64,
) -> Result<Normalized, FingerprintError> {
    let width = capture.width() as usize;
    let height = capture.height() as usize;
    let valid = |index: usize| capture.valid_mask().is_none_or(|mask| mask[index]);
    // Raw u8 sums and squared sums are exact at the default source-pixel budget.
    // Keeping the mean in these units also makes an exact mean-valued pixel a
    // true zero seed, rather than assigning it a class through roundoff.
    let value = |index: usize| f64::from(capture.pixels()[index]);
    let count = Integral::new(width, height, |i| f64::from(valid(i)))?;
    let sum = Integral::new(width, height, |i| if valid(i) { value(i) } else { 0.0 })?;
    let squared = Integral::new(
        width,
        height,
        |i| {
            if valid(i) { value(i).powi(2) } else { 0.0 }
        },
    )?;
    let mut normalized = Normalized {
        values: vec![0.0; capture.pixels().len()],
        usable: vec![false; capture.pixels().len()],
    };

    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let window = Window::around(x, y, width, height, radius);
            let n = window.count() as f64;
            // An incomplete mask window is unsupported, not missing data to be
            // averaged across. Invalid pixel values never enter any statistic.
            if !valid(index) || count.sum(window) != n {
                continue;
            }
            let mean = sum.sum(window) / n;
            let variance = (squared.sum(window) / n - mean * mean).max(0.0);
            let deviation = variance.sqrt();
            if variance <= NUMERICAL_VARIANCE_FLOOR * 255.0_f64.powi(2)
                || deviation / 255.0 < min_contrast
            {
                continue;
            }
            normalized.values[index] = ((value(index) - mean) / deviation).clamp(-3.0, 3.0) as f32;
            normalized.usable[index] = true;
        }
    }
    Ok(normalized)
}

pub(super) fn estimate_palette(
    capture: &Capture,
    seed: &Normalized,
    radius: usize,
) -> Result<LocalPalette, FingerprintError> {
    let mut palette = LocalPalette {
        radius_px: radius,
        ridge_gray: vec![0.0; capture.pixels().len()],
        valley_gray: vec![0.0; capture.pixels().len()],
        usable: seed.usable.clone(),
    };
    // Each class releases its integral buffers before the next is built.
    fill_reference(
        capture,
        seed,
        radius,
        GrayClass::Dark,
        &mut palette.ridge_gray,
        &mut palette.usable,
    )?;
    fill_reference(
        capture,
        seed,
        radius,
        GrayClass::Light,
        &mut palette.valley_gray,
        &mut palette.usable,
    )?;

    for index in 0..palette.usable.len() {
        if !palette.usable[index] || palette.ridge_gray[index] >= palette.valley_gray[index] {
            palette.usable[index] = false;
            palette.ridge_gray[index] = 0.0;
            palette.valley_gray[index] = 0.0;
        }
    }
    Ok(palette)
}

pub(super) fn apply_palette(capture: &Capture, palette: &LocalPalette) -> Normalized {
    let values = capture
        .pixels()
        .iter()
        .enumerate()
        .map(|(index, &pixel)| {
            if !palette.usable[index] {
                return 0.0;
            }
            let dark = f64::from(palette.ridge_gray[index]);
            let light = f64::from(palette.valley_gray[index]);
            let midpoint = (dark + light) / 2.0;
            let half_spread = (light - dark) / 2.0;
            ((f64::from(pixel) - midpoint) / half_spread).clamp(-3.0, 3.0) as f32
        })
        .collect();
    Normalized {
        values,
        usable: palette.usable.clone(),
    }
}

#[derive(Clone, Copy)]
enum GrayClass {
    Dark,
    Light,
}

impl GrayClass {
    fn contains(self, standardized: f32) -> bool {
        match self {
            Self::Dark => standardized < 0.0,
            Self::Light => standardized > 0.0,
        }
    }
}

fn fill_reference(
    capture: &Capture,
    seed: &Normalized,
    radius: usize,
    class: GrayClass,
    reference: &mut [f32],
    usable: &mut [bool],
) -> Result<(), FingerprintError> {
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    let belongs = |index: usize| seed.usable[index] && class.contains(seed.values[index]);
    let count = Integral::new(width, height, |index| f64::from(belongs(index)))?;
    let sum = Integral::new(width, height, |index| {
        if belongs(index) {
            f64::from(capture.pixels()[index])
        } else {
            0.0
        }
    })?;

    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if !usable[index] {
                continue;
            }
            // Seed support already requires a complete original-mask window
            // and the original minimum standard deviation at this center.
            let area = Window::around(x, y, width, height, radius);
            let samples = count.sum(area);
            if samples == 0.0 {
                usable[index] = false;
            } else {
                reference[index] = (sum.sum(area) / samples) as f32;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(width: usize, height: usize, pixels: Vec<u8>) -> Capture {
        Capture::new(width as u32, height as u32, pixels, None, None).unwrap()
    }

    fn palette(input: &Capture, radius: usize) -> LocalPalette {
        let seed = standardize(input, radius, 0.035).unwrap();
        estimate_palette(input, &seed, radius).unwrap()
    }

    #[test]
    fn identical_gray_is_light_and_dark_under_different_observed_palettes() {
        let (width, height) = (96, 48);
        let pixels = (0..width * height)
            .map(|index| {
                let x = index % width;
                match (x < 48, x % 12 < 4) {
                    (true, true) => 20,
                    (true, false) | (false, true) => 100,
                    (false, false) => 220,
                }
            })
            .collect();
        let input = capture(width, height, pixels);
        let palette = palette(&input, 6);
        let normalized = apply_palette(&input, &palette);
        let (light, dark) = (24 * width + 20, 24 * width + 60);

        assert_eq!(input.pixels()[light], input.pixels()[dark]);
        assert_eq!(palette.ridge_gray[light], 20.0);
        assert_eq!(palette.valley_gray[light], 100.0);
        assert_eq!(palette.ridge_gray[dark], 100.0);
        assert_eq!(palette.valley_gray[dark], 220.0);
        assert_eq!(normalized.values[light], 1.0);
        assert_eq!(normalized.values[dark], -1.0);
        assert!(normalized.usable[light] && normalized.usable[dark]);
    }

    #[test]
    fn sliding_references_follow_smooth_illumination_without_block_seams() {
        let (width, height) = (192, 32);
        let pixels = (0..width * height)
            .map(|index| {
                let x = index % width;
                30 + (x / 3) as u8 + if x % 12 < 4 { 0 } else { 80 }
            })
            .collect();
        let input = capture(width, height, pixels);
        let palette = palette(&input, 6);
        let normalized = apply_palette(&input, &palette);

        for x in 24..156 {
            let index = 16 * width + x;
            let next_period = index + 12;
            assert!(palette.usable[index] && palette.usable[next_period]);
            assert!(
                (palette.ridge_gray[next_period] - palette.ridge_gray[index] - 4.0).abs() < 1e-5
            );
            assert!(
                (palette.valley_gray[next_period] - palette.valley_gray[index] - 4.0).abs() < 1e-5
            );
            assert!((normalized.values[next_period] - normalized.values[index]).abs() < 1e-5);
            assert!((palette.ridge_gray[index + 1] - palette.ridge_gray[index]).abs() <= 3.0);
            assert!((palette.valley_gray[index + 1] - palette.valley_gray[index]).abs() <= 3.0);
        }
    }

    #[test]
    fn blank_regions_have_no_fabricated_dark_or_light_reference() {
        let input = capture(32, 32, vec![120; 32 * 32]);
        let palette = palette(&input, 6);
        assert!(palette.usable.iter().all(|&usable| !usable));
        assert!(palette.ridge_gray.iter().all(|&value| value == 0.0));
        assert!(palette.valley_gray.iter().all(|&value| value == 0.0));
        assert!(
            apply_palette(&input, &palette)
                .values
                .iter()
                .all(|&value| value == 0.0)
        );
    }

    #[test]
    fn pixels_equal_to_the_exact_local_mean_seed_neither_class() {
        let (width, height) = (100, 16);
        let values = [32, 80, 128, 176, 224];
        let input = capture(
            width,
            height,
            (0..width * height)
                .map(|index| values[(index % width) % values.len()])
                .collect(),
        );
        let seed = standardize(&input, 2, 0.035).unwrap();
        for y in 2..height - 2 {
            for x in (2..width - 2).step_by(5) {
                let index = y * width + x;
                assert!(seed.usable[index]);
                assert_eq!(seed.values[index], 0.0);
                assert!(!GrayClass::Dark.contains(seed.values[index]));
                assert!(!GrayClass::Light.contains(seed.values[index]));
            }
        }

        let constant = capture(width, height, vec![128; width * height]);
        let seed = standardize(&constant, 2, 0.0).unwrap();
        assert!(seed.usable.iter().all(|&usable| !usable));
    }

    #[test]
    fn missing_or_reversed_classes_do_not_create_a_palette() {
        let input = capture(
            5,
            5,
            (0..25)
                .map(|i| if i % 2 == 0 { 200 } else { 100 })
                .collect(),
        );
        for values in [
            vec![-1.0; 25],
            (0..25)
                .map(|i| if i % 2 == 0 { -1.0 } else { 1.0 })
                .collect(),
        ] {
            let seed = Normalized {
                values,
                usable: vec![true; 25],
            };
            let palette = estimate_palette(&input, &seed, 1).unwrap();
            assert!(palette.usable.iter().all(|&usable| !usable));
            assert!(
                palette
                    .ridge_gray
                    .iter()
                    .chain(&palette.valley_gray)
                    .all(|&value| value == 0.0)
            );
        }
    }
}
