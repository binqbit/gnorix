"""Raw centerline evidence, length-weighted scale validation and PNG diagnostics."""

from __future__ import annotations

import math
from dataclasses import dataclass

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.collections import LineCollection
from matplotlib.colors import Normalize

from .scale import WidthConsistency, validate_width_consistency

IMAGE_KEYS = ("measurement_skeleton", "pattern_support")


@dataclass(frozen=True)
class WidthObservation:
    link: tuple[int, int]
    start: tuple[float, float]
    end: tuple[float, float]
    represented_length_px: float
    mean_gray: float

    @property
    def width_px(self):
        return math.dist(self.start, self.end)


@dataclass(frozen=True)
class Coverage:
    total_length_px: float
    measured_length_px: float
    total_links: int
    measured_links: int
    isolated_pixels: int


@dataclass(frozen=True)
class FullScale:
    unit_px: float
    coverage: Coverage
    observations: tuple[WidthObservation, ...]
    width_consistency: tuple[WidthConsistency, ...]


def validate_full_scale(value, width: int, height: int) -> FullScale:
    """Reconstruct primary scale from every supplied measured centerline link."""
    if not isinstance(value, dict) or value.get("method") != "raw-centerline-link-width":
        raise ValueError("scale.method must be raw-centerline-link-width")
    supplied = value.get("observations")
    maximum = width * height * 4
    if not isinstance(supplied, list) or not 1 <= len(supplied) <= maximum:
        raise ValueError("scale.observations must be nonempty and bounded by four links per pixel")
    observations, seen = [], set()
    for raw in supplied:
        observation = _observation(raw, width, height)
        if observation.link in seen:
            raise ValueError("scale contains duplicate undirected link observations")
        seen.add(observation.link)
        observations.append(observation)
    observations = tuple(observations)
    lengths = [observation.represented_length_px for observation in observations]
    widths = [observation.width_px for observation in observations]
    measured_length = math.fsum(lengths)
    expected = (
        math.fsum(length * span for length, span in zip(lengths, widths, strict=True))
        / measured_length
    )
    unit = _positive(value.get("unit_px"), "scale.unit_px")
    _same(unit, expected, "scale.unit_px")
    coverage = _coverage(value.get("coverage"), maximum, width * height)
    if coverage.measured_links != len(observations):
        raise ValueError("scale measured_links does not match observations")
    _same(coverage.measured_length_px, measured_length, "scale measured_length_px")
    bands = validate_width_consistency(value.get("width_consistency"), widths, unit)
    return FullScale(unit, coverage, observations, bands)


