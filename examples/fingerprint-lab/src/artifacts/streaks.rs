//! Finite seeded scratches, cut bands and dark streaks on an altered capture.

use image::GrayImage;

use super::{ScenarioRandom, pixel, smoothstep};
use crate::Result;

const MAX_STREAKS: usize = 144;
const MAX_SAMPLES: usize = 2048;

pub(super) fn apply(image: &mut GrayImage, unit: f64, strength: f64, seed: u64) -> Result<()> {
    if image.width() == 0 || image.height() == 0 {
        return Err("streak input must have nonzero dimensions".into());
    }
    if !unit.is_finite() || unit <= 0.0 || !strength.is_finite() || !(0.0..=1.0).contains(&strength)
    {
        return Err("streak scale must be positive and strength must be in [0, 1]".into());
    }
    if strength == 0.0 {
        return Ok(());
    }

    let unit = unit.min((f64::from(image.width().min(image.height())) / 12.0).max(1.0));
    let area = f64::from(image.width()) * f64::from(image.height());
    let density = (area / (unit * unit * 120.0))
        .ceil()
        .clamp(20.0, MAX_STREAKS as f64);
    let count = (density * strength.powf(1.3)).ceil() as usize;
    let mut random = ScenarioRandom(seed ^ 0x29b9_7e3a_8f5d_c641);
    let style_phase = (random.next() % 6) as usize;
    for index in 0..count {
        let stroke = Stroke::random(image, unit, index + style_phase, &mut random);
        render(image, stroke, strength)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Stroke {
    start: [f64; 2],
    control: [f64; 2],
    end: [f64; 2],
    radius: f64,
    start_width: f64,
    end_width: f64,
    irregularity: f64,
    phase: f64,
    target: f64,
    opacity: f64,
    style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Sharp,
    Blurred,
    Textured(u64),
}

impl Stroke {
    fn random(image: &GrayImage, unit: f64, index: usize, random: &mut ScenarioRandom) -> Self {
        let center = [
            random.between(0.0, f64::from(image.width())),
            random.between(0.0, f64::from(image.height())),
        ];
        let angle = random.between(0.0, std::f64::consts::TAU);
        let (sine, cosine) = angle.sin_cos();
        let diagonal = f64::from(image.width()).hypot(f64::from(image.height()));
        let broad = index.is_multiple_of(5);
        let length = (unit * random.between(4.0, if broad { 28.0 } else { 18.0 }))
            .min(diagonal * 0.55)
            .max(1.0);
        let bend = length * random.between(-0.28, 0.28);
        Self {
            start: [
                center[0] - cosine * length / 2.0,
                center[1] - sine * length / 2.0,
            ],
            control: [center[0] - sine * bend, center[1] + cosine * bend],
            end: [
                center[0] + cosine * length / 2.0,
                center[1] + sine * length / 2.0,
            ],
            radius: unit / 2.0,
            start_width: random.between(0.25, 1.0),
            end_width: random.between(0.25, 1.0),
            irregularity: random.between(2.0, 7.0),
            phase: random.between(0.0, std::f64::consts::TAU),
            target: if index.is_multiple_of(2) { 255.0 } else { 0.0 },
            opacity: random.between(0.85, 1.0),
            style: match index % 3 {
                0 => Style::Sharp,
                1 => Style::Blurred,
                _ => Style::Textured(random.next()),
            },
        }
    }

    fn position(self, t: f64) -> [f64; 2] {
        let u = 1.0 - t;
        [
            u * u * self.start[0] + 2.0 * u * t * self.control[0] + t * t * self.end[0],
            u * u * self.start[1] + 2.0 * u * t * self.control[1] + t * t * self.end[1],
        ]
    }

    /// Nominal half-width; antialiased sharp edges span at most another 0.5 px.
    fn width(self, t: f64) -> f64 {
        let taper = self.start_width + (self.end_width - self.start_width) * t;
        let irregular = 0.12 * (self.phase + t * self.irregularity * std::f64::consts::TAU).sin();
        self.radius * (taper + irregular).clamp(0.25, 1.0)
    }

    fn bounds(self, width: u32, height: u32) -> [u32; 4] {
        let x = [self.start[0], self.control[0], self.end[0]];
        let y = [self.start[1], self.control[1], self.end[1]];
        [
            (x.into_iter().fold(f64::INFINITY, f64::min) - self.radius)
                .floor()
                .clamp(0.0, f64::from(width)) as u32,
            (y.into_iter().fold(f64::INFINITY, f64::min) - self.radius)
                .floor()
                .clamp(0.0, f64::from(height)) as u32,
            (x.into_iter().fold(f64::NEG_INFINITY, f64::max) + self.radius + 1.0)
                .ceil()
                .clamp(0.0, f64::from(width)) as u32,
            (y.into_iter().fold(f64::NEG_INFINITY, f64::max) + self.radius + 1.0)
                .ceil()
                .clamp(0.0, f64::from(height)) as u32,
        ]
    }
}

fn render(image: &mut GrayImage, stroke: Stroke, strength: f64) -> Result<()> {
    let [left, top, right, bottom] = stroke.bounds(image.width(), image.height());
    let width = (right - left) as usize;
    let height = (bottom - top) as usize;
    let size = width
        .checked_mul(height)
        .ok_or("streak mask dimensions overflow")?;
    let mut mask = vec![0.0_f32; size];
    let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let length = distance(stroke.start, stroke.control) + distance(stroke.control, stroke.end);
    let samples = (length / (stroke.radius * 0.075).max(0.25))
        .ceil()
        .clamp(2.0, MAX_SAMPLES as f64) as usize;
    for index in 0..=samples {
        let t = index as f64 / samples as f64;
        stamp(
            &mut mask,
            [left, top, right, bottom],
            stroke.position(t),
            stroke.width(t),
            smoothstep((t.min(1.0 - t) / 0.045).min(1.0)),
            stroke.style,
        );
    }
    for (index, coverage) in mask.into_iter().enumerate() {
        if coverage == 0.0 {
            continue;
        }
        let x = left + (index % width) as u32;
        let y = top + (index / width) as u32;
        let original = f64::from(image.get_pixel(x, y).0[0]);
        let alpha = strength * stroke.opacity * f64::from(coverage);
        image.put_pixel(x, y, pixel(original + (stroke.target - original) * alpha));
    }
    Ok(())
}

fn stamp(
    mask: &mut [f32],
    bounds: [u32; 4],
    center: [f64; 2],
    radius: f64,
    opacity: f64,
    style: Style,
) {
    let [left, top, right, bottom] = bounds;
    let x0 = (center[0] - radius)
        .floor()
        .clamp(f64::from(left), f64::from(right)) as u32;
    let y0 = (center[1] - radius)
        .floor()
        .clamp(f64::from(top), f64::from(bottom)) as u32;
    let x1 = (center[0] + radius + 1.0)
        .ceil()
        .clamp(f64::from(left), f64::from(right)) as u32;
    let y1 = (center[1] + radius + 1.0)
        .ceil()
        .clamp(f64::from(top), f64::from(bottom)) as u32;
    let width = (right - left) as usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let distance = (f64::from(x) - center[0]).hypot(f64::from(y) - center[1]);
            let sharp = smoothstep(radius + 0.5 - distance);
            let coverage = match style {
                Style::Sharp => sharp,
                Style::Blurred => smoothstep((radius - distance) / radius),
                Style::Textured(seed) => sharp * (0.1 + 0.9 * texture(x, y, seed)),
            } * opacity;
            let index = (y - top) as usize * width + (x - left) as usize;
            mask[index] = mask[index].max(coverage as f32);
        }
    }
}

