"""Bounded decoding of the native extractor's 8-bit grayscale PNG stages."""

from __future__ import annotations

import os
import warnings
from pathlib import Path

from PIL import Image, UnidentifiedImageError

MAX_FRAME_SIDE = 16_384
MAX_FRAME_PIXELS = 16_777_216
MAX_IMAGE_BYTES = 128 * 1024 * 1024


def load_image(path: Path, expected_size: tuple[int, int]) -> Image.Image:
    """Return an owned grayscale image; the caller must close the returned copy.

    Format, dimensions and 8-bit sample mode are checked before full decoding.
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


def _validate_image(image: Image.Image, expected_size: tuple[int, int]) -> None:
    if image.format != "PNG":
        raise ValueError("native image stages must be PNG")
    width, height = image.size
    if width * height > MAX_FRAME_PIXELS or max(width, height) > MAX_FRAME_SIDE:
        raise ValueError("image exceeds the supported pixel dimensions")
    if image.size != expected_size:
        raise ValueError("decoded image dimensions do not match the frame")
    if image.mode != "L":
        raise ValueError("native image stages must be 8-bit grayscale")
