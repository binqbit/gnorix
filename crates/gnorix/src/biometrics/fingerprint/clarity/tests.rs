use super::*;
use std::f64::consts::PI;

fn capture(width: usize, height: usize, pixel: impl Fn(f64, f64) -> f64) -> Capture {
    let values = (0..width * height)
        .map(|i| {
            pixel((i % width) as f64, (i / width) as f64)
                .round()
                .clamp(0.0, 255.0) as u8
        })
        .collect();
    Capture::new(width as u32, height as u32, values, None, None).unwrap()
}

fn stripe(position: f64, period: f64, dark_fraction: f64, dark: f64, light: f64) -> f64 {
    if position.rem_euclid(period) < period * dark_fraction {
        dark
    } else {
        light
    }
}

fn center(input: &Capture, period: f64) -> Components {
    Sampler::new(input, period, 1)
        .unwrap()
        .measure(input.width() as usize / 2, input.height() as usize / 2)
        .unwrap()
}

fn value(component: Components) -> f64 {
    component.contrast * component.separation * component.repetition
}

#[test]
fn sharp_ridges_exceed_blurred_weak_and_matched_contrast_random_pixels() {
    let period = 24.0;
    let sharp = capture(144, 144, |x, _| stripe(x + 3.0, period, 0.4, 35.0, 215.0));
    let weak = capture(144, 144, |x, _| stripe(x + 3.0, period, 0.4, 120.0, 130.0));
    let blurred = capture(144, 144, |x, _| {
        let sigma = period / 8.0;
        let (mut sum, mut mass) = (0.0, 0.0);
        for offset in -12..=12 {
            let weight = (-0.5 * (offset as f64 / sigma).powi(2)).exp();
            sum += weight * stripe(x + 3.0 + offset as f64, period, 0.4, 35.0, 215.0);
            mass += weight;
        }
        sum / mass
    });
    let noise = capture(144, 144, |x, y| {
        let mut bits = (x as u64).wrapping_mul(0x9e3779b97f4a7c15)
            ^ (y as u64).wrapping_mul(0xbf58476d1ce4e5b9);
        bits ^= bits >> 30;
        bits = bits.wrapping_mul(0xbf58476d1ce4e5b9);
        bits ^= bits >> 27;
        if bits & 1 == 0 { 35.0 } else { 215.0 }
    });
    let clear = value(center(&sharp, period));
    let soft = value(center(&blurred, period));
    let low = value(center(&weak, period));
    let random = value(center(&noise, period));
    assert!(clear > 0.5, "clear={clear}");
    assert!(soft < clear * 0.8, "clear={clear}, blurred={soft}");
    assert!(low < clear * 0.1, "clear={clear}, weak={low}");
    assert!(random < clear * 0.15, "clear={clear}, noise={random}");
}

#[test]
fn uniform_illumination_ramps_edges_and_merged_centers_do_not_look_like_clear_ridges() {
    let period = 24.0;
    for input in [
        capture(144, 144, |_, _| 100.0),
        capture(144, 144, |x, y| 20.0 + x + y / 3.0),
        capture(144, 144, |x, _| if x < 72.0 { 35.0 } else { 215.0 }),
        capture(144, 144, |x, _| {
            if (x - 72.0).abs() < 0.65 * period {
                35.0
            } else {
                stripe(x, period, 0.4, 35.0, 215.0)
            }
        }),
    ] {
        let result = center(&input, period);
        assert!(
            value(result) < 0.015,
            "contrast={}, separation={}, repetition={}",
            result.contrast,
            result.separation,
            result.repetition
        );
    }
}

#[test]
fn spatial_labels_keep_dark_width_independent_of_the_period() {
    for fraction in [0.2, 0.35, 0.65, 0.8] {
        let input = capture(160, 160, |x, _| {
            stripe(x + 3.0, 32.0, fraction, 40.0, 210.0)
        });
        let result = center(&input, 32.0);
        assert!(
            result.contrast > 0.55,
            "fraction={fraction}, contrast={}",
            result.contrast
        );
        assert!(
            result.repetition > 0.9,
            "fraction={fraction}, repetition={}",
            result.repetition
        );
        assert!(
            value(result) > 0.4,
            "fraction={fraction}, value={}",
            value(result)
        );
    }
}

