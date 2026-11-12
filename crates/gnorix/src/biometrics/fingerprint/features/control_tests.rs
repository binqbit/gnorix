//! Independent image/graph controls for diagnostic evidence, not recognition rates.

use super::super::{LocalPalette, RidgeCycle, RidgeEdge, RidgeNode};
use super::*;

const SIDE: usize = 128;
const UNIT: f64 = 8.0;

fn point(x: f64, y: f64) -> Point {
    Point::new(x, y).unwrap()
}

fn policy() -> FeaturePolicy {
    FeaturePolicy {
        reference_width_px: UNIT,
        branch_horizon_px: 2.0 * UNIT,
        tangent_support_px: UNIT / 2.0,
        sample_spacing_px: 0.5,
        source_search_px: 1.2 * UNIT,
        continuation_search_px: 3.0 * UNIT,
        terminal_barrier_width_px: UNIT,
        terminal_barrier_depth_px: UNIT / 4.0,
        context_horizon_px: 4.0 * UNIT,
        neighborhood_radius_px: 3.0 * UNIT,
        contrast_reference: ExtractionConfig::default().min_contrast,
        stability: StabilityPolicy {
            max_position_shift_px: UNIT,
            arm_radius_px: UNIT,
            max_arm_angle_radians: std::f64::consts::PI / 6.0,
            max_arm_shape_error_px: UNIT / 4.0,
            max_isolated_shift_px: UNIT / 4.0,
            cycle_relative_tolerance: 0.2,
        },
        stress_cases: [StressCase {
            directional_margin: 0.0,
            min_coherence: 0.2,
            node_count: 0,
            cycle_count: 0,
        }; 8],
    }
}

fn capture(
    width: usize,
    dark: u8,
    light: u8,
    is_dark: impl Fn(f64, f64) -> bool,
    mask: Option<Vec<bool>>,
) -> (Capture, LocalPalette) {
    let pixels = (0..width * SIDE)
        .map(|index| {
            if is_dark((index % width) as f64, (index / width) as f64) {
                dark
            } else {
                light
            }
        })
        .collect();
    (
        Capture::new(width as u32, SIDE as u32, pixels, mask, None).unwrap(),
        LocalPalette {
            radius_px: UNIT as usize,
            ridge_gray: vec![f32::from(dark); width * SIDE],
            valley_gray: vec![f32::from(light); width * SIDE],
            usable: vec![true; width * SIDE],
        },
    )
}

fn node(position: Point, kind: NodeKind) -> RidgeNode {
    RidgeNode {
        position,
        kind,
        touches_unobserved: false,
    }
}

