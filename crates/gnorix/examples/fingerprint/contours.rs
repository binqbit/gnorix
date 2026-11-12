//! Serialization of reconstructed geometry and its observed-source provenance.

use gnorix::biometrics::fingerprint::{
    ContourReconstruction, GapStatus, NodeKind, SmoothingAdjustment, SmoothingRejection,
};
use serde_json::{Value, json};

pub(super) fn print_summary(contours: &ContourReconstruction) {
    let smoothed = contours
        .edge_smoothing
        .iter()
        .chain(&contours.cycle_smoothing)
        .filter(|adjustment| adjustment.smoothing_applied)
        .count();
    let accepted = contours
        .joins
        .iter()
        .filter(|join| join.status == GapStatus::Accepted)
        .count();
    println!(
        "Contours: {smoothed} paths smoothed, {accepted} source-supported joins / {} proposals; {} edges, {} isolated cycles",
        contours.joins.len(),
        contours.graph.edges.len(),
        contours.graph.cycles.len()
    );
}

pub(super) fn snapshot(contours: &ContourReconstruction, unit_px: f64) -> Value {
    let nodes: Vec<_> = contours.graph.nodes.iter().zip(&contours.node_sources).enumerate()
        .map(|(id, (node, source_node))| json!({
            "id":id,"x":node.position.x(),"y":node.position.y(),
            "kind":match node.kind { NodeKind::Ending=>"ending",NodeKind::Junction=>"junction",NodeKind::Isolated=>"isolated" },
            "touches_unobserved":node.touches_unobserved,"source_node":source_node,
        })).collect();
    let edges: Vec<_> = contours
        .graph
        .edges
        .iter()
        .zip(&contours.edge_origins)
        .zip(&contours.edge_geometry)
        .enumerate()
        .map(|(id, ((edge, origin), geometry))| {
            let (geometry, error) = super::geometry_json(geometry, unit_px);
            json!({"id":id,"start":edge.start,"end":edge.end,
                "points":edge.points.iter().map(|p|[p.x(),p.y()]).collect::<Vec<_>>(),
                "source_edges":origin.source_edges,"source_cycles":origin.source_cycles,
                "source_joins":origin.source_joins,"geometry":geometry,"geometry_error":error})
        })
        .collect();
    let cycles: Vec<_> = contours
        .graph
        .cycles
        .iter()
        .zip(&contours.cycle_origins)
        .zip(&contours.cycle_geometry)
        .enumerate()
        .map(|(id, ((cycle, origin), geometry))| {
            let (geometry, error) = super::geometry_json(geometry, unit_px);
            json!({"id":id,"points":cycle.points.iter().map(|p|[p.x(),p.y()]).collect::<Vec<_>>(),
                "source_edges":origin.source_edges,"source_cycles":origin.source_cycles,
                "source_joins":origin.source_joins,"geometry":geometry,"geometry_error":error})
        })
        .collect();
    let joins: Vec<_> = contours.joins.iter().enumerate().map(|(id, join)|json!({
        "id":id,"start_node":join.start_node,"end_node":join.end_node,
        "points":join.points.iter().map(|p|[p.x(),p.y()]).collect::<Vec<_>>(),
        "gap_px":join.gap_px,"max_tangent_error_radians":join.max_tangent_error_radians,
        "evidence":join.evidence.map(|evidence|json!({
            "mean_darkness":evidence.mean_darkness,"dark_fraction":evidence.dark_fraction,
            "mean_flank_contrast":evidence.mean_flank_contrast,
            "longest_light_gap_px":evidence.longest_light_gap_px,
        })),
        "status":match join.status {
            GapStatus::Accepted=>"accepted", GapStatus::ImageUnsupported=>"image_unsupported",
            GapStatus::Unobserved=>"unobserved", GapStatus::Ambiguous=>"ambiguous",
            GapStatus::Obstructed=>"obstructed",
        },
    })).collect();

    let policy = contours.gap_policy;
    json!({
        "method":"source-supported-smooth-contours",
        "nodes":nodes,"edges":edges,"cycles":cycles,"joins":joins,
        "smoothing":{
            "sigma_px":contours.smoothing.sigma_px,
            "sample_spacing_px":contours.smoothing.sample_spacing_px,
            "max_displacement_px":contours.smoothing.max_displacement_px,
            "edges":contours.edge_smoothing.iter().enumerate().map(|(id, adjustment)|
                adjustment_json(id, "source_edge", adjustment)).collect::<Vec<_>>(),
            "cycles":contours.cycle_smoothing.iter().enumerate().map(|(id, adjustment)|
                adjustment_json(id, "source_cycle", adjustment)).collect::<Vec<_>>(),
        },
        "policy":{
            "max_gap_px":policy.max_gap_px,"tangent_support_px":policy.tangent_support_px,
            "max_tangent_error_radians":policy.max_tangent_error_radians,
            "max_curvature_per_px":policy.max_curvature_per_px,"clearance_px":policy.clearance_px,
            "flank_offset_px":policy.flank_offset_px,
            "min_mean_darkness":policy.min_mean_darkness,"min_dark_fraction":policy.min_dark_fraction,
            "min_mean_flank_contrast":policy.min_flank_contrast,"max_light_gap_px":policy.max_light_gap_px,
        },
    })
}

fn adjustment_json(id: usize, source_key: &str, adjustment: &SmoothingAdjustment) -> Value {
    json!({
        source_key:id,
        "max_displacement_px":adjustment.max_displacement_px,
        "smoothing_applied":adjustment.smoothing_applied,
        "applied_fraction":adjustment.applied_fraction,
        "retained_reason":adjustment.retained_reason.map(|reason|match reason {
            SmoothingRejection::Support=>"support",
            SmoothingRejection::Intersection=>"intersection",
            SmoothingRejection::Degenerate=>"degenerate",
        }),
    })
}
