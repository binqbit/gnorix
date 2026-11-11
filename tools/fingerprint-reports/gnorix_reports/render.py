"""Static figures for supplied inspection evidence; no biometric computation."""

from __future__ import annotations

import math
import re
import sys
import textwrap
from pathlib import Path

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.colors import Normalize
from matplotlib.ticker import MaxNLocator
from mpl_toolkits.axes_grid1 import make_axes_locatable
from PIL import Image

from .images import load_image
from .model import Frame, Metric, Raster, Series, Snapshot, Stage, metric_domain
from .options import OverlayOptions
from .overlay import description as overlay_description
from .overlay import draw_annotations, point_groups, view_bounds

_STYLE = {
    "font.family": "DejaVu Sans",
    "font.size": 10,
    "axes.titlesize": 12,
    "axes.labelsize": 10,
    "axes.edgecolor": "#a3adb9",
    "axes.labelcolor": "#233247",
    "text.color": "#233247",
    "xtick.color": "#526174",
    "ytick.color": "#526174",
    "grid.color": "#dce2e8",
    "grid.linewidth": 0.6,
    "figure.facecolor": "white",
    "savefig.facecolor": "white",
    "text.usetex": False,
    "text.parse_math": False,
}


def render_snapshot(
    snapshot: Snapshot,
    output: Path,
    dpi: int,
    metric_keys: tuple[str, ...] | None = None,
    *,
    overlay: OverlayOptions | None = None,
    overlay_only: bool = False,
) -> list[Path]:
    """Render every selected stage/metric into PNG reports and overlays.

    An unfiltered stage without metrics still receives a report. A metric filter
    selects matching stages; unknown keys fail before creating output. The caller
    can supply a temporary directory and publish the complete directory atomically.
    """
    if isinstance(dpi, bool) or not isinstance(dpi, int) or dpi <= 0:
        raise ValueError("report DPI must be a positive integer")

    selected = _select_reports(snapshot, metric_keys)
    _validate_plot_ranges(selected)
    frames = {frame.id: frame for frame in snapshot.frames}
    options = overlay or OverlayOptions()
    for _, stage, _, _ in selected:
        options.validate_frame(frames[stage.frame])
    images: dict[str, Image.Image | None] = {}
    paths: list[Path] = []

    try:
        # Decode once per selected frame, before writing partial report output.
        for _, stage, _, _ in selected:
            if stage.frame not in images:
                images[stage.frame] = _load_image(frames[stage.frame])

        output.mkdir(parents=True, exist_ok=True)
        with matplotlib.rc_context(_STYLE):
            for stage_index, stage, metric_index, metric in selected:
                arguments = (
                    snapshot,
                    frames[stage.frame],
                    stage,
                    metric,
                    images[stage.frame],
                    options,
                )
                stem = _report_stem(stage_index, stage, metric_index, metric)
                if not overlay_only:
                    paths.append(_save_figure(_render_report(*arguments), output, stem, dpi))
                paths.append(
                    _save_figure(_render_overlay(*arguments), output, stem + "-overlay", dpi)
                )
    finally:
        for image in images.values():
            if image is not None:
                image.close()

    return paths


def _save_figure(figure, output: Path, stem: str, dpi: int) -> Path:
    path = output / f"{stem}.png"
    try:
        figure.savefig(path, dpi=dpi, bbox_inches="tight", pad_inches=0.12)
    finally:
        plt.close(figure)
    return path


def _select_reports(
    snapshot: Snapshot, metric_keys: tuple[str, ...] | None
) -> list[tuple[int, Stage, int, Metric | None]]:
    if metric_keys is not None:
        available = {metric.key for stage in snapshot.stages for metric in stage.metrics}
        unknown = set(metric_keys) - available
        if unknown:
            raise ValueError("unknown metric keys: " + ", ".join(sorted(unknown)))
        if not metric_keys:
            raise ValueError("metric filter must not be empty")

    reports = []
    for stage_index, stage in enumerate(snapshot.stages, start=1):
        for metric_index, metric in enumerate(stage.metrics, start=1):
            if metric_keys is None or metric.key in metric_keys:
                reports.append((stage_index, stage, metric_index, metric))
        if not stage.metrics and metric_keys is None:
            reports.append((stage_index, stage, 0, None))

    return reports


