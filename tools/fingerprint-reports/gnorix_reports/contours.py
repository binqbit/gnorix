"""Validate source-supported contours and their reconstruction provenance."""

from __future__ import annotations

import math
from collections import Counter
from dataclasses import dataclass

from .clarity import validate_segment_clarity
from .cleanup import LoopAssessment, RemovedSpur
from .cleanup_evidence import CleanupWidth
from .graph_validation import path_points, reference_ids, unique_items
from .validation import array_value, finite_number, identifier_value, object_value, require_close

_MAX_POINTS = 1_000_000
_STATUSES = {"accepted", "image_unsupported", "unobserved", "ambiguous", "obstructed"}


@dataclass(frozen=True)
class ContourPath:
    points: tuple[tuple[float, float], ...]
    closed: bool
    segment_clarity: tuple[float | None, ...]
    source_edges: tuple[int, ...]
    source_cycles: tuple[int, ...]
    source_joins: tuple[int, ...]


@dataclass(frozen=True)
class GapJoin:
    identifier: int
    start_node: int
    end_node: int
    points: tuple[tuple[float, float], ...]
    status: str
    gap_px: float
    segment_clarity: tuple[float | None, ...]


@dataclass(frozen=True)
class SmoothingAdjustment:
    source: int
    closed: bool
    max_displacement_px: float
    smoothing_applied: bool
    applied_fraction: float
    retained_reason: str | None


@dataclass(frozen=True)
class Contours:
    width: int
    height: int
    unit_px: float
    cleanup_width: CleanupWidth
    nodes: tuple[dict, ...]
    paths: tuple[ContourPath, ...]
    raw_paths: tuple[ContourPath, ...]
    input_paths: tuple[ContourPath, ...]
    removed_spurs: tuple[RemovedSpur, ...]
    retained_ambiguous_edges: tuple[int, ...]
    retained_unobserved_edges: tuple[int, ...]
    retained_unconfirmed_edges: tuple[int, ...]
    loops: tuple[LoopAssessment, ...]
    joins: tuple[GapJoin, ...]
    adjustments: tuple[SmoothingAdjustment, ...]
    sigma_px: float
    sample_spacing_px: float
    max_displacement_px: float


def validate_contours(
    value,
    raw_nodes,
    raw_edges,
    raw_cycles,
    width,
    height,
    unit,
    *,
    cleanup_width,
    observed_edges=None,
    observed_cycles=None,
    removed_spurs=(),
    retained_ambiguous_edges=(),
    retained_unobserved_edges=(),
    retained_unconfirmed_edges=(),
    loops=(),
) -> Contours:
    """Require complete provenance against the already validated cleaned input graph.

    Geometry and photometric scores describe the supplied native reconstruction.
    Validation does not reinterpret those scores as biometric confidence, nor
    reproduce its source classifier from lossily rounded palette PNGs.
    """
    data = object_value(value, "contours")
    if data.get("method") != "source-supported-smooth-contours":
        raise ValueError("contours.method must be source-supported-smooth-contours")
    _policy(data.get("policy"), unit)
    originals = {node["id"]: node for node in raw_nodes}
    edge_ids = {edge["id"] for edge in raw_edges}
    cycle_ids = {cycle["id"] for cycle in raw_cycles}
    joins = _joins(data.get("joins"), originals, width, height, unit)
    accepted = {join.identifier for join in joins if join.status == "accepted"}
    removed_nodes = {
        node
        for join in joins
        if join.status == "accepted"
        for node in (join.start_node, join.end_node)
    }
    nodes = _nodes(data.get("nodes"), originals, removed_nodes)
    node_ids = {node["id"] for node in nodes}
    edges = _paths(data.get("edges"), False, node_ids, width, height)
    cycles = _paths(data.get("cycles"), True, node_ids, width, height)
    paths = (*edges, *cycles)
    for name, expected in (
        ("source_edges", edge_ids),
        ("source_cycles", cycle_ids),
        ("source_joins", accepted),
    ):
        references = Counter(reference for path in paths for reference in getattr(path, name))
        if references != Counter({identifier: 1 for identifier in expected}):
            raise ValueError(f"contour {name} must retain each corresponding input exactly once")
    if sum(len(path.points) for path in paths) > _MAX_POINTS:
        raise ValueError("contours exceed the total path point limit")
    _connections(data, raw_edges, nodes, joins)
    adjustments, sigma, spacing, displacement = _smoothing(
        data.get("smoothing"), edge_ids, cycle_ids, unit
    )
    input_paths = _input_paths(raw_edges, raw_cycles)
    raw_paths = _input_paths(
        raw_edges if observed_edges is None else observed_edges,
        raw_cycles if observed_cycles is None else observed_cycles,
    )
    return Contours(
        width,
        height,
        unit,
        cleanup_width,
        nodes,
        paths,
        raw_paths,
        input_paths,
        removed_spurs,
        retained_ambiguous_edges,
        retained_unobserved_edges,
        retained_unconfirmed_edges,
        loops,
        joins,
        adjustments,
        sigma,
        spacing,
        displacement,
    )


