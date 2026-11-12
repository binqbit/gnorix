"""Reconstructed contours retain evidence and distinguish accepted gap bridges."""

import copy
import unittest

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.collections import LineCollection
from matplotlib.colors import to_rgba
from matplotlib.text import Text
from PIL import Image
from test_structure import unchanged_contours

from gnorix_reports.contours import (
    render_contour_comparison,
    render_contour_overlay,
    validate_contours,
)


def fixture(accepted=True):
    raw = {
        "scale": {"unit_px": 4},
        "nodes": [
            {"id": i, "x": x, "y": 15, "kind": "ending", "touches_unobserved": False}
            for i, x in ((10, 3), (20, 15), (30, 18), (40, 31))
        ],
        "edges": [
            {"id": 5, "start": 10, "end": 20, "points": [[3, 15], [9, 14.5], [15, 15]]},
            {"id": 8, "start": 30, "end": 40, "points": [[18, 15], [24, 15.5], [31, 15]]},
        ],
        "cycles": [],
    }
    data = unchanged_contours(raw)
    data["joins"] = [
        {
            "id": 7,
            "start_node": 20,
            "end_node": 30,
            "points": [[15, 15], [16, 15], [17, 15], [18, 15]],
            "segment_clarity": [1, 1, 1],
            "gap_px": 3,
            "max_tangent_error_radians": 0.1,
            "evidence": {
                "mean_darkness": 0.8,
                "dark_fraction": 0.9,
                "mean_flank_contrast": 0.6,
                "longest_light_gap_px": 0.5,
            },
            "status": "accepted" if accepted else "image_unsupported",
        }
    ]
    if accepted:
        data["nodes"] = [data["nodes"][0], data["nodes"][-1]]
        data["edges"] = [
            {
                "id": 1,
                "start": 10,
                "end": 40,
                "points": [[3, 15], [9, 14.8], [15, 15], [18, 15], [24, 15.2], [31, 15]],
                "segment_clarity": [1] * 5,
                "source_edges": [5, 8],
                "source_cycles": [],
                "source_joins": [7],
            }
        ]
        for adjustment in data["smoothing"]["edges"]:
            adjustment.update(max_displacement_px=0.3, smoothing_applied=True, applied_fraction=1)
    else:
        data["joins"][0]["evidence"]["dark_fraction"] = 0.2
    return raw, data


def validate(raw, data):
    return validate_contours(data, raw["nodes"], raw["edges"], raw["cycles"], 40, 32, 4)


