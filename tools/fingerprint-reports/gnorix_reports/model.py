"""Validated, algorithm-independent diagnostic snapshots for local reports."""

from __future__ import annotations

import json
import math
from dataclasses import dataclass
from pathlib import Path, PureWindowsPath
from types import MappingProxyType
from typing import Mapping

SCHEMA = "gnorix.inspection.v1"
MAX_SNAPSHOT_BYTES = 64 * 1024 * 1024
MAX_FRAME_SIDE = 16_384
MAX_FRAME_PIXELS = 16_777_216
MAX_POINTS = 100_000
MAX_RASTER_CELLS = 1_000_000
MAX_LATTICE_COORDINATE = 2**53 - 1


class ValidationError(ValueError):
    """An invalid snapshot, with the failing input path in its message."""


@dataclass(frozen=True, slots=True)
class Frame:
    id: str
    label: str
    width: int
    height: int
    image: Path | None


@dataclass(frozen=True, slots=True)
class Metric:
    key: str
    label: str
    unit: str
    domain: tuple[float, float] | None


@dataclass(frozen=True, slots=True)
class Point:
    """Image-space location with an optional circular tolerance radius in pixels."""

    id: str
    x: float
    y: float
    q: int | None
    r: int | None
    valid: bool
    values: Mapping[str, float | None]
    tolerance_radius: float | None = None


@dataclass(frozen=True, slots=True)
class Raster:
    label: str
    x: float
    y: float
    width: float
    height: float
    columns: int
    rows: int
    values: tuple[float | None, ...]
    domain: tuple[float, float] | None


@dataclass(frozen=True, slots=True)
class SeriesPoint:
    x: float
    y: float


@dataclass(frozen=True, slots=True)
class Series:
    id: str
    label: str
    x_label: str
    y_label: str
    points: tuple[SeriesPoint, ...]


@dataclass(frozen=True, slots=True)
class Stage:
    id: str
    label: str
    description: str
    frame: str
    metrics: tuple[Metric, ...]
    points: tuple[Point, ...]
    raster: Raster | None
    series: tuple[Series, ...]


@dataclass(frozen=True, slots=True)
class Snapshot:
    title: str
    description: str
    frames: tuple[Frame, ...]
    stages: tuple[Stage, ...]


def load_snapshot(path: str | Path) -> Snapshot:
    """Read at most 64 MiB of UTF-8 JSON and resolve images inside its bundle."""
    path = Path(path)
    try:
        with path.open("rb") as stream:
            encoded = stream.read(MAX_SNAPSHOT_BYTES + 1)
    except OSError as error:
        raise ValidationError(f"snapshot: cannot read {path}: {error}") from error

    if len(encoded) > MAX_SNAPSHOT_BYTES:
        _fail("snapshot", "JSON file exceeds the 64 MiB limit")

    try:
        data = json.loads(
            encoded.decode("utf-8"),
            parse_constant=_reject_constant,
            object_pairs_hook=_unique_json_keys,
        )
    except ValidationError:
        raise
    except (ValueError, RecursionError) as error:
        raise ValidationError(f"snapshot: invalid UTF-8 JSON: {error}") from error

    return validate_snapshot(data, path.parent)


def validate_snapshot(data: object, base_dir: str | Path) -> Snapshot:
    """Validate the v1 contract and return immutable renderer-facing values."""
    root = _object(data, "snapshot")
    if root.get("schema") != SCHEMA:
        _fail("snapshot.schema", f"must equal {SCHEMA!r}")

    bundle = Path(base_dir).resolve()
    if not bundle.is_dir():
        _fail("base_dir", "must be an existing directory")

    title = _text(root.get("title"), "snapshot.title")
    description = _optional_text(root, "description", "snapshot")
    frame_data = _array(root.get("frames"), "snapshot.frames", 24, 1)
    stage_data = _array(root.get("stages"), "snapshot.stages", 100, 1)

    frame_ids: set[str] = set()
    frames = tuple(
        _frame(value, f"snapshot.frames[{index}]", frame_ids, bundle)
        for index, value in enumerate(frame_data)
    )
    frames_by_id = {frame.id: frame for frame in frames}
    stage_ids: set[str] = set()
    stages = tuple(
        _stage(value, f"snapshot.stages[{index}]", stage_ids, frames_by_id)
        for index, value in enumerate(stage_data)
    )

    return Snapshot(title, description, frames, stages)


def _frame(value: object, path: str, ids: set[str], bundle: Path) -> Frame:
    data = _object(value, path)
    identifier = _identifier(data.get("id"), f"{path}.id", ids)
    label = _text(data.get("label"), f"{path}.label")
    width = _positive_integer(data.get("width"), f"{path}.width", MAX_FRAME_SIDE)
    height = _positive_integer(data.get("height"), f"{path}.height", MAX_FRAME_SIDE)
    if width * height > MAX_FRAME_PIXELS:
        _fail(path, f"frame must contain at most {MAX_FRAME_PIXELS} pixels")

    image = _image_path(data["image"], f"{path}.image", bundle) if "image" in data else None
    return Frame(identifier, label, width, height, image)


