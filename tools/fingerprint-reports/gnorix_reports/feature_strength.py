"""Validate native feature evidence and its original-graph identity contract."""

from __future__ import annotations

import math
from collections import Counter
from dataclasses import dataclass

from .feature_strength_context import (
    FeatureContext,
    Neighborhood,
    unit_score,
    validate_context,
    validate_neighbor_positions,
    validate_neighborhood,
)
from .validation import array_value, finite_number, identifier_value, object_value, require_close


@dataclass(frozen=True)
class FeatureArm:
    edge: int
    length_px: float
    source_darkness: float | None
    dark_fraction: float | None
    source_contrast: float | None
    longest_light_gap_px: float | None


@dataclass(frozen=True)
class Candidate:
    id: int
    kind: str
    source_node: int | None
    source_cycle: int | None
    position: tuple[float, float]
    source_support: float | None
    geometry_support: float | None
    stability: float
    strength: float | None
    source_coverage: float
    geometry_coverage: float
    source_contrast: float | None
    arms: tuple[FeatureArm, ...]
    separation: float | None
    geometry_separation: float | None
    termination: float | None
    continuation: float | None
    reasons: tuple[str, ...]
    stress_matches: tuple[int | None, ...]
    context: FeatureContext
    neighborhood: Neighborhood
    priority: float | None


@dataclass(frozen=True)
class FeatureStrength:
    width: int
    height: int
    unit_px: float
    candidates: tuple[Candidate, ...]
    edge_points: dict[int, tuple[tuple[float, float], ...]]
    cycle_points: dict[int, tuple[tuple[float, float], ...]]


def _score(value, name, optional=False):
    return unit_score(value, "feature_strength." + name, optional=optional)


def _position(value, width, height):
    if not isinstance(value, list) or len(value) != 2:
        raise ValueError("feature position must contain source x and y")
    x, y = (finite_number(item, "feature position") for item in value)
    if not 0 <= x <= width - 1 or not 0 <= y <= height - 1:
        raise ValueError("feature position lies outside source pixel centers")
    return x, y


def _component(value, name):
    if name not in value:
        raise ValueError("feature candidate is missing " + name)
    return _score(value[name], name, optional=True)


def _validate_arms(value, expected, horizon):
    arms = []
    for item in array_value(value, "feature arms"):
        item = object_value(item, "feature arm")
        if item.keys() != FeatureArm.__dataclass_fields__.keys():
            raise ValueError("feature arm must declare exactly its native evidence fields")
        edge = identifier_value(item.get("edge"), "feature arm.edge")
        length = finite_number(item.get("length_px"), "feature arm.length_px")
        if not 0 <= length <= horizon * (1 + 1e-12):
            raise ValueError("feature arm length exceeds the native branch horizon")
        if "source_darkness" not in item or "dark_fraction" not in item:
            raise ValueError("feature arm must declare source darkness and dark fraction")
        darkness = _score(item["source_darkness"], "arm.source_darkness", optional=True)
        fraction = _score(item["dark_fraction"], "arm.dark_fraction", optional=True)
        contrast = _component(item, "source_contrast")
        if "longest_light_gap_px" not in item:
            raise ValueError("feature arm must declare its longest source light gap")
        gap = item["longest_light_gap_px"]
        if gap is not None:
            gap = finite_number(gap, "feature arm.longest_light_gap_px")
            if not 0 <= gap <= length * (1 + 1e-12):
                raise ValueError("feature arm light gap must lie within its represented length")
        if len({part is None for part in (darkness, fraction, contrast, gap)}) != 1:
            raise ValueError("feature arm source evidence must be jointly available")
        arms.append(FeatureArm(edge, length, darkness, fraction, contrast, gap))
    if Counter(arm.edge for arm in arms) != Counter(expected):
        raise ValueError("feature arms must cover all original node incidences")
    return tuple(arms)


