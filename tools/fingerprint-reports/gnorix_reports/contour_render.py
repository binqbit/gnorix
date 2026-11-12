"""Draw contour reconstruction and retained source evidence."""

import textwrap

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
from matplotlib.patches import Rectangle
from matplotlib.ticker import MaxNLocator

from .clarity import OPACITY_LABEL, UNKNOWN_COLOR, clarity_lines
from .contours import Contours

_RAW_COLOR = "#a34981"
_SMOOTH_COLOR = "#00a6b2"
_JOIN_COLOR = "#009e73"
_CANDIDATE_COLOR = "#e69f00"
_REMOVED_COLOR = "#d55e00"


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
