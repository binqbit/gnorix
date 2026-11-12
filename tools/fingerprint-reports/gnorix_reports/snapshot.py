"""Bounded native extraction snapshots and their validated report data."""

from __future__ import annotations

import json
import math
from dataclasses import dataclass
from pathlib import Path, PureWindowsPath

from .clarity import IMAGE_KEYS as CLARITY_IMAGES
from .clarity import Clarity, validate_clarity, validate_segment_clarity
from .cleanup import validate_cleanup
from .contours import Contours, validate_contours
from .feature_strength import FeatureStrength, validate_feature_strength
from .full_scale import IMAGE_KEYS as MEASUREMENT_IMAGES
from .full_scale import FullScale, validate_full_scale
from .images import MAX_FRAME_PIXELS, MAX_FRAME_SIDE
from .normalization import IMAGE_KEYS, Normalization, validate_normalization
from .scale import ScaleAnalysis, validate_scale
from .validation import array_value, finite_number, identifier_value, object_value

SCHEMA = "gnorix.ridge-extraction.v1"
MAX_SNAPSHOT_BYTES = 64 * 1024 * 1024
_MAX_PATH_POINTS = 1_000_000

PROCESSING_STAGES = (
    ("source", "Source · 8-bit grayscale"),
    ("normalized", "Normalized intensity · −3 to 3"),
    ("denoised", "Denoised intensity · −3 to 3"),
    ("coherence", "Orientation coherence · 0 to 1"),
    ("usable", "Usable region · white = usable"),
    ("binary", "Binary ridges · dark = ridge"),
    ("skeleton", "Skeleton · dark = ridge"),
)
SOURCE_IMAGE_KEYS = (
    *[key for key, _ in PROCESSING_STAGES],
    *IMAGE_KEYS,
    *MEASUREMENT_IMAGES,
    *CLARITY_IMAGES,
)
SOURCE_IMAGE_FILES = frozenset(key.replace("_", "-") + ".png" for key in SOURCE_IMAGE_KEYS)


@dataclass(frozen=True)
class Node:
    x: float
    y: float
    kind: str
    touches_unobserved: bool


@dataclass(frozen=True)
class Curve:
    points: tuple[tuple[float, float], ...]
    geometry_error: str | None
    arc_length_units: float | None
    segment_clarity: tuple[float | None, ...]


@dataclass(frozen=True)
class StructureSnapshot:
    width: int
    height: int
    bootstrap: ScaleAnalysis
    scale: FullScale
    normalization: Normalization
    contrast: float
    coherence: float
    nodes: tuple[Node, ...]
    edges: tuple[Curve, ...]
    cycles: tuple[Curve, ...]
    images: dict[str, Path]
    graph_cycle_rank: int
    contours: Contours
    clarity: Clarity
    feature_strength: FeatureStrength


def load_snapshot(path: Path) -> StructureSnapshot:
    data = _read_document(path)

    width = _dimension(data.get("width"), "width")
    height = _dimension(data.get("height"), "height")
    if width * height > MAX_FRAME_PIXELS:
        raise ValueError("snapshot exceeds the image pixel limit")
    config = object_value(data.get("config"), "config")
    contrast = finite_number(config.get("min_contrast"), "config.min_contrast")
    coherence = finite_number(config.get("min_coherence"), "config.min_coherence")
    if not 0 <= contrast <= 1 or not 0 <= coherence <= 1:
        raise ValueError("config thresholds must be in [0, 1]")
    bootstrap = validate_scale(
        data.get("bootstrap"), width, height, config.get("bootstrap_grid_locations")
    )
    scale = validate_full_scale(data.get("scale"), width, height)
    normalization = validate_normalization(data.get("normalization"), bootstrap.ridge_period_px)
    clarity = validate_clarity(data.get("clarity"), bootstrap.ridge_period_px, width, height)

    raw_nodes = array_value(data.get("nodes"), "nodes")
    node_ids = _identifiers(raw_nodes, "nodes")
    nodes = tuple(_node(node, width, height) for node in raw_nodes)
    edges = _curves(data.get("edges"), "edges", width, height, node_ids, scale.unit_px)
    cycles = _curves(data.get("cycles"), "cycles", width, height, node_ids, scale.unit_px)
    if sum(len(curve.points) for curve in (*edges, *cycles)) > _MAX_PATH_POINTS:
        raise ValueError("graph exceeds the total path point limit")
    feature_strength = validate_feature_strength(
        data.get("feature_strength"),
        raw_nodes,
        data["edges"],
        data["cycles"],
        width,
        height,
        scale.unit_px,
        contrast,
        coherence,
    )

    raw_images = object_value(data.get("images"), "images")
    base = path.resolve().parent
    images = {key: _image_path(base, raw_images.get(key), key) for key in SOURCE_IMAGE_KEYS}
    rank = _graph_cycle_rank(node_ids, data["edges"], len(cycles))
    cleanup = validate_cleanup(
        data.get("cleanup"), raw_nodes, data["edges"], data["cycles"], width, height, scale.unit_px
    )
    contours = validate_contours(
        data.get("contours"),
        cleanup.nodes,
        cleanup.edges,
        cleanup.cycles,
        width,
        height,
        scale.unit_px,
        cleanup_width=cleanup.width,
        observed_edges=data["edges"],
        observed_cycles=data["cycles"],
        removed_spurs=cleanup.removed_spurs,
        retained_ambiguous_edges=cleanup.retained_ambiguous_edges,
        retained_unobserved_edges=cleanup.retained_unobserved_edges,
        retained_unconfirmed_edges=cleanup.retained_unconfirmed_edges,
        loops=cleanup.loops,
    )
    return StructureSnapshot(
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
        contours,
        clarity,
        feature_strength,
    )


