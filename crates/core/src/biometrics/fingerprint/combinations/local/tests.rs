use super::*;

fn positions(values: &[(f64, f64)]) -> Vec<Point> {
    values
        .iter()
        .map(|&(x, y)| Point::new(x, y).unwrap())
        .collect()
}

fn cost(center: Point, selected: &[Point]) -> (f64, f64) {
    let distances = selected
        .iter()
        .map(|point| (point.x() - center.x()).hypot(point.y() - center.y()))
        .collect::<Vec<_>>();
    (
        distances.iter().copied().fold(0.0_f64, f64::max),
        distances.into_iter().sum(),
    )
}

#[test]
fn compact_frontier_matches_an_independent_exhaustive_distance_oracle() {
    let points = positions(&[
        (0., 0.),
        (1., 0.),
        (-1., 2.),
        (4., 0.),
        (0., 5.),
        (-4., -3.),
        (3., 4.),
    ]);
    let priorities = [1., 0.9, 0.8, 0.7, 0.6, 0.5, 0.4];
    for minimum in 3..=5 {
        for maximum in minimum..=5 {
            for separation in [0., 2., 5.] {
                let policy = spatial::SpatialPixels {
                    anchors: 3,
                    separation,
                };
                for center in 0..points.len() {
                    let expected = (1_usize..1 << points.len())
                        .filter(|mask| {
                            mask & (1 << center) != 0
                                && (minimum..=maximum).contains(&(mask.count_ones() as usize))
                        })
                        .filter_map(|mask| {
                            let selected = points
                                .iter()
                                .enumerate()
                                .filter_map(|(rank, &point)| {
                                    (mask & (1 << rank) != 0).then_some(point)
                                })
                                .collect::<Vec<_>>();
                            spatial::check(&selected, policy, &mut WorkBudget::new(10_000))
                                .unwrap()
                                .then(|| cost(points[center], &selected))
                        })
                        .min_by(|first, second| {
                            first
                                .0
                                .total_cmp(&second.0)
                                .then(first.1.total_cmp(&second.1))
                        });
                    let mut work = WorkBudget::new(100_000);
                    let nearby = neighbors(center, &priorities, &points, &mut work).unwrap();
                    let actual =
                        nearest_valid(&nearby, minimum, maximum, &points, policy, &mut work)
                            .unwrap();
                    assert_eq!(actual.is_some(), expected.is_some());
                    if let Some(ranks) = actual {
                        assert!(ranks.contains(&center));
                        let selected = ranks.iter().map(|&rank| points[rank]).collect::<Vec<_>>();
                        let actual = cost(points[center], &selected);
                        let expected = expected.unwrap();
                        assert_eq!(actual.0, expected.0);
                        assert!((actual.1 - expected.1).abs() < 1e-12);
                    }
                }
            }
        }
    }
}

#[test]
fn local_witnesses_preserve_priority_selection_and_cover_a_weaker_compact_cluster() {
    let points = positions(&[
        (0., 0.),
        (80., 0.),
        (0., 80.),
        (80., 80.),
        (0., 40.),
        (80., 40.),
        (40., 0.),
        (40., 40.),
        (45., 40.),
        (48., 44.),
        (45., 48.),
        (40., 48.),
        (37., 44.),
        (42., 44.),
    ]);
    let priorities = [
        0.99, 0.97, 0.95, 0.85, 0.75, 0.65, 0.49, 0.40, 0.38, 0.36, 0.34, 0.32, 0.30, 0.28,
    ];
    let policy = spatial::SpatialPixels {
        anchors: 3,
        separation: 2.,
    };
    let mut work = WorkBudget::new(1_000_000);
    let coverage =
        enumeration::enumerate_spatial(&priorities, &points, 5, 5, policy, &mut work).unwrap();
    assert!(coverage.uncovered.is_empty());
    assert!(coverage.subsets.iter().all(|group| group.ranks[0] < 7));
    let before = coverage.subsets;
    let mut selected = before.clone();
    supplement(&priorities, &points, 5, 5, policy, &mut selected, &mut work).unwrap();
    for group in before {
        assert!(selected.contains(&group));
    }
    assert!(selected.iter().any(|group| group.ranks[0] >= 7));
    let distinct = selected
        .iter()
        .map(|group| &group.ranks)
        .collect::<BTreeSet<_>>();
    assert_eq!(distinct.len(), selected.len());
    for group in &selected {
        assert_eq!(group.ranks.len(), 5);
        assert!(spatial::subset(&points, &group.ranks, policy, &mut work).unwrap());
        assert_eq!(
            group.score,
            enumeration::ranked_subset(&priorities, group.ranks.clone()).score
        );
    }
    let once = selected.clone();
    supplement(&priorities, &points, 5, 5, policy, &mut selected, &mut work).unwrap();
    assert_eq!(
        selected, once,
        "adding the same witnesses must be idempotent"
    );
}

#[test]
fn compact_search_expands_past_close_points_until_the_required_anchors_exist() {
    let points = positions(&[
        (0., 0.),
        (0.5, 0.5),
        (1., 0.),
        (0., 1.),
        (-0.5, 0.),
        (10., 0.),
        (0., 10.),
        (-10., -10.),
    ]);
    let priorities = [1., 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3];
    let policy = spatial::SpatialPixels {
        anchors: 3,
        separation: 5.,
    };
    let mut work = WorkBudget::new(100_000);
    let nearby = neighbors(0, &priorities, &points, &mut work).unwrap();
    assert!(nearby[..5].iter().all(|neighbor| neighbor.distance <= 1.));
    let selected = nearest_valid(&nearby, 5, 5, &points, policy, &mut work)
        .unwrap()
        .unwrap();
    assert!(selected.contains(&0));
    assert!(selected.iter().filter(|&&rank| rank >= 5).count() >= 2);
    assert!(spatial::subset(&points, &selected, policy, &mut work).unwrap());
}

#[test]
fn local_selection_reports_resource_exhaustion() {
    let points = positions(&[(0., 0.), (10., 0.), (0., 10.), (8., 8.)]);
    assert!(matches!(
        supplement(
            &[1., 0.8, 0.6, 0.4],
            &points,
            3,
            3,
            spatial::SpatialPixels {
                anchors: 3,
                separation: 1.
            },
            &mut Vec::new(),
            &mut WorkBudget::new(3)
        ),
        Err(CombinationError::WorkLimit)
    ));
}