#[test]
fn additive_illumination_offsets_and_slopes_preserve_source_separability() {
    let original = capture(160, 160, |x, _| stripe(x + 3.0, 24.0, 0.4, 40.0, 160.0));
    let shifted = capture(160, 160, |x, y| {
        stripe(x + 3.0, 24.0, 0.4, 40.0, 160.0) + 20.0 + x * 0.1 - y * 0.05
    });
    let (a, b) = (center(&original, 24.0), center(&shifted, 24.0));
    assert!(
        (value(a) - value(b)).abs() < 0.025,
        "{} vs {}",
        value(a),
        value(b)
    );
    assert!((a.contrast - b.contrast).abs() < 0.02);
    assert!((a.separation - b.separation).abs() < 0.025);
}

#[test]
fn rotations_and_rescaled_resolvable_ridges_remain_comparable() {
    let mut values = Vec::new();
    for period in [16.0, 24.0, 32.0] {
        for angle in [0.0_f64, PI / 5.0, PI / 2.0] {
            let input = capture(192, 192, |x, y| {
                let u = (x - 96.0) * angle.cos() + (y - 96.0) * angle.sin();
                125.0 + 85.0 * (2.0 * PI * u / period).cos()
            });
            values.push(value(center(&input, period)));
        }
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(0.0, f64::max);
    assert!(min > 0.25, "{values:?}");
    assert!(max - min < 0.08, "{values:?}");
}

#[test]
fn curved_and_locally_multidirectional_patterns_have_no_categorical_coherence_veto() {
    let curved = capture(160, 160, |x, y| {
        let radius = (x - 80.0).hypot(y - 80.0);
        stripe(radius + 2.0, 16.0, 0.4, 35.0, 215.0)
    });
    let sampler = Sampler::new(&curved, 16.0, 2).unwrap();
    let bend = sampler.measure(112, 80).unwrap();
    let center = sampler.measure(80, 80).unwrap();
    assert!(value(bend) > 0.35, "bend={}", value(bend));
    assert!(value(center) > 0.03, "center={}", value(center));
}

#[test]
fn map_distinguishes_measured_zero_from_missing_support_and_redacts_debug() {
    let image = capture(96, 96, |_, _| 100.0);
    let map = assess(&image, 16.0).unwrap();
    assert_eq!(map.width(), 96);
    assert_eq!(map.height(), 96);
    assert_eq!(map.grid_spacing_px, 4);
    assert_eq!(map.sample(48.0, 48.0), Some(0.0));
    assert_eq!(map.sample(0.0, 0.0), None);
    assert_eq!(map.sample(f64::NAN, 48.0), None);
    assert_eq!(map.sample(-0.1, 48.0), None);
    assert!(format!("{map:?}").contains("[REDACTED]"));
}

#[test]
fn hidden_capture_pixels_do_not_change_any_component_or_bleed_into_the_map() {
    let source = capture(96, 96, |x, _| stripe(x, 16.0, 0.4, 35.0, 215.0));
    let mut mask = vec![true; source.pixels().len()];
    mask[48 * 96 + 48] = false;
    let mut pixels = source.pixels().to_vec();
    pixels[48 * 96 + 48] = 0;
    let first = Capture::new(96, 96, pixels.clone(), Some(mask.clone()), None).unwrap();
    pixels[48 * 96 + 48] = 255;
    let second = Capture::new(96, 96, pixels, Some(mask), None).unwrap();
    let a = assess(&first, 16.0).unwrap();
    let b = assess(&second, 16.0).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.sample(48.0, 48.0), None);
    assert!(a.supported.iter().any(|&value| value));
}

#[test]
fn dense_maps_are_deterministic_bounded_and_keep_anchor_products() {
    let source = capture(96, 96, |x, y| {
        120.0 + 75.0 * (2.0 * PI * (0.8 * x + 0.6 * y) / 16.0).cos()
    });
    let first = assess(&source, 16.0).unwrap();
    assert_eq!(first, assess(&source, 16.0).unwrap());
    for index in 0..first.values.len() {
        for value in [
            first.values[index],
            first.contrast[index],
            first.separation[index],
            first.repetition[index],
        ] {
            assert!(value.is_finite() && (0.0..=1.0).contains(&value));
        }
        if (index % first.width).is_multiple_of(first.grid_spacing_px)
            && (index / first.width).is_multiple_of(first.grid_spacing_px)
        {
            let expected = f64::from(first.contrast[index])
                * f64::from(first.separation[index])
                * f64::from(first.repetition[index]);
            assert!((f64::from(first.values[index]) - expected).abs() < 1e-7);
        }
        if !first.supported[index] {
            assert_eq!(first.values[index], 0.0);
        }
    }
}

