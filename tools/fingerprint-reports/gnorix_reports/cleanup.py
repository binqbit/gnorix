"""Validate source-supported ridge cleanup and exact retained raster provenance."""

import math
from collections import Counter, defaultdict
from dataclasses import dataclass

from .clarity import validate_segment_clarity
from .cleanup_evidence import (
    CleanupWidth,
    validate_loop_evidence,
    validate_source_policy,
    validate_spur_evidence,
)
from .graph_validation import path_points, reference_ids, unique_items
from .validation import array_value, finite_number, identifier_value, object_value, require_close


@dataclass(frozen=True)
class RemovedSpur:
    source_edge: int
    attachment_node: int
    terminal_node: int
    arc_length_px: float
    points: tuple[tuple[float, float], ...]


@dataclass(frozen=True)
class LoopAssessment:
    source_edges: tuple[int, int]
    source_nodes: tuple[int, int]
    decision: str
    removed_edge: int | None
    max_width_px: float | None
    removed_points: tuple[tuple[float, float], ...]
    retained_points: tuple[tuple[float, float], ...]


@dataclass(frozen=True)
class Cleanup:
    width: CleanupWidth
    nodes: tuple[dict, ...]
    edges: tuple[dict, ...]
    cycles: tuple[dict, ...]
    removed_spurs: tuple[RemovedSpur, ...]
    retained_ambiguous_edges: tuple[int, ...]
    retained_unobserved_edges: tuple[int, ...]
    retained_unconfirmed_edges: tuple[int, ...]
    loops: tuple[LoopAssessment, ...]


def validate_cleanup(value, raw_nodes, raw_edges, raw_cycles, width, height, unit):
    """Check the declared transformation without running its pruning classifier."""
    data = object_value(value, "cleanup")
    if data.get("method") != "source-supported-ridge-cleanup":
        raise ValueError("cleanup.method must be source-supported-ridge-cleanup")
    policy = object_value(data.get("policy"), "cleanup.policy")
    require_close(
        finite_number(policy.get("cursor_diameter_px"), "cleanup threshold"),
        unit,
        "cleanup threshold",
    )
    if type(policy.get("pass_count")) is not int or policy["pass_count"] != 1:
        raise ValueError("cleanup requires exactly one pass")
    cleanup_width = validate_source_policy(policy.get("source"), unit)
    maximum = cleanup_width.maximum_px
    originals = {node["id"]: node for node in raw_nodes}
    edges = {edge["id"]: edge for edge in raw_edges}
    cycles = {cycle["id"]: cycle for cycle in raw_cycles}
    clusters = _source_clusters(data.get("source_node_pixels"), originals, width, height)
    assessments = _spur_assessments(data.get("spur_assessments"), originals, edges, maximum)
    removed = _removals(data.get("removed_spurs"), originals, edges, maximum, assessments)
    loops = _loops(data.get("loops"), originals, edges, maximum, unit / 3)
    removed_ids = {spur.source_edge for spur in removed}
    ambiguous = _retained_candidates(
        data.get("retained_ambiguous_edges"), "ambiguous", originals, edges, maximum
    )
    unobserved = _retained_candidates(
        data.get("retained_unobserved_edges"), "unobserved", originals, edges, maximum
    )
    unconfirmed = _retained_candidates(
        data.get("retained_unconfirmed_edges"), "unconfirmed", originals, edges, maximum
    )
    _spur_partition(assessments, removed_ids, ambiguous, unobserved, unconfirmed)
    removed_ids.update(loop.removed_edge for loop in loops if loop.removed_edge is not None)
    nodes = _nodes(data.get("nodes"), originals, clusters, removed, width, height)
    observed_pixels = set().union(*clusters.values()) if clusters else set()
    observed_pixels.update(
        tuple(point) for path in (*raw_edges, *raw_cycles) for point in path["points"]
    )
    routes = _routes(
        data.get("junction_routes"), originals, clusters, observed_pixels, width, height
    )
    cleaned_edges = _paths(data.get("edges"), False, nodes, width, height)
    cleaned_cycles = _paths(data.get("cycles"), True, nodes, width, height)
    paths = (*cleaned_edges, *cleaned_cycles)
    if sum(len(path["points"]) for path in paths) > 1_000_000:
        raise ValueError("cleanup exceeds the total path point limit")
    for key, expected in (
        ("source_edges", edges.keys() - removed_ids),
        ("source_cycles", cycles.keys()),
        ("source_routes", routes.keys()),
    ):
        references = Counter(item for path in paths for item in path[key])
        if references != Counter({identifier: 1 for identifier in expected}):
            raise ValueError(f"cleanup {key} must retain each input exactly once")
    _path_evidence(cleaned_edges, cleaned_cycles, edges, cycles, routes)
    _topology(
        raw_nodes,
        raw_edges,
        raw_cycles,
        nodes,
        cleaned_edges,
        cleaned_cycles,
        sum(loop.removed_edge is not None for loop in loops),
    )
    return Cleanup(
        cleanup_width,
        tuple(nodes),
        tuple(cleaned_edges),
        tuple(cleaned_cycles),
        removed,
        ambiguous,
        unobserved,
        unconfirmed,
        loops,
    )


