use super::super::super::{
    Capture, NodeKind, RidgeCycle, RidgeEdge, RidgeMap, RidgeNode, enhancement,
};
use super::*;
use crate::biometrics::geometry::Point;

fn point(x: f64, y: f64) -> Point {
    Point::new(x, y).unwrap()
}

fn branch(arms: &[[f64; 2]], shift: [f64; 2]) -> RidgeGraph {
    let center = point(20.0 + shift[0], 20.0 + shift[1]);
    let mut nodes = vec![RidgeNode {
        position: center,
        kind: if arms.len() == 1 {
            NodeKind::Ending
        } else {
            NodeKind::Junction
        },
        touches_unobserved: false,
    }];
    let mut edges = Vec::new();
    for arm in arms {
        let end = point(center.x() + arm[0], center.y() + arm[1]);
        nodes.push(RidgeNode {
            position: end,
            kind: NodeKind::Ending,
            touches_unobserved: false,
        });
        edges.push(RidgeEdge {
            start: 0,
            end: nodes.len() - 1,
            points: vec![center, end],
        });
    }
    RidgeGraph {
        nodes,
        edges,
        cycles: vec![],
    }
}

fn matches(a: &RidgeGraph, b: &RidgeGraph) -> Vec<Option<usize>> {
    let policy = StabilityPolicy::for_unit(8.0).unwrap();
    let mut work = matching::Work::default();
    let first = matching::describe(a, policy, &mut work).unwrap();
    let second = matching::describe(b, policy, &mut work).unwrap();
    matching::correspond(&first, &second, policy, &mut work).unwrap()
}

#[test]
fn eight_actual_cases_are_distinct_at_default_and_configuration_boundaries() {
    for coherence in [0.0, f64::MIN_POSITIVE, 0.2, 0.999, 1.0] {
        let cases = stress_cases(coherence);
        for (index, case) in cases.iter().enumerate() {
            assert!((0.0..=1.0).contains(&case.min_coherence));
            assert!(case.directional_margin != 0.0 || case.min_coherence != coherence);
            assert!(
                !cases[..index]
                    .iter()
                    .any(|other| other.directional_margin == case.directional_margin
                        && other.min_coherence == case.min_coherence)
            );
        }
    }
    let cases = stress_cases(0.2);
    assert_eq!(cases[0].directional_margin, -0.05);
    assert!((cases[0].min_coherence - 0.18).abs() < 1e-15);
    assert_eq!(cases[7].directional_margin, 0.05);
    assert!((cases[7].min_coherence - 0.22).abs() < 1e-15);
}

#[test]
fn corresponding_forks_require_degree_and_each_outward_arm() {
    let original = branch(&[[-8.0, 0.0], [8.0, 0.0], [0.0, 8.0]], [0.0, 0.0]);
    let translated = branch(&[[-8.0, 0.0], [8.0, 0.0], [0.0, 8.0]], [1.0, 1.0]);
    assert_eq!(matches(&original, &translated)[0], Some(0));
    let ending = branch(&[[8.0, 0.0]], [0.0, 0.0]);
    assert_eq!(matches(&original, &ending)[0], None);
    let wrong = branch(&[[-8.0, 0.0], [8.0, 0.0], [0.0, -8.0]], [0.0, 0.0]);
    assert_eq!(matches(&original, &wrong)[0], None);
    let extra = branch(
        &[[-8.0, 0.0], [8.0, 0.0], [0.0, 8.0], [0.0, -8.0]],
        [0.0, 0.0],
    );
    assert_eq!(matches(&original, &extra)[0], None);
    let mut wrong_shape = branch(&[[-8.0, 0.0], [8.0, 0.0], [0.0, 8.0]], [0.0, 0.0]);
    wrong_shape.edges[1].points = vec![
        point(20.0, 20.0),
        point(24.0, 20.0),
        point(24.0, 24.0),
        point(28.0, 20.0),
    ];
    assert_eq!(matches(&original, &wrong_shape)[0], None);
}

