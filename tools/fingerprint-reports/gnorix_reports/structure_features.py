"""Project original evidence onto surviving, validated final graph features."""

from collections import Counter
from dataclasses import dataclass


@dataclass(frozen=True)
class FeatureMarker:
    kind: str
    position: tuple[float, float]
    priority: float | None
    touches_unobserved: bool
    unassessed: bool


@dataclass(frozen=True)
class FinalStructure:
    markers: tuple[FeatureMarker, ...]
    cycle_rank: int
    unassessed_cycles: int


def project_final_structure(strength, cleanup, contours, final_edges, cycle_rank):
    """Retain original scores only when feature kind and incidence degree survive.

    Contour origins identify cleaned objects; cleanup origins identify original
    objects. Following both steps is essential even when native IDs happen to
    coincide. This projection never assesses or modifies biometric evidence.
    """
    nodes = {node["id"]: node for node in cleanup.nodes}
    originals = {
        item.source_node: item for item in strength.candidates if item.source_node is not None
    }
    degree = Counter(endpoint for edge in final_edges for endpoint in (edge["start"], edge["end"]))
    markers = []
    for node in contours.nodes:
        original = originals[nodes[node["source_node"]]["source_node"]]
        count = degree[node["id"]]
        kind = "bifurcation" if node["kind"] == "junction" and count == 3 else node["kind"]
        unchanged = kind == original.kind and count == len(original.arms)
        markers.append(
            FeatureMarker(
                kind,
                (node["x"], node["y"]),
                original.priority if unchanged else None,
                node["touches_unobserved"],
                not unchanged,
            )
        )

    cycles, unassessed = _cycle_markers(strength, cleanup, contours)
    return FinalStructure(tuple(markers) + cycles, cycle_rank, unassessed)


def _cycle_markers(strength, cleanup, contours):
    cleaned = {cycle["id"]: cycle for cycle in cleanup.cycles}
    originals = {
        candidate.source_cycle: candidate
        for candidate in strength.candidates
        if candidate.source_cycle is not None
    }
    markers, unassessed = [], 0
    for path in contours.paths:
        if not path.closed:
            continue
        origins = [
            source for cycle in path.source_cycles for source in cleaned[cycle]["source_cycles"]
        ]
        if len(origins) != 1:
            # A newly closed chain has no assessed original isolated-cycle
            # candidate. Its path remains visible without inventing a score.
            unassessed += 1
            continue
        original = originals[origins[0]]
        markers.append(FeatureMarker("cycle", original.position, original.priority, False, False))
    return tuple(markers), unassessed
