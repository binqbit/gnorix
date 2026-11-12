"""Draw supplied final graph features and original processing diagnostics."""

import math

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
from matplotlib.ticker import MaxNLocator

from .clarity import OPACITY_LABEL, UNKNOWN_COLOR, clarity_lines
from .feature_strength_render import (
    MARKER_OPACITY_LABEL,
    add_priority_colorbar,
    draw_feature_markers,
    feature_legend_handles,
)
from .snapshot import PROCESSING_STAGES


def _image_axes(axis, image, snapshot):
    extent = (-0.5, snapshot.width - 0.5, snapshot.height - 0.5, -0.5)
    axis.imshow(
        image, cmap="gray", vmin=0, vmax=255, extent=extent, alpha=1, interpolation="nearest"
    )
    axis.set(xlim=extent[:2], ylim=extent[2:], xlabel="x (source px)", ylabel="y (source px)")
    axis.set_aspect("equal")
    axis.xaxis.set_major_locator(MaxNLocator(4))
    axis.yaxis.set_major_locator(MaxNLocator(4))


def render_overview(snapshot, images):
    height = min(16, max(6, 6.5 * snapshot.height / snapshot.width + 1.5))
    figure, axes = plt.subplots(2, 4, figsize=(16, height), layout="constrained")
    try:
        for axis, (key, label) in zip(axes.flat[:-1], PROCESSING_STAGES, strict=True):
            _image_axes(axis, images[key], snapshot)
            axis.set_title(label)
        summary = axes.flat[-1]
        summary.set_axis_off()
        mean = snapshot.clarity.mean_score
        mean_text = "unavailable" if mean is None else f"{mean:.4f}"
        feature_count = len(snapshot.feature_strength.candidates)
        measured = sum(c.priority is not None for c in snapshot.feature_strength.candidates)
        summary.text(
            0.1,
            0.95,
            "Extracted graph topology\n\n"
            f"Nodes: {len(snapshot.nodes)}\n"
            f"Paths: {len(snapshot.edges)}\n"
            f"Graph cycle rank: {snapshot.graph_cycle_rank}\n"
            f"Isolated cycles: {len(snapshot.cycles)}\n\n"
            f"Described path length: {_described_length(snapshot):.6g} R\n\n"
            "Cycle rank counts independent graph cycles.\n"
            "Image coordinates use source pixels.\n"
            "Described path length is expressed in R.\n\n"
            f"Clarity mean: {mean_text}\n"
            f"Clarity support: {snapshot.clarity.supported_pixels} / "
            f"{snapshot.clarity.total_pixels} pixels\n"
            "Includes supported background.\n"
            "Diagnostic, not matching probability.\n\n"
            f"Feature candidates: {feature_count}\n"
            f"Measured priority: {measured} / {feature_count}\n"
            "Feature strength: feature-strength.png\n"
            "All original candidates remain visible.",
            transform=summary.transAxes,
            ha="left",
            va="top",
            fontsize=10,
            linespacing=1.25,
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


def render_structure_overlay(snapshot, images):
    height = min(16, max(7, 9 * snapshot.height / snapshot.width))
    figure, axis = plt.subplots(figsize=(11, height), layout="constrained")
    try:
        _image_axes(axis, images["source"], snapshot)
        _draw_final_paths(axis, snapshot.contours.paths)
        markers = snapshot.final_structure.markers
        for boundary in (False, True):
            selected = [item for item in markers if item.touches_unobserved == boundary]
            draw_feature_markers(axis, selected, 30, hollow=boundary)
        add_priority_colorbar(figure, axis)
        _graph_legend(figure)
        counts = {
            kind: sum(marker.kind == kind for marker in markers)
            for kind in ("ending", "bifurcation", "junction", "isolated")
        }
        closed = sum(path.closed for path in snapshot.contours.paths)
        unassessed = sum(marker.unassessed for marker in markers)
        boundary = sum(marker.touches_unobserved for marker in markers)
        figure.suptitle(
            "Cleaned + refined ridge graph · surviving features\n"
            f"Full mean ridge width: 1 R = {snapshot.scale.unit_px:.4g} px · "
            f"bootstrap period: {snapshot.bootstrap.ridge_period_px:.4g} px\n"
            f"Graph cycle rank: {snapshot.final_structure.cycle_rank} · "
            f"closed contours: {closed} · "
            f"open paths: {len(snapshot.contours.paths) - closed}\n"
            f"Endings: {counts['ending']} · bifurcations: {counts['bifurcation']} · "
            f"other junctions: {counts['junction']} · "
            f"isolated nodes: {counts['isolated']}\n"
            f"Unobserved boundary: {boundary} · changed features not rescored: {unassessed} · "
            f"new closed contours without priority: {snapshot.final_structure.unassessed_cycles}",
            fontsize=12,
        )
        if not markers and not snapshot.contours.paths:
            axis.text(
                0.5,
                0.5,
                "No retained graph structure",
                transform=axis.transAxes,
                ha="center",
                va="center",
                bbox={"facecolor": "white", "alpha": 0.9},
            )
        axis.set_xlabel(
            "x (source px)\n"
            + OPACITY_LABEL
            + "\n"
            + MARKER_OPACITY_LABEL
            + "\nPriority uses original evidence; changed feature types/degrees are gray."
            + "\nAll original candidates and scores: feature-strength.png",
            fontsize=9,
        )
        return figure
    except Exception:
        plt.close(figure)
        raise


def _draw_final_paths(axis, curves):
    for closed in (False, True):
        paths = [curve for curve in curves if curve.closed == closed]
        if paths:
            axis.add_collection(
                clarity_lines(
                    ((curve.points, closed, curve.segment_clarity) for curve in paths),
                    "#a44cad" if closed else "#00a6b2",
                    linewidths=0.9,
                    linestyles="dashed" if closed else "solid",
                    zorder=3,
                )
            )


def _graph_legend(figure):
    handles = [
        Line2D([], [], color="#00a6b2", label="Cleaned + refined path"),
        Line2D([], [], color="#a44cad", linestyle="--", label="Final closed contour"),
        Line2D([], [], color=UNKNOWN_COLOR, alpha=0.4, label="Clarity unavailable"),
    ]
    handles.extend(feature_legend_handles())
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
