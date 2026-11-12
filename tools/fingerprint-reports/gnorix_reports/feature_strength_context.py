"""Validate supplied scale-context and neighborhood evidence without estimating it."""

import math
from collections import Counter, defaultdict
from dataclasses import dataclass

from .validation import array_value, finite_number, identifier_value, object_value, require_close


@dataclass(frozen=True)
class ContextArm:
    edge: int
    edges: tuple[int, ...]
    length_px: float
    ridge_separation: float | None
    continuity: float | None
    supported_length_px: float | None
    longest_unsupported_px: float | None
    stop: str


@dataclass(frozen=True)
class FeatureContext:
    support: float | None
    support_2r: float | None
    source_coverage: float
    separation: float | None
    termination: float | None
    continuation: float | None
    forward_continuation: float | None
    curved_continuation: float | None
    arms: tuple[ContextArm, ...]


@dataclass(frozen=True)
class Neighborhood:
    neighbor_count: int
    weighted_count: float
    observed_fraction: float
    factor: float


def unit_score(value, name, *, optional=False):
    if value is None and optional:
        return None
    result = finite_number(value, name)
    if not 0 <= result <= 1:
        raise ValueError(f"{name} must be in [0, 1]")
    return result


def _exact_object(value, model, name):
    result = object_value(value, name)
    if result.keys() != model.__dataclass_fields__.keys():
        raise ValueError(f"{name} must declare exactly its native evidence fields")
    return result


def _optional_length(value, maximum, name):
    if value is None:
        return None
    result = finite_number(value, name)
    if not 0 <= result <= maximum * (1 + 1e-12):
        raise ValueError(f"{name} exceeds the represented context length")
    return result


def _context_arm(value, edges, start_node, horizon):
    value = _exact_object(value, ContextArm, "feature context arm")
    edge = identifier_value(value["edge"], "context arm.edge")
    traversed = tuple(
        identifier_value(item, "context arm.edges")
        for item in array_value(value["edges"], "context arm.edges")
    )
    if not traversed or traversed[0] != edge or any(item not in edges for item in traversed):
        raise ValueError("context arm path must begin with its incidence and use original edges")
    if len(set(traversed)) != len(traversed):
        raise ValueError("context arm cannot traverse an original edge twice")
    _validate_edge_chain(traversed, edges, start_node)
    length = finite_number(value["length_px"], "context arm.length_px")
    if not 0 <= length <= horizon * (1 + 1e-12):
        raise ValueError("context arm length exceeds its declared horizon")
    separation = unit_score(value["ridge_separation"], "context arm separation", optional=True)
    continuity = unit_score(value["continuity"], "context arm continuity", optional=True)
    supported = _optional_length(value["supported_length_px"], length, "context supported length")
    unsupported = _optional_length(
        value["longest_unsupported_px"], length, "context longest unsupported length"
    )
    if len({part is None for part in (separation, continuity, supported, unsupported)}) != 1:
        raise ValueError("context arm source evidence must be jointly available")
    stop = value["stop"]
    if not isinstance(stop, str) or stop not in {
        "horizon",
        "graph_ending",
        "ambiguous_junction",
        "unsupported_continuation",
        "missing_source",
        "closed_path",
    }:
        raise ValueError("context arm must declare a known native stop reason")
    if stop == "horizon":
        require_close(length, horizon, "context arm horizon stop length")
    return ContextArm(edge, traversed, length, separation, continuity, supported, unsupported, stop)


def _validate_edge_chain(traversed, edges, start_node):
    node = start_node
    for identifier in traversed:
        first, second = edges[identifier]
        if node == first:
            node = second
        elif node == second:
            node = first
        else:
            raise ValueError("context arm edges must form a chain from the original candidate")


