"""Display controls, independent of biometric acceptance or measurement policy."""

from __future__ import annotations

import math
from dataclasses import dataclass

from matplotlib.ticker import MaxNLocator

from .model import Frame


@dataclass(frozen=True, slots=True)
class OverlayOptions:
    alpha: float = 0.5
    point_radius: float = 3.0
    tolerance_radius: float | None = None
    crop: tuple[float, float, float, float] | None = None

    def __post_init__(self) -> None:
        if not _finite(self.alpha) or not 0 <= self.alpha <= 1:
            raise ValueError("overlay alpha must be a finite number from 0 to 1")
        if not _finite(self.point_radius) or self.point_radius <= 0:
            raise ValueError("point radius must be finite and positive (image pixels)")
        if self.tolerance_radius is not None and (
            not _finite(self.tolerance_radius) or self.tolerance_radius < 0
        ):
            raise ValueError("tolerance radius must be finite and nonnegative (image pixels)")
        if self.crop is not None:
            if (
                not isinstance(self.crop, tuple)
                or len(self.crop) != 4
                or not all(_finite(value) for value in self.crop)
            ):
                raise ValueError("crop must contain finite x, y, width and height")
            x, y, width, height = self.crop
            if x < 0 or y < 0 or width <= 0 or height <= 0:
                raise ValueError("crop x/y must be nonnegative and width/height positive")

    def validate_frame(self, frame: Frame) -> None:
        if self.point_radius > math.hypot(frame.width, frame.height):
            raise ValueError(f"point radius exceeds the diagonal of frame {frame.id!r}")
        if self.crop is not None:
            x, y, width, height = self.crop
            if x + width > frame.width or y + height > frame.height:
                raise ValueError(f"crop must lie within frame {frame.id!r}")
            right, bottom = x + width, y + height
            locator = MaxNLocator()
            if (
                locator.nonsingular(x, right) != (x, right)
                or locator.nonsingular(y, bottom) != (y, bottom)
                or not math.isfinite(width / height)
                or width / height == 0
            ):
                raise ValueError("crop is too narrow to preserve its requested plotting scale")


def _finite(value: object) -> bool:
    try:
        return type(value) in (int, float) and math.isfinite(value)
    except OverflowError:
        return False
