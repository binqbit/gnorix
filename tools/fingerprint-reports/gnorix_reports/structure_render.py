"""Draw supplied observed graphs and processing stages without recomputing them."""

import math
import textwrap

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
from matplotlib.ticker import MaxNLocator

from .clarity import OPACITY_LABEL, UNKNOWN_COLOR, clarity_lines
from .snapshot import PROCESSING_STAGES

_KINDS = {
    "ending": ("o", "#0072b2", "Ending candidate"),
    "junction": ("^", "#d55e00", "Junction candidate"),
    "isolated": ("s", "#8a4c99", "Isolated node"),
}


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
        measured = sum(c.strength is not None for c in snapshot.feature_strength.candidates)
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
            f"Measured Q: {measured} / {feature_count}\n"
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


def render_structure_overlay(snapshot, images, alpha):
    height = min(16, max(7, 9 * snapshot.height / snapshot.width))
    figure, axis = plt.subplots(figsize=(11, height), layout="constrained")
    try:
        _image_axes(axis, images["source"], snapshot)
        _draw_curves(axis, snapshot.edges, False)
        _draw_curves(axis, snapshot.cycles, True)
        _draw_nodes(axis, snapshot.nodes, alpha)
        _graph_legend(figure)
        counts = {kind: sum(node.kind == kind for node in snapshot.nodes) for kind in _KINDS}
        boundary = sum(node.touches_unobserved for node in snapshot.nodes)
        errors = [curve.geometry_error for curve in (*snapshot.edges, *snapshot.cycles)]
        errors = [error for error in errors if error is not None]
        figure.suptitle(
            "Extracted ridge graph · candidate structure\n"
            f"Full mean ridge width: 1 R = {snapshot.scale.unit_px:.4g} px · "
            f"bootstrap period: {snapshot.bootstrap.ridge_period_px:.4g} px\n"
            f"Graph cycle rank: {snapshot.graph_cycle_rank} · "
            f"isolated cycles: {len(snapshot.cycles)}\n"
            f"Paths: {len(snapshot.edges)} · "
            f"endings: {counts['ending']} · junctions: {counts['junction']} · "
            f"isolated nodes: {counts['isolated']}\n"
            f"Nodes touching unobserved regions: {boundary} · "
            f"curves without geometry: {len(errors)} · marker opacity: {alpha:g}",
            fontsize=12,
        )
        if not snapshot.nodes and not snapshot.edges and not snapshot.cycles:
            axis.text(
                0.5,
                0.5,
                "No extracted graph structure",
                transform=axis.transAxes,
                ha="center",
                va="center",
                bbox={"facecolor": "white", "alpha": 0.9},
            )
        if errors:
            message = "; ".join(sorted(set(errors)))
            axis.set_title(textwrap.fill("Geometry unavailable: " + message[:350], 100), fontsize=9)
        axis.set_xlabel("x (source px)\n" + OPACITY_LABEL, fontsize=9)
        return figure
    except Exception:
        plt.close(figure)
        raise


def _draw_curves(axis, curves, closed):
    for failed in (False, True):
        paths = [curve for curve in curves if (curve.geometry_error is not None) == failed]
        if paths:
            color = "#e69f00" if failed else ("#a44cad" if closed else "#00a6b2")
            axis.add_collection(
                clarity_lines(
                    ((curve.points, closed, curve.segment_clarity) for curve in paths),
                    color,
                    linewidths=0.9,
                    linestyles="dashed" if closed else "solid",
                    zorder=3,
                )
            )


def _draw_nodes(axis, nodes, alpha):
    for kind, (marker, color, _) in _KINDS.items():
        for boundary in (False, True):
            selected = [
                node for node in nodes if node.kind == kind and node.touches_unobserved == boundary
            ]
            if not selected:
                continue
            x, y = [node.x for node in selected], [node.y for node in selected]
            axis.scatter(
                x,
                y,
                marker=marker,
                s=30,
                facecolors="none" if boundary else color,
                edgecolors=color,
                linewidths=0.9,
                alpha=alpha,
                zorder=4,
            )
            if boundary:
                axis.scatter(
                    x, y, marker="x", s=13, color="#242424", linewidths=0.7, alpha=alpha, zorder=5
                )


def _graph_legend(figure):
    handles = [
        Line2D([], [], color="#00a6b2", label="Observed path"),
        Line2D([], [], color="#a44cad", linestyle="--", label="Isolated closed cycle"),
        Line2D([], [], color="#e69f00", label="Geometry unavailable"),
        Line2D([], [], color=UNKNOWN_COLOR, alpha=0.4, label="Clarity unavailable"),
    ]
    handles.extend(
        Line2D([], [], marker=marker, color=color, linestyle="none", label=label)
        for marker, color, label in _KINDS.values()
    )
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
