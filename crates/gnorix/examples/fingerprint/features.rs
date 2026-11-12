//! Lossless native feature-evidence export; no scoring in the report adapter.

use gnorix::biometrics::fingerprint::{
    FeatureAssessment, FeatureKind, FeaturePolicy, FeatureReason, FeatureStrength,
};
use serde_json::{Value, json};

pub(super) fn snapshot(assessment: &FeatureAssessment) -> Value {
    let policy = &assessment.policy;
    let stability = &policy.stability;
    json!({
        "method": "source-geometry-stress-v1",
        "score": "min(P,G) * (0.5 + 0.5*T)",
        "policy": policy_snapshot(policy),
        "stability_policy": {
            "max_position_shift_px": stability.max_position_shift_px,
            "arm_radius_px": stability.arm_radius_px,
            "max_arm_angle_radians": stability.max_arm_angle_radians,
            "max_arm_shape_error_px": stability.max_arm_shape_error_px,
            "max_isolated_shift_px": stability.max_isolated_shift_px,
            "cycle_relative_tolerance": stability.cycle_relative_tolerance,
        },
        "stress_cases": policy.stress_cases.iter().map(|case| json!({
            "directional_margin":case.directional_margin,
            "min_coherence":case.min_coherence,
            "node_count":case.node_count,
            "cycle_count":case.cycle_count,
        })).collect::<Vec<_>>(),
        "candidates": assessment.candidates.iter().map(candidate_snapshot).collect::<Vec<_>>(),
    })
}

fn policy_snapshot(policy: &FeaturePolicy) -> Value {
    json!({
        "reference_width_px": policy.reference_width_px,
        "branch_horizon_px": policy.branch_horizon_px,
        "tangent_support_px": policy.tangent_support_px,
        "sample_spacing_px": policy.sample_spacing_px,
        "source_search_px": policy.source_search_px,
        "continuation_search_px": policy.continuation_search_px,
        "terminal_barrier_width_px": policy.terminal_barrier_width_px,
        "terminal_barrier_depth_px": policy.terminal_barrier_depth_px,
        "contrast_reference": policy.contrast_reference,
    })
}

fn candidate_snapshot(candidate: &FeatureStrength) -> Value {
    json!({
        "id": candidate.id,
        "kind": match candidate.kind {
            FeatureKind::Ending => "ending",
            FeatureKind::Bifurcation => "bifurcation",
            FeatureKind::Junction => "junction",
            FeatureKind::Isolated => "isolated",
            FeatureKind::Cycle => "cycle",
        },
        "source_node": candidate.source_node,
        "source_cycle": candidate.source_cycle,
        "position": [candidate.position.x(),candidate.position.y()],
        "source_support":candidate.source_support,
        "geometry_support":candidate.geometry_support,
        "stability":candidate.stability,
        "strength":candidate.strength,
        "source_coverage":candidate.source_coverage,
        "geometry_coverage":candidate.geometry_coverage,
        "source_contrast":candidate.source_contrast,
        "arms":candidate.arms.iter().map(|arm| json!({
            "edge":arm.edge, "length_px":arm.length_px,
            "source_darkness":arm.source_darkness,
            "dark_fraction":arm.dark_fraction,
            "source_contrast":arm.source_contrast,
            "longest_light_gap_px":arm.longest_light_gap_px,
        })).collect::<Vec<_>>(),
        "separation":candidate.separation,
        "geometry_separation":candidate.geometry_separation,
        "termination":candidate.termination,
        "continuation":candidate.continuation,
        "reasons":candidate.reasons.iter().map(reason_label).collect::<Vec<_>>(),
        "stress_matches":candidate.stress_matches,
    })
}

fn reason_label(reason: &FeatureReason) -> &'static str {
    match reason {
        FeatureReason::MissingSourceSupport => "missing_source_support",
        FeatureReason::InsufficientArmExtent => "insufficient_arm_extent",
        FeatureReason::UnresolvedDirection => "unresolved_direction",
        FeatureReason::NoSeparatedBranches => "no_separated_branches",
        FeatureReason::UnconfirmedTermination => "unconfirmed_termination",
        FeatureReason::PossibleContinuation => "possible_continuation",
        FeatureReason::IsolatedGeometry => "isolated_geometry",
        FeatureReason::LightRidge => "light_ridge",
        FeatureReason::StressSensitive => "stress_sensitive",
        FeatureReason::NoInteriorLight => "no_interior_light",
    }
}

pub(super) fn print_summary(assessment: &FeatureAssessment) {
    let count = assessment.candidates.len();
    let measured = assessment
        .candidates
        .iter()
        .filter(|candidate| candidate.strength.is_some())
        .count();
    println!(
        "Feature evidence: {count} original candidates; {measured} ranked, {} unknown; 8 threshold stress cases",
        count - measured,
    );
}
