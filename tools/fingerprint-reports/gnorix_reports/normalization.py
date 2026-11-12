"""Validation and PNG figure construction for supplied local grayscale references."""

from __future__ import annotations

import math
from dataclasses import dataclass

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.ticker import MaxNLocator

IMAGE_KEYS = ("ridge_level", "valley_level", "normalization_support")


@dataclass(frozen=True)
class Normalization:
    radius_px: int


@dataclass(frozen=True)
class Palette:
    ridge: np.ndarray
    valley: np.ndarray
    supported: np.ndarray


def validate_normalization(value, period: float) -> Normalization:
    """Require the current producer method and its estimated-scale radius."""
    if not isinstance(value, dict):
        raise ValueError("normalization must be an object")
    radius = value.get("radius_px")
    if type(radius) is not int or radius != math.ceil(period / 2):
        raise ValueError("normalization.radius_px must equal ceil(ridge_period_px / 2)")
    if value.get("method") != "local-two-class-midpoint":
        raise ValueError("normalization.method must be local-two-class-midpoint")
    if value.get("reference_encoding") != "rounded-u8":
        raise ValueError("normalization.reference_encoding must be rounded-u8")
    return Normalization(radius)


def validate_palette(images) -> Palette:
    """Validate lossily rounded references without inventing missing values.

    A supported pair may round to equal bytes despite positive native contrast.
    Zero is a valid supported level; only the binary support map marks absence.
    Native normalization cannot be reconstructed exactly from these rounded PNGs.
    """
    for key in ("source", "normalized", *IMAGE_KEYS):
        if images[key].mode != "L":
            raise ValueError(f"images.{key} must be opaque 8-bit grayscale")
    ridge = np.asarray(images["ridge_level"], dtype=np.float64)
    valley = np.asarray(images["valley_level"], dtype=np.float64)
    support = np.asarray(images["normalization_support"])
    if not np.all((support == 0) | (support == 255)):
        raise ValueError("normalization support pixels must be 0 or 255")
    supported = support == 255
    if np.any(ridge[supported] > valley[supported]):
        raise ValueError("supported ridge reference must not exceed valley reference")
    if np.any(ridge[~supported] != 0) or np.any(valley[~supported] != 0):
        raise ValueError("unsupported reference pixels must be zero")
    return Palette(ridge, valley, supported)


def render_normalization_analysis(normalization: Normalization, palette: Palette, images):
    """Plot source, rounded local references and their explicitly derived values."""
    width, height = images["source"].size
    figure, axes = plt.subplots(2, 4, figsize=(18, 12), layout="constrained")
    try:
        row = height // 2
        source_axis = axes[0, 0]
        _map(source_axis, images["source"], "Source · unchanged grayscale", width, height)
        source_axis.axhline(row, color="#0072b2", linewidth=0.8, alpha=0.7)
        _reference_map(axes[0, 1], palette.ridge, palette, "Local dark reference", width, height)
        _reference_map(axes[0, 2], palette.valley, palette, "Local light reference", width, height)
        _map(
            axes[0, 3],
            images["normalization_support"],
            "Reference support · white = present",
            width,
            height,
        )
        _reference_map(
            axes[1, 0],
            (palette.ridge + palette.valley) / 2,
            palette,
            "Midpoint · from rounded references",
            width,
            height,
        )
        _reference_map(
            axes[1, 1],
            palette.valley - palette.ridge,
            palette,
            "Light − dark · rounded gray spread",
            width,
            height,
            cmap="cividis",
            colorbar_label="Gray-level difference",
        )
        _map(axes[1, 2], images["normalized"], "Normalized intensity · −3 to 3", width, height)
        _cross_section(axes[1, 3], images["source"], palette, row)
        supported = np.count_nonzero(palette.supported)
        figure.suptitle(
            "Local two-class midpoint normalization\n"
            f"Neighborhood radius: {normalization.radius_px} source px · "
            f"reference support: {supported} / {width * height} pixels\n"
            "References are local seed-class means, not anatomical ground truth.",
            fontsize=14,
        )
        figure.supxlabel(
            "Reference maps: raw gray 0 = black, 255 = white; missing = pale blue. "
            "PNGs round reference levels to bytes.\n"
            "Derived midpoint/spread use those rounded values; native normalization uses "
            "unrounded references and clips (source − midpoint) / half-spread to [−3, 3].",
            fontsize=10,
        )
        return figure
    except Exception:
        plt.close(figure)
        raise


def _map(axis, values, title, width, height, cmap="gray"):
    extent = (-0.5, width - 0.5, height - 0.5, -0.5)
    artist = axis.imshow(
        values,
        cmap=cmap,
        vmin=0,
        vmax=255,
        extent=extent,
        interpolation="nearest",
        alpha=1,
    )
    axis.set(
        title=title,
        xlim=extent[:2],
        ylim=extent[2:],
        aspect="equal",
        xlabel="x (source px)",
        ylabel="y (source px)",
    )
    axis.xaxis.set_major_locator(MaxNLocator(4))
    axis.yaxis.set_major_locator(MaxNLocator(4))
    return artist


def _reference_map(
    axis, values, palette, title, width, height, cmap="gray", colorbar_label="Raw gray level"
):
    colors = matplotlib.colormaps[cmap].with_extremes(bad="#dfeaf1")
    masked = np.ma.array(values, mask=~palette.supported)
    artist = _map(axis, masked, title, width, height, colors)
    axis.figure.colorbar(artist, ax=axis, fraction=0.046, pad=0.03, label=colorbar_label)


def _cross_section(axis, image, palette, row):
    source = np.asarray(image)[row]
    x = np.arange(len(source))
    dark = np.where(palette.supported[row], palette.ridge[row], np.nan)
    light = np.where(palette.supported[row], palette.valley[row], np.nan)
    axis.plot(x, source, color="#555555", linewidth=0.7, label="Source")
    axis.plot(x, dark, color="#0072b2", linewidth=1, label="Dark reference")
    axis.plot(x, light, color="#d55e00", linewidth=1, label="Light reference")
    axis.plot(x, (dark + light) / 2, color="#009e73", linewidth=1, linestyle=":", label="Midpoint")
    axis.set(
        title=f"Source cross-section · y = {row} px",
        xlim=(-0.5, len(source) - 0.5),
        ylim=(0, 255),
        xlabel="x (source px)",
        ylabel="Raw grayscale",
    )
    axis.grid(alpha=0.2)
    axis.legend(fontsize=8)