def _input_paths(edges, cycles):
    return tuple(
        ContourPath(
            tuple(tuple(point) for point in item["points"]),
            closed,
            validate_segment_clarity(item.get("segment_clarity"), len(item["points"]), closed),
            () if closed else (item["id"],),
            (item["id"],) if closed else (),
            (),
        )
        for closed, items in ((False, edges), (True, cycles))
        for item in items
    )


def _policy(value, unit):
    policy = object_value(value, "contours.policy")
    for key, expected in (
        ("max_gap_px", unit),
        ("max_tangent_error_radians", math.pi / 6),
        ("min_mean_darkness", 0.5),
        ("min_dark_fraction", 0.7),
        ("min_mean_flank_contrast", 0.25),
        ("max_light_gap_px", unit / 4),
    ):
        require_close(finite_number(policy.get(key), "policy." + key), expected, "policy." + key)


def _joins(value, nodes, width, height, unit):
    result, pairs, consumed = [], set(), set()
    for raw in unique_items(value, "contours.joins"):
        start, end = (
            identifier_value(raw.get(key), "join." + key) for key in ("start_node", "end_node")
        )
        if start == end or start not in nodes or end not in nodes:
            raise ValueError("join must connect two distinct observed nodes")
        if nodes[start]["kind"] != "ending" or nodes[end]["kind"] != "ending":
            raise ValueError("join endpoints must be observed ending nodes")
        pair = frozenset((start, end))
        if pair in pairs:
            raise ValueError("joins contain a duplicate endpoint pair")
        pairs.add(pair)
        points = path_points(raw.get("points"), False, width, height)
        endpoints = tuple((nodes[index]["x"], nodes[index]["y"]) for index in (start, end))
        if points[0] != endpoints[0] or points[-1] != endpoints[1]:
            raise ValueError("join path endpoints must equal its observed ending positions")
        gap = finite_number(raw.get("gap_px"), "join.gap_px")
        if not 0 < gap <= unit * (1 + 1e-12):
            raise ValueError("join gap must be positive and at most one R")
        require_close(gap, math.dist(*endpoints), "join.gap_px")
        angle = finite_number(raw.get("max_tangent_error_radians"), "join tangent error")
        if not 0 <= angle <= math.pi / 6 * (1 + 1e-12):
            raise ValueError("join tangent error exceeds the declared policy")
        status = raw.get("status")
        if not isinstance(status, str) or status not in _STATUSES:
            raise ValueError("unknown contour join status")
        evidence = _evidence(raw.get("evidence"), unit)
        if status == "accepted":
            if pair & consumed:
                raise ValueError("accepted joins cannot reuse an ending")
            if not evidence:
                raise ValueError("accepted join requires sufficient source evidence")
            consumed.update(pair)
        scores = validate_segment_clarity(raw.get("segment_clarity"), len(points), False)
        result.append(GapJoin(raw["id"], start, end, points, status, gap, scores))
    if sum(len(join.points) for join in result) > _MAX_POINTS:
        raise ValueError("joins exceed the total path point limit")
    return tuple(result)


def _evidence(value, unit):
    if value is None:
        return False
    value = object_value(value, "join.evidence")
    darkness = finite_number(value.get("mean_darkness"), "join mean_darkness")
    fraction = finite_number(value.get("dark_fraction"), "join dark_fraction")
    contrast = finite_number(value.get("mean_flank_contrast"), "join mean_flank_contrast")
    light_gap = finite_number(value.get("longest_light_gap_px"), "join longest_light_gap_px")
    if not 0 <= fraction <= 1 or light_gap < 0:
        raise ValueError("join evidence has an invalid dark fraction or light-gap length")
    # Local-reference darkness and flank contrast need not lie in [0, 1].
    return darkness >= 0.5 and fraction >= 0.7 and contrast >= 0.25 and light_gap <= unit / 4


def _nodes(value, originals, removed):
    nodes, sources = [], set()
    for node in unique_items(value, "contours.nodes"):
        source = identifier_value(node.get("source_node"), "contour node source_node")
        if source not in originals or source in sources:
            raise ValueError("contour node must reference a unique observed node")
        sources.add(source)
        for key in ("x", "y"):
            finite_number(node.get(key), "contour node." + key)
        for key in ("x", "y", "kind", "touches_unobserved"):
            if type(node.get(key)) is not type(originals[source][key]) and key not in ("x", "y"):
                raise ValueError("retained contour node metadata must match its observed node")
            if node.get(key) != originals[source][key]:
                raise ValueError("retained contour node must match its observed node")
        nodes.append(node)
    if sources != set(originals) - removed:
        raise ValueError("contour nodes must retain exactly the unconsumed observed nodes")
    return tuple(nodes)


