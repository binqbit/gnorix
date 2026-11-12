"""Source decisions authorize topology changes without altering observed paths."""

import copy
import unittest

import matplotlib.pyplot as plt
import numpy as np
from matplotlib.collections import LineCollection
from matplotlib.colors import to_rgba
from PIL import Image
from test_cleanup import fixture, validate
from test_structure import unchanged_contours

from gnorix_reports.cleanup import validate_cleanup
from gnorix_reports.contours import _detail, render_contour_comparison, validate_contours


def loop_fixture(confirmed=True):
    raw = {
        "scale": {"unit_px": 4},
        "nodes": [
            {"id": identifier, "x": x, "y": 10, "kind": kind, "touches_unobserved": False}
            for identifier, x, kind in (
                (10, 1, "ending"),
                (20, 5, "junction"),
                (30, 15, "junction"),
                (40, 21, "ending"),
            )
        ],
        "edges": [
            {"id": 1, "start": 10, "end": 20, "points": [[x, 10] for x in range(1, 6)]},
            {
                "id": 5,
                "start": 20,
                "end": 30,
                "points": [[5, 10], *[[x, 9] for x in range(6, 15)], [15, 10]],
            },
            {
                "id": 8,
                "start": 20,
                "end": 30,
                "points": [[5, 10], *[[x, 11] for x in range(6, 15)], [15, 10]],
            },
            {"id": 2, "start": 30, "end": 40, "points": [[x, 10] for x in range(15, 22)]},
        ],
        "cycles": [],
    }
    unchanged_contours(raw)
    data = raw["cleanup"]
    evidence = {
        "decision": "confirmed" if confirmed else "separate_ridges",
        "max_width_px": 2,
        "mean_interior_darkness": 0.7,
        "max_valley_lightness": 0.7,
        "min_flank_lightness": 0.8,
        "longest_valley_length_px": 0.5 if confirmed else 2,
        "arm_darkness": [0.8, 0.7],
        "arm_center_distance_px": [0.1, 0.9],
        "preferred_arm": 0 if confirmed else None,
    }
    data["loops"] = [
        {
            "source_edges": [5, 8],
            "source_nodes": [20, 30],
            "evidence": evidence,
            "removed_edge": 8 if confirmed else None,
        }
    ]
    if confirmed:
        data["nodes"] = [data["nodes"][0], data["nodes"][3]]
        points = [
            *raw["edges"][0]["points"],
            *raw["edges"][1]["points"][1:],
            *raw["edges"][3]["points"][1:],
        ]
        data["edges"] = [
            {
                "id": 99,
                "start": 10,
                "end": 40,
                "points": points,
                "segment_clarity": [1] * (len(points) - 1),
                "source_edges": [1, 5, 2],
                "source_cycles": [],
                "source_routes": [],
            }
        ]
    return raw, data


def validate_loop(raw, data):
    return validate_cleanup(data, raw["nodes"], raw["edges"], [], 24, 24, 4)


