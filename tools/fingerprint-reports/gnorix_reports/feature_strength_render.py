"""Display supplied feature evidence without detecting or filtering features."""

from __future__ import annotations

import math
import textwrap

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.cm import ScalarMappable
from matplotlib.colors import Normalize, to_rgba
from matplotlib.lines import Line2D

from .clarity import UNKNOWN_COLOR

_KINDS = {
    "ending": ("o", "Ending"),
    "bifurcation": ("^", "Bifurcation"),
    "junction": ("D", "Junction"),
    "isolated": ("s", "Isolated node"),
    "cycle": ("P", "Isolated cycle"),
}
_COMPONENTS = (
    ("source_support", "P · local source", "#0072b2"),
    ("geometry_support", "G · local geometry", "#d55e00"),
    ("stability", "T · stability", "#009e73"),
    ("context", "C · 4R context", "#e69f00"),
    ("strength", "Q · feature quality", "#7b3294"),
    ("priority", "Priority · Q × N", "#005f73"),
)
MARKER_OPACITY_LABEL = (
    "Feature markers: color and opacity = priority Q × N · 40% + 50% × sqrt(priority)."
)


def candidate_colors(candidates):
    """A fixed 0–1 color scale; unavailable and measured zero remain distinct."""
    colors = []
    cmap = plt.get_cmap("viridis")
    for candidate in candidates:
        if candidate.priority is None:
            colors.append(to_rgba(UNKNOWN_COLOR, 0.4))
        else:
            color = list(cmap(candidate.priority))
            color[3] = 0.4 + 0.5 * math.sqrt(candidate.priority)
            colors.append(color)
    return np.asarray(colors).reshape(-1, 4)


def _source(axis, image, width, height):
    extent = (-0.5, width - 0.5, height - 0.5, -0.5)
    axis.imshow(
        image, cmap="gray", vmin=0, vmax=255, extent=extent, interpolation="nearest", alpha=1
    )
    axis.set(xlim=extent[:2], ylim=extent[2:], xlabel="x (source px)", ylabel="y (source px)")
    axis.set_aspect("equal")


def draw_feature_markers(axis, candidates, size, *, hollow=False):
    """Draw supplied priority, retaining uncertain-boundary notation."""
    for kind, (marker, _) in _KINDS.items():
        selected = [item for item in candidates if item.kind == kind]
        if not selected:
            continue
        colors = candidate_colors(selected)
        x, y = np.asarray([item.position for item in selected]).T
        axis.scatter(
            x,
            y,
            s=size,
            marker=marker,
            facecolors="none" if hollow else colors,
            edgecolors=colors,
            linewidths=0.9 if hollow else 0.3,
            zorder=4,
        )
        if hollow:
            axis.scatter(x, y, marker="x", s=size * 0.43, color=colors, linewidths=0.7, zorder=5)


def feature_legend_handles():
    handles = [
        Line2D([], [], marker=marker, linestyle="none", color="#31688e", label=label)
        for marker, label in _KINDS.values()
    ]
    handles.append(
        Line2D(
            [],
            [],
            marker="o",
            linestyle="none",
            color=UNKNOWN_COLOR,
            label="Unknown priority · gray",
        )
    )
    return handles


def add_priority_colorbar(figure, axis):
    figure.colorbar(
        ScalarMappable(norm=Normalize(0, 1), cmap="viridis"),
        ax=axis,
        shrink=0.6,
        pad=0.02,
        label="Feature priority Q × N · 0 = dim, 1 = bright",
    )


