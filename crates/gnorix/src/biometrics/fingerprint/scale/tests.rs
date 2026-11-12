use super::*;
use std::f64::consts::PI;

fn config() -> ExtractionConfig {
    ExtractionConfig {
        bootstrap_grid_locations: 128,
        ..ExtractionConfig::default()
    }
}

fn stripes(
    dimensions: (usize, usize),
    period: f64,
    dark_width: f64,
    angle: f64,
    phase: f64,
    noise: f64,
    illumination: bool,
) -> Capture {
    let (width, height) = dimensions;
    let (sin, cos) = angle.sin_cos();
    let mut random = 7_u64;
    let pixels = (0..width * height)
        .map(|i| {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let grain = ((random >> 32) as f64 / f64::from(u32::MAX) - 0.5) * noise;
            let (x, y) = ((i % width) as f64, (i / width) as f64);
            let distance = ((x * cos + y * sin + phase + period / 2.0).rem_euclid(period)
                - period / 2.0)
                .abs();
            let intensity = 128.0
                + 105.0 * ((distance - dark_width / 2.0) / 0.4).tanh()
                + grain
                + if illumination {
                    30.0 * (x / width as f64 - 0.5) + 15.0 * (y / height as f64 - 0.5)
                } else {
                    0.0
                };
            intensity.round().clamp(0.0, 255.0) as u8
        })
        .collect();
    Capture::new(width as u32, height as u32, pixels, None, None).unwrap()
}

#[test]
fn measures_independent_width_and_period_over_angles_phase_noise_and_light() {
    for (angle, phase) in [(0.0, 0.0), (0.31, 0.7), (PI / 4.0, 2.3), (PI / 2.0, 0.4)] {
        let result = estimate(
            &stripes((180, 180), 20.0, 6.0, angle, phase, 6.0, true),
            &config(),
        )
        .unwrap();
        assert!(
            (result.ridge_width_px - 6.0).abs() < 0.7,
            "angle {angle}: width {}",
            result.ridge_width_px
        );
        assert!(
            (result.ridge_period_px - 20.0).abs() < 1.0,
            "angle {angle}: period {}",
            result.ridge_period_px
        );
        assert!((result.valley_width_px - 14.0).abs() < 1.2);
        assert!((result.ridge_period_px - 2.0 * result.ridge_width_px).abs() > 5.0);
        assert!(result.examined_locations <= 128 && !result.profiles.is_empty());
    }
}

#[test]
fn three_complete_dark_ridges_are_required_and_edge_fragments_do_not_count() {
    for (dark_centers, accepted) in [
        (&[30.0, 50.0][..], false),
        (&[30.0, 50.0, 70.0][..], true),
        (&[0.0, 30.0, 50.0][..], false),
    ] {
        let pixels = (0..100 * 40)
            .map(|i| {
                let distance = dark_centers
                    .iter()
                    .map(|&x| ((i % 100) as f64 - x).abs())
                    .fold(f64::INFINITY, f64::min);
                if distance < 3.0 { 20 } else { 230 }
            })
            .collect();
        let input = Capture::new(100, 40, pixels, None, None).unwrap();
        let result = estimate(&input, &config());
        assert_eq!(
            result.is_ok(),
            accepted,
            "centers {dark_centers:?}: {result:?}"
        );
    }
}

#[test]
fn unavailable_scale_never_uses_a_default_or_clamps_an_unsupported_period() {
    let blank = Capture::new(128, 128, vec![127; 128 * 128], None, None).unwrap();
    assert!(matches!(
        estimate(&blank, &config()),
        Err(FingerprintError::ScaleUnavailable(_))
    ));
    let masked = Capture::new(
        128,
        128,
        vec![127; 128 * 128],
        Some(vec![false; 128 * 128]),
        None,
    )
    .unwrap();
    assert!(matches!(
        estimate(&masked, &config()),
        Err(FingerprintError::ScaleUnavailable(_))
    ));
    for period in [2.0, 240.0] {
        let input = stripes((900, 128), period, period / 3.0, 0.0, 0.0, 0.0, false);
        assert!(
            matches!(
                estimate(&input, &config()),
                Err(FingerprintError::ScaleUnavailable(_))
            ),
            "period {period}"
        );
    }
}