fn fork(short_tail: bool) -> RidgeGraph {
    let center = point(64.0, 64.0);
    let ends = [
        point(64.0, if short_tail { 68.0 } else { 88.0 }),
        point(48.0, 40.0),
        point(80.0, 40.0),
    ];
    RidgeGraph {
        nodes: std::iter::once(node(center, NodeKind::Junction))
            .chain(ends.map(|end| node(end, NodeKind::Ending)))
            .collect(),
        edges: ends
            .into_iter()
            .enumerate()
            .map(|(id, end)| RidgeEdge {
                start: 0,
                end: id + 1,
                points: vec![center, end],
            })
            .collect(),
        cycles: vec![],
    }
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x() - a.x();
    let dy = b.y() - a.y();
    let fraction =
        (((p.x() - a.x()) * dx + (p.y() - a.y()) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (p.x() - a.x() - fraction * dx).hypot(p.y() - a.y() - fraction * dy)
}

fn on_graph(graph: &RidgeGraph, x: f64, y: f64) -> bool {
    graph.edges.iter().any(|edge| {
        edge.points
            .windows(2)
            .any(|pair| segment_distance(point(x, y), pair[0], pair[1]) <= UNIT / 2.0)
    })
}

fn assess(capture: &Capture, palette: &LocalPalette, graph: &RidgeGraph) -> Vec<FeatureStrength> {
    let original = graph.clone();
    let features = evidence::assess(capture, palette, graph, &policy()).unwrap();
    assert_eq!(
        *graph, original,
        "evidence must not alter observed geometry"
    );
    assert_eq!(features.len(), graph.nodes.len() + graph.cycles.len());
    for (id, feature) in features.iter().enumerate() {
        assert_eq!(feature.id, id, "weak candidates retain original IDs");
    }
    features
}

fn score(feature: &FeatureStrength) -> Option<f64> {
    combine(
        feature.source_support,
        feature.geometry_support,
        feature.context.support,
        1.0,
    )
}

fn scaled_policy(unit: f64) -> FeaturePolicy {
    let mut result = policy();
    result.reference_width_px = unit;
    result.branch_horizon_px = 2.0 * unit;
    result.tangent_support_px = unit / 2.0;
    result.source_search_px = 1.2 * unit;
    result.continuation_search_px = 3.0 * unit;
    result.terminal_barrier_width_px = unit;
    result.terminal_barrier_depth_px = unit / 4.0;
    result.context_horizon_px = 4.0 * unit;
    result.neighborhood_radius_px = 3.0 * unit;
    result
}

fn scaled_fork(unit: f64, directions: [[f64; 2]; 3]) -> RidgeGraph {
    let mut graph = fork(false);
    for (index, [x, y]) in directions.into_iter().enumerate() {
        let end = point(64.0 + unit * x, 64.0 + unit * y);
        graph.nodes[index + 1].position = end;
        graph.edges[index].points[1] = end;
    }
    graph
}

fn on_scaled_graph(graph: &RidgeGraph, unit: f64, x: f64, y: f64) -> bool {
    graph.edges.iter().any(|edge| {
        edge.points
            .windows(2)
            .any(|pair| segment_distance(point(x, y), pair[0], pair[1]) <= unit / 2.0)
    })
}

#[test]
fn a_faint_separated_fork_outscores_a_bright_fork_with_one_merged_valley_at_each_scale() {
    // R and geometry scale together. Binary raster widths differ by a pixel,
    // so assert the physical ordering at each scale, not identical scores.
    for unit in [4.0, 8.0, 16.0] {
        let graph = scaled_fork(unit, [[-3.0, 0.0], [3.0, 0.0], [0.0, -3.0]]);
        let active = scaled_policy(unit);
        let (faint_image, faint_palette) = capture(
            SIDE,
            124,
            132,
            |x, y| on_scaled_graph(&graph, unit, x, y),
            None,
        );
        let faint = evidence::assess(&faint_image, &faint_palette, &graph, &active).unwrap();

        for mirror in [-1.0, 1.0] {
            let (merged_image, merged_palette) = capture(
                SIDE,
                20,
                220,
                |x, y| {
                    on_scaled_graph(&graph, unit, x, y)
                        || ((0.0..=3.0 * unit).contains(&(mirror * (x - 64.0)))
                            && (64.0 - 3.0 * unit..=64.0).contains(&y))
                },
                None,
            );
            let merged = evidence::assess(&merged_image, &merged_palette, &graph, &active).unwrap();
            assert!(
                score(&faint[0]).unwrap() > score(&merged[0]).unwrap(),
                "R={unit}, mirror={mirror}: faint={:?}, merged={:?}",
                score(&faint[0]),
                score(&merged[0]),
            );

            let mut reordered = graph.clone();
            reordered.edges.rotate_left(1);
            let reordered =
                evidence::assess(&merged_image, &merged_palette, &reordered, &active).unwrap();
            assert!(
                (score(&reordered[0]).unwrap() - score(&merged[0]).unwrap()).abs() < 1e-12,
                "equal-angle branch pairs must not depend on incident-edge order",
            );
        }
    }
}

#[test]
fn filling_the_daughter_fan_cannot_borrow_separation_from_the_trunk() {
    let graph = fork(false);
    let (clear_image, clear_palette) = capture(SIDE, 124, 132, |x, y| on_graph(&graph, x, y), None);
    let clear = assess(&clear_image, &clear_palette, &graph);
    let (merged_image, merged_palette) = capture(
        SIDE,
        20,
        220,
        |x, y| {
            on_graph(&graph, x, y)
                || ((40.0..=64.0).contains(&y) && (x - 64.0).abs() <= (64.0 - y) * 2.0 / 3.0)
        },
        None,
    );
    let merged = assess(&merged_image, &merged_palette, &graph);
    assert!(score(&clear[0]).unwrap() > score(&merged[0]).unwrap());
}

#[test]
fn additive_gray_offset_preserves_feature_evidence() {
    let graph = fork(false);
    let (image, palette) = capture(SIDE, 20, 220, |x, y| on_graph(&graph, x, y), None);
    let original = assess(&image, &palette, &graph);
    let (image, palette) = capture(SIDE, 40, 240, |x, y| on_graph(&graph, x, y), None);
    let shifted = assess(&image, &palette, &graph);
    assert_eq!(shifted, original);
}

#[test]
fn faint_consistent_ridges_outscore_bright_overlapping_noisy_pixel_classes() {
    let graph = fork(false);
    let (image, palette) = capture(SIDE, 124, 132, |x, y| on_graph(&graph, x, y), None);
    let faint = assess(&image, &palette, &graph);

    let (_, palette) = capture(SIDE, 20, 220, |x, y| on_graph(&graph, x, y), None);
    let pixels = (0..SIDE * SIDE)
        .map(|index| {
            let x = (index % SIDE) as f64;
            let y = (index / SIDE) as f64;
            let mean = if on_graph(&graph, x, y) { 100_i16 } else { 150 };
            let bits = (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let noise = ((bits >> 32) % 161) as i16 - 80;
            (mean + noise).clamp(0, 255) as u8
        })
        .collect();
    let noisy = Capture::new(SIDE as u32, SIDE as u32, pixels, None, None).unwrap();
    let noisy = assess(&noisy, &palette, &graph);

    assert!(faint[0].context.support.unwrap() > noisy[0].context.support.unwrap());
    assert!(
        score(&faint[0]).unwrap() > score(&noisy[0]).unwrap(),
        "at T=1, consistent faint structure must beat overlapping noisy classes: faint={:?}, noisy={:?}",
        score(&faint[0]),
        score(&noisy[0]),
    );
}

#[test]
fn perfect_stability_cannot_rescue_a_candidate_without_source_structure() {
    let graph = fork(false);
    let (image, palette) = capture(SIDE, 20, 220, |_, _| false, None);
    let unsupported = assess(&image, &palette, &graph);
    assert_eq!(unsupported[0].context.support, Some(0.0));
    assert_eq!(score(&unsupported[0]), Some(0.0));
    assert_eq!(unsupported[0].source_node, Some(0));
}

#[test]
fn incidental_short_raw_branch_does_not_truncate_supported_physical_continuation() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let original = assess(&image, &palette, &graph);

    let mut split = graph.clone();
    split
        .nodes
        .push(node(point(56.0, 64.0), NodeKind::Junction));
    split.nodes.push(node(point(56.0, 62.0), NodeKind::Ending));
    split.edges = vec![
        RidgeEdge {
            start: 0,
            end: 2,
            points: vec![point(32.0, 64.0), point(56.0, 64.0)],
        },
        RidgeEdge {
            start: 2,
            end: 1,
            points: vec![point(56.0, 64.0), point(64.0, 64.0)],
        },
        RidgeEdge {
            start: 2,
            end: 3,
            points: vec![point(56.0, 64.0), point(56.0, 62.0)],
        },
    ];
    let split = assess(&image, &palette, &split);
    assert_eq!(split[1].context.support, original[1].context.support);
    assert_eq!(
        split[1].context.arms[0].supported_length_px,
        original[1].context.arms[0].supported_length_px,
    );
    assert!(
        score(&split[1]).unwrap() > 0.75 * score(&original[1]).unwrap(),
        "an incidental node may affect local evidence but must not erase long supported geometry",
    );
    assert!(score(&split[3]).unwrap() < score(&split[1]).unwrap());
}

#[test]
fn missing_required_flank_is_unknown_even_when_the_centerline_is_visible() {
    let graph = fork(false);
    let mut mask = vec![true; SIDE * SIDE];
    for y in 74..80 {
        for x in 70..73 {
            mask[y * SIDE + x] = false;
        }
    }
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| on_graph(&graph, x, y),
        Some(mask.clone()),
    );
    let original = assess(&image, &palette, &graph);
    assert!(original[0].source_support.is_some());
    assert_eq!(original[0].context.support, None);
    assert_eq!(score(&original[0]), None);

    let pixels = image
        .pixels()
        .iter()
        .zip(&mask)
        .map(|(&pixel, &observed)| if observed { pixel } else { 255 - pixel })
        .collect();
    let changed = Capture::new(SIDE as u32, SIDE as u32, pixels, Some(mask), None).unwrap();
    assert_eq!(assess(&changed, &palette, &graph), original);
}

#[test]
fn separated_fork_outscores_short_tail_filled_fork_and_same_normalized_low_contrast() {
    let graph = fork(false);
    let (image, palette) = capture(SIDE, 20, 220, |x, y| on_graph(&graph, x, y), None);
    let strong = assess(&image, &palette, &graph);
    assert_eq!(strong[0].kind, FeatureKind::Bifurcation);
    assert!(score(&strong[0]).unwrap() > 0.0);
    assert_eq!(strong[0].geometry_coverage, 1.0);

    let short = fork(true);
    let (image, palette) = capture(SIDE, 20, 220, |x, y| on_graph(&short, x, y), None);
    let short = assess(&image, &palette, &short);
    assert!(score(&short[0]).unwrap() < score(&strong[0]).unwrap());
    assert!(
        short[0]
            .reasons
            .contains(&FeatureReason::InsufficientArmExtent)
    );

    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| (x - 64.0).hypot(y - 64.0) <= 32.0 || on_graph(&graph, x, y),
        None,
    );
    let filled = assess(&image, &palette, &graph);
    assert_eq!(filled[0].separation, Some(0.0));
    assert_eq!(score(&filled[0]), Some(0.0));

    let (image, palette) = capture(SIDE, 124, 132, |x, y| on_graph(&graph, x, y), None);
    let faint = assess(&image, &palette, &graph);
    assert_eq!(faint[0].geometry_support, strong[0].geometry_support);
    assert!(faint[0].source_contrast.unwrap() < strong[0].source_contrast.unwrap());
    assert!(
        score(&faint[0]).unwrap() > score(&short[0]).unwrap(),
        "a faint supported split must outrank a bright short tail",
    );
    assert!(
        score(&faint[0]).unwrap() < score(&strong[0]).unwrap(),
        "faint P={:?}, G={:?}, separation={:?}; strong P={:?}, G={:?}, separation={:?}",
        faint[0].source_support,
        faint[0].geometry_support,
        faint[0].separation,
        strong[0].source_support,
        strong[0].geometry_support,
        strong[0].separation,
    );
}

