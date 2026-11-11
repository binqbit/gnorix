"""Source-coordinate point centers, lattice edges and supplied tolerance outlines."""

from __future__ import annotations

import math

import matplotlib
import numpy as np
from matplotlib.collections import LineCollection, PatchCollection
from matplotlib.lines import Line2D
from matplotlib.patches import Circle, Rectangle

from .model import Frame, Metric, Point, Stage, build_edges
from .options import OverlayOptions


def draw_annotations(
    axis, frame: Frame, stage: Stage, metric: Metric | None, options: OverlayOptions, norm
):
    """Draw geometry in original pixels and return its scalar mappable, if any."""
    bounds = view_bounds(frame, options)
    _draw_edges(axis, stage, options.alpha)
    _draw_tolerances(axis, stage, metric, options, norm, bounds)
    measured, missing, rejected = point_groups(stage, metric)
    plot = _draw_centers(axis, measured, metric, options, norm, bounds)
    _draw_missing(axis, missing, rejected, options, bounds)
    values = [point.values[metric.key] for point in measured] if metric is not None else []
    return plot, values


def point_groups(
    stage: Stage, metric: Metric | None
) -> tuple[list[Point], list[Point], list[Point]]:
    measured, missing, rejected = [], [], []
    for point in stage.points:
        if not point.valid:
            rejected.append(point)
        elif metric is None or point.values.get(metric.key) is not None:
            measured.append(point)
        else:
            missing.append(point)
    return measured, missing, rejected


def view_bounds(frame: Frame, options: OverlayOptions) -> tuple[float, float, float, float]:
    return options.crop if options.crop is not None else (0, 0, frame.width, frame.height)


def description(stage: Stage, options: OverlayOptions) -> str:
    text = f"Point radius: {options.point_radius:g} px · overlay alpha: {options.alpha:g}. "
    if options.tolerance_radius is not None:
        text += f"Tolerance display override: {options.tolerance_radius:g} px for all points. "
    else:
        radii = [
            point.tolerance_radius for point in stage.points if point.tolerance_radius is not None
        ]
        if radii:
            text += (
                f"Per-point tolerance radii: [{min(radii):g}, {max(radii):g}] px "
                f"for {len(radii)}/{len(stage.points)} points. "
            )
        else:
            text += "Tolerance radii not supplied. "
    text += "Tolerance outlines visualize supplied radii."
    if options.crop is not None:
        x, y, width, height = options.crop
        text += (
            f" Display crop: x={x:g}, y={y:g}, width={width:g}, height={height:g} px. "
            "Statistics use the full stage."
        )
    return text


def _draw_edges(axis, stage: Stage, alpha: float) -> None:
    edges = build_edges(stage)
    if not edges:
        return
    segments = [
        [
            (stage.points[first].x, stage.points[first].y),
            (stage.points[second].x, stage.points[second].y),
        ]
        for first, second in edges
    ]
    lines = LineCollection(segments, colors="#6b7a8c", linewidths=0.45, alpha=alpha * 0.6, zorder=1)
    lines.set_gid("lattice-edges")
    axis.add_collection(lines)


def _draw_tolerances(
    axis, stage: Stage, metric: Metric | None, options: OverlayOptions, norm, bounds
) -> None:
    circles, colors = [], []
    for point in stage.points:
        radius = options.tolerance_radius
        if radius is None:
            radius = point.tolerance_radius
        if radius is None or radius == 0 or radius > _farthest_corner(point, bounds):
            # A zero-radius circumference has no visible area. An enclosing
            # circle's circumference is entirely outside the displayed bounds.
            continue
        circles.append(Circle((point.x, point.y), radius, fill=False))
        colors.append(_point_color(point, metric, norm))
    if circles:
        outlines = PatchCollection(
            circles,
            facecolors="none",
            edgecolors=colors,
            linewidths=0.8,
            alpha=options.alpha,
            zorder=2,
        )
        outlines.set_gid("tolerance-outlines")
        outlines.set_clip_path(axis.patch)
        axis.add_collection(outlines)


def _draw_centers(
    axis, points: list[Point], metric: Metric | None, options: OverlayOptions, norm, bounds
):
    if not points:
        return None
    patches = []
    for point in points:
        if options.point_radius >= _farthest_corner(point, bounds):
            # The visible portion of an enclosing filled center is precisely
            # the display rectangle; avoid giant transformed circle coordinates.
            patches.append(Rectangle(bounds[:2], bounds[2], bounds[3]))
        else:
            patches.append(Circle((point.x, point.y), options.point_radius))
    centers = PatchCollection(patches, linewidths=0, alpha=options.alpha, zorder=3)
    centers.set_gid("point-centers")
    centers.set_clip_path(axis.patch)
    if metric is None:
        centers.set_facecolor("#16758a")
    else:
        centers.set_cmap("cividis")
        centers.set_norm(norm)
        centers.set_array(np.array([point.values[metric.key] for point in points]))
    axis.add_collection(centers)
    return centers


def _draw_missing(
    axis, missing: list[Point], rejected: list[Point], options: OverlayOptions, bounds
) -> None:
    handles = []
    radius = options.point_radius
    for points, color, label in [
        (missing, "#6b7280", "Missing"),
        (rejected, "#202a35", "Rejected"),
    ]:
        if not points:
            continue
        segments = []
        for point in points:
            segments.extend(_cross_segments(point, radius, bounds))
        crosses = LineCollection(
            segments, colors=color, linewidths=0.8, alpha=options.alpha, zorder=4
        )
        crosses.set_gid(label.lower() + "-centers")
        crosses.set_clip_path(axis.patch)
        axis.add_collection(crosses)
        handles.append(
            Line2D(
                [],
                [],
                marker="x",
                linestyle="none",
                color=color,
                alpha=options.alpha,
                label=f"{label}: {len(points)}",
            )
        )
    if handles:
        axis.legend(handles=handles, loc="lower left", fontsize=9, framealpha=0.9)


def _point_color(point: Point, metric: Metric | None, norm):
    if not point.valid:
        return "#202a35"
    if metric is None:
        return "#16758a"
    value = point.values.get(metric.key)
    return "#6b7280" if value is None else matplotlib.colormaps["cividis"](norm(value))


def _farthest_corner(point: Point, bounds) -> float:
    x, y, width, height = bounds
    return max(
        math.hypot(point.x - corner_x, point.y - corner_y)
        for corner_x in (x, x + width)
        for corner_y in (y, y + height)
    )


def _cross_segments(point: Point, radius: float, bounds) -> list:
    x, y, width, height = bounds
    segments = []
    for slope in (-1, 1):
        y_limits = sorted(((y - point.y) * slope, (y + height - point.y) * slope))
        lower = max(-radius, x - point.x, y_limits[0])
        upper = min(radius, x + width - point.x, y_limits[1])
        if lower <= upper:
            segments.append(
                [
                    (point.x + lower, point.y + slope * lower),
                    (point.x + upper, point.y + slope * upper),
                ]
            )
    return segments
