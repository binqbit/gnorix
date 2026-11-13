import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import import_socofing as importer
from PIL import Image


class GalleryImportTests(unittest.TestCase):
    def test_transparent_edge_padding_is_removed_without_changing_observed_pixels(self):
        source = Image.new("RGBA", (4, 3), (0, 0, 0, 0))
        pixels = [15, 80, 230, 90]
        for (x, y), value in zip([(0, 0), (1, 0), (0, 1), (1, 1)], pixels):
            source.putpixel((x, y), (value, value, value, 255))
        result = importer.observed_grayscale(source)
        self.assertEqual(result.size, (2, 2))
        self.assertEqual(result.tobytes(), bytes(pixels))

    def test_missing_interior_pixels_partial_alpha_and_color_are_rejected(self):
        for alpha in [0, 128]:
            source = Image.new("RGBA", (3, 3), (90, 90, 90, 255))
            source.putpixel((1, 1), (90, 90, 90, alpha))
            with self.assertRaisesRegex(ValueError, "validity mask"):
                importer.observed_grayscale(source)
        with self.assertRaisesRegex(ValueError, "equal RGB"):
            importer.observed_grayscale(Image.new("RGB", (2, 2), (10, 20, 30)))

    def test_import_checks_all_sources_before_replacing_existing_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Real").mkdir()
            output = root / "gallery"
            output.mkdir()
            first = self.create_source(root, "first", [0, 128, 200, 255])
            second = self.create_source(root, "second", [30, 80, 100, 220])
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"images": [first, second]}))
            (output / "first.png").write_bytes(b"existing image")
            (root / second["source"]).write_bytes(b"unexpected dataset version")
            with (
                patch.object(importer, "MANIFEST", manifest),
                self.assertRaisesRegex(ValueError, "source SHA-256 mismatch"),
            ):
                importer.import_gallery(root, output)
            self.assertEqual((output / "first.png").read_bytes(), b"existing image")
            self.assertEqual(sorted(path.name for path in output.iterdir()), ["first.png"])

    def test_valid_import_is_lossless_and_rejects_an_incorrect_pixel_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Real").mkdir()
            values = [0, 99, 100, 255]
            entry = self.create_source(root, "example", values)
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"images": [entry]}))
            output = root / "gallery"
            with patch.object(importer, "MANIFEST", manifest):
                self.assertEqual(importer.import_gallery(root, output), 1)
                self.assertEqual(importer.import_gallery(root, output), 1)
            with Image.open(output / "example.png") as result:
                self.assertEqual(result.mode, "L")
                self.assertEqual(result.size, (2, 2))
                self.assertEqual(result.tobytes(), bytes(values))
            entry["pixels_sha256"] = "0" * 64
            with self.assertRaisesRegex(ValueError, "pixel SHA-256 mismatch"):
                importer.prepare_image(root, entry)

    @staticmethod
    def create_source(root, name, values):
        source = root / "Real" / f"{name}.BMP"
        image = Image.frombytes("L", (2, 2), bytes(values))
        image.save(source, format="BMP")
        return {
            "id": name,
            "source": f"Real/{name}.BMP",
            "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
            "pixels_sha256": hashlib.sha256(bytes(values)).hexdigest(),
            "width": 2,
            "height": 2,
        }


if __name__ == "__main__":
    unittest.main()