def _image_path(value: object, path: str, bundle: Path) -> Path:
    name = _text(value, path)
    relative = Path(name)
    if (
        ":" in name
        or "\\" in name
        or "\0" in name
        or relative.is_absolute()
        or PureWindowsPath(name).is_absolute()
        or ".." in relative.parts
    ):
        _fail(path, "must be a relative local image path without URLs or parent traversal")
    if relative.suffix.lower() not in {".png", ".jpg", ".jpeg"}:
        _fail(path, "must refer to a PNG or JPEG file")

    try:
        image = (bundle / relative).resolve(strict=True)
    except (OSError, RuntimeError, ValueError) as error:
        raise ValidationError(f"{path}: image cannot be resolved: {error}") from error
    if not image.is_relative_to(bundle):
        _fail(path, "image resolves outside the snapshot bundle")
    if not image.is_file():
        _fail(path, "image must be a regular file")

    return image


def _stage(value: object, path: str, ids: set[str], frames: Mapping[str, Frame]) -> Stage:
    data = _object(value, path)
    identifier = _identifier(data.get("id"), f"{path}.id", ids)
    label = _text(data.get("label"), f"{path}.label")
    description = _optional_text(data, "description", path)
    frame_id = _text(data.get("frame"), f"{path}.frame")
    if frame_id not in frames:
        _fail(f"{path}.frame", "references an unknown frame")
    frame = frames[frame_id]

    metric_ids: set[str] = set()
    metrics = tuple(
        _metric(item, f"{path}.metrics[{index}]", metric_ids)
        for index, item in enumerate(_array(data.get("metrics"), f"{path}.metrics", 256))
    )
    point_ids: set[str] = set()
    cells: set[tuple[int, int]] = set()
    points = tuple(
        _point(item, f"{path}.points[{index}]", point_ids, cells, metric_ids, frame)
        for index, item in enumerate(_array(data.get("points", []), f"{path}.points", MAX_POINTS))
    )
    raster = _raster(data["raster"], f"{path}.raster", frame) if "raster" in data else None
    series_ids: set[str] = set()
    series = tuple(
        _series(item, f"{path}.series[{index}]", series_ids)
        for index, item in enumerate(_array(data.get("series", []), f"{path}.series", 32))
    )

    return Stage(identifier, label, description, frame_id, metrics, points, raster, series)


def _metric(value: object, path: str, ids: set[str]) -> Metric:
    data = _object(value, path)
    return Metric(
        _identifier(data.get("key"), f"{path}.key", ids),
        _text(data.get("label"), f"{path}.label"),
        _optional_text(data, "unit", path),
        _domain(data["domain"], f"{path}.domain") if "domain" in data else None,
    )


def _point(
    value: object,
    path: str,
    ids: set[str],
    cells: set[tuple[int, int]],
    metrics: set[str],
    frame: Frame,
) -> Point:
    data = _object(value, path)
    identifier = _identifier(data.get("id"), f"{path}.id", ids)
    x = _coordinate(data.get("x"), f"{path}.x", frame.width)
    y = _coordinate(data.get("y"), f"{path}.y", frame.height)
    valid = data.get("valid", True)
    if type(valid) is not bool:
        _fail(f"{path}.valid", "must be a boolean")

    q = r = None
    if "q" in data or "r" in data:
        q, r = data.get("q"), data.get("r")
        if any(type(item) is not int or abs(item) > MAX_LATTICE_COORDINATE for item in (q, r)):
            _fail(path, "q and r must both be integers within the exact 53-bit range")
        if (q, r) in cells:
            _fail(path, f"duplicate lattice coordinate ({q}, {r})")
        cells.add((q, r))

    source_values = _object(data.get("values"), f"{path}.values")
    values: dict[str, float | None] = {}
    for key, item in source_values.items():
        if key not in metrics:
            _fail(f"{path}.values[{key!r}]", "metric is not declared by the stage")
        values[key] = None if item is None else _finite(item, f"{path}.values[{key!r}]")

    tolerance_radius = data.get("toleranceRadius")
    if tolerance_radius is not None:
        tolerance_radius = _finite(tolerance_radius, f"{path}.toleranceRadius")
        if tolerance_radius < 0:
            _fail(f"{path}.toleranceRadius", "must be a nonnegative radius in image pixels")

    return Point(identifier, x, y, q, r, valid, MappingProxyType(values), tolerance_radius)


