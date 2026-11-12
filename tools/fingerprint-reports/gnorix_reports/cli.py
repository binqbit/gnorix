"""Run native fingerprint extraction and replace the canonical local report."""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import subprocess
from contextlib import contextmanager
from pathlib import Path
from tempfile import mkdtemp

from .structure import MAX_SNAPSHOT_BYTES, render_structure

_REPORTS = (
    "overview.png",
    "structure-overlay.png",
    "scale-analysis.png",
    "bootstrap-analysis.png",
    "normalization-analysis.png",
    "contour-comparison.png",
    "contour-overlay.png",
    "clarity-analysis.png",
)
_DATA_FILES = {
    "features.json",
    "source.png",
    "normalized.png",
    "denoised.png",
    "coherence.png",
    "usable.png",
    "binary.png",
    "skeleton.png",
    "ridge-level.png",
    "valley-level.png",
    "normalization-support.png",
    "measurement-skeleton.png",
    "pattern-support.png",
    "clarity.png",
    "clarity-contrast.png",
    "clarity-separation.png",
    "clarity-repetition.png",
    "clarity-support.png",
}


class _RecoveryRequired(RuntimeError):
    """A previous complete report must survive a failed rollback."""


def parser() -> argparse.ArgumentParser:
    command = argparse.ArgumentParser(
        description="Analyze one opaque PNG with native Gnorix and refresh reports/fingerprint/."
    )
    command.add_argument("--image", type=Path, required=True, help="Opaque fingerprint PNG")
    command.add_argument("--dpi", type=int, default=160, help="PNG resolution (50 to 600)")
    command.add_argument(
        "--overlay-alpha",
        type=float,
        default=0.5,
        help="Map and annotation opacity (0 to 1); clarity lines use a fixed 40–90% range",
    )
    return command


def repository_root() -> Path:
    root = Path(__file__).resolve().parents[3]
    if (
        not (root / "Cargo.toml").is_file()
        or not (root / "crates" / "gnorix" / "examples" / "fingerprint.rs").is_file()
    ):
        raise ValueError("cannot locate the Gnorix checkout; run the tool from its source checkout")
    return root


def _output_directory(root: Path) -> Path:
    parent = root / "reports"
    if parent.is_symlink() or (parent.exists() and not parent.is_dir()):
        raise ValueError("repository reports path must be a real directory")
    parent.mkdir(exist_ok=True)
    return parent / "fingerprint"


def _validate_existing(output: Path) -> None:
    if output.is_symlink() or (output.exists() and not output.is_dir()):
        raise ValueError("reports/fingerprint must be a real directory")
    if not output.exists() or not any(output.iterdir()):
        return
    expected = {*_REPORTS, "README.md", "data"}
    children = {path.name: path for path in output.iterdir()}
    # Recognize owned output by its core files and known filenames. A new
    # diagnostic may be absent in a previous run; it needs no legacy renderer.
    if not {"overview.png", "README.md", "data"} <= children.keys() <= expected or any(
        path.is_symlink() for path in children.values()
    ):
        raise ValueError(
            "reports/fingerprint contains unrelated or incomplete files; refusing replacement"
        )
    if not children["data"].is_dir() or any(
        not path.is_file() for name, path in children.items() if name != "data"
    ):
        raise ValueError("reports/fingerprint contains unexpected file types")
    data = {path.name: path for path in children["data"].iterdir()}
    if not {"features.json", "source.png"} <= data.keys() <= _DATA_FILES or any(
        path.is_symlink() or not path.is_file() for path in data.values()
    ):
        raise ValueError("reports/fingerprint/data contains unrelated or incomplete files")
    with data["features.json"].open("rb") as stream:
        # Identification only: the full new bundle is validated by the renderer.
        # An existing broken report is still replaceable if its schema is intact.
        encoded = stream.read(MAX_SNAPSHOT_BYTES + 1)
    if len(encoded) > MAX_SNAPSHOT_BYTES:
        raise ValueError("existing report JSON exceeds the supported size")
    try:
        snapshot = json.loads(encoded)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise ValueError("existing report does not contain identifiable native output") from error
    if not isinstance(snapshot, dict) or snapshot.get("schema") != "gnorix.ridge-extraction.v1":
        raise ValueError("existing report is not a native Gnorix extraction")


@contextmanager
def _run_lock(parent: Path):
    lock = parent / ".fingerprint.lock"
    try:
        descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    except FileExistsError as error:
        raise ValueError(
            "another fingerprint report run holds reports/.fingerprint.lock; "
            "if it was interrupted, confirm no run is active before removing that lock"
        ) from error
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(f"pid={os.getpid()}\n")
        yield
    finally:
        lock.unlink()


