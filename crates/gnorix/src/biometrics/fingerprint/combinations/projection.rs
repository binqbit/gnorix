//! Original feature identities that survive cleanup and contour reconstruction.

use std::collections::{BTreeMap, BTreeSet};

use super::super::{
    CleanupOrigin, FeatureKind, FeatureStrength, Fingerprint, FingerprintError, NodeKind,
    PathOrigin, RidgeGraph,
};

/// Preserve original evidence only while its assessed type and degree survive.
/// Repeated source identities appear once, in ascending original candidate ID.
pub(super) fn retained_candidates(
    fingerprint: &Fingerprint,
) -> Result<Vec<&FeatureStrength>, FingerprintError> {
    let contours = &fingerprint.contours;
    let projection = Projection {
        source: &fingerprint.graph,
        cleaned: &contours.cleanup.graph,
        final_graph: &contours.graph,
        cleaned_node_sources: &contours.cleanup.node_sources,
        final_node_sources: &contours.node_sources,
        cleaned_cycle_origins: &contours.cleanup.cycle_origins,
        final_cycle_origins: &contours.cycle_origins,
    };

    project(&fingerprint.feature_strength.candidates, projection)
}

struct Projection<'a> {
    source: &'a RidgeGraph,
    cleaned: &'a RidgeGraph,
    final_graph: &'a RidgeGraph,
    cleaned_node_sources: &'a [usize],
    final_node_sources: &'a [usize],
    cleaned_cycle_origins: &'a [CleanupOrigin],
    final_cycle_origins: &'a [PathOrigin],
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    Node(usize),
    Cycle(usize),
}

fn project<'a>(
    candidates: &'a [FeatureStrength],
    projection: Projection<'_>,
) -> Result<Vec<&'a FeatureStrength>, FingerprintError> {
    validate_provenance(&projection)?;
    let originals = index_candidates(candidates, projection.source)?;
    let degree = node_degrees(projection.final_graph)?;
    let mut retained = BTreeMap::new();

    retain_nodes(&projection, &originals, &degree, &mut retained)?;
    retain_cycles(&projection, &originals, &mut retained)?;

    Ok(retained.into_values().collect())
}

fn validate_provenance(projection: &Projection<'_>) -> Result<(), FingerprintError> {
    if projection.cleaned_node_sources.len() != projection.cleaned.nodes.len()
        || projection.final_node_sources.len() != projection.final_graph.nodes.len()
        || projection.cleaned_cycle_origins.len() != projection.cleaned.cycles.len()
        || projection.final_cycle_origins.len() != projection.final_graph.cycles.len()
    {
        return Err(FingerprintError::InvalidTopology(
            "feature projection provenance does not cover its graph",
        ));
    }
    if projection
        .cleaned_node_sources
        .iter()
        .any(|&source| source >= projection.source.nodes.len())
        || projection
            .final_node_sources
            .iter()
            .any(|&source| source >= projection.cleaned.nodes.len())
        || projection.cleaned_cycle_origins.iter().any(|origin| {
            origin
                .source_cycles
                .iter()
                .any(|&source| source >= projection.source.cycles.len())
        })
        || projection.final_cycle_origins.iter().any(|origin| {
            origin
                .source_cycles
                .iter()
                .any(|&source| source >= projection.cleaned.cycles.len())
        })
    {
        return Err(FingerprintError::InvalidTopology(
            "feature projection refers to an absent source",
        ));
    }
    Ok(())
}

fn index_candidates<'a>(
    candidates: &'a [FeatureStrength],
    source: &RidgeGraph,
) -> Result<BTreeMap<Origin, &'a FeatureStrength>, FingerprintError> {
    let mut originals = BTreeMap::new();
    let mut identities = BTreeSet::new();
    for candidate in candidates {
        let origin = match (candidate.source_node, candidate.source_cycle) {
            (Some(node), None)
                if node < source.nodes.len() && candidate.kind != FeatureKind::Cycle =>
            {
                Origin::Node(node)
            }
            (None, Some(cycle))
                if cycle < source.cycles.len() && candidate.kind == FeatureKind::Cycle =>
            {
                Origin::Cycle(cycle)
            }
            _ => {
                return Err(FingerprintError::InvalidTopology(
                    "feature candidate has an invalid original identity",
                ));
            }
        };
        if !identities.insert(candidate.id) || originals.insert(origin, candidate).is_some() {
            return Err(FingerprintError::InvalidTopology(
                "feature candidates repeat an original identity",
            ));
        }
    }
    Ok(originals)
}

