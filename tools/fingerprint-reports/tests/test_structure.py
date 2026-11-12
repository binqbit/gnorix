"""Native graph reports exercised with explicit synthetic geometry and rasters."""

import copy
import json
import math
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import matplotlib.pyplot as plt
import numpy as np
from feature_fixture import feature_fixture
from matplotlib.collections import LineCollection, PathCollection
from matplotlib.text import Text
from PIL import Image

from gnorix_reports.structure import _load_structure, _save_figure, render_structure


def fixture():
    geometry = {
        "arc_length": 14,
        "chord_length": 10,
        "arc_length_units": 7,
        "chord_length_units": 5,
        "midpoint": [9, 5],
        "turning": [
            {"arc_fraction": 0, "angle_radians": 0},
            {"arc_fraction": 1, "angle_radians": 1.2},
        ],
    }
    data = {
        "schema": "gnorix.ridge-extraction.v1",
        "width": 32,
        "height": 24,
        "config": {"bootstrap_grid_locations": 16, "min_contrast": 0.035, "min_coherence": 0.2},
        "bootstrap": {
            "ridge_width_px": 2,
            "valley_width_px": 3,
            "ridge_period_px": 5,
            "ridge_width_mad_px": 0,
            "ridge_period_mad_px": 0,
            "mean_ridge_width_px": 2,
            "mean_valley_width_px": 3,
            "mean_ridge_gray": 80,
            "mean_valley_gray": 180,
            "examined_locations": 1,
            "profiles": [
                {
                    "ridges": [
                        {"start": [1, 10], "end": [3, 10]},
                        {"start": [6, 10], "end": [8, 10]},
                        {"start": [11, 10], "end": [13, 10]},
                    ],
                    "ridge_mean_gray": [80, 80, 80],
                    "valley_mean_gray": [180, 180],
                }
            ],
            "width_consistency": [
                {
                    "relative_tolerance": tolerance,
                    "accepted_measurements": 3,
                    "total_measurements": 3,
                    "fraction": 1,
                }
                for tolerance in (0.1, 0.2, 0.3)
            ],
        },
        "scale": {
            "method": "raw-centerline-link-width",
            "unit_px": 2,
            "coverage": {
                "total_length_px": 2,
                "measured_length_px": 2,
                "total_links": 2,
                "measured_links": 2,
                "isolated_pixels": 1,
            },
            "observations": [
                {
                    "link": [163 + index, 164 + index],
                    "start": [3.5 + index, 4],
                    "end": [3.5 + index, 6],
                    "represented_length_px": 1,
                    "mean_gray": 80,
                }
                for index in (0, 1)
            ],
            "width_consistency": [
                {
                    "relative_tolerance": tolerance,
                    "accepted_measurements": 2,
                    "total_measurements": 2,
                    "fraction": 1,
                }
                for tolerance in (0.1, 0.2, 0.3)
            ],
        },
        "normalization": {
            "radius_px": 3,
            "method": "local-two-class-midpoint",
            "reference_encoding": "rounded-u8",
        },
        "clarity": {
            "method": "local-ridge-separability-v1",
            "period_px": 5,
            "grid_spacing_px": 1,
            "score": "interpolated(contrast * separation * repetition)",
            "segment_sampling": "bilinear-map-line-mean",
            "encoding": "rounded-u8",
            "summary": {"supported_pixels": 768, "total_pixels": 768, "mean_score": 1},
        },
        "nodes": [
            {"id": 10, "x": 3, "y": 5, "kind": "ending", "touches_unobserved": False},
            {"id": 20, "x": 14, "y": 5, "kind": "junction", "touches_unobserved": False},
            {"id": 30, "x": 31, "y": 7, "kind": "ending", "touches_unobserved": True},
            {"id": 40, "x": 3, "y": 17, "kind": "isolated", "touches_unobserved": False},
            {"id": 50, "x": 15, "y": 17, "kind": "ending", "touches_unobserved": False},
        ],
        "edges": [
            {
                "id": 8,
                "start": 10,
                "end": 20,
                "points": [[3, 5], [9, 4], [14, 5]],
                "geometry": geometry,
                "geometry_error": None,
            },
            {
                "id": 9,
                "start": 20,
                "end": 30,
                "points": [[14, 5], [20, 6], [31, 7]],
                "geometry": None,
                "geometry_error": "synthetic unresolved tangent",
            },
        ],
        "cycles": [
            {
                "id": 7,
                "points": [[20, 10], [25, 10], [25, 16], [20, 16]],
                "geometry": {
                    **geometry,
                    "arc_length": 22,
                    "chord_length": 0,
                    "arc_length_units": 11,
                    "chord_length_units": 0,
                    "midpoint": [25, 16],
                },
                "geometry_error": None,
            },
        ],
        "images": {
            key: key + ".png"
            for key in (
                "source",
                "normalized",
                "denoised",
                "coherence",
                "usable",
                "binary",
                "skeleton",
                "ridge_level",
                "valley_level",
                "normalization_support",
                "measurement_skeleton",
                "pattern_support",
                "clarity",
                "clarity_contrast",
                "clarity_separation",
                "clarity_repetition",
                "clarity_support",
            )
        },
    }

    data["edges"].append(
        {
            "id": 100,
            "start": 20,
            "end": 50,
            "points": [[14, 5], [14, 10], [15, 17]],
            "geometry": None,
            "geometry_error": "synthetic additional branch",
        }
    )
    data["contours"] = unchanged_contours(data)
    return data


