"""Validated geometric evidence and diagnostic plots for automatic ridge scale."""

from __future__ import annotations

import math
import statistics
import sys
from dataclasses import dataclass

import matplotlib

from .validation import finite_number

matplotlib.use("Agg")

import matplotlib.pyplot as plt
from matplotlib.collections import LineCollection
from matplotlib.lines import Line2D
from matplotlib.patches import Rectangle
from matplotlib.ticker import MaxNLocator

_TOLERANCES = (0.1, 0.2, 0.3)
_MAX_PROFILES = 100_000


@dataclass(frozen=True)
class RidgeSpan:
    start: tuple[float, float]
    end: tuple[float, float]


@dataclass(frozen=True)
class WidthConsistency:
    relative_tolerance: float
    accepted_measurements: int
    total_measurements: int
    fraction: float


@dataclass(frozen=True)
class ScaleAnalysis:
    ridge_width_px: float
    valley_width_px: float
    ridge_period_px: float
    ridge_width_mad_px: float
    ridge_period_mad_px: float
    mean_ridge_width_px: float
    mean_valley_width_px: float
    mean_ridge_gray: float
    mean_valley_gray: float
    examined_locations: int
    profiles: tuple[tuple[RidgeSpan, ...], ...]
    widths: tuple[float, ...]
    periods: tuple[float, ...]
    ridge_grays: tuple[float, ...]
    valley_grays: tuple[float, ...]
    width_consistency: tuple[WidthConsistency, ...]


def validate_scale(value, width: int, height: int, max_profiles) -> ScaleAnalysis:
    """Reconstruct all measurements; reject summaries inconsistent with evidence.

    Width and period MADs are unscaled median absolute deviations. Every accepted
    triple contributes three widths, two gaps and two center spacings. No values
    are removed before medians or consistency fractions are calculated.
    """
    if not isinstance(value, dict):
        raise ValueError("scale must be an object")
    _count(max_profiles, "config.bootstrap_grid_locations", 1, _MAX_PROFILES)
    # The initial grid is bounded by its configured location budget. If it
    # finds no triple, bootstrap may also attempt each source pixel once.
    examined = _count(
        value.get("examined_locations"),
        "bootstrap.examined_locations",
        1,
        width * height + max_profiles,
    )
    raw_profiles = value.get("profiles")
    if not isinstance(raw_profiles, list) or not 1 <= len(raw_profiles) <= min(
        examined, max_profiles
    ):
        raise ValueError("bootstrap.profiles exceeds examined locations or the initial grid budget")

    profiles = tuple(_profile(profile, width, height) for profile in raw_profiles)
    widths, gaps, periods = _measure_profiles(profiles)
    ridge_grays = _gray_means(raw_profiles, "ridge_mean_gray", 3)
    valley_grays = _gray_means(raw_profiles, "valley_mean_gray", 2)
    expected = {
        "ridge_width_px": statistics.median(widths),
        "valley_width_px": statistics.median(gaps),
        "ridge_period_px": statistics.median(periods),
        "mean_ridge_width_px": statistics.fmean(widths),
        "mean_valley_width_px": statistics.fmean(gaps),
        "mean_ridge_gray": _weighted_mean(ridge_grays, widths),
        "mean_valley_gray": _weighted_mean(valley_grays, gaps),
    }
    expected["ridge_width_mad_px"] = _mad(widths, expected["ridge_width_px"])
    expected["ridge_period_mad_px"] = _mad(periods, expected["ridge_period_px"])
    declared = {}
    for name, measurement in expected.items():
        actual = finite_number(value.get(name), f"scale.{name}")
        if actual < 0 or (name.endswith("_px") and "mad" not in name and actual == 0):
            raise ValueError(f"scale.{name} must be positive, or nonnegative for MAD")
        if name.endswith("_gray") and actual > 255:
            raise ValueError(f"scale.{name} must be in [0, 255]")
        if not math.isclose(actual, measurement, rel_tol=1e-8, abs_tol=1e-7):
            raise ValueError(f"scale.{name} does not match all supplied profiles")
        declared[name] = actual

    consistency = validate_width_consistency(
        value.get("width_consistency"), widths, declared["ridge_width_px"]
    )
    return ScaleAnalysis(
        **declared,
        examined_locations=examined,
        profiles=profiles,
        widths=widths,
        periods=periods,
        ridge_grays=ridge_grays,
        valley_grays=valley_grays,
        width_consistency=consistency,
    )


