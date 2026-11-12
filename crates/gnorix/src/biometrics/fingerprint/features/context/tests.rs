use super::super::super::{RidgeEdge, RidgeNode};
use super::super::{ExtractionConfig, StabilityPolicy, StressCase};
use super::tracing::TraceStep;
use super::*;
use crate::biometrics::geometry::Point;

fn point(x: f64, y: f64) -> Point {
    Point::new(x, y).unwrap()
}

#[test]
fn shared_trunks_do_not_count_twice_but_enclosure_is_retained() {
    let mut traces = vec![
        Trace {
            points: vec![
                point(0.0, 0.0),
                point(1.0, 1.0),
                point(2.0, 0.0),
                point(3.0, 0.0),
                point(5.0, 0.0),
            ],
            steps: steps(0),
            length: 2.0_f64.sqrt() * 2.0 + 3.0,
            stop: ContextStop::Horizon,
        },
        Trace {
            points: vec![
                point(0.0, 0.0),
                point(1.0, -1.0),
                point(2.0, 0.0),
                point(3.0, 0.0),
                point(5.0, 0.0),
            ],
            steps: steps(1),
            length: 2.0_f64.sqrt() * 2.0 + 3.0,
            stop: ContextStop::Horizon,
        },
    ];
    tracing::limit_shared(&mut traces, &mut Work::default()).unwrap();
    assert_eq!(edges(&traces[0]), vec![0]);
    assert_eq!(edges(&traces[1]), vec![1]);
    assert_eq!(traces[0].stop, ContextStop::ClosedPath);
    assert!((traces[0].length - 2.0_f64.sqrt() * 2.0).abs() < 1e-9);
    assert_eq!(traces[0].points.last(), Some(&point(2.0, 0.0)));
}

fn steps(first: usize) -> Vec<TraceStep> {
    let branch_length = 2.0_f64.sqrt() * 2.0;
    vec![
        TraceStep {
            edge: first,
            bridge_start_px: 0.0,
            entry_px: 0.0,
            reversed: false,
        },
        TraceStep {
            edge: 2,
            bridge_start_px: branch_length,
            entry_px: branch_length + 1.0,
            reversed: false,
        },
    ]
}

fn edges(trace: &Trace) -> Vec<usize> {
    trace.steps.iter().map(|step| step.edge).collect()
}

