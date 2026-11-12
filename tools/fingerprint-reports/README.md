# Fingerprint report

Generate the current native fingerprint analysis and its PNG report with one command from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/gnorix/tests/assets/test-fingerprint.png
```

The command runs the Rust extractor, validates its measurements and geometry, and publishes the complete result in **`reports/fingerprint/`**. Every successful run refreshes this same directory. Open `overview.png` first; the generated `README.md` links the detailed figures and records the actual run's measurements.

## Requirements

Use the Gnorix checkout with its pinned Rust toolchain and `uv`. The Python tool requires Python 3.11 or later; `uv.lock` pins its matplotlib, NumPy and Pillow environment. It invokes Cargo from the checkout root, so discovery of `reports/fingerprint/` does not depend on the launch directory. An explicitly supplied relative image path resolves from the caller's working directory.

The input must be an opaque PNG. The Rust adapter converts it to grayscale and rejects transparency, excessive dimensions and unsupported inputs. Source dimensions, grayscale conversion and algorithm bounds are described in the [fingerprint guide](../../docs/fingerprint.md). The checked-in fingerprint is a synthetic fixture.

## One report directory

```text
reports/fingerprint/
  README.md
  overview.png
  structure-overlay.png
  scale-analysis.png
  bootstrap-analysis.png
  normalization-analysis.png
  contour-comparison.png
  contour-overlay.png
  data/
    features.json
    source.png
    ... native grayscale processing stages
```

| Figure | Content |
|---|---|
| `overview.png` | Source, normalization, denoising, coherence, usable region, binary ridges and skeleton, with measurement summaries. |
| `structure-overlay.png` | Observed paths and ending/junction/isolated-node candidates, including unknown continuation. |
| `scale-analysis.png` | Full-source length-weighted mean ridge width R, measured/unmeasured coverage and width consistency. |
| `bootstrap-analysis.png` | Initial ridge triples, provisional widths and period, and original grayscale measurements. |
| `normalization-analysis.png` | Local dark/light reference maps, supported regions and normalization behavior. |
| `contour-comparison.png` | Observed and reconstructed paths on identical source coordinates, with a matched detail. |
| `contour-overlay.png` | Smooth contours and explicit accepted or unaccepted gap continuations. |

The full-source width mean is the common unit R. Bootstrap measurements initialize the pipeline and remain separate diagnostics. Width consistency at ±10%, ±20% and ±30% describes measured variation; it is not a recognition probability. The scale overlay displays a spatial subset for readability, while every measurement contributes to its statistics.

Observed and reconstructed geometry remain distinct. Green bridge segments are accepted source-supported joins; amber dashed segments are proposals that did not change connectivity. The comparison detail selects the longest accepted join or, when none exists, the path with the largest smoothing displacement. Reconstruction does not establish anatomical correctness.

`data/features.json` retains all measured widths, source-coordinate paths, local-reference metadata, reconstruction provenance and processing controls. The adjacent images are the extractor's original 8-bit grayscale PNG stages. The renderer checks dimensions, image containment, finite values, scale summaries, path origins, endpoint preservation and join evidence before publishing. Image processing is local; fingerprints and measurements are not uploaded.

## Display controls

- `--overlay-alpha 0.5`: geometry opacity in `[0, 1]`; the source image stays fully visible. Default: 0.5.
- `--dpi 160`: PNG export resolution from 50 to 600. Default: 160.

These options change presentation only. They do not change ridge extraction, matching tolerances or the source-coordinate system. The command generates PNG figures only.

## Refresh and recovery

Extraction and rendering finish in a temporary sibling directory before publication. The previous report is then moved aside, the completed report takes its place, and the previous generated tree is removed. A handled publication failure restores the previous report; the swap is not a guarantee of uninterrupted directory visibility to concurrent readers. If restoring the previous directory also fails, the command retains its backup and reports its location.

The command refuses symlink destinations and unrecognized files in the managed report tree. Keep personal files outside `reports/fingerprint/`. An exclusive `reports/.fingerprint.lock` prevents simultaneous writers. A process killed without cleanup can leave the lock; remove it only after confirming that no report generation is running. Successful runs leave only the canonical report directory and the maintained root `reports/README.md`.

## Verification

```sh
uv run --project tools/fingerprint-reports --locked ruff check tools/fingerprint-reports
uv run --project tools/fingerprint-reports --locked ruff format --check tools/fingerprint-reports
uv run --project tools/fingerprint-reports --locked python -Werror -m unittest discover \
  -s tools/fingerprint-reports/tests -v
```

CI also executes the one-command report generation against the checked-in fingerprint. Tests exercise native measurement validation, geometry and provenance, image decoding, complete report publication and failure recovery. They do not establish real-scanner recognition accuracy.