def detail_examples(candidates):
    """Choose varied evidence extremes deterministically, without fixture IDs."""
    known = [item for item in candidates if item.priority is not None]
    unknown = [item for item in candidates if item.priority is None]
    selections = []
    if known:
        selections.append(
            ("Highest measured priority", max(known, key=lambda c: (c.priority, -c.id)))
        )
    forks = [item for item in known if item.kind in ("bifurcation", "junction")]
    if forks:
        selections.append(
            ("Strongest measured fork", max(forks, key=lambda c: (c.priority, -c.id)))
        )
    if known:
        selections.append(
            (
                "Stability / context disagreement",
                max(known, key=lambda c: (c.stability - c.context.support, -c.id)),
            )
        )
    if unknown:
        selections.append(("Unavailable evidence", min(unknown, key=lambda c: c.id)))
    gains = [item for item in known if item.context.support_2r is not None]
    if gains:
        selections.append(
            (
                "Largest 2R / 4R context gain",
                max(gains, key=lambda c: (c.context.support - c.context.support_2r, -c.id)),
            )
        )
    cycles = [item for item in candidates if item.kind == "cycle"]
    if cycles:
        selections.append(
            (
                "Isolated-cycle evidence",
                min(
                    cycles,
                    key=lambda c: (
                        c.priority is None,
                        c.priority if c.priority is not None else 0,
                        c.id,
                    ),
                ),
            )
        )
    disagreements = sorted(known, key=lambda c: (-abs(c.source_support - c.geometry_support), c.id))
    selections.extend(("Source / geometry disagreement", item) for item in disagreements)
    selections.extend(
        ("Candidate evidence", item) for item in sorted(candidates, key=lambda c: c.id)
    )
    result, seen = [], set()
    for title, candidate in selections:
        if candidate.id not in seen:
            result.append((title, candidate))
            seen.add(candidate.id)
        if len(result) == 4:
            break
    return result


def _component(candidate, field):
    return candidate.context.support if field == "context" else getattr(candidate, field)


def _histogram(axis, candidates, components):
    bins = np.linspace(0, 1, 11)
    for field, label, color in components:
        values = [
            _component(item, field) for item in candidates if _component(item, field) is not None
        ]
        counts, _ = np.histogram(values, bins)
        unknown = len(candidates) - len(values)
        axis.stairs(
            counts, bins, color=color, linewidth=1.8, label=f"{label} · {unknown} unavailable"
        )
    axis.set(xlim=(0, 1), xlabel="Native score · fixed 0–1 scale", ylabel="Candidate count")
    axis.set_ylim(bottom=0)
    axis.grid(alpha=0.2)
    axis.legend(fontsize=8, loc="upper right")


def _detail(axis, snapshot, image, candidate, title):
    _source(axis, image, snapshot.width, snapshot.height)
    radius = 4 * snapshot.unit_px
    x, y = candidate.position
    axis.set(
        xlim=(max(-0.5, x - radius), min(snapshot.width - 0.5, x + radius)),
        ylim=(min(snapshot.height - 0.5, y + radius), max(-0.5, y - radius)),
    )
    context_edges = {edge for arm in candidate.context.arms for edge in arm.edges}
    for edge in sorted(context_edges | {arm.edge for arm in candidate.arms}):
        points = snapshot.edge_points[edge]
        axis.plot(*np.asarray(points).T, color="#40b9e6", linewidth=1, alpha=0.55, zorder=2)
    if candidate.source_cycle is not None:
        points = list(snapshot.cycle_points[candidate.source_cycle])
        axis.plot(*np.asarray(points + points[:1]).T, color="#40b9e6", linewidth=1, alpha=0.55)
    draw_feature_markers(axis, [candidate], 95)
    axis.set_title(f"{title}\n#{candidate.id} · {_KINDS[candidate.kind][1]}", fontsize=10)
    values = "  ".join(
        f"{label[0]}={_number(_component(candidate, name))}" for name, label, _ in _COMPONENTS[:-1]
    )
    neighborhood = candidate.neighborhood
    priority = f"N={neighborhood.factor:.3f} · priority={_number(candidate.priority)}"
    scale = (
        f"Context: 2R={_number(candidate.context.support_2r)}, "
        f"4R={_number(candidate.context.support)}; "
        f"source coverage {_coverage(candidate.context.source_coverage)}"
    )
    shape = (
        f"Graded cap={_number(candidate.context.termination)}; "
        f"combined continuation={_number(candidate.context.continuation)}\n"
        f"Continuation: forward={_number(candidate.context.forward_continuation)}; "
        f"curved={_number(candidate.context.curved_continuation)}"
        if candidate.kind == "ending"
        else f"Context separation={_number(candidate.context.separation)}"
    )
    crowding = (
        f"3R neighbors: {neighborhood.neighbor_count}, weighted {neighborhood.weighted_count:.2f}; "
        f"observed {_coverage(neighborhood.observed_fraction)}"
    )
    reasons = ", ".join(_reason_label(reason) for reason in candidate.reasons) or "none"
    axis.set_xlabel(
        values
        + "\n"
        + priority
        + "\n"
        + scale
        + "\n"
        + shape
        + "\n"
        + crowding
        + "\n"
        + _arm_summary(candidate.context.arms, snapshot.unit_px)
        + "\n"
        + textwrap.fill("Reasons: " + reasons, 65),
        fontsize=8,
    )