fn ending() -> RidgeGraph {
    RidgeGraph {
        nodes: vec![
            node(point(32.0, 64.0), NodeKind::Ending),
            node(point(64.0, 64.0), NodeKind::Ending),
        ],
        edges: vec![RidgeEdge {
            start: 0,
            end: 1,
            points: vec![point(32.0, 64.0), point(64.0, 64.0)],
        }],
        cycles: vec![],
    }
}

fn ending_with_aligned_continuation() -> RidgeGraph {
    let mut graph = ending();
    graph.nodes.extend([
        node(point(76.0, 64.0), NodeKind::Ending),
        node(point(108.0, 64.0), NodeKind::Ending),
    ]);
    graph.edges.push(RidgeEdge {
        start: 2,
        end: 3,
        points: vec![point(76.0, 64.0), point(108.0, 64.0)],
    });
    graph
}

fn horizontal_ridge(x: f64, y: f64, end: f64) -> bool {
    (24.0..=end).contains(&x) && (y - 64.0).abs() < UNIT / 2.0
}

fn quarter_turn(point: Point, turns: usize) -> Point {
    let (mut x, mut y) = (point.x() - 64.0, point.y() - 64.0);
    for _ in 0..turns {
        (x, y) = (-y, x);
    }
    Point::new(64.0 + x, 64.0 + y).unwrap()
}

