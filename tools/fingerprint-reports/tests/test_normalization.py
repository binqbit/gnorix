"""Local reference diagnostics tested with explicitly synthetic grayscale maps."""

import copy
import unittest
from contextlib import ExitStack

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.text import Text
from PIL import Image

from gnorix_reports.normalization import (
    render_normalization_analysis,
    validate_normalization,
    validate_palette,
)


def metadata():
    return {
        "radius_px": 3,
        "method": "local-two-class-midpoint",
        "reference_encoding": "rounded-u8",
    }


class NormalizationTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)

    def tearDown(self):
        self.assertEqual(plt.get_fignums(), [])

    def images(self, ridge=None, valley=None, support=None):
        source = np.array([[0, 64, 128, 255], [60, 100, 140, 200]], dtype=np.uint8)
        arrays = {
            "source": source,
            "normalized": source,
            "ridge_level": ridge if ridge is not None else np.full((2, 4), 80),
            "valley_level": valley if valley is not None else np.full((2, 4), 180),
            "normalization_support": support if support is not None else np.full((2, 4), 255),
        }
        result = {}
        for key, array in arrays.items():
            image = Image.fromarray(np.asarray(array, dtype=np.uint8))
            self.stack.callback(image.close)
            result[key] = image
        return result

    def test_radius_and_reference_encoding_follow_current_producer_contract(self):
        self.assertEqual(validate_normalization(metadata(), 5).radius_px, 3)
        for key, value in (
            ("radius_px", 2),
            ("radius_px", 3.0),
            ("radius_px", True),
            ("method", "global"),
            ("reference_encoding", "float"),
        ):
            raw = copy.deepcopy(metadata())
            raw[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                validate_normalization(raw, 5)
        with self.assertRaises(ValueError):
            validate_normalization(None, 5)

    def test_support_distinguishes_valid_black_and_quantized_equal_references_from_missing(self):
        images = self.images(
            ridge=[[0, 42, 0, 255], [0, 42, 0, 255]],
            valley=[[1, 42, 0, 255], [1, 42, 0, 255]],
            support=[[255, 255, 0, 255], [255, 255, 0, 255]],
        )
        palette = validate_palette(images)
        self.assertTrue(palette.supported[0, 0])
        self.assertTrue(palette.supported[0, 1])
        self.assertFalse(palette.supported[0, 2])
        self.assertEqual(palette.ridge[0, 0], 0)
        self.assertEqual(palette.valley[0, 1] - palette.ridge[0, 1], 0)

    def test_rejects_inverted_references_nonbinary_support_and_nonzero_missing_values(self):
        for options in (
            {"ridge": np.full((2, 4), 181)},
            {"support": np.full((2, 4), 127)},
            {"support": np.zeros((2, 4))},
        ):
            with self.subTest(options=options), self.assertRaises(ValueError):
                validate_palette(self.images(**options))
        images = self.images()
        images["ridge_level"] = self.stack.enter_context(images["ridge_level"].convert("RGB"))
        with self.assertRaisesRegex(ValueError, "8-bit grayscale"):
            validate_palette(images)

    def test_maps_share_absolute_scale_and_cross_section_uses_original_source_coordinates(self):
        images = self.images(
            ridge=[[20, 50, 0, 140], [20, 50, 0, 140]],
            valley=[[80, 130, 0, 220], [80, 130, 0, 220]],
            support=[[255, 255, 0, 255], [255, 255, 0, 255]],
        )
        source = np.asarray(images["source"]).copy()
        figure = render_normalization_analysis(
            validate_normalization(metadata(), 5),
            validate_palette(images),
            images,
        )
        try:
            source_axis, dark, light, _, midpoint, spread, normalized, profile = figure.axes[:8]
            for axis in (source_axis, dark, light, midpoint, spread, normalized):
                self.assertEqual(axis.images[0].get_clim(), (0, 255))
                self.assertEqual(tuple(axis.images[0].get_extent()), (-0.5, 3.5, 1.5, -0.5))
            np.testing.assert_array_equal(source_axis.images[0].get_array(), source)
            self.assertEqual(source_axis.images[0].get_alpha(), 1)
            self.assertTrue(dark.images[0].get_array().mask[0, 2])
            self.assertFalse(dark.images[0].get_array().mask[0, 0])
            self.assertEqual(midpoint.images[0].get_array()[0, 0], 50)
            self.assertEqual(spread.images[0].get_array()[0, 0], 60)
            np.testing.assert_array_equal(profile.lines[0].get_xdata(), [0, 1, 2, 3])
            np.testing.assert_array_equal(profile.lines[0].get_ydata(), source[1])
            np.testing.assert_allclose(profile.lines[1].get_ydata(), [20, 50, np.nan, 140])
            np.testing.assert_allclose(profile.lines[3].get_ydata(), [50, 90, np.nan, 180])
            np.testing.assert_array_equal(np.asarray(images["source"]), source)
            labels = " ".join(text.get_text() for text in figure.findobj(Text))
            self.assertIn("y = 1 px", labels)
            self.assertIn("6 / 8 pixels", labels)
            self.assertIn("unrounded references", labels)
            self.assertIn("not anatomical ground truth", labels)
        finally:
            plt.close(figure)

    def test_absent_reference_support_remains_missing_in_maps_and_cross_section(self):
        empty = np.zeros((2, 4), dtype=np.uint8)
        images = self.images(ridge=empty, valley=empty, support=empty)
        figure = render_normalization_analysis(
            validate_normalization(metadata(), 5),
            validate_palette(images),
            images,
        )
        try:
            self.assertTrue(figure.axes[1].images[0].get_array().mask.all())
            self.assertTrue(np.isnan(figure.axes[7].lines[1].get_ydata()).all())
            self.assertIn("0 / 8 pixels", figure._suptitle.get_text())
        finally:
            plt.close(figure)


if __name__ == "__main__":
    unittest.main()