class ContourTests(unittest.TestCase):
    def tearDown(self):
        plt.close("all")

    def test_accepted_bridge_combines_raw_paths_without_dropping_provenance(self):
        raw, data = fixture()
        contours = validate(raw, data)
        self.assertEqual(len(contours.raw_paths), 2)
        self.assertEqual(len(contours.paths), 1)
        self.assertEqual(contours.paths[0].source_edges, (5, 8))
        self.assertEqual(contours.paths[0].source_joins, (7,))
        self.assertEqual(contours.joins[0].status, "accepted")

    def test_unaccepted_hypothesis_retains_endpoints_and_separate_paths(self):
        raw, data = fixture(accepted=False)
        contours = validate(raw, data)
        self.assertEqual(len(contours.paths), 2)
        self.assertEqual(len(contours.nodes), 4)
        self.assertEqual(contours.joins[0].status, "image_unsupported")
        data["edges"][0]["source_joins"] = [7]
        with self.assertRaisesRegex(ValueError, "source_joins"):
            validate(raw, data)

    def test_smoothing_diagnostics_cover_paths_and_respect_displacement_bound(self):
        raw, baseline = fixture()
        changes = (
            lambda d: d["smoothing"]["edges"].pop(),
            lambda d: d["smoothing"]["edges"][1].update(source_edge=5),
            lambda d: d["smoothing"]["edges"][0].update(max_displacement_px=1.1),
            lambda d: d["smoothing"]["edges"][0].update(smoothing_applied=False),
            lambda d: d["smoothing"]["edges"][0].update(applied_fraction=1.1),
            lambda d: d["smoothing"]["edges"][0].update(retained_reason="unknown"),
            lambda d: d["smoothing"].update(sigma_px=1),
        )
        for change in changes:
            data = copy.deepcopy(baseline)
            change(data)
            with self.subTest(data=data), self.assertRaises(ValueError):
                validate(raw, data)

    def test_rejects_lost_or_duplicate_origins_and_unconsumed_nodes(self):
        raw, baseline = fixture()
        changes = (
            lambda d: d["edges"][0].update(source_edges=[5]),
            lambda d: d["edges"][0].update(source_edges=[5, 5, 8]),
            lambda d: d["edges"][0].update(source_joins=[]),
            lambda d: d["nodes"].append({**raw["nodes"][1], "source_node": 20}),
            lambda d: d["nodes"][0].update(source_node=40),
            lambda d: d["nodes"][0].update(x=4),
            lambda d: d["edges"][0].update(end=20),
        )
        for change in changes:
            data = copy.deepcopy(baseline)
            change(data)
            with self.subTest(data=data), self.assertRaises(ValueError):
                validate(raw, data)

    def test_join_must_match_source_endings_policy_and_source_evidence(self):
        raw, baseline = fixture()
        changes = (
            lambda d: d["joins"][0].update(evidence=None),
            lambda d: d["joins"][0]["evidence"].update(dark_fraction=0.1),
            lambda d: d["joins"][0]["evidence"].update(mean_flank_contrast=0.1),
            lambda d: d["joins"][0]["evidence"].update(longest_light_gap_px=2),
            lambda d: d["joins"][0].update(gap_px=4.1),
            lambda d: d["joins"][0].update(gap_px=2.5),
            lambda d: d["joins"][0].update(max_tangent_error_radians=0.6),
            lambda d: d["joins"][0].update(start_node=999),
            lambda d: d["joins"][0].update(status="certain"),
            lambda d: d["joins"][0]["points"][0].__setitem__(0, 16),
            lambda d: d["policy"].update(max_gap_px=8),
        )
        for change in changes:
            data = copy.deepcopy(baseline)
            change(data)
            with self.subTest(data=data), self.assertRaises(ValueError):
                validate(raw, data)
        # Enhancement boundary provenance alone is not the acquisition mask.
        raw["nodes"][1]["touches_unobserved"] = True
        validate(raw, baseline)
        raw["nodes"][1]["kind"] = "junction"
        with self.assertRaisesRegex(ValueError, "ending"):
            validate(raw, baseline)

    def test_photometric_ratios_may_exceed_one_but_must_be_finite(self):
        raw, data = fixture()
        data["joins"][0]["evidence"].update(mean_darkness=1.2, mean_flank_contrast=1.3)
        validate(raw, data)
        for key in (
            "mean_darkness",
            "mean_flank_contrast",
            "dark_fraction",
            "longest_light_gap_px",
        ):
            invalid = copy.deepcopy(data)
            invalid["joins"][0]["evidence"][key] = float("nan")
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate(raw, invalid)

    def test_origin_partition_also_requires_connected_chains_and_pinned_ports(self):
        raw, data = fixture(accepted=False)
        data["edges"][0]["source_edges"], data["edges"][1]["source_edges"] = [8], [5]
        with self.assertRaisesRegex(ValueError, "endpoints"):
            validate(raw, data)
        raw, data = fixture()
        data["edges"][0]["points"][0] = [4, 15]
        with self.assertRaisesRegex(ValueError, "port"):
            validate(raw, data)

    def test_join_can_close_a_path_without_losing_the_cycle_or_its_evidence(self):
        raw = {
            "scale": {"unit_px": 4},
            "nodes": [
                {"id": i, "x": x, "y": 10, "kind": "ending", "touches_unobserved": False}
                for i, x in ((10, 10), (20, 13))
            ],
            "edges": [
                {
                    "id": 5,
                    "start": 10,
                    "end": 20,
                    "points": [[10, 10], [10, 20], [20, 20], [13, 10]],
                }
            ],
            "cycles": [],
        }
        data = unchanged_contours(raw)
        join = fixture()[1]["joins"][0]
        join.update(start_node=20, end_node=10, points=[[13, 10], [12, 10], [11, 10], [10, 10]])
        data.update(
            nodes=[],
            edges=[],
            joins=[join],
            cycles=[
                {
                    "id": 3,
                    "points": [*raw["edges"][0]["points"], [12, 10], [11, 10]],
                    "segment_clarity": [1] * 6,
                    "source_edges": [5],
                    "source_cycles": [],
                    "source_joins": [7],
                }
            ],
        )
        contours = validate(raw, data)
        self.assertTrue(contours.paths[0].closed)
        with Image.new("L", (40, 32), 160) as source:
            figure = render_contour_overlay(contours, source, 0.5)
        lines = figure.axes[0].collections[0].get_segments()
        np.testing.assert_array_equal(lines[0][0], lines[-1][-1])

    def test_rejects_nonfinite_or_outside_refined_geometry(self):
        raw, baseline = fixture()
        for point in ([float("inf"), 15], [-1, 15], [40, 15], [True, 15]):
            data = copy.deepcopy(baseline)
            data["edges"][0]["points"][1] = point
            with self.subTest(point=point), self.assertRaises(ValueError):
                validate(raw, data)
        data = copy.deepcopy(baseline)
        data["edges"][0]["points"] = [[3, 15], [3, 15]]
        with self.assertRaises(ValueError):
            validate(raw, data)

    def test_rendered_overlay_uses_exact_supplied_geometry_and_transparency(self):
        raw, data = fixture()
        contours = validate(raw, data)
        pixels = np.arange(40 * 32, dtype=np.uint8).reshape(32, 40)
        with Image.fromarray(pixels) as source:
            comparison = render_contour_comparison(contours, source, 0.4)
            overlay = render_contour_overlay(contours, source, 0.4)
        for figure in (comparison, overlay):
            for axis in figure.axes[:-1]:
                np.testing.assert_array_equal(axis.images[0].get_array(), pixels)
                self.assertEqual(axis.images[0].get_alpha(), 1)
                self.assertEqual(tuple(axis.images[0].get_extent()), (-0.5, 39.5, 31.5, -0.5))
            labels = " ".join(text.get_text() for text in figure.findobj(Text))
            self.assertIn("accepted joins: 1", labels)
            self.assertIn("Accepted source-supported join", labels)
            figure.canvas.draw()
            renderer = figure.canvas.get_renderer()
            footer = figure.axes[-1]
            caption = footer.texts[0].get_window_extent(renderer)
            legend = footer.get_legend().get_window_extent(renderer)
            self.assertLess(caption.y1, legend.y0)
        self.assertEqual(comparison.axes[2].get_xlim(), comparison.axes[3].get_xlim())
        self.assertEqual(comparison.axes[2].get_ylim(), comparison.axes[3].get_ylim())
        lines = [item for item in overlay.axes[0].collections if isinstance(item, LineCollection)]
        np.testing.assert_array_equal(
            lines[0].get_segments(),
            list(zip(data["edges"][0]["points"][:-1], data["edges"][0]["points"][1:], strict=True)),
        )
        np.testing.assert_array_equal(
            lines[1].get_segments(),
            list(zip(data["joins"][0]["points"][:-1], data["joins"][0]["points"][1:], strict=True)),
        )
        np.testing.assert_allclose(lines[0].get_colors()[:, 3], 0.9)
        np.testing.assert_allclose(lines[1].get_colors()[:, 3], 0.9)

    def test_unaccepted_candidates_render_dashed_without_changing_path_count(self):
        raw, data = fixture(accepted=False)
        contours = validate(raw, data)
        with Image.new("L", (40, 32), 160) as source:
            figure = render_contour_overlay(contours, source, 0.5)
        lines = [item for item in figure.axes[0].collections if isinstance(item, LineCollection)]
        self.assertEqual(len(lines[0].get_segments()), 4)
        self.assertIsNone(lines[0].get_linestyles()[0][1])
        self.assertIsNotNone(lines[1].get_linestyles()[0][1])

    def test_segment_clarity_is_required_for_raw_refined_paths_and_joins(self):
        raw, data = fixture()
        targets = (raw["edges"][0], data["edges"][0], data["joins"][0])
        for target in targets:
            saved = target.pop("segment_clarity")
            with self.assertRaisesRegex(ValueError, "segment_clarity"):
                validate(raw, data)
            target["segment_clarity"] = saved

    def test_join_quality_opacity_preserves_acceptance_style_and_endpoint_status(self):
        for accepted in (True, False):
            raw, data = fixture(accepted)
            data["joins"][0]["segment_clarity"] = [0, 1, None]
            contours = validate(raw, data)
            with Image.new("L", (40, 32), 160) as source:
                figure = render_contour_overlay(contours, source, 0.5)
            axis = figure.axes[0]
            lines = [item for item in axis.collections if isinstance(item, LineCollection)]
            join = lines[-1]
            expected = to_rgba("#009e73" if accepted else "#e69f00")[:3]
            np.testing.assert_allclose(join.get_colors()[:, 3], [0.4, 0.9, 0.4])
            np.testing.assert_allclose(join.get_colors()[:2, :3], [expected] * 2)
            np.testing.assert_allclose(join.get_colors()[2, :3], expected)
            self.assertEqual(join.get_linestyles()[0][1] is None, accepted)
            np.testing.assert_allclose(axis.collections[-1].get_edgecolors()[0, :3], expected)


if __name__ == "__main__":
    unittest.main()