def _pixels(value, width, height):
    pixels = array_value(value, "node pixels")
    result = set()
    for point in pixels:
        if not isinstance(point, list) or len(point) != 2:
            raise ValueError("node pixel must contain x and y")
        x, y = (finite_number(v, "node pixel") for v in point)
        if not (x.is_integer() and y.is_integer() and 0 <= x < width and 0 <= y < height):
            raise ValueError("node pixels must be source raster coordinates")
        result.add((x, y))
    if not result or len(result) != len(pixels):
        raise ValueError("node pixels must be nonempty and unique")
    return result


def _source_clusters(value, nodes, width, height):
    clusters, used = {}, set()
    for item in array_value(value, "cleanup.source_node_pixels"):
        item = object_value(item, "source node pixels")
        source = identifier_value(item.get("source_node"), "source_node")
        if source not in nodes or source in clusters:
            raise ValueError("source node pixels must reference every observed node once")
        pixels = _pixels(item.get("pixels"), width, height)
        if used & pixels or (nodes[source]["x"], nodes[source]["y"]) not in pixels:
            raise ValueError("source node clusters must be disjoint and contain representatives")
        clusters[source] = pixels
        used.update(pixels)
    if clusters.keys() != nodes.keys():
        raise ValueError("source node pixels must cover every observed node")
    if len(used) > 1_000_000:
        raise ValueError("source node pixels exceed the total point limit")
    return clusters


def _spur_assessments(value, nodes, edges, maximum):
    expected = set()
    for identifier, edge in edges.items():
        if {nodes[edge[key]]["kind"] for key in ("start", "end")} == {"junction", "ending"}:
            points = edge["points"]
            if sum(math.dist(a, b) for a, b in zip(points, points[1:])) < maximum:
                expected.add(identifier)
    result = {}
    for item in array_value(value, "cleanup.spur_assessments"):
        item = object_value(item, "spur assessment")
        source = identifier_value(item.get("source_edge"), "spur assessment source_edge")
        if source in result or source not in expected:
            raise ValueError("spur assessments must cover each original short terminal once")
        result[source] = validate_spur_evidence(item.get("evidence"), maximum)
    if result.keys() != expected:
        raise ValueError("spur assessments must cover every original short terminal")
    return result


def _spur_partition(assessments, removed, ambiguous, unobserved, unconfirmed):
    groups = (removed, ambiguous, unobserved, unconfirmed)
    classified = Counter(identifier for group in groups for identifier in group)
    if classified != Counter({identifier: 1 for identifier in assessments}):
        raise ValueError("cleanup terminal classifications must form a disjoint complete partition")
    for group, decisions in (
        ((*removed, *ambiguous), {"confirmed"}),
        (unobserved, {"missing_support"}),
        (
            unconfirmed,
            {"possible_continuation", "separate_ridges", "ambiguous_geometry", "image_unsupported"},
        ),
    ):
        if any(assessments[identifier]["decision"] not in decisions for identifier in group):
            raise ValueError("terminal classification contradicts native source evidence")