fn node_degrees(graph: &RidgeGraph) -> Result<Vec<usize>, FingerprintError> {
    let mut degree = vec![0_usize; graph.nodes.len()];
    for edge in &graph.edges {
        for node in [edge.start, edge.end] {
            let count = degree
                .get_mut(node)
                .ok_or(FingerprintError::InvalidTopology(
                    "feature projection edge refers to an absent node",
                ))?;
            *count = count.checked_add(1).ok_or(FingerprintError::WorkLimit)?;
        }
    }
    Ok(degree)
}

fn retain_nodes<'a>(
    projection: &Projection<'_>,
    originals: &BTreeMap<Origin, &'a FeatureStrength>,
    degree: &[usize],
    retained: &mut BTreeMap<usize, &'a FeatureStrength>,
) -> Result<(), FingerprintError> {
    for (index, node) in projection.final_graph.nodes.iter().enumerate() {
        let cleaned = projection.final_node_sources[index];
        let source = projection.cleaned_node_sources[cleaned];
        let candidate =
            originals
                .get(&Origin::Node(source))
                .ok_or(FingerprintError::InvalidTopology(
                    "retained node has no original feature evidence",
                ))?;
        if node_kind(node.kind, degree[index]) == candidate.kind
            && degree[index] == candidate.arms.len()
        {
            retained.insert(candidate.id, *candidate);
        }
    }
    Ok(())
}

fn node_kind(kind: NodeKind, degree: usize) -> FeatureKind {
    match kind {
        NodeKind::Ending => FeatureKind::Ending,
        NodeKind::Isolated => FeatureKind::Isolated,
        NodeKind::Junction if degree == 3 => FeatureKind::Bifurcation,
        NodeKind::Junction => FeatureKind::Junction,
    }
}

