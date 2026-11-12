"""Validate and display source-supported geometric contour reconstruction."""

from __future__ import annotations

import math
import textwrap
from collections import Counter
from dataclasses import dataclass

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
from matplotlib.patches import Rectangle
from matplotlib.ticker import MaxNLocator

from .clarity import OPACITY_LABEL, UNKNOWN_COLOR, clarity_lines, validate_segment_clarity
from .cleanup import LoopAssessment, RemovedSpur
from .cleanup_evidence import CleanupWidth
from .graph_validation import (
    _array,
    _id,
    _number,
    _object,
    _points,
    _references,
    _same,
    _unique_items,
)

_MAX_POINTS = 1_000_000
_STATUSES = {"accepted", "image_unsupported", "unobserved", "ambiguous", "obstructed"}
_RAW_COLOR = "#a34981"
_SMOOTH_COLOR = "#00a6b2"
_JOIN_COLOR = "#009e73"
_CANDIDATE_COLOR = "#e69f00"
_REMOVED_COLOR = "#d55e00"


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
    data = _object(value, "contours")
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
    policy = _object(value, "contours.policy")
    for key, expected in (
        ("max_gap_px", unit),
        ("max_tangent_error_radians", math.pi / 6),
        ("min_mean_darkness", 0.5),
        ("min_dark_fraction", 0.7),
        ("min_mean_flank_contrast", 0.25),
        ("max_light_gap_px", unit / 4),
    ):
        _same(_number(policy.get(key), "policy." + key), expected, "policy." + key)


def _joins(value, nodes, width, height, unit):
    result, pairs, consumed = [], set(), set()
    for raw in _unique_items(value, "contours.joins"):
        start, end = (_id(raw.get(key), "join." + key) for key in ("start_node", "end_node"))
        if start == end or start not in nodes or end not in nodes:
            raise ValueError("join must connect two distinct observed nodes")
        if nodes[start]["kind"] != "ending" or nodes[end]["kind"] != "ending":
            raise ValueError("join endpoints must be observed ending nodes")
        pair = frozenset((start, end))
        if pair in pairs:
            raise ValueError("joins contain a duplicate endpoint pair")
        pairs.add(pair)
        points = _points(raw.get("points"), False, width, height)
        endpoints = tuple((nodes[index]["x"], nodes[index]["y"]) for index in (start, end))
        if points[0] != endpoints[0] or points[-1] != endpoints[1]:
            raise ValueError("join path endpoints must equal its observed ending positions")
        gap = _number(raw.get("gap_px"), "join.gap_px")
        if not 0 < gap <= unit * (1 + 1e-12):
            raise ValueError("join gap must be positive and at most one R")
        _same(gap, math.dist(*endpoints), "join.gap_px")
        angle = _number(raw.get("max_tangent_error_radians"), "join tangent error")
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
    value = _object(value, "join.evidence")
    darkness = _number(value.get("mean_darkness"), "join mean_darkness")
    fraction = _number(value.get("dark_fraction"), "join dark_fraction")
    contrast = _number(value.get("mean_flank_contrast"), "join mean_flank_contrast")
    light_gap = _number(value.get("longest_light_gap_px"), "join longest_light_gap_px")
    if not 0 <= fraction <= 1 or light_gap < 0:
        raise ValueError("join evidence has an invalid dark fraction or light-gap length")
    # Local-reference darkness and flank contrast need not lie in [0, 1].
    return darkness >= 0.5 and fraction >= 0.7 and contrast >= 0.25 and light_gap <= unit / 4