def _loops(value, nodes, edges, maximum, minimum_valley_length):
    incident = defaultdict(list)
    parallel = defaultdict(set)
    for identifier, edge in edges.items():
        incident[edge["start"]].append(identifier)
        incident[edge["end"]].append(identifier)
        parallel[frozenset((edge["start"], edge["end"]))].add(identifier)
    expected = {
        pair
        for pair, arms in parallel.items()
        if len(pair) == 2
        and len(arms) == 2
        and all(nodes[node]["kind"] == "junction" and len(incident[node]) == 3 for node in pair)
    }
    result, consumed, seen_pairs = [], set(), set()
    for item in array_value(value, "cleanup.loops"):
        item = object_value(item, "loop assessment")
        source_edges = reference_ids(item.get("source_edges"), "loop source_edges")
        source_nodes = reference_ids(item.get("source_nodes"), "loop source_nodes")
        if len(source_edges) != 2 or len(source_nodes) != 2:
            raise ValueError("loop assessment requires two original edges and two nodes")
        if any(
            node not in nodes or nodes[node]["kind"] != "junction" or len(incident[node]) != 3
            for node in source_nodes
        ):
            raise ValueError("loop endpoints must be original degree-three junctions")
        if set(source_edges) != parallel[frozenset(source_nodes)]:
            raise ValueError("loop must be exactly two parallel original edges")
        if consumed & set(source_nodes):
            raise ValueError("loop assessments cannot reuse original junctions")
        consumed.update(source_nodes)
        seen_pairs.add(frozenset(source_nodes))
        evidence = validate_loop_evidence(item.get("evidence"), maximum, minimum_valley_length)
        removed = item.get("removed_edge")
        if removed is not None:
            removed = identifier_value(removed, "loop removed_edge")
            if (
                evidence["decision"] != "confirmed"
                or removed != source_edges[1 - evidence["preferred_arm"]]
            ):
                raise ValueError(
                    "loop removal must retain the native source-confirmed preferred arm"
                )
        elif evidence["decision"] == "confirmed":
            raise ValueError("confirmed loop assessment must identify its removed duplicate arm")
        points = (
            () if removed is None else tuple(tuple(point) for point in edges[removed]["points"])
        )
        retained_points = (
            ()
            if removed is None
            else tuple(
                tuple(point) for point in edges[source_edges[evidence["preferred_arm"]]]["points"]
            )
        )
        result.append(
            LoopAssessment(
                source_edges,
                source_nodes,
                evidence["decision"],
                removed,
                evidence["max_width_px"],
                points,
                retained_points,
            )
        )
    if seen_pairs != expected:
        raise ValueError("loop assessments must cover every original split/rejoin pair")
    return tuple(result)


def _removals(value, nodes, edges, maximum, assessments):
    removed, seen = [], set()
    incident = defaultdict(set)
    for key, edge in edges.items():
        incident[edge["start"]].add(key)
        incident[edge["end"]].add(key)
    for item in array_value(value, "cleanup.removed_spurs"):
        item = object_value(item, "removed spur")
        edge_id = identifier_value(item.get("source_edge"), "removed source_edge")
        attachment = identifier_value(item.get("attachment_node"), "attachment_node")
        terminal = identifier_value(item.get("terminal_node"), "terminal_node")
        if edge_id not in edges or edge_id in seen:
            raise ValueError("removed spur must reference a unique observed edge")
        edge = edges[edge_id]
        if attachment == terminal or {attachment, terminal} != {edge["start"], edge["end"]}:
            raise ValueError("removed spur endpoints must match its observed edge")
        if nodes[attachment]["kind"] != "junction" or nodes[terminal]["kind"] != "ending":
            raise ValueError("cleanup may remove only original junction-to-ending branches")
        if edge_id not in assessments or assessments[edge_id]["decision"] != "confirmed":
            raise ValueError("removed terminal branch requires confirmed source evidence")
        length = finite_number(item.get("arc_length_px"), "removed arc length")
        points = tuple(tuple(point) for point in edge["points"])
        observed_length = sum(math.dist(a, b) for a, b in zip(points, points[1:]))
        require_close(length, observed_length, "removed arc length")
        if not (0 < length < maximum and 0 < observed_length < maximum):
            raise ValueError(
                "removed terminal arc length must be strictly below the upper tolerance bound"
            )
        removed.append(RemovedSpur(edge_id, attachment, terminal, length, points))
        seen.add(edge_id)
    # An entire component of short branches cannot establish which branch is the ridge.
    for spur in removed:
        if incident[spur.attachment_node] <= seen:
            raise ValueError("cleanup must retain ambiguous all-short junctions")
    return tuple(removed)