fn curved_ending_graph(unit: f64, mirror: f64, turns: usize, raster: bool) -> RidgeGraph {
    let radius = 3.0 * unit;
    let mut points: Vec<_> = (0..=128)
        .map(|index| {
            let angle = std::f64::consts::PI * (1.0 + 0.5 * index as f64 / 128.0);
            let point = quarter_turn(
                point(
                    64.0 + mirror * radius * angle.cos(),
                    64.0 + radius + radius * angle.sin(),
                ),
                turns,
            );
            if raster {
                Point::new(point.x().round(), point.y().round()).unwrap()
            } else {
                point
            }
        })
        .collect();
    points.dedup();
    RidgeGraph {
        nodes: vec![
            node(points[0], NodeKind::Ending),
            node(*points.last().unwrap(), NodeKind::Ending),
        ],
        edges: vec![RidgeEdge {
            start: 0,
            end: 1,
            points,
        }],
        cycles: Vec::new(),
    }
}

#[test]
fn a_supported_curved_ending_survives_raster_turns_rotations_and_scale() {
    for unit in [4.0, 8.0, 12.0] {
        for mirror in [-1.0, 1.0] {
            for turns in 0..4 {
                let radius = 3.0 * unit;
                let (image, palette) = capture(
                    SIDE,
                    20,
                    220,
                    |x, y| {
                        let local = quarter_turn(point(x, y), 4 - turns);
                        let (x, y) = (mirror * (local.x() - 64.0), local.y() - 64.0);
                        ((x.hypot(y - radius) - radius).abs() <= unit / 2.0
                            && x <= 0.0
                            && y <= radius)
                            || (x + radius).hypot(y - radius) <= unit / 2.0
                            || x.hypot(y) <= unit / 2.0
                    },
                    None,
                );
                let active = scaled_policy(unit);
                let smooth = curved_ending_graph(unit, mirror, turns, false);
                let raster = curved_ending_graph(unit, mirror, turns, true);
                let smooth = evidence::assess(&image, &palette, &smooth, &active).unwrap();
                let raster = evidence::assess(&image, &palette, &raster, &active).unwrap();

                assert!(smooth[1].context.termination.unwrap() > 0.0);
                assert!(
                    raster[1].context.termination.unwrap() > 0.0,
                    "the same source cap must remain confirmed through raster turns: R={unit}, mirror={mirror}, turns={turns}, context={:?}",
                    raster[1].context,
                );
                assert!(
                    score(&raster[1]).unwrap() > 0.75 * score(&smooth[1]).unwrap(),
                    "raster ownership gaps cannot erase source-supported geometry",
                );
            }
        }
    }
}

