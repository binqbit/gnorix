use std::collections::{BTreeMap, BTreeSet};

use gnorix::biometrics::{
    Capture,
    fingerprint::{self, ExtractionConfig, FingerprintError, NodeKind, RidgeMap, SourceDecision},
};

#[test]
fn rejects_invalid_maps_and_configuration_before_processing() {
    assert!(RidgeMap::new(0, 2, vec![], vec![]).is_err());
    assert!(RidgeMap::new(1, 1, vec![true], vec![false]).is_err());
    assert!(RidgeMap::new(2, 1, vec![false], vec![true]).is_err());
    let capture = Capture::new(16, 16, vec![0; 256], None, None).unwrap();
    let mut config = ExtractionConfig {
        bootstrap_grid_locations: 0,
        ..ExtractionConfig::default()
    };
    assert!(matches!(
        fingerprint::extract(&capture, &config),
        Err(FingerprintError::InvalidConfig(_))
    ));
    config.bootstrap_grid_locations = 4096;
    config.max_pixels = 255;
    assert!(matches!(
        fingerprint::extract(&capture, &config),
        Err(FingerprintError::WorkLimit)
    ));
}

#[test]
fn native_pipeline_is_deterministic_and_never_uses_masked_pixels() {
    let size = 96;
    let pixels: Vec<_> = (0..size * size)
        .map(|index| if (index % size) % 12 < 6 { 25 } else { 225 })
        .collect();
    let mask: Vec<_> = (0..size * size)
        .map(|index| !(44..52).contains(&(index % size)))
        .collect();
    let mut changed = pixels.clone();
    for (index, valid) in mask.iter().enumerate() {
        if !valid {
            changed[index] = 17;
        }
    }
    let first = Capture::new(size as u32, size as u32, pixels, Some(mask.clone()), None).unwrap();
    let second = Capture::new(size as u32, size as u32, changed, Some(mask), None).unwrap();
    let config = ExtractionConfig::default();
    let a = fingerprint::extract(&first, &config).unwrap();
    let b = fingerprint::extract(&second, &config).unwrap();
    assert!(!a.graph.edges.is_empty());
    assert_eq!(a.graph, b.graph);
    assert_eq!(a.skeleton, b.skeleton);
    assert_eq!(a.denoised, b.denoised);
    assert_eq!(a.palette, b.palette);
    assert_eq!(a.scale, b.scale);
    assert_eq!(a.edge_geometry, b.edge_geometry);
    assert_eq!(a.contours, b.contours);
    assert_eq!(a.clarity, b.clarity);
    assert_eq!(a.feature_strength, b.feature_strength);
    for edge in &a.graph.edges {
        let left = edge.points.iter().any(|p| p.x() < 44.0);
        let right = edge.points.iter().any(|p| p.x() >= 52.0);
        assert!(!(left && right), "a graph path crossed the invalid stripe");
    }
    for edge in &a.contours.graph.edges {
        let left = edge.points.iter().any(|p| p.x() < 44.0);
        let right = edge.points.iter().any(|p| p.x() >= 52.0);
        assert!(
            !(left && right),
            "a reconstructed contour crossed missing source"
        );
    }
}

#[test]
fn full_measurement_finds_a_small_pattern_outside_the_bootstrap_grid() {
    let size = 300;
    let mut pixels = vec![220; size * size];
    for y in 20..90 {
        for left in [20, 36, 52] {
            for x in left..left + 4 {
                pixels[y * size + x] = 30;
            }
        }
    }
    let capture = Capture::new(size as u32, size as u32, pixels, None, None).unwrap();
    let config = ExtractionConfig {
        bootstrap_grid_locations: 1,
        ..ExtractionConfig::default()
    };
    let result = fingerprint::estimate_scale(&capture, &config).unwrap();
    assert!((result.unit_px - 4.0).abs() < 0.2, "{}", result.unit_px);
    for center in [21.5, 37.5, 53.5] {
        assert!(
            result.observations.iter().any(|observation| {
                let x = (observation.span.start.x() + observation.span.end.x()) / 2.0;
                (x - center).abs() < 1.0
            }),
            "the full pass omitted ridge centered at {center}"
        );
    }
    assert!(result.coverage.total_links > 100);
    assert_eq!(result.coverage.measured_links, result.observations.len());
}

