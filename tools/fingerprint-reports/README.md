# Fingerprint PNG reports

This tool runs the native Rust extractor and renders its measurements as nine static PNG figures. Python validates and presents the results; it does not extract features, alter topology or choose biometric thresholds. See the [algorithm reference](../../docs/fingerprint.md) for measurement and scoring definitions.

## Run

Requirements: the repository checkout, its pinned Rust toolchain, `uv` and Python 3.11 or newer. Follow the one-time [local setup](../../docs/local-testing.md) first. From the repository root, one command builds/runs the Rust extractor and produces every report figure:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/core/tests/assets/test-fingerprint.png
```

The first run downloads dependencies and builds the native example. Wait for the
printed `overview.png` path, then open **`reports/fingerprint/overview.png`**;
there is no web server or browser port. Run the same command again to refresh
the bundle. Reports inspect extraction and feature quality; they do not enroll
keys or execute NEAR transfers. For interactive signing use the
[Lab](../../examples/fingerprint-lab/README.md), or the
[NEAR demo](../../examples/near/README.md) for token transfers.

Input must be an opaque PNG. Transparency is rejected; missing pixels require the library's `Capture` validity mask. The native example limits source dimensions to 4096 per side and extraction to 4,000,000 pixels. Processing estimates its scale from the image; there is no manual ridge-period argument. For external research inputs, see [SOCOFing conversion](../../docs/fingerprint-testing.md#convert-without-inventing-scanner-evidence).

Output is always **`reports/fingerprint/` in this checkout**. Relative input paths use the caller's working directory. The tool uses no external image service and does not upload fingerprints. It generates no SVG or HTML reports.

| Option | Default | Meaning |
| --- | --- | --- |
| `--image PATH` | Required | Opaque source PNG. |
| `--dpi INTEGER` | 160 | Figure export resolution, 50–600. |
| `--overlay-alpha NUMBER` | 0.5 | Map and annotation opacity, 0–1. |

Display options do not change extraction, source coordinates or matching tolerances.

## Contents

The report root contains a generated index and these figures:

| File | Purpose |
| --- | --- |
| `overview.png` | Source and processing stages. |
| `structure-overlay.png` | Final contours and surviving features. |
| `scale-analysis.png` | Full width traversal, measured R and coverage. |
| `bootstrap-analysis.png` | Provisional ridge/gap widths, period and source brightness. |
| `normalization-analysis.png` | Local dark/light references and normalization support. |
| `contour-comparison.png` | Observed versus reconstructed geometry, with a shared detail crop. |
| `contour-overlay.png` | Reconstructed contours and gap proposals on the source. |
| `clarity-analysis.png` | Local clarity, its components and support. |
| `feature-strength.png` | All original candidates and their evidence/priority diagnostics. |

`data/features.json` contains native measurements, controls, provenance, paths, per-segment clarity and candidate evidence. Adjacent grayscale PNGs retain source and intermediate maps, including their support masks. Unsupported map slots must not be interpreted as measured zeros. Byte-valued maps are rounded exports; JSON measurements retain floating-point values.

The main structural overlay follows cleanup/reconstruction provenance. Removed candidates stay in the feature-strength diagnostic; changed types/degrees appear gray and unassessed in the main overlay rather than inheriting the old score. The compact library analysis excludes those changed candidates. Dashed removed paths in the comparison show confirmed artifacts, while proposed joins retain their acceptance status.

Feature color uses a fixed 0–1 priority scale: red → orange → yellow → bright green. Shapes distinguish types. Marker opacity is `0.4 + 0.5 × sqrt(priority)`; contour opacity uses the same transfer with local clarity. Both span 40–90%, independently of `--overlay-alpha`. Unavailable scores use gray; measured zero remains a valid low score. The source image remains fully opaque. Scores and visibility are not recognition probabilities.

## Refresh and recovery

A run extracts, validates and renders in a temporary sibling directory before replacing the managed report. A handled publication failure restores the previous directory; if restoration also fails, the backup is retained and its location reported. The rename sequence does not guarantee uninterrupted visibility to concurrent readers.

The tool refuses symlink destinations, unrelated files and unrecognized schemas. Keep personal files outside `reports/fingerprint/`. An exclusive `reports/.fingerprint.lock` prevents simultaneous writers; remove a stale lock only after confirming its process is no longer running. Other report directories are not replaced.

Recognized combination/grid exports inside the managed directory, including the current `gnorix.feature-combinations.v7` export, are removed on refresh because they belong to the previous capture. Unknown export schemas remain protected from replacement.

## Implementation and checks

- `cli.py` owns native execution, locking, destination validation and report replacement/recovery.
- `snapshot.py` and measurement modules load bounded JSON and validate dimensions, values, geometry, formulas and provenance.
- `structure.py` declares the figure sequence and renders one figure at a time. The render modules only draw validated results.

Run local checks without downloading research data:

```sh
uv run --project tools/fingerprint-reports --locked ruff check tools/fingerprint-reports
uv run --project tools/fingerprint-reports --locked ruff format --check tools/fingerprint-reports
uv run --project tools/fingerprint-reports --locked python -Werror -m unittest discover \
  -s tools/fingerprint-reports/tests -v
```

The Python tests validate report parsing, rendering contracts and publication
behavior. To test Rust extraction directly, run
`cargo test -p gnorix-core --test fingerprint --release --locked`.
CI also runs the report command above on the checked-in synthetic fixture,
covering native extraction through PNG publication. None of these checks needs
a scanner or research dataset, and they do not measure sensor recognition
accuracy.