#[test]
fn source_continuation_rejects_an_apparent_ending_without_removing_it() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let clear = assess(&image, &palette, &graph);
    assert!(clear[1].termination.unwrap() > 0.0);
    assert!(
        score(&clear[1]).unwrap() > 0.0,
        "clear ending context: {:?}",
        clear[1].context,
    );

    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 100.0), None);
    let continued = assess(&image, &palette, &graph);
    assert_eq!(continued[1].termination, Some(0.0));
    assert_eq!(score(&continued[1]), Some(0.0));
    assert!(
        continued[1]
            .reasons
            .contains(&FeatureReason::PossibleContinuation)
    );
    assert_eq!(continued[1].source_node, Some(1));
}

#[test]
fn broad_dark_region_with_a_front_cap_is_not_a_confirmed_ridge_ending() {
    let graph = ending();
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| (24.0..=64.0).contains(&x) && (y - 64.0).abs() < 4.0 * UNIT,
        None,
    );
    let broad = assess(&image, &palette, &graph);
    assert_eq!(broad[1].termination, Some(0.0));
    assert_eq!(score(&broad[1]), Some(0.0));
    assert!(
        broad[1]
            .reasons
            .contains(&FeatureReason::PossibleContinuation),
    );
}

#[test]
fn gradual_ending_scores_its_sustained_light_cap_not_the_first_threshold_crossing() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let clear = assess(&image, &palette, &graph);
    let mut pixels = image.pixels().to_vec();
    for y in 61..68 {
        for x in 65..70 {
            pixels[y * SIDE + x] = 20 + ((x - 64) * 200 / 6) as u8;
        }
    }
    let gradual = Capture::new(SIDE as u32, SIDE as u32, pixels, None, None).unwrap();
    let gradual = assess(&gradual, &palette, &graph);
    assert_eq!(gradual[1].arms, clear[1].arms);
    assert_eq!(gradual[1].geometry_support, clear[1].geometry_support);
    assert_eq!(gradual[1].termination, clear[1].termination);
    assert_eq!(score(&gradual[1]), score(&clear[1]));
}

#[test]
fn lateral_cap_contamination_weakens_evidence_without_a_midpoint_cliff() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let clear = assess(&image, &palette, &graph);
    for side in [60, 68] {
        let mut previous = clear[1].context.termination.unwrap();
        for gray in [160_u8, 130, 122, 118, 114, 100] {
            let mut pixels = image.pixels().to_vec();
            for x in 66..75 {
                pixels[side * SIDE + x] = gray;
            }
            let contaminated = Capture::new(SIDE as u32, SIDE as u32, pixels, None, None).unwrap();
            let observed = assess(&contaminated, &palette, &graph);
            let support = observed[1].context.termination.unwrap();
            assert!(
                support > 0.0 && support <= previous + 1e-12,
                "a lateral cap pixel should weaken support smoothly: side={side}, gray={gray}, previous={previous}, measured={support}",
            );
            assert!(score(&observed[1]).unwrap() > 0.0);
            if gray == 118 {
                assert!(
                    support > 0.75 * previous,
                    "four gray levels across the old midpoint cannot erase most cap evidence",
                );
            }
            previous = support;
        }
        assert!(previous < clear[1].context.termination.unwrap());
    }
}

#[test]
fn a_single_transverse_dip_in_the_incoming_ridge_does_not_erase_its_ending() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let clear = assess(&image, &palette, &graph);
    for x in [47, 48, 49] {
        for y in [62, 66] {
            for gray in [118_u8, 122] {
                let mut pixels = image.pixels().to_vec();
                pixels[y * SIDE + x] = gray;
                let softened = Capture::new(SIDE as u32, SIDE as u32, pixels, None, None).unwrap();
                let observed = assess(&softened, &palette, &graph);
                assert!(
                    observed[1].context.support.unwrap() > 0.75 * clear[1].context.support.unwrap(),
                    "an isolated source dip cannot become a hard rejection when it crosses the inspection boundary: ({x},{y}), gray={gray}, context={:?}",
                    observed[1].context,
                );
                assert!(score(&observed[1]).unwrap() > 0.5 * score(&clear[1]).unwrap());
            }
        }
    }
}

#[test]
fn a_short_bright_stripe_cannot_match_a_sustained_light_cap() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let clear = assess(&image, &palette, &graph);
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| horizontal_ridge(x, y, 100.0) && x != 68.0,
        None,
    );
    let stripe = assess(&image, &palette, &graph);
    assert!(stripe[1].context.termination.unwrap() < clear[1].context.termination.unwrap());
    assert!(score(&stripe[1]).unwrap() < score(&clear[1]).unwrap());
}

#[test]
fn a_narrow_but_sustained_source_bend_keeps_an_apparent_ending_weak() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let clear = assess(&image, &palette, &graph);
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| {
            horizontal_ridge(x, y, 64.0)
                || ((x - 64.0).abs() < UNIT / 4.0 && (64.0..=100.0).contains(&y))
        },
        None,
    );
    let bent = assess(&image, &palette, &graph);
    assert!(score(&bent[1]).unwrap() < 0.5 * score(&clear[1]).unwrap());
}

