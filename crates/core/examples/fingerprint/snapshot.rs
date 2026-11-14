//! JSON measurements and source-coordinate geometry for the report consumer.

use gnorix_core::biometrics::fingerprint::{
    self, CurveGeometry, ExtractionConfig, Fingerprint, NodeKind, WidthConsistency,
};
use serde_json::{Value, json};

use super::{clarity, cleanup, contours, features};

pub(super) fn build(
    result: &Fingerprint,
    config: &ExtractionConfig,
) -> Result<Value, fingerprint::FingerprintError> {
    Ok(json!({
        "schema": "gnorix.ridge-extraction.v1",
        "width": result.binary.width(),
        "height": result.binary.height(),
        "algorithm": "Native local dark/light normalization, directional ridge enhancement, Zhang-Suen thinning, topology tracing, source-supported ridge cleanup, constrained Gaussian contours and source-supported cubic gap reconstruction",
        "config": {
            "bootstrap_grid_locations": config.bootstrap_grid_locations,
            "min_contrast": config.min_contrast,
            "min_coherence": config.min_coherence,
        },
        "bootstrap": bootstrap_snapshot(result)?,
        "scale": full_scale_snapshot(result),
        "normalization": {
            "radius_px": result.palette.radius_px,
            "method": "local-two-class-midpoint",
            "reference_encoding": "rounded-u8",
        },
        "clarity": clarity::snapshot(&result.clarity),
        "coordinate_system": "source image pixels, x right and y down; no rescaling",
        "qualifications": [
            "Extraction controls are not biometric acceptance tolerances.",
            "Topology is measured from the processed image, not confirmed anatomy.",
            "No matching, key reconstruction or real-capture qualification is provided.",
        ],
        "images": {
            "source": "source.png",
            "normalized": "normalized.png",
            "denoised": "denoised.png",
            "coherence": "coherence.png",
            "usable": "usable.png",
            "binary": "binary.png",
            "skeleton": "skeleton.png",
            "ridge_level": "ridge-level.png",
            "valley_level": "valley-level.png",
            "normalization_support": "normalization-support.png",
            "measurement_skeleton": "measurement-skeleton.png",
            "pattern_support": "pattern-support.png",
            "clarity": "clarity.png",
            "clarity_contrast": "clarity-contrast.png",
            "clarity_separation": "clarity-separation.png",
            "clarity_repetition": "clarity-repetition.png",
            "clarity_support": "clarity-support.png",
        },
        "nodes": observed_nodes(result),
        "edges": observed_edges(result),
        "cycles": observed_cycles(result),
        "cleanup": cleanup::snapshot(&result.contours.cleanup, &result.clarity),
        "contours": contours::snapshot(&result.contours, result.scale.unit_px, &result.clarity),
        "feature_strength": features::snapshot(&result.feature_strength),
    }))
}

fn observed_nodes(result: &Fingerprint) -> Vec<Value> {
    result
        .graph
        .nodes
        .iter()
        .enumerate()
        .map(|(id, node)| {
            let kind = match node.kind {
                NodeKind::Ending => "ending",
                NodeKind::Junction => "junction",
                NodeKind::Isolated => "isolated",
            };
            json!({
                "id": id,
                "x": node.position.x(),
                "y": node.position.y(),
                "kind": kind,
                "touches_unobserved": node.touches_unobserved,
            })
        })
        .collect()
}

fn observed_edges(result: &Fingerprint) -> Vec<Value> {
    result
        .graph
        .edges
        .iter()
        .zip(&result.edge_geometry)
        .enumerate()
        .map(|(id, (edge, geometry))| {
            let (geometry, error) = geometry_json(geometry, result.scale.unit_px);
            json!({
                "id": id,
                "start": edge.start,
                "end": edge.end,
                "points": edge.points.iter().map(|p| [p.x(), p.y()]).collect::<Vec<_>>(),
                "segment_clarity": clarity::path_scores(&result.clarity, &edge.points, false),
                "geometry": geometry,
                "geometry_error": error,
            })
        })
        .collect()
}