def _retained_candidates(value, reason, nodes, edges, maximum):
    references = reference_ids(value, "retained " + reason + " edges")
    for identifier in references:
        if identifier not in edges:
            raise ValueError("retained cleanup candidate must reference an observed edge")
        edge = edges[identifier]
        kinds = {nodes[edge[key]]["kind"] for key in ("start", "end")}
        if kinds != {"junction", "ending"}:
            raise ValueError("retained cleanup candidate must be an original terminal branch")
        points = edge["points"]
        length = sum(math.dist(a, b) for a, b in zip(points, points[1:]))
        if not 0 < length < maximum:
            raise ValueError("retained cleanup candidate must be below the upper tolerance bound")
    return references


def _nodes(value, originals, clusters, removed, width, height):
    result, used = [], set()
    by_source = defaultdict(list)
    deleted = {spur.terminal_node for spur in removed}
    for node in unique_items(value, "cleanup.nodes"):
        source = identifier_value(node.get("source_node"), "cleaned source_node")
        if source not in originals or source in deleted:
            raise ValueError("cleaned node must retain an undeleted observed node")
        pixels = _pixels(node.get("pixels"), width, height)
        if not pixels <= clusters[source] or pixels & used:
            raise ValueError("cleaned node pixels must be disjoint subsets of their source cluster")
        position = (finite_number(node.get("x"), "node.x"), finite_number(node.get("y"), "node.y"))
        if position not in pixels:
            raise ValueError("cleaned node representative must be an observed member pixel")
        if node.get("kind") not in {"ending", "junction", "isolated"}:
            raise ValueError("invalid cleaned node kind")
        if type(node.get("touches_unobserved")) is not bool:
            raise ValueError("invalid cleaned node boundary flag")
        if node["touches_unobserved"] and not originals[source]["touches_unobserved"]:
            raise ValueError("cleanup cannot invent unobserved provenance")
        used.update(pixels)
        result.append(node)
        by_source[source].append(node)
    for source, original in originals.items():
        if original["kind"] in {"ending", "isolated"} and source not in deleted:
            retained = by_source[source]
            if len(retained) != 1 or any(
                retained[0][key] != original[key]
                for key in ("x", "y", "kind", "touches_unobserved")
            ):
                raise ValueError("cleanup must preserve unaffected endings and isolated nodes")
    return result


def _routes(value, originals, clusters, observed_pixels, width, height):
    routes, used_links = {}, set()
    for item in unique_items(value, "cleanup.junction_routes"):
        source = identifier_value(item.get("source_node"), "route source_node")
        if source not in originals or originals[source]["kind"] != "junction":
            raise ValueError("junction route must belong to an observed junction")
        points = path_points(item.get("points"), False, width, height)
        if any(point not in clusters[source] for point in points):
            raise ValueError("junction route must use actual source cluster pixels")
        if any(not 0 < math.dist(a, b) <= math.sqrt(2) for a, b in zip(points, points[1:])):
            raise ValueError("junction route must follow adjacent observed pixels")
        links = _links(points)
        if set(links) & used_links or any(count != 1 for count in links.values()):
            raise ValueError("junction routes must retain each observed link once")
        for a, b in zip(points, points[1:]):
            if a[0] != b[0] and a[1] != b[1]:
                if (a[0], b[1]) in observed_pixels or (b[0], a[1]) in observed_pixels:
                    raise ValueError("junction route invents an originally suppressed diagonal")
            elif a[1] == b[1]:
                if (a[0], a[1] + 1) in observed_pixels and (b[0], b[1] + 1) in observed_pixels:
                    raise ValueError("junction route invents a suppressed filled-cell top link")
        used_links.update(links)
        routes[item["id"]] = item
    return routes