def _number(value):
    return "?" if value is None else f"{value:.3f}"


def _coverage(value):
    percentage = f"{value:.0%}"
    return "<100%" if value < 1 and percentage == "100%" else percentage


def _reason_label(reason):
    if reason == "possible_continuation":
        return "local possible continuation"
    return reason.replace("_", " ")


def _in_units(value, unit):
    return _number(None if value is None else value / unit) + "R"


def _arm_summary(arms, unit):
    return (
        "\n".join(
            f"Arm {arm.edge}: {arm.length_px / unit:.2f}R · "
            f"separation {_number(arm.ridge_separation)} · continuity {_number(arm.continuity)} · "
            f"{arm.stop.replace('_', ' ')}\n"
            f"Supported length {_in_units(arm.supported_length_px, unit)}; "
            f"longest unsupported {_in_units(arm.longest_unsupported_px, unit)}"
            for arm in arms
        )
        or "No incident arms"
    )


def render_feature_strength(snapshot, source):
    figure = plt.figure(figsize=(22, 18), layout="constrained")
    try:
        grid = figure.add_gridspec(3, 4, width_ratios=(1, 1, 1.05, 1.05))
        overlay = figure.add_subplot(grid[:, :2])
        _source(overlay, source, snapshot.width, snapshot.height)
        draw_feature_markers(overlay, snapshot.candidates, 18)
        known = sum(candidate.priority is not None for candidate in snapshot.candidates)
        overlay.set_title(
            f"All {len(snapshot.candidates)} original feature candidates\n"
            f"{known} measured · {len(snapshot.candidates) - known} unavailable"
        )
        figure.legend(
            handles=feature_legend_handles(),
            loc="outside lower center",
            ncol=6,
            fontsize=9,
            frameon=False,
        )
        add_priority_colorbar(figure, overlay)
        _histogram(figure.add_subplot(grid[0, 2]), snapshot.candidates, _COMPONENTS[:3])
        _histogram(figure.add_subplot(grid[0, 3]), snapshot.candidates, _COMPONENTS[3:])
        examples = detail_examples(snapshot.candidates)
        for index in range(4):
            axis = figure.add_subplot(grid[1 + index // 2, 2 + index % 2])
            if index < len(examples):
                title, candidate = examples[index]
                _detail(axis, snapshot, source, candidate, title)
            else:
                axis.set_axis_off()
                axis.text(
                    0.5, 0.5, "No additional candidate", transform=axis.transAxes, ha="center"
                )
        figure.suptitle(
            f"Native feature evidence · R={snapshot.unit_px:.3f} px · "
            "4R context C, local P/G, stability T and neighborhood N\n"
            "Q = C × (0.75 + 0.25 × min(P, G)) × (0.5 + 0.5T); priority = Q × N\n"
            "Higher priority = brighter markers, opacity 40–90%; all raw candidates retained.\n"
            "Details show source within ±4R; graph ending means extraction stopped.\n"
            "Diagnostic evidence, not biometric probability.",
            fontsize=14,
        )
        return figure
    except Exception:
        plt.close(figure)
        raise