def cleanup_source_policy(unit):
    return {
        "relative_width_tolerance": 0.2,
        "max_ridge_width_px": 1.2 * unit,
        "dark_midpoint": 0.5,
        "sample_spacing_px": 0.5,
        "tangent_radius_px": unit / 2,
        "max_backbone_extension_px": 2 * unit,
        "terminal_lookahead_px": 1.2 * unit,
        "terminal_barrier_width_px": unit,
        "terminal_barrier_depth_px": unit / 4,
        "loop_flank_search_px": 1.2 * unit,
        "loop_min_valley_length_px": 1.0,
    }


def unchanged_cleanup(data):
    unit = data["scale"]["unit_px"]
    nodes = {node["id"]: node for node in data["nodes"]}
    candidates = [
        edge["id"]
        for edge in data["edges"]
        if {nodes[edge[key]]["kind"] for key in ("start", "end")} == {"junction", "ending"}
        and sum(math.dist(a, b) for a, b in zip(edge["points"], edge["points"][1:])) < 1.2 * unit
    ]
    return {
        "method": "source-supported-ridge-cleanup",
        "policy": {
            "cursor_diameter_px": unit,
            "pass_count": 1,
            "source": cleanup_source_policy(unit),
        },
        "nodes": [
            {**node, "source_node": node["id"], "pixels": [[node["x"], node["y"]]]}
            for node in data["nodes"]
        ],
        "source_node_pixels": [
            {"source_node": node["id"], "pixels": [[node["x"], node["y"]]]}
            for node in data["nodes"]
        ],
        **{
            kind: [
                {
                    **copy.deepcopy(path),
                    "source_edges": [path["id"]] if kind == "edges" else [],
                    "source_cycles": [path["id"]] if kind == "cycles" else [],
                    "source_routes": [],
                }
                for path in data[kind]
            ]
            for kind in ("edges", "cycles")
        },
        "junction_routes": [],
        "removed_spurs": [],
        "retained_ambiguous_edges": [],
        "retained_unobserved_edges": [],
        "retained_unconfirmed_edges": candidates,
        "spur_assessments": [
            {
                "source_edge": identifier,
                "evidence": {
                    "decision": "possible_continuation",
                    "terminal_darkness": 1,
                    "light_barrier_distance_px": None,
                    "min_barrier_lightness": None,
                },
            }
            for identifier in candidates
        ],
        "loops": [],
    }