def validate_feature_strength(
    value, nodes, edges, cycles, width, height, unit, min_contrast, min_coherence
):
    value = object_value(value, "feature_strength")
    if set(value) != {
        "method",
        "score",
        "priority_score",
        "policy",
        "stability_policy",
        "stress_cases",
        "candidates",
    }:
        raise ValueError("feature_strength must declare exactly its native assessment fields")
    for name, expected in (
        ("method", "scale-context-priority-v2"),
        ("score", "C * (0.75 + 0.25*min(P,G)) * (0.5 + 0.5*T)"),
        ("priority_score", "Q * N"),
    ):
        if value.get(name) != expected:
            raise ValueError(f"feature_strength.{name} must be {expected!r}")
    horizon, stress_counts = _validate_policy_and_stress(
        value, unit, min_contrast, min_coherence, width * height
    )
    raw = array_value(value.get("candidates"), "feature_strength.candidates", 200_000)
    if len(raw) != len(nodes) + len(cycles):
        raise ValueError("feature candidates must cover every raw node and isolated cycle")
    incidence = {node["id"]: [] for node in nodes}
    edge_endpoints = {edge["id"]: (edge["start"], edge["end"]) for edge in edges}
    for edge in edges:
        incidence[edge["start"]].append(edge["id"])
        incidence[edge["end"]].append(edge["id"])
    candidates = []
    case_matches = [set() for _ in range(8)]
    for identifier, item in enumerate(raw):
        item = object_value(item, "feature candidate")
        if item.keys() != Candidate.__dataclass_fields__.keys():
            raise ValueError("feature candidate must declare exactly its native evidence fields")
        if identifier_value(item.get("id"), "feature id") != identifier:
            raise ValueError("feature IDs must follow original nodes then isolated cycles")
        node = nodes[identifier] if identifier < len(nodes) else None
        cycle = cycles[identifier - len(nodes)] if node is None else None
        expected_node = node["id"] if node is not None else None
        expected_cycle = cycle["id"] if cycle is not None else None
        for name, expected in (("source_node", expected_node), ("source_cycle", expected_cycle)):
            if name not in item or item[name] != expected or isinstance(item[name], bool):
                raise ValueError("feature source reference must match its original candidate")
            if item[name] is not None:
                identifier_value(item[name], "feature " + name)
        expected_arms = incidence[node["id"]] if node is not None else []
        kind = item.get("kind")
        expected_kind = (
            "cycle"
            if node is None
            else "bifurcation"
            if node["kind"] == "junction" and len(expected_arms) == 3
            else node["kind"]
        )
        if kind != expected_kind:
            raise ValueError("feature kind must match the original node kind and degree")
        position = _position(item.get("position"), width, height)
        if node is not None:
            for actual, expected in zip(position, (node["x"], node["y"]), strict=True):
                require_close(actual, expected, "feature node position")
        else:
            for actual, expected in zip(position, _cycle_center(cycle["points"]), strict=True):
                require_close(actual, expected, "feature cycle position")
        parts = {
            name: _component(item, name)
            for name in (
                "source_support",
                "geometry_support",
                "strength",
                "separation",
                "geometry_separation",
                "termination",
                "continuation",
                "source_contrast",
                "priority",
            )
        }
        stability = _score(item.get("stability"), "stability")
        coverage = _score(item.get("source_coverage"), "source_coverage")
        geometry_coverage = _score(item.get("geometry_coverage"), "geometry_coverage")
        matches = _validate_matches(
            item.get("stress_matches"), stress_counts, case_matches, cycle is not None
        )
        require_close(
            stability, sum(match is not None for match in matches) / 8, "feature stability"
        )
        p, g, q = (parts[name] for name in ("source_support", "geometry_support", "strength"))
        context = validate_context(
            item.get("context"), expected_arms, edge_endpoints, expected_node, 4 * unit, kind=kind
        )
        neighborhood = validate_neighborhood(item.get("neighborhood"), len(raw))
        if coverage < 1 and p is not None:
            raise ValueError("partial source coverage requires unavailable source support")
        if p is None or g is None or context.support is None:
            if q is not None:
                raise ValueError("unknown source, geometry or context evidence requires unknown Q")
        elif q is None:
            raise ValueError("measured source, geometry and context evidence require measured Q")
        else:
            require_close(
                q,
                context.support * (0.75 + 0.25 * min(p, g)) * (0.5 + 0.5 * stability),
                "feature combined strength",
            )
        if q is None:
            if parts["priority"] is not None:
                raise ValueError("unknown Q requires unknown feature priority")
        elif parts["priority"] is None:
            raise ValueError("measured Q requires measured feature priority")
        else:
            require_close(parts["priority"], q * neighborhood.factor, "feature priority")
        reasons = _validate_reasons(item.get("reasons"))
        if ("stress_sensitive" in reasons) != (stability < 1):
            raise ValueError("feature stress-sensitive reason must match its stability")
        if coverage < 1 and "missing_source_support" not in reasons:
            raise ValueError("partial source coverage requires its missing-support reason")
        if ("missing_context_support" in reasons) != (context.support is None):
            raise ValueError("feature missing-context reason must match its context support")
        if ("unconfirmed_context" in reasons) != (context.support == 0):
            raise ValueError("feature unconfirmed-context reason must match measured zero context")
        arms = _validate_arms(item.get("arms"), expected_arms, horizon)
        candidates.append(
            Candidate(
                identifier,
                kind,
                expected_node,
                expected_cycle,
                position,
                p,
                g,
                stability,
                q,
                coverage,
                geometry_coverage,
                parts["source_contrast"],
                arms,
                parts["separation"],
                parts["geometry_separation"],
                parts["termination"],
                parts["continuation"],
                reasons,
                matches,
                context,
                neighborhood,
                parts["priority"],
            )
        )
    validate_neighbor_positions(candidates, 3 * unit)
    return FeatureStrength(
        width,
        height,
        unit,
        tuple(candidates),
        {edge["id"]: tuple(map(tuple, edge["points"])) for edge in edges},
        {cycle["id"]: tuple(map(tuple, cycle["points"])) for cycle in cycles},
    )


