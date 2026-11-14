# Fingerprint Lab gallery import

The Lab includes its synthetic test image and offers five curated SOCOFing originals when their local PNGs are installed. [gallery.json](../../examples/fingerprint-lab/gallery.json) pins the source-file hashes, decoded-pixel hashes, dimensions and display metadata. These external research images are not bundled repository assets; the [dataset terms and citations](../../docs/fingerprint-testing.md#provenance-and-use) apply.

From the repository root, after downloading SOCOFing version 2:

```sh
uv run --project tools/fingerprint-reports --locked python \
  tools/fingerprint-gallery/import_socofing.py \
  --source /absolute/path/to/SOCOFing
```

The source directory must contain `Real/`. All five sources and decoded pixel hashes are validated before writing opaque grayscale PNGs into the ignored `test-data/fingerprint-gallery/` directory. Each file is replaced individually. Native scale and grayscale intensities are preserved; transparent edge padding can be cropped only when the retained rectangle is fully opaque. Partial alpha or interior missing pixels are rejected.

Start or restart the Lab after importing. For another directory, pass `--output /path/to/gallery` to the importer and `-- --gallery /path/to/gallery` to `cargo run -p gnorix-lab --release --locked`. Missing entries are skipped; a present entry with invalid dimensions or pixels fails validation. The bundled synthetic image remains available without a download.

## Selection

All selected originals are 241 × 298 pixels:

| Source in `Real/` | Demonstration detail |
| --- | --- |
| `226__M_Right_middle_finger.BMP` | Broad turn with separated ridges. |
| `270__M_Right_index_finger.BMP` | Compact central spiral. |
| `53__M_Right_ring_finger.BMP` | Sweeping arcs and clear valleys. |
| `560__F_Left_little_finger.BMP` | Fine curves with softer contrast. |
| `590__M_Left_middle_finger.BMP` | Central turn and neighboring ridges. |

The selection followed visual review of native results from 44 larger originals and 40 contrast-screened standard originals. It provides varied ridge flow and subject labels, not a population quality ranking. Feature counts and usable combinations depend on the current extractor and controls; successful preparation does not guarantee recovery from every crop.

## Checks

Importer tests use local fixtures and require no dataset download:

```sh
uv run --project tools/fingerprint-reports --locked ruff check \
  --config tools/fingerprint-reports/pyproject.toml tools/fingerprint-gallery
uv run --project tools/fingerprint-reports --locked ruff format --check \
  --config tools/fingerprint-reports/pyproject.toml tools/fingerprint-gallery
uv run --project tools/fingerprint-reports --locked python -Werror -m unittest discover \
  -s tools/fingerprint-gallery -p 'test_*.py'
```