def _extract(root: Path, image: Path, destination: Path) -> None:
    result = subprocess.run(
        [
            "cargo",
            "run",
            "-p",
            "gnorix",
            "--release",
            "--example",
            "fingerprint",
            "--locked",
            "--",
            str(image),
            str(destination),
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode:
        detail = (result.stderr or result.stdout or "no diagnostic output").strip()
        raise RuntimeError(
            f"native fingerprint extraction failed (exit {result.returncode}):\n{detail}"
        )


def _write_readme(staging: Path) -> None:
    data = json.loads((staging / "data" / "features.json").read_text(encoding="utf-8"))
    contours = data["contours"]
    cleanup = data["cleanup"]
    source_policy = cleanup["policy"]["source"]
    relative = source_policy["relative_width_tolerance"]
    lower = (1 - relative) * data["scale"]["unit_px"]
    upper = source_policy["max_ridge_width_px"]
    collapsed = sum(loop["removed_edge"] is not None for loop in cleanup["loops"])
    adjustments = contours["smoothing"]["edges"] + contours["smoothing"]["cycles"]
    accepted = sum(join["status"] == "accepted" for join in contours["joins"])
    applied = sum(item["smoothing_applied"] for item in adjustments)
    clarity = data["clarity"]["summary"]
    mean = clarity["mean_score"]
    mean_text = "unavailable" if mean is None else f"{mean:.4f}"
    text = f"""# Fingerprint report

Start with [the overview](overview.png). This folder contains the latest successful analysis.

- [Observed structure](structure-overlay.png)
- [Full-source width measurement](scale-analysis.png)
- [Bootstrap profiles](bootstrap-analysis.png)
- [Local normalization](normalization-analysis.png)
- [Contour comparison](contour-comparison.png)
- [Refined contour overlay](contour-overlay.png)
- [Local clarity and components](clarity-analysis.png)

Common width unit R: **{data["scale"]["unit_px"]:.6g} source pixels**.
Global width tolerance: **±{relative:.0%}**, **{lower:.6g}–{upper:.6g} px** (0.8R–1.2R).
The lower reference does not exclude smaller artifacts; R remains the measured mean.
Source-confirmed terminal spurs removed: **{len(cleanup["removed_spurs"])}**, in one pass with
branch arc length strictly less than the upper tolerance bound **{upper:.6g} px** (1.2R).
Ambiguous branches retained: **{len(cleanup["retained_ambiguous_edges"])}**.
Branches retained due to missing source support: **{len(cleanup["retained_unobserved_edges"])}**.
Unconfirmed terminal branches retained: **{len(cleanup["retained_unconfirmed_edges"])}**.
Split/rejoin loops collapsed: **{collapsed}**.
Each collapse retains a source-supported observed arm; the split between the arms is less than 1.2R.
Smoothing applied to **{applied} / {len(adjustments)} paths**.
Accepted source-supported joins: **{accepted}**.
Candidates kept separate: **{len(contours["joins"]) - accepted}**.
Native mean clarity over supported pixels: **{mean_text}**.
Clarity support: **{clarity["supported_pixels"]} / {clarity["total_pixels"]} pixels**.

Vermillion dashed paths show removed terminal spurs and duplicate loop arms.
Overlays retain source coordinates. Green joins were accepted using image evidence;
amber dashed candidates remain disconnected. Smoothed geometry is an estimate,
not verified anatomy or a biometric matching score.

Contour opacity follows native segment clarity: 40% + 50% × sqrt(score), a fixed 40–90% range.
Map and annotation opacity does not multiply this line range.
The fixed square-root transfer changes display only; native scores and heatmap stay linear.
Gray paths mean unavailable clarity at 40%; a measured zero retains its path color.
Joins retain status colors, with minimum opacity for missing clarity.
Faint dotted originals in the comparison detail use 55% of line opacity as a reference layer.
Native grid-anchor scores are contrast × separation × repetition, then interpolated
on a fixed 0–1 scale. Components interpolate separately; support can include flat background.

[Native measurements and geometry](data/features.json) and the grayscale processing
stages are retained in `data/`. The report is generated locally from the selected
image. The checked-in test fingerprint is synthetic.

Run `gnorix-report --image /path/to/fingerprint.png` through the repository's
report-tool environment to refresh this folder. A failed extraction or rendering
leaves the previous successful report in place.
"""
    (staging / "README.md").write_text(text, encoding="utf-8")


def _publish(staging: Path, output: Path, backup: Path) -> None:
    had_previous = output.exists()
    if had_previous:
        output.rename(backup)
    try:
        staging.rename(output)
    except BaseException:
        if had_previous:
            try:
                backup.rename(output)
            except OSError as error:
                raise _RecoveryRequired(
                    f"report publication and rollback failed; previous report preserved at {backup}"
                ) from error
        raise
    if had_previous:
        shutil.rmtree(backup)


def generate_report(image: Path, dpi: int, alpha: float) -> Path:
    root = repository_root()
    image = image.resolve(strict=True)
    if not image.is_file():
        raise ValueError("--image must point to a readable PNG file")
    output = _output_directory(root)
    with _run_lock(output.parent):
        _validate_existing(output)
        temporary = Path(mkdtemp(prefix=".fingerprint-", dir=output.parent))
        preserve_recovery = False
        try:
            staging = temporary / "report"
            staging.mkdir()
            _extract(root, image, staging / "data")
            render_structure(staging / "data" / "features.json", staging, dpi, alpha)
            _write_readme(staging)
            _validate_existing(output)
            _publish(staging, output, temporary / "previous")
        except _RecoveryRequired:
            preserve_recovery = True
            raise
        finally:
            if not preserve_recovery:
                shutil.rmtree(temporary)
    return output / "overview.png"


def main(argv: list[str] | None = None) -> int:
    command = parser()
    arguments = command.parse_args(argv)
    if not 50 <= arguments.dpi <= 600:
        command.error("--dpi must be from 50 to 600")
    if not math.isfinite(arguments.overlay_alpha) or not 0 <= arguments.overlay_alpha <= 1:
        command.error("--overlay-alpha must be finite and from 0 to 1")
    try:
        overview = generate_report(arguments.image, arguments.dpi, arguments.overlay_alpha)
    except (OSError, ValueError, RuntimeError) as error:
        command.exit(2, f"error: {error}\n")
    print(overview)
    return 0