def _paths(value, closed, nodes, width, height):
    result = []
    pixels = {node["id"]: {tuple(point) for point in node["pixels"]} for node in nodes}
    for path in unique_items(value, "cleanup paths"):
        points = path_points(path.get("points"), closed, width, height)
        validate_segment_clarity(path.get("segment_clarity"), len(points), closed)
        if not closed:
            start, end = (
                identifier_value(path.get(key), "cleanup endpoint") for key in ("start", "end")
            )
            if start not in pixels or end not in pixels:
                raise ValueError("cleanup edge references unknown node")
            if points[0] not in pixels[start] or points[-1] not in pixels[end]:
                raise ValueError("cleanup edge ports must belong to their retained node pixels")
        origins = [
            reference_ids(path.get(key), key)
            for key in ("source_edges", "source_cycles", "source_routes")
        ]
        if not any(origins):
            raise ValueError("cleanup path must have observed provenance")
        if origins[1] and (not closed or origins[0] or origins[2] or len(origins[1]) != 1):
            raise ValueError("cleanup must preserve isolated cycles separately")
        result.append(path)
    return result


def _links(points, closed=False):
    points = [tuple(point) for point in points]
    pairs = list(zip(points, points[1:]))
    if closed:
        pairs.append((points[-1], points[0]))
    return Counter(tuple(sorted((a, b))) for a, b in pairs if a != b)


def _path_evidence(cleaned_edges, cleaned_cycles, edges, cycles, routes):
    for closed, paths in ((False, cleaned_edges), (True, cleaned_cycles)):
        for path in paths:
            expected = Counter()
            for key, sources, source_closed in (
                ("source_edges", edges, False),
                ("source_cycles", cycles, True),
                ("source_routes", routes, False),
            ):
                for source in path[key]:
                    expected.update(_links(sources[source]["points"], source_closed))
            if _links(path["points"], closed) != expected:
                raise ValueError(
                    "cleanup geometry must retain the exact source edges and junction routes"
                )


def _topology(raw_nodes, raw_edges, raw_cycles, nodes, edges, cycles, collapsed_loops):
    degree = Counter(endpoint for edge in edges for endpoint in (edge["start"], edge["end"]))
    for node in nodes:
        if node["kind"] == "ending" and degree[node["id"]] != 1:
            raise ValueError("cleaned ending must have exactly one incident edge")
        if node["kind"] == "junction" and degree[node["id"]] < 3:
            raise ValueError("cleaned junction must have at least three incident edges")
        if node["kind"] == "isolated" and degree[node["id"]] != 0:
            raise ValueError("cleaned isolated node cannot have incident edges")
    components, rank = _invariants(raw_nodes, raw_edges, raw_cycles)
    if (components, rank - collapsed_loops) != _invariants(nodes, edges, cycles):
        raise ValueError("cleanup must preserve components and remove exactly its confirmed cycles")


def _invariants(nodes, edges, cycles):
    adjacency = {node["id"]: set() for node in nodes}
    for edge in edges:
        adjacency[edge["start"]].add(edge["end"])
        adjacency[edge["end"]].add(edge["start"])
    unseen, components = set(adjacency), 0
    while unseen:
        pending = [unseen.pop()]
        components += 1
        while pending:
            neighbors = adjacency[pending.pop()] & unseen
            unseen.difference_update(neighbors)
            pending.extend(neighbors)
    return components + len(cycles), len(edges) - len(nodes) + components + len(cycles)
