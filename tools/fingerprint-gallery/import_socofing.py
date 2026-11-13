"""Import the five curated SOCOFing originals without changing observed pixels."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "crates/gnorix/examples/fingerprint_lab/gallery.json"
DEFAULT_OUTPUT = ROOT / "test-data/fingerprint-gallery"


def observed_grayscale(source: Image.Image) -> Image.Image:
    """Remove only transparent edge padding; reject holes and partial alpha."""
    box = (0, 0, source.width, source.height)
    if "A" in source.getbands():
        alpha = source.getchannel("A")
        box = alpha.getbbox()
        if box is None or alpha.crop(box).getextrema() != (255, 255):
            raise ValueError("capture requires a validity mask, not alpha removal")

    red, green, blue = source.convert("RGB").crop(box).split()
    if red.tobytes() != green.tobytes() or red.tobytes() != blue.tobytes():
        raise ValueError("expected a grayscale capture with equal RGB channels")
    return red


def prepare_image(dataset: Path, entry: dict) -> bytes:
    path = dataset / entry["source"]
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != entry["source_sha256"]:
        raise ValueError(f"source SHA-256 mismatch: {entry['source']}")

    with Image.open(io.BytesIO(data)) as source:
        grayscale = observed_grayscale(source)
    if grayscale.size != (entry["width"], entry["height"]):
        raise ValueError(f"observed dimensions differ: {entry['source']}")
    pixels = grayscale.tobytes()
    if hashlib.sha256(pixels).hexdigest() != entry["pixels_sha256"]:
        raise ValueError(f"observed pixel SHA-256 mismatch: {entry['source']}")

    encoded = io.BytesIO()
    grayscale.save(encoded, format="PNG")
    with Image.open(io.BytesIO(encoded.getvalue())) as decoded:
        if decoded.mode != "L" or decoded.size != grayscale.size or decoded.tobytes() != pixels:
            raise ValueError(f"PNG did not preserve observed pixels: {entry['source']}")
    return encoded.getvalue()


def write_image(output: Path, name: str, data: bytes) -> None:
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=output, prefix=".import-", delete=False) as handle:
            temporary = Path(handle.name)
            handle.write(data)
        os.replace(temporary, output / name)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def import_gallery(dataset: Path, output: Path) -> int:
    entries = json.loads(MANIFEST.read_text())["images"]
    # Validate every source before replacing any gallery image.
    prepared = [(entry["id"] + ".png", prepare_image(dataset, entry)) for entry in entries]
    output.mkdir(parents=True, exist_ok=True)
    for name, data in prepared:
        write_image(output, name, data)
    return len(prepared)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True, help="SOCOFing directory containing Real/")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT, help="local gallery PNG directory")
    args = parser.parse_args()
    try:
        count = import_gallery(args.source, args.output)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"Gallery import failed: {error}\n")
    print(f"Imported {count} verified original captures into {args.output}")


if __name__ == "__main__":
    main()
