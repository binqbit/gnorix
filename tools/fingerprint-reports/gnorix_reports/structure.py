"""PNG diagnostics for supplied native ridge graphs and processing stages."""

from __future__ import annotations

import json
import math
import tempfile
import textwrap
from contextlib import ExitStack
from dataclasses import dataclass
from pathlib import Path, PureWindowsPath

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
from matplotlib.collections import LineCollection
from matplotlib.lines import Line2D
from matplotlib.ticker import MaxNLocator

from .full_scale import (
    IMAGE_KEYS as MEASUREMENT_IMAGES,
)
from .full_scale import (
    FullScale,
    render_full_scale,
    validate_full_scale,
    validate_measurement_maps,
)
from .images import load_image
from .model import MAX_FRAME_PIXELS, MAX_FRAME_SIDE, MAX_SNAPSHOT_BYTES
from .normalization import (
    IMAGE_KEYS,
    Normalization,
    render_normalization_analysis,
    validate_normalization,
    validate_palette,
)
from .scale import ScaleAnalysis, render_scale_analysis, validate_scale

_SCHEMA = "gnorix.ridge-extraction.v1"
_MAX_ITEMS = 100_000
_MAX_PATH_POINTS = 1_000_000
_STAGES = (
    ("source", "Source · 8-bit grayscale"),
    ("normalized", "Normalized intensity · −3 to 3"),
    ("denoised", "Denoised intensity · −3 to 3"),
    ("coherence", "Orientation coherence · 0 to 1"),
    ("usable", "Usable region · white = usable"),
    ("binary", "Binary ridges · dark = ridge"),
    ("skeleton", "Skeleton · dark = ridge"),
)
_KINDS = {
    "ending": ("o", "#0072b2", "Ending candidate"),
    "junction": ("^", "#d55e00", "Junction candidate"),
    "isolated": ("s", "#8a4c99", "Isolated node"),
}
_STYLE = {
    "font.family": "DejaVu Sans",
    "font.size": 10,
    "axes.titlesize": 11,
    "axes.labelsize": 9,
    "figure.facecolor": "white",
    "savefig.facecolor": "white",
    "text.usetex": False,
    "text.parse_math": False,
}


@dataclass(frozen=True)
class _Node:
    x: float
    y: float
    kind: str
    touches_unobserved: bool


@dataclass(frozen=True)
class _Curve:
    points: tuple[tuple[float, float], ...]
    geometry_error: str | None
    arc_length_units: float | None


@dataclass(frozen=True)
class _Structure:
    width: int
    height: int
    bootstrap: ScaleAnalysis
    scale: FullScale
    normalization: Normalization
    contrast: float
    coherence: float
    nodes: tuple[_Node, ...]
    edges: tuple[_Curve, ...]
    cycles: tuple[_Curve, ...]
    images: dict[str, Path]
    graph_cycle_rank: int


def render_structure(
    snapshot_path: Path, output: Path, dpi: int = 160, alpha: float = 0.5
) -> list[Path]:
    """Validate a native extraction bundle and export five PNG diagnostics.

    All rasters must have the declared source dimensions. Coordinates are pixel
    centers: (0, 0) is the center of the top-left pixel. Curves are displayed as
    supplied, including curves whose descriptor failed; no graph repair, shape
    fitting, matching tolerance, or confidence calculation is performed here.
    """
    if type(dpi) is not int or not 1 <= dpi <= 600:
        raise ValueError("report DPI must be an integer in [1, 600]")
    alpha = _number(alpha, "alpha")
    if not 0 <= alpha <= 1:
        raise ValueError("alpha must be in [0, 1]")
    snapshot = _load_structure(Path(snapshot_path))
    output = Path(output)

    with ExitStack() as stack:
        images = {}
        for key, path in snapshot.images.items():
            image = load_image(path, (snapshot.width, snapshot.height))
            stack.callback(image.close)
            images[key] = image
        palette = validate_palette(images)
        validate_measurement_maps(snapshot.scale, images)

        # Finish validation/decoding before creating output; finish all figures
        # in a sibling directory before replacing any existing report files.
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix=".ridge-report-", dir=output.parent) as temporary:
            staging = Path(temporary)
            with matplotlib.rc_context(_STYLE):
                _save_figure(_overview(snapshot, images), staging / "structure-overview.png", dpi)
                _save_figure(
                    _overlay(snapshot, images, alpha), staging / "structure-overlay.png", dpi
                )
                _save_figure(
                    render_full_scale(snapshot.scale, images, alpha),
                    staging / "scale-analysis.png",
                    dpi,
                )
                _save_figure(
                    render_scale_analysis(snapshot.bootstrap, images["source"], alpha),
                    staging / "bootstrap-analysis.png",
                    dpi,
                )
                _save_figure(
                    render_normalization_analysis(snapshot.normalization, palette, images),
                    staging / "normalization-analysis.png",
                    dpi,
                )
            output.mkdir(parents=True, exist_ok=True)
            paths = []
            for name in (
                "structure-overview.png",
                "structure-overlay.png",
                "scale-analysis.png",
                "bootstrap-analysis.png",
                "normalization-analysis.png",
            ):
                target = output / name
                (staging / name).replace(target)
                paths.append(target)
            return paths