fn retain_cycles<'a>(
    projection: &Projection<'_>,
    originals: &BTreeMap<Origin, &'a FeatureStrength>,
    retained: &mut BTreeMap<usize, &'a FeatureStrength>,
) -> Result<(), FingerprintError> {
    for origin in projection.final_cycle_origins {
        let mut sources = origin
            .source_cycles
            .iter()
            .flat_map(|&cycle| &projection.cleaned_cycle_origins[cycle].source_cycles)
            .copied();
        // A newly closed chain has no assessed original isolated cycle.
        // Multiple original cycles cannot supply one unchanged feature identity.
        // Two items establish ambiguity without expanding repeated provenance.
        let (Some(source), None) = (sources.next(), sources.next()) else {
            continue;
        };
        let candidate =
            originals
                .get(&Origin::Cycle(source))
                .ok_or(FingerprintError::InvalidTopology(
                    "retained cycle has no original feature evidence",
                ))?;
        retained.insert(candidate.id, *candidate);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::{
        FeatureArm, FeatureContext, FeatureNeighborhood, RidgeCycle, RidgeEdge, RidgeNode,
    };
    use super::*;
    use crate::biometrics::geometry::Point;

    fn point(x: usize) -> Point {
        Point::new(x as f64, 0.0).unwrap()
    }

    fn graph(kinds: &[NodeKind], edges: &[(usize, usize)], cycles: usize) -> RidgeGraph {
        RidgeGraph {
            nodes: kinds
                .iter()
                .enumerate()
                .map(|(index, &kind)| RidgeNode {
                    position: point(index),
                    kind,
                    touches_unobserved: false,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|&(start, end)| RidgeEdge {
                    start,
                    end,
                    points: vec![point(start), point(end)],
                })
                .collect(),
            cycles: (0..cycles)
                .map(|_| RidgeCycle {
                    points: vec![point(0), point(1), Point::new(0.0, 1.0).unwrap()],
                })
                .collect(),
        }
    }

    fn candidate(id: usize, origin: Origin, kind: FeatureKind, degree: usize) -> FeatureStrength {
        FeatureStrength {
            id,
            kind,
            source_node: match origin {
                Origin::Node(node) => Some(node),
                _ => None,
            },
            source_cycle: match origin {
                Origin::Cycle(cycle) => Some(cycle),
                _ => None,
            },
            position: point(id),
            source_support: None,
            geometry_support: None,
            stability: 1.0,
            strength: None,
            context: FeatureContext::default(),
            neighborhood: FeatureNeighborhood::default(),
            priority: None,
            source_coverage: 0.0,
            geometry_coverage: 0.0,
            source_contrast: None,
            arms: (0..degree)
                .map(|edge| FeatureArm {
                    edge,
                    length_px: 1.0,
                    source_darkness: None,
                    dark_fraction: None,
                    source_contrast: None,
                    longest_light_gap_px: None,
                })
                .collect(),
            separation: None,
            geometry_separation: None,
            termination: None,
            continuation: None,
            reasons: vec![],
            stress_matches: [None; 8],
        }
    }

    struct Fixture {
        source: RidgeGraph,
        cleaned: RidgeGraph,
        final_graph: RidgeGraph,
        cleaned_nodes: Vec<usize>,
        final_nodes: Vec<usize>,
        cleaned_cycles: Vec<CleanupOrigin>,
        final_cycles: Vec<PathOrigin>,
    }

    impl Fixture {
        fn new(source: RidgeGraph) -> Self {
            Self {
                cleaned: source.clone(),
                final_graph: source.clone(),
                cleaned_nodes: (0..source.nodes.len()).collect(),
                final_nodes: (0..source.nodes.len()).collect(),
                cleaned_cycles: (0..source.cycles.len())
                    .map(|cycle| CleanupOrigin {
                        source_cycles: vec![cycle],
                        ..CleanupOrigin::default()
                    })
                    .collect(),
                final_cycles: (0..source.cycles.len())
                    .map(|cycle| PathOrigin {
                        source_cycles: vec![cycle],
                        ..PathOrigin::default()
                    })
                    .collect(),
                source,
            }
        }

        fn project<'a>(
            &self,
            candidates: &'a [FeatureStrength],
        ) -> Result<Vec<&'a FeatureStrength>, FingerprintError> {
            project(
                candidates,
                Projection {
                    source: &self.source,
                    cleaned: &self.cleaned,
                    final_graph: &self.final_graph,
                    cleaned_node_sources: &self.cleaned_nodes,
                    final_node_sources: &self.final_nodes,
                    cleaned_cycle_origins: &self.cleaned_cycles,
                    final_cycle_origins: &self.final_cycles,
                },
            )
        }
    }

    fn ids(candidates: &[&FeatureStrength]) -> Vec<usize> {
        candidates.iter().map(|candidate| candidate.id).collect()
    }

    #[test]
    fn follows_both_node_transitions_and_sorts_original_ids() {
        let mut fixture = Fixture::new(graph(&[NodeKind::Ending; 4], &[(0, 1), (2, 3)], 0));
        fixture.cleaned = graph(&[NodeKind::Ending; 3], &[(0, 1)], 0);
        fixture.cleaned_nodes = vec![2, 0, 3];
        fixture.final_graph = graph(&[NodeKind::Ending; 2], &[(0, 1)], 0);
        fixture.final_nodes = vec![2, 0];
        let candidates: Vec<_> = (0..4)
            .rev()
            .map(|node| candidate(node, Origin::Node(node), FeatureKind::Ending, 1))
            .collect();

        let result = fixture.project(&candidates).unwrap();
        assert_eq!(ids(&result), vec![2, 3]);
        assert!(std::ptr::eq(result[0], &candidates[1]));
        assert!(result.iter().all(|feature| feature.priority.is_none()));
    }

    #[test]
    fn omits_changed_kind_and_degree_but_keeps_unchanged_fork() {
        let mut fixture = Fixture::new(graph(
            &[NodeKind::Junction; 3],
            &[(0, 1), (0, 1), (0, 2), (1, 2), (1, 2)],
            0,
        ));
        fixture.final_graph = graph(
            &[NodeKind::Ending, NodeKind::Junction, NodeKind::Junction],
            &[(0, 2), (1, 2), (1, 2)],
            0,
        );
        let candidates = vec![
            candidate(0, Origin::Node(0), FeatureKind::Bifurcation, 3),
            candidate(1, Origin::Node(1), FeatureKind::Junction, 4),
            candidate(2, Origin::Node(2), FeatureKind::Bifurcation, 3),
        ];
        assert_eq!(ids(&fixture.project(&candidates).unwrap()), vec![2]);
    }

    #[test]
    fn counts_self_edge_as_two_incidences() {
        let fixture = Fixture::new(graph(&[NodeKind::Junction], &[(0, 0)], 0));
        let candidates = vec![candidate(0, Origin::Node(0), FeatureKind::Junction, 2)];
        assert_eq!(ids(&fixture.project(&candidates).unwrap()), vec![0]);
    }

    #[test]
    fn deduplicates_split_node_source_identities() {
        let mut fixture = Fixture::new(graph(&[NodeKind::Ending; 2], &[(0, 1)], 0));
        fixture.cleaned = graph(&[NodeKind::Ending; 4], &[(0, 1), (2, 3)], 0);
        fixture.cleaned_nodes = vec![0, 1, 0, 1];
        fixture.final_graph = fixture.cleaned.clone();
        fixture.final_nodes = vec![0, 1, 2, 3];
        let candidates = vec![
            candidate(0, Origin::Node(0), FeatureKind::Ending, 1),
            candidate(1, Origin::Node(1), FeatureKind::Ending, 1),
        ];
        assert_eq!(ids(&fixture.project(&candidates).unwrap()), vec![0, 1]);
    }

    #[test]
    fn follows_both_cycle_transitions_without_scoring_new_cycles() {
        let mut fixture = Fixture::new(graph(&[], &[], 3));
        fixture.cleaned_cycles[0].source_cycles = vec![2];
        fixture.cleaned_cycles[1].source_cycles.clear();
        fixture.cleaned_cycles[2].source_cycles = vec![0];
        fixture.final_graph = graph(&[], &[], 4);
        fixture.final_cycles = vec![
            PathOrigin {
                source_cycles: vec![2],
                ..PathOrigin::default()
            },
            PathOrigin {
                source_cycles: vec![0],
                ..PathOrigin::default()
            },
            PathOrigin {
                source_cycles: vec![1],
                ..PathOrigin::default()
            },
            PathOrigin {
                source_cycles: vec![],
                ..PathOrigin::default()
            },
        ];
        let candidates: Vec<_> = (0..3)
            .rev()
            .map(|cycle| candidate(cycle + 10, Origin::Cycle(cycle), FeatureKind::Cycle, 0))
            .collect();
        assert_eq!(ids(&fixture.project(&candidates).unwrap()), vec![10, 12]);
    }

    #[test]
    fn deduplicates_repeated_cycle_identity_across_final_cycles() {
        let mut fixture = Fixture::new(graph(&[], &[], 1));
        fixture.final_graph = graph(&[], &[], 2);
        fixture.final_cycles.push(fixture.final_cycles[0].clone());
        let candidates = vec![candidate(0, Origin::Cycle(0), FeatureKind::Cycle, 0)];
        assert_eq!(ids(&fixture.project(&candidates).unwrap()), vec![0]);
    }

    #[test]
    fn omits_ambiguous_cycle_sources_from_either_transition() {
        let candidates = vec![candidate(0, Origin::Cycle(0), FeatureKind::Cycle, 0)];
        for (cleaned, final_sources) in [(vec![0, 0], vec![0]), (vec![0], vec![0, 0])] {
            let mut fixture = Fixture::new(graph(&[], &[], 1));
            fixture.cleaned_cycles[0].source_cycles = cleaned;
            fixture.final_cycles[0].source_cycles = final_sources;
            assert!(fixture.project(&candidates).unwrap().is_empty());
        }
    }

    #[test]
    fn rejects_invalid_node_provenance_without_panicking() {
        let candidates = vec![candidate(0, Origin::Node(0), FeatureKind::Isolated, 0)];
        for case in 0..5 {
            let mut fixture = Fixture::new(graph(&[NodeKind::Isolated], &[], 0));
            match case {
                0 => fixture.cleaned_nodes.clear(),
                1 => fixture.final_nodes.clear(),
                2 => fixture.cleaned_nodes[0] = 1,
                3 => fixture.final_nodes[0] = 1,
                _ => fixture.final_graph.edges.push(RidgeEdge {
                    start: 0,
                    end: 1,
                    points: vec![point(0), point(1)],
                }),
            }
            assert!(matches!(
                fixture.project(&candidates),
                Err(FingerprintError::InvalidTopology(_))
            ));
        }
    }

    #[test]
    fn rejects_invalid_cycle_provenance_without_panicking() {
        let candidates = vec![candidate(0, Origin::Cycle(0), FeatureKind::Cycle, 0)];
        for case in 0..6 {
            let mut fixture = Fixture::new(graph(&[], &[], 1));
            match case {
                0 => fixture.cleaned_cycles.clear(),
                1 => fixture.final_cycles.clear(),
                2 => fixture.cleaned_cycles[0].source_cycles = vec![1],
                3 => fixture.final_cycles[0].source_cycles = vec![1],
                // Projection may stop after two sources; validation must still
                // reject an invalid index later in either provenance list.
                4 => fixture.cleaned_cycles[0].source_cycles = vec![0, 0, 1],
                _ => fixture.final_cycles[0].source_cycles = vec![0, 0, 1],
            }
            assert!(matches!(
                fixture.project(&candidates),
                Err(FingerprintError::InvalidTopology(_))
            ));
        }
    }

    #[test]
    fn rejects_missing_or_ambiguous_candidate_identity() {
        let fixture = Fixture::new(graph(&[NodeKind::Isolated; 2], &[], 0));
        let first = candidate(4, Origin::Node(0), FeatureKind::Isolated, 0);
        let second = candidate(5, Origin::Node(1), FeatureKind::Isolated, 0);
        let mut repeated_id = second.clone();
        repeated_id.id = first.id;
        let mut repeated_source = second.clone();
        repeated_source.source_node = first.source_node;
        let mut absent_source = second.clone();
        absent_source.source_node = Some(2);
        for candidates in [
            vec![first.clone()],
            vec![first.clone(), repeated_id],
            vec![first.clone(), repeated_source],
            vec![first, absent_source],
        ] {
            assert!(matches!(
                fixture.project(&candidates),
                Err(FingerprintError::InvalidTopology(_))
            ));
        }
    }
}