def _gray_means(profiles, name, count):
    values = []
    for profile in profiles:
        supplied = profile.get(name)
        if not isinstance(supplied, list) or len(supplied) != count:
            raise ValueError(f"scale profile {name} must contain exactly {count} means")
        for raw in supplied:
            value = finite_number(raw, f"scale profile {name}")
            if not 0 <= value <= 255:
                raise ValueError(f"scale profile {name} must be in [0, 255]")
            values.append(value)
    return tuple(values)


def _weighted_mean(values, lengths):
    return math.fsum(
        value * length for value, length in zip(values, lengths, strict=True)
    ) / math.fsum(lengths)


def _count(value, name, minimum, maximum):
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError(f"{name} must be an integer in [{minimum}, {maximum}]")
    return value


def _point(value, width, height):
    if not isinstance(value, list) or len(value) != 2:
        raise ValueError("scale span endpoints must contain x and y")
    x, y = (finite_number(coordinate, "scale endpoint") for coordinate in value)
    if not 0 <= x <= width - 1 or not 0 <= y <= height - 1:
        raise ValueError("scale endpoint lies outside source pixel centers")
    return x, y


def _profile(value, width, height):
    if not isinstance(value, dict) or not isinstance(value.get("ridges"), list):
        raise ValueError("scale profile must contain ridge spans")
    if len(value["ridges"]) != 3:
        raise ValueError("every scale profile must contain exactly three ridge spans")
    spans = []
    for span in value["ridges"]:
        if not isinstance(span, dict):
            raise ValueError("scale ridge span must be an object")
        start = _point(span.get("start"), width, height)
        end = _point(span.get("end"), width, height)
        if math.dist(start, end) == 0:
            raise ValueError("scale ridge spans must have positive width")
        spans.append(RidgeSpan(start, end))

    origin = spans[0].start
    length = math.dist(origin, spans[0].end)
    ux, uy = ((spans[0].end[axis] - origin[axis]) / length for axis in (0, 1))
    # Coordinate arithmetic allowance only, far below one source pixel.
    precision = 64 * sys.float_info.epsilon * max(width, height, 1)
    previous = None
    for span in spans:
        projections = []
        for point in (span.start, span.end):
            dx, dy = point[0] - origin[0], point[1] - origin[1]
            if abs(dx * uy - dy * ux) > precision:
                raise ValueError("scale profile spans must be collinear")
            projections.append(dx * ux + dy * uy)
        if projections[1] <= projections[0] or (
            previous is not None and projections[0] <= previous
        ):
            raise ValueError("scale profile spans must be ordered with positive widths and gaps")
        previous = projections[1]
    return tuple(spans)


def _measure_profiles(profiles):
    widths, gaps, periods = [], [], []
    for spans in profiles:
        widths.extend(math.dist(span.start, span.end) for span in spans)
        centers = [tuple((a + b) / 2 for a, b in zip(span.start, span.end)) for span in spans]
        gaps.extend(math.dist(a.end, b.start) for a, b in zip(spans, spans[1:]))
        periods.extend(math.dist(a, b) for a, b in zip(centers, centers[1:]))
    return tuple(widths), tuple(gaps), tuple(periods)


def _mad(values, median):
    return statistics.median(abs(value - median) for value in values)


def _within_width_band(width, median, tolerance):
    # Matches Rust's inclusive boundary with a floating-point roundoff allowance.
    slack = 8 * sys.float_info.epsilon * max(width, median)
    return abs(width - median) <= tolerance * median + slack


