"""Descriptive sample data for exercising reports, not a biometric algorithm."""

from __future__ import annotations

import json
import math
from pathlib import Path

import numpy as np
from PIL import Image

from .images import load_image

MAX_POINTS = 100_000


def write_sample(
    image_path: Path, output: Path, spacing: float, radius: int, rotation: float
) -> Path:
    """Write a self-contained diagnostic bundle using the image's actual pixels."""
    validate_parameters(spacing, radius, rotation)
    with load_grayscale(image_path) as image:
        points = sample_points(image, spacing, radius, rotation)
        snapshot = make_snapshot(image, points, spacing, radius, rotation)
        output.mkdir(parents=True, exist_ok=True)
        image.save(output / "capture.png")

    destination = output / "snapshot.json"
    destination.write_text(json.dumps(snapshot, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    return destination


def validate_parameters(spacing: float, radius: int, rotation: float) -> None:
    if not math.isfinite(spacing) or spacing <= 0:
        raise ValueError("spacing must be finite and positive")
    if not isinstance(radius, int) or isinstance(radius, bool) or not 1 <= radius <= 2048:
        raise ValueError("radius must be an integer from 1 to 2048 pixels")
    if not math.isfinite(rotation):
        raise ValueError("rotation must be finite")


def load_grayscale(path: Path) -> Image.Image:
    """Decode explicitly; generated pixel dimensions do not imply sensor DPI."""
    with load_image(path) as image:
        if image.mode in {"RGBA", "LA"} or "transparency" in image.info:
            raise ValueError("sample input must be opaque; composite transparency explicitly first")
        return image.convert("L")


def grid_positions(width: int, height: int, spacing: float, rotation: float):
    """Triangular preview grid in image pixels, rotated about the frame center."""
    angle = math.radians(rotation % 360)
    cosine, sine = math.cos(angle), math.sin(angle)
    origin_x, origin_y = width / 2, height / 2
    row_height = spacing * (math.sqrt(3) / 2)
    corners = []

    for x, y in ((0, 0), (width, 0), (0, height), (width, height)):
        dx, dy = x - origin_x, y - origin_y
        local_x = cosine * dx + sine * dy
        local_y = -sine * dx + cosine * dy
        r = local_y / row_height
        q = local_x / spacing - r / 2
        if not math.isfinite(q) or not math.isfinite(r):
            raise ValueError("preview grid is too dense; increase spacing")
        corners.append((q, r))

    q_start = math.floor(min(q for q, _ in corners)) - 1
    q_end = math.ceil(max(q for q, _ in corners)) + 1
    r_start = math.floor(min(r for _, r in corners)) - 1
    r_end = math.ceil(max(r for _, r in corners)) + 1
    if (q_end - q_start + 1) * (r_end - r_start + 1) > MAX_POINTS * 4:
        raise ValueError("preview grid is too dense; increase spacing")

    count = 0
    for r in range(r_start, r_end + 1):
        for q in range(q_start, q_end + 1):
            local_x, local_y = spacing * (q + r / 2), row_height * r
            x = origin_x + cosine * local_x - sine * local_y
            y = origin_y + sine * local_x + cosine * local_y
            if 0 <= x < width and 0 <= y < height:
                count += 1
                if count > MAX_POINTS:
                    raise ValueError("preview grid exceeds 100,000 points; increase spacing")
                yield q, r, x, y


def summed_area(values: np.ndarray) -> np.ndarray:
    return np.pad(values.cumsum(axis=0).cumsum(axis=1), ((1, 0), (1, 0)))


def rectangle_sum(area: np.ndarray, left: int, top: int, right: int, bottom: int) -> float:
    return float(area[bottom, right] - area[top, right] - area[bottom, left] + area[top, left])


def sample_points(image: Image.Image, spacing: float, radius: int, rotation: float) -> list[dict]:
    values = np.asarray(image, dtype=np.float64)
    sums = summed_area(values)
    squares = summed_area(values * values)
    width, height = image.size
    points = []

    for q, r, x, y in grid_positions(width, height, spacing, rotation):
        column, row = math.floor(x), math.floor(y)
        left, right = max(0, column - radius), min(width, column + radius + 1)
        top, bottom = max(0, row - radius), min(height, row + radius + 1)
        count = (right - left) * (bottom - top)
        mean = rectangle_sum(sums, left, top, right, bottom) / count
        variance = rectangle_sum(squares, left, top, right, bottom) / count - mean * mean
        points.append(
            {
                "id": f"{q}:{r}",
                "x": x,
                "y": y,
                "q": q,
                "r": r,
                "valid": True,
                "values": {"intensity": mean, "contrast": math.sqrt(max(0, variance))},
            }
        )
    if not points:
        raise ValueError("preview grid produced no points")
    return points


def make_snapshot(
    image: Image.Image, points: list[dict], spacing: float, radius: int, rotation: float
) -> dict:
    width, height = image.size
    columns, rows = math.ceil(width / 32), math.ceil(height / 32)
    reduced = np.asarray(image.resize((columns, rows), Image.Resampling.BOX))
    values = np.asarray(image)
    center_row = values[height // 2]

    return {
        "schema": "gnorix.inspection.v1",
        "title": "Fingerprint pixel diagnostics",
        "description": (
            "Descriptive image measurements and a preview grid. "
            "No biometric weights, acceptance criteria or registration are computed."
        ),
        "frames": [
            {
                "id": "capture",
                "label": "Grayscale input",
                "width": width,
                "height": height,
                "image": "capture.png",
            }
        ],
        "stages": [
            {"id": "input", "label": "Input image", "frame": "capture", "metrics": []},
            {
                "id": "sampled",
                "label": "Triangular grid measurements",
                "description": (
                    f"Preview spacing {spacing:g} px; rotation {rotation:g} degrees; "
                    f"square window radius {radius} px. Edge windows are clipped to the image. "
                    "All image pixels are sampled; no fingerprint segmentation is assumed. "
                    "Contrast is local pixel standard deviation, not reliability."
                ),
                "frame": "capture",
                "metrics": [
                    {
                        "key": "intensity",
                        "label": "Local mean intensity",
                        "unit": "gray level",
                        "domain": [0, 255],
                    },
                    {
                        "key": "contrast",
                        "label": "Local intensity standard deviation",
                        "unit": "gray level",
                    },
                ],
                "points": points,
                "raster": {
                    "label": "Area-averaged intensity (display reduction)",
                    "x": 0,
                    "y": 0,
                    "width": width,
                    "height": height,
                    "columns": columns,
                    "rows": rows,
                    "values": reduced.flatten().astype(int).tolist(),
                    "domain": [0, 255],
                },
                "series": [
                    {
                        "id": "center-row",
                        "label": f"Intensity profile at y={height // 2}",
                        "xLabel": "x (px)",
                        "yLabel": "Gray level",
                        "points": [
                            {"x": x + 0.5, "y": int(value)} for x, value in enumerate(center_row)
                        ],
                    }
                ],
            },
        ],
    }