#[test]
fn a_faint_source_bend_is_a_contradiction_relative_to_its_own_ridge() {
    let graph = ending();
    let (image, mut palette) = capture(SIDE, 110, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    palette.ridge_gray.fill(20.0);
    let clear = assess(&image, &palette, &graph);
    let (image, mut palette) = capture(
        SIDE,
        110,
        220,
        |x, y| {
            horizontal_ridge(x, y, 64.0)
                || ((x - 64.0).abs() < UNIT / 2.0 && (64.0..=100.0).contains(&y))
        },
        None,
    );
    palette.ridge_gray.fill(20.0);
    let bent = assess(&image, &palette, &graph);
    assert!(
        score(&bent[1]).unwrap() < 0.5 * score(&clear[1]).unwrap(),
        "the continuing ridge has the same source strength as the incoming ridge: clear={:?}, bent={:?}, context={:?}",
        score(&clear[1]),
        score(&bent[1]),
        bent[1].context,
    );
}

#[test]
fn a_weak_tip_pixel_does_not_hide_a_sustained_source_bend() {
    let graph = ending();
    let observed = |curved, gray| {
        let (image, palette) = capture(
            SIDE,
            20,
            220,
            |x, y| {
                horizontal_ridge(x, y, 64.0)
                    || (curved && (x - 64.0).abs() < UNIT / 2.0 && (64.0..=100.0).contains(&y))
            },
            None,
        );
        let mut pixels = image.pixels().to_vec();
        pixels[64 * SIDE + 64] = gray;
        let image = Capture::new(SIDE as u32, SIDE as u32, pixels, None, None).unwrap();
        assess(&image, &palette, &graph)
    };
    for gray in [118, 120] {
        let clear = observed(false, gray);
        let curved = observed(true, gray);
        assert!(
            score(&curved[1]).unwrap() < 0.5 * score(&clear[1]).unwrap(),
            "one weak tip pixel cannot disconnect a strong incoming core from its visible bend: gray={gray}, clear={:?}, curved={:?}, context={:?}",
            score(&clear[1]),
            score(&curved[1]),
            curved[1].context,
        );
    }
}

#[test]
fn incoming_core_seeds_do_not_jump_a_light_gap_to_a_separate_ridge() {
    let graph = ending();
    for gray in [118, 120] {
        let observed = |separate| {
            let (image, palette) = capture(
                SIDE,
                20,
                220,
                |x, y| {
                    horizontal_ridge(x, y, 64.0)
                        || (separate
                            && (x - 64.0).abs() < UNIT / 2.0
                            && (69.0..=100.0).contains(&y))
                },
                None,
            );
            let mut pixels = image.pixels().to_vec();
            pixels[64 * SIDE + 64] = gray;
            let image = Capture::new(SIDE as u32, SIDE as u32, pixels, None, None).unwrap();
            assess(&image, &palette, &graph)
        };
        let clear = observed(false);
        let separate = observed(true);
        assert_eq!(separate[1].context.curved_continuation, Some(0.0));
        assert_eq!(separate[1].context.support, clear[1].context.support);
        assert_eq!(score(&separate[1]), score(&clear[1]));
    }
}

#[test]
fn an_unresolved_incoming_reference_is_not_missing_source_pixels() {
    let graph = ending();
    for gray in [120, 130] {
        let (image, mut palette) =
            capture(SIDE, gray, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
        palette.ridge_gray.fill(20.0);
        let observed = assess(&image, &palette, &graph);
        let ending = &observed[1];
        assert_eq!(ending.context.curved_continuation, None);
        assert_eq!(ending.context.support, None);
        assert_eq!(ending.source_support, None);
        assert_eq!(score(ending), None);
        assert_eq!(ending.context.source_coverage, 1.0);
        assert_eq!(ending.source_coverage, 1.0);
    }
}

#[test]
fn an_unobserved_cap_corner_cannot_be_averaged_away() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let mut mask = vec![true; SIDE * SIDE];
    let corner = 68 * SIDE + 68;
    mask[corner] = false;
    let mut baseline = None;
    for hidden in [0, 255] {
        let mut pixels = image.pixels().to_vec();
        pixels[corner] = hidden;
        let masked =
            Capture::new(SIDE as u32, SIDE as u32, pixels, Some(mask.clone()), None).unwrap();
        let observed = assess(&masked, &palette, &graph);
        assert_eq!(observed[1].context.support, None);
        assert_eq!(score(&observed[1]), None);
        if let Some(original) = &baseline {
            assert_eq!(&observed, original);
        } else {
            baseline = Some(observed);
        }
    }
}

#[test]
fn a_light_forward_cap_does_not_override_visible_curved_continuation() {
    let graph = ending();
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| {
            horizontal_ridge(x, y, 64.0)
                || ((x - 64.0).abs() < UNIT / 2.0 && (64.0..=100.0).contains(&y))
        },
        None,
    );
    let curved = assess(&image, &palette, &graph);
    assert_eq!(curved[1].termination, Some(0.0));
    assert_eq!(score(&curved[1]), Some(0.0));
    assert!(
        curved[1]
            .reasons
            .contains(&FeatureReason::PossibleContinuation)
    );
}

