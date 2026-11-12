"""Scale evidence validation using explicit synthetic crossing measurements."""

import copy
import math
import statistics
import sys
import unittest

import matplotlib.pyplot as plt
from matplotlib.text import Text
from PIL import Image

from gnorix_reports.scale import display_profiles, render_scale_analysis, validate_scale


def evidence(groups=((2, 2, 2),), gaps=None, origins=None):
    gaps = gaps or [(3, 3)] * len(groups)
    origins = origins or [(1, 10 + 10 * index) for index in range(len(groups))]
    profiles, widths, valleys, periods = [], [], [], []
    for group, group_gaps, (x, y) in zip(groups, gaps, origins, strict=True):
        ridges = []
        for index, width in enumerate(group):
            ridges.append({"start": [x, y], "end": [x + width, y]})
            x += width + (group_gaps[index] if index < 2 else 0)
        profiles.append(
            {
                "ridges": ridges,
                "ridge_mean_gray": [80, 80, 80],
                "valley_mean_gray": [180, 180],
            }
        )
        widths.extend(group)
        valleys.extend(group_gaps)
        periods.extend(
            group[index] / 2 + group_gaps[index] + group[index + 1] / 2 for index in (0, 1)
        )
    median = statistics.median(widths)
    period = statistics.median(periods)
    consistency = []
    for tolerance in (0.1, 0.2, 0.3):
        accepted = sum(
            abs(width - median)
            <= tolerance * median + 8 * sys.float_info.epsilon * max(width, median)
            for width in widths
        )
        consistency.append(
            {
                "relative_tolerance": tolerance,
                "accepted_measurements": accepted,
                "total_measurements": len(widths),
                "fraction": accepted / len(widths),
            }
        )
    return {
        "ridge_width_px": median,
        "valley_width_px": statistics.median(valleys),
        "ridge_period_px": period,
        "ridge_width_mad_px": statistics.median(abs(width - median) for width in widths),
        "ridge_period_mad_px": statistics.median(abs(value - period) for value in periods),
        "mean_ridge_width_px": statistics.fmean(widths),
        "mean_valley_width_px": statistics.fmean(valleys),
        "mean_ridge_gray": 80,
        "mean_valley_gray": 180,
        "examined_locations": len(groups) + 2,
        "profiles": profiles,
        "width_consistency": consistency,
    }


