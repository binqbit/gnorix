"""Static export checks using explicit synthetic diagnostic values."""

import copy
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import matplotlib
import matplotlib.pyplot as plt
import numpy as np
from matplotlib.text import Text
from PIL import Image

from gnorix_reports.images import MAX_IMAGE_BYTES, load_image
from gnorix_reports.model import validate_snapshot
from gnorix_reports.options import OverlayOptions
from gnorix_reports.render import (
    _STYLE,
    _render_overlay,
    _render_report,
    _save_figure,
    render_snapshot,
)


def fixture():
    return {
        "schema": "gnorix.inspection.v1",
        "title": "Synthetic renderer fixture",
        "description": "Supplied test values only.",
        "frames": [
            {"id": "capture", "label": "Frame", "width": 32, "height": 24, "image": "input.png"}
        ],
        "stages": [
            {
                "id": "measured",
                "label": "Measured stage",
                "frame": "capture",
                "metrics": [
                    {"key": "weight", "label": "Supplied weight", "unit": "a.u.", "domain": [0, 5]}
                ],
                "points": [
                    {"id": "zero", "x": 4, "y": 4, "q": 0, "r": 0, "values": {"weight": 0}},
                    {"id": "five", "x": 12, "y": 4, "q": 1, "r": 0, "values": {"weight": 5}},
                    {"id": "missing", "x": 8, "y": 11, "q": 0, "r": 1, "values": {"weight": None}},
                    {
                        "id": "invalid",
                        "x": 20,
                        "y": 11,
                        "q": 1,
                        "r": 1,
                        "valid": False,
                        "values": {"weight": 99},
                    },
                ],
                "raster": {
                    "label": "Supplied scalar map",
                    "x": 0,
                    "y": 0,
                    "width": 32,
                    "height": 24,
                    "columns": 2,
                    "rows": 2,
                    "values": [0, None, 3, 5],
                    "domain": [0, 5],
                },
                "series": [
                    {
                        "id": "line",
                        "label": "Supplied series",
                        "xLabel": "Iteration",
                        "yLabel": "Residual",
                        "points": [{"x": 0, "y": 0}, {"x": 1, "y": 2}],
                    }
                ],
            }
        ],
    }


class RenderTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.base = Path(self.directory.name)
        with Image.new("L", (32, 24), color=127) as image:
            image.save(self.base / "input.png")

    def tearDown(self):
        self.assertEqual(plt.get_fignums(), [])

    def render(self, data, name="output", metric_keys=None, **options):
        snapshot = validate_snapshot(data, self.base)
        self.rendered_figures = []

        def save(figure, *arguments):
            self.rendered_figures.append(figure)
            return _save_figure(figure, *arguments)

        with patch("gnorix_reports.render._save_figure", side_effect=save):
            return render_snapshot(snapshot, self.base / name, 60, metric_keys, **options)

    def rendered_text(self, index=0):
        labels = self.rendered_figures[index].findobj(Text)
        return " ".join(" ".join(label.get_text() for label in labels).split())

    def test_exports_png_pixels_and_labels_with_honest_counts(self):
        paths = self.render(fixture())
        self.assertEqual([path.suffix for path in paths], [".png", ".png"])
        self.assertEqual(set(paths), set((self.base / "output").iterdir()))
        for path in paths:
            with Image.open(path) as image:
                image.load()
                self.assertEqual(image.format, "PNG")
                self.assertGreater(image.width, 500)
                self.assertGreater(image.height, 500)

        text = self.rendered_text()
        for label in (
            "Supplied weight (a.u.)",
            "2 measured · 1 missing · 1 rejected",
            "explicit color domain: [0, 5]",
            "Supplied scalar map",
            "1 missing cells",
            "Supplied series",
            "Iteration",
            "Residual",
            "x (px)",
            "y (px)",
        ):
            self.assertIn(label, text)
        axes = self.rendered_figures[0].axes
        overlay = axes[0]
        raster = next(axis for axis in axes if axis.get_title(loc="left") == "Supplied scalar map")
        series = next(axis for axis in axes if axis.get_title(loc="left") == "Supplied series")
        self.assertEqual(tuple(overlay.images[0].get_extent()), (0, 32, 24, 0))
        scalar_map = raster.images[0].get_array()
        self.assertEqual(scalar_map[0, 0], 0)
        self.assertTrue(scalar_map.mask[0, 1])
        np.testing.assert_array_equal(series.lines[0].get_xydata(), [[0, 0], [1, 2]])

    def test_explicit_domain_keeps_outliers_and_histogram_excludes_rejected_values(self):
        data = fixture()
        data["stages"][0]["points"][0]["values"]["weight"] = -2
        data["stages"][0]["points"][1]["values"]["weight"] = 7
        snapshot = validate_snapshot(data, self.base)
        stage = snapshot.stages[0]
        with matplotlib.rc_context(_STYLE):
            figure = _render_report(snapshot, snapshot.frames[0], stage, stage.metrics[0], None)
        try:
            overlay, histogram = figure.axes[:2]
            measured = next(plot for plot in overlay.collections if plot.get_array() is not None)
            self.assertEqual(list(measured.get_array()), [-2, 7])
            self.assertEqual((measured.norm.vmin, measured.norm.vmax), (0, 5))
            self.assertEqual(sum(bar.get_height() for bar in histogram.patches), 2)
            self.assertEqual(tuple(overlay.get_xlim()), (0, 32))
            self.assertEqual(tuple(overlay.get_ylim()), (24, 0))
        finally:
            plt.close(figure)
        self.assertEqual(len(self.render(data)), 2)

    def test_empty_missing_and_constant_zero_metrics_still_export(self):
        cases = []
        empty = fixture()
        empty["stages"][0]["points"] = []
        cases.append(("empty", empty, "No valid measured values"))
        missing = fixture()
        for point in missing["stages"][0]["points"]:
            point["valid"] = True
            point["values"]["weight"] = None
        cases.append(("missing", missing, "0 measured · 4 missing · 0 rejected"))
        constant = fixture()
        constant["stages"][0]["metrics"][0].pop("domain")
        for point in constant["stages"][0]["points"]:
            point["valid"] = True
            point["values"]["weight"] = 0
        cases.append(("constant", constant, "constant; display scale expanded"))
        for name, data, message in cases:
            with self.subTest(name=name):
                paths = self.render(data, name)
                self.assertEqual(len(paths), 2)
                self.assertIn(message, self.rendered_text())

    def test_no_metrics_and_empty_raster_series_receive_informational_panels(self):
        data = fixture()
        stage = data["stages"][0]
        stage["metrics"] = []
        stage["points"] = []
        stage["raster"]["values"] = [None] * 4
        stage["series"][0]["points"] = []
        paths = self.render(data)
        self.assertIn("overview", paths[0].name)
        text = self.rendered_text()
        for message in (
            "No point metric supplied",
            "No sampling points supplied",
            "No raster values supplied",
            "No series values supplied",
        ):
            self.assertIn(message, text)

    def test_dimension_mismatch_and_invalid_image_fail_before_output(self):
        for name, content in (("wrong-size", None), ("malformed", b"not a PNG image")):
            with self.subTest(name=name):
                if content is None:
                    with Image.new("RGB", (20, 20)) as image:
                        image.save(self.base / "input.png")
                else:
                    (self.base / "input.png").write_bytes(content)
                with self.assertRaises(ValueError):
                    self.render(fixture(), name)
                self.assertFalse((self.base / name).exists())

    def test_unknown_metric_and_extreme_values_fail_before_output(self):
        with self.assertRaisesRegex(ValueError, "unknown metric"):
            self.render(fixture(), "unknown", metric_keys=("other",))
        self.assertFalse((self.base / "unknown").exists())
        for kind in ("domain", "point", "series", "raster"):
            data = fixture()
            stage = data["stages"][0]
            if kind == "domain":
                stage["metrics"][0]["domain"] = [-1e308, 1e308]
            elif kind == "point":
                stage["points"][0]["values"]["weight"] = 1e308
            elif kind == "series":
                stage["series"][0]["points"][0]["y"] = 1e308
            else:
                stage["raster"]["values"][0] = 1e308
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, "plotting range"):
                self.render(data, kind)
            self.assertFalse((self.base / kind).exists())

    def test_metric_filter_and_indexed_names_avoid_slug_collisions(self):
        data = fixture()
        data["stages"][0]["id"] = "../stage"
        other = copy.deepcopy(data["stages"][0])
        other["id"] = "stage"
        other["metrics"][0]["key"] = "other"
        for point in other["points"]:
            point["values"] = {"other": point["values"]["weight"]}
        data["stages"].append(other)
        all_paths = self.render(data, "all")
        self.assertEqual(len(all_paths), 4)
        self.assertNotEqual(all_paths[0].name, all_paths[1].name)
        self.assertTrue(all(path.parent == self.base / "all" for path in all_paths))
        selected = self.render(data, "selected", metric_keys=("other",))
        self.assertEqual(len(selected), 2)
        self.assertTrue(selected[0].name.startswith("02-stage"))

    def test_overlay_preserves_source_pixels_and_exact_geometry_under_crop(self):
        data = fixture()
        for point, radius in zip(data["stages"][0]["points"], (2, 4, 3, 5), strict=True):
            point["toleranceRadius"] = radius
        snapshot = validate_snapshot(data, self.base)
        frame, stage = snapshot.frames[0], snapshot.stages[0]
        options = OverlayOptions(alpha=0.5, point_radius=1.5, crop=(2, 2, 8, 8))
        source_bytes = (self.base / "input.png").read_bytes()
        with load_image(self.base / "input.png") as image, matplotlib.rc_context(_STYLE):
            figure = _render_report(snapshot, frame, stage, stage.metrics[0], image, options)
        try:
            axis, histogram = figure.axes[:2]
            self.assertEqual(tuple(axis.get_xlim()), (2, 10))
            self.assertEqual(tuple(axis.get_ylim()), (10, 2))
            self.assertEqual(axis.get_aspect(), 1)
            self.assertEqual(axis.images[0].get_alpha(), 1)
            np.testing.assert_array_equal(
                np.asarray(axis.images[0].get_array()), np.full((24, 32), 127)
            )
            layers = {layer.get_gid(): layer for layer in axis.collections}
            centers = layers["point-centers"]
            self.assertEqual(centers.get_alpha(), 0.5)
            np.testing.assert_allclose(
                centers.get_paths()[0].get_extents().bounds, (2.5, 2.5, 3, 3)
            )
            tolerance = layers["tolerance-outlines"]
            self.assertEqual(tolerance.get_alpha(), 0.5)
            self.assertEqual(len(tolerance.get_facecolors()), 0)
            self.assertTrue(tolerance.get_clip_on())
            self.assertIsNotNone(tolerance.get_clip_box())
            for path, bounds in zip(
                tolerance.get_paths(),
                ((2, 2, 4, 4), (8, 0, 8, 8), (5, 8, 6, 6), (15, 6, 10, 10)),
                strict=True,
            ):
                np.testing.assert_allclose(path.get_extents().bounds, bounds)
            self.assertEqual(layers["missing-centers"].get_alpha(), 0.5)
            self.assertEqual(layers["rejected-centers"].get_alpha(), 0.5)
            self.assertAlmostEqual(layers["lattice-edges"].get_alpha(), 0.3)
            self.assertEqual(sum(bar.get_height() for bar in histogram.patches), 2)
        finally:
            plt.close(figure)
        self.assertEqual((self.base / "input.png").read_bytes(), source_bytes)

    def test_uniform_override_replaces_supplied_radii_and_explicit_zero_is_not_absent(self):
        data = fixture()
        data["stages"][0]["points"][0]["toleranceRadius"] = 2
        snapshot = validate_snapshot(data, self.base)
        frame, stage = snapshot.frames[0], snapshot.stages[0]
        for radius in (6, 0, 1e308):
            options = OverlayOptions(tolerance_radius=radius)
            with matplotlib.rc_context(_STYLE):
                figure = _render_overlay(snapshot, frame, stage, stage.metrics[0], None, options)
            try:
                layers = {layer.get_gid(): layer for layer in figure.axes[0].collections}
                if radius == 6:
                    paths = layers["tolerance-outlines"].get_paths()
                    self.assertEqual(len(paths), 4)
                    for path, point in zip(paths, stage.points, strict=True):
                        np.testing.assert_allclose(
                            path.get_extents().bounds, (point.x - 6, point.y - 6, 12, 12)
                        )
                else:
                    self.assertNotIn("tolerance-outlines", layers)
                self.assertIn(
                    f"Tolerance display override: {radius:g} px", figure._supxlabel.get_text()
                )
                figure.canvas.draw()
            finally:
                plt.close(figure)

    def test_standalone_overlay_exports_preserve_crop_and_omit_other_panels(self):
        options = OverlayOptions(alpha=0.5, point_radius=2, tolerance_radius=4, crop=(2, 2, 20, 14))
        paths = self.render(fixture(), overlay=options, overlay_only=True)
        self.assertEqual(len(paths), 1)
        self.assertTrue(all("-overlay." in path.name for path in paths))
        text = self.rendered_text()
        self.assertIn("Display crop: x=2, y=2, width=20, height=14 px", text)
        self.assertIn("Tolerance display override: 4 px", text)
        self.assertIn("overlay alpha: 0.5", text)
        self.assertIn("Full stage: 2 measured · 1 missing · 1 rejected", text)
        self.assertNotIn("Value distribution", text)
        self.assertNotIn("Supplied series", text)
        self.assertNotIn("Supplied scalar map", text)

    def test_out_of_frame_crop_fails_before_output(self):
        with self.assertRaisesRegex(ValueError, "crop must lie within frame"):
            self.render(fixture(), overlay=OverlayOptions(crop=(30, 20, 10, 10)))
        self.assertFalse((self.base / "output").exists())

    def test_standalone_export_bounds_include_colorbar_ticks_and_wrapped_label(self):
        data = fixture()
        data["stages"][0]["metrics"][0]["label"] = "Local intensity standard deviation"
        data["stages"][0]["metrics"][0]["unit"] = "gray level"
        snapshot = validate_snapshot(data, self.base)
        layouts = []

        def save(figure, *arguments):
            def capture_draw(event):
                colorbar = figure.axes[-1]
                labels = [colorbar.yaxis.label, *colorbar.get_yticklabels()]
                layouts.append(
                    [
                        (label.get_text(), label.get_window_extent(event.renderer).frozen())
                        for label in labels
                    ]
                )

            connection = figure.canvas.mpl_connect("draw_event", capture_draw)
            try:
                return _save_figure(figure, *arguments)
            finally:
                figure.canvas.mpl_disconnect(connection)

        with patch("gnorix_reports.render._save_figure", side_effect=save):
            paths = render_snapshot(
                snapshot,
                self.base / "output",
                60,
                overlay=OverlayOptions(crop=(2, 2, 20, 20)),
                overlay_only=True,
            )

        with Image.open(paths[0]) as image:
            width, height = image.size
            pixels = np.asarray(image.convert("RGB"))
        self.assertTrue(layouts)
        self.assertGreater(len(layouts[-1]), 2)
        for label, bounds in layouts[-1]:
            with self.subTest(label=label):
                self.assertGreater(bounds.x0, 0)
                self.assertGreater(bounds.y0, 0)
                self.assertLess(bounds.x1, width)
                self.assertLess(bounds.y1, height)
        for border in (pixels[:3], pixels[-3:], pixels[:, :3], pixels[:, -3:]):
            self.assertTrue(np.all(border == 255), "annotations reach the image boundary")


class ImageLoadingTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.base = Path(self.directory.name)

    def test_returned_copy_preserves_alpha_after_the_source_is_removed(self):
        source = self.base / "alpha.png"
        with Image.new("RGBA", (3, 2), (10, 20, 30, 40)) as image:
            image.save(source)
        image = load_image(source, (3, 2))
        try:
            source.unlink()
            self.assertEqual(image.mode, "RGBA")
            self.assertEqual(image.getpixel((0, 0)), (10, 20, 30, 40))
        finally:
            image.close()

    def test_non_8bit_image_is_rejected_instead_of_clamped(self):
        source = self.base / "high-depth.png"
        with Image.new("I;16", (3, 2), 65535) as image:
            image.save(source)
        with self.assertRaisesRegex(ValueError, "8-bit"):
            load_image(source)

    def test_excessive_file_size_is_rejected_before_decoding(self):
        source = self.base / "oversized.png"
        with source.open("wb") as stream:
            stream.truncate(MAX_IMAGE_BYTES + 1)
        with self.assertRaisesRegex(ValueError, "128 MiB"):
            load_image(source)

    def test_other_image_formats_are_rejected_even_with_png_extension(self):
        source = self.base / "disguised.png"
        with Image.new("RGB", (3, 2)) as image:
            image.save(source, format="BMP")
        with self.assertRaisesRegex(ValueError, "PNG or JPEG"):
            load_image(source)


if __name__ == "__main__":
    unittest.main()
