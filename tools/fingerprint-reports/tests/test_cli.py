"""One-command native reports preserve the last completed output on failure."""

import contextlib
import io
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np
from PIL import Image
from test_structure import fixture

from gnorix_reports.cli import _publish, generate_report, main, repository_root


class CommandTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "checkout"
        self.root.mkdir()
        self.image = self.root / "fingerprint $(touch unexpected).png"
        with Image.new("L", (32, 24), 100) as image:
            image.save(self.image)
        self.output = self.root / "reports" / "fingerprint"
        self.checkout = patch("gnorix_reports.cli.repository_root", return_value=self.root)
        self.checkout.start()
        self.addCleanup(self.checkout.stop)
        self.calls = []

    def native_success(self, arguments, **options):
        self.calls.append((arguments, options))
        destination = Path(arguments[-1])
        self.assertFalse(destination.exists())
        self.write_native_bundle(destination)
        return subprocess.CompletedProcess(arguments, 0, "native output", "")

    def write_native_bundle(self, destination):
        destination.mkdir(parents=True)
        data = fixture()
        data["config"]["min_contrast"] = 0.035 + len(self.calls) / 1000
        data["feature_strength"]["policy"]["contrast_reference"] = data["config"]["min_contrast"]
        data["images"] = {key: key.replace("_", "-") + ".png" for key in data["images"]}
        source = np.arange(32 * 24, dtype=np.uint8).reshape(24, 32)
        for name in data["images"].values():
            with Image.fromarray(source) as image:
                image.save(destination / name)
        for key, value in (
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
                image.save(destination / data["images"][key])
        measurement = np.full((24, 32), 255, dtype=np.uint8)
        measurement[5, 3:6] = 0
        measurement[17, 3] = 0
        with Image.fromarray(measurement) as image:
            image.save(destination / data["images"]["measurement_skeleton"])
        (destination / "features.json").write_text(json.dumps(data), encoding="utf-8")

    def complete_previous(self):
        self.output.mkdir(parents=True)
        self.write_native_bundle(self.output / "data")
        for name in (
            "overview.png",
            "structure-overlay.png",
            "scale-analysis.png",
            "bootstrap-analysis.png",
            "normalization-analysis.png",
            "contour-comparison.png",
            "contour-overlay.png",
            "clarity-analysis.png",
            "feature-strength.png",
        ):
            with Image.new("L", (8, 8), 42) as image:
                image.save(self.output / name)
        (self.output / "README.md").write_text("previous complete report", encoding="utf-8")

    def contents(self, folder=None):
        folder = folder or self.output
        return {
            str(path.relative_to(folder)): path.read_bytes()
            for path in folder.rglob("*")
            if path.is_file()
        }

    def write_combinations(
        self, *, include_json=True, include_png=True, schema="gnorix.feature-combinations.v4"
    ):
        self.output.mkdir(parents=True, exist_ok=True)
        if include_json:
            (self.output / "feature-combinations.json").write_text(
                json.dumps({"schema": schema}), encoding="utf-8"
            )
        if include_png:
            shutil.copyfile(self.image, self.output / "feature-combinations.png")

    def assert_only_canonical_folder(self):
        self.assertEqual({path.name for path in self.output.parent.iterdir()}, {"fingerprint"})

    def test_runs_native_rust_and_renders_the_complete_canonical_report(self):
        previous_cwd = Path.cwd()
        try:
            os.chdir(self.root.parent)
            with patch("gnorix_reports.cli.subprocess.run", side_effect=self.native_success):
                with contextlib.redirect_stdout(io.StringIO()) as stdout:
                    self.assertEqual(main(["--image", str(self.image), "--dpi", "50"]), 0)
        finally:
            os.chdir(previous_cwd)
        self.assertEqual(stdout.getvalue().strip(), str(self.output / "overview.png"))
        self.assert_only_canonical_folder()
        self.assertEqual(len(list(self.output.glob("*.png"))), 9)
        self.assertEqual(len(list((self.output / "data").iterdir())), 18)
        for path in self.output.glob("*.png"):
            with Image.open(path) as image:
                image.verify()
        arguments, options = self.calls[0]
        self.assertEqual(
            arguments[:-2],
            [
                "cargo",
                "run",
                "-p",
                "gnorix-core",
                "--release",
                "--example",
                "fingerprint",
                "--locked",
                "--",
            ],
        )
        self.assertEqual(arguments[-2], str(self.image.resolve()))
        self.assertEqual(options["cwd"], self.root)
        self.assertNotIn("shell", options)
        self.assertFalse((self.root / "unexpected").exists())
        readme = (self.output / "README.md").read_text()
        self.assertIn("[the overview](overview.png)", readme)
        self.assertIn("**2 source pixels**", readme)
        self.assertIn("**±20%**, **1.6–2.4 px**", readme)
        self.assertIn("upper tolerance bound **2.4 px** (1.2R)", readme)
        self.assertIn("[Refined contour overlay](contour-overlay.png)", readme)
        self.assertIn("[Original feature strength and evidence](feature-strength.png)", readme)

    def test_successful_rerun_replaces_the_previous_bundle_without_accumulation(self):
        self.complete_previous()
        previous = self.contents()
        with patch("gnorix_reports.cli.subprocess.run", side_effect=self.native_success):
            self.assertEqual(generate_report(self.image, 50, 0.4), self.output / "overview.png")
        self.assertNotEqual(self.contents(), previous)
        self.assert_only_canonical_folder()
        self.assertNotIn("previous complete report", (self.output / "README.md").read_text())

    def test_previous_native_bundle_without_clarity_refreshes_without_legacy_rendering(self):
        self.complete_previous()
        (self.output / "clarity-analysis.png").unlink()
        path = self.output / "data" / "features.json"
        data = json.loads(path.read_text())
        del data["clarity"]
        for key in list(data["images"]):
            if key.startswith("clarity"):
                (path.parent / data["images"].pop(key)).unlink()
        for graph in (data, data["contours"]):
            for kind in ("edges", "cycles"):
                for curve in graph[kind]:
                    del curve["segment_clarity"]
        path.write_text(json.dumps(data))
        self.assertEqual(len(list(self.output.glob("*.png"))), 8)
        self.assertEqual(len(list(path.parent.iterdir())), 13)
        old = self.contents()
        with patch("gnorix_reports.cli.subprocess.run", side_effect=OSError("failed native run")):
            with self.assertRaises(OSError):
                generate_report(self.image, 50, 0.5)
        self.assertEqual(self.contents(), old)
        with patch("gnorix_reports.cli.subprocess.run", side_effect=self.native_success):
            generate_report(self.image, 50, 0.5)
        self.assertEqual(len(list(self.output.glob("*.png"))), 9)
        self.assertEqual(len(list(path.parent.iterdir())), 18)
        self.assertIn("clarity", json.loads(path.read_text()))
        self.assert_only_canonical_folder()

    def test_optional_combinations_survive_failures_and_expire_on_successful_refresh(self):
        for native, include_json, include_png, schema in (
            (False, True, False, "gnorix.feature-combinations.v5"),
            (False, True, True, "gnorix.feature-combinations.v5"),
            (True, True, True, "gnorix.feature-combinations.v5"),
            (True, True, True, "gnorix.feature-combinations.v6"),
            (False, True, False, "gnorix.feature-combinations.v4"),
            (False, True, True, "gnorix.feature-combinations.v4"),
            (True, True, True, "gnorix.feature-combinations.v4"),
            (True, False, True, "gnorix.feature-combinations.v4"),
            (False, True, True, "gnorix.feature-combinations.v2"),
            (True, True, True, "gnorix.feature-combinations.v2"),
            (False, True, True, "gnorix.feature-combinations.v3"),
            (True, True, True, "gnorix.feature-combinations.v3"),
        ):
            with self.subTest(native=native, json=include_json, png=include_png, schema=schema):
                if native:
                    self.complete_previous()
                self.write_combinations(
                    include_json=include_json, include_png=include_png, schema=schema
                )
                previous = self.contents()

                with patch("gnorix_reports.cli.subprocess.run", side_effect=OSError("extraction")):
                    with self.assertRaisesRegex(OSError, "extraction"):
                        generate_report(self.image, 50, 0.5)
                self.assertEqual(self.contents(), previous)
                self.assert_only_canonical_folder()

                with patch("gnorix_reports.cli.subprocess.run", side_effect=self.native_success):
                    with patch(
                        "gnorix_reports.cli.render_structure", side_effect=ValueError("render")
                    ):
                        with self.assertRaisesRegex(ValueError, "render"):
                            generate_report(self.image, 50, 0.5)
                self.assertEqual(self.contents(), previous)
                self.assert_only_canonical_folder()

                # Rendering itself is exercised above; this case checks publication ownership.
                def render_overview(snapshot, destination, dpi, alpha):
                    shutil.copyfile(self.image, destination / "overview.png")

                with patch("gnorix_reports.cli.subprocess.run", side_effect=self.native_success):
                    with patch("gnorix_reports.cli.render_structure", side_effect=render_overview):
                        generate_report(self.image, 50, 0.5)
                self.assertTrue((self.output / "overview.png").is_file())
                self.assertTrue((self.output / "data" / "features.json").is_file())
                self.assertFalse((self.output / "feature-combinations.json").exists())
                self.assertFalse((self.output / "feature-combinations.png").exists())
                self.assert_only_canonical_folder()
                shutil.rmtree(self.output.parent)

    def test_unidentified_combination_files_and_symlinks_remain_protected(self):
        outside = self.root / "outside.json"
        outside.write_text('{"schema":"gnorix.feature-combinations.v4"}')
        for native in (False, True):
            for case in (
                "wrong-schema",
                "obsolete-v1-schema",
                "malformed-json",
                "non-object",
                "extra",
                "json-link",
                "png-link",
                "json-directory",
                "png-directory",
            ):
                with self.subTest(native=native, case=case):
                    if native:
                        self.complete_previous()
                    self.write_combinations()
                    data = self.output / "feature-combinations.json"
                    if case == "wrong-schema":
                        data.write_text('{"schema":"other"}')
                    elif case == "obsolete-v1-schema":
                        data.write_text('{"schema":"gnorix.feature-combinations.v1"}')
                    elif case == "malformed-json":
                        data.write_text("{")
                    elif case == "non-object":
                        data.write_text("[]")
                    elif case == "extra":
                        (self.output / "keep.txt").write_text("keep")
                    else:
                        path = (
                            data
                            if case.startswith("json")
                            else self.output / "feature-combinations.png"
                        )
                        path.unlink()
                        if case.endswith("link"):
                            path.symlink_to(outside)
                        else:
                            path.mkdir()
                    before = self.contents()
                    with patch("gnorix_reports.cli.subprocess.run") as run:
                        with self.assertRaises(ValueError):
                            generate_report(self.image, 50, 0.5)
                    run.assert_not_called()
                    self.assertEqual(self.contents(), before)
                    self.assertEqual(
                        outside.read_text(), '{"schema":"gnorix.feature-combinations.v4"}'
                    )
                    shutil.rmtree(self.output.parent)

        self.write_combinations(include_json=False)
        with patch("gnorix_reports.cli.subprocess.run") as run:
            with self.assertRaises(ValueError):
                generate_report(self.image, 50, 0.5)
        run.assert_not_called()
        self.assertTrue((self.output / "feature-combinations.png").is_file())

    def test_grid_stage_identification_and_refresh_preserve_the_same_ownership_rules(self):
        self.output.mkdir(parents=True)
        data = self.output / "feature-grid.json"
        picture = self.output / "feature-grid.png"
        shutil.copyfile(self.image, picture)
        with patch("gnorix_reports.cli.subprocess.run") as run:
            with self.assertRaises(ValueError):
                generate_report(self.image, 50, 0.5)
            data.write_text('{"schema":"wrong"}')
            with self.assertRaises(ValueError):
                generate_report(self.image, 50, 0.5)
            run.assert_not_called()

        data.write_text('{"schema":"gnorix.feature-grid.v1"}')
        self.write_combinations()
        previous = self.contents()
        with patch("gnorix_reports.cli.subprocess.run", side_effect=OSError("extraction")):
            with self.assertRaises(OSError):
                generate_report(self.image, 50, 0.5)
        self.assertEqual(self.contents(), previous)

        def render_overview(snapshot, destination, dpi, alpha):
            shutil.copyfile(self.image, destination / "overview.png")

        with patch("gnorix_reports.cli.subprocess.run", side_effect=self.native_success):
            with patch("gnorix_reports.cli.render_structure", side_effect=render_overview):
                generate_report(self.image, 50, 0.5)
        self.assertFalse(data.exists())
        self.assertFalse(picture.exists())
        self.assertFalse((self.output / "feature-combinations.json").exists())
        self.assertTrue((self.output / "overview.png").is_file())
        self.assert_only_canonical_folder()

    def test_extraction_and_render_failures_preserve_the_previous_complete_report(self):
        self.complete_previous()
        previous = self.contents()
        failures = (
            (subprocess.CompletedProcess([], 7, "", "synthetic extractor failure"), None),
            (None, ValueError("synthetic invalid geometry")),
        )
        for extraction_failure, render_failure in failures:
            with self.subTest(extraction=extraction_failure):
                with patch("gnorix_reports.cli.subprocess.run") as run:
                    run.side_effect = self.native_success if extraction_failure is None else None
                    run.return_value = extraction_failure
                    with patch("gnorix_reports.cli.render_structure", side_effect=render_failure):
                        with contextlib.redirect_stderr(io.StringIO()) as stderr:
                            with self.assertRaises(SystemExit) as error:
                                main(["--image", str(self.image), "--dpi", "50"])
                self.assertEqual(error.exception.code, 2)
                self.assertIn("synthetic", stderr.getvalue())
                self.assertEqual(self.contents(), previous)
                self.assert_only_canonical_folder()

    def test_malformed_native_strength_preserves_the_previous_complete_report(self):
        self.complete_previous()
        previous = self.contents()

        def malformed(arguments, **options):
            result = self.native_success(arguments, **options)
            path = Path(arguments[-1]) / "features.json"
            data = json.loads(path.read_text())
            data["feature_strength"]["candidates"].pop()
            path.write_text(json.dumps(data))
            return result

        with patch("gnorix_reports.cli.subprocess.run", side_effect=malformed):
            with self.assertRaisesRegex(ValueError, "every raw node"):
                generate_report(self.image, 50, 0.5)
        self.assertEqual(self.contents(), previous)
        self.assert_only_canonical_folder()

    def test_previous_bundle_without_strength_refreshes_in_the_same_directory(self):
        self.complete_previous()
        (self.output / "feature-strength.png").unlink()
        path = self.output / "data" / "features.json"
        data = json.loads(path.read_text())
        del data["feature_strength"]
        path.write_text(json.dumps(data))
        with patch("gnorix_reports.cli.subprocess.run", side_effect=self.native_success):
            generate_report(self.image, 50, 0.5)
        self.assertEqual(len(list(self.output.glob("*.png"))), 9)
        self.assertIn("feature_strength", json.loads(path.read_text()))
        self.assert_only_canonical_folder()

    def test_failed_first_run_leaves_no_report_lock_or_staging_directories(self):
        with patch("gnorix_reports.cli.subprocess.run", side_effect=FileNotFoundError("cargo")):
            with self.assertRaises(FileNotFoundError):
                generate_report(self.image, 50, 0.5)
        self.assertFalse(self.output.exists())
        self.assertEqual(list(self.output.parent.iterdir()), [])

    def test_publication_failure_rolls_back_the_previous_report(self):
        self.complete_previous()
        before = self.contents()
        staging = self.output.parent / "pending"
        staging.mkdir()
        (staging / "new.txt").write_text("new")
        backup = self.output.parent / "previous"
        rename = Path.rename

        def failing_rename(source, destination):
            if source == staging:
                raise OSError("synthetic publication failure")
            return rename(source, destination)

        with patch.object(Path, "rename", failing_rename):
            with self.assertRaisesRegex(OSError, "publication"):
                _publish(staging, self.output, backup)
        self.assertEqual(self.contents(), before)
        self.assertFalse(backup.exists())
        self.assertTrue((staging / "new.txt").exists())

    def test_failed_rollback_preserves_the_backup_and_reports_its_recovery_path(self):
        self.complete_previous()
        before = self.contents()
        rename = Path.rename

        def failing_rename(source, destination):
            if source.name in ("report", "previous"):
                raise OSError("synthetic rename failure")
            return rename(source, destination)

        with patch("gnorix_reports.cli.subprocess.run", side_effect=self.native_success):
            with patch("gnorix_reports.cli.render_structure"):
                with patch.object(Path, "rename", failing_rename):
                    with self.assertRaisesRegex(
                        RuntimeError, "previous report preserved at"
                    ) as error:
                        generate_report(self.image, 50, 0.5)
        pending = list(self.output.parent.glob(".fingerprint-*"))
        self.assertEqual(len(pending), 1)
        self.assertEqual(self.contents(pending[0] / "previous"), before)
        self.assertIn(str(pending[0] / "previous"), str(error.exception))
        self.assertFalse((self.output.parent / ".fingerprint.lock").exists())

    def test_existing_run_lock_prevents_a_second_run_without_removing_the_lock(self):
        self.output.parent.mkdir()
        lock = self.output.parent / ".fingerprint.lock"
        lock.write_text("pid=123\n")
        with patch("gnorix_reports.cli.subprocess.run") as run:
            with self.assertRaisesRegex(ValueError, "another fingerprint report run"):
                generate_report(self.image, 50, 0.5)
        run.assert_not_called()
        self.assertEqual(lock.read_text(), "pid=123\n")

    def test_rejects_unrelated_files_and_symlinks_without_running_extraction(self):
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "keep.txt").write_text("keep")
        for case in ("extra", "output-link", "parent-link", "data-link", "wrong-schema"):
            with self.subTest(case=case):
                self.complete_previous()
                if case == "extra":
                    (self.output / "keep.txt").write_text("keep")
                elif case == "output-link":
                    shutil.rmtree(self.output)
                    self.output.symlink_to(outside, target_is_directory=True)
                elif case == "parent-link":
                    shutil.rmtree(self.output.parent)
                    self.output.parent.symlink_to(outside, target_is_directory=True)
                elif case == "data-link":
                    shutil.rmtree(self.output / "data")
                    (self.output / "data").symlink_to(outside, target_is_directory=True)
                else:
                    (self.output / "data" / "features.json").write_text('{"schema":"old"}')
                with patch("gnorix_reports.cli.subprocess.run") as run:
                    with self.assertRaises(ValueError):
                        generate_report(self.image, 50, 0.5)
                run.assert_not_called()
                self.assertEqual((outside / "keep.txt").read_text(), "keep")
                if self.output.parent.is_symlink():
                    self.output.parent.unlink()
                else:
                    shutil.rmtree(self.output.parent)

    def test_invalid_inputs_and_removed_commands_fail_before_native_execution(self):
        for options in (
            ["--dpi", "0"],
            ["--dpi", "601"],
            ["--overlay-alpha", "nan"],
            ["--overlay-alpha", "2"],
            ["--output", "elsewhere"],
            ["--overwrite"],
            ["--snapshot", "features.json"],
            ["--format", "png"],
            ["sample"],
            ["render"],
            ["structure"],
        ):
            with self.subTest(options=options), patch("gnorix_reports.cli.subprocess.run") as run:
                with (
                    contextlib.redirect_stderr(io.StringIO()),
                    self.assertRaises(SystemExit) as error,
                ):
                    main(["--image", str(self.image), *options])
                self.assertEqual(error.exception.code, 2)
                run.assert_not_called()
        with self.assertRaises(FileNotFoundError):
            generate_report(self.root / "missing.png", 50, 0.5)
        self.assertFalse(self.output.parent.exists())

    def test_checkout_discovery_is_independent_of_current_directory(self):
        actual = Path(__file__).resolve().parents[3]
        previous_cwd = Path.cwd()
        try:
            os.chdir(self.root)
            self.assertEqual(repository_root(), actual)
        finally:
            os.chdir(previous_cwd)
        installed = self.root / "lib" / "site-packages" / "gnorix_reports" / "cli.py"
        with patch("gnorix_reports.cli.__file__", str(installed)):
            with self.assertRaisesRegex(ValueError, "source checkout"):
                repository_root()


if __name__ == "__main__":
    unittest.main()