#[test]
fn interpolating_complementary_weak_anchors_does_not_create_a_strong_peak() {
    let low_separation = Components {
        contrast: 1.0,
        separation: 0.1,
        repetition: 0.2,
    };
    let low_contrast = Components {
        contrast: 0.1,
        separation: 1.0,
        repetition: 1.0,
    };
    let measured = [
        Some(low_separation),
        Some(low_contrast),
        Some(low_contrast),
        Some(low_separation),
    ];
    let source = capture(4, 4, |_, _| 100.0);
    let mut map = numerical_map(|_, _| 0.0);
    map.supported.fill(false);
    interpolate_anchors(&mut map, &source, &[0, 3], &[0, 3], &measured);

    for y in 0..4 {
        for x in 0..4 {
            let expected = bilinear([0.02, 0.1, 0.1, 0.02], x as f64 / 3.0, y as f64 / 3.0);
            let actual = map.sample(x as f64, y as f64).unwrap();
            assert!((actual - expected).abs() < 1e-8);
            assert!((0.02 - 1e-8..=0.1 + 1e-8).contains(&actual));
        }
    }
    assert!((map.sample(1.5, 1.5).unwrap() - 0.06).abs() < 1e-8);

    // Missing evidence is still unknown, even with three valid neighbors.
    let mut missing = numerical_map(|_, _| 0.0);
    missing.supported.fill(false);
    let mut measured = measured;
    measured[3] = None;
    interpolate_anchors(&mut missing, &source, &[0, 3], &[0, 3], &measured);
    assert_eq!(missing.sample(1.5, 1.5), None);
    assert!(missing.sample(0.0, 0.0).is_some());
}

fn numerical_map(value: impl Fn(usize, usize) -> f32) -> ClarityMap {
    ClarityMap {
        width: 4,
        height: 4,
        values: (0..16).map(|i| value(i % 4, i / 4)).collect(),
        contrast: vec![0.0; 16],
        separation: vec![0.0; 16],
        repetition: vec![0.0; 16],
        supported: vec![true; 16],
        period_px: 4.0,
        grid_spacing_px: 1,
    }
}

#[test]
fn segment_means_integrate_the_bilinear_map_exactly_and_handle_zero_length() {
    let map = numerical_map(|x, y| ((x + 1) * (y + 1)) as f32 / 16.0);
    let a = Point::new(0.2, 0.3).unwrap();
    let b = Point::new(2.7, 2.6).unwrap();
    let (dx, dy) = (b.x() - a.x(), b.y() - a.y());
    let expected = ((a.x() + 1.0) * (a.y() + 1.0)
        + ((a.x() + 1.0) * dy + (a.y() + 1.0) * dx) / 2.0
        + dx * dy / 3.0)
        / 16.0;
    assert!((map.segment_mean(a, b).unwrap() - expected).abs() < 1e-12);
    assert!((map.segment_mean(b, a).unwrap() - expected).abs() < 1e-12);
    assert_eq!(map.segment_mean(a, a), map.sample(a.x(), a.y()));
    let constant = numerical_map(|_, _| 0.5);
    assert_eq!(constant.segment_mean(a, b), Some(0.5));
}

#[test]
fn segment_support_checks_tiny_oblique_cell_clips_instead_of_only_midpoints() {
    let mut map = numerical_map(|_, _| 0.5);
    map.supported[2 * 4] = false;
    let a = Point::new(0.95, 0.96).unwrap();
    let b = Point::new(1.15, 1.159).unwrap();
    assert_eq!(map.sample(a.x(), a.y()), Some(0.5));
    assert_eq!(map.sample(b.x(), b.y()), Some(0.5));
    assert_eq!(
        map.sample((a.x() + b.x()) / 2.0, (a.y() + b.y()) / 2.0),
        Some(0.5)
    );
    assert_eq!(map.segment_mean(a, b), None);
    let a = Point::new(0.0, 0.0).unwrap();
    let b = Point::new(1.4999999999999998, 1.5).unwrap();
    assert_eq!(map.segment_mean(a, b), None);
    assert_eq!(map.segment_mean(b, a), None);
    // Coincident rounded crossings conservatively check both corner neighbors.
    let b = Point::new(1.5, 1.5).unwrap();
    assert_eq!(map.segment_mean(a, b), None);
}

#[test]
fn invalid_periods_and_excessive_work_are_explicit_errors() {
    let image = capture(16, 16, |_, _| 100.0);
    for period in [0.0, 3.9, 128.1, f64::NAN, f64::INFINITY] {
        assert!(assess(&image, period).is_err());
    }
    assert!(matches!(
        Sampler::new(&image, 16.0, usize::MAX),
        Err(FingerprintError::WorkLimit)
    ));
}