def validate_width_consistency(value, widths, median):
    """Validate count-based width bands around the supplied reference width."""
    if not isinstance(value, list) or len(value) != len(_TOLERANCES):
        raise ValueError("width_consistency must contain the 10%, 20%, and 30% bands")
    result = []
    total = len(widths)
    for raw, tolerance in zip(value, _TOLERANCES, strict=True):
        if (
            not isinstance(raw, dict)
            or finite_number(raw.get("relative_tolerance"), "tolerance") != tolerance
        ):
            raise ValueError("width_consistency bands must be ordered as 0.1, 0.2, 0.3")
        supplied_total = _count(raw.get("total_measurements"), "total_measurements", total, total)
        accepted = _count(raw.get("accepted_measurements"), "accepted_measurements", 0, total)
        expected = sum(_within_width_band(width, median, tolerance) for width in widths)
        if accepted != expected:
            raise ValueError("width consistency count does not match all width observations")
        fraction = finite_number(raw.get("fraction"), "width consistency fraction")
        if not 0 <= fraction <= 1 or not math.isclose(
            fraction, accepted / total, rel_tol=1e-12, abs_tol=1e-12
        ):
            raise ValueError("width consistency fraction does not match accepted / total")
        result.append(WidthConsistency(tolerance, accepted, supplied_total, fraction))
    return tuple(result)


def display_profiles(scale: ScaleAnalysis, width: int, height: int):
    """Select at most one measured triple per cell of a fixed 6×6 display grid."""
    selected = {}
    for profile in scale.profiles:
        middle = profile[1]
        x, y = ((a + b) / 2 for a, b in zip(middle.start, middle.end))
        column, row = min(5, int(6 * x / width)), min(5, int(6 * y / height))
        distance = math.hypot(x - (column + 0.5) * width / 6, y - (row + 0.5) * height / 6)
        key = row, column
        if key not in selected or distance < selected[key][0]:
            selected[key] = distance, profile
    return tuple(selected[key][1] for key in sorted(selected))


def render_scale_analysis(scale: ScaleAnalysis, image, alpha: float = 0.5):
    """Build an evidence figure; only the overlay is spatially subsampled."""
    figure = plt.figure(figsize=(17, 12), layout="constrained")
    try:
        grid = figure.add_gridspec(3, 3, width_ratios=(1.45, 1, 1), height_ratios=(1, 0.9, 1.1))
        overlay = figure.add_subplot(grid[:, 0])
        _draw_profiles(overlay, scale, image, alpha)
        _histogram(
            figure.add_subplot(grid[0, 1]),
            scale.widths,
            scale.ridge_width_px,
            scale.ridge_width_mad_px,
            "Ridge width",
            "Ridge width (source px)",
        )
        _histogram(
            figure.add_subplot(grid[0, 2]),
            scale.periods,
            scale.ridge_period_px,
            scale.ridge_period_mad_px,
            "Ridge period",
            "Adjacent center spacing (source px)",
        )
        _draw_gray_distribution(figure.add_subplot(grid[1, 1:]), scale)
        _consistency_table(figure.add_subplot(grid[2, 1:]), scale)
        figure.suptitle("Bootstrap scale · sampled triples and source grayscale", fontsize=16)
        return figure
    except Exception:
        plt.close(figure)
        raise


def _draw_profiles(axis, scale, image, alpha):
    width, height = image.size
    extent = (-0.5, width - 0.5, height - 0.5, -0.5)
    axis.imshow(
        image, cmap="gray", vmin=0, vmax=255, extent=extent, alpha=1, interpolation="nearest"
    )
    selected = display_profiles(scale, width, height)
    axis.add_collection(
        LineCollection(
            [(p[0].start, p[-1].end) for p in selected],
            colors="#e69f00",
            linewidths=0.6,
            alpha=alpha * 0.8,
        )
    )
    axis.add_collection(
        LineCollection(
            [(span.start, span.end) for p in selected for span in p],
            colors="#00a6b2",
            linewidths=2,
            alpha=alpha,
        )
    )
    axis.set(
        xlim=extent[:2],
        ylim=extent[2:],
        aspect="equal",
        xlabel="x (source px)",
        ylabel="y (source px)",
    )
    axis.set_title(
        f"{len(selected)} of {len(scale.profiles)} measured triples shown\n"
        "Uniform spatial selection for display",
        fontsize=11,
    )
    axis.legend(
        handles=[
            Line2D([], [], color="#00a6b2", lw=2, label="Measured dark span"),
            Line2D([], [], color="#e69f00", lw=0.6, label="Triple sampling line"),
        ],
        loc="upper center",
        bbox_to_anchor=(0.5, -0.09),
        frameon=False,
        fontsize=9,
    )