fn observed_cycles(result: &Fingerprint) -> Vec<Value> {
    result
        .graph
        .cycles
        .iter()
        .zip(&result.cycle_geometry)
        .enumerate()
        .map(|(id, (cycle, geometry))| {
            let (geometry, error) = geometry_json(geometry, result.scale.unit_px);
            json!({
                "id": id,
                "points": cycle.points.iter().map(|p| [p.x(), p.y()]).collect::<Vec<_>>(),
                "segment_clarity": clarity::path_scores(&result.clarity, &cycle.points, true),
                "geometry": geometry,
                "geometry_error": error,
            })
        })
        .collect()
}

fn bootstrap_snapshot(result: &Fingerprint) -> Result<Value, fingerprint::FingerprintError> {
    let scale = &result.bootstrap;
    let consistency = scale.width_consistency()?;
    Ok(json!({
        "ridge_width_px":scale.ridge_width_px,
        "valley_width_px":scale.valley_width_px,
        "ridge_period_px":scale.ridge_period_px,
        "ridge_width_mad_px":scale.ridge_width_mad_px,
        "ridge_period_mad_px":scale.ridge_period_mad_px,
        "mean_ridge_width_px":scale.mean_ridge_width_px,
        "mean_valley_width_px":scale.mean_valley_width_px,
        "mean_ridge_gray":scale.mean_ridge_gray,
        "mean_valley_gray":scale.mean_valley_gray,
        "examined_locations":scale.examined_locations,
        "profiles":scale.profiles.iter().map(|profile| json!({
            "ridges":profile.ridges.iter().map(|ridge| json!({
                "start":[ridge.start.x(),ridge.start.y()],
                "end":[ridge.end.x(),ridge.end.y()],
            })).collect::<Vec<_>>(),
            "ridge_mean_gray":profile.ridge_mean_gray,
            "valley_mean_gray":profile.valley_mean_gray,
        })).collect::<Vec<_>>(),
        "width_consistency":width_consistency(&consistency),
    }))
}

fn full_scale_snapshot(result: &Fingerprint) -> Value {
    let scale = &result.scale;
    let coverage = &scale.coverage;
    json!({
        "method":"raw-centerline-link-width",
        "unit_px":scale.unit_px,
        "coverage":{
            "total_length_px":coverage.total_length_px,
            "measured_length_px":coverage.measured_length_px,
            "total_links":coverage.total_links,
            "measured_links":coverage.measured_links,
            "isolated_pixels":coverage.isolated_pixels,
        },
        "observations":scale.observations.iter().map(|observation|json!({
            "link":observation.link,
            "start":[observation.span.start.x(),observation.span.start.y()],
            "end":[observation.span.end.x(),observation.span.end.y()],
            "represented_length_px":observation.represented_length_px,
            "mean_gray":observation.mean_gray,
        })).collect::<Vec<_>>(),
        "width_consistency":width_consistency(&result.width_consistency),
    })
}

pub(super) fn geometry_json(
    result: &Result<CurveGeometry, fingerprint::FingerprintError>,
    unit_px: f64,
) -> (Value, Option<String>) {
    match result {
        Ok(value) => (
            json!({
                "arc_length": value.arc_length,
                "chord_length": value.chord_length,
                "arc_length_units": value.arc_length / unit_px,
                "chord_length_units": value.chord_length / unit_px,
                "midpoint": [value.midpoint.x(), value.midpoint.y()],
                "turning": value.turning.iter().map(|sample| json!({
                    "arc_fraction": sample.arc_fraction,
                    "angle_radians": sample.angle_radians,
                })).collect::<Vec<_>>(),
            }),
            None,
        ),
        Err(error) => (Value::Null, Some(error.to_string())),
    }
}

fn width_consistency(qualities: &[WidthConsistency]) -> Vec<Value> {
    qualities
        .iter()
        .map(|quality| {
            json!({
                "relative_tolerance":quality.relative_tolerance,
                "accepted_measurements":quality.accepted_measurements,
                "total_measurements":quality.total_measurements,
                "fraction":quality.fraction,
            })
        })
        .collect()
}