def _load_structure(path: Path) -> _Structure:
    with path.open("rb") as stream:
        encoded = stream.read(MAX_SNAPSHOT_BYTES + 1)
    if len(encoded) > MAX_SNAPSHOT_BYTES:
        raise ValueError("snapshot exceeds the 64 MiB JSON limit")
    try:
        data = json.loads(
            encoded.decode("utf-8"),
            object_pairs_hook=_unique_fields,
            parse_constant=_reject_constant,
        )
    except (UnicodeError, json.JSONDecodeError, RecursionError) as error:
        raise ValueError("snapshot must contain valid UTF-8 JSON") from error
    data = _object(data, "snapshot")
    if data.get("schema") != _SCHEMA:
        raise ValueError(f"schema must be {_SCHEMA!r}")

    width = _dimension(data.get("width"), "width")
    height = _dimension(data.get("height"), "height")
    if width * height > MAX_FRAME_PIXELS:
        raise ValueError("snapshot exceeds the image pixel limit")
    config = _object(data.get("config"), "config")
    contrast = _number(config.get("min_contrast"), "config.min_contrast")
    coherence = _number(config.get("min_coherence"), "config.min_coherence")
    if not 0 <= contrast <= 1 or not 0 <= coherence <= 1:
        raise ValueError("config thresholds must be in [0, 1]")
    bootstrap = validate_scale(
        data.get("bootstrap"), width, height, config.get("bootstrap_grid_locations")
    )
    scale = validate_full_scale(data.get("scale"), width, height)
    normalization = validate_normalization(data.get("normalization"), bootstrap.ridge_period_px)

    raw_nodes = _array(data.get("nodes"), "nodes")
    node_ids = _identifiers(raw_nodes, "nodes")
    nodes = tuple(_node(node, width, height) for node in raw_nodes)
    edges = _curves(data.get("edges"), "edges", width, height, node_ids, scale.unit_px)
    cycles = _curves(data.get("cycles"), "cycles", width, height, node_ids, scale.unit_px)
    if sum(len(curve.points) for curve in (*edges, *cycles)) > _MAX_PATH_POINTS:
        raise ValueError("graph exceeds the total path point limit")

    raw_images = _object(data.get("images"), "images")
    base = path.resolve().parent
    keys = (*[key for key, _ in _STAGES], *IMAGE_KEYS, *MEASUREMENT_IMAGES)
    images = {key: _image_path(base, raw_images.get(key), key) for key in keys}
    rank = _graph_cycle_rank(node_ids, data["edges"], len(cycles))
    return _Structure(
        width,
        height,
        bootstrap,
        scale,
        normalization,
        contrast,
        coherence,
        nodes,
        edges,
        cycles,
        images,
        rank,
    )


def _graph_cycle_rank(node_ids, edges, isolated_cycles):
    """Count independent cycles in the validated undirected multigraph.

    E - V + C counts self-edges and parallel edges without interpreting their
    biological meaning. Disconnected isolated nodes each add both V and C.
    Node-free cycles stored separately each contribute one additional cycle.
    """
    parent = {node: node for node in node_ids}
    components = len(node_ids)

    def root(node):
        while parent[node] != node:
            parent[node] = parent[parent[node]]
            node = parent[node]
        return node

    for edge in edges:
        start, end = root(edge["start"]), root(edge["end"])
        if start != end:
            parent[start] = end
            components -= 1

    return len(edges) - len(node_ids) + components + isolated_cycles