def _histogram(axis, values, median, mad, title, label):
    bins = min(30, max(1, math.ceil(math.sqrt(len(values)))))
    axis.hist(values, bins=bins, color="#4477aa", edgecolor="white", linewidth=0.5)
    axis.axvline(median, color="#d55e00", linestyle="--", label=f"Median {median:.4g} px")
    axis.set(
        title=f"{title} · all {len(values)} measurements\nUnscaled MAD {mad:.4g} px",
        xlabel=label,
        ylabel="Measurements",
    )
    axis.yaxis.set_major_locator(MaxNLocator(integer=True))
    axis.grid(axis="y", alpha=0.2)
    axis.set_axisbelow(True)
    axis.legend(fontsize=9)


def _consistency_table(axis, scale):
    axis.set_axis_off()
    axis.text(
        0,
        0.99,
        "Thickness consistency around the global median\n"
        f"{len(scale.profiles)} accepted triples · {scale.examined_locations} locations examined\n"
        f"Median width {scale.ridge_width_px:.4g} px · valley {scale.valley_width_px:.4g} px · "
        f"period {scale.ridge_period_px:.4g} px\n"
        f"Arithmetic mean width {scale.mean_ridge_width_px:.4g} px · "
        f"valley {scale.mean_valley_width_px:.4g} px",
        transform=axis.transAxes,
        va="top",
        fontsize=11,
        linespacing=1.5,
    )
    rows = [
        [
            f"±{row.relative_tolerance:.0%}",
            str(row.accepted_measurements),
            str(row.total_measurements),
            f"{row.fraction:.2%}",
        ]
        for row in scale.width_consistency
    ]
    table = axis.table(
        cellText=rows,
        colLabels=("Width band", "Within band", "All measurements", "Share"),
        cellLoc="center",
        bbox=(0, 0.28, 1, 0.36),
    )
    table.auto_set_font_size(False)
    table.set_fontsize(10)
    for (row, _), cell in table.get_celld().items():
        cell.set_edgecolor("#d6dce2")
        if row == 0:
            cell.set_facecolor("#e8eef3")
            cell.set_text_props(weight="bold")
    axis.text(
        0,
        0.18,
        "All three spans per triple contribute; no widths are removed.\n"
        "These fractions describe thickness consistency, not calibrated capture quality.\n"
        "Measurements may revisit physical ridges; they are not distinct ridge counts.",
        transform=axis.transAxes,
        va="top",
        fontsize=9,
        linespacing=1.6,
    )


def _draw_gray_distribution(axis, scale):
    bins = tuple(range(0, 257, 8))
    axis.hist(
        [scale.ridge_grays, scale.valley_grays],
        bins=bins,
        color=("#4477aa", "#ee9933"),
        label=("Dark span means", "Light gap means"),
        histtype="step",
        linewidth=1.5,
    )
    for mean, color in ((scale.mean_ridge_gray, "#4477aa"), (scale.mean_valley_gray, "#ee9933")):
        axis.axvline(mean, color=color, linestyle="--", linewidth=1)
    axis.set(
        xlim=(0, 255),
        xlabel="Raw source grayscale (0 = black, 255 = white)",
        ylabel="Span observations",
        title=f"All {len(scale.ridge_grays)} dark / {len(scale.valley_grays)} light means · "
        "dashed lines: length-weighted means",
    )
    axis.yaxis.set_major_locator(MaxNLocator(integer=True))
    axis.legend(loc="upper left", fontsize=9)
    axis.grid(axis="y", alpha=0.2)
    axis.set_axisbelow(True)
    for index, (name, mean) in enumerate(
        (("Dark", scale.mean_ridge_gray), ("Light", scale.mean_valley_gray))
    ):
        y = 0.9 - index * 0.15
        axis.add_patch(
            Rectangle(
                (0.67, y - 0.035),
                0.045,
                0.07,
                transform=axis.transAxes,
                facecolor=str(mean / 255),
                edgecolor="#444444",
                linewidth=0.7,
            )
        )
        axis.text(
            0.73, y, f"{name} mean {mean:.4g}", transform=axis.transAxes, va="center", fontsize=9
        )