#[test]
fn source_pixel_geometry_and_measurements_scale_under_resizing() {
    let a = estimate(
        &stripes((160, 160), 16.0, 5.0, 0.21, 0.3, 0.0, false),
        &config(),
    )
    .unwrap();
    let b = estimate(
        &stripes((320, 320), 32.0, 10.0, 0.21, 0.6, 0.0, false),
        &config(),
    )
    .unwrap();
    assert!((b.ridge_period_px / a.ridge_period_px - 2.0).abs() < 0.08);
    assert!((b.ridge_width_px / a.ridge_width_px - 2.0).abs() < 0.15);
    for result in [&a, &b] {
        let widths: Vec<_> = result
            .profiles
            .iter()
            .flat_map(|p| p.ridges.iter().map(|r| distance(r.start, r.end)))
            .collect();
        assert_eq!(median(&mut widths.clone()), result.ridge_width_px);
    }
}

#[test]
fn measures_the_supported_period_endpoints_without_a_scale_hint() {
    // At the lower supported period use a resolved, symmetric half-local-
    // contrast width. An underlying sub-two-pixel physical stripe need not
    // reach its continuous dark extremum at the sampled pixel phase.
    for (dimensions, period, width) in [((128, 96), 4.0, 2.0), ((800, 200), 128.0, 32.0)] {
        let result = estimate(
            &stripes(dimensions, period, width, 0.0, 0.3, 0.0, false),
            &config(),
        )
        .unwrap();
        assert!(
            (result.ridge_period_px - period).abs() < 0.03 * period,
            "period {period}: {}",
            result.ridge_period_px
        );
        assert!(
            (result.ridge_width_px - width).abs() < 0.15 * width,
            "width {width}: {}",
            result.ridge_width_px
        );
    }
}

#[test]
fn undersampled_stripe_width_uses_observed_half_contrast_not_latent_thickness() {
    let input = stripes((128, 96), 4.0, 1.5, 0.0, 0.3, 0.0, false);
    // Independently tabulated source pixels for this analytic stripe. The
    // nearest sampled trough is43, not the latent continuous dark extreme.
    assert_eq!(&input.pixels()[..4], &[43, 220, 231, 115]);
    let pixels = [43.0, 220.0, 231.0, 115.0];
    // One centered site is x=63.5, so the refinement's half-step samples
    // begin at x=54.5 (phase2.5 modulo4). Construct one periodic profile
    // directly from those four source samples, without calling any scale
    // profile, smoothing, extrema or crossing implementation helpers.
    let sampled: [f64; 8] = std::array::from_fn(|i| {
        let x = 2.5 + 0.5 * i as f64;
        let left = x.floor() as usize;
        let fraction = x - left as f64;
        pixels[left % 4] * (1.0 - fraction) + pixels[(left + 1) % 4] * fraction
    });
    // σ=P/32=.125px=.25samples: truncate3σ gives the exact3-tap kernel.
    let neighbor = (-8.0_f64).exp();
    let observed: [f64; 8] = std::array::from_fn(|i| {
        (neighbor * sampled[(i + 7) % 8] + sampled[i] + neighbor * sampled[(i + 1) % 8])
            / (1.0 + 2.0 * neighbor)
    });
    let darkest = (0..8)
        .min_by(|&a, &b| observed[a].total_cmp(&observed[b]))
        .unwrap();
    let brightest = observed.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let half = (observed[darkest] + brightest) / 2.0;
    let value = |i: isize| observed[i.rem_euclid(8) as usize];
    let mut left = darkest as isize;
    while value(left) < half {
        left -= 1;
    }
    let mut right = darkest as isize;
    while value(right) < half {
        right += 1;
    }
    let a = left as f64 + (value(left) - half) / (value(left) - value(left + 1));
    let b = (right - 1) as f64 + (half - value(right - 1)) / (value(right) - value(right - 1));
    let expected = (b - a) * 0.5;
    assert!((expected - 1.720844863948994).abs() < 1e-12);
    let one = estimate(
        &input,
        &ExtractionConfig {
            bootstrap_grid_locations: 1,
            ..config()
        },
    )
    .unwrap();
    assert!((one.ridge_width_px - expected).abs() < 1e-8);
    assert!((one.ridge_period_px - 4.0).abs() < 1e-8);
    // Preserve the originally discovered multi-site phase case as well.
    // Its median samples several shifted half-pixel profiles and therefore
    // differs from both this one-site oracle and the unobservable latent1.5.
    let multiple = estimate(&input, &config()).unwrap();
    assert!((multiple.ridge_width_px - 1.8044967114337922).abs() < 1e-8);
    assert!((multiple.ridge_period_px - 4.0).abs() < 1e-8);
}