#[test]
fn extracts_the_checked_in_image_in_source_coordinates() {
    let image = image::load_from_memory(include_bytes!("assets/test-fingerprint.png"))
        .unwrap()
        .into_luma8();
    let capture =
        Capture::new(image.width(), image.height(), image.into_raw(), None, None).unwrap();
    let mut result = fingerprint::extract(&capture, &ExtractionConfig::default()).unwrap();
    assert!(!result.bootstrap.profiles.is_empty());
    assert!((28.0..50.0).contains(&result.bootstrap.ridge_period_px));
    assert!(!result.scale.observations.is_empty());
    assert!(result.scale.unit_px.is_finite() && result.scale.unit_px > 0.0);
    assert_eq!(
        result.scale.coverage.measured_links,
        result.scale.observations.len()
    );
    assert!(result.scale.coverage.measured_links <= result.scale.coverage.total_links);
    assert!(result.scale.coverage.measured_length_px <= result.scale.coverage.total_length_px);
    for quality in &result.width_consistency {
        assert_eq!(quality.total_measurements, result.scale.observations.len());
        assert!(quality.accepted_measurements <= quality.total_measurements);
        assert!((0.0..=1.0).contains(&quality.fraction));
    }
    assert!(
        result
            .width_consistency
            .windows(2)
            .all(|pair| pair[0].accepted_measurements <= pair[1].accepted_measurements)
    );
    assert_eq!(result.binary.width(), 1086);
    assert_eq!(result.binary.height(), 1448);
    assert!(!result.graph.nodes.is_empty());
    assert!(!result.graph.edges.is_empty());
    assert_eq!(result.edge_geometry.len(), result.graph.edges.len());
    assert_eq!(result.cycle_geometry.len(), result.graph.cycles.len());
    assert_contour_provenance(&result);
    assert_reported_artifacts_removed(&result);
    assert_clarity_map(&result);
    assert_feature_strength(&result);
    assert_visible_ending_context(&result);
    assert_eq!(result.normalized.len(), capture.pixels().len());
    assert_eq!(result.palette.ridge_gray.len(), capture.pixels().len());
    assert_eq!(result.palette.valley_gray.len(), capture.pixels().len());
    assert_eq!(result.palette.usable.len(), capture.pixels().len());
    assert_eq!(
        result.palette.radius_px,
        (result.bootstrap.ridge_period_px / 2.0).ceil() as usize
    );
    assert!(result.palette.usable.iter().any(|&usable| usable));
    for (index, &usable) in result.palette.usable.iter().enumerate() {
        let dark = result.palette.ridge_gray[index];
        let light = result.palette.valley_gray[index];
        if usable {
            assert!(dark.is_finite() && light.is_finite());
            assert!(0.0 <= dark && dark < light && light <= 255.0);
        } else {
            assert_eq!((dark, light, result.normalized[index]), (0.0, 0.0, 0.0));
        }
    }
    assert_eq!(result.denoised.len(), capture.pixels().len());
    assert!(result.denoised.iter().all(|value| value.is_finite()));
    assert_eq!(result.coherence.len(), capture.pixels().len());

    // The original filter turned pinholes inside this broad ridge into eleven
    // junctions. Retain its larger structure without reproducing that cluster.
    let pinhole_junctions = result
        .graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == NodeKind::Junction
                && (955.0..995.0).contains(&node.position.x())
                && (530.0..558.0).contains(&node.position.y())
        })
        .count();
    assert!(
        pinhole_junctions <= 2,
        "pinhole cluster returned: {pinhole_junctions}"
    );
    // Keep the independently inspected ridge cap and larger three-arm event.
    // These fixture ROIs are regression checks, not biometric tolerance values.
    for (kind, x, y) in [
        (NodeKind::Ending, 592.0, 876.0),
        (NodeKind::Junction, 775.0, 1215.0),
    ] {
        assert!(
            result.graph.nodes.iter().any(|node| node.kind == kind
                && (node.position.x() - x).hypot(node.position.y() - y) < 10.0)
        );
    }

    for (edge, geometry) in result.graph.edges.iter().zip(&result.edge_geometry) {
        assert!(edge.start < result.graph.nodes.len() && edge.end < result.graph.nodes.len());
        for point in &edge.points {
            assert!(point.x() >= 0.0 && point.x() < 1086.0);
            assert!(point.y() >= 0.0 && point.y() < 1448.0);
        }
        if let Ok(value) = geometry {
            assert!(value.arc_length >= value.chord_length);
            assert!(value.turning.iter().all(|p| p.angle_radians.is_finite()));
        }
    }
    assert!(!format!("{result:?}").contains("Point"));
    assert_regional_combinations(&mut result);
}

