import math
import unittest

from gnorix_reports.model import Frame
from gnorix_reports.options import OverlayOptions


class OverlayOptionsTests(unittest.TestCase):
    def test_default_is_half_transparent_with_no_inferred_tolerance(self):
        options = OverlayOptions()
        self.assertEqual(options.alpha, 0.5)
        self.assertIsNone(options.tolerance_radius)

    def test_zero_opacity_and_zero_tolerance_are_explicit_values(self):
        options = OverlayOptions(alpha=0, tolerance_radius=0)
        self.assertEqual(options.alpha, 0)
        self.assertEqual(options.tolerance_radius, 0)

    def test_invalid_opacity_radii_and_crop_are_rejected(self):
        for alpha in (-0.1, 1.1, math.nan, math.inf, True, 10**1000):
            with self.subTest(alpha=str(alpha)), self.assertRaises(ValueError):
                OverlayOptions(alpha=alpha)
        for radius in (-1, 0, math.inf, math.nan, False):
            with self.subTest(point_radius=radius), self.assertRaises(ValueError):
                OverlayOptions(point_radius=radius)
        for radius in (-1, math.inf, math.nan, True):
            with self.subTest(tolerance_radius=radius), self.assertRaises(ValueError):
                OverlayOptions(tolerance_radius=radius)
        for crop in ((0, 0, 0, 10), (-1, 0, 10, 10), (0, 0, 10, math.inf), (0, 0, 10), 5):
            with self.subTest(crop=crop), self.assertRaises(ValueError):
                OverlayOptions(crop=crop)

    def test_crop_remains_in_original_frame_coordinates(self):
        frame = Frame("f", "Frame", 100, 80, None)
        options = OverlayOptions(crop=(20, 10, 40, 30))
        options.validate_frame(frame)
        self.assertEqual(options.crop, (20, 10, 40, 30))
        with self.assertRaisesRegex(ValueError, "within frame"):
            OverlayOptions(crop=(70, 0, 40, 30)).validate_frame(frame)
        with self.assertRaisesRegex(ValueError, "diagonal"):
            OverlayOptions(point_radius=1000).validate_frame(frame)

    def test_unrepresentable_crop_cannot_silently_expand_the_view(self):
        frame = Frame("f", "Frame", 100, 80, None)
        for crop in ((20, 10, 1e-300, 10), (0, 0, 1e-300, 10), (0, 0, 10, 1e-300)):
            with self.subTest(crop=crop), self.assertRaisesRegex(ValueError, "plotting scale"):
                OverlayOptions(crop=crop).validate_frame(frame)


if __name__ == "__main__":
    unittest.main()