#[test]
fn multiple_measured_thickness_groups_remain_in_the_output() {
    let source = (0..180 * 180)
        .map(|i| {
            let dark_width = if i / 180 < 90 { 4.0 } else { 10.0 };
            let distance = (((i % 180) as f64 + 10.0).rem_euclid(20.0) - 10.0).abs();
            (128.0 + 105.0 * ((distance - dark_width / 2.0) / 0.4).tanh())
                .round()
                .clamp(0.0, 255.0) as u8
        })
        .collect();
    let input = Capture::new(180, 180, source, None, None).unwrap();
    let result = estimate(&input, &config()).unwrap();
    let mut widths: Vec<_> = result
        .profiles
        .iter()
        .flat_map(|p| p.ridges.iter().map(|r| distance(r.start, r.end)))
        .collect();
    assert!(widths.iter().any(|&w| w < 5.0));
    assert!(widths.iter().any(|&w| w > 9.0));
    assert_eq!(median(&mut widths), result.ridge_width_px);
}

#[test]
fn a_mask_cut_is_not_bridged_and_hidden_values_do_not_change_measurements() {
    let original = stripes((160, 120), 20.0, 6.0, 0.0, 1.0, 0.0, false);
    let mut mask = vec![true; 160 * 120];
    for y in 0..120 {
        for x in 74..87 {
            mask[y * 160 + x] = false;
        }
    }
    let mut a = original.pixels().to_vec();
    let mut b = a.clone();
    for i in 0..a.len() {
        if !mask[i] {
            a[i] = 0;
            b[i] = 255;
        }
    }
    let first = Capture::new(160, 120, a, Some(mask.clone()), None).unwrap();
    let second = Capture::new(160, 120, b, Some(mask), None).unwrap();
    let a = estimate(&first, &config()).unwrap();
    let b = estimate(&second, &config()).unwrap();
    assert_eq!(a, b);
    for p in &a.profiles {
        let min = p
            .ridges
            .iter()
            .map(|r| r.start.x().min(r.end.x()))
            .fold(f64::INFINITY, f64::min);
        let max = p
            .ridges
            .iter()
            .map(|r| r.start.x().max(r.end.x()))
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(max < 74.0 || min > 86.0, "triple crosses unobserved mask");
    }
}

