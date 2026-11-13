# Fingerprint Lab gallery import

The Lab keeps its synthetic test fingerprint and offers five curated SOCOFing originals when their local PNGs are installed. The selection is fixed in [gallery.json](../../crates/gnorix/examples/fingerprint_lab/gallery.json). The images are external research data, not bundled repository assets; see the [dataset provenance and usage terms](../../docs/fingerprint-testing.md).

From the repository root, after downloading SOCOFing version 2:

```sh
uv run --project tools/fingerprint-reports --locked python \
  tools/fingerprint-gallery/import_socofing.py \
  --source /absolute/path/to/SOCOFing
```

The source directory must contain `Real/`. The importer validates all five source-file hashes and observed pixel hashes before writing opaque grayscale PNGs into the ignored `test-data/fingerprint-gallery/` directory. It preserves native scale and intensities. Transparent edge padding is removed only when the retained rectangle is fully opaque; missing interior pixels and partial alpha are rejected. Repeating the command safely refreshes those five files. `--output` selects another local directory.

The curated originals are 241 × 298 pixels. They were chosen for visible ridge separation, varied ridge flow, distinct subject labels and successful preparation with the Lab's default priority and spatial constraints. Selection used a visual review of successful native results from 44 larger originals and 40 contrast-screened standard originals. This is a demonstration gallery, not a population quality ranking or recognition benchmark. It deliberately avoids choosing scans solely by detected feature count, which can favor broken ridges and artifacts.

| Original | Eligible features at priority ≥ 0.5 | Default combinations |
| --- | ---: | ---: |
| `226__M_Right_middle_finger.BMP` | 47 | 33 |
| `270__M_Right_index_finger.BMP` | 20 | 24 |
| `53__M_Right_ring_finger.BMP` | 14 | 24 |
| `560__F_Left_little_finger.BMP` | 25 | 24 |
| `590__M_Left_middle_finger.BMP` | 16 | 24 |

Counts describe the current extractor and default controls; changing those controls or extraction mechanics may change them. Passing preparation does not establish biometric accuracy or guarantee a key match from every cropped or altered scan.

Run importer tests without downloading the dataset:

```sh
uv run --project tools/fingerprint-reports --locked python -m unittest discover \
  -s tools/fingerprint-gallery -p 'test_*.py'
```