def _number(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be finite")
    try:
        result = float(value)
    except OverflowError as error:
        raise ValueError(f"{name} must be finite") from error
    if not math.isfinite(result):
        raise ValueError(f"{name} must be finite")
    return result


def _positive(value, name):
    result = _number(value, name)
    if result <= 0:
        raise ValueError(f"{name} must be positive")
    return result


def _count(value, maximum, name):
    if type(value) is not int or not 0 <= value <= maximum:
        raise ValueError(f"{name} must be an integer in [0, {maximum}]")
    return value


def _same(actual, expected, name):
    if not math.isclose(actual, expected, rel_tol=1e-8, abs_tol=1e-7):
        raise ValueError(f"{name} does not match supplied evidence")


def _point(value, width, height):
    if not isinstance(value, list) or len(value) != 2:
        raise ValueError("scale width endpoints must contain x and y")
    x, y = (_number(v, "scale width endpoint") for v in value)
    if not 0 <= x <= width - 1 or not 0 <= y <= height - 1:
        raise ValueError("scale width endpoint lies outside source pixel centers")
    return x, y


def _observation(value, width, height):
    if not isinstance(value, dict):
        raise ValueError("scale observation must be an object")
    link = value.get("link")
    if not isinstance(link, list) or len(link) != 2:
        raise ValueError("scale observation link must contain two pixel indices")
    a, b = (_count(v, width * height - 1, "scale link pixel") for v in link)
    if a >= b:
        raise ValueError("scale link pixel indices must be canonical and increasing")
    dx, dy = abs(a % width - b % width), abs(a // width - b // width)
    if max(dx, dy) != 1:
        raise ValueError("scale link must connect neighboring source pixels")
    length = _positive(value.get("represented_length_px"), "scale represented_length_px")
    _same(length, math.hypot(dx, dy), "scale represented_length_px")
    start, end = (_point(value.get(key), width, height) for key in ("start", "end"))
    if start == end:
        raise ValueError("scale width spans must have positive length")
    station = ((a % width + b % width) / 2, (a // width + b // width) / 2)
    dx, dy = end[0] - start[0], end[1] - start[1]
    length_px = math.hypot(dx, dy)
    ux, uy = dx / length_px, dy / length_px
    sx, sy = station[0] - start[0], station[1] - start[1]
    precision = 64 * np.finfo(float).eps * max(width, height, 1)
    projection = sx * ux + sy * uy
    if abs(sx * uy - sy * ux) > precision or not -precision <= projection <= length_px + precision:
        raise ValueError("scale own-width span must contain its link midpoint on the segment")
    gray = _number(value.get("mean_gray"), "scale observation mean_gray")
    if not 0 <= gray <= 255:
        raise ValueError("scale observation mean_gray must be in [0, 255]")
    return WidthObservation((a, b), start, end, length, gray)


def _coverage(value, maximum, pixels):
    if not isinstance(value, dict):
        raise ValueError("scale.coverage must be an object")
    total = _positive(value.get("total_length_px"), "scale total_length_px")
    measured = _positive(value.get("measured_length_px"), "scale measured_length_px")
    total_links = _count(value.get("total_links"), maximum, "scale total_links")
    measured_links = _count(value.get("measured_links"), maximum, "scale measured_links")
    isolated = _count(value.get("isolated_pixels"), pixels, "scale isolated_pixels")
    if measured_links > total_links or measured > total * (1 + 1e-12):
        raise ValueError("scale measured coverage must not exceed total coverage")
    return Coverage(total, measured, total_links, measured_links, isolated)


def validate_measurement_maps(scale: FullScale, images):
    """Verify coverage against the actual raw skeleton's graph-neighbor rule.

    Diagonals with an orthogonal ridge bridge are suppressed. Filled 2×2 cells
    also suppress their top horizontal link, exactly as the native graph does.
    Unsupported links and isolated pixels remain in the total denominator.
    """
    skeleton, support = (_binary(images[key], key) for key in IMAGE_KEYS)
    ridge = skeleton == 0
    supported = support == 255
    height, width = ridge.shape
    for observation in scale.observations:
        a, b = observation.link
        ay, ax = divmod(a, width)
        by, bx = divmod(b, width)
        if not supported[ay, ax] or not supported[by, bx]:
            raise ValueError("measured link endpoints must have pattern support")
        if not _has_link(ridge, ax, ay, bx, by):
            raise ValueError("observed link is absent from the measurement skeleton topology")
    total_links, total_length, isolated = _topology_counts(ridge)
    if total_links != scale.coverage.total_links or isolated != scale.coverage.isolated_pixels:
        raise ValueError("scale link/isolate coverage does not match measurement skeleton")
    _same(scale.coverage.total_length_px, total_length, "scale total_length_px")


def _binary(image, name):
    if image.mode != "L":
        raise ValueError(f"images.{name} must be opaque 8-bit grayscale")
    values = np.asarray(image)
    if not np.all((values == 0) | (values == 255)):
        raise ValueError(f"images.{name} must contain only 0 or 255")
    return values


def _has_link(ridge, ax, ay, bx, by):
    if not ridge[ay, ax] or not ridge[by, bx]:
        return False
    if ax != bx and ay != by:
        return not (ridge[ay, bx] or ridge[by, ax])
    if ay == by and ay + 1 < ridge.shape[0]:
        return not (ridge[ay + 1, ax] and ridge[ay + 1, bx])
    return True


def _topology_counts(ridge):
    east = ridge[:, :-1] & ridge[:, 1:]
    east[:-1] &= ~(ridge[1:, :-1] & ridge[1:, 1:])
    south = ridge[:-1] & ridge[1:]
    southeast = ridge[:-1, :-1] & ridge[1:, 1:] & ~ridge[:-1, 1:] & ~ridge[1:, :-1]
    southwest = ridge[:-1, 1:] & ridge[1:, :-1] & ~ridge[:-1, :-1] & ~ridge[1:, 1:]
    connected = np.zeros_like(ridge)
    for links, first, second in (
        (east, (slice(None), slice(None, -1)), (slice(None), slice(1, None))),
        (south, (slice(None, -1), slice(None)), (slice(1, None), slice(None))),
        (southeast, (slice(None, -1), slice(None, -1)), (slice(1, None), slice(1, None))),
        (southwest, (slice(None, -1), slice(1, None)), (slice(1, None), slice(None, -1))),
    ):
        connected[first] |= links
        connected[second] |= links
    straight = int(np.count_nonzero(east) + np.count_nonzero(south))
    diagonal = int(np.count_nonzero(southeast) + np.count_nonzero(southwest))
    return (
        straight + diagonal,
        straight + diagonal * math.sqrt(2),
        int(np.count_nonzero(ridge & ~connected)),
    )


def render_full_scale(scale: FullScale, images, alpha=0.5):
    figure = plt.figure(figsize=(17, 12), layout="constrained")
    try:
        grid = figure.add_gridspec(2, 3, width_ratios=(1.4, 1, 1), height_ratios=(1, 1))
        _span_overlay(figure.add_subplot(grid[:, 0]), scale, images["source"], alpha)
        for slot, key, label in (
            (grid[0, 1], "measurement_skeleton", "Raw measurement centerline · dark = ridge"),
            (grid[0, 2], "pattern_support", "Confirmed pattern region · white = present"),
        ):
            axis = figure.add_subplot(slot)
            _image(axis, images[key])
            axis.set_title(label, fontsize=10)
        _width_histogram(figure.add_subplot(grid[1, 1]), scale)
        _coverage_summary(figure.add_subplot(grid[1, 2]), scale)
        figure.suptitle(
            f"Full raw-centerline scale · 1 R = {scale.unit_px:.5g} source px\n"
            "Every measured line portion contributes in proportion to its represented arclength",
            fontsize=15,
        )
        return figure
    except Exception:
        plt.close(figure)
        raise


def _image(axis, image):
    width, height = image.size
    extent = (-0.5, width - 0.5, height - 0.5, -0.5)
    axis.imshow(
        image, cmap="gray", vmin=0, vmax=255, extent=extent, alpha=1, interpolation="nearest"
    )
    axis.set(
        xlim=extent[:2],
        ylim=extent[2:],
        aspect="equal",
        xlabel="x (source px)",
        ylabel="y (source px)",
    )


def _display_observations(observations, width, height):
    selected = {}
    for item in observations:
        a, b = item.link
        x, y = (a % width + b % width) / 2, (a // width + b // width) / 2
        column, row = min(11, int(12 * x / width)), min(11, int(12 * y / height))
        distance = math.hypot(x - (column + 0.5) * width / 12, y - (row + 0.5) * height / 12)
        cell = row, column
        if cell not in selected or distance < selected[cell][0]:
            selected[cell] = distance, item
    return tuple(selected[cell][1] for cell in sorted(selected))


def _span_overlay(axis, scale, image, alpha):
    _image(axis, image)
    selected = _display_observations(scale.observations, *image.size)
    widths = [item.width_px for item in scale.observations]
    low, high = min(widths), max(widths)
    if low == high:
        low, high = max(0, low - 0.5), high + 0.5
    artist = LineCollection(
        [(item.start, item.end) for item in selected],
        array=np.asarray([item.width_px for item in selected]),
        cmap="viridis",
        norm=Normalize(low, high),
        linewidths=1.4,
        alpha=alpha,
    )
    axis.add_collection(artist)
    axis.figure.colorbar(
        artist,
        ax=axis,
        orientation="horizontal",
        fraction=0.04,
        pad=0.05,
        label="Measured own width (source px)",
    )
    axis.set_title(
        f"{len(selected)} of {len(scale.observations)} own-width spans shown\n"
        "Spatial subset for display only",
        fontsize=11,
    )


def _width_histogram(axis, scale):
    widths = [item.width_px for item in scale.observations]
    lengths = [item.represented_length_px for item in scale.observations]
    axis.hist(
        widths,
        weights=lengths,
        bins=min(40, max(1, math.ceil(math.sqrt(len(widths))))),
        color="#4477aa",
        edgecolor="white",
        linewidth=0.5,
    )
    axis.axvline(
        scale.unit_px, color="#d55e00", linestyle="--", label=f"1 R = {scale.unit_px:.5g} px"
    )
    axis.set(
        title=f"All {len(widths)} observations · length-weighted",
        xlabel="Measured own width (source px)",
        ylabel="Represented centerline length (px)",
    )
    axis.legend(fontsize=9)
    axis.grid(axis="y", alpha=0.2)
    axis.set_axisbelow(True)


def _coverage_summary(axis, scale):
    coverage = scale.coverage
    skipped = max(0, coverage.total_length_px - coverage.measured_length_px)
    axis.set_axis_off()
    lines = [
        "Measurement coverage",
        "",
        f"Measured: {coverage.measured_length_px:.6g} px",
        f"Unmeasured: {skipped:.6g} px",
        f"Total: {coverage.total_length_px:.6g} px",
        f"Measured links: {coverage.measured_links} / {coverage.total_links}",
        f"Isolated pixels: {coverage.isolated_pixels} (zero length)",
        "",
        f"Measured length / R: {coverage.measured_length_px / scale.unit_px:.6g}",
        "",
        "Own-width consistency around R (counts)",
    ]
    lines.extend(
        f"±{band.relative_tolerance:.0%}: {band.accepted_measurements} / "
        f"{band.total_measurements} ({band.fraction:.2%})"
        for band in scale.width_consistency
    )
    lines.extend(
        [
            "",
            "Fractions count observations, not length coverage.",
            "No biological accuracy or recognition probability.",
        ]
    )
    axis.text(
        0, 0.98, "\n".join(lines), transform=axis.transAxes, va="top", fontsize=10, linespacing=1.6
    )
