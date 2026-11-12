"""Native spur removal preserves observed evidence and cleaned graph provenance."""

import copy
import unittest

import matplotlib.pyplot as plt
from matplotlib.collections import LineCollection
from matplotlib.colors import to_rgba
from PIL import Image
from test_structure import cleanup_source_policy, unchanged_contours

from gnorix_reports.cleanup import validate_cleanup
from gnorix_reports.contour_render import _detail, render_contour_comparison
from gnorix_reports.contours import validate_contours


def fixture(with_route=False):
    right_port = 12 if with_route else 10
    raw = {
        "scale": {"unit_px": 3},
        "nodes": [
            {"id": identifier, "x": x, "y": y, "kind": kind, "touches_unobserved": False}
            for identifier, x, y, kind in (
                (10, 3, 10, "ending"),
                (20, 11 if with_route else 10, 10, "junction"),
                (30, 17, 10, "ending"),
                (40, 11 if with_route else 10, 8, "ending"),
            )
        ],
        "edges": [
            {"id": 5, "start": 10, "end": 20, "points": [[x, 10] for x in range(3, 11)]},
            {"id": 8, "start": 20, "end": 30, "points": [[x, 10] for x in range(right_port, 18)]},
            {
                "id": 7,
                "start": 20,
                "end": 40,
                "points": [[11 if with_route else 10, y] for y in (10, 9, 8)],
            },
        ],
        "cycles": [],
    }
    unchanged_contours(raw)
    cleanup = raw["cleanup"]
    cleanup["nodes"] = [cleanup["nodes"][0], cleanup["nodes"][2]]
    cleanup["edges"] = [
        {
            "id": 99,
            "start": 10,
            "end": 30,
            "points": [[x, 10] for x in range(3, 18)],
            "segment_clarity": [1] * 14,
            "source_edges": [5, 8],
            "source_cycles": [],
            "source_routes": [17] if with_route else [],
        }
    ]
    cleanup["removed_spurs"] = [
        {
            "source_edge": 7,
            "attachment_node": 20,
            "terminal_node": 40,
            "arc_length_px": 2,
        }
    ]
    confirm_spur(cleanup, 7)
    if with_route:
        cleanup["source_node_pixels"][1]["pixels"] = [[10, 10], [11, 10], [12, 10]]
        cleanup["junction_routes"] = [
            {
                "id": 17,
                "source_node": 20,
                "points": [[10, 10], [11, 10], [12, 10]],
            }
        ]
    return raw, cleanup


def confirm_spur(cleanup, identifier):
    assessment = next(
        item for item in cleanup["spur_assessments"] if item["source_edge"] == identifier
    )
    assessment["evidence"] = {
        "decision": "confirmed",
        "terminal_darkness": 0.8,
        "light_barrier_distance_px": cleanup["policy"]["cursor_diameter_px"] / 2,
        "min_barrier_lightness": 0.75,
    }
    if identifier in cleanup["retained_unconfirmed_edges"]:
        cleanup["retained_unconfirmed_edges"].remove(identifier)


def validate(raw, data):
    return validate_cleanup(
        data, raw["nodes"], raw["edges"], raw["cycles"], 24, 24, raw["scale"]["unit_px"]
    )


