"""Native image-stage decoding retains source bytes and enforces bounds."""

import tempfile
import unittest
from pathlib import Path

from PIL import Image

from gnorix_reports.images import MAX_IMAGE_BYTES, load_image


class ImageLoadingTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.base = Path(self.directory.name)

    def test_returned_copy_preserves_grayscale_after_the_source_is_removed(self):
        source = self.base / "source.png"
        with Image.new("L", (3, 2), 123) as image:
            image.save(source)
        with load_image(source, (3, 2)) as image:
            source.unlink()
            self.assertEqual(image.mode, "L")
            self.assertEqual(image.tobytes(), bytes([123] * 6))

    def test_non_native_image_modes_are_rejected_instead_of_converted(self):
        source = self.base / "stage.png"
        for mode, value in (("I;16", 65535), ("RGB", (10, 20, 30)), ("RGBA", (10, 20, 30, 40))):
            with self.subTest(mode=mode):
                with Image.new(mode, (3, 2), value) as image:
                    image.save(source)
                with self.assertRaisesRegex(ValueError, "8-bit grayscale"):
                    load_image(source, (3, 2))

    def test_excessive_file_size_is_rejected_before_decoding(self):
        source = self.base / "oversized.png"
        with source.open("wb") as stream:
            stream.truncate(MAX_IMAGE_BYTES + 1)
        with self.assertRaisesRegex(ValueError, "128 MiB"):
            load_image(source, (3, 2))

    def test_other_image_formats_are_rejected_even_with_png_extension(self):
        source = self.base / "disguised.png"
        for format in ("BMP", "JPEG"):
            with self.subTest(format=format):
                with Image.new("L", (3, 2)) as image:
                    image.save(source, format=format)
                with self.assertRaisesRegex(ValueError, "must be PNG"):
                    load_image(source, (3, 2))

    def test_mismatched_dimensions_are_rejected(self):
        source = self.base / "stage.png"
        with Image.new("L", (3, 2)) as image:
            image.save(source)
        with self.assertRaisesRegex(ValueError, "dimensions"):
            load_image(source, (4, 2))
