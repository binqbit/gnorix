"""Model checks require only Python's standard library."""

import copy
import json
import tempfile
import unittest
from dataclasses import FrozenInstanceError
from pathlib import Path
from unittest.mock import patch

from gnorix_reports.model import (
    ValidationError,
    build_edges,
    load_snapshot,
    metric_domain,
    validate_snapshot,
)


def fixture():
    return {
        "schema": "gnorix.inspection.v1",
        "title": "Synthetic model fixture",
        "frames": [{"id": "capture", "label": "Frame", "width": 10, "height": 10}],
        "stages": [
            {
                "id": "measured",
                "label": "Measured",
                "frame": "capture",
                "metrics": [{"key": "weight", "label": "Weight"}],
                "points": [
                    {"id": "a", "x": 1, "y": 1, "q": 0, "r": 0, "values": {"weight": 0}},
                    {"id": "b", "x": 2, "y": 1, "q": 1, "r": 0, "values": {"weight": 1}},
                    {"id": "c", "x": 1.5, "y": 2, "q": 0, "r": 1, "values": {"weight": None}},
                    {
                        "id": "d",
                        "x": 3,
                        "y": 1,
                        "q": 2,
                        "r": 0,
                        "valid": False,
                        "values": {"weight": 99},
                    },
                    {"id": "e", "x": 4, "y": 1, "values": {}},
                ],
                "raster": {
                    "label": "Raster",
                    "x": 0,
                    "y": 0,
                    "width": 10,
                    "height": 10,
                    "columns": 2,
                    "rows": 2,
                    "values": [0, 1, None, 2],
                },
                "series": [
                    {
                        "id": "loss",
                        "label": "Loss",
                        "xLabel": "Iteration",
                        "yLabel": "Residual",
                        "points": [{"x": 0, "y": 2}, {"x": 1, "y": 1}],
                    }
                ],
            }
        ],
    }


class ModelTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.base = Path(self.directory.name)

    def validate(self, data):
        return validate_snapshot(data, self.base)

    def test_normalizes_immutable_values_without_mutating_the_input(self):
        data = fixture()
        original = copy.deepcopy(data)
        snapshot = self.validate(data)
        self.assertEqual(data, original)
        self.assertTrue(snapshot.stages[0].points[0].valid)
        self.assertEqual(snapshot.stages[0].points[0].values["weight"], 0)
        self.assertIsNone(snapshot.stages[0].points[2].values["weight"])
        self.assertIsNone(snapshot.frames[0].image)
        self.assertEqual(snapshot.stages[0].series[0].x_label, "Iteration")
        with self.assertRaises(FrozenInstanceError):
            snapshot.title = "changed"
        with self.assertRaises(TypeError):
            snapshot.stages[0].points[0].values["weight"] = 123
        data["stages"][0]["points"][0]["values"]["weight"] = 123
        self.assertEqual(snapshot.stages[0].points[0].values["weight"], 0)

    def test_tolerance_radius_preserves_missing_zero_and_explicit_pixel_distances(self):
        data = fixture()
        self.assertIsNone(self.validate(data).stages[0].points[0].tolerance_radius)

        for radius in [None, 0, 2.5, 20, 1e308]:
            with self.subTest(radius=radius):
                data["stages"][0]["points"][0]["toleranceRadius"] = radius
                point = self.validate(data).stages[0].points[0]
                self.assertEqual(point.tolerance_radius, radius)
                self.assertEqual((point.x, point.y), (1, 1))

    def test_tolerance_radius_rejects_negative_nonfinite_and_nonnumeric_values(self):
        for radius in [-0.1, float("nan"), float("inf"), -float("inf"), True, False, "2", []]:
            with self.subTest(radius=radius):
                data = fixture()
                data["stages"][0]["points"][0]["toleranceRadius"] = radius
                with self.assertRaisesRegex(ValidationError, r"points\[0\].toleranceRadius"):
                    self.validate(data)

    def test_rejects_duplicate_ids_references_and_undeclared_metrics(self):
        cases = [
            (lambda d: d.update(schema="unknown"), r"snapshot.schema"),
            (lambda d: d["frames"].append(d["frames"][0]), r"frames\[1\].id.*duplicate"),
            (lambda d: d["stages"].append(d["stages"][0]), r"stages\[1\].id.*duplicate"),
            (lambda d: d["stages"][0].update(frame="unknown"), r".frame.*unknown"),
            (
                lambda d: d["stages"][0]["metrics"].append(d["stages"][0]["metrics"][0]),
                r"metrics\[1\].key.*duplicate",
            ),
            (lambda d: d["stages"][0]["points"][1].update(id="a"), r"points\[1\].id.*duplicate"),
            (lambda d: d["stages"][0]["points"][1].update(q=0), r"duplicate lattice"),
            (lambda d: d["stages"][0]["points"][0].pop("r"), r"q and r"),
            (lambda d: d["stages"][0]["points"][0]["values"].update(unknown=1), r"not declared"),
            (
                lambda d: d["stages"][0]["series"].append(d["stages"][0]["series"][0]),
                r"series\[1\].id.*duplicate",
            ),
        ]
        for change, message in cases:
            with self.subTest(message=message):
                data = fixture()
                change(data)
                with self.assertRaisesRegex(ValidationError, message):
                    self.validate(data)

    def test_rejects_invalid_numbers_bounds_and_rasters(self):
        cases = [
            (lambda d: d["frames"][0].update(width=True), r".width"),
            (lambda d: d["frames"][0].update(width=0), r".width"),
            (lambda d: d["stages"][0]["points"][0].update(x=10), r".x.*inside"),
            (lambda d: d["stages"][0]["points"][0].update(y=-1), r".y.*inside"),
            (lambda d: d["stages"][0]["points"][0].update(q=0.5), r"q and r"),
            (lambda d: d["stages"][0]["points"][0].update(valid="yes"), r".valid.*boolean"),
            (
                lambda d: d["stages"][0]["points"][0]["values"].update(weight=float("nan")),
                r"finite number",
            ),
            (lambda d: d["stages"][0]["points"][0]["values"].update(weight=True), r"finite number"),
            (
                lambda d: d["stages"][0]["points"][0]["values"].update(weight=10**400),
                r"finite number",
            ),
            (lambda d: d["stages"][0]["metrics"][0].update(domain=[2, 1]), r"minimum"),
            (lambda d: d["stages"][0]["raster"]["values"].pop(), r"raster.values"),
            (lambda d: d["stages"][0]["raster"].update(width=11), r"contained"),
            (
                lambda d: d["stages"][0]["series"][0]["points"][0].update(y=float("inf")),
                r"finite number",
            ),
        ]
        for change, message in cases:
            with self.subTest(message=message):
                data = fixture()
                change(data)
                with self.assertRaisesRegex(ValidationError, message):
                    self.validate(data)

    def test_bounds_collections_before_traversing_them(self):
        cases = [
            (lambda d: d.update(frames=[None] * 25), r"snapshot.frames"),
            (lambda d: d.update(stages=[None] * 101), r"snapshot.stages"),
            (lambda d: d["frames"][0].update(width=16_385), r".width"),
            (lambda d: d["frames"][0].update(width=8192, height=8192), r"at most.*pixels"),
            (lambda d: d["stages"][0].update(points=[None] * 100_001), r".points"),
            (lambda d: d["stages"][0].update(series=[None] * 33), r".series"),
            (lambda d: d["stages"][0]["series"][0].update(points=[None] * 20_001), r".points"),
            (lambda d: d["stages"][0]["raster"].update(columns=1001, rows=1000), r"raster exceeds"),
        ]
        for change, message in cases:
            with self.subTest(message=message):
                data = fixture()
                change(data)
                with self.assertRaisesRegex(ValidationError, message):
                    self.validate(data)

    def test_domains_distinguish_missing_zero_invalid_and_declared_values(self):
        data = fixture()
        self.assertEqual(metric_domain(self.validate(data).stages[0], "weight"), (0, 1))
        data["stages"][0]["points"][1]["values"]["weight"] = 0
        self.assertEqual(metric_domain(self.validate(data).stages[0], "weight"), (0, 0))
        data["stages"][0]["points"] = [data["stages"][0]["points"][2]]
        self.assertIsNone(metric_domain(self.validate(data).stages[0], "weight"))
        data["stages"][0]["metrics"][0]["domain"] = [-5, 5]
        stage = self.validate(data).stages[0]
        self.assertEqual(metric_domain(stage, "weight"), (-5, 5))
        with self.assertRaisesRegex(ValidationError, "unknown key"):
            metric_domain(stage, "unknown")

    def test_edges_include_each_axial_neighbor_once_and_retain_zero_index(self):
        data = fixture()
        coordinates = [(0, 0), (1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)]
        data["stages"][0]["points"] = [
            {"id": str(index), "x": 3, "y": 3, "q": q, "r": r, "values": {}}
            for index, (q, r) in enumerate(coordinates)
        ]
        edges = build_edges(self.validate(data).stages[0])
        self.assertEqual(len(edges), 12)
        self.assertEqual(len({tuple(sorted(edge)) for edge in edges}), 12)
        self.assertEqual(sum(0 in edge for edge in edges), 6)
        data["stages"][0]["points"] = [{"id": "free", "x": 0, "y": 0, "values": {}}]
        self.assertEqual(build_edges(self.validate(data).stages[0]), [])

    def test_image_paths_remain_inside_bundle_and_resolve_from_snapshot_directory(self):
        image = self.base / "images" / "capture.PNG"
        image.parent.mkdir()
        image.write_bytes(b"renderer validates image content")
        data = fixture()
        data["frames"][0]["image"] = "images/capture.PNG"
        path = self.base / "snapshot.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        self.assertEqual(load_snapshot(path).frames[0].image, image.resolve())

        for invalid in [
            "https://host/image.png",
            "data:image/png;base64,AAAA",
            "/tmp/image.png",
            "C:\\image.png",
            "../image.png",
            "images/../images/capture.PNG",
            "images\\capture.PNG",
            "missing.png",
            "images",
            "image.svg",
            "\0.png",
        ]:
            with self.subTest(image=invalid):
                data["frames"][0]["image"] = invalid
                with self.assertRaisesRegex(ValidationError, r"frames\[0\].image"):
                    self.validate(data)

    def test_rejects_images_that_escape_through_symlinks(self):
        with tempfile.TemporaryDirectory() as outside:
            target = Path(outside) / "outside.png"
            target.write_bytes(b"not a real image")
            link = self.base / "link.png"
            try:
                link.symlink_to(target)
            except OSError:
                self.skipTest("creating symlinks is unavailable")
            data = fixture()
            data["frames"][0]["image"] = "link.png"
            with self.assertRaisesRegex(ValidationError, "outside the snapshot bundle"):
                self.validate(data)

    def test_json_rejects_constants_duplicates_overflow_and_malformed_documents(self):
        path = self.base / "snapshot.json"
        for encoded in [
            b'{"value":NaN}',
            b'{"value":Infinity}',
            b'{"value":-Infinity}',
            b'{"schema":"a","schema":"b"}',
            b'{"value":' + b"9" * 10_000 + b"}",
            b"not json",
            b"\xff",
        ]:
            with self.subTest(encoded=encoded):
                path.write_bytes(encoded)
                with self.assertRaises(ValidationError):
                    load_snapshot(path)

        data = fixture()
        encoded = json.dumps(data).replace('"weight": 0', '"weight": 1e400')
        path.write_text(encoded, encoding="utf-8")
        with self.assertRaisesRegex(ValidationError, "finite number"):
            load_snapshot(path)

    def test_json_reader_enforces_size_cap_before_parsing(self):
        path = self.base / "snapshot.json"
        path.write_bytes(b"x" * 33)
        with patch("gnorix_reports.model.MAX_SNAPSHOT_BYTES", 32):
            with self.assertRaisesRegex(ValidationError, "64 MiB limit"):
                load_snapshot(path)


if __name__ == "__main__":
    unittest.main()