def _raster(value: object, path: str, frame: Frame) -> Raster:
    data = _object(value, path)
    label = _text(data.get("label"), f"{path}.label")
    x = _coordinate(data.get("x"), f"{path}.x", frame.width)
    y = _coordinate(data.get("y"), f"{path}.y", frame.height)
    width = _finite(data.get("width"), f"{path}.width")
    height = _finite(data.get("height"), f"{path}.height")
    if width <= 0 or height <= 0 or x + width > frame.width or y + height > frame.height:
        _fail(path, "raster rectangle must be positive and contained in its frame")

    columns = _positive_integer(data.get("columns"), f"{path}.columns", MAX_RASTER_CELLS)
    rows = _positive_integer(data.get("rows"), f"{path}.rows", MAX_RASTER_CELLS)
    count = columns * rows
    if count > MAX_RASTER_CELLS:
        _fail(path, f"raster exceeds {MAX_RASTER_CELLS} cells")
    source_values = _array(data.get("values"), f"{path}.values", count, count)
    values = tuple(
        None if item is None else _finite(item, f"{path}.values[{index}]")
        for index, item in enumerate(source_values)
    )
    domain = _domain(data["domain"], f"{path}.domain") if "domain" in data else None

    return Raster(label, x, y, width, height, columns, rows, values, domain)


def _series(value: object, path: str, ids: set[str]) -> Series:
    data = _object(value, path)
    identifier = _identifier(data.get("id"), f"{path}.id", ids)
    label = _text(data.get("label"), f"{path}.label")
    x_label = _text(data.get("xLabel"), f"{path}.xLabel")
    y_label = _text(data.get("yLabel"), f"{path}.yLabel")
    points = []
    for index, item in enumerate(_array(data.get("points"), f"{path}.points", 20_000)):
        point_path = f"{path}.points[{index}]"
        point = _object(item, point_path)
        points.append(
            SeriesPoint(
                _finite(point.get("x"), f"{point_path}.x"),
                _finite(point.get("y"), f"{point_path}.y"),
            )
        )

    return Series(identifier, label, x_label, y_label, tuple(points))


def metric_domain(stage: Stage, key: str) -> tuple[float, float] | None:
    """Return declared limits or the observed range of valid, present values."""
    metric = next((item for item in stage.metrics if item.key == key), None)
    if metric is None:
        _fail("metric", f"unknown key {key!r}")
    if metric.domain is not None:
        return metric.domain

    minimum = maximum = None
    for point in stage.points:
        value = point.values.get(key)
        if point.valid and value is not None:
            minimum = value if minimum is None else min(minimum, value)
            maximum = value if maximum is None else max(maximum, value)

    return None if minimum is None else (minimum, maximum)


def build_edges(stage: Stage) -> list[tuple[int, int]]:
    """Return each axial-neighbor edge once, including invalid-point geometry."""
    cells = {
        (point.q, point.r): index
        for index, point in enumerate(stage.points)
        if point.q is not None and point.r is not None
    }
    edges = []
    for index, point in enumerate(stage.points):
        if point.q is None or point.r is None:
            continue
        for dq, dr in ((1, 0), (0, 1), (1, -1)):
            neighbor = cells.get((point.q + dq, point.r + dr))
            if neighbor is not None:
                edges.append((index, neighbor))

    return edges


def _fail(path: str, message: str) -> None:
    raise ValidationError(f"{path}: {message}")


def _object(value: object, path: str) -> dict:
    if not isinstance(value, dict):
        _fail(path, "must be an object")
    return value


def _text(value: object, path: str, allow_empty: bool = False) -> str:
    if not isinstance(value, str) or (not allow_empty and not value.strip()):
        _fail(path, "must be a string" if allow_empty else "must be a nonempty string")
    return value


def _optional_text(data: dict, key: str, path: str) -> str:
    return _text(data[key], f"{path}.{key}", True) if key in data else ""


def _array(value: object, path: str, maximum: int, minimum: int = 0) -> list:
    if not isinstance(value, list) or not minimum <= len(value) <= maximum:
        _fail(path, f"must be an array with {minimum} to {maximum} items")
    return value


def _finite(value: object, path: str) -> float:
    if type(value) not in (int, float):
        _fail(path, "must be a finite number")
    try:
        number = float(value)
    except OverflowError:
        _fail(path, "must be a finite number")
    if not math.isfinite(number):
        _fail(path, "must be a finite number")
    return number


def _positive_integer(value: object, path: str, maximum: int) -> int:
    if type(value) is not int or not 1 <= value <= maximum:
        _fail(path, f"must be an integer from 1 to {maximum}")
    return value


def _identifier(value: object, path: str, ids: set[str]) -> str:
    identifier = _text(value, path)
    if identifier in ids:
        _fail(path, f"duplicate identifier {identifier!r}")
    ids.add(identifier)
    return identifier


def _coordinate(value: object, path: str, bound: int) -> float:
    number = _finite(value, path)
    if not 0 <= number < bound:
        _fail(path, f"must be inside the frame [0, {bound})")
    return number


def _domain(value: object, path: str) -> tuple[float, float]:
    values = _array(value, path, 2, 2)
    minimum = _finite(values[0], f"{path}[0]")
    maximum = _finite(values[1], f"{path}[1]")
    if minimum > maximum:
        _fail(path, "minimum must not exceed maximum")
    return minimum, maximum


def _reject_constant(value: str) -> None:
    _fail("snapshot", f"nonfinite JSON constant {value!r} is not allowed")


def _unique_json_keys(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            _fail("snapshot", f"duplicate JSON key {key!r}")
        result[key] = value
    return result