def _read_document(path):
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
    data = object_value(data, "snapshot")
    if data.get("schema") != SCHEMA:
        raise ValueError(f"schema must be {SCHEMA!r}")
    return data


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


def _dimension(value, name):
    if type(value) is not int or not 1 <= value <= MAX_FRAME_SIDE:
        raise ValueError(f"{name} must be an integer in [1, {MAX_FRAME_SIDE}]")
    return value


def _identifiers(items, name):
    seen = set()
    for item in items:
        item = object_value(item, name)
        item_id = identifier_value(item.get("id"), f"{name}.id")
        if item_id in seen:
            raise ValueError(f"{name} contains duplicate IDs")
        seen.add(item_id)
    return seen


def _position(value, name, width, height):
    if not isinstance(value, list) or len(value) != 2:
        raise ValueError(f"{name} must contain x and y")
    x, y = (finite_number(coordinate, name) for coordinate in value)
    if not 0 <= x <= width - 1 or not 0 <= y <= height - 1:
        raise ValueError(f"{name} lies outside source pixel centers")
    return x, y


def _node(data, width, height):
    x, y = _position([data.get("x"), data.get("y")], "node position", width, height)
    kind = data.get("kind")
    if not isinstance(kind, str) or kind not in ("ending", "junction", "isolated"):
        raise ValueError("node kind must be ending, junction, or isolated")
    boundary = data.get("touches_unobserved")
    if type(boundary) is not bool:
        raise ValueError("node touches_unobserved must be a boolean")
    return Node(x, y, kind, boundary)


def _curves(value, name, width, height, node_ids, unit):
    items = array_value(value, name)
    _identifiers(items, name)
    curves = []
    total = 0
    for item in items:
        if name == "edges":
            for end in ("start", "end"):
                item_id = identifier_value(item.get(end), f"edge.{end}")
                if item_id not in node_ids:
                    raise ValueError(f"edge.{end} references an unknown node")
        raw_points = array_value(item.get("points"), f"{name}.points")
        if len(raw_points) < (3 if name == "cycles" else 2):
            raise ValueError(f"{name} contains too few path points")
        total += len(raw_points)
        if total > _MAX_PATH_POINTS:
            raise ValueError("graph exceeds the total path point limit")
        points = tuple(_position(point, name, width, height) for point in raw_points)
        error = _geometry(item, width, height, unit)
        arc_units = item["geometry"]["arc_length_units"] if error is None else None
        scores = validate_segment_clarity(
            item.get("segment_clarity"), len(points), name == "cycles"
        )
        curves.append(Curve(points, error, arc_units, scores))
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
    geometry = object_value(geometry, "geometry")
    length = finite_number(geometry.get("arc_length"), "geometry.arc_length")
    chord = finite_number(geometry.get("chord_length"), "geometry.chord_length")
    if length <= 0 or chord < 0 or chord > length * (1 + 1e-12):
        raise ValueError("geometry requires a positive length and a chord no longer than the path")
    for key, pixels in (("arc_length_units", length), ("chord_length_units", chord)):
        units = finite_number(geometry.get(key), "geometry." + key)
        if not math.isclose(units, pixels / unit, rel_tol=1e-8, abs_tol=1e-7):
            raise ValueError(
                f"geometry.{key} must equal its source length divided by scale.unit_px"
            )
    _position(geometry.get("midpoint"), "geometry.midpoint", width, height)
    turning = array_value(geometry.get("turning"), "geometry.turning")
    previous = -1.0
    for item in turning:
        item = object_value(item, "turning sample")
        fraction = finite_number(item.get("arc_fraction"), "turning.arc_fraction")
        finite_number(item.get("angle_radians"), "turning.angle_radians")
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
