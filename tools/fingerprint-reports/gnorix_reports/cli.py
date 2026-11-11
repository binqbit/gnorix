"""Command-line orchestration for static diagnostic reports."""

from __future__ import annotations

import argparse
from pathlib import Path
from tempfile import TemporaryDirectory

from .model import load_snapshot
from .options import OverlayOptions
from .render import render_snapshot
from .sample import write_sample


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description="Render fingerprint diagnostics as local PNG files.")
    commands = root.add_subparsers(dest="command", required=True)
    render = commands.add_parser("render", help="Render a saved diagnostic snapshot")
    render.add_argument("snapshot", type=Path)
    render.add_argument("--metric", action="append", help="Render only named metrics (repeatable)")
    sample = commands.add_parser("sample", help="Measure image intensities on a preview grid")
    sample.add_argument("--image", type=Path, required=True)
    sample.add_argument("--spacing", type=float, default=36, help="Preview grid spacing in pixels")
    sample.add_argument("--radius", type=int, default=12, help="Intensity window radius in pixels")
    sample.add_argument(
        "--rotation", type=float, default=0, help="Preview grid rotation in degrees"
    )

    for command in (render, sample):
        command.add_argument(
            "--output", type=Path, help="Output directory (default: repository-root reports/)"
        )
        command.add_argument("--dpi", type=int, default=160, help="PNG resolution (50 to 600)")
        command.add_argument(
            "--overlay-alpha", type=float, default=0.5, help="Overlay opacity from 0 to 1"
        )
        command.add_argument(
            "--point-radius", type=float, default=3, help="Point marker radius in image pixels"
        )
        command.add_argument(
            "--tolerance-radius",
            type=float,
            help="Display a circular tolerance radius for every point, in image pixels",
        )
        command.add_argument(
            "--crop",
            type=float,
            nargs=4,
            metavar=("X", "Y", "WIDTH", "HEIGHT"),
            help="Display this crop in original image pixel coordinates",
        )
        command.add_argument(
            "--overlay-only", action="store_true", help="Export only the large image overlays"
        )
        command.add_argument(
            "--overwrite", action="store_true", help="Replace matching output files"
        )
    return root


def publish_files(paths: list[Path], output: Path, overwrite: bool) -> list[Path]:
    destinations = [output / path.name for path in paths]
    if not overwrite:
        conflicts = [path.name for path in destinations if path.exists()]
        if conflicts:
            raise ValueError(f"output already exists: {', '.join(conflicts)}; use --overwrite")

    for source, destination in zip(paths, destinations, strict=True):
        source.replace(destination)
    return destinations


def render_bundle(arguments: argparse.Namespace) -> list[Path]:
    snapshot = load_snapshot(arguments.snapshot)
    arguments.output.mkdir(parents=True, exist_ok=True)

    with TemporaryDirectory(prefix=".pending-", dir=arguments.output) as temporary:
        files = render_snapshot(
            snapshot,
            Path(temporary),
            arguments.dpi,
            tuple(arguments.metric) if arguments.metric else None,
            overlay=overlay_options(arguments),
            overlay_only=arguments.overlay_only,
        )
        return publish_files(files, arguments.output, arguments.overwrite)


def sample_bundle(arguments: argparse.Namespace) -> list[Path]:
    arguments.output.mkdir(parents=True, exist_ok=True)

    with TemporaryDirectory(prefix=".pending-", dir=arguments.output) as temporary:
        folder = Path(temporary)
        snapshot_path = write_sample(
            arguments.image, folder, arguments.spacing, arguments.radius, arguments.rotation
        )
        snapshot = load_snapshot(snapshot_path)
        reports = render_snapshot(
            snapshot,
            folder,
            arguments.dpi,
            overlay=overlay_options(arguments),
            overlay_only=arguments.overlay_only,
        )
        files = [folder / "capture.png", snapshot_path, *reports]
        return publish_files(files, arguments.output, arguments.overwrite)


def overlay_options(arguments: argparse.Namespace) -> OverlayOptions:
    return OverlayOptions(
        alpha=arguments.overlay_alpha,
        point_radius=arguments.point_radius,
        tolerance_radius=arguments.tolerance_radius,
        crop=tuple(arguments.crop) if arguments.crop is not None else None,
    )


def default_output() -> Path:
    root = Path(__file__).resolve().parents[3]
    if not (root / "Cargo.toml").is_file() or not (root / "crates" / "gnorix").is_dir():
        raise ValueError("cannot locate the Gnorix checkout; specify --output explicitly")
    return root / "reports"


def main(argv: list[str] | None = None) -> int:
    command = parser()
    arguments = command.parse_args(argv)
    if not 50 <= arguments.dpi <= 600:
        command.error("--dpi must be from 50 to 600")

    try:
        if arguments.output is None:
            arguments.output = default_output()
        files = (
            sample_bundle(arguments) if arguments.command == "sample" else render_bundle(arguments)
        )
    except (OSError, ValueError) as error:
        command.exit(2, f"error: {error}\n")

    for path in files:
        print(path)
    return 0
