use super::*;

fn point(x: f64, y: f64) -> Point {
    Point::new(x, y).unwrap()
}

fn fixture(pixel: impl Fn(usize, usize) -> u8) -> (Capture, LocalPalette) {
    let pixels = (0..64 * 64).map(|i| pixel(i % 64, i / 64)).collect();
    let capture = Capture::new(64, 64, pixels, None, None).unwrap();
    let palette = LocalPalette {
        radius_px: 4,
        ridge_gray: vec![40.0; 64 * 64],
        valley_gray: vec![200.0; 64 * 64],
        usable: vec![true; 64 * 64],
    };
    (capture, palette)
}

fn ridge() -> (Capture, LocalPalette) {
    fixture(|_, y| (40 + 40 * y.abs_diff(32)).min(200) as u8)
}

fn arms() -> (Vec<Point>, Vec<Point>) {
    (
        vec![point(16.0, 32.0), point(32.0, 32.0), point(48.0, 32.0)],
        vec![point(16.0, 32.0), point(32.0, 33.0), point(48.0, 32.0)],
    )
}

#[test]
fn source_light_barrier_confirms_a_short_observed_end() {
    let (capture, palette) = fixture(|x, y| {
        if x <= 36 && (30..=34).contains(&y) {
            40
        } else {
            200
        }
    });
    let evidence = verify_spur(
        &capture,
        &palette,
        &[point(28.0, 32.0), point(32.0, 32.0)],
        8.0,
    )
    .unwrap();
    assert_eq!(evidence.decision, SourceDecision::Confirmed);
    assert_eq!(evidence.terminal_darkness, Some(1.0));
    assert!(evidence.light_barrier_distance_px.unwrap() >= 4.5);
    assert!(evidence.min_barrier_lightness.unwrap() > 0.5);
}

#[test]
fn cleanup_retains_its_first_source_confirmation() {
    let (capture, palette) = fixture(|x, y| {
        if (30..=34).contains(&y) {
            (40 + 32 * x.saturating_sub(32)).min(200) as u8
        } else {
            200
        }
    });
    let points = [point(28.0, 32.0), point(32.0, 32.0)];
    let cleanup = verify_spur(&capture, &palette, &points, 8.0).unwrap();
    assert_eq!(cleanup.decision, SourceDecision::Confirmed);
    // The unchanged 0.48 px source grid first crosses the midpoint at 2.88 px.
    assert!((cleanup.light_barrier_distance_px.unwrap() - 2.88).abs() < 1e-12);
    assert!((cleanup.min_barrier_lightness.unwrap() - 0.576).abs() < 1e-12);
}

#[test]
fn narrow_bright_spikes_cannot_confirm_a_cleanup_barrier() {
    let (capture, palette) = fixture(|x, y| {
        if (30..=34).contains(&y) && x != 36 {
            40
        } else {
            200
        }
    });
    let points = [point(28.0, 32.0), point(32.0, 32.0)];
    let evidence = verify_spur(&capture, &palette, &points, 8.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::PossibleContinuation);
    assert_eq!(evidence.min_barrier_lightness, None);
}

#[test]
fn dark_continuation_and_inadequate_lookahead_remain_unconfirmed() {
    for endpoint in [40, 64] {
        let (capture, palette) = fixture(|x, y| {
            if x <= endpoint && (30..=34).contains(&y) {
                40
            } else {
                200
            }
        });
        let evidence = verify_spur(
            &capture,
            &palette,
            &[point(28.0, 32.0), point(32.0, 32.0)],
            8.0,
        )
        .unwrap();
        assert_eq!(evidence.decision, SourceDecision::PossibleContinuation);
        assert_eq!(evidence.light_barrier_distance_px, None);
    }
}

#[test]
fn threshold_equal_light_and_flat_blur_do_not_confirm_a_barrier() {
    let (capture, palette) = fixture(|_, _| 120);
    let evidence = verify_spur(
        &capture,
        &palette,
        &[point(28.0, 32.0), point(32.0, 32.0)],
        8.0,
    )
    .unwrap();
    assert_eq!(evidence.decision, SourceDecision::PossibleContinuation);
    assert_eq!(evidence.terminal_darkness, Some(0.5));
}

