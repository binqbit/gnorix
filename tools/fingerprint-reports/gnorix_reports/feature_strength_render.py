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
    ("source_support", "P · source", "#0072b2"),
    ("geometry_support", "G · geometry", "#d55e00"),
    ("stability", "T · stability", "#009e73"),
    ("strength", "Q · combined", "#7b3294"),
)


def candidate_colors(candidates):
    """A fixed 0–1 color scale; unavailable and measured zero remain distinct."""
    colors = []
    cmap = plt.get_cmap("viridis")
    for candidate in candidates:
        if candidate.strength is None:
            colors.append(to_rgba(UNKNOWN_COLOR, 0.4))
        else:
            color = list(cmap(candidate.strength))
            color[3] = 0.4 + 0.5 * math.sqrt(candidate.strength)
            colors.append(color)
    return np.asarray(colors).reshape(-1, 4)


def _source(axis, image, width, height):
    extent = (-0.5, width - 0.5, height - 0.5, -0.5)
    axis.imshow(
        image, cmap="gray", vmin=0, vmax=255, extent=extent, interpolation="nearest", alpha=1
    )
    axis.set(xlim=extent[:2], ylim=extent[2:], xlabel="x (source px)", ylabel="y (source px)")
    axis.set_aspect("equal")


def _markers(axis, candidates, size):
    for kind, (marker, _) in _KINDS.items():
        selected = [item for item in candidates if item.kind == kind]
        if not selected:
            continue
        colors = candidate_colors(selected)
        axis.scatter(
            [item.position[0] for item in selected],
            [item.position[1] for item in selected],
            s=size,
            marker=marker,
            facecolors=colors,
            edgecolors=[to_rgba("white", color[3]) for color in colors],
            linewidths=0.3,
            zorder=3,
        )


def detail_examples(candidates):
    """Choose varied evidence extremes deterministically, without fixture IDs."""
    known = [item for item in candidates if item.strength is not None]
    unknown = [item for item in candidates if item.strength is None]
    selections = []
    if known:
        selections.append(("Lowest measured Q", min(known, key=lambda c: (c.strength, c.id))))
        selections.append(("Highest measured Q", max(known, key=lambda c: (c.strength, -c.id))))
    if unknown:
        selections.append(("Unavailable evidence", min(unknown, key=lambda c: c.id)))
    cycles = [item for item in candidates if item.kind == "cycle"]
    if cycles:
        selections.append(
            (
                "Isolated-cycle evidence",
                min(
                    cycles,
                    key=lambda c: (
                        c.strength is None,
                        c.strength if c.strength is not None else 0,
                        c.id,
                    ),
                ),
            )
        )
    disagreements = sorted(known, key=lambda c: (-abs(c.source_support - c.geometry_support), c.id))
    selections.extend(("Source / geometry disagreement", item) for item in disagreements)
    selections.extend(("Candidate evidence", item) for item in candidates)
    result, seen = [], set()
    for title, candidate in selections:
        if candidate.id not in seen:
            result.append((title, candidate))
            seen.add(candidate.id)
        if len(result) == 4:
            break
    return result


def _histogram(axis, candidates, components):
    bins = np.linspace(0, 1, 11)
    for field, label, color in components:
        values = [getattr(item, field) for item in candidates if getattr(item, field) is not None]
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
    radius = max(2 * snapshot.unit_px, 4)
    x, y = candidate.position
    axis.set(
        xlim=(max(-0.5, x - radius), min(snapshot.width - 0.5, x + radius)),
        ylim=(min(snapshot.height - 0.5, y + radius), max(-0.5, y - radius)),
    )
    for arm in candidate.arms:
        points = snapshot.edge_points[arm.edge]
        axis.plot(*np.asarray(points).T, color="#40b9e6", linewidth=1, alpha=0.55, zorder=2)
    if candidate.source_cycle is not None:
        points = list(snapshot.cycle_points[candidate.source_cycle])
        axis.plot(*np.asarray(points + points[:1]).T, color="#40b9e6", linewidth=1, alpha=0.55)
    _markers(axis, [candidate], 95)
    axis.set_title(f"{title}\n#{candidate.id} · {_KINDS[candidate.kind][1]}", fontsize=10)
    values = "  ".join(
        f"{label[0]}={getattr(candidate, name):.3f}"
        if getattr(candidate, name) is not None
        else f"{label[0]}=?"
        for name, label, _ in _COMPONENTS
    )
    reasons = ", ".join(reason.replace("_", " ") for reason in candidate.reasons) or "none"
    axis.set_xlabel(
        values + "\n" + f"Source probes {candidate.source_coverage:.0%}; "
        f"arc extent {candidate.geometry_coverage:.0%}; "
        f"stress matches {sum(item is not None for item in candidate.stress_matches)}/8"
        + "\n"
        + textwrap.fill("Reasons: " + reasons, 52),
        fontsize=8,
    )


def render_feature_strength(snapshot, source):
    figure = plt.figure(figsize=(20, 15), layout="constrained")
    try:
        grid = figure.add_gridspec(3, 4, width_ratios=(1, 1, 1.05, 1.05))
        overlay = figure.add_subplot(grid[:, :2])
        _source(overlay, source, snapshot.width, snapshot.height)
        _markers(overlay, snapshot.candidates, 18)
        known = sum(candidate.strength is not None for candidate in snapshot.candidates)
        overlay.set_title(
            f"All {len(snapshot.candidates)} original feature candidates\n"
            f"{known} measured · {len(snapshot.candidates) - known} unavailable"
        )
        handles = [
            Line2D([], [], marker=marker, linestyle="none", color="#31688e", label=label)
            for marker, label in _KINDS.values()
        ]
        handles.append(
            Line2D(
                [], [], marker="o", linestyle="none", color=UNKNOWN_COLOR, label="Unknown Q · gray"
            )
        )
        figure.legend(
            handles=handles, loc="outside lower center", ncol=6, fontsize=9, frameon=False
        )
        figure.colorbar(
            ScalarMappable(norm=Normalize(0, 1), cmap="viridis"),
            ax=overlay,
            shrink=0.6,
            pad=0.02,
            label="Q · native combined strength (0–1)",
        )
        _histogram(figure.add_subplot(grid[0, 2]), snapshot.candidates, _COMPONENTS[:2])
        _histogram(figure.add_subplot(grid[0, 3]), snapshot.candidates, _COMPONENTS[2:])
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
            "Native feature evidence · source P, geometry G, stability T, combined Q\n"
            "Q = min(P, G) × (0.5 + 0.5T) · all raw candidates retained, "
            "including cleanup removals\n"
            "Source stays visible; marker opacity 40–90%. "
            "Diagnostic evidence, not biometric probability.",
            fontsize=14,
        )
        return figure
    except Exception:
        plt.close(figure)
        raise
