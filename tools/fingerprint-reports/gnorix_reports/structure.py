"""Render validated native extraction data as a complete PNG report."""

from __future__ import annotations

import tempfile
from contextlib import ExitStack
from pathlib import Path

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt

from .clarity import render_clarity_analysis, validate_clarity_maps
from .contour_render import render_contour_comparison, render_contour_overlay
from .feature_strength_render import render_feature_strength
from .full_scale import render_full_scale, validate_measurement_maps
from .images import load_image
from .normalization import render_normalization_analysis, validate_palette
from .scale import render_scale_analysis
from .snapshot import load_snapshot
from .structure_render import render_overview, render_structure_overlay
from .validation import finite_number

REPORT_FILES = (
    "overview.png",
    "structure-overlay.png",
    "scale-analysis.png",
    "bootstrap-analysis.png",
    "normalization-analysis.png",
    "contour-comparison.png",
    "contour-overlay.png",
    "clarity-analysis.png",
    "feature-strength.png",
)
_STYLE = {
    "font.family": "DejaVu Sans",
    "font.size": 10,
    "axes.titlesize": 11,
    "axes.labelsize": 9,
    "figure.facecolor": "white",
    "savefig.facecolor": "white",
    "text.usetex": False,
    "text.parse_math": False,
}


def render_structure(
    snapshot_path: Path, output: Path, dpi: int = 160, alpha: float = 0.5
) -> list[Path]:
    """Validate all inputs and render nine PNGs before replacing report files.

    Coordinates remain source pixel centers. This pipeline displays supplied
    measurements and geometry; it performs no biometric extraction or repair.
    """
    alpha = _render_options(dpi, alpha)
    snapshot = load_snapshot(Path(snapshot_path))
    output = Path(output)

    with ExitStack() as stack:
        images = _load_images(snapshot, stack)
        palette = validate_palette(images)
        validate_measurement_maps(snapshot.scale, images)
        clarity_maps = validate_clarity_maps(snapshot.clarity, images)

        # Decode and validate everything before touching report output. Finish
        # every figure in a sibling directory before replacing existing files.
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix=".ridge-report-", dir=output.parent) as temporary:
            staging = Path(temporary)
            with matplotlib.rc_context(_STYLE):
                figures = _figures(snapshot, images, palette, clarity_maps, alpha)
                for name, figure in zip(REPORT_FILES, figures, strict=True):
                    _save_figure(figure, staging / name, dpi)
            return _publish_figures(staging, output)


def _render_options(dpi, alpha):
    if type(dpi) is not int or not 1 <= dpi <= 600:
        raise ValueError("report DPI must be an integer in [1, 600]")
    alpha = finite_number(alpha, "alpha")
    if not 0 <= alpha <= 1:
        raise ValueError("alpha must be in [0, 1]")
    return alpha


def _load_images(snapshot, stack):
    images = {}
    for key, path in snapshot.images.items():
        image = load_image(path, (snapshot.width, snapshot.height))
        stack.callback(image.close)
        images[key] = image
    return images


def _figures(snapshot, images, palette, clarity_maps, alpha):
    """Construct one figure at a time in REPORT_FILES order to bound memory."""
    yield render_overview(snapshot, images)
    yield render_structure_overlay(snapshot, images)
    yield render_full_scale(snapshot.scale, images, alpha)
    yield render_scale_analysis(snapshot.bootstrap, images["source"], alpha)
    yield render_normalization_analysis(snapshot.normalization, palette, images)
    yield render_contour_comparison(snapshot.contours, images["source"], alpha)
    yield render_contour_overlay(snapshot.contours, images["source"], alpha)
    yield render_clarity_analysis(snapshot.clarity, clarity_maps, images["source"], alpha)
    yield render_feature_strength(snapshot.feature_strength, images["source"])


def _publish_figures(staging, output):
    output.mkdir(parents=True, exist_ok=True)
    paths = []
    for name in REPORT_FILES:
        target = output / name
        (staging / name).replace(target)
        paths.append(target)
    return paths


def _save_figure(figure, path, dpi):
    try:
        figure.savefig(path, dpi=dpi, bbox_inches="tight", pad_inches=0.15)
    finally:
        plt.close(figure)