def _paths(value, closed, node_ids, width, height):
    paths = []
    for raw in unique_items(value, "contours.cycles" if closed else "contours.edges"):
        if not closed:
            for key in ("start", "end"):
                if identifier_value(raw.get(key), "contour edge." + key) not in node_ids:
                    raise ValueError("contour edge references an unknown retained node")
        points = path_points(raw.get("points"), closed, width, height)
        origins = tuple(
            reference_ids(raw.get(key), key)
            for key in ("source_edges", "source_cycles", "source_joins")
        )
        if not origins[0] and not origins[1]:
            raise ValueError("contour path requires an observed edge or cycle origin")
        if origins[1] and (not closed or origins[0] or origins[2] or len(origins[1]) != 1):
            raise ValueError("an observed isolated cycle must remain a separate closed contour")
        scores = validate_segment_clarity(raw.get("segment_clarity"), len(points), closed)
        paths.append(ContourPath(points, closed, scores, *origins))
    return tuple(paths)


def _connections(data, raw_edges, nodes, joins):
    """Origin IDs must actually form the retained path's connected chain."""
    edges = {edge["id"]: edge for edge in raw_edges}
    retained = {node["id"]: node["source_node"] for node in nodes}
    bridges = {join.identifier: join for join in joins}
    for closed, paths in ((False, data["edges"]), (True, data["cycles"])):
        for path in paths:
            if path["source_cycles"]:
                continue
            source_edges = [edges[identifier] for identifier in path["source_edges"]]
            links = [(edge["start"], edge["end"]) for edge in source_edges]
            links.extend(
                (bridges[identifier].start_node, bridges[identifier].end_node)
                for identifier in path["source_joins"]
            )
            degree = Counter(node for link in links for node in link)
            adjacency = {node: set() for node in degree}
            for start, end in links:
                adjacency[start].add(end)
                adjacency[end].add(start)
            ends = () if closed else (retained[path["start"]], retained[path["end"]])
            if ends and any(end not in degree for end in ends):
                raise ValueError("contour endpoints must belong to their source path")
            if any(
                count != (1 if len(set(ends)) == 2 and node in ends else 2)
                for node, count in degree.items()
            ):
                raise ValueError("contour origins do not form the claimed open or closed chain")
            pending, reached = [next(iter(degree))], set()
            while pending:
                node = pending.pop()
                if node not in reached:
                    reached.add(node)
                    pending.extend(adjacency[node] - reached)
            if reached != set(degree):
                raise ValueError("contour origins contain disconnected paths")
            for endpoint, point in zip(ends, (path["points"][0], path["points"][-1])):
                ports = [
                    edge["points"][index]
                    for edge in source_edges
                    for key, index in (("start", 0), ("end", -1))
                    if edge[key] == endpoint
                ]
                if point not in ports:
                    raise ValueError("contour must preserve its observed endpoint port")


def _smoothing(value, edge_ids, cycle_ids, unit):
    data = object_value(value, "contours.smoothing")
    controls = []
    for key, expected in (
        ("sigma_px", unit / 2),
        ("sample_spacing_px", min(unit / 8, 1)),
        ("max_displacement_px", unit / 4),
    ):
        actual = finite_number(data.get(key), "smoothing." + key)
        require_close(actual, expected, "smoothing." + key)
        controls.append(actual)
    adjustments = []
    for kind, key, expected in (
        ("edges", "source_edge", edge_ids),
        ("cycles", "source_cycle", cycle_ids),
    ):
        seen = set()
        for item in array_value(data.get(kind), "smoothing." + kind):
            item = object_value(item, "smoothing adjustment")
            source = identifier_value(item.get(key), "smoothing." + key)
            if source not in expected or source in seen:
                raise ValueError("smoothing adjustments must reference each raw path once")
            seen.add(source)
            displacement = finite_number(item.get("max_displacement_px"), "smoothing displacement")
            if not 0 <= displacement <= unit / 4 * (1 + 1e-12):
                raise ValueError("smoothing displacement exceeds the declared bound")
            applied = item.get("smoothing_applied")
            fraction = finite_number(item.get("applied_fraction"), "smoothing applied_fraction")
            reason = item.get("retained_reason")
            if type(applied) is not bool or not 0 <= fraction <= 1:
                raise ValueError("invalid smoothing application state")
            if reason is not None and reason not in ("support", "intersection", "degenerate"):
                raise ValueError("unknown smoothing retention reason")
            if not applied and (displacement != 0 or fraction != 0):
                raise ValueError("unapplied smoothing cannot report displacement or blend")
            adjustments.append(
                SmoothingAdjustment(
                    source, kind == "cycles", displacement, applied, fraction, reason
                )
            )
        if seen != expected:
            raise ValueError("smoothing adjustments must cover every raw path")
    return tuple(adjustments), *controls