#[test]
fn compatible_matches_are_one_to_one_and_deterministic() {
    let node = |x| RidgeNode {
        position: point(x, 20.0),
        kind: NodeKind::Isolated,
        touches_unobserved: false,
    };
    let first = RidgeGraph {
        nodes: vec![node(20.0), node(21.0)],
        edges: vec![],
        cycles: vec![],
    };
    let second = RidgeGraph {
        nodes: vec![node(20.5)],
        edges: vec![],
        cycles: vec![],
    };
    let result = matches(&first, &second);
    assert_eq!(result.iter().flatten().count(), 1);
    assert_eq!(result, matches(&first, &second));
    let distant = RidgeGraph {
        nodes: vec![node(25.0)],
        edges: vec![],
        cycles: vec![],
    };
    assert!(matches(&first, &distant).iter().all(Option::is_none));

    // First candidate can use either target; the second can use only target0.
    // An augmenting path must move the first rather than lose a valid match.
    let first = RidgeGraph {
        nodes: vec![node(21.0), node(19.0)],
        edges: vec![],
        cycles: vec![],
    };
    let second = RidgeGraph {
        nodes: vec![node(20.0), node(22.0)],
        edges: vec![],
        cycles: vec![],
    };
    assert_eq!(matches(&first, &second), vec![Some(1), Some(0)]);
}

fn cycle(radius: [f64; 2], shift: [f64; 2]) -> RidgeGraph {
    let points = (0..32)
        .map(|i| {
            let angle = std::f64::consts::TAU * i as f64 / 32.0;
            point(
                20.0 + shift[0] + radius[0] * angle.cos(),
                20.0 + shift[1] + radius[1] * angle.sin(),
            )
        })
        .collect();
    RidgeGraph {
        nodes: vec![],
        edges: vec![],
        cycles: vec![RidgeCycle { points }],
    }
}

#[test]
fn tiny_cycles_cannot_match_nearby_different_extent_or_shape() {
    let original = cycle([1.0, 1.0], [0.0, 0.0]);
    assert_eq!(
        matches(&original, &cycle([1.0, 1.0], [1.0, 0.0])),
        vec![Some(0)]
    );
    assert_eq!(
        matches(&original, &cycle([2.0, 2.0], [0.0, 0.0])),
        vec![None]
    );
    assert_eq!(
        matches(&original, &cycle([1.3, 0.7], [0.0, 0.0])),
        vec![None]
    );
    assert_eq!(
        matches(
            &original,
            &RidgeGraph {
                nodes: vec![],
                edges: vec![],
                cycles: vec![]
            }
        ),
        vec![None]
    );
}

#[test]
fn frozen_baseline_response_recreates_the_exact_observed_graph_and_ids() {
    let (width, height) = (80, 80);
    let pixels = (0..width * height)
        .map(|i| {
            let x = (i % width) as f64;
            let y = (i / width) as f64;
            (128.0 + 70.0 * (x / 3.0 + y / 11.0).cos()).round() as u8
        })
        .collect();
    let capture = Capture::new(width as u32, height as u32, pixels, None, None).unwrap();
    let config = ExtractionConfig::default();
    let prepared = enhancement::preprocess(&capture, &config, 12.0).unwrap();
    let observed = graph::trace(&thinning::thin(&prepared.map).unwrap()).unwrap();
    assert!(!observed.nodes.is_empty());
    let cached = prepared
        .response
        .binarize(0.0, config.min_coherence)
        .unwrap();
    let replay = graph::trace(&thinning::thin(&cached).unwrap()).unwrap();
    assert_eq!(replay, observed);
    assert_eq!(
        matches(&observed, &replay),
        (0..observed.nodes.len() + observed.cycles.len())
            .map(Some)
            .collect::<Vec<_>>()
    );
    let score = assess(&prepared, &observed, 6.0, &config).unwrap();
    assert_eq!(
        score.matches.len(),
        observed.nodes.len() + observed.cycles.len()
    );
    for (index, case) in score.cases.iter().enumerate() {
        let mut ids: Vec<_> = score.matches.iter().filter_map(|row| row[index]).collect();
        assert!(
            ids.iter()
                .all(|&id| id < case.node_count + case.cycle_count)
        );
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}

#[test]
fn source_mask_remains_unusable_in_every_stress_case() {
    let mut mask = vec![true; 64 * 64];
    mask[32 * 64 + 32] = false;
    let pixels = (0..64 * 64)
        .map(|i| if i % 12 < 6 { 40 } else { 200 })
        .collect();
    let capture = Capture::new(64, 64, pixels, Some(mask), None).unwrap();
    let config = ExtractionConfig::default();
    let prepared = enhancement::preprocess(&capture, &config, 12.0).unwrap();
    for case in stress_cases(config.min_coherence) {
        let map: RidgeMap = prepared
            .response
            .binarize(case.directional_margin, case.min_coherence)
            .unwrap();
        assert!(!map.usable()[32 * 64 + 32]);
        assert!(!map.ridges()[32 * 64 + 32]);
    }
}