#[test]
fn original_mask_and_palette_support_block_the_complete_footprint() {
    let (capture, mut palette) = fixture(|_, _| 200);
    let points = [point(28.2, 32.3), point(32.2, 32.3)];
    let index = 36 * 64 + 40;
    let mut mask = vec![true; 64 * 64];
    mask[index] = false;
    for hidden in [0, 255] {
        let mut pixels = capture.pixels().to_vec();
        pixels[index] = hidden;
        let masked = Capture::new(64, 64, pixels, Some(mask.clone()), None).unwrap();
        assert_eq!(
            verify_spur(&masked, &palette, &points, 8.0).unwrap(),
            SpurEvidence::missing_support()
        );
    }
    palette.usable[index] = false;
    assert_eq!(
        verify_spur(&capture, &palette, &points, 8.0).unwrap(),
        SpurEvidence::missing_support()
    );
    assert_eq!(
        verify_spur(
            &capture,
            &palette,
            &[point(58.0, 32.0), point(60.0, 32.0)],
            8.0
        )
        .unwrap(),
        SpurEvidence::missing_support()
    );
}

#[test]
fn single_supported_ridge_selects_source_center_not_shorter_id() {
    let (capture, palette) = ridge();
    let (center, bent) = arms();
    let evidence = verify_loop(&capture, &palette, &bent, &center, 8.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::Confirmed);
    assert_eq!(evidence.preferred_arm, Some(1));
    assert!(
        evidence.arm_center_distance_px.unwrap()[1] < evidence.arm_center_distance_px.unwrap()[0]
    );
    assert_eq!(evidence.longest_valley_length_px, Some(0.0));
    assert!(evidence.min_flank_lightness.unwrap() > 0.5);
}

#[test]
fn false_arm_on_a_bright_shoulder_is_not_a_separating_valley() {
    let (capture, palette) = ridge();
    let (center, _) = arms();
    let shoulder = [point(16.0, 32.0), point(30.0, 35.5), point(48.0, 32.0)];
    let evidence = verify_loop(&capture, &palette, &center, &shoulder, 8.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::Confirmed);
    assert_eq!(evidence.preferred_arm, Some(0));
    assert_eq!(evidence.max_valley_lightness, Some(0.0));
    assert_eq!(evidence.longest_valley_length_px, Some(0.0));
    assert!(evidence.mean_interior_darkness.unwrap() >= 0.5);
}

#[test]
fn observed_local_width_can_exceed_the_global_mean_half_width() {
    let (capture, palette) = fixture(|_, y| (40 + 16 * y.abs_diff(32)).min(200) as u8);
    let (first, second) = arms();
    let evidence = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::Confirmed);
    assert_eq!(evidence.preferred_arm, Some(0));
    // This image's light boundary is beyond R/2=4, but within one R.
    assert_eq!(capture.pixels()[28 * 64 + 32], 104);
    assert!(evidence.min_flank_lightness.unwrap() > 0.5);
}

#[test]
fn diverging_arms_use_common_midpoint_flow_and_keep_a_true_valley() {
    let first = [point(24.0, 32.0), point(25.0, 28.0), point(40.0, 32.0)];
    let second = [point(24.0, 32.0), point(25.0, 36.0), point(40.0, 32.0)];
    let (capture, palette) = fixture(|_, y| (40 + 16 * y.abs_diff(31)).min(200) as u8);
    let evidence = verify_loop(&capture, &palette, &first, &second, 12.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::Confirmed);
    assert_eq!(evidence.preferred_arm, Some(0));

    let (separate, palette) =
        fixture(|_, y| (40 + 80 * y.abs_diff(28).min(y.abs_diff(36))).min(200) as u8);
    let evidence = verify_loop(&separate, &palette, &first, &second, 12.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::SeparateRidges);
    assert!(evidence.longest_valley_length_px.unwrap() >= 1.0);
    assert_eq!(evidence.preferred_arm, None);
}