#[test]
fn all_confirmed_width_groups_contribute_to_the_median_and_mad() {
    let profile = |widths: [f64; 3]| ScaleProfile {
        ridges: std::array::from_fn(|i| {
            let center = 20.0 * i as f64;
            RidgeSpan {
                start: Point::new(center - widths[i] / 2.0, 5.0).unwrap(),
                end: Point::new(center + widths[i] / 2.0, 5.0).unwrap(),
            }
        }),
        ridge_mean_gray: [32.0; 3],
        valley_mean_gray: [224.0; 2],
    };
    let result = summarize(
        vec![profile([2.0, 2.0, 2.0]), profile([10.0, 10.0, 18.0])],
        2,
    )
    .unwrap();
    assert_eq!(result.ridge_width_px, 6.0);
    assert_eq!(result.ridge_width_mad_px, 4.0);
    assert_eq!(result.ridge_period_px, 20.0);
    assert_eq!(result.profiles.len(), 2);
    assert_eq!(result.mean_ridge_width_px, 44.0 / 6.0);
    assert_eq!(result.mean_valley_width_px, 13.0);
    assert_eq!(result.mean_ridge_gray, 32.0);
    assert_eq!(result.mean_valley_gray, 224.0);
}

#[test]
fn photometry_is_length_weighted_while_width_means_count_every_span() {
    let profiles = [(2.0, 0.0, 100.0), (10.0, 200.0, 200.0)]
        .into_iter()
        .map(|(width, ridge_gray, valley_gray)| ScaleProfile {
            ridges: std::array::from_fn(|i| RidgeSpan {
                start: Point::new(i as f64 * 20.0 - width / 2.0, 5.0).unwrap(),
                end: Point::new(i as f64 * 20.0 + width / 2.0, 5.0).unwrap(),
            }),
            ridge_mean_gray: [ridge_gray; 3],
            valley_mean_gray: [valley_gray; 2],
        })
        .collect();
    let result = summarize(profiles, 2).unwrap();
    assert_eq!(result.mean_ridge_width_px, 6.0);
    assert_eq!(result.mean_valley_width_px, 14.0);
    assert!((result.mean_ridge_gray - 6000.0 / 36.0).abs() < 1e-12);
    assert!((result.mean_valley_gray - 7600.0 / 56.0).abs() < 1e-12);
    // Different thickness groups remain in both the mean and the median.
    assert_eq!(result.ridge_width_px, 6.0);
    assert_eq!(result.profiles.len(), 2);
}

#[test]
fn budgets_are_validated_before_work_and_locations_cover_both_dimensions() {
    let input = stripes((160, 120), 20.0, 6.0, 0.0, 0.0, 0.0, false);
    let mut settings = config();
    settings.max_pixels = 10;
    assert!(matches!(
        estimate(&input, &settings),
        Err(FingerprintError::WorkLimit)
    ));
    settings = config();
    settings.bootstrap_grid_locations = 0;
    assert!(matches!(
        estimate(&input, &settings),
        Err(FingerprintError::InvalidConfig(_))
    ));
    settings = config();
    settings.bootstrap_grid_locations = 7;
    let result = estimate(&input, &settings).unwrap();
    assert!(result.examined_locations <= 7);
    assert!(result.profiles.iter().any(|p| p.ridges[1].start.y() < 60.0));
    assert!(result.profiles.iter().any(|p| p.ridges[1].start.y() > 60.0));
}

#[test]
fn bootstrap_fallback_finds_a_small_patch_between_grid_sites() {
    let (width, height) = (240, 160);
    let mut pixels = vec![230; width * height];
    for y in 15..65 {
        for x in 12_usize..88 {
            if [30_usize, 50, 70]
                .iter()
                .any(|&center| x.abs_diff(center) < 3)
            {
                pixels[y * width + x] = 20;
            }
        }
    }
    let capture = Capture::new(width as u32, height as u32, pixels, None, None).unwrap();
    let settings = ExtractionConfig {
        bootstrap_grid_locations: 1,
        ..config()
    };
    let result = estimate(&capture, &settings).unwrap();
    assert!(
        result.examined_locations > 1,
        "the one-site grid must miss this patch"
    );
    assert_eq!(result.profiles.len(), 1);
    assert!((result.ridge_period_px - 20.0).abs() < 0.5);
    assert!(
        result.profiles[0]
            .ridges
            .iter()
            .all(|ridge| ridge.start.y() < 65.0)
    );
}