def _validate_matches(value, counts, case_matches, is_cycle):
    if not isinstance(value, list) or len(value) != 8:
        raise ValueError("feature stress_matches must contain all eight cases")
    result = []
    for index, match in enumerate(value):
        if match is not None:
            match = identifier_value(match, "feature stress match")
            node_count, cycle_count = counts[index]
            if match >= node_count + cycle_count:
                raise ValueError("feature stress match exceeds that case's candidate count")
            if (match >= node_count) != is_cycle:
                raise ValueError("feature stress match must preserve node-versus-cycle category")
            if match in case_matches[index]:
                raise ValueError("feature stress matches must be one-to-one within each case")
            case_matches[index].add(match)
        result.append(match)
    return tuple(result)


def _cycle_center(points):
    """Check the exported coordinate convention, without estimating a feature."""
    total = x = y = 0.0
    for a, b in zip(points, points[1:] + points[:1], strict=True):
        length = math.dist(a, b)
        total += length
        x += (a[0] + b[0]) * 0.5 * length
        y += (a[1] + b[1]) * 0.5 * length
    if total <= 0:
        raise ValueError("isolated cycle requires positive source path length")
    return x / total, y / total


def _validate_reasons(value):
    allowed = {
        "missing_source_support",
        "insufficient_arm_extent",
        "unresolved_direction",
        "no_separated_branches",
        "unconfirmed_termination",
        "possible_continuation",
        "isolated_geometry",
        "light_ridge",
        "stress_sensitive",
        "no_interior_light",
        "missing_context_support",
        "unconfirmed_context",
    }
    reasons = array_value(value, "feature reasons", len(allowed))
    if any(not isinstance(reason, str) or reason not in allowed for reason in reasons):
        raise ValueError("feature reasons must contain only known native reason codes")
    if len(set(reasons)) != len(reasons):
        raise ValueError("feature reasons cannot repeat")
    return tuple(reasons)


def _validate_policy_and_stress(value, unit, min_contrast, min_coherence, pixels):
    policy = object_value(value.get("policy"), "feature policy")
    expected_policy = {
        "reference_width_px": unit,
        "branch_horizon_px": 2 * unit,
        "context_horizon_px": 4 * unit,
        "neighborhood_radius_px": 3 * unit,
        "tangent_support_px": unit / 2,
        "sample_spacing_px": 0.5,
        "source_search_px": 1.2 * unit,
        "continuation_search_px": 3 * unit,
        "terminal_barrier_width_px": unit,
        "terminal_barrier_depth_px": unit / 4,
        "contrast_reference": min_contrast,
    }
    for name, expected in expected_policy.items():
        require_close(finite_number(policy.get(name), name), expected, "feature policy " + name)
    if policy.keys() != expected_policy.keys():
        raise ValueError("feature policy must declare exactly the native source controls")
    stability = object_value(value.get("stability_policy"), "feature stability policy")
    expected_stability = {
        "max_position_shift_px": unit,
        "arm_radius_px": unit,
        "max_arm_angle_radians": math.pi / 6,
        "max_arm_shape_error_px": unit / 4,
        "max_isolated_shift_px": unit / 4,
        "cycle_relative_tolerance": 0.2,
    }
    for name, expected in expected_stability.items():
        require_close(
            finite_number(stability.get(name), name), expected, "feature stability policy " + name
        )
    if stability.keys() != expected_stability.keys():
        raise ValueError("feature stability policy must declare exactly the native match controls")
    delta = max(0.02, 0.1 * min_coherence)
    low, high = max(0, min_coherence - delta), min(1, min_coherence + delta)
    if low == min_coherence:
        low = min(1, min_coherence + 2 * delta)
    if high == min_coherence:
        high = max(0, min_coherence - 2 * delta)
    cases = [
        (margin, coherence)
        for margin in (-0.05, 0, 0.05)
        for coherence in sorted((low, min_coherence, high))
        if not (margin == 0 and coherence == min_coherence)
    ]
    raw_cases = value.get("stress_cases")
    if not isinstance(raw_cases, list) or len(raw_cases) != 8:
        raise ValueError("feature assessment must declare exactly eight stress cases")
    counts = []
    for index, (item, expected) in enumerate(zip(raw_cases, cases, strict=True)):
        item = object_value(item, "feature stress case")
        if set(item) != {"directional_margin", "min_coherence", "node_count", "cycle_count"}:
            raise ValueError("feature stress case must declare parameters and variant counts")
        for name, number in zip(("directional_margin", "min_coherence"), expected, strict=True):
            require_close(
                finite_number(item.get(name), name), number, f"feature stress case {index} {name}"
            )
        node_count = identifier_value(item.get("node_count"), "stress node_count")
        cycle_count = identifier_value(item.get("cycle_count"), "stress cycle_count")
        if node_count + cycle_count > pixels:
            raise ValueError("stress candidate count exceeds source pixel count")
        counts.append((node_count, cycle_count))
    return policy["branch_horizon_px"], counts