class ScaleTests(unittest.TestCase):
    def tearDown(self):
        self.assertEqual(plt.get_fignums(), [])

    def test_medians_and_unscaled_mads_use_every_span_and_both_gaps(self):
        raw = evidence(((1, 5, 8), (2, 3, 4)), ((1, 3), (2, 6)))
        scale = validate_scale(raw, 100, 60, 32)
        self.assertEqual(scale.widths, (1, 5, 8, 2, 3, 4))
        self.assertEqual(scale.periods, (4, 9.5, 4.5, 9.5))
        self.assertEqual(scale.ridge_width_px, 3.5)
        self.assertEqual(scale.valley_width_px, 2.5)
        self.assertEqual(scale.ridge_period_px, 7)
        self.assertEqual(scale.ridge_width_mad_px, 1.5)
        self.assertEqual(scale.ridge_period_mad_px, 2.5)
        for name in (
            "ridge_width_px",
            "valley_width_px",
            "ridge_period_px",
            "ridge_width_mad_px",
            "ridge_period_mad_px",
            "mean_ridge_width_px",
            "mean_valley_width_px",
            "mean_ridge_gray",
            "mean_valley_gray",
        ):
            changed = copy.deepcopy(raw)
            changed[name] += 1
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "supplied profiles"):
                validate_scale(changed, 100, 60, 32)

    def test_width_consistency_boundaries_are_inclusive_and_share_one_denominator(self):
        scale = validate_scale(evidence(((7, 8, 9), (10, 10, 10), (11, 12, 13))), 100, 60, 32)
        self.assertEqual(scale.ridge_width_px, 10)
        self.assertEqual(
            [band.accepted_measurements for band in scale.width_consistency], [5, 7, 9]
        )
        self.assertEqual([band.total_measurements for band in scale.width_consistency], [9, 9, 9])
        self.assertEqual([band.fraction for band in scale.width_consistency], [5 / 9, 7 / 9, 1])
        outside = validate_scale(evidence(((9, 10, 11.00001),)), 100, 60, 32)
        self.assertEqual(outside.width_consistency[0].accepted_measurements, 2)

    def test_gray_summary_weights_observations_by_span_length_not_span_count(self):
        raw = evidence(((1, 2, 3),), ((2, 4),))
        raw["profiles"][0].update(ridge_mean_gray=[0, 30, 90], valley_mean_gray=[100, 200])
        raw.update(mean_ridge_gray=55, mean_valley_gray=1000 / 6)
        scale = validate_scale(raw, 100, 60, 32)
        self.assertEqual(scale.mean_ridge_width_px, 2)
        self.assertEqual(scale.mean_valley_width_px, 3)
        self.assertEqual(scale.mean_ridge_gray, 55)
        self.assertEqual(scale.ridge_grays, (0, 30, 90))
        self.assertAlmostEqual(scale.mean_valley_gray, 1000 / 6)
        for field, unweighted in (("mean_ridge_gray", 40), ("mean_valley_gray", 150)):
            changed = copy.deepcopy(raw)
            changed[field] = unweighted
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "supplied profiles"):
                validate_scale(changed, 100, 60, 32)

    def test_gray_evidence_requires_exact_counts_finite_values_and_byte_range(self):
        for field, values in (
            ("ridge_mean_gray", [1, 2]),
            ("valley_mean_gray", [1, 2, 3]),
            ("ridge_mean_gray", [0, 255, float("nan")]),
            ("ridge_mean_gray", [0, 255, -0.01]),
            ("valley_mean_gray", [0, 255.01]),
            ("ridge_mean_gray", None),
        ):
            raw = evidence()
            raw["profiles"][0][field] = values
            with self.subTest(field=field, values=values), self.assertRaises(ValueError):
                validate_scale(raw, 100, 60, 32)
        raw = evidence()
        raw["profiles"][0].update(ridge_mean_gray=[0, 0, 0], valley_mean_gray=[255, 255])
        raw.update(mean_ridge_gray=0, mean_valley_gray=255)
        self.assertEqual(validate_scale(raw, 100, 60, 32).mean_ridge_gray, 0)

    def test_zero_mad_is_valid_and_roundoff_does_not_exclude_exact_decimal_boundary(self):
        constant = validate_scale(evidence(), 100, 60, 32)
        self.assertEqual(constant.ridge_width_mad_px, 0)
        self.assertEqual(constant.ridge_period_mad_px, 0)
        boundary = validate_scale(evidence(((33.3, 37, 40.7),)), 150, 60, 32)
        self.assertEqual(boundary.width_consistency[0].accepted_measurements, 3)

    def test_rotated_collinear_evidence_preserves_scale(self):
        raw = evidence(((1, 5, 8), (2, 3, 4)), ((1, 3), (2, 6)))
        cosine, sine = math.cos(0.63), math.sin(0.63)
        for profile in raw["profiles"]:
            for span in profile["ridges"]:
                for endpoint in ("start", "end"):
                    x, y = span[endpoint]
                    span[endpoint] = [50 + cosine * x - sine * y, 50 + sine * x + cosine * y]
        scale = validate_scale(raw, 150, 150, 32)
        self.assertEqual(scale.ridge_width_px, 3.5)
        self.assertEqual(scale.ridge_period_px, 7)

    def test_rejects_missing_empty_nonfinite_or_out_of_budget_evidence(self):
        changes = (
            lambda raw: raw.update(profiles=[]),
            lambda raw: raw.update(examined_locations=0),
            lambda raw: raw.update(examined_locations=True),
            lambda raw: raw.update(examined_locations=6033),
            lambda raw: raw.update(ridge_width_px=float("nan")),
            lambda raw: raw.update(ridge_period_mad_px=-1),
        )
        for change in changes:
            raw = evidence()
            change(raw)
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                validate_scale(raw, 100, 60, 32)
        for cap in (0, True, 100_001):
            with self.subTest(cap=cap), self.assertRaises(ValueError):
                validate_scale(evidence(), 100, 60, cap)
        with self.assertRaises(ValueError):
            validate_scale(None, 100, 60, 32)
        raw = evidence(((2, 2, 2), (2, 2, 2)))
        raw["examined_locations"] = 1
        with self.assertRaises(ValueError):
            validate_scale(raw, 100, 60, 32)

    def test_bootstrap_fallback_attempts_can_exceed_grid_budget_but_remain_source_bounded(self):
        raw = evidence()
        for examined in (33, 6032):
            raw["examined_locations"] = examined
            with self.subTest(examined=examined):
                self.assertEqual(validate_scale(raw, 100, 60, 32).examined_locations, examined)
        raw["examined_locations"] = 6033
        with self.assertRaisesRegex(ValueError, "examined_locations"):
            validate_scale(raw, 100, 60, 32)
        raw = evidence(((2, 2, 2), (2, 2, 2)))
        with self.assertRaisesRegex(ValueError, "initial grid budget"):
            validate_scale(raw, 100, 60, 1)

    def test_rejects_noncollinear_reversed_overlapping_and_invalid_spans(self):
        changes = (
            lambda ridges: ridges.pop(),
            lambda ridges: ridges[0].update(end=ridges[0]["start"]),
            lambda ridges: ridges[1].update(start=[6, 11]),
            lambda ridges: ridges[1].update(start=[8, 10], end=[6, 10]),
            lambda ridges: ridges[1].update(start=[3, 10]),
            lambda ridges: ridges[1].update(start=[2, 10]),
            lambda ridges: ridges[2].update(end=[101, 10]),
            lambda ridges: ridges[2].update(end=[float("inf"), 10]),
        )
        for change in changes:
            raw = evidence()
            change(raw["profiles"][0]["ridges"])
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                validate_scale(raw, 100, 60, 32)

    def test_rejects_false_counts_fractions_and_nonstandard_bands(self):
        changes = (
            lambda rows: rows[0].update(accepted_measurements=0, fraction=0),
            lambda rows: rows[0].update(total_measurements=1),
            lambda rows: rows[0].update(fraction=0),
            lambda rows: rows[0].update(relative_tolerance=0.15),
            lambda rows: rows[1].update(relative_tolerance=0.1),
            lambda rows: rows.pop(),
        )
        for change in changes:
            raw = evidence()
            change(raw["width_consistency"])
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                validate_scale(raw, 100, 60, 32)

    def test_spatial_display_subset_does_not_subsample_histograms_or_consistency_counts(self):
        origins = [
            (100 * column + 20 + shift, 100 * row + 20)
            for row in range(6)
            for column in range(6)
            for shift in (0, 1)
        ]
        raw = evidence(((2, 2, 2),) * len(origins), origins=origins)
        scale = validate_scale(raw, 600, 600, 100)
        selected = display_profiles(scale, 600, 600)
        self.assertEqual(len(selected), 36)
        self.assertTrue(all(profile in scale.profiles for profile in selected))
        with Image.new("L", (600, 600), color=127) as image:
            figure = render_scale_analysis(scale, image)
        try:
            overlay, widths, periods, gray, _ = figure.axes
            self.assertEqual(overlay.images[0].get_alpha(), 1)
            self.assertEqual(len(overlay.collections[1].get_segments()), 108)
            self.assertEqual(sum(bar.get_height() for bar in widths.patches), 216)
            self.assertEqual(sum(bar.get_height() for bar in periods.patches), 144)
            self.assertEqual(
                len(gray.patches), 4
            )  # Two histogram paths and two grayscale swatches.
            self.assertEqual(tuple(gray.get_xlim()), (0, 255))
            self.assertEqual(gray.patches[-2].get_facecolor(), (80 / 255, 80 / 255, 80 / 255, 1))
            self.assertEqual(scale.width_consistency[0].total_measurements, 216)
            labels = " ".join(text.get_text() for text in figure.findobj(Text))
            self.assertIn("36 of 72 measured triples", labels)
            self.assertIn("Thickness consistency", labels)
            self.assertIn("not distinct ridge counts", labels)
            self.assertIn("All 216 dark / 144 light means", labels)
            self.assertIn("Arithmetic mean width", labels)
        finally:
            plt.close(figure)


if __name__ == "__main__":
    unittest.main()