def _load_image(frame: Frame) -> Image.Image | None:
    if frame.image is None:
        return None

    image = load_image(frame.image, expected_size=(frame.width, frame.height))
    try:
        return image.convert("RGBA")
    finally:
        image.close()


def _validate_plot_ranges(reports: list[tuple[int, Stage, int, Metric | None]]) -> None:
    """Reserve float arithmetic headroom for plotting, never alter input values."""
    for _, stage, _, metric in reports:
        if metric is not None:
            values = [
                point.values[metric.key]
                for point in stage.points
                if point.valid and point.values.get(metric.key) is not None
            ]
            domain = metric_domain(stage, metric.key)
            _check_plot_range(values + list(domain or ()), f"metric {metric.key!r}")
        if stage.raster is not None:
            raster = stage.raster
            values = [value for value in raster.values if value is not None]
            _check_plot_range(values + list(raster.domain or ()), f"raster in stage {stage.id!r}")
        for series in stage.series:
            _check_plot_range([point.x for point in series.points], f"series {series.id!r} x axis")
            _check_plot_range([point.y for point in series.points], f"series {series.id!r} y axis")


def _check_plot_range(values: list[float], label: str) -> None:
    if values and (
        not math.isfinite(max(values) - min(values))
        or max(abs(value) for value in values) > sys.float_info.max / 100
    ):
        raise ValueError(f"{label}: magnitude exceeds safe floating-point plotting range")


