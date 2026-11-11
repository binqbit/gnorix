import math
import tempfile
import unittest
from pathlib import Path

import numpy as np
from PIL import Image

from gnorix_reports.model import load_snapshot
from gnorix_reports.sample import grid_positions, sample_points, validate_parameters, write_sample


class SampleTests(unittest.TestCase):
    def test_constant_black_preserves_measured_zeros(self):
        with Image.new("L", (8, 8), color=0) as image:
            points = sample_points(image, spacing=4, radius=1, rotation=0)

        self.assertTrue(points)
        self.assertTrue(all(point["values"] == {"intensity": 0, "contrast": 0} for point in points))

    def test_center_window_matches_direct_pixel_statistics(self):
        values = np.arange(49, dtype=np.uint8).reshape(7, 7)
        with Image.fromarray(values) as image:
            points = sample_points(image, spacing=20, radius=1, rotation=0)

        self.assertEqual(len(points), 1)
        self.assertAlmostEqual(points[0]["values"]["intensity"], values[2:5, 2:5].mean())
        self.assertAlmostEqual(points[0]["values"]["contrast"], values[2:5, 2:5].std())

    def test_rotated_grid_preserves_axial_neighbor_distances(self):
        points = {(q, r): (x, y) for q, r, x, y in grid_positions(100, 100, 10, 37)}
        center = points[(0, 0)]
        for key in ((1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)):
            x, y = points[key]
            self.assertAlmostEqual(math.hypot(x - center[0], y - center[1]), 10)
        self.assertTrue(all(0 <= x < 100 and 0 <= y < 100 for x, y in points.values()))

    def test_invalid_or_excessive_preview_parameters_fail(self):
        for spacing in (0, -1, math.nan, math.inf):
            with self.assertRaises(ValueError):
                validate_parameters(spacing, 1, 0)
        for radius in (0, -1, 2049, True):
            with self.assertRaises(ValueError):
                validate_parameters(10, radius, 0)
        for spacing in (0.001, 1e-320):
            with self.assertRaisesRegex(ValueError, "dense"):
                list(grid_positions(100, 100, spacing, 0))

    def test_saved_sample_reloads_with_actual_measurements_and_image(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source.png"
            with Image.new("RGB", (20, 16), color=(128, 128, 128)) as image:
                image.save(source)
            path = write_sample(source, root / "bundle", spacing=8, radius=2, rotation=10)
            snapshot = load_snapshot(path)

            self.assertEqual(snapshot.frames[0].image, path.parent / "capture.png")
            self.assertEqual((snapshot.frames[0].width, snapshot.frames[0].height), (20, 16))
            self.assertEqual(len(snapshot.stages), 2)
            measured = snapshot.stages[1]
            self.assertTrue(all(point.values["intensity"] == 128 for point in measured.points))
            self.assertTrue(all(point.values["contrast"] == 0 for point in measured.points))
            self.assertNotIn("weight", {metric.key for metric in measured.metrics})
            self.assertEqual(len(measured.series[0].points), 20)


if __name__ == "__main__":
    unittest.main()
