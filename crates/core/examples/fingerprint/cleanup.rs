//! Source evidence and raster provenance for intermediate ridge cleanup.

use gnorix_core::biometrics::fingerprint::{
    ClarityMap, LoopEvidence, NodeKind, RidgeCleanup, SourceDecision, SpurEvidence,
};
use serde_json::{Value, json};

pub(super) fn snapshot(cleanup: &RidgeCleanup, clarity: &ClarityMap) -> Value {
    let nodes: Vec<_> = cleanup.graph.nodes.iter().enumerate().map(|(id, node)| json!({
        "id":id, "x":node.position.x(), "y":node.position.y(),
        "kind":match node.kind {
            NodeKind::Ending=>"ending", NodeKind::Junction=>"junction", NodeKind::Isolated=>"isolated",
        },
        "touches_unobserved":node.touches_unobserved,
        "source_node":cleanup.node_sources[id],
        "pixels":cleanup.node_pixels[id].iter().map(|p|[p.x(),p.y()]).collect::<Vec<_>>(),
    })).collect();
    let edges: Vec<_> = cleanup
        .graph
        .edges
        .iter()
        .zip(&cleanup.edge_origins)
        .enumerate()
        .map(|(id, (edge, origin))| {
            json!({
                "id":id,"start":edge.start,"end":edge.end,
                "points":edge.points.iter().map(|p|[p.x(),p.y()]).collect::<Vec<_>>(),
                "segment_clarity":super::clarity::path_scores(clarity, &edge.points, false),
                "source_edges":origin.source_edges,"source_cycles":origin.source_cycles,
                "source_routes":origin.source_routes,
            })
        })
        .collect();
    let cycles: Vec<_> = cleanup
        .graph
        .cycles
        .iter()
        .zip(&cleanup.cycle_origins)
        .enumerate()
        .map(|(id, (cycle, origin))| {
            json!({
                "id":id,"points":cycle.points.iter().map(|p|[p.x(),p.y()]).collect::<Vec<_>>(),
                "segment_clarity":super::clarity::path_scores(clarity, &cycle.points, true),
                "source_edges":origin.source_edges,"source_cycles":origin.source_cycles,
                "source_routes":origin.source_routes,
            })
        })
        .collect();
    let policy = cleanup.source_policy;
    json!({
        "method":"source-supported-ridge-cleanup",
        "policy":{
            "cursor_diameter_px":cleanup.cursor_diameter_px,"pass_count":1,
            "source":{
                "relative_width_tolerance":policy.relative_width_tolerance,
                "max_ridge_width_px":policy.max_ridge_width_px,
                "dark_midpoint":policy.dark_midpoint,
                "sample_spacing_px":policy.sample_spacing_px,
                "tangent_radius_px":policy.tangent_radius_px,
                "max_backbone_extension_px":policy.max_backbone_extension_px,
                "terminal_lookahead_px":policy.terminal_lookahead_px,
                "terminal_barrier_width_px":policy.terminal_barrier_width_px,
                "terminal_barrier_depth_px":policy.terminal_barrier_depth_px,
                "loop_flank_search_px":policy.loop_flank_search_px,
                "loop_min_valley_length_px":policy.loop_min_valley_length_px,
            },
        },
        "nodes":nodes,"edges":edges,"cycles":cycles,
        "source_node_pixels":cleanup.source_node_pixels.iter().enumerate().map(|(id,pixels)|json!({
            "source_node":id,"pixels":pixels.iter().map(|p|[p.x(),p.y()]).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "junction_routes":cleanup.junction_routes.iter().enumerate().map(|(id,route)|json!({
            "id":id,"source_node":route.source_node,
            "points":route.points.iter().map(|p|[p.x(),p.y()]).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "removed_spurs":cleanup.removed_spurs.iter().map(|spur|json!({
            "source_edge":spur.source_edge,"attachment_node":spur.attachment_node,
            "terminal_node":spur.terminal_node,"arc_length_px":spur.arc_length_px,
        })).collect::<Vec<_>>(),
        "retained_ambiguous_edges":cleanup.retained_ambiguous_edges,
        "retained_unobserved_edges":cleanup.retained_unobserved_edges,
        "retained_unconfirmed_edges":cleanup.retained_unconfirmed_edges,
        "spur_assessments":cleanup.spur_assessments.iter().map(|assessment|json!({
            "source_edge":assessment.source_edge,
            "evidence":spur_evidence(&assessment.evidence),
        })).collect::<Vec<_>>(),
        "loops":cleanup.loops.iter().map(|assessment|json!({
            "source_edges":assessment.source_edges,
            "source_nodes":assessment.source_nodes,
            "removed_edge":assessment.removed_edge,
            "evidence":loop_evidence(&assessment.evidence),
        })).collect::<Vec<_>>(),
    })
}

fn decision(value: SourceDecision) -> &'static str {
    match value {
        SourceDecision::Confirmed => "confirmed",
        SourceDecision::MissingSupport => "missing_support",
        SourceDecision::PossibleContinuation => "possible_continuation",
        SourceDecision::SeparateRidges => "separate_ridges",
        SourceDecision::ImageUnsupported => "image_unsupported",
        SourceDecision::AmbiguousGeometry => "ambiguous_geometry",
    }
}

fn spur_evidence(value: &SpurEvidence) -> Value {
    json!({
        "decision":decision(value.decision),
        "terminal_darkness":value.terminal_darkness,
        "light_barrier_distance_px":value.light_barrier_distance_px,
        "min_barrier_lightness":value.min_barrier_lightness,
    })
}

fn loop_evidence(value: &LoopEvidence) -> Value {
    json!({
        "decision":decision(value.decision),
        "max_width_px":value.max_width_px,
        "mean_interior_darkness":value.mean_interior_darkness,
        "max_valley_lightness":value.max_valley_lightness,
        "longest_valley_length_px":value.longest_valley_length_px,
        "min_flank_lightness":value.min_flank_lightness,
        "arm_darkness":value.arm_darkness,
        "arm_center_distance_px":value.arm_center_distance_px,
        "preferred_arm":value.preferred_arm,
    })
}