#[test]
fn nearby_facing_path_reduces_ending_geometry_without_inventing_a_join() {
    let mut graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let isolated = assess(&image, &palette, &graph);
    graph.nodes.extend([
        node(point(74.0, 64.0), NodeKind::Ending),
        node(point(106.0, 64.0), NodeKind::Ending),
    ]);
    graph.edges.push(RidgeEdge {
        start: 2,
        end: 3,
        points: vec![point(74.0, 64.0), point(106.0, 64.0)],
    });
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| {
            horizontal_ridge(x, y, 64.0)
                || ((74.0..=106.0).contains(&x) && (y - 64.0).abs() < UNIT / 2.0)
        },
        None,
    );
    let facing = assess(&image, &palette, &graph);
    assert!(facing[1].continuation.unwrap() > isolated[1].continuation.unwrap());
    assert!(facing[1].geometry_support.unwrap() < isolated[1].geometry_support.unwrap());
    assert_eq!(graph.edges.len(), 2);
}

#[test]
fn aligned_ridge_after_a_short_gap_weakens_the_broad_ending_context() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let isolated = assess(&image, &palette, &graph);

    let graph = ending_with_aligned_continuation();
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| {
            horizontal_ridge(x, y, 64.0)
                || ((76.0..=108.0).contains(&x) && (y - 64.0).abs() < UNIT / 2.0)
        },
        None,
    );
    let broken = assess(&image, &palette, &graph);
    assert!(
        broken[1].context.support.unwrap() < isolated[1].context.support.unwrap(),
        "a visible aligned continuation after 1.5R must contradict the broad ending evidence; isolated P={:?}, G={:?}, C={:?}, Q={:?}; broken P={:?}, G={:?}, C={:?}, Q={:?}",
        isolated[1].source_support,
        isolated[1].geometry_support,
        isolated[1].context.support,
        score(&isolated[1]),
        broken[1].source_support,
        broken[1].geometry_support,
        broken[1].context.support,
        score(&broken[1]),
    );
    assert!(
        score(&broken[1]).unwrap() < 0.75 * score(&isolated[1]).unwrap(),
        "the contradiction must affect structural support beyond the limited local correction",
    );
    assert_eq!(graph.edges.len(), 2, "scoring must not invent a join");
}

#[test]
fn a_forward_graph_path_without_source_ridges_does_not_contradict_the_ending_context() {
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let isolated = assess(&image, &palette, &ending());
    let unconfirmed = assess(&image, &palette, &ending_with_aligned_continuation());
    assert_eq!(unconfirmed[1].context.support, isolated[1].context.support);
    assert_eq!(
        unconfirmed[1].context.termination,
        isolated[1].context.termination,
    );
    assert!(
        unconfirmed[1].geometry_support < isolated[1].geometry_support,
        "raw local geometry stays separate from source-confirmed context",
    );
}

#[test]
fn a_separate_parallel_ridge_behind_the_tip_does_not_contradict_termination() {
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let isolated = assess(&image, &palette, &ending());
    let mut parallel = ending();
    parallel.nodes.extend([
        node(point(16.0, 48.0), NodeKind::Ending),
        node(point(48.0, 48.0), NodeKind::Ending),
    ]);
    parallel.edges.push(RidgeEdge {
        start: 2,
        end: 3,
        points: vec![point(16.0, 48.0), point(48.0, 48.0)],
    });
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| {
            horizontal_ridge(x, y, 64.0)
                || ((8.0..=48.0).contains(&x) && (y - 48.0).abs() < UNIT / 2.0)
        },
        None,
    );
    let parallel = assess(&image, &palette, &parallel);
    assert_eq!(parallel[1].context.support, isolated[1].context.support);
    assert_eq!(
        parallel[1].context.termination,
        isolated[1].context.termination
    );
}

#[test]
fn missing_forward_context_keeps_measured_source_but_geometry_and_score_unknown() {
    let graph = ending();
    let mut mask = vec![true; SIDE * SIDE];
    mask[64 * SIDE + 80] = false;
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| horizontal_ridge(x, y, 64.0),
        Some(mask),
    );
    let masked = assess(&image, &palette, &graph);
    assert!(masked[1].source_support.is_some());
    assert_eq!(masked[1].geometry_support, None);
    assert_eq!(masked[1].context.support, None);
    assert_eq!(score(&masked[1]), None);

    // The source cap is visible, but the complete 3R geometry horizon is not.
    let (image, palette) = capture(80, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let frame = assess(&image, &palette, &graph);
    assert!(frame[1].source_support.is_some());
    assert_eq!(frame[1].geometry_support, None);
    assert_eq!(frame[1].context.support, None);
    assert_eq!(score(&frame[1]), None);
}

