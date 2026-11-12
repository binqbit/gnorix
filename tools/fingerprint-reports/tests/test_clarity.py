"""Native clarity is bounded, explicitly supported and displayed without rescaling."""

import unittest

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.collections import LineCollection
from matplotlib.colors import to_rgba
from PIL import Image

from gnorix_reports.clarity import (
    IMAGE_KEYS,
    UNKNOWN_COLOR,
    clarity_lines,
    render_clarity_analysis,
    validate_clarity,
    validate_clarity_maps,
    validate_segment_clarity,
)


def metadata(supported=12, mean=1):
    return {
        "method": "local-ridge-separability-v1",
        "period_px": 5,
        "grid_spacing_px": 1,
        "score": "interpolated(contrast * separation * repetition)",
        "segment_sampling": "bilinear-map-line-mean",
        "encoding": "rounded-u8",
        "summary": {"supported_pixels": supported, "total_pixels": 12, "mean_score": mean},
    }


class ClarityTests(unittest.TestCase):
    def tearDown(self):
        plt.close("all")

    def maps(self, **values):
        images = {}
        for key in IMAGE_KEYS:
            pixels = np.full((3, 4), values.get(key, 255), dtype=np.uint8)
            images[key] = Image.fromarray(pixels)
            self.addCleanup(images[key].close)
        return images

    def test_requires_native_metadata_and_consistent_summary(self):
        self.assertEqual(validate_clarity(metadata(), 5, 4, 3).mean_score, 1)
        changes = (
            lambda d: d.update(method="invented"),
            lambda d: d.update(period_px=6),
            lambda d: d.update(period_px=float("nan")),
            lambda d: d.update(grid_spacing_px=True),
            lambda d: d.update(grid_spacing_px=2),
            lambda d: d.update(segment_sampling="midpoint"),
            lambda d: d.update(score="contrast * separation * repetition"),
            lambda d: d.update(encoding="float"),
            lambda d: d["summary"].update(supported_pixels=13),
            lambda d: d["summary"].update(total_pixels=13),
            lambda d: d["summary"].update(mean_score=None),
            lambda d: d["summary"].update(mean_score=1.01),
            lambda d: d["summary"].update(supported_pixels=0),
        )
        for change in changes:
            data = metadata()
            change(data)
            with self.subTest(data=data), self.assertRaises(ValueError):
                validate_clarity(data, 5, 4, 3)

    def test_product_validation_respects_independent_byte_rounding(self):
        images = self.maps(
            clarity=32, clarity_contrast=128, clarity_separation=128, clarity_repetition=128
        )
        clarity = validate_clarity(metadata(mean=0.125), 5, 4, 3)
        maps = validate_clarity_maps(clarity, images)
        np.testing.assert_array_equal(maps.score, np.full((3, 4), 32 / 255))
        images["clarity"].putpixel((1, 1), 100)
        with self.assertRaisesRegex(ValueError, "component product"):
            validate_clarity_maps(clarity, images)

    def test_known_zero_is_distinct_from_missing_and_support_is_binary(self):
        known = self.maps(clarity=0, clarity_contrast=0)
        maps = validate_clarity_maps(validate_clarity(metadata(mean=0), 5, 4, 3), known)
        self.assertTrue(maps.supported.all())
        missing = self.maps(**dict.fromkeys(IMAGE_KEYS, 0))
        empty = validate_clarity(metadata(supported=0, mean=None), 5, 4, 3)
        self.assertFalse(validate_clarity_maps(empty, missing).supported.any())
        for key in IMAGE_KEYS[:-1]:
            missing[key].putpixel((0, 0), 1)
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "unsupported"):
                validate_clarity_maps(empty, missing)
            missing[key].putpixel((0, 0), 0)
        missing["clarity_support"].putpixel((0, 0), 1)
        with self.assertRaisesRegex(ValueError, "0 or 255"):
            validate_clarity_maps(empty, missing)

    def anchored_maps(self, supported=12, mean=0.07):
        data = metadata(supported, mean)
        data.update(period_px=8, grid_spacing_px=2)
        clarity = validate_clarity(data, 8, 4, 3)
        images = self.maps(
            clarity=[5, 15, 26, 26],
            clarity_contrast=[255, 140, 26, 26],
            clarity_separation=[26, 140, 255, 255],
            clarity_repetition=[51, 153, 255, 255],
        )
        return clarity, images

    def test_opposing_weak_anchors_cannot_invent_a_stronger_midpoint(self):
        # A=(1,.1,.2) scores .02; B=(.1,1,1) scores .1.
        # Interpolated scores give .06 at the midpoint, not .55*.55*.6=.1815.
        clarity, images = self.anchored_maps()
        maps = validate_clarity_maps(clarity, images)
        self.assertAlmostEqual(maps.score[1, 1], 15 / 255)
        self.assertGreater(
            maps.contrast[1, 1] * maps.separation[1, 1] * maps.repetition[1, 1], 0.18
        )
        images["clarity"].putpixel((1, 1), 46)
        with self.assertRaisesRegex(ValueError, "interpolated grid-anchor scores"):
            validate_clarity_maps(clarity, images)
        images["clarity"].putpixel((1, 1), 15)
        # The final column at x=3 is also an anchor, despite spacing=2.
        images["clarity"].putpixel((3, 2), 5)
        with self.assertRaisesRegex(ValueError, "grid-anchor component product"):
            validate_clarity_maps(clarity, images)

    def test_only_positive_weight_anchors_are_required_and_capture_can_reduce_support(self):
        clarity, images = self.anchored_maps(supported=11, mean=(0.84 - 0.02) / 11)
        for image in images.values():
            image.putpixel((0, 0), 0)
        with self.assertRaisesRegex(ValueError, "positive-weight grid anchor"):
            validate_clarity_maps(clarity, images)

        # Missing top-left anchor excludes four pixels, not exact other anchors.
        data = metadata(supported=8, mean=0.68 / 8)
        data.update(period_px=8, grid_spacing_px=2)
        for image in images.values():
            for point in ((1, 0), (0, 1), (1, 1)):
                image.putpixel(point, 0)
        maps = validate_clarity_maps(validate_clarity(data, 8, 4, 3), images)
        self.assertTrue(maps.supported[0, 2])
        self.assertTrue(maps.supported[2, 0])

        clarity, images = self.anchored_maps(supported=11, mean=(0.84 - 0.06) / 11)
        for image in images.values():
            image.putpixel((1, 1), 0)
        maps = validate_clarity_maps(clarity, images)
        self.assertFalse(maps.supported[1, 1])

    def test_summary_matches_native_map_support_and_quantized_mean(self):
        images = self.maps()
        for supported, mean, error in ((11, 1, "supported_pixels"), (12, 0.9, "mean_score")):
            summary = validate_clarity(metadata(supported, mean), 5, 4, 3)
            with self.subTest(supported=supported), self.assertRaisesRegex(ValueError, error):
                validate_clarity_maps(summary, images)

    def test_segment_scores_have_exact_count_and_reject_nonfinite_or_out_of_range(self):
        self.assertEqual(validate_segment_clarity([0, None, 1], 4, False), (0, None, 1))
        self.assertEqual(validate_segment_clarity([0, None, 1], 3, True), (0, None, 1))
        for invalid in (
            None,
            [0],
            [0, 1, 2],
            [0, -0.1, 1],
            [0, True, 1],
            [0, float("nan"), 1],
            [0, float("inf"), 1],
        ):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                validate_segment_clarity(invalid, 4, False)

    def test_rgba_maps_quality_without_changing_geometry_or_losing_cycle_closure(self):
        points = ((1, 1), (3, 1), (3, 2), (1, 1))
        collection = clarity_lines([(points, True, (0, 0.5, 1, None))], "#009e73", 0.5)
        np.testing.assert_array_equal(
            collection.get_segments(), list(zip(points, (*points[1:], points[0]), strict=True))
        )
        colors = collection.get_colors()
        np.testing.assert_allclose(colors[:, 3], [0.1, 0.1 + 0.4 * np.sqrt(0.5), 0.5, 0.1])
        np.testing.assert_allclose(colors[:3, :3], [to_rgba("#009e73")[:3]] * 3)
        np.testing.assert_allclose(colors[-1, :3], to_rgba(UNKNOWN_COLOR)[:3])
        hidden = clarity_lines([(points, False, (0, 1, None))], "red", 0)
        self.assertTrue((hidden.get_colors()[:, 3] == 0).all())

    def test_map_figure_retains_source_and_fixed_scales_for_components(self):
        images = self.maps(clarity=128, clarity_contrast=128)
        clarity = validate_clarity(metadata(mean=0.5), 5, 4, 3)
        maps = validate_clarity_maps(clarity, images)
        pixels = np.arange(12, dtype=np.uint8).reshape(3, 4)
        with Image.fromarray(pixels) as source:
            figure = render_clarity_analysis(clarity, maps, source, 0.4)
        for axis in figure.axes[:2]:
            np.testing.assert_array_equal(axis.images[0].get_array(), pixels)
            self.assertEqual(axis.images[0].get_alpha(), 1)
            self.assertEqual(tuple(axis.images[0].get_extent()), (-0.5, 3.5, 2.5, -0.5))
        self.assertEqual(figure.axes[1].images[1].get_alpha(), 0.4)
        for axis in (figure.axes[1], *figure.axes[3:6]):
            self.assertEqual(axis.images[-1].get_clim(), (0, 1))
        figure.canvas.draw()

    def test_dense_segment_colors_preserve_a_continuous_dash_pattern(self):
        figure, axis = plt.subplots(figsize=(8, 3), dpi=100)
        axis.set(xlim=(0, 100), ylim=(0, 3))
        points = tuple((index / 10, 1) for index in range(1001))
        segmented = clarity_lines(
            [(points, False, [1] * 1000)],
            "black",
            1,
            linewidths=2,
            linestyles="dashed",
        )
        reference = LineCollection(
            [((0, 2), (100, 2))],
            colors="black",
            linewidths=2,
            linestyles="dashed",
        )
        axis.add_collection(segmented)
        axis.add_collection(reference)
        figure.canvas.draw()
        pixels = np.asarray(figure.canvas.buffer_rgba())
        width, height = figure.canvas.get_width_height()
        rows = [height - round(axis.transData.transform((0, y))[1]) for y in (1, 2)]
        left = round(axis.transData.transform((0, 0))[0]) + 3
        right = min(width, round(axis.transData.transform((100, 0))[0]) - 3)
        ink = [pixels[row, left:right, 0] < 128 for row in rows]
        self.assertGreater(np.mean(~ink[0]), 0.2)
        self.assertLess(np.mean(ink[0] != ink[1]), 0.03)
        self.assertGreater(segmented.get_linestyles()[1][0], 0)


if __name__ == "__main__":
    unittest.main()