fn policy(horizon: f64) -> FeaturePolicy {
    FeaturePolicy {
        reference_width_px: 10.0,
        branch_horizon_px: 20.0,
        context_horizon_px: horizon,
        tangent_support_px: 5.0,
        sample_spacing_px: 0.5,
        source_search_px: 12.0,
        continuation_search_px: 30.0,
        terminal_barrier_width_px: 10.0,
        terminal_barrier_depth_px: 2.5,
        contrast_reference: ExtractionConfig::default().min_contrast,
        neighborhood_radius_px: 30.0,
        stability: StabilityPolicy {
            max_position_shift_px: 10.0,
            arm_radius_px: 10.0,
            max_arm_angle_radians: std::f64::consts::PI / 6.0,
            max_arm_shape_error_px: 2.5,
            max_isolated_shift_px: 2.5,
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

fn port_graph(reverse: bool) -> RidgeGraph {
    let nodes = [
        (point(20.0, 64.0), NodeKind::Ending),
        (point(33.0, 64.0), NodeKind::Junction),
        (point(80.0, 64.0), NodeKind::Ending),
    ]
    .map(|(position, kind)| RidgeNode {
        position,
        kind,
        touches_unobserved: false,
    });
    let mut edges = vec![
        RidgeEdge {
            start: 0,
            end: 1,
            points: vec![point(20.0, 64.0), point(32.0, 64.0)],
        },
        RidgeEdge {
            start: 1,
            end: 2,
            points: vec![point(34.0, 64.0), point(80.0, 64.0)],
        },
    ];
    if reverse {
        for edge in &mut edges {
            std::mem::swap(&mut edge.start, &mut edge.end);
            edge.points.reverse();
        }
    }
    RidgeGraph {
        nodes: nodes.into(),
        edges,
        cycles: Vec::new(),
    }
}

fn horizontal_source(masked_column: Option<usize>, hidden_gray: u8) -> (Capture, LocalPalette) {
    let mut pixels: Vec<_> = (0..128 * 128)
        .map(|index| {
            if (60..=68).contains(&(index / 128)) {
                20
            } else {
                220
            }
        })
        .collect();
    let mask = masked_column.map(|column| {
        let mut mask = vec![true; pixels.len()];
        mask[64 * 128 + column] = false;
        pixels[64 * 128 + column] = hidden_gray;
        mask
    });
    let palette = LocalPalette {
        radius_px: 10,
        ridge_gray: vec![20.0; pixels.len()],
        valley_gray: vec![220.0; pixels.len()],
        usable: vec![true; pixels.len()],
    };
    (Capture::new(128, 128, pixels, mask, None).unwrap(), palette)
}

fn port_trace(
    graph: &RidgeGraph,
    capture: &Capture,
    palette: &LocalPalette,
    horizon: f64,
) -> (Trace, bool) {
    let mut work = Work::default();
    let mut sampler = Sampler::new(capture, palette, &mut work);
    let mut arms = tracing::arms(
        graph,
        &graph::incidence(graph),
        0,
        &policy(horizon),
        &mut sampler,
    )
    .unwrap();
    (arms.remove(0), sampler.complete())
}

#[test]
fn port_connector_does_not_count_as_original_edge_when_clipping_either_direction() {
    let (capture, palette) = horizontal_source(None, 0);
    for reverse in [false, true] {
        let graph = port_graph(reverse);
        let (trace, complete) = port_trace(&graph, &capture, &palette, 40.0);
        assert!(complete);
        assert_eq!(edges(&trace), vec![0, 1]);
        assert_eq!(trace.steps[1].bridge_start_px, 12.0);
        assert_eq!(trace.steps[1].entry_px, 14.0);
        assert_eq!(trace.steps[1].reversed, reverse);
        for horizon in [12.0, 13.0, 14.0, 20.0] {
            let clipped = clipped_backbone(&graph, &trace, horizon, &mut Work::default()).unwrap();
            assert_eq!(clipped[0], Some(Vec::new()));
            if horizon <= 14.0 {
                assert_eq!(clipped[1], None, "the original edge has not been reached");
            } else {
                let expected = if reverse {
                    vec![point(80.0, 64.0), point(40.0, 64.0)]
                } else {
                    vec![point(40.0, 64.0), point(80.0, 64.0)]
                };
                assert_eq!(clipped[1], Some(expected));
            }
        }
    }
}

#[test]
fn a_horizon_inside_or_at_the_port_connector_does_not_export_an_unused_edge() {
    // The hidden column is beyond both tested horizons. It must not be sampled
    // merely because an outgoing edge exists beyond the confirmed connector.
    let (capture, palette) = horizontal_source(Some(35), 0);
    let graph = port_graph(false);
    for horizon in [13.0, 14.0] {
        let (trace, complete) = port_trace(&graph, &capture, &palette, horizon);
        assert!(complete);
        assert_eq!(edges(&trace), vec![0]);
        assert_eq!(trace.stop, ContextStop::Horizon);
        assert_eq!(trace.length, horizon);
        assert_eq!(trace.points.last(), Some(&point(20.0 + horizon, 64.0)));
    }
}

#[test]
fn hidden_port_connector_stops_observation_independently_of_hidden_gray() {
    let graph = port_graph(false);
    for gray in [0, 255] {
        let (capture, palette) = horizontal_source(Some(33), gray);
        let (trace, complete) = port_trace(&graph, &capture, &palette, 40.0);
        assert!(!complete);
        assert_eq!(edges(&trace), vec![0]);
        assert_eq!(trace.stop, ContextStop::MissingSource);
        assert_eq!(trace.length, 12.0);
    }
}