#[test]
fn masked_required_arm_is_unknown_and_hidden_gray_cannot_change_any_evidence() {
    let graph = fork(false);
    let hidden = 74 * SIDE + 64;
    let mut mask = vec![true; SIDE * SIDE];
    mask[hidden] = false;
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| on_graph(&graph, x, y),
        Some(mask.clone()),
    );
    let mut baseline = None;
    for hidden_gray in [0, 255] {
        let mut pixels = image.pixels().to_vec();
        pixels[hidden] = hidden_gray;
        let changed =
            Capture::new(SIDE as u32, SIDE as u32, pixels, Some(mask.clone()), None).unwrap();
        let features = assess(&changed, &palette, &graph);
        assert_eq!(features[0].source_support, None);
        assert_eq!(score(&features[0]), None);
        assert!(features[0].source_coverage < 1.0);
        assert!(
            features[0]
                .reasons
                .contains(&FeatureReason::MissingSourceSupport)
        );
        if let Some(original) = &baseline {
            assert_eq!(&features, original);
        } else {
            baseline = Some(features);
        }
    }
}

fn ring() -> RidgeGraph {
    RidgeGraph {
        nodes: vec![],
        edges: vec![],
        cycles: vec![RidgeCycle {
            points: vec![
                point(48.0, 48.0),
                point(80.0, 48.0),
                point(80.0, 80.0),
                point(48.0, 80.0),
            ],
        }],
    }
}

fn on_ring(x: f64, y: f64) -> bool {
    let outer = (44.0..=84.0).contains(&x) && (44.0..=84.0).contains(&y);
    outer && !((52.0..=76.0).contains(&x) && (52.0..=76.0).contains(&y))
}

#[test]
fn dark_ring_requires_a_light_interior_and_white_artifacts_stay_in_inventory() {
    let graph = ring();
    let (image, palette) = capture(SIDE, 20, 220, on_ring, None);
    let real = assess(&image, &palette, &graph);
    assert!(score(&real[0]).unwrap() > 0.0);
    assert!(real[0].separation.unwrap() > 0.0);

    let (image, palette) = capture(SIDE, 20, 220, |_, _| false, None);
    let white = assess(&image, &palette, &graph);
    assert_eq!(score(&white[0]), Some(0.0));
    assert_eq!(white[0].kind, FeatureKind::Cycle);
    assert_eq!(white[0].source_cycle, Some(0));
    assert!(white[0].reasons.contains(&FeatureReason::LightRidge));

    let (image, palette) = capture(SIDE, 20, 220, |_, _| true, None);
    let filled = assess(&image, &palette, &graph);
    assert_eq!(score(&filled[0]), Some(0.0));
    assert!(filled[0].reasons.contains(&FeatureReason::NoInteriorLight));
}

#[test]
fn cycle_source_evidence_is_invariant_to_seam_and_reversal() {
    let graph = ring();
    let (image, palette) = capture(
        SIDE,
        20,
        220,
        |x, y| on_ring(x, y) && !(x < 54.0 && y < 54.0),
        None,
    );
    let mut baseline = None;
    let mut context_baseline = None;
    for reverse in [false, true] {
        for rotation in 0..4 {
            let mut points = graph.cycles[0].points.clone();
            if reverse {
                points.reverse();
            }
            points.rotate_left(rotation);
            let mut work = sampling::Work::default();
            let mut sampler = sampling::Sampler::new(&image, &palette, &mut work);
            let path = sampler.path(&points, true, 0.5).unwrap();
            let evidence = (path.longest_light.unwrap(), path.support(128.0).unwrap());
            let original = baseline.get_or_insert(evidence);
            assert!((original.0 - evidence.0).abs() < 1e-12);
            assert!((original.1 - evidence.1).abs() < 1e-12);
            // The one physical gap crosses the original polygon seam.
            assert!(evidence.0 > 8.0);

            let oriented = RidgeGraph {
                nodes: Vec::new(),
                edges: Vec::new(),
                cycles: vec![RidgeCycle { points }],
            };
            let features = assess(&image, &palette, &oriented);
            let context = &features[0].context;
            let evidence = (context.support.unwrap(), context.support_2r.unwrap());
            let original = context_baseline.get_or_insert(evidence);
            assert!((original.0 - evidence.0).abs() < 1e-12);
            assert!((original.1 - evidence.1).abs() < 1e-12);
        }
    }
}
