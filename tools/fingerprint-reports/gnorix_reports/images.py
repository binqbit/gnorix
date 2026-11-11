"""Bounded local PNG/JPEG decoding shared by sampling and report rendering."""

from __future__ import annotations

import os
import warnings
from pathlib import Path

from PIL import Image, UnidentifiedImageError

from .model import MAX_FRAME_PIXELS, MAX_FRAME_SIDE

MAX_IMAGE_BYTES = 128 * 1024 * 1024
SUPPORTED_MODES = frozenset({"L", "LA", "P", "RGB", "RGBA"})


def load_image(path: Path, expected_size: tuple[int, int] | None = None) -> Image.Image:
    """Return an owned native-mode image; the caller must close the returned copy.

    Format, dimensions and 8-bit sample mode are checked before full decoding.
    Palette/alpha information is retained for an explicit caller-side conversion.
    """
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("error", Image.DecompressionBombWarning)
            with path.open("rb") as source:
                if os.fstat(source.fileno()).st_size > MAX_IMAGE_BYTES:
                    raise ValueError("image exceeds the 128 MiB file limit")

                with Image.open(source) as image:
                    _validate_image(image, expected_size)
                    image.load()
                    return image.copy()
    except (
        OSError,
        SyntaxError,
        OverflowError,
        UnidentifiedImageError,
        Image.DecompressionBombError,
        Image.DecompressionBombWarning,
    ) as error:
        raise ValueError(f"image {path.name!r} cannot be decoded safely") from error


def _validate_image(image: Image.Image, expected_size: tuple[int, int] | None) -> None:
    if image.format not in {"PNG", "JPEG"}:
        raise ValueError("image content must be PNG or JPEG")
    width, height = image.size
    if width * height > MAX_FRAME_PIXELS or max(width, height) > MAX_FRAME_SIDE:
        raise ValueError("image exceeds the supported pixel dimensions")
    if expected_size is not None and image.size != expected_size:
        raise ValueError("decoded image dimensions do not match the frame")
    if image.mode not in SUPPORTED_MODES:
        raise ValueError(f"image mode {image.mode!r} is not a supported 8-bit mode")