def validate_context(value, expected_arms, edges, start_node, horizon, *, kind):
    value = _exact_object(value, FeatureContext, "feature context")
    coverage = unit_score(value["source_coverage"], "feature context source coverage")
    support, local = (
        unit_score(value[name], "feature context " + name, optional=True)
        for name in ("support", "support_2r")
    )
    if coverage < 1 and (support is not None or local is not None):
        raise ValueError("partial context source coverage requires unavailable context support")
    arms = tuple(
        _context_arm(item, edges, start_node, horizon)
        for item in array_value(value["arms"], "feature context arms")
    )
    if Counter(arm.edge for arm in arms) != Counter(expected_arms):
        raise ValueError("context arms must cover all original node incidences")
    if any(arm.stop == "missing_source" for arm in arms) and (
        support is not None or local is not None
    ):
        raise ValueError("missing context continuation requires unavailable context support")
    termination, continuation, forward, curved = _ending_evidence(value, kind, support, local)
    return FeatureContext(
        support,
        local,
        coverage,
        unit_score(value["separation"], "feature context separation", optional=True),
        termination,
        continuation,
        forward,
        curved,
        arms,
    )


def _ending_evidence(value, kind, support, local):
    parts = tuple(
        unit_score(value[name], "feature context " + name, optional=True)
        for name in ("termination", "continuation", "forward_continuation", "curved_continuation")
    )
    termination, continuation, forward, curved = parts
    if kind != "ending":
        if any(part is not None for part in parts):
            raise ValueError("only an ending may declare termination or continuation evidence")
        return parts

    if forward is None or curved is None:
        if continuation is not None:
            raise ValueError(
                "missing continuation component requires unavailable combined evidence"
            )
    else:
        if continuation is None:
            raise ValueError("measured continuation components require their combined evidence")
        require_close(continuation, max(forward, curved), "combined context continuation")

    if any(score is not None for score in (support, local)):
        if termination is None or continuation is None:
            raise ValueError("missing ending evidence requires unavailable context support")
        limit = min(termination, 1 - continuation)
        if any(score is not None and score > limit + 1e-12 for score in (support, local)):
            raise ValueError("context support cannot override termination or continuation evidence")
    return parts


def validate_neighborhood(value, candidate_count):
    value = _exact_object(value, Neighborhood, "feature neighborhood")
    count = identifier_value(value["neighbor_count"], "feature neighbor_count")
    if count >= candidate_count:
        raise ValueError("feature neighborhood must exclude the candidate itself")
    weighted = finite_number(value["weighted_count"], "feature weighted_count")
    if not 0 <= weighted <= count:
        raise ValueError("feature weighted neighbor count must lie within its raw count")
    observed = unit_score(value["observed_fraction"], "feature neighborhood observed fraction")
    factor = unit_score(value["factor"], "feature neighborhood factor")
    require_close(factor, 0.5 + 0.5 * observed / (1 + weighted), "feature neighborhood factor")
    return Neighborhood(count, weighted, observed, factor)


def validate_neighbor_positions(candidates, radius):
    """Check exported counts against raw candidate positions, without source sampling."""
    if not math.isfinite(radius) or radius <= 0:
        raise ValueError("feature neighborhood radius must be finite and positive")
    cells = defaultdict(list)
    for candidate in candidates:
        cells[_cell(candidate.position, radius)].append(candidate)
    work = 0
    for candidate in candidates:
        weights = []
        x, y = _cell(candidate.position, radius)
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                nearby = cells.get((x + dx, y + dy), ())
                work += len(nearby)
                if work > 200_000_000:
                    raise ValueError("feature neighborhood validation exceeds native work limit")
                for other in nearby:
                    if other.id == candidate.id:
                        continue
                    distance = math.dist(candidate.position, other.position)
                    if distance < radius:
                        weights.append((1 - distance / radius) ** 2)
        if candidate.neighborhood.neighbor_count != len(weights):
            raise ValueError("feature neighbor count does not match original candidate positions")
        require_close(
            candidate.neighborhood.weighted_count,
            math.fsum(weights),
            "feature weighted neighbor count",
        )


def _cell(position, radius):
    return tuple(math.floor(value / radius) for value in position)