class CleanupSourceTests(unittest.TestCase):
    def tearDown(self):
        plt.close("all")

    def test_source_policy_is_explicit_and_scaled_from_measured_r(self):
        raw, data = fixture()
        for field, value in (
            ("relative_width_tolerance", 0.3),
            ("max_ridge_width_px", 3),
            ("dark_midpoint", 0.6),
            ("sample_spacing_px", 1),
            ("tangent_radius_px", 3),
            ("max_backbone_extension_px", 3),
            ("terminal_lookahead_px", 2),
            ("terminal_barrier_width_px", 1),
            ("terminal_barrier_depth_px", 1),
            ("loop_flank_search_px", 1.5),
            ("loop_min_valley_length_px", 2),
        ):
            invalid = copy.deepcopy(data)
            invalid["policy"]["source"][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                validate(raw, invalid)

    def test_confirmed_spur_requires_complete_source_barrier_but_darkness_is_not_probability(self):
        raw, data = fixture()
        evidence = data["spur_assessments"][0]["evidence"]
        evidence.update(terminal_darkness=-0.1, min_barrier_lightness=1.2)
        for distance in (0, 3, 1.2 * 3):
            evidence["light_barrier_distance_px"] = distance
            validate(raw, data)
        for field, value in (
            ("terminal_darkness", None),
            ("terminal_darkness", float("inf")),
            ("min_barrier_lightness", 0.5),
            ("light_barrier_distance_px", 3.61),
            ("light_barrier_distance_px", -0.1),
            ("decision", "certain"),
        ):
            invalid = copy.deepcopy(data)
            invalid["spur_assessments"][0]["evidence"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                validate(raw, invalid)

    def test_all_terminal_candidates_and_decisions_are_accounted_for(self):
        raw, data = fixture()
        for change in (
            lambda d: d.update(spur_assessments=[]),
            lambda d: d["spur_assessments"].append(copy.deepcopy(d["spur_assessments"][0])),
            lambda d: d.update(removed_spurs=[]),
            lambda d: d.update(retained_unconfirmed_edges=[7]),
            lambda d: d["spur_assessments"][0].update(source_edge=5),
        ):
            invalid = copy.deepcopy(data)
            change(invalid)
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                validate(raw, invalid)
        unchanged_contours(raw)
        data = raw["cleanup"]
        validate(raw, data)
        data["retained_unconfirmed_edges"] = []
        data["retained_unobserved_edges"] = [7]
        with self.assertRaisesRegex(ValueError, "contradicts"):
            validate(raw, data)

    def test_retained_terminal_decisions_keep_complete_or_partial_cap_diagnostics(self):
        raw, data = fixture()
        cap = copy.deepcopy(data["spur_assessments"][0]["evidence"])
        unchanged_contours(raw)
        data = raw["cleanup"]
        for decision, retention_key in (
            ("possible_continuation", "retained_unconfirmed_edges"),
            ("missing_support", "retained_unobserved_edges"),
        ):
            data["retained_unconfirmed_edges"] = []
            data["retained_unobserved_edges"] = []
            data[retention_key] = [7]
            data["spur_assessments"][0]["evidence"] = {**cap, "decision": decision}
            with self.subTest(decision=decision):
                self.assertEqual(validate(raw, data).removed_spurs, ())
                data["spur_assessments"][0]["evidence"]["terminal_darkness"] = None
                self.assertEqual(validate(raw, data).removed_spurs, ())

    def test_loop_collapse_keeps_preferred_observed_arm_and_deletes_exactly_one_cycle(self):
        raw, data = loop_fixture()
        original = copy.deepcopy(raw)
        cleanup = validate_loop(raw, data)
        self.assertEqual(raw, original)
        self.assertEqual(cleanup.loops[0].removed_edge, 8)
        self.assertEqual(
            cleanup.loops[0].retained_points, tuple(map(tuple, raw["edges"][1]["points"]))
        )
        self.assertEqual(cleanup.edges[0]["source_edges"], [1, 5, 2])
        self.assertEqual(len(cleanup.nodes), 2)
        self.assertEqual(len(cleanup.edges), 1)

    def test_loop_confirmation_has_strict_width_and_valley_extent_boundaries(self):
        raw, data = loop_fixture()
        evidence = data["loops"][0]["evidence"]
        evidence.update(
            mean_interior_darkness=0.5, max_valley_lightness=1.2, arm_darkness=[1.3, -0.1]
        )
        for width in (2, 4, 4.799999):
            evidence["max_width_px"] = width
            validate_loop(raw, data)
        for field, value in (
            ("max_width_px", 1.2 * 4),
            ("longest_valley_length_px", 1),
            ("mean_interior_darkness", 0.4999),
            ("min_flank_lightness", 0.5),
            ("arm_center_distance_px", [0.1, -0.1]),
            ("arm_darkness", [1]),
            ("preferred_arm", True),
            ("preferred_arm", None),
            ("max_valley_lightness", float("nan")),
            ("max_width_px", None),
        ):
            invalid = copy.deepcopy(data)
            invalid["loops"][0]["evidence"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                validate_loop(raw, invalid)

    def test_unconfirmed_loop_keeps_both_arms_and_its_cycle(self):
        raw, data = loop_fixture(confirmed=False)
        cleanup = validate_loop(raw, data)
        self.assertIsNone(cleanup.loops[0].removed_edge)
        self.assertEqual(len(cleanup.edges), 4)
        for decision in ("missing_support", "ambiguous_geometry", "image_unsupported"):
            evidence = data["loops"][0]["evidence"]
            evidence.update({name: None for name in evidence if name != "decision"})
            evidence["decision"] = decision
            validate_loop(raw, data)
        data["loops"][0]["removed_edge"] = 8
        with self.assertRaisesRegex(ValueError, "source-confirmed"):
            validate_loop(raw, data)

    def test_loop_provenance_cannot_remove_unrecorded_geometry_or_choose_other_arm(self):
        raw, data = loop_fixture()
        for change in (
            lambda d: d.update(loops=[]),
            lambda d: d["loops"].append(copy.deepcopy(d["loops"][0])),
            lambda d: d["loops"][0].update(source_edges=[5, 2]),
            lambda d: d["loops"][0].update(source_nodes=[10, 30]),
            lambda d: d["loops"][0].update(removed_edge=5),
            lambda d: d["loops"][0].update(removed_edge=None),
            lambda d: d["edges"][0].update(source_edges=[1, 5, 8, 2]),
            lambda d: d["edges"][0]["points"].__setitem__(6, [7, 10]),
        ):
            invalid = copy.deepcopy(data)
            change(invalid)
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                validate_loop(raw, invalid)

    def test_loop_comparison_uses_observed_removed_arm_and_fixed_primary_opacity(self):
        raw, data = loop_fixture()
        cleanup = validate_loop(raw, data)
        clean_input = {
            "scale": raw["scale"],
            "nodes": list(cleanup.nodes),
            "edges": list(cleanup.edges),
            "cycles": [],
        }
        final = unchanged_contours(clean_input)
        contours = validate_contours(
            final,
            cleanup.nodes,
            cleanup.edges,
            cleanup.cycles,
            24,
            24,
            4,
            cleanup_width=cleanup.width,
            observed_edges=raw["edges"],
            observed_cycles=[],
            loops=cleanup.loops,
        )
        crop, detail = _detail(contours)
        self.assertIn("collapsed loop", detail)
        self.assertIn("duplicate arm 8", detail)
        for x, y in (*cleanup.loops[0].removed_points, *cleanup.loops[0].retained_points):
            self.assertTrue(crop[0] <= x <= crop[0] + crop[2])
            self.assertTrue(crop[1] <= y <= crop[1] + crop[3])
        with Image.new("L", (24, 24), 180) as source:
            figure = render_contour_comparison(contours, source, 0.4)
        for axis in figure.axes[:4]:
            self.assertEqual(axis.images[0].get_alpha(), 1)
            lines = [artist for artist in axis.collections if isinstance(artist, LineCollection)]
            np.testing.assert_allclose(lines[-1].get_colors()[0], to_rgba("#d55e00", 0.4))
        np.testing.assert_allclose(figure.axes[1].collections[0].get_colors()[:, 3], 0.9)
        figure.canvas.draw()
        renderer = figure.canvas.get_renderer()
        footer = figure.axes[-1]
        self.assertLess(
            footer.texts[0].get_window_extent(renderer).y1,
            footer.get_legend().get_window_extent(renderer).y0,
        )
