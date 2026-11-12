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
    combine(feature.source_support, feature.geometry_support, 1.0)
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

fn horizontal_ridge(x: f64, y: f64, end: f64) -> bool {
    (24.0..=end).contains(&x) && (y - 64.0).abs() < UNIT / 2.0
}

#[test]
fn source_continuation_rejects_an_apparent_ending_without_removing_it() {
    let graph = ending();
    let (image, palette) = capture(SIDE, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let clear = assess(&image, &palette, &graph);
    assert!(clear[1].termination.unwrap() > 0.0);
    assert!(score(&clear[1]).unwrap() > 0.0);

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
    assert_eq!(score(&masked[1]), None);

    // The source cap is visible, but the complete 3R geometry horizon is not.
    let (image, palette) = capture(80, 20, 220, |x, y| horizontal_ridge(x, y, 64.0), None);
    let frame = assess(&image, &palette, &graph);
    assert!(frame[1].source_support.is_some());
    assert_eq!(frame[1].geometry_support, None);
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
        }
    }
}