class CleanupTests(unittest.TestCase):
    def tearDown(self):
        plt.close("all")

    def test_terminal_spur_becomes_continuation_with_exact_observed_geometry(self):
        raw, data = fixture()
        original = copy.deepcopy(raw)
        cleanup = validate(raw, data)
        self.assertEqual(raw, original)
        self.assertEqual([node["id"] for node in cleanup.nodes], [10, 30])
        self.assertEqual(cleanup.edges[0]["source_edges"], [5, 8])
        self.assertEqual(cleanup.removed_spurs[0].source_edge, 7)

    def test_reconnection_uses_actual_junction_pixels_and_routes(self):
        raw, data = fixture(with_route=True)
        validate(raw, data)
        for change in (
            lambda d: d["junction_routes"][0].update(points=[[10, 10], [12, 10]]),
            lambda d: d["junction_routes"][0]["points"].__setitem__(1, [11, 11]),
            lambda d: d["edges"][0].update(source_routes=[]),
            lambda d: d["edges"][0]["points"].__setitem__(8, [11, 9]),
        ):
            invalid = copy.deepcopy(data)
            change(invalid)
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                validate(raw, invalid)

    def test_strict_length_boundary_retains_exact_upper_tolerance_bound(self):
        raw, data = fixture()
        unit = 2 / 1.2
        raw["scale"]["unit_px"] = unit
        data["policy"]["cursor_diameter_px"] = unit
        data["policy"]["source"] = cleanup_source_policy(unit)
        with self.assertRaises(ValueError):
            validate(raw, data)
        data["removed_spurs"][0]["arc_length_px"] = 2 - 1e-9
        with self.assertRaises(ValueError):
            validate(raw, data)
        data["removed_spurs"][0]["arc_length_px"] = 2
        unit += 0.000001
        raw["scale"]["unit_px"] = unit
        data["policy"]["cursor_diameter_px"] = unit
        data["policy"]["source"] = cleanup_source_policy(unit)
        validate(raw, data)

    def test_lower_width_reference_does_not_exclude_smaller_spurs(self):
        raw, data = fixture()
        self.assertLess(data["removed_spurs"][0]["arc_length_px"], 0.8 * raw["scale"]["unit_px"])
        self.assertEqual(len(validate(raw, data).removed_spurs), 1)

    def test_source_confirmed_branch_at_mean_r_is_inside_tolerance(self):
        raw, data = fixture()
        raw["scale"]["unit_px"] = 2
        data["policy"]["cursor_diameter_px"] = 2
        data["policy"]["source"] = cleanup_source_policy(2)
        self.assertEqual(validate(raw, data).removed_spurs[0].arc_length_px, 2)

    def test_processing_unknown_branch_requires_confirmed_source_evidence(self):
        for key in (20, 40):
            raw, data = fixture()
            next(node for node in raw["nodes"] if node["id"] == key)["touches_unobserved"] = True
            validate(raw, data)
            data["spur_assessments"][0]["evidence"]["decision"] = "missing_support"
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "confirmed source"):
                validate(raw, data)
        raw, data = fixture()
        raw["nodes"][3]["kind"] = "junction"
        with self.assertRaises(ValueError):
            validate(raw, data)
        raw, data = fixture()
        data["removed_spurs"].append(copy.deepcopy(data["removed_spurs"][0]))
        with self.assertRaisesRegex(ValueError, "unique"):
            validate(raw, data)

    def test_interior_unknown_retention_is_explicit_even_when_endpoints_are_known(self):
        raw, _ = fixture()
        unchanged_contours(raw)
        data = raw["cleanup"]
        data["retained_unobserved_edges"] = [7]
        data["retained_unconfirmed_edges"] = []
        data["spur_assessments"][0]["evidence"]["decision"] = "missing_support"
        cleanup = validate(raw, data)
        self.assertEqual(cleanup.retained_unobserved_edges, (7,))
        self.assertTrue(all(not node["touches_unobserved"] for node in raw["nodes"]))
        for change in (
            lambda d: d.update(retained_unobserved_edges=[7, 7]),
            lambda d: d.update(retained_unobserved_edges=[5]),
            lambda d: d.update(retained_unobserved_edges=[999]),
            lambda d: d.update(retained_ambiguous_edges=[7]),
        ):
            invalid = copy.deepcopy(data)
            change(invalid)
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                validate(raw, invalid)
        raw, data = fixture()
        data["retained_unobserved_edges"] = [7]
        with self.assertRaisesRegex(ValueError, "disjoint"):
            validate(raw, data)

    def test_policy_partition_and_raster_node_provenance_are_checked(self):
        raw, data = fixture()
        for change in (
            lambda d: d["policy"].update(pass_count=2),
            lambda d: d["policy"].update(cursor_diameter_px=4),
            lambda d: d["edges"][0].update(source_edges=[5]),
            lambda d: d["edges"][0].update(source_edges=[5, 7, 8]),
            lambda d: d["removed_spurs"][0].update(arc_length_px=1),
            lambda d: d["retained_ambiguous_edges"].append(7),
            lambda d: d["nodes"][0].update(x=3.5),
            lambda d: d["nodes"][0].update(kind="junction"),
            lambda d: d["nodes"][0].update(pixels=[[3, 10], [4, 10]]),
        ):
            invalid = copy.deepcopy(data)
            change(invalid)
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                validate(raw, invalid)

    def test_source_supported_paths_cannot_use_newly_available_diagonal(self):
        raw, data = fixture(with_route=True)
        data["source_node_pixels"][1]["pixels"].append([11, 11])
        data["junction_routes"][0]["points"] = [[10, 10], [11, 11], [12, 10]]
        with self.assertRaisesRegex(ValueError, "suppressed diagonal"):
            validate(raw, data)

    def test_all_short_component_remains_ambiguous_and_unchanged(self):
        raw, _ = fixture()
        raw["nodes"][0]["x"], raw["nodes"][2]["x"] = 8, 12
        raw["edges"][0]["points"] = [[8, 10], [9, 10], [10, 10]]
        raw["edges"][1]["points"] = [[10, 10], [11, 10], [12, 10]]
        for edge in raw["edges"]:
            edge["segment_clarity"] = [1, 1]
        unchanged_contours(raw)
        data = raw["cleanup"]
        for identifier in (5, 7, 8):
            confirm_spur(data, identifier)
        data["retained_ambiguous_edges"] = [5, 7, 8]
        self.assertEqual(validate(raw, data).retained_ambiguous_edges, (5, 7, 8))
        data["retained_ambiguous_edges"] = []
        data["removed_spurs"] = [
            {
                "source_edge": edge["id"],
                "attachment_node": 20,
                "terminal_node": edge["start"] if edge["start"] != 20 else edge["end"],
                "arc_length_px": 2,
            }
            for edge in raw["edges"]
        ]
        with self.assertRaisesRegex(ValueError, "all-short"):
            validate(raw, data)

    def test_split_cluster_can_retain_two_junctions_and_a_route_only_edge(self):
        raw, _ = fixture(with_route=True)
        raw["nodes"].extend(
            [
                {"id": 50, "x": 10, "y": 15, "kind": "ending", "touches_unobserved": False},
                {"id": 60, "x": 12, "y": 15, "kind": "ending", "touches_unobserved": False},
            ]
        )
        raw["edges"].extend(
            [
                {"id": 15, "start": 20, "end": 50, "points": [[10, y] for y in range(10, 16)]},
                {"id": 16, "start": 20, "end": 60, "points": [[12, y] for y in range(10, 16)]},
            ]
        )
        unchanged_contours(raw)
        data = raw["cleanup"]
        data["source_node_pixels"][1]["pixels"] = [[10, 10], [11, 10], [12, 10]]
        data["nodes"] = [node for node in data["nodes"] if node["id"] not in (20, 40)]
        data["nodes"].extend(
            [
                {
                    "id": identifier,
                    "source_node": 20,
                    "x": x,
                    "y": 10,
                    "pixels": [[x, 10]],
                    "kind": "junction",
                    "touches_unobserved": False,
                }
                for identifier, x in ((20, 10), (21, 12))
            ]
        )
        data["edges"] = [edge for edge in data["edges"] if edge["id"] != 7]
        for edge in data["edges"]:
            if edge["id"] in (8, 16):
                edge["start"] = 21
        route = [[10, 10], [11, 10], [12, 10]]
        data["edges"].append(
            {
                "id": 99,
                "start": 20,
                "end": 21,
                "points": route,
                "segment_clarity": [1, 1],
                "source_edges": [],
                "source_cycles": [],
                "source_routes": [17],
            }
        )
        data["junction_routes"] = [{"id": 17, "source_node": 20, "points": route}]
        data["removed_spurs"] = [
            {
                "source_edge": 7,
                "attachment_node": 20,
                "terminal_node": 40,
                "arc_length_px": 2,
            }
        ]
        confirm_spur(data, 7)
        cleanup = validate(raw, data)
        self.assertEqual(sum(node["source_node"] == 20 for node in cleanup.nodes), 2)
        self.assertEqual(cleanup.edges[-1]["source_routes"], [17])

    def test_final_smoothing_references_cleaned_ids_and_comparison_keeps_removed_spur(self):
        raw, data = fixture()
        cleanup = validate(raw, data)
        clean_input = {
            "scale": raw["scale"],
            "nodes": list(cleanup.nodes),
            "edges": list(cleanup.edges),
            "cycles": list(cleanup.cycles),
        }
        final = unchanged_contours(clean_input)
        contours = validate_contours(
            final,
            cleanup.nodes,
            cleanup.edges,
            cleanup.cycles,
            24,
            24,
            3,
            cleanup_width=cleanup.width,
            observed_edges=raw["edges"],
            observed_cycles=raw["cycles"],
            removed_spurs=cleanup.removed_spurs,
        )
        self.assertEqual(contours.adjustments[0].source, 99)
        self.assertEqual(len(contours.raw_paths), 3)
        self.assertEqual(len(contours.input_paths), 1)
        self.assertIn("observed edge 7", _detail(contours)[1])
        with Image.new("L", (24, 24), 180) as image:
            figure = render_contour_comparison(contours, image, 0.4)
        for axis in figure.axes[:4]:
            ghosts = [
                artist
                for artist in axis.collections
                if isinstance(artist, LineCollection)
                and tuple(artist.get_colors()[0]) == to_rgba("#d55e00", 0.4)
            ]
            self.assertEqual(len(ghosts), 1)
            self.assertEqual(axis.images[0].get_alpha(), 1)
        labels = " ".join(text.get_text() for text in figure.texts)
        self.assertIn("spur arc, loop width < 1.2R", labels)
        self.assertIn("Mean R = 3 px · ±20%: 2.4–3.6 px", labels)
        figure.canvas.draw()

    def test_cleaned_id_detail_fallback_does_not_look_up_raw_id(self):
        raw, data = fixture()
        cleanup = validate(raw, data)
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
            3,
            cleanup_width=cleanup.width,
            observed_edges=raw["edges"],
            observed_cycles=[],
        )
        self.assertIn("edge 99", _detail(contours)[1])