def _render_report(
    snapshot: Snapshot,
    frame: Frame,
    stage: Stage,
    metric: Metric | None,
    image: Image.Image | None,
    overlay: OverlayOptions | None = None,
):
    options = overlay or OverlayOptions()
    extras = ([stage.raster] if stage.raster is not None else []) + list(stage.series)
    extra_rows = math.ceil(len(extras) / 2)
    figure = plt.figure(figsize=(12.5, 7.2 + 3.5 * extra_rows), layout="constrained")

    try:
        grid = figure.add_gridspec(
            1 + extra_rows, 2, width_ratios=(1.45, 1), height_ratios=[1.5] + [1] * extra_rows
        )
        title = f"{stage.label} · {_metric_label(metric)}" if metric else stage.label
        figure.suptitle(
            textwrap.fill(snapshot.title, 105) + "\n" + textwrap.fill(title, 105),
            fontsize=15,
            x=0.04,
            ha="left",
        )

        overlay_axis = figure.add_subplot(grid[0, 0])
        histogram = figure.add_subplot(grid[0, 1])
        _draw_overlay(figure, overlay_axis, frame, stage, metric, image, options)
        _draw_histogram(histogram, stage, metric)

        for index, extra in enumerate(extras):
            axis = figure.add_subplot(grid[1 + index // 2, index % 2])
            if isinstance(extra, Raster):
                _draw_raster(figure, axis, frame, extra)
            else:
                _draw_series(axis, extra)

        description = " ".join(item for item in (snapshot.description, stage.description) if item)
        footer = (
            f"Frame: {frame.label} · {frame.width} × {frame.height} px. "
            "Coordinates refer to the original image."
        )
        if description:
            footer += "\n" + textwrap.fill(description, 150)
        footer += "\n" + textwrap.fill(overlay_description(stage, options), 150)
        figure.supxlabel(footer, fontsize=9, color="#526174", ha="left", x=0.04)
        return figure
    except BaseException:
        plt.close(figure)
        raise


def _render_overlay(
    snapshot: Snapshot,
    frame: Frame,
    stage: Stage,
    metric: Metric | None,
    image: Image.Image | None,
    overlay: OverlayOptions | None = None,
):
    options = overlay or OverlayOptions()
    _, _, width, height = view_bounds(frame, options)
    image_width = min(11.0, 13.0 * width / height)
    image_height = image_width * height / width
    figure = plt.figure(
        figsize=(max(7, image_width + 2), max(5, image_height + 2.5)), layout="constrained"
    )
    try:
        title = f"{stage.label} · {_metric_label(metric)}" if metric else stage.label
        figure.suptitle(
            textwrap.fill(snapshot.title, 105) + "\n" + textwrap.fill(title, 105),
            fontsize=15,
            x=0.04,
            ha="left",
        )
        axis = figure.add_subplot(111)
        _draw_overlay(figure, axis, frame, stage, metric, image, options)
        footer = (
            f"Frame: {frame.label} · {frame.width} × {frame.height} px. "
            + overlay_description(stage, options)
        )
        if metric is not None:
            scope = "explicit" if metric.domain is not None else "stage"
            measured, missing, rejected = point_groups(stage, metric)
            footer += (
                f" {scope} color domain: {_domain_label(metric_domain(stage, metric.key))}. "
                f"Full stage: {len(measured)} measured · {len(missing)} missing · "
                f"{len(rejected)} rejected."
            )
        figure.supxlabel(textwrap.fill(footer, 135), fontsize=9, color="#526174", ha="left", x=0.04)
        return figure
    except BaseException:
        plt.close(figure)
        raise


def _draw_overlay(
    figure,
    axis,
    frame: Frame,
    stage: Stage,
    metric: Metric | None,
    image: Image.Image | None,
    overlay: OverlayOptions | None = None,
) -> None:
    options = overlay or OverlayOptions()
    options.validate_frame(frame)
    _frame_axes(axis, frame)
    axis.set_title("Image and sampling points", loc="left")
    if image is not None:
        axis.imshow(
            image,
            extent=(0, frame.width, frame.height, 0),
            interpolation="nearest",
            alpha=1,
            zorder=0,
        )
    else:
        axis.text(
            0.02,
            0.98,
            "No source image supplied",
            transform=axis.transAxes,
            va="top",
            color="#526174",
            fontsize=9,
        )

    domain = metric_domain(stage, metric.key) if metric is not None else None
    plot, values = draw_annotations(axis, frame, stage, metric, options, _normalization(domain))
    if metric is not None and plot is not None:
        _colorbar(figure, axis, plot, domain, values, _metric_label(metric))
    x, y, width, height = view_bounds(frame, options)
    axis.set_xlim(x, x + width)
    axis.set_ylim(y + height, y)
    if not stage.points:
        axis.set_title("Source image\nNo sampling points supplied", loc="left", fontsize=11)


def _draw_histogram(axis, stage: Stage, metric: Metric | None) -> None:
    measured, missing, rejected = point_groups(stage, metric)
    axis.set_title("Value distribution", loc="left")
    if metric is None:
        _informational(
            axis, f"No point metric supplied\n{len(stage.points)} points · {len(rejected)} rejected"
        )
        return

    domain = metric_domain(stage, metric.key)
    scope = "explicit color domain" if metric.domain is not None else "stage color domain"
    range_text = _domain_label(domain)
    if domain is not None and domain[0] == domain[1]:
        range_text += " (constant; display scale expanded)"
    axis.set_title(
        f"Value distribution\n{len(measured)} measured · "
        f"{len(missing)} missing · {len(rejected)} rejected",
        loc="left",
        fontsize=11,
    )
    axis.set_xlabel(
        _metric_label(metric) + "\n" + textwrap.fill(f"{scope}: {range_text}", 58),
        fontsize=9,
        linespacing=1.6,
    )
    axis.set_ylabel("Point count")

    if not measured:
        _informational(axis, "No valid measured values")
        return

    values = [point.values[metric.key] for point in measured]
    bins = min(30, max(1, math.ceil(math.sqrt(len(values)))))
    axis.hist(values, bins=bins, color="#16758a", edgecolor="white", linewidth=0.6)
    axis.set_ylim(bottom=0)
    axis.yaxis.set_major_locator(MaxNLocator(integer=True, nbins=5))
    axis.xaxis.set_major_locator(MaxNLocator(nbins=5))
    axis.grid(axis="y", zorder=0)
    axis.set_axisbelow(True)
    axis.spines[["top", "right"]].set_visible(False)


def _draw_raster(figure, axis, frame: Frame, raster: Raster) -> None:
    _frame_axes(axis, frame)
    axis.set_title(textwrap.fill(raster.label, 65), loc="left")
    finite = [value for value in raster.values if value is not None]
    if not finite:
        _informational(axis, "No raster values supplied")
        return

    domain = raster.domain if raster.domain is not None else (min(finite), max(finite))
    values = np.array([np.nan if value is None else value for value in raster.values]).reshape(
        raster.rows, raster.columns
    )
    palette = matplotlib.colormaps["cividis"].with_extremes(bad=(0, 0, 0, 0))
    plot = axis.imshow(
        np.ma.masked_invalid(values),
        extent=(raster.x, raster.x + raster.width, raster.y + raster.height, raster.y),
        cmap=palette,
        norm=_normalization(domain),
        interpolation="nearest",
    )
    # imshow can autoscale the axes to the raster; restore the complete frame.
    axis.set_xlim(0, frame.width)
    axis.set_ylim(frame.height, 0)
    _colorbar(figure, axis, plot, domain, finite, raster.label)
    scope = "explicit" if raster.domain is not None else "raster"
    axis.set_xlabel(
        "x (px)\n"
        + textwrap.fill(
            f"{scope} color domain: {_domain_label(domain)} · "
            f"{len(raster.values) - len(finite)} missing cells (uncolored)",
            58,
        ),
        fontsize=9,
        linespacing=1.6,
    )


def _draw_series(axis, series: Series) -> None:
    axis.set_title(textwrap.fill(series.label, 65), loc="left")
    axis.set_xlabel(series.x_label)
    axis.set_ylabel(series.y_label)
    if not series.points:
        _informational(axis, "No series values supplied")
        return

    axis.plot(
        [point.x for point in series.points],
        [point.y for point in series.points],
        color="#16758a",
        linewidth=1.6,
        marker="o" if len(series.points) <= 40 else None,
        markersize=3,
    )
    axis.grid(True)
    axis.xaxis.set_major_locator(MaxNLocator(nbins=5))
    axis.yaxis.set_major_locator(MaxNLocator(nbins=5))
    axis.spines[["top", "right"]].set_visible(False)


def _normalization(domain: tuple[float, float] | None) -> Normalize:
    if domain is None:
        return Normalize(vmin=0, vmax=1)
    low, high = domain
    if low == high:
        padding = max(abs(low) * 0.05, 0.5)
        lower, upper = low - padding, high + padding
        low = lower if math.isfinite(lower) else low
        high = upper if math.isfinite(upper) else high
    return Normalize(vmin=low, vmax=high)


def _colorbar(figure, axis, plot, domain: tuple[float, float], values: list, label: str) -> None:
    below = any(value < domain[0] for value in values)
    above = any(value > domain[1] for value in values)
    extend = "both" if below and above else "min" if below else "max" if above else "neither"
    divider = make_axes_locatable(axis)
    color_axis = divider.append_axes("right", size="4%", pad=0.10)
    bar = figure.colorbar(plot, cax=color_axis, extend=extend)
    bar.set_label(textwrap.fill(label, 45), fontsize=9)
    if domain[0] == domain[1]:
        bar.set_ticks([domain[0]])


def _frame_axes(axis, frame: Frame) -> None:
    axis.set_facecolor("#eef1f4")
    axis.set_xlim(0, frame.width)
    axis.set_ylim(frame.height, 0)
    axis.set_aspect("equal")
    axis.set_xlabel("x (px)")
    axis.set_ylabel("y (px)")
    axis.xaxis.set_major_locator(MaxNLocator(nbins=5))
    axis.yaxis.set_major_locator(MaxNLocator(nbins=5))


def _informational(axis, message: str) -> None:
    axis.text(
        0.5,
        0.5,
        message,
        transform=axis.transAxes,
        ha="center",
        va="center",
        fontsize=11,
        color="#526174",
    )
    axis.set_xticks([])
    axis.set_yticks([])
    axis.spines[:].set_visible(False)


def _metric_label(metric: Metric) -> str:
    return f"{metric.label} ({metric.unit})" if metric.unit else metric.label


def _domain_label(domain: tuple[float, float] | None) -> str:
    return "unavailable" if domain is None else f"[{domain[0]:.6g}, {domain[1]:.6g}]"


def _report_stem(stage_index: int, stage: Stage, metric_index: int, metric: Metric | None) -> str:
    stage_slug = _slug(stage.id)
    suffix = f"{metric_index:02d}-{_slug(metric.key)}" if metric is not None else "overview"
    return f"{stage_index:02d}-{stage_slug}-{suffix}"


def _slug(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")[:64] or "item"