#[test]
fn unequal_raster_ports_do_not_turn_a_subpixel_hook_into_a_reversal() {
    let first = [
        point(24.0, 32.0),
        point(24.0, 31.0),
        point(29.0, 31.0),
        point(40.0, 40.0),
    ];
    let second = [point(24.0, 32.0), point(24.0, 33.0), point(40.0, 40.0)];
    let paths = (Path::new(&first).unwrap(), Path::new(&second).unwrap());
    let fraction = 0.5 / paths.0.length().max(paths.1.length());
    let midpoint = sampling::lerp(
        paths.0.at(fraction * paths.0.length()),
        paths.1.at(fraction * paths.1.length()),
        0.5,
    );
    assert!(midpoint.y() < first[0].y());
    assert!(sampling::distance(first[0], midpoint) < 0.5);
    for unit in [8.0, 12.0, 16.0] {
        let policy = SourcePolicy::for_unit(unit);
        assert_eq!(policy.sample_spacing_px, 0.5);
        assert!(common_normal(&paths.0, &paths.1, fraction, policy).is_some());
    }

    let (capture, palette) = fixture(|x, y| {
        let distance = (2.0 * y as f64 - x as f64 - 40.0).abs() / 5.0_f64.sqrt();
        (40.0 + 16.0 * distance).min(200.0).round() as u8
    });
    let evidence = verify_loop(&capture, &palette, &first, &second, 12.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::Confirmed);
    assert!(evidence.min_flank_lightness.unwrap() > 0.5);

    let (bright, palette) = fixture(|_, _| 200);
    let evidence = verify_loop(&bright, &palette, &first, &second, 12.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::ImageUnsupported);
    assert_eq!(evidence.preferred_arm, None);
}

#[test]
fn a_resolved_midpoint_reversal_remains_ambiguous() {
    let points = [
        point(20.0, 32.0),
        point(22.0, 32.0),
        point(21.0, 32.0),
        point(25.0, 32.0),
    ];
    let path = Path::new(&points).unwrap();
    let policy = SourcePolicy::for_unit(3.0);
    // Incoming displacement is 1.5 px and outgoing reversal is exactly the
    // existing 0.5 px resolution; the overall secant is nonzero.
    assert!(common_normal(&path, &path, 2.0 / path.length(), policy).is_none());

    let reverse: Vec<_> = points.iter().copied().rev().collect();
    let reversed = Path::new(&reverse).unwrap();
    assert!(common_normal(&reversed, &reversed, 1.0 - 2.0 / path.length(), policy).is_none());
}

#[test]
fn longer_observed_arm_wins_when_it_follows_the_source_center() {
    let (capture, palette) = ridge();
    let (_, second) = arms();
    let first = [
        point(16.0, 32.0),
        point(24.0, 32.0),
        point(24.25, 32.3),
        point(24.5, 31.7),
        point(24.75, 32.3),
        point(25.0, 31.7),
        point(25.25, 32.3),
        point(25.5, 32.0),
        point(48.0, 32.0),
    ];
    assert!(Path::new(&first).unwrap().length() > Path::new(&second).unwrap().length());
    let evidence = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::Confirmed);
    assert_eq!(evidence.preferred_arm, Some(0));
}

#[test]
fn local_gray_affine_change_preserves_source_classification() {
    let (capture, palette) = ridge();
    let (changed, mut changed_palette) = fixture(|_, y| (60 + 20 * y.abs_diff(32)).min(140) as u8);
    changed_palette.ridge_gray.fill(60.0);
    changed_palette.valley_gray.fill(140.0);
    let (first, second) = arms();
    let original = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
    let changed = verify_loop(&changed, &changed_palette, &first, &second, 8.0).unwrap();
    assert_eq!(original.decision, changed.decision);
    assert_eq!(original.preferred_arm, changed.preferred_arm);
    assert!(
        (original.mean_interior_darkness.unwrap() - changed.mean_interior_darkness.unwrap()).abs()
            < 1e-12
    );
}

#[test]
fn two_dark_source_masses_with_internal_light_are_separate_ridges() {
    let (capture, palette) = fixture(|_, y| {
        let distance = y.abs_diff(30).min(y.abs_diff(34));
        (40 + distance * 120).min(200) as u8
    });
    let first = [
        point(16.0, 32.0),
        point(24.0, 30.0),
        point(40.0, 30.0),
        point(48.0, 32.0),
    ];
    let second = [
        point(16.0, 32.0),
        point(24.0, 34.0),
        point(40.0, 34.0),
        point(48.0, 32.0),
    ];
    let evidence = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::SeparateRidges);
    assert!(evidence.max_valley_lightness.unwrap() > 0.5);
    assert!(evidence.longest_valley_length_px.unwrap() >= 1.0);
    assert_eq!(evidence.preferred_arm, None);
}