def _nodes(value, originals, removed):
    nodes, sources = [], set()
    for node in _unique_items(value, "contours.nodes"):
        source = _id(node.get("source_node"), "contour node source_node")
        if source not in originals or source in sources:
            raise ValueError("contour node must reference a unique observed node")
        sources.add(source)
        for key in ("x", "y"):
            _number(node.get(key), "contour node." + key)
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
    for raw in _unique_items(value, "contours.cycles" if closed else "contours.edges"):
        if not closed:
            for key in ("start", "end"):
                if _id(raw.get(key), "contour edge." + key) not in node_ids:
                    raise ValueError("contour edge references an unknown retained node")
        points = _points(raw.get("points"), closed, width, height)
        origins = tuple(
            _references(raw.get(key), key)
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
    data = _object(value, "contours.smoothing")
    controls = []
    for key, expected in (
        ("sigma_px", unit / 2),
        ("sample_spacing_px", min(unit / 8, 1)),
        ("max_displacement_px", unit / 4),
    ):
        actual = _number(data.get(key), "smoothing." + key)
        _same(actual, expected, "smoothing." + key)
        controls.append(actual)
    adjustments = []
    for kind, key, expected in (
        ("edges", "source_edge", edge_ids),
        ("cycles", "source_cycle", cycle_ids),
    ):
        seen = set()
        for item in _array(data.get(kind), "smoothing." + kind):
            item = _object(item, "smoothing adjustment")
            source = _id(item.get(key), "smoothing." + key)
            if source not in expected or source in seen:
                raise ValueError("smoothing adjustments must reference each raw path once")
            seen.add(source)
            displacement = _number(item.get("max_displacement_px"), "smoothing displacement")
            if not 0 <= displacement <= unit / 4 * (1 + 1e-12):
                raise ValueError("smoothing displacement exceeds the declared bound")
            applied = item.get("smoothing_applied")
            fraction = _number(item.get("applied_fraction"), "smoothing applied_fraction")
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


def render_contour_comparison(contours: Contours, source, alpha):
    """Show all raw/refined contours and a deterministic same-coordinate detail."""
    figure = plt.figure(figsize=(15, 16), layout="constrained")
    grid = figure.add_gridspec(3, 2, height_ratios=(1, 1, 0.29))
    axes = [[figure.add_subplot(grid[row, column]) for column in range(2)] for row in range(2)]
    try:
        crop, detail = _detail(contours)
        for row in range(2):
            for column in range(2):
                axis = axes[row][column]
                _image(axis, source, contours)
                if column == 0:
                    _draw_paths(axis, contours.raw_paths, _RAW_COLOR)
                else:
                    if row == 1:
                        _draw_paths(axis, contours.raw_paths, _RAW_COLOR, reference=True)
                    _draw_paths(axis, contours.paths, _SMOOTH_COLOR)
                    _draw_joins(axis, contours.joins, alpha)
                _draw_removed(axis, contours, alpha)
                if row == 0:
                    x, y, width, height = crop
                    axis.add_patch(
                        Rectangle((x, y), width, height, fill=False, color="#222222", linewidth=1)
                    )
                else:
                    axis.set(xlim=(crop[0], crop[0] + crop[2]), ylim=(crop[1] + crop[3], crop[1]))
        axes[0][0].set_title("Observed raster paths · before geometric refinement")
        axes[0][1].set_title("Cleaned + refined contours · removed artifacts marked")
        axes[1][0].set_title("Same source detail · observed paths")
        axes[1][1].set_title("Same source detail · retained contours + removed artifacts")
        figure.suptitle(_title(contours, "Geometric contour refinement"), fontsize=13)
        _footer(
            figure.add_subplot(grid[2, :]),
            detail + "\nBoth views use identical source coordinates and axis scales; "
            "reconstruction is an estimate, not verified anatomy.",
            width=140,
        )
        return figure
    except Exception:
        plt.close(figure)
        raise


def render_contour_overlay(contours: Contours, source, alpha):
    height = min(17, max(7, 9 * contours.height / contours.width))
    figure = plt.figure(figsize=(11, height + 1.5), layout="constrained")
    grid = figure.add_gridspec(2, 1, height_ratios=(height, 1.7))
    axis = figure.add_subplot(grid[0, 0])
    try:
        _image(axis, source, contours)
        _draw_paths(axis, contours.paths, _SMOOTH_COLOR)
        _draw_joins(axis, contours.joins, alpha)
        if not contours.paths:
            axis.text(0.5, 0.5, "No extracted contours", transform=axis.transAxes, ha="center")
        figure.suptitle(_title(contours, "Refined contours over the source image"), fontsize=12)
        _footer(
            figure.add_subplot(grid[1, 0]),
            f"Marker opacity: {alpha:g}. Amber candidates remain disconnected. "
            "Source pixels are unchanged.",
            width=110,
            raw=False,
        )
        return figure
    except Exception:
        plt.close(figure)
        raise


def _title(contours, title):
    accepted = sum(join.status == "accepted" for join in contours.joins)
    applied = sum(item.smoothing_applied for item in contours.adjustments)
    limited = sum(item.retained_reason is not None for item in contours.adjustments)
    maximum = max((item.max_displacement_px for item in contours.adjustments), default=0)
    relative = contours.cleanup_width.relative_tolerance
    lower = (1 - relative) * contours.unit_px
    upper = contours.cleanup_width.maximum_px
    return (
        f"{title}\n"
        f"Raw paths: {len(contours.raw_paths)} → cleaned: {len(contours.input_paths)} "
        f"→ refined contours: {len(contours.paths)} · "
        f"accepted joins: {accepted} · kept separate: {len(contours.joins) - accepted}\n"
        f"Removed: {len(contours.removed_spurs)} terminal spurs, "
        f"{sum(loop.removed_edge is not None for loop in contours.loops)} duplicate loop arms · "
        f"retained spurs: {len(contours.retained_ambiguous_edges)} ambiguous, "
        f"{len(contours.retained_unobserved_edges)} missing support, "
        f"{len(contours.retained_unconfirmed_edges)} unconfirmed\n"
        f"Smoothing applied: {applied} / {len(contours.adjustments)} paths · "
        f"limited by safeguards: {limited} · largest displacement: {maximum:.3g} px\n"
        f"Mean R = {contours.unit_px:.4g} px · ±{relative:.0%}: "
        f"{lower:.3g}–{upper:.3g} px · spur arc, loop width < 1.2R · "
        f"σ = {contours.sigma_px:.4g} px · "
        f"displacement bound = {contours.max_displacement_px:.4g} px"
    )


def _image(axis, source, contours):
    extent = (-0.5, contours.width - 0.5, contours.height - 0.5, -0.5)
    axis.imshow(
        source, cmap="gray", vmin=0, vmax=255, extent=extent, interpolation="nearest", alpha=1
    )
    axis.set(
        xlim=extent[:2],
        ylim=extent[2:],
        aspect="equal",
        xlabel="x (source px)",
        ylabel="y (source px)",
    )
    axis.xaxis.set_major_locator(MaxNLocator(4))
    axis.yaxis.set_major_locator(MaxNLocator(4))


def _draw_paths(axis, paths, color, *, reference=False):
    if paths:
        lines = clarity_lines(
            ((path.points, path.closed, path.segment_clarity) for path in paths),
            color,
            linewidths=0.9,
            linestyles="dotted" if reference else "solid",
            zorder=3,
        )
        if reference:
            # The faint original is only a comparison guide behind final paths.
            lines.set_alpha(lines.get_colors()[:, 3] * 0.55)
        axis.add_collection(lines)


def _draw_removed(axis, contours, alpha):
    # Removal is a native decision, independent of diagnostic clarity opacity.
    paths = [spur.points for spur in contours.removed_spurs]
    paths.extend(loop.removed_points for loop in contours.loops if loop.removed_edge is not None)
    if paths:
        from matplotlib.collections import LineCollection

        axis.add_collection(
            LineCollection(
                paths,
                colors=_REMOVED_COLOR,
                alpha=alpha,
                linewidths=1.6,
                linestyles="--",
                zorder=7,
            )
        )


def _draw_joins(axis, joins, alpha):
    for accepted in (False, True):
        selected = [join for join in joins if (join.status == "accepted") == accepted]
        if not selected:
            continue
        color = _JOIN_COLOR if accepted else _CANDIDATE_COLOR
        axis.add_collection(
            clarity_lines(
                ((join.points, False, join.segment_clarity) for join in selected),
                color,
                unknown_color=color,
                linewidths=2.2 if accepted else 1.5,
                linestyles="solid" if accepted else "dashed",
                zorder=5,
            )
        )
        endpoints = [point for join in selected for point in (join.points[0], join.points[-1])]
        axis.scatter(
            [point[0] for point in endpoints],
            [point[1] for point in endpoints],
            marker="o",
            s=13,
            facecolors="none",
            edgecolors=color,
            linewidths=0.8,
            alpha=alpha,
            zorder=6,
        )


def _detail(contours):
    accepted = [join for join in contours.joins if join.status == "accepted"]
    collapsed = [loop for loop in contours.loops if loop.removed_edge is not None]
    detail_bounds = None
    if collapsed:
        loop = max(collapsed, key=lambda item: (item.max_width_px, -item.removed_edge))
        points = (*loop.removed_points, *loop.retained_points)
        xs, ys = zip(*points, strict=True)
        detail_bounds = (min(xs), min(ys), max(xs), max(ys))
        center = ((min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2)
        label = (
            "Detail selected by widest source-confirmed collapsed loop: "
            f"edges {loop.source_edges}; "
            f"removed duplicate arm {loop.removed_edge}, width {loop.max_width_px:.3g} px "
            f"< 1.2R = {contours.cleanup_width.maximum_px:.3g} px. "
            "Vermillion is removed evidence."
        )
    elif contours.removed_spurs:
        spur = max(contours.removed_spurs, key=lambda item: (item.arc_length_px, -item.source_edge))
        center = spur.points[len(spur.points) // 2]
        label = (
            f"Detail selected by longest removed terminal spur: observed edge {spur.source_edge}, "
            f"arc length {spur.arc_length_px:.3g} px "
            f"< 1.2R = {contours.cleanup_width.maximum_px:.3g} px. "
            "Vermillion marks removed evidence, not a retained contour."
        )
    elif accepted:
        join = max(accepted, key=lambda item: (item.gap_px, -item.identifier))
        center = tuple(
            (start + end) / 2 for start, end in zip(join.points[0], join.points[-1], strict=True)
        )
        label = (
            f"Detail selected by longest accepted gap: join {join.identifier}, "
            f"{join.gap_px:.3g} px."
        )
    elif contours.adjustments:
        adjustment = max(contours.adjustments, key=lambda item: item.max_displacement_px)
        path = next(
            path
            for path in contours.input_paths
            if adjustment.source in (path.source_cycles if adjustment.closed else path.source_edges)
        )
        center = path.points[len(path.points) // 2]
        label = (
            "No removed artifacts or accepted joins. "
            "Detail uses the largest smoothing displacement: "
            f"{'cycle' if adjustment.closed else 'edge'} {adjustment.source}."
        )
    else:
        center = ((contours.width - 1) / 2, (contours.height - 1) / 2)
        label = "No paths available; showing the image center."
    span = max(8 * contours.unit_px, 24)
    width, height = min(span, contours.width), min(span, contours.height)
    if detail_bounds is not None:
        width = min(
            contours.width, max(span, detail_bounds[2] - detail_bounds[0] + 2 * contours.unit_px)
        )
        height = min(
            contours.height, max(span, detail_bounds[3] - detail_bounds[1] + 2 * contours.unit_px)
        )
    x = max(-0.5, min(center[0] - width / 2, contours.width - 0.5 - width))
    y = max(-0.5, min(center[1] - height / 2, contours.height - 0.5 - height))
    return (x, y, width, height), label


def _footer(axis, text, width, raw=True):
    axis.set_axis_off()
    handles = []
    if raw:
        handles.append(Line2D([], [], color=_RAW_COLOR, label="Observed raster path"))
        handles.append(
            Line2D(
                [],
                [],
                color=_REMOVED_COLOR,
                linestyle="--",
                label="Removed artifact: spur / loop arm",
            )
        )
    handles.extend(
        [
            Line2D([], [], color=_SMOOTH_COLOR, label="Refined contour"),
            Line2D([], [], color=_JOIN_COLOR, linewidth=2, label="Accepted source-supported join"),
            Line2D([], [], color=_CANDIDATE_COLOR, linestyle="--", label="Unaccepted candidate"),
            Line2D([], [], color=UNKNOWN_COLOR, alpha=0.4, label="Clarity unavailable"),
        ]
    )
    axis.legend(
        handles=handles, loc="upper center", ncol=2, frameon=False, fontsize=9, borderaxespad=0
    )
    text += (
        "\n" + OPACITY_LABEL + "\nJoins retain status colors; missing clarity uses minimum opacity."
    )
    wrapped = "\n".join(textwrap.fill(line, width) for line in text.splitlines())
    axis.text(0.5, 0, wrapped, transform=axis.transAxes, ha="center", va="bottom", fontsize=9)
