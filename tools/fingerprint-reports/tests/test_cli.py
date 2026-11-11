import contextlib
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from PIL import Image

from gnorix_reports.cli import default_output, main


class CommandTests(unittest.TestCase):
    def test_render_produces_only_png_and_does_not_overwrite_by_default(self):
        data = {
            "schema": "gnorix.inspection.v1",
            "title": "Provided weights",
            "frames": [{"id": "f", "label": "Frame", "width": 100, "height": 80}],
            "stages": [
                {
                    "id": "weights",
                    "label": "Weights",
                    "frame": "f",
                    "metrics": [{"key": "weight", "label": "Weight", "domain": [0, 1]}],
                    "points": [
                        {"id": "zero", "x": 20, "y": 20, "values": {"weight": 0}},
                        {"id": "one", "x": 40, "y": 20, "values": {"weight": 1}},
                    ],
                }
            ],
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = root / "snapshot.json"
            snapshot.write_text(json.dumps(data))
            output = root / "reports"
            args = [
                "render",
                str(snapshot),
                "--output",
                str(output),
                "--dpi",
                "50",
                "--overlay-alpha",
                "0.5",
                "--point-radius",
                "2",
                "--tolerance-radius",
                "12",
                "--crop",
                "10",
                "10",
                "70",
                "60",
            ]
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main(args), 0)
            self.assertEqual(len(list(output.glob("*.png"))), 2)
            self.assertEqual(len(list(output.iterdir())), 2)
            png = next(output.glob("*.png"))
            original = png.read_bytes()
            with Image.open(png) as image:
                image.verify()

            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                main(args)
            self.assertEqual(error.exception.code, 2)
            self.assertEqual(png.read_bytes(), original)
            self.assertFalse(list(output.glob(".pending-*")))

            overlay_output = root / "overlays"
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(
                    main(
                        [
                            "render",
                            str(snapshot),
                            "--output",
                            str(overlay_output),
                            "--overlay-only",
                            "--tolerance-radius",
                            "12",
                            "--dpi",
                            "50",
                        ]
                    ),
                    0,
                )
            self.assertEqual(len(list(overlay_output.glob("*-overlay.png"))), 1)
            self.assertEqual(len(list(overlay_output.iterdir())), 1)

    def test_sample_bundle_can_be_rendered_again_from_its_saved_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "input.png"
            with Image.new("L", (12, 12), color=100) as image:
                image.save(source)
            output, replay = root / "sample", root / "replay"
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(
                    main(
                        [
                            "sample",
                            "--image",
                            str(source),
                            "--output",
                            str(output),
                            "--spacing",
                            "6",
                            "--radius",
                            "1",
                            "--dpi",
                            "50",
                        ]
                    ),
                    0,
                )
                self.assertEqual(
                    main(
                        [
                            "render",
                            str(output / "snapshot.json"),
                            "--output",
                            str(replay),
                            "--metric",
                            "contrast",
                            "--dpi",
                            "50",
                        ]
                    ),
                    0,
                )
            self.assertTrue((output / "capture.png").exists())
            self.assertEqual(len(list(output.glob("*.png"))), 7)  # input, reports and overlays
            self.assertEqual(len(list(replay.glob("*.png"))), 2)

    def test_default_output_is_repository_reports_even_from_another_working_directory(self):
        repository = Path(__file__).resolve().parents[3]
        expected = repository / "reports"
        previous = Path.cwd()
        with tempfile.TemporaryDirectory() as temporary:
            try:
                os.chdir(temporary)
                self.assertEqual(default_output(), expected)
                with patch("gnorix_reports.cli.render_bundle", return_value=[]) as render:
                    self.assertEqual(main(["render", "snapshot.json"]), 0)
                self.assertEqual(render.call_args.args[0].output, expected)
            finally:
                os.chdir(previous)

    def test_installed_layout_requires_explicit_output_when_no_checkout_can_be_located(self):
        with tempfile.TemporaryDirectory() as temporary:
            script = Path(temporary) / "lib" / "site-packages" / "gnorix_reports" / "cli.py"
            with patch("gnorix_reports.cli.__file__", str(script)):
                with self.assertRaisesRegex(ValueError, "specify --output"):
                    default_output()
                with patch("gnorix_reports.cli.render_bundle", return_value=[]) as render:
                    self.assertEqual(main(["render", "snapshot.json", "--output", temporary]), 0)
                self.assertEqual(render.call_args.args[0].output, Path(temporary))

    def test_format_option_is_not_supported(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            main(["render", "snapshot.json", "--format", "png"])
        self.assertEqual(error.exception.code, 2)

    def test_failure_leaves_no_partially_published_sample_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "input.png"
            with Image.new("L", (8, 8)) as image:
                image.save(source)
            output = root / "output"
            for invalid in (
                ["--spacing", "0"],
                ["--overlay-alpha", "2"],
                ["--tolerance-radius", "-1"],
                ["--crop", "0", "0", "20", "20"],
            ):
                with self.subTest(invalid=invalid):
                    with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                        main(
                            [
                                "sample",
                                "--image",
                                str(source),
                                "--output",
                                str(output),
                                *invalid,
                            ]
                        )
                    self.assertEqual(list(output.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