#[test]
fn a_short_light_speck_does_not_preserve_a_duplicate_ridge_but_a_valley_does() {
    let first = [
        point(16.0, 32.0),
        point(24.0, 31.0),
        point(40.0, 31.0),
        point(48.0, 32.0),
    ];
    let second = [
        point(16.0, 32.0),
        point(24.0, 34.0),
        point(40.0, 34.0),
        point(48.0, 32.0),
    ];
    for (bright_x, expected) in [
        (31..=32, SourceDecision::Confirmed),
        (24..=40, SourceDecision::SeparateRidges),
    ] {
        let (capture, palette) = fixture(|x, y| {
            if bright_x.contains(&x) && y == 32 {
                200
            } else {
                (40 + 30 * y.abs_diff(32)).min(200) as u8
            }
        });
        let evidence = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
        assert_eq!(evidence.decision, expected, "{evidence:?}");
        assert!(evidence.max_valley_lightness.unwrap() > 0.5);
        if expected == SourceDecision::Confirmed {
            assert!(evidence.longest_valley_length_px.unwrap() >= 1.0);
            assert!(evidence.longest_valley_length_px.unwrap() < 8.0 / 3.0);
            assert_eq!(evidence.preferred_arm, Some(0));
        } else {
            assert!(evidence.longest_valley_length_px.unwrap() >= 8.0 / 3.0);
            assert_eq!(evidence.preferred_arm, None);
        }
    }
}

#[test]
fn symmetric_observed_arms_are_not_selected_by_roundoff() {
    let (capture, palette) = ridge();
    let first = [point(16.0, 32.0), point(32.0, 31.0), point(48.0, 32.0)];
    let second = [point(16.0, 32.0), point(32.0, 33.0), point(48.0, 32.0)];
    let evidence = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::AmbiguousGeometry);
    assert_eq!(evidence.preferred_arm, None);
}

#[test]
fn flat_blur_is_unsupported_but_a_bright_false_arm_is_not_a_valley() {
    let (first, second) = arms();
    for value in [120, 200] {
        let (capture, palette) = fixture(|_, _| value);
        let evidence = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
        assert_eq!(evidence.decision, SourceDecision::ImageUnsupported);
        assert_eq!(evidence.preferred_arm, None);
    }
    let (capture, palette) = fixture(|x, y| {
        if (25..=35).contains(&x) && y == 32 {
            200
        } else {
            (40 + 40 * y.abs_diff(32)).min(200) as u8
        }
    });
    let evidence = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
    // The bright band lies on the first arm, not between two dark arms.
    assert_eq!(evidence.decision, SourceDecision::Confirmed);
    assert_eq!(evidence.preferred_arm, Some(1));
    assert_eq!(evidence.longest_valley_length_px, Some(0.0));
}

#[test]
fn loop_missing_source_and_outside_flank_remain_unknown() {
    let (capture, mut palette) = ridge();
    let (first, second) = arms();
    palette.usable[29 * 64 + 27] = false;
    assert_eq!(
        verify_loop(&capture, &palette, &first, &second, 8.0).unwrap(),
        LoopEvidence::missing_support()
    );
    let (capture, palette) = ridge();
    let first = [point(1.0, 32.0), point(20.0, 32.0)];
    let second = [point(1.0, 32.0), point(20.0, 33.0)];
    assert_eq!(
        verify_loop(&capture, &palette, &first, &second, 8.0).unwrap(),
        LoopEvidence::missing_support()
    );
}