fn assert_regional_combinations(result: &mut fingerprint::Fingerprint) {
    let config = fingerprint::CombinationConfig {
        min_points: 3,
        max_points: 6,
        min_priority: 0.0,
        spatial: fingerprint::SpatialPolicy {
            min_anchor_points: 1,
            min_anchor_distance_units: 0.0,
        },
        radius_units: 30.0,
        max_work: 10_000_000,
    };
    let selected =
        fingerprint::generate_combinations(&result.features().unwrap(), &config).unwrap();
    assert_eq!(
        selected,
        fingerprint::generate_combinations(&result.features().unwrap(), &config).unwrap()
    );
    assert!(selected.work_used > 0 && selected.work_used <= config.max_work);
    assert_eq!(selected.regions.len(), 4);
    assert!(selected.points.len() >= config.max_points);
    assert!(
        selected
            .points
            .windows(2)
            .all(|pair| pair[0].id < pair[1].id)
    );

    let originals: BTreeMap<_, _> = result
        .feature_strength
        .candidates
        .iter()
        .map(|feature| (feature.id, feature))
        .collect();
    for point in &selected.points {
        let original = originals[&point.id];
        assert_eq!(point.position, original.position);
        assert_eq!(point.kind, original.kind);
        assert_eq!(Some(point.priority), original.priority);
    }

    assert_combination_subsets(&selected, &config);
    for (id, region) in selected.regions.iter().enumerate() {
        assert_eq!(region.id, id);
        assert_combination_region(&selected, region, &config);
    }
    assert_combination_failures(result, &selected, config);
    assert_single_region_selection(result, config);

    // Source IDs, rather than incidental storage order, define the output.
    result.feature_strength.candidates.reverse();
    let reordered =
        fingerprint::generate_combinations(&result.features().unwrap(), &config).unwrap();
    result.feature_strength.candidates.reverse();
    assert_eq!(selected, reordered);
}

fn assert_single_region_selection(
    result: &fingerprint::Fingerprint,
    config: fingerprint::CombinationConfig,
) {
    let selected = fingerprint::generate_combinations(
        &result.features().unwrap(),
        &fingerprint::CombinationConfig {
            radius_units: 1000.0,
            ..config
        },
    )
    .unwrap();
    assert_eq!(selected.regions.len(), 1);
    assert!(selected.combinations.len() >= selected.points.len() - config.max_points);
    for region in &selected.regions {
        assert_eq!(region.feature_ids.len(), selected.points.len());
        assert_eq!(region.feature_ids, selected.regions[0].feature_ids);
        assert_eq!(region.combination_count, selected.combinations.len());
    }
    for combination in &selected.combinations {
        assert_eq!(combination.regions, [0]);
    }
}

