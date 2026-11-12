use gnorix::biometrics::{
    Capture,
    fingerprint::{self, ExtractionConfig, FingerprintError, NodeKind, RidgeMap},
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
    let result = fingerprint::extract(&capture, &ExtractionConfig::default()).unwrap();
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
    assert_clarity_map(&result);
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
}

fn assert_contour_provenance(result: &fingerprint::Fingerprint) {
    let contours = &result.contours;
    assert_eq!(contours.edge_geometry.len(), contours.graph.edges.len());
    assert_eq!(contours.cycle_geometry.len(), contours.graph.cycles.len());
    assert_eq!(contours.edge_smoothing.len(), result.graph.edges.len());
    assert_eq!(contours.cycle_smoothing.len(), result.graph.cycles.len());
    assert_eq!(contours.node_sources.len(), contours.graph.nodes.len());
    for (node, &source) in contours.graph.nodes.iter().zip(&contours.node_sources) {
        assert_eq!(node, &result.graph.nodes[source]);
    }

    let mut edges = vec![0; result.graph.edges.len()];
    let mut cycles = vec![0; result.graph.cycles.len()];
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
