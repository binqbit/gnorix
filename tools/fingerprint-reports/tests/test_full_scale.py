"""Full scale and coverage validated against explicit synthetic pixel topology."""

import copy
import math
import sys
import unittest
from contextlib import ExitStack

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.text import Text
from PIL import Image

from gnorix_reports.full_scale import (
    render_full_scale,
    validate_full_scale,
    validate_measurement_maps,
)


def evidence(measure_diagonal=True):
    observations = [
        {
            "link": [84, 85],
            "start": [4.5, 3],
            "end": [4.5, 5],
            "represented_length_px": 1,
            "mean_gray": 80,
        },
    ]
    if measure_diagonal:
        observations.append(
            {
                "link": [210, 231],
                "start": [10.5, 8.5],
                "end": [10.5, 12.5],
                "represented_length_px": math.sqrt(2),
                "mean_gray": 120,
            },
        )
    widths = [math.dist(item["start"], item["end"]) for item in observations]
    measured = sum(item["represented_length_px"] for item in observations)
    unit = (
        sum(width * item["represented_length_px"] for width, item in zip(widths, observations))
        / measured
    )
    bands = []
    for tolerance in (0.1, 0.2, 0.3):
        count = sum(
            abs(width - unit) <= tolerance * unit + 8 * sys.float_info.epsilon * max(width, unit)
            for width in widths
        )
        bands.append(
            {
                "relative_tolerance": tolerance,
                "accepted_measurements": count,
                "total_measurements": len(widths),
                "fraction": count / len(widths),
            }
        )
    return {
        "method": "raw-centerline-link-width",
        "unit_px": unit,
        "observations": observations,
        "width_consistency": bands,
        "coverage": {
            "total_length_px": 1 + math.sqrt(2),
            "measured_length_px": measured,
            "total_links": 2,
            "measured_links": len(observations),
            "isolated_pixels": 1,
        },
    }


class FullScaleTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)

    def tearDown(self):
        self.assertEqual(plt.get_fignums(), [])

    def images(self, pixels=((4, 4), (5, 4), (10, 10), (11, 11), (18, 18)), unsupported=()):
        skeleton = np.full((20, 20), 255, dtype=np.uint8)
        support = np.full((20, 20), 255, dtype=np.uint8)
        for x, y in pixels:
            skeleton[y, x] = 0
        for x, y in unsupported:
            support[y, x] = 0
        arrays = {
            "measurement_skeleton": skeleton,
            "pattern_support": support,
            "source": np.arange(400, dtype=np.uint8).reshape(20, 20),
        }
        result = {}
        for key, array in arrays.items():
            image = Image.fromarray(array)
            self.stack.callback(image.close)
            result[key] = image
        return result

    def test_unit_uses_arclength_weights_but_width_consistency_counts_observations(self):
        raw = evidence()
        scale = validate_full_scale(raw, 20, 20)
        self.assertAlmostEqual(scale.unit_px, (2 + 4 * math.sqrt(2)) / (1 + math.sqrt(2)))
        self.assertNotEqual(scale.unit_px, 3)
        self.assertEqual(
            [band.accepted_measurements for band in scale.width_consistency], [0, 0, 1]
        )
        self.assertEqual(scale.width_consistency[-1].fraction, 0.5)
        images = self.images()
        validate_measurement_maps(scale, images)
        figure = render_full_scale(scale, images)
        try:
            histogram = next(axis for axis in figure.axes if "length-weighted" in axis.get_title())
            self.assertAlmostEqual(
                sum(patch.get_height() for patch in histogram.patches), 1 + math.sqrt(2)
            )
        finally:
            plt.close(figure)
        raw["unit_px"] = 3
        with self.assertRaisesRegex(ValueError, "unit_px"):
            validate_full_scale(raw, 20, 20)

    def test_unmeasured_links_and_isolates_remain_in_total_coverage_and_weighted_histogram(self):
        scale = validate_full_scale(evidence(False), 20, 20)
        images = self.images(unsupported=((10, 10), (11, 11)))
        validate_measurement_maps(scale, images)
        source = np.asarray(images["source"]).copy()
        figure = render_full_scale(scale, images, alpha=0.4)
        try:
            labels = " ".join(text.get_text() for text in figure.findobj(Text))
            self.assertIn("Measured links: 1 / 2", labels)
            self.assertIn("Isolated pixels: 1 (zero length)", labels)
            self.assertIn("Unmeasured: 1.41421 px", labels)
            self.assertIn("1 R = 2", labels)
            histogram = next(axis for axis in figure.axes if "length-weighted" in axis.get_title())
            self.assertAlmostEqual(sum(patch.get_height() for patch in histogram.patches), 1)
            self.assertEqual(figure.axes[0].images[0].get_alpha(), 1)
            self.assertEqual(figure.axes[0].collections[0].get_alpha(), 0.4)
            np.testing.assert_array_equal(np.asarray(images["source"]), source)
        finally:
            plt.close(figure)

    def test_rejects_duplicate_non_neighbor_wrong_weight_and_nonlocal_span_observations(self):
        changes = (
            lambda raw: raw["observations"].append(copy.deepcopy(raw["observations"][0])),
            lambda raw: raw["observations"][0].update(link=[85, 84]),
            lambda raw: raw["observations"][0].update(link=[84, 84]),
            lambda raw: raw["observations"][0].update(link=[79, 80]),
            lambda raw: raw["observations"][0].update(link=[84, 86]),
            lambda raw: raw["observations"][0].update(link=[84, True]),
            lambda raw: raw["observations"][0].update(represented_length_px=0.5),
            lambda raw: raw["observations"][0].update(start=[5, 3], end=[5, 5]),
            lambda raw: raw["observations"][0].update(start=[4.5, 6], end=[4.5, 8]),
            lambda raw: raw["observations"][0].update(start=[4.5, 3], end=[4.5, 3]),
            lambda raw: raw["observations"][0].update(start=[-1, 3]),
            lambda raw: raw["observations"][0].update(mean_gray=256),
        )
        for change in changes:
            raw = evidence()
            change(raw)
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                validate_full_scale(raw, 20, 20)

    def test_rejects_false_coverage_missing_empty_nonfinite_and_wrong_width_counts(self):
        changes = (
            lambda raw: raw.update(method="bootstrap"),
            lambda raw: raw.update(observations=[]),
            lambda raw: raw.update(unit_px=float("nan")),
            lambda raw: raw["coverage"].update(measured_links=1),
            lambda raw: raw["coverage"].update(measured_length_px=2),
            lambda raw: raw["coverage"].update(total_links=1),
            lambda raw: raw["coverage"].update(total_length_px=1),
            lambda raw: raw["coverage"].update(isolated_pixels=-1),
            lambda raw: raw["width_consistency"][2].update(accepted_measurements=2, fraction=1),
        )
        for change in changes:
            raw = evidence()
            change(raw)
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                validate_full_scale(raw, 20, 20)
        oversized = evidence()
        oversized["observations"] *= 9
        with self.assertRaisesRegex(ValueError, "four links per pixel"):
            validate_full_scale(oversized, 2, 2)

    def test_actual_maps_reject_missing_links_wrong_counts_support_and_nonbinary_pixels(self):
        scale = validate_full_scale(evidence(), 20, 20)
        with self.assertRaisesRegex(ValueError, "pattern support"):
            validate_measurement_maps(scale, self.images(unsupported=((4, 4),)))
        with self.assertRaisesRegex(ValueError, "absent"):
            validate_measurement_maps(scale, self.images(pixels=((4, 4), (10, 10), (11, 11))))
        with self.assertRaisesRegex(ValueError, "coverage"):
            validate_measurement_maps(
                scale, self.images(pixels=((4, 4), (5, 4), (10, 10), (11, 11)))
            )
        images = self.images()
        images["pattern_support"].putpixel((0, 0), 128)
        with self.assertRaisesRegex(ValueError, "0 or 255"):
            validate_measurement_maps(scale, images)

    def test_topology_suppresses_corner_shortcuts_and_filled_cell_top_link(self):
        raw = evidence(False)
        # A filled 2x2 cell keeps both vertical sides and its bottom edge.
        # The top edge and both redundant diagonals must not appear.
        raw["observations"][0].update(link=[104, 105], start=[4.5, 4], end=[4.5, 6])
        raw["coverage"].update(total_length_px=3, total_links=3, isolated_pixels=0)
        scale = validate_full_scale(raw, 20, 20)
        images = self.images(pixels=((4, 4), (5, 4), (4, 5), (5, 5)))
        validate_measurement_maps(scale, images)
        for link, start, end in (
            ([84, 85], [4.5, 3], [4.5, 5]),
            ([84, 105], [4.5, 3.5], [4.5, 5.5]),
        ):
            changed = copy.deepcopy(raw)
            length = math.dist((link[0] % 20, link[0] // 20), (link[1] % 20, link[1] // 20))
            changed["observations"][0].update(
                link=link, start=start, end=end, represented_length_px=length
            )
            changed["coverage"]["measured_length_px"] = length
            with self.subTest(link=link), self.assertRaisesRegex(ValueError, "topology"):
                validate_measurement_maps(validate_full_scale(changed, 20, 20), images)


if __name__ == "__main__":
    unittest.main()