fn texture(x: u32, y: u32, seed: u64) -> f64 {
    let noise = |x, y| ScenarioRandom(seed ^ (u64::from(x) << 32) ^ u64::from(y)).unit();
    0.65 * noise(x / 2, y / 2) + 0.35 * noise(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    #[test]
    fn zero_is_exact_and_invalid_parameters_fail() {
        let source = GrayImage::from_pixel(160, 120, Luma([128]));
        let mut unchanged = source.clone();
        apply(&mut unchanged, 4.0, 0.0, 3).unwrap();
        assert_eq!(unchanged, source);
        for strength in [f64::NAN, -0.1, 1.1] {
            assert!(apply(&mut unchanged, 4.0, strength, 3).is_err());
        }
        assert!(apply(&mut unchanged, 0.0, 1.0, 3).is_err());
        assert!(apply(&mut GrayImage::new(0, 1), 1.0, 1.0, 3).is_err());
    }

    #[test]
    fn strokes_are_seeded_and_include_white_and_black() {
        let source = GrayImage::from_pixel(160, 120, Luma([128]));
        let mut first = source.clone();
        let mut same = source.clone();
        let mut different = source.clone();
        apply(&mut first, 4.0, 1.0, 19).unwrap();
        apply(&mut same, 4.0, 1.0, 19).unwrap();
        apply(&mut different, 4.0, 1.0, 20).unwrap();
        assert_eq!(first, same);
        assert_ne!(first, different);
        assert!(first.as_raw().iter().filter(|&&value| value > 210).count() > 20);
        assert!(first.as_raw().iter().filter(|&&value| value < 45).count() > 20);
        assert_eq!(first.dimensions(), source.dimensions());
    }

    #[test]
    fn a_cut_is_finite_curved_and_keeps_remote_pixels() {
        let mut image = GrayImage::from_pixel(100, 60, Luma([100]));
        let stroke = Stroke {
            start: [15.0, 20.0],
            control: [50.0, 40.0],
            end: [85.0, 20.0],
            radius: 3.0,
            start_width: 0.7,
            end_width: 0.9,
            irregularity: 3.0,
            phase: 0.4,
            target: 255.0,
            opacity: 1.0,
            style: Style::Sharp,
        };
        render(&mut image, stroke, 1.0).unwrap();
        assert_eq!(image.get_pixel(0, 0).0[0], 100);
        assert_eq!(image.get_pixel(50, 20).0[0], 100);
        assert!(image.get_pixel(50, 30).0[0] > 200);
        assert!((20..80).all(|x| (20..40).any(|y| image.get_pixel(x, y).0[0] > 120)));
    }

    #[test]
    fn severity_adds_more_cuts_and_small_images_remain_bounded() {
        let source = GrayImage::from_pixel(160, 120, Luma([128]));
        let mut low = source.clone();
        let mut high = source.clone();
        apply(&mut low, 4.0, 0.15, 27).unwrap();
        apply(&mut high, 4.0, 1.0, 27).unwrap();
        let changed = |image: &GrayImage| {
            image
                .as_raw()
                .iter()
                .filter(|&&value| value.abs_diff(128) > 10)
                .count()
        };
        assert!(changed(&high) > changed(&low) * 3);
        for size in [(1, 1), (2, 9), (13, 2)] {
            let mut image = GrayImage::from_pixel(size.0, size.1, Luma([128]));
            apply(&mut image, 1e100, 1.0, 42).unwrap();
            assert_eq!(image.dimensions(), size);
        }
    }

    #[test]
    fn nominal_width_stays_between_a_quarter_ridge_and_one_ridge() {
        let image = GrayImage::new(160, 120);
        let unit = 6.0;
        let mut random = ScenarioRandom(77);
        for index in 0..MAX_STREAKS {
            let stroke = Stroke::random(&image, unit, index, &mut random);
            for step in 0..=100 {
                let width = 2.0 * stroke.width(f64::from(step) / 100.0);
                assert!((unit / 4.0..=unit).contains(&width));
            }
        }
    }

    #[test]
    fn sharp_blurred_and_textured_lines_have_distinct_reproducible_profiles() {
        let stroke = Stroke {
            start: [10.0, 25.0],
            control: [50.0, 25.0],
            end: [90.0, 25.0],
            radius: 6.0,
            start_width: 1.0,
            end_width: 1.0,
            irregularity: 0.0,
            phase: 0.0,
            target: 255.0,
            opacity: 1.0,
            style: Style::Sharp,
        };
        let render_style = |style| {
            let mut image = GrayImage::from_pixel(100, 50, Luma([100]));
            render(&mut image, Stroke { style, ..stroke }, 1.0).unwrap();
            image
        };
        let sharp = render_style(Style::Sharp);
        let blurred = render_style(Style::Blurred);
        let textured = render_style(Style::Textured(23));
        assert_ne!(sharp, blurred);
        assert_ne!(sharp, textured);
        assert_ne!(blurred, textured);
        assert_eq!(textured, render_style(Style::Textured(23)));
        assert_ne!(textured, render_style(Style::Textured(24)));
        assert!(blurred.get_pixel(50, 29).0[0] < sharp.get_pixel(50, 29).0[0]);
        let center_values: Vec<_> = (20..80).map(|x| blurred.get_pixel(x, 25).0[0]).collect();
        assert!(center_values.iter().max().unwrap() - center_values.iter().min().unwrap() <= 2);
    }
}