fn assert_combination_subsets(
    selected: &fingerprint::FeatureCombinations,
    config: &fingerprint::CombinationConfig,
) {
    let mut unique = BTreeSet::new();
    assert!(!selected.combinations.is_empty());
    for combination in &selected.combinations {
        assert!(unique.insert(&combination.feature_ids));
        assert!(
            combination
                .feature_ids
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!((config.min_points..=config.max_points).contains(&combination.feature_ids.len()));
        assert!(combination.score.is_finite() && (0.1..=1.0).contains(&combination.score));
        assert!(!combination.regions.is_empty());
        assert!(combination.regions.windows(2).all(|pair| pair[0] < pair[1]));
        for &id in &combination.regions {
            let region = &selected.regions[id];
            assert!(combination.feature_ids.len() <= region.feature_ids.len());
            assert!(
                combination
                    .feature_ids
                    .iter()
                    .all(|id| region.feature_ids.contains(id))
            );
        }
    }
    assert!(selected.combinations.windows(2).all(|pair| {
        pair[0].score > pair[1].score
            || (pair[0].score == pair[1].score && pair[0].feature_ids < pair[1].feature_ids)
    }));
}

fn assert_combination_region(
    selected: &fingerprint::FeatureCombinations,
    region: &fingerprint::CombinationRegion,
    config: &fingerprint::CombinationConfig,
) {
    let points: BTreeMap<_, _> = selected
        .points
        .iter()
        .map(|point| (point.id, point))
        .collect();
    assert!(region.feature_ids.len() >= config.max_points);
    assert!(region.radius.is_finite() && region.radius >= region.initial_radius);
    assert!(region.feature_ids.windows(2).all(|pair| {
        let (a, b) = (points[&pair[0]], points[&pair[1]]);
        a.priority > b.priority || (a.priority == b.priority && a.id < b.id)
    }));
    for id in &region.feature_ids {
        let position = points[id].position;
        let distance = (position.x() - region.center.x()).hypot(position.y() - region.center.y());
        assert!(distance <= region.radius);
    }

    let subsets: Vec<_> = selected
        .combinations
        .iter()
        .filter(|combination| combination.regions.contains(&region.id))
        .collect();
    assert_eq!(subsets.len(), region.combination_count);
    let incidence: Vec<_> = region
        .feature_ids
        .iter()
        .map(|id| {
            subsets
                .iter()
                .filter(|subset| subset.feature_ids.contains(id))
                .count()
        })
        .collect();
    assert!(incidence.iter().all(|&count| count > 0));
}

fn assert_combination_failures(
    result: &fingerprint::Fingerprint,
    selected: &fingerprint::FeatureCombinations,
    config: fingerprint::CombinationConfig,
) {
    use fingerprint::{CombinationConfig, CombinationError, generate_combinations};

    assert!(matches!(
        generate_combinations(
            &result.features().unwrap(),
            &CombinationConfig {
                min_points: 0,
                ..config
            }
        ),
        Err(CombinationError::InvalidConfig(_))
    ));
    for max_work in [1, selected.work_used - 1] {
        assert_eq!(
            generate_combinations(
                &result.features().unwrap(),
                &CombinationConfig { max_work, ..config }
            ),
            Err(CombinationError::WorkLimit)
        );
    }
    let required = selected.points.len() + 1;
    assert_eq!(
        generate_combinations(
            &result.features().unwrap(),
            &CombinationConfig {
                min_points: required,
                max_points: required,
                ..config
            }
        ),
        Err(CombinationError::InsufficientPoints {
            required,
            available: selected.points.len(),
        })
    );
}

fn assert_visible_ending_context(result: &fingerprint::Fingerprint) {
    // Independently inspected source caps, not scoring coordinates. These
    // endings lost all context to footprint gaps or one contaminated cap edge
    // despite separated, continuous incoming ridges in the original capture.
    for (x, y) in [
        (466.0, 535.0),
        (316.0, 628.0),
        (567.0, 292.0),
        (855.0, 694.0),
        (957.0, 925.0),
        (101.0, 384.0),
        (223.0, 307.0),
    ] {
        let candidate = result
            .feature_strength
            .candidates
            .iter()
            .find(|candidate| candidate.position.x() == x && candidate.position.y() == y)
            .expect("reported ridge ending must remain observed");
        assert_eq!(candidate.kind, fingerprint::FeatureKind::Ending);
        assert!(
            candidate
                .context
                .termination
                .is_some_and(|value| value > 0.0),
            "visible source cap at ({x}, {y}) was rejected: {:?}",
            candidate.context
        );
        assert!(candidate.priority.is_some_and(|value| value > 0.0));
    }
}

fn assert_reported_artifacts_removed(result: &fingerprint::Fingerprint) {
    let cleanup = &result.contours.cleanup;
    // These exact source-image landmarks identify the reported H-shaped
    // processing artifact. They are regression locations, not algorithm knobs.
    for (x, y) in [(926.0, 1322.0), (934.0, 1323.0), (925.0, 1335.0)] {
        let terminal = result
            .graph
            .nodes
            .iter()
            .position(|node| {
                node.kind == NodeKind::Ending && node.position.x() == x && node.position.y() == y
            })
            .expect("reported short branch must remain in the original observation");
        assert!(
            cleanup
                .removed_spurs
                .iter()
                .any(|spur| spur.terminal_node == terminal),
            "reported short branch at ({x}, {y}) was not corrected"
        );
    }

    // The fourth arm has a source-dark lateral continuation from (934, 1336)
    // to (920, 1336). A light pixel at (920, 1335) prevents explaining it as
    // the retained parent's normal footprint. Preserve this uncertainty.
    let uncertain_terminal = result
        .graph
        .nodes
        .iter()
        .position(|node| node.position.x() == 934.0 && node.position.y() == 1340.0)
        .expect("reported uncertain terminal must remain observed");
    let uncertain_edge = result
        .graph
        .edges
        .iter()
        .position(|edge| edge.start == uncertain_terminal || edge.end == uncertain_terminal)
        .unwrap();
    assert!(cleanup.retained_unconfirmed_edges.contains(&uncertain_edge));
    assert!(cleanup.spur_assessments.iter().any(|assessment| {
        assessment.source_edge == uncertain_edge
            && assessment.evidence.decision == fingerprint::SourceDecision::PossibleContinuation
    }));

    let arms: Vec<_> = result
        .graph
        .edges
        .iter()
        .enumerate()
        .filter_map(|(id, edge)| {
            let first = edge.points.first().unwrap();
            let last = edge.points.last().unwrap();
            ((first.x(), first.y()) == (833.0, 423.0) && (last.x(), last.y()) == (850.0, 436.0))
                .then_some(id)
        })
        .collect();
    assert_eq!(
        arms.len(),
        2,
        "reported lens must remain in the original observation"
    );
    assert!(
        cleanup.loops.iter().any(|assessment| {
            assessment.source_edges.as_slice() == arms.as_slice()
                && assessment.removed_edge.is_some()
        }),
        "reported duplicate ridge was not collapsed"
    );
}

fn assert_feature_strength(result: &fingerprint::Fingerprint) {
    let assessment = &result.feature_strength;
    assert_eq!(
        assessment.candidates.len(),
        result.graph.nodes.len() + result.graph.cycles.len()
    );
    for (id, candidate) in assessment.candidates.iter().enumerate() {
        assert_eq!(candidate.id, id);
        if id < result.graph.nodes.len() {
            assert_eq!(candidate.source_node, Some(id));
            assert_eq!(candidate.source_cycle, None);
            assert_eq!(candidate.position, result.graph.nodes[id].position);
        } else {
            assert_eq!(candidate.source_node, None);
            assert_eq!(candidate.source_cycle, Some(id - result.graph.nodes.len()));
        }
        let matches = candidate.stress_matches.iter().flatten().count();
        assert_eq!(candidate.stability, matches as f64 / 8.0);
        assert!((0.0..=1.0).contains(&candidate.source_coverage));
        assert!((0.0..=1.0).contains(&candidate.context.source_coverage));
        assert!((0.5..=1.0).contains(&candidate.neighborhood.factor));
        for score in [
            candidate.source_support,
            candidate.geometry_support,
            candidate.strength,
            candidate.context.support,
            candidate.context.support_2r,
            candidate.priority,
        ]
        .into_iter()
        .flatten()
        {
            assert!(score.is_finite() && (0.0..=1.0).contains(&score));
        }
        match (
            candidate.source_support,
            candidate.geometry_support,
            candidate.context.support,
        ) {
            (Some(source), Some(geometry), Some(context)) => {
                let expected = context
                    * (0.75 + 0.25 * source.min(geometry))
                    * (0.5 + 0.5 * candidate.stability);
                assert!((candidate.strength.unwrap() - expected).abs() < 1e-12);
                assert!(candidate.strength.unwrap() <= context);
                assert!(
                    (candidate.priority.unwrap() - expected * candidate.neighborhood.factor).abs()
                        < 1e-12
                );
                assert!(candidate.priority.unwrap() <= candidate.strength.unwrap());
            }
            _ => {
                assert_eq!(candidate.strength, None);
                assert_eq!(candidate.priority, None);
            }
        }
    }
    for case in 0..8 {
        let mut matches: Vec<_> = assessment
            .candidates
            .iter()
            .filter_map(|candidate| candidate.stress_matches[case])
            .collect();
        let matched = matches.len();
        matches.sort_unstable();
        matches.dedup();
        assert_eq!(
            matches.len(),
            matched,
            "stress features must match one-to-one"
        );
    }

    // Ranking must preserve candidates removed from the separate contour view.
    for removed in &result.contours.cleanup.removed_spurs {
        assert_eq!(
            assessment.candidates[removed.terminal_node].source_node,
            Some(removed.terminal_node)
        );
    }
}

fn assert_contour_provenance(result: &fingerprint::Fingerprint) {
    let contours = &result.contours;
    let cleanup = &contours.cleanup;
    let cleaned = &cleanup.graph;
    assert_eq!(cleanup.cursor_diameter_px, result.scale.unit_px);
    assert_eq!(cleanup.source_policy.relative_width_tolerance, 0.2);
    assert_eq!(
        cleanup.source_policy.max_ridge_width_px,
        1.2 * result.scale.unit_px
    );
    let mut raw_edges = vec![0; result.graph.edges.len()];
    let mut raw_cycles = vec![0; result.graph.cycles.len()];
    let mut routes = vec![0; cleanup.junction_routes.len()];
    for origin in cleanup.edge_origins.iter().chain(&cleanup.cycle_origins) {
        for &id in &origin.source_edges {
            raw_edges[id] += 1;
        }
        for &id in &origin.source_cycles {
            raw_cycles[id] += 1;
        }
        for &id in &origin.source_routes {
            routes[id] += 1;
        }
    }
    for removed in &cleanup.removed_spurs {
        assert!(removed.arc_length_px < cleanup.source_policy.max_ridge_width_px);
        raw_edges[removed.source_edge] += 1;
        assert_eq!(
            result.graph.nodes[removed.terminal_node].kind,
            NodeKind::Ending
        );
        let assessment = cleanup
            .spur_assessments
            .iter()
            .find(|assessment| assessment.source_edge == removed.source_edge)
            .unwrap();
        assert_eq!(assessment.evidence.decision, SourceDecision::Confirmed);
    }
    for assessment in &cleanup.loops {
        if let Some(removed) = assessment.removed_edge {
            assert_eq!(assessment.evidence.decision, SourceDecision::Confirmed);
            assert!(assessment.source_edges.contains(&removed));
            let retained = assessment.source_edges[assessment.evidence.preferred_arm.unwrap()];
            assert_ne!(removed, retained);
            assert!(
                assessment.evidence.max_width_px.unwrap()
                    < cleanup.source_policy.max_ridge_width_px
            );
            raw_edges[removed] += 1;
        }
    }
    assert!(
        raw_edges
            .iter()
            .chain(&raw_cycles)
            .chain(&routes)
            .all(|&count| count == 1)
    );
    assert_eq!(cleanup.node_sources.len(), cleaned.nodes.len());
    for ((node, pixels), &source) in cleaned
        .nodes
        .iter()
        .zip(&cleanup.node_pixels)
        .zip(&cleanup.node_sources)
    {
        assert!(pixels.contains(&node.position));
        assert!(
            pixels
                .iter()
                .all(|pixel| cleanup.source_node_pixels[source].contains(pixel))
        );
    }
    assert_eq!(contours.edge_geometry.len(), contours.graph.edges.len());
    assert_eq!(contours.cycle_geometry.len(), contours.graph.cycles.len());
    assert_eq!(contours.edge_smoothing.len(), cleaned.edges.len());
    assert_eq!(contours.cycle_smoothing.len(), cleaned.cycles.len());
    assert_eq!(contours.node_sources.len(), contours.graph.nodes.len());
    for (node, &source) in contours.graph.nodes.iter().zip(&contours.node_sources) {
        assert_eq!(node, &cleaned.nodes[source]);
    }

    let mut edges = vec![0; cleaned.edges.len()];
    let mut cycles = vec![0; cleaned.cycles.len()];
    let mut joins = vec![0; contours.joins.len()];
    for origin in contours.edge_origins.iter().chain(&contours.cycle_origins) {
        for &index in &origin.source_edges {
            edges[index] += 1;
        }
        for &index in &origin.source_cycles {
            cycles[index] += 1;
        }
        for &index in &origin.source_joins {
            joins[index] += 1;
        }
    }
    assert!(edges.iter().chain(&cycles).all(|&count| count == 1));
    for (join, count) in contours.joins.iter().zip(joins) {
        assert_eq!(
            count,
            usize::from(join.status == fingerprint::GapStatus::Accepted)
        );
    }

    for (edge, geometry) in contours.graph.edges.iter().zip(&contours.edge_geometry) {
        if let Ok(geometry) = geometry {
            let length: f64 = edge
                .points
                .windows(2)
                .map(|pair| (pair[1].x() - pair[0].x()).hypot(pair[1].y() - pair[0].y()))
                .sum();
            assert!((length - geometry.arc_length).abs() < 1e-8);
        }
    }
}

fn assert_clarity_map(result: &fingerprint::Fingerprint) {
    let map = &result.clarity;
    assert_eq!(map.width(), result.binary.width());
    assert_eq!(map.height(), result.binary.height());
    assert_eq!(map.period_px, result.bootstrap.ridge_period_px);
    let size = map.width() * map.height();
    assert_eq!(map.supported.len(), size);
    for values in [&map.values, &map.contrast, &map.separation, &map.repetition] {
        assert_eq!(values.len(), size);
        for (&value, &supported) in values.iter().zip(&map.supported) {
            assert!(value.is_finite() && (0.0..=1.0).contains(&value));
            if !supported {
                assert_eq!(value, 0.0);
            }
        }
    }
    assert!(map.supported.iter().any(|&valid| valid));
    assert!(map.values.iter().any(|&value| value > 0.0));
    for y in (0..map.height()).step_by(map.grid_spacing_px) {
        for x in (0..map.width()).step_by(map.grid_spacing_px) {
            let index = y * map.width() + x;
            let expected = map.contrast[index] * map.separation[index] * map.repetition[index];
            assert!((map.values[index] - expected).abs() < 1e-6);
        }
    }
    assert!(!format!("{map:?}").contains("values: ["));
}
