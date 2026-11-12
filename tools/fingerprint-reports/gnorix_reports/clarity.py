"""Validate native local clarity and display it without reclassifying the image."""

from __future__ import annotations

import math
from dataclasses import dataclass

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.collections import LineCollection
from matplotlib.colors import to_rgba
from matplotlib.ticker import MaxNLocator

IMAGE_KEYS = (
    "clarity",
    "clarity_contrast",
    "clarity_separation",
    "clarity_repetition",
    "clarity_support",
)
UNKNOWN_COLOR = "#858585"
_HALF_BYTE = 0.5 / 255
OPACITY_LABEL = (
    "Opacity = selected maximum × (0.2 + 0.8 × sqrt(clarity)); gray paths = unavailable clarity."
)


@dataclass(frozen=True)
class Clarity:
    period_px: float
    grid_spacing_px: int
    supported_pixels: int
    total_pixels: int
    mean_score: float | None


@dataclass(frozen=True)
class ClarityMaps:
    score: np.ndarray
    contrast: np.ndarray
    separation: np.ndarray
    repetition: np.ndarray
    supported: np.ndarray


def _number(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a finite number")
    try:
        number = float(value)
    except OverflowError as error:
        raise ValueError(f"{name} must be a finite number") from error
    if not math.isfinite(number):
        raise ValueError(f"{name} must be a finite number")
    return number


def validate_clarity(value, period: float, width: int, height: int) -> Clarity:
    if not isinstance(value, dict):
        raise ValueError("clarity must be an object")
    for key, expected in (
        ("method", "local-ridge-separability-v1"),
        ("score", "interpolated(contrast * separation * repetition)"),
        ("segment_sampling", "bilinear-map-line-mean"),
        ("encoding", "rounded-u8"),
    ):
        if value.get(key) != expected:
            raise ValueError(f"clarity.{key} must be {expected!r}")
    native_period = _number(value.get("period_px"), "clarity.period_px")
    if native_period <= 0 or not math.isclose(native_period, period, rel_tol=1e-8):
        raise ValueError("clarity.period_px must equal bootstrap.ridge_period_px")
    spacing = value.get("grid_spacing_px")
    if type(spacing) is not int or spacing != max(1, math.floor(period / 4)):
        raise ValueError("clarity.grid_spacing_px must equal max(1, floor(period / 4))")
    summary = value.get("summary")
    if not isinstance(summary, dict):
        raise ValueError("clarity.summary must be an object")
    supported, total = (summary.get(key) for key in ("supported_pixels", "total_pixels"))
    if type(total) is not int or total != width * height:
        raise ValueError("clarity total_pixels must equal the source pixel count")
    if type(supported) is not int or not 0 <= supported <= total:
        raise ValueError("clarity supported_pixels must be a bounded pixel count")
    mean = summary.get("mean_score")
    if supported == 0:
        if mean is not None:
            raise ValueError("unsupported clarity must have a null mean_score")
    else:
        mean = _number(mean, "clarity.mean_score")
        if not 0 <= mean <= 1:
            raise ValueError("clarity.mean_score must be in [0, 1]")
    return Clarity(native_period, spacing, supported, total, mean)


def validate_clarity_maps(clarity: Clarity, images) -> ClarityMaps:
    arrays = [np.asarray(images[key], dtype=np.float64) / 255 for key in IMAGE_KEYS]
    score, *components, support = arrays
    if not np.all((support == 0) | (support == 1)):
        raise ValueError("clarity support pixels must be 0 or 255")
    supported = support == 1
    if np.count_nonzero(supported) != clarity.supported_pixels:
        raise ValueError("clarity support map must match supported_pixels")
    if any(np.any(values[~supported] != 0) for values in (score, *components)):
        raise ValueError("unsupported clarity and component pixels must be zero")

    _validate_anchor_interpolation(score, components, supported, clarity.grid_spacing_px)
    if (
        clarity.mean_score is not None
        and abs(float(np.mean(score[supported])) - clarity.mean_score) > _HALF_BYTE + 1e-7
    ):
        raise ValueError("clarity mean_score must match the rounded score map")
    return ClarityMaps(score, *components, supported)


def _grid_axis(length, spacing):
    positions = np.arange(0, length, spacing)
    if positions[-1] != length - 1:
        positions = np.append(positions, length - 1)
    return positions


def _axis_weights(positions, length):
    coordinates = np.arange(length)
    upper = np.searchsorted(positions, coordinates)
    lower = np.maximum(upper - 1, 0)
    span = positions[upper] - positions[lower]
    weights = np.divide(coordinates - positions[lower], span, out=np.zeros(length), where=span != 0)
    return lower, upper, weights


def _interpolate_anchors(values, x, y):
    left, right, wx = x
    top, bottom, wy = y
    rows = values[:, left] * (1 - wx) + values[:, right] * wx
    return rows[top] * (1 - wy[:, None]) + rows[bottom] * wy[:, None]


def _validate_anchor_interpolation(score, components, supported, spacing):
    """Check the exported interpolation contract, not source-image analysis.

    Products exist at native grid anchors. Interpolated component products can
    invent a peak, so dense scores are bounded by interpolated anchor scores.
    Bounds retain each PNG's independent rounding and native float precision.
    """
    height, width = score.shape
    xs, ys = _grid_axis(width, spacing), _grid_axis(height, spacing)
    grid = np.ix_(ys, xs)
    anchors = score[grid]
    known = supported[grid]
    parts = [values[grid] for values in components]
    lower = np.prod([np.maximum(0, values - _HALF_BYTE) for values in parts], axis=0)
    upper = np.prod([np.minimum(1, values + _HALF_BYTE) for values in parts], axis=0)
    _check_rounding(anchors, lower, upper, known, "grid-anchor component product")

    x, y = _axis_weights(xs, width), _axis_weights(ys, height)
    left, right, wx = x
    top, bottom, wy = y
    rows_known = (known[:, left] | (wx == 1)) & (known[:, right] | (wx == 0))
    dense_known = (rows_known[top] | (wy[:, None] == 1)) & (rows_known[bottom] | (wy[:, None] == 0))
    if np.any(supported & ~dense_known):
        raise ValueError("supported clarity requires every positive-weight grid anchor")
    lower = _interpolate_anchors(np.maximum(0, anchors - _HALF_BYTE), x, y)
    upper = _interpolate_anchors(np.minimum(1, anchors + _HALF_BYTE), x, y)
    _check_rounding(score, lower, upper, supported, "interpolated grid-anchor scores")


def _check_rounding(values, lower, upper, supported, contract):
    if np.any(values[supported] + _HALF_BYTE + 1e-7 < lower[supported]) or np.any(
        values[supported] - _HALF_BYTE - 1e-7 > upper[supported]
    ):
        raise ValueError(f"clarity score must match {contract} within PNG rounding")


def validate_segment_clarity(value, point_count: int, closed: bool) -> tuple[float | None, ...]:
    count = point_count if closed else point_count - 1
    if not isinstance(value, list) or len(value) != count:
        raise ValueError("segment_clarity must contain one value per supplied path segment")
    result = []
    for item in value:
        score = None if item is None else _number(item, "segment_clarity")
        if score is not None and not 0 <= score <= 1:
            raise ValueError("segment_clarity values must be null or in [0, 1]")
        result.append(score)
    return tuple(result)


class _ClarityLines(LineCollection):
    """Carry dash phase across colored segments at the actual rendered scale."""

    def __init__(self, segments, path_sizes, **style):
        super().__init__(segments, **style)
        self._path_sizes = path_sizes
        self._pattern = self.get_linestyles()[0][1]

    def draw(self, renderer):
        if self._pattern is not None:
            points = np.asarray(self.get_segments())
            display = self.get_transform().transform(points.reshape(-1, 2)).reshape(-1, 2, 2)
            lengths = np.linalg.norm(display[:, 1] - display[:, 0], axis=1)
            lengths /= renderer.points_to_pixels(1)
            scale = self.get_linewidths()[0] if matplotlib.rcParams["lines.scale_dashes"] else 1
            pattern = tuple(value / scale for value in self._pattern)
            period = sum(self._pattern)
            styles, start = [], 0
            for size in self._path_sizes:
                offsets = np.r_[0, np.cumsum(lengths[start : start + size - 1])]
                styles.extend(((offset % period) / scale, pattern) for offset in offsets)
                start += size
            self.set_linestyle(styles)
        super().draw(renderer)


def clarity_lines(paths, color, alpha, *, unknown_color=UNKNOWN_COLOR, **style) -> LineCollection:
    """Build exact source-coordinate segments with display-only RGBA mapping.

    Each path supplies (points, closed, native scores). Cycles always include
    their final-to-first segment, including a possible zero-length closing pair.
    """
    segments, colors, sizes = [], [], []
    for points, closed, scores in paths:
        ends = (*points[1:], points[0]) if closed else points[1:]
        starts = points if closed else points[:-1]
        sizes.append(len(scores))
        for start, end, score in zip(starts, ends, scores, strict=True):
            segments.append((start, end))
            opacity = alpha * (0.2 if score is None else 0.2 + 0.8 * math.sqrt(score))
            colors.append(to_rgba(unknown_color if score is None else color, opacity))
    return _ClarityLines(segments, sizes, colors=colors, **style)


def render_clarity_analysis(clarity: Clarity, maps: ClarityMaps, source, alpha):
    figure = plt.figure(figsize=(17, 13), layout="constrained")
    grid = figure.add_gridspec(3, 3, height_ratios=(1, 1, 0.13))
    axes = [figure.add_subplot(grid[row, column]) for row in range(2) for column in range(3)]
    try:
        width, height = source.size
        extent = (-0.5, width - 0.5, height - 0.5, -0.5)
        colors = matplotlib.colormaps["cividis"].with_extremes(bad="#dfeaf1")
        for axis in axes:
            axis.set(
                xlim=extent[:2],
                ylim=extent[2:],
                aspect="equal",
                xlabel="x (source px)",
                ylabel="y (source px)",
            )
            axis.xaxis.set_major_locator(MaxNLocator(4))
            axis.yaxis.set_major_locator(MaxNLocator(4))
        for axis in axes[:2]:
            axis.imshow(
                source,
                cmap="gray",
                vmin=0,
                vmax=255,
                extent=extent,
                interpolation="nearest",
                alpha=1,
            )
        axes[0].set_title("Source · unchanged grayscale")
        axes[1].imshow(
            np.ma.array(maps.score, mask=~maps.supported),
            cmap=colors.with_extremes(bad=(0, 0, 0, 0)),
            vmin=0,
            vmax=1,
            extent=extent,
            interpolation="nearest",
            alpha=alpha,
        )
        axes[1].set_title("Local clarity · source overlay")
        axes[2].imshow(
            maps.supported, cmap="gray", vmin=0, vmax=1, extent=extent, interpolation="nearest"
        )
        axes[2].set_title("Clarity support · white = measured")
        for axis, values, title in zip(
            axes[3:],
            (maps.contrast, maps.separation, maps.repetition),
            ("Contrast", "Class separation", "Ridge repetition"),
            strict=True,
        ):
            artist = axis.imshow(
                np.ma.array(values, mask=~maps.supported),
                cmap=colors,
                vmin=0,
                vmax=1,
                extent=extent,
                interpolation="nearest",
            )
            axis.set_title(title)
        figure.colorbar(
            artist,
            ax=[axes[1], *axes[3:]],
            fraction=0.025,
            pad=0.02,
            label="Native diagnostic value · fixed 0 to 1",
        )
        mean = "unavailable" if clarity.mean_score is None else f"{clarity.mean_score:.4f}"
        figure.suptitle(
            "Local ridge clarity · interpolated grid-anchor scores\n"
            f"Supported pixels: {clarity.supported_pixels} / {clarity.total_pixels} · "
            f"native supported-pixel mean: {mean}\n"
            f"Period: {clarity.period_px:.4g} px · sampling grid: {clarity.grid_spacing_px} px",
            fontsize=13,
        )
        footer = figure.add_subplot(grid[2, :])
        footer.set_axis_off()
        footer.text(
            0.5,
            0.5,
            "Zero is a measured low score; unavailable areas have no overlay and are "
            "pale blue in component maps.\n"
            "PNG maps round native values to 1/255. Scores are not rescaled per image.\n"
            "Anchor score = contrast × separation × repetition. Components interpolate separately; "
            "their dense product is not the score.\n"
            "Supported pixels may include flat background; clarity is diagnostic, "
            "not recognition probability or anatomical confidence.",
            ha="center",
            va="center",
            transform=footer.transAxes,
            fontsize=10,
        )
        return figure
    except Exception:
        plt.close(figure)
        raise