def _unique_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field {key!r}")
        result[key] = value
    return result


def _reject_constant(value):
    raise ValueError(f"JSON number {value!r} must be finite")


def _object(value, name):
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _array(value, name, limit=_MAX_ITEMS):
    if not isinstance(value, list) or len(value) > limit:
        raise ValueError(f"{name} must be an array with at most {limit} items")
    return value


def _number(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a finite number")
    try:
        converted = float(value)
    except OverflowError as error:
        raise ValueError(f"{name} must be a finite number") from error
    if not math.isfinite(converted):
        raise ValueError(f"{name} must be a finite number")
    return converted


def _dimension(value, name):
    if type(value) is not int or not 1 <= value <= MAX_FRAME_SIDE:
        raise ValueError(f"{name} must be an integer in [1, {MAX_FRAME_SIDE}]")
    return value


def _identifier(value, name):
    if type(value) is not int or not 0 <= value < 2**64:
        raise ValueError(f"{name} must be a nonnegative 64-bit integer")
    return value


def _identifiers(items, name):
    seen = set()
    for item in items:
        item = _object(item, name)
        identifier = _identifier(item.get("id"), f"{name}.id")
        if identifier in seen:
            raise ValueError(f"{name} contains duplicate IDs")
        seen.add(identifier)
    return seen


def _position(value, name, width, height):
    if not isinstance(value, list) or len(value) != 2:
        raise ValueError(f"{name} must contain x and y")
    x, y = (_number(coordinate, name) for coordinate in value)
    if not 0 <= x <= width - 1 or not 0 <= y <= height - 1:
        raise ValueError(f"{name} lies outside source pixel centers")
    return x, y


def _node(data, width, height):
    x, y = _position([data.get("x"), data.get("y")], "node position", width, height)
    kind = data.get("kind")
    if not isinstance(kind, str) or kind not in _KINDS:
        raise ValueError("node kind must be ending, junction, or isolated")
    boundary = data.get("touches_unobserved")
    if type(boundary) is not bool:
        raise ValueError("node touches_unobserved must be a boolean")
    return _Node(x, y, kind, boundary)


def _curves(value, name, width, height, node_ids, unit):
    items = _array(value, name)
    _identifiers(items, name)
    curves = []
    total = 0
    for item in items:
        if name == "edges":
            for end in ("start", "end"):
                identifier = _identifier(item.get(end), f"edge.{end}")
                if identifier not in node_ids:
                    raise ValueError(f"edge.{end} references an unknown node")
        raw_points = _array(item.get("points"), f"{name}.points")
        if len(raw_points) < (3 if name == "cycles" else 2):
            raise ValueError(f"{name} contains too few path points")
        total += len(raw_points)
        if total > _MAX_PATH_POINTS:
            raise ValueError("graph exceeds the total path point limit")
        points = tuple(_position(point, name, width, height) for point in raw_points)
        error = _geometry(item, width, height, unit)
        arc_units = item["geometry"]["arc_length_units"] if error is None else None
        curves.append(_Curve(points, error, arc_units))
    return tuple(curves)


def _geometry(curve, width, height, unit):
    if "geometry" not in curve or "geometry_error" not in curve:
        raise ValueError("curve must declare geometry and geometry_error")
    geometry, error = curve["geometry"], curve["geometry_error"]
    if geometry is None:
        if not isinstance(error, str) or not error.strip() or len(error) > 512:
            raise ValueError("unavailable curve geometry requires a bounded error message")
        return error
    if error is not None:
        raise ValueError("available curve geometry cannot also have an error")
    geometry = _object(geometry, "geometry")
    length = _number(geometry.get("arc_length"), "geometry.arc_length")
    chord = _number(geometry.get("chord_length"), "geometry.chord_length")
    if length <= 0 or chord < 0 or chord > length * (1 + 1e-12):
        raise ValueError("geometry requires a positive length and a chord no longer than the path")
    for key, pixels in (("arc_length_units", length), ("chord_length_units", chord)):
        units = _number(geometry.get(key), "geometry." + key)
        if not math.isclose(units, pixels / unit, rel_tol=1e-8, abs_tol=1e-7):
            raise ValueError(
                f"geometry.{key} must equal its source length divided by scale.unit_px"
            )
    _position(geometry.get("midpoint"), "geometry.midpoint", width, height)
    turning = _array(geometry.get("turning"), "geometry.turning")
    previous = -1.0
    for item in turning:
        item = _object(item, "turning sample")
        fraction = _number(item.get("arc_fraction"), "turning.arc_fraction")
        _number(item.get("angle_radians"), "turning.angle_radians")
        if not previous < fraction <= 1:
            raise ValueError("turning fractions must strictly increase within [0, 1]")
        previous = fraction
    if len(turning) < 2 or turning[0]["arc_fraction"] != 0 or previous != 1:
        raise ValueError("turning profile must cover fractions zero through one")
    return None


def _image_path(base, value, name):
    if not isinstance(value, str) or not value or "\\" in value:
        raise ValueError(f"images.{name} must be a relative local image path")
    relative = Path(value)
    if relative.is_absolute() or PureWindowsPath(value).drive or ".." in relative.parts:
        raise ValueError(f"images.{name} must stay inside the snapshot directory")
    try:
        path = (base / relative).resolve(strict=True)
    except (OSError, RuntimeError, ValueError) as error:
        raise ValueError(f"images.{name} cannot be resolved") from error
    if not path.is_relative_to(base) or not path.is_file():
        raise ValueError(f"images.{name} must be a file inside the snapshot directory")
    return path


def _image_axes(axis, image, snapshot):
    extent = (-0.5, snapshot.width - 0.5, snapshot.height - 0.5, -0.5)
    axis.imshow(
        image, cmap="gray", vmin=0, vmax=255, extent=extent, alpha=1, interpolation="nearest"
    )
    axis.set(xlim=extent[:2], ylim=extent[2:], xlabel="x (source px)", ylabel="y (source px)")
    axis.set_aspect("equal")
    axis.xaxis.set_major_locator(MaxNLocator(4))
    axis.yaxis.set_major_locator(MaxNLocator(4))


def _overview(snapshot, images):
    height = min(16, max(6, 6.5 * snapshot.height / snapshot.width + 1.5))
    figure, axes = plt.subplots(2, 4, figsize=(16, height), layout="constrained")
    try:
        for axis, (key, label) in zip(axes.flat[:-1], _STAGES, strict=True):
            _image_axes(axis, images[key], snapshot)
            axis.set_title(label)
        summary = axes.flat[-1]
        summary.set_axis_off()
        summary.text(
            0.1,
            0.75,
            "Extracted graph topology\n\n"
            f"Nodes: {len(snapshot.nodes)}\n"
            f"Paths: {len(snapshot.edges)}\n"
            f"Graph cycle rank: {snapshot.graph_cycle_rank}\n"
            f"Isolated cycles: {len(snapshot.cycles)}\n\n"
            f"Described path length: {_described_length(snapshot):.6g} R\n\n"
            "Cycle rank counts independent graph cycles.\n"
            "Image coordinates use source pixels.\n"
            "Described path length is expressed in R.",
            transform=summary.transAxes,
            ha="left",
            va="top",
            fontsize=10,
            linespacing=1.6,
        )
        figure.suptitle(
            "Native ridge extraction · supplied processing stages\n"
            f"{snapshot.width} × {snapshot.height} source px · "
            f"full mean width R = {snapshot.scale.unit_px:.4g} px · "
            f"bootstrap period {snapshot.bootstrap.ridge_period_px:.4g} px · "
            f"contrast ≥ {snapshot.contrast:g} · "
            f"coherence ≥ {snapshot.coherence:g}",
            fontsize=14,
        )
        return figure
    except Exception:
        plt.close(figure)
        raise


def _described_length(snapshot):
    return math.fsum(
        curve.arc_length_units
        for curve in (*snapshot.edges, *snapshot.cycles)
        if curve.arc_length_units is not None
    )


def _overlay(snapshot, images, alpha):
    height = min(16, max(7, 9 * snapshot.height / snapshot.width))
    figure, axis = plt.subplots(figsize=(11, height), layout="constrained")
    try:
        _image_axes(axis, images["source"], snapshot)
        _draw_curves(axis, snapshot.edges, False, alpha)
        _draw_curves(axis, snapshot.cycles, True, alpha)
        _draw_nodes(axis, snapshot.nodes, alpha)
        _graph_legend(figure)
        counts = {kind: sum(node.kind == kind for node in snapshot.nodes) for kind in _KINDS}
        boundary = sum(node.touches_unobserved for node in snapshot.nodes)
        errors = [curve.geometry_error for curve in (*snapshot.edges, *snapshot.cycles)]
        errors = [error for error in errors if error is not None]
        figure.suptitle(
            "Extracted ridge graph · candidate structure\n"
            f"Full mean ridge width: 1 R = {snapshot.scale.unit_px:.4g} px · "
            f"bootstrap period: {snapshot.bootstrap.ridge_period_px:.4g} px\n"
            f"Graph cycle rank: {snapshot.graph_cycle_rank} · "
            f"isolated cycles: {len(snapshot.cycles)}\n"
            f"Paths: {len(snapshot.edges)} · "
            f"endings: {counts['ending']} · junctions: {counts['junction']} · "
            f"isolated nodes: {counts['isolated']}\n"
            f"Nodes touching unobserved regions: {boundary} · "
            f"curves without geometry: {len(errors)} · overlay opacity: {alpha:g}",
            fontsize=12,
        )
        if not snapshot.nodes and not snapshot.edges and not snapshot.cycles:
            axis.text(
                0.5,
                0.5,
                "No extracted graph structure",
                transform=axis.transAxes,
                ha="center",
                va="center",
                bbox={"facecolor": "white", "alpha": 0.9},
            )
        if errors:
            message = "; ".join(sorted(set(errors)))
            axis.set_title(textwrap.fill("Geometry unavailable: " + message[:350], 100), fontsize=9)
        return figure
    except Exception:
        plt.close(figure)
        raise


def _draw_curves(axis, curves, closed, alpha):
    for failed in (False, True):
        paths = []
        for curve in curves:
            if (curve.geometry_error is not None) == failed:
                points = curve.points
                if closed and points[-1] != points[0]:
                    points = (*points, points[0])
                paths.append(points)
        if paths:
            color = "#e69f00" if failed else ("#a44cad" if closed else "#00a6b2")
            axis.add_collection(
                LineCollection(
                    paths,
                    colors=color,
                    linewidths=0.9,
                    alpha=alpha,
                    linestyles="dashed" if closed else "solid",
                    zorder=3,
                )
            )


def _draw_nodes(axis, nodes, alpha):
    for kind, (marker, color, _) in _KINDS.items():
        for boundary in (False, True):
            selected = [
                node for node in nodes if node.kind == kind and node.touches_unobserved == boundary
            ]
            if not selected:
                continue
            x, y = [node.x for node in selected], [node.y for node in selected]
            axis.scatter(
                x,
                y,
                marker=marker,
                s=30,
                facecolors="none" if boundary else color,
                edgecolors=color,
                linewidths=0.9,
                alpha=alpha,
                zorder=4,
            )
            if boundary:
                axis.scatter(
                    x, y, marker="x", s=13, color="#242424", linewidths=0.7, alpha=alpha, zorder=5
                )


def _graph_legend(figure):
    handles = [
        Line2D([], [], color="#00a6b2", label="Observed path"),
        Line2D([], [], color="#a44cad", linestyle="--", label="Isolated closed cycle"),
        Line2D([], [], color="#e69f00", label="Geometry unavailable"),
    ]
    handles.extend(
        Line2D([], [], marker=marker, color=color, linestyle="none", label=label)
        for marker, color, label in _KINDS.values()
    )
    handles.append(
        Line2D(
            [],
            [],
            marker="o",
            markerfacecolor="none",
            color="#242424",
            linestyle="none",
            label="Hollow + ×: touches unobserved region",
        )
    )
    figure.legend(handles=handles, loc="outside lower center", ncol=3, frameon=False, fontsize=9)


def _save_figure(figure, path, dpi):
    try:
        figure.savefig(path, dpi=dpi, bbox_inches="tight", pad_inches=0.15)
    finally:
        plt.close(figure)