def unchanged_contours(data):
    unit = data["scale"]["unit_px"]
    if "config" in data:
        data["feature_strength"] = feature_fixture(data)
    for kind in ("edges", "cycles"):
        for path in data[kind]:
            path.setdefault("segment_clarity", [1] * (len(path["points"]) - (kind == "edges")))
    data["cleanup"] = unchanged_cleanup(data)
    return {
        "method": "source-supported-smooth-contours",
        "nodes": [{**node, "source_node": node["id"]} for node in data["nodes"]],
        "edges": [
            {
                "id": edge["id"],
                "start": edge["start"],
                "end": edge["end"],
                "points": copy.deepcopy(edge["points"]),
                "segment_clarity": edge["segment_clarity"].copy(),
                "source_edges": [edge["id"]],
                "source_cycles": [],
                "source_joins": [],
            }
            for edge in data["edges"]
        ],
        "cycles": [
            {
                "id": cycle["id"],
                "points": copy.deepcopy(cycle["points"]),
                "segment_clarity": cycle["segment_clarity"].copy(),
                "source_edges": [],
                "source_cycles": [cycle["id"]],
                "source_joins": [],
            }
            for cycle in data["cycles"]
        ],
        "joins": [],
        "policy": {
            "max_gap_px": unit,
            "max_tangent_error_radians": np.pi / 6,
            "min_mean_darkness": 0.5,
            "min_dark_fraction": 0.7,
            "min_mean_flank_contrast": 0.25,
            "max_light_gap_px": unit / 4,
        },
        "smoothing": {
            "sigma_px": unit / 2,
            "sample_spacing_px": min(unit / 8, 1),
            "max_displacement_px": unit / 4,
            **{
                kind: [
                    {
                        source_key: item["id"],
                        "max_displacement_px": 0,
                        "smoothing_applied": False,
                        "applied_fraction": 0,
                        "retained_reason": None,
                    }
                    for item in data[kind]
                ]
                for kind, source_key in (("edges", "source_edge"), ("cycles", "source_cycle"))
            },
        },
    }


class StructureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.source = np.arange(32 * 24, dtype=np.uint8).reshape(24, 32)
        for name in fixture()["images"].values():
            with Image.fromarray(self.source) as image:
                image.save(self.base / name)
        for name, value in (
            ("ridge_level", 80),
            ("valley_level", 180),
            ("normalization_support", 255),
            ("pattern_support", 255),
            ("clarity", 255),
            ("clarity_contrast", 255),
            ("clarity_separation", 255),
            ("clarity_repetition", 255),
            ("clarity_support", 255),
        ):
            with Image.new("L", (32, 24), value) as image:
                image.save(self.base / f"{name}.png")
        measurement = np.full((24, 32), 255, dtype=np.uint8)
        measurement[5, 3:6] = 0
        measurement[17, 3] = 0
        with Image.fromarray(measurement) as image:
            image.save(self.base / "measurement_skeleton.png")
        self.figures = []

    def tearDown(self):
        self.assertEqual(plt.get_fignums(), [])

    def write(self, data):
        path = self.base / "snapshot.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        return path

    def render(self, data, **options):
        def save(figure, path, dpi):
            self.figures.append(figure)
            return _save_figure(figure, path, dpi)

        with patch("gnorix_reports.structure._save_figure", side_effect=save):
            return render_structure(self.write(data), self.base / "report", dpi=50, **options)

    def labels(self, figure):
        return " ".join(" ".join(text.get_text() for text in figure.findobj(Text)).split())

    def test_exports_stages_and_structure_without_changing_source_pixels(self):
        original = (self.base / "source.png").read_bytes()
        denoised = 255 - self.source
        with Image.fromarray(denoised) as image:
            image.save(self.base / "denoised.png")
        paths = self.render(fixture(), alpha=0.4)
        self.assertEqual(
            [path.name for path in paths],
            [
                "overview.png",
                "structure-overlay.png",
                "scale-analysis.png",
                "bootstrap-analysis.png",
                "normalization-analysis.png",
                "contour-comparison.png",
                "contour-overlay.png",
                "clarity-analysis.png",
                "feature-strength.png",
            ],
        )
        self.assertEqual((self.base / "source.png").read_bytes(), original)
        self.assertEqual(set(paths), set((self.base / "report").iterdir()))
        for path in paths:
            with Image.open(path) as image:
                image.load()
                self.assertEqual(image.format, "PNG")
                self.assertGreater(image.width, 300)
                self.assertGreater(image.height, 300)
        self.assertEqual(len(self.figures[0].axes), 8)
        for index, axis in enumerate(self.figures[0].axes[:7]):
            expected = denoised if index == 2 else self.source
            np.testing.assert_array_equal(axis.images[0].get_array(), expected)
        self.assertIn("Denoised intensity", self.labels(self.figures[0]))
        overlay = self.figures[1].axes[0]
        np.testing.assert_array_equal(overlay.images[0].get_array(), self.source)
        self.assertEqual(overlay.images[0].get_alpha(), 1)
        self.assertEqual(tuple(overlay.images[0].get_extent()), (-0.5, 31.5, 23.5, -0.5))
        self.assertEqual(tuple(overlay.get_xlim()), (-0.5, 31.5))
        self.assertEqual(tuple(overlay.get_ylim()), (23.5, -0.5))
        for collection in overlay.collections:
            if isinstance(collection, LineCollection):
                np.testing.assert_allclose(collection.get_colors()[:, 3], 0.9)
            else:
                self.assertEqual(collection.get_alpha(), 0.4)
        text = self.labels(self.figures[1])
        for expected in (
            "Paths: 3",
            "isolated cycles: 1",
            "Graph cycle rank: 1",
            "Full mean ridge width: 1 R = 2 px",
            "endings: 3",
            "junctions: 1",
            "isolated nodes: 1",
            "curves without geometry: 2",
            "source px",
            "synthetic unresolved tangent",
            "Hollow + ×",
        ):
            self.assertIn(expected, text)
        self.assertIn("All 2 observations", self.labels(self.figures[2]))
        self.assertIn("all 3 measurements", self.labels(self.figures[3]))
        self.assertIn("all 2 measurements", self.labels(self.figures[3]))
        self.assertIn("thickness consistency", self.labels(self.figures[3]))
        self.assertEqual(self.figures[2].axes[0].images[0].get_alpha(), 1)
        self.assertEqual(self.figures[2].axes[0].collections[0].get_alpha(), 0.4)
        np.testing.assert_array_equal(self.figures[4].axes[0].images[0].get_array(), self.source)
        self.assertIn("local seed-class means", self.labels(self.figures[4]))
        self.assertIn("Described path length: 18 R", self.labels(self.figures[0]))

    def test_error_curve_and_closed_cycle_remain_visible_with_unobserved_node_marked(self):
        data = fixture()
        self.render(data)
        axis = self.figures[1].axes[0]
        lines = [artist for artist in axis.collections if isinstance(artist, LineCollection)]
        self.assertEqual(len(lines), 3)
        expected = (
            [data["edges"][0]["points"]],
            [data["edges"][1]["points"], data["edges"][2]["points"]],
            [data["cycles"][0]["points"] + [[20, 10]]],
        )
        for collection, paths in zip(lines, expected, strict=True):
            segments = [
                segment for points in paths for segment in zip(points[:-1], points[1:], strict=True)
            ]
            np.testing.assert_array_equal(collection.get_segments(), segments)
        scatter = [artist for artist in axis.collections if isinstance(artist, PathCollection)]
        boundary = [artist for artist in scatter if list(artist.get_offsets()[0]) == [31, 7]]
        self.assertEqual(len(boundary), 2)  # Hollow kind marker plus cross.
        self.assertEqual(len(boundary[0].get_facecolors()), 0)

    def test_empty_graph_still_produces_structure_and_scale_reports(self):
        data = fixture()
        data.update(nodes=[], edges=[], cycles=[])
        data["contours"] = unchanged_contours(data)
        self.assertEqual(len(self.render(data)), 9)
        self.assertIn("No extracted graph structure", self.labels(self.figures[1]))
        self.assertIn("Graph cycle rank: 0", self.labels(self.figures[1]))
        self.assertEqual(len(self.figures[1].axes[0].collections), 0)

    def test_cycle_rank_counts_self_edges_parallel_paths_and_disconnected_nodes(self):
        data = fixture()
        data["nodes"][0]["kind"] = "junction"
        data["nodes"].append(
            {"id": 99, "x": 5, "y": 19, "kind": "isolated", "touches_unobserved": False}
        )
        data["edges"].extend(
            [
                {
                    "id": 10,
                    "start": 20,
                    "end": 10,
                    "points": [[14, 5], [8, 2], [3, 5]],
                    "geometry": None,
                    "geometry_error": "synthetic unavailable description",
                },
                {
                    "id": 11,
                    "start": 10,
                    "end": 10,
                    "points": [[3, 5], [5, 6], [5, 8], [3, 5]],
                    "geometry": None,
                    "geometry_error": "synthetic unavailable description",
                },
            ]
        )
        data["contours"] = unchanged_contours(data)
        # One cycle from the parallel pair, one self-edge, one isolated cycle.
        # The two isolated nodes must not change the cycle count.
        self.assertEqual(_load_structure(self.write(data)).graph_cycle_rank, 3)
        self.render(data)
        self.assertIn("Graph cycle rank: 3", self.labels(self.figures[0]))
        self.assertIn("Graph cycle rank: 3", self.labels(self.figures[1]))
        self.assertIn("isolated cycles: 1", self.labels(self.figures[1]))

    def test_denoised_image_is_required_and_decoded_before_output(self):
        data = fixture()
        del data["images"]["denoised"]
        with self.assertRaises(ValueError):
            self.render(data)
        (self.base / "denoised.png").write_bytes(b"invalid denoised image")
        with self.assertRaises(ValueError):
            self.render(fixture())
        self.assertFalse((self.base / "report").exists())

    def test_feature_evidence_is_required_before_any_report_is_published(self):
        data = fixture()
        del data["feature_strength"]
        with self.assertRaisesRegex(ValueError, "feature_strength"):
            self.render(data)
        self.assertFalse((self.base / "report").exists())
        data = fixture()
        data["feature_strength"]["candidates"][0]["strength"] = 0.9
        with self.assertRaisesRegex(ValueError, "combined strength"):
            self.render(data)
        self.assertFalse((self.base / "report").exists())

    def test_normalization_metadata_and_reference_images_are_required_before_output(self):
        for key in ("ridge_level", "valley_level", "normalization_support"):
            data = fixture()
            del data["images"][key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.render(data)
        data = fixture()
        del data["normalization"]
        with self.assertRaises(ValueError):
            self.render(data)
        with Image.new("L", (32, 24), 127) as image:
            image.save(self.base / "normalization_support.png")
        with self.assertRaisesRegex(ValueError, "0 or 255"):
            self.render(fixture())
        self.assertFalse((self.base / "report").exists())

    def test_rejects_escaping_absolute_and_symlink_image_paths_before_output(self):
        outside = self.base.parent / (self.base.name + "-outside.png")
        outside.write_bytes((self.base / "source.png").read_bytes())
        self.addCleanup(outside.unlink)
        (self.base / "escape.png").symlink_to(outside)
        for name in ("../" + outside.name, str(outside), "escape.png", "C:\\source.png"):
            with self.subTest(name=name):
                data = fixture()
                data["images"]["source"] = name
                with self.assertRaises(ValueError):
                    self.render(data)
                self.assertFalse((self.base / "report").exists())

    def test_rejects_unknown_node_references_and_duplicate_ids(self):
        cases = []
        unknown = fixture()
        unknown["edges"][0]["end"] = 999
        cases.append(unknown)
        duplicate_node = fixture()
        duplicate_node["nodes"][1]["id"] = 10
        cases.append(duplicate_node)
        duplicate_edge = fixture()
        duplicate_edge["edges"][1]["id"] = 8
        cases.append(duplicate_edge)
        boolean_id = fixture()
        boolean_id["nodes"][0]["id"] = True
        cases.append(boolean_id)
        for data in cases:
            with self.subTest(data=data):
                with self.assertRaises(ValueError):
                    _load_structure(self.write(data))

    def test_rejects_dimensions_nonfinite_values_and_outside_coordinates(self):
        modifications = [
            lambda d: d.update(width=-1),
            lambda d: d.update(height=True),
            lambda d: d.update(width=16384, height=16384),
            lambda d: d["scale"].update(unit_px=float("nan")),
            lambda d: d["nodes"][0].update(x=float("inf")),
            lambda d: d["nodes"][0].update(y=-1),
            lambda d: d["edges"][0]["points"].append([32, 5]),
            lambda d: d["edges"][0]["geometry"].update(arc_length=float("nan")),
            lambda d: d["edges"][0]["geometry"]["turning"][1].update(angle_radians=float("inf")),
        ]
        for change in modifications:
            data = fixture()
            change(data)
            with self.subTest(data=data):
                with self.assertRaises(ValueError):
                    self.render(data)
                self.assertFalse((self.base / "report").exists())

    def test_requires_explicit_geometry_error_and_monotone_complete_profile(self):
        for change in (
            lambda d: d["edges"][1].update(geometry_error=None),
            lambda d: d["edges"][0].update(geometry_error="conflicting state"),
            lambda d: d["edges"][0]["geometry"]["turning"][1].update(arc_fraction=0),
            lambda d: d["edges"][0]["geometry"].update(chord_length=100),
            lambda d: d["edges"][0]["geometry"].update(arc_length_units=14),
            lambda d: d["edges"][0]["geometry"].pop("chord_length_units"),
        ):
            data = fixture()
            change(data)
            with self.assertRaises(ValueError):
                _load_structure(self.write(data))

    def test_all_images_are_decoded_and_size_checked_before_report_output(self):
        for invalid in (b"not an image", None):
            with self.subTest(invalid=invalid):
                path = self.base / "skeleton.png"
                if invalid is None:
                    with Image.new("L", (31, 24)) as image:
                        image.save(path)
                else:
                    path.write_bytes(invalid)
                with self.assertRaises(ValueError):
                    self.render(fixture())
                self.assertFalse((self.base / "report").exists())

    def test_rejects_wrong_schema_duplicate_json_fields_and_invalid_options(self):
        data = fixture()
        data["schema"] = "other"
        with self.assertRaises(ValueError):
            _load_structure(self.write(data))
        path = self.base / "snapshot.json"
        path.write_text('{"schema":"a","schema":"b"}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            _load_structure(path)
        for options in (
            {"dpi": 0},
            {"dpi": 601},
            {"dpi": True},
            {"alpha": -1},
            {"alpha": float("nan")},
        ):
            with self.subTest(options=options), self.assertRaises(ValueError):
                render_structure(self.write(fixture()), self.base / "report", **options)

    def test_failed_final_export_does_not_replace_existing_reports(self):
        output = self.base / "report"
        output.mkdir()
        old = output / "overview.png"
        old.write_bytes(b"previous report")
        with patch("gnorix_reports.structure._save_figure") as save:

            def fail(figure, path, dpi):
                if path.name == "contour-overlay.png":
                    plt.close(figure)
                    raise OSError("simulated output failure")
                _save_figure(figure, path, dpi)

            save.side_effect = fail
            with self.assertRaises(OSError):
                render_structure(self.write(copy.deepcopy(fixture())), output, dpi=50)
        self.assertEqual(old.read_bytes(), b"previous report")
        self.assertEqual(list(output.iterdir()), [old])
        self.assertEqual(save.call_count, 7)


if __name__ == "__main__":
    unittest.main()