#[test]
fn loop_width_is_strict_and_raw_corner_extrema_are_not_skipped() {
    let (capture, palette) = ridge();
    let first = [point(20.0, 32.0), point(44.0, 32.0)];
    let policy = SourcePolicy::for_unit(10.0);
    assert_eq!(policy.max_ridge_width_px, 12.0);
    for apex in [44.0_f64.next_down(), 44.0, 44.0_f64.next_up()] {
        let second = [point(20.0, 32.0), point(32.0, apex), point(44.0, 32.0)];
        let evidence = verify_loop(&capture, &palette, &first, &second, 10.0).unwrap();
        if apex < 44.0 {
            assert!(evidence.max_width_px.unwrap() < policy.max_ridge_width_px);
            assert_ne!(evidence.decision, SourceDecision::AmbiguousGeometry);
        } else {
            assert!(evidence.max_width_px.unwrap() >= policy.max_ridge_width_px);
            assert_eq!(evidence.decision, SourceDecision::AmbiguousGeometry);
        }
        assert_eq!(evidence.preferred_arm, None);
    }

    let second = [point(20.0, 32.0), point(32.123, 45.0), point(44.0, 32.0)];
    let evidence = verify_loop(&capture, &palette, &first, &second, 10.0).unwrap();
    assert_eq!(evidence.decision, SourceDecision::AmbiguousGeometry);
    assert!(evidence.max_width_px.unwrap() >= policy.max_ridge_width_px);
    assert_eq!(evidence.preferred_arm, None);
}

#[test]
fn reversal_and_extra_collinear_vertices_preserve_source_preference() {
    let (capture, palette) = ridge();
    let (mut first, mut second) = arms();
    let original = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
    first.reverse();
    second.reverse();
    let reversed = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
    assert_eq!(original.decision, reversed.decision);
    assert_eq!(original.preferred_arm, reversed.preferred_arm);
    assert!(
        (original.mean_interior_darkness.unwrap() - reversed.mean_interior_darkness.unwrap()).abs()
            < 1e-12
    );
    first.insert(1, point(40.123, 32.0));
    let subdivided = verify_loop(&capture, &palette, &first, &second, 8.0).unwrap();
    assert_eq!(subdivided.preferred_arm, original.preferred_arm);
    assert!(
        (subdivided.mean_interior_darkness.unwrap() - original.mean_interior_darkness.unwrap())
            .abs()
            < 1e-10
    );
}

#[test]
fn valley_extent_uses_actual_longitudinal_distances_and_strict_threshold() {
    let section = |lightness| Section {
        interior_darkness: 0.9,
        valley_lightness: lightness,
        flank_lightness: 0.9,
        arm_darkness: [0.9, 0.8],
        center_distance: [0.1, 0.2],
    };
    for unit in [0.75, 3.0, 8.0, 16.820792511697714, 48.0] {
        let policy = SourcePolicy::for_unit(unit);
        for (middle, expected) in [
            (unit / 3.0 * 0.999, SourceDecision::Confirmed),
            (unit / 3.0, SourceDecision::SeparateRidges),
            (unit / 3.0 * 1.001, SourceDecision::SeparateRidges),
        ] {
            let sections = [section(0.5), section(1.0), section(0.5)];
            let positions = [0.0, middle / 2.0, middle];
            let evidence = summarize_loop(&sections, &positions, unit / 4.0, policy);
            assert_eq!(evidence.decision, expected, "unit={unit}, extent={middle}");
            assert!((evidence.longest_valley_length_px.unwrap() - middle).abs() < 1e-12);
        }
    }
    let policy = SourcePolicy::for_unit(8.0);
    let a = summarize_loop(
        &[section(0.0), section(1.0), section(0.0)],
        &[0.0, 0.8, 2.0],
        2.0,
        policy,
    );
    let b = summarize_loop(
        &[section(0.0), section(1.0), section(0.0)],
        &[0.0, 1.2, 2.0],
        2.0,
        policy,
    );
    assert_eq!(a.longest_valley_length_px, b.longest_valley_length_px);
    assert_eq!(a.decision, b.decision);
}

#[test]
fn invalid_scale_and_huge_work_fail_without_truncation_or_panics() {
    let (capture, palette) = ridge();
    let points = [point(20.0, 32.0), point(24.0, 32.0)];
    assert!(matches!(
        verify_spur(&capture, &palette, &points, 0.0),
        Err(FingerprintError::InvalidConfig(_))
    ));
    assert!(matches!(
        verify_spur(&capture, &palette, &points, f64::MAX),
        Err(FingerprintError::WorkLimit)
    ));
    assert_eq!(
        verify_loop(&capture, &palette, &[], &points, 8.0)
            .unwrap()
            .decision,
        SourceDecision::AmbiguousGeometry
    );
}
