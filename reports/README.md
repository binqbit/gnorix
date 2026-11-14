# Fingerprint report

The current analysis has one generated home: [fingerprint/](fingerprint/). Open [the overview](fingerprint/overview.png) or [the report index](fingerprint/README.md) for its detailed PNG figures.

Generate or refresh the complete report from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/core/tests/assets/test-fingerprint.png
```

The command runs native Rust extraction and publishes one consistent bundle in `reports/fingerprint/`. The next successful run replaces it. Source images and measurements are retained under its `data/` directory. Generated content is ignored by Git; this README is maintained documentation.

The [contour comparison](fingerprint/contour-comparison.png) shows observed structure alongside cleaned and reconstructed lines. It marks removed short terminal branches and collapsed narrow loops. Original-image evidence, retained alternatives and the original paths remain in `data/features.json`.

Cleanup allows a fixed ±20% width variation around the measured mean R: short branches and narrow loops must be smaller than 1.2R and still require original-image confirmation. The lower bound 0.8R does not exclude smaller artifacts. R remains the shared measurement unit.

The nine PNG figures include [local clarity](fingerprint/clarity-analysis.png), its native contrast/separation/repetition components and support mask. Contour opacity follows a fixed square-root display transfer from 40% to 90%, independent of other overlay settings. Unavailable path values are gray at 40%, while joins retain their acceptance colors at the same minimum opacity.

[Feature strength](fingerprint/feature-strength.png) retains every original node and isolated cycle. It separates extended source-confirmed context (C), local source/geometry (P/G), and stability across eight threshold variants (T). Intrinsic strength is `Q = C × (0.75 + 0.25 × min(P, G)) × (0.5 + 0.5 × T)`. Final priority is `Q × N`, with crowding and source coverage in radius 3R supplying a bounded factor N between 0.5 and 1. Feature brightness follows priority; diagnostics retain both Q and priority. Missing evidence remains unknown. A low rank does not delete a feature, and a high rank is not a calibrated recognition probability.

See the [tool guide](../tools/fingerprint-reports/README.md) for setup, opacity, PNG resolution and recovery behavior.

The separate [regional-combinations example](../docs/feature-combinations.md) writes its unified point-subset list to `fingerprint/feature-combinations.json`. An optional `fingerprint/feature-combinations.png` can illustrate the adaptive selected pools. These outputs belong to one capture and its explicit combination settings; refreshing the main report removes them along with the previous analysis. Generate the combinations again after a refresh.

The next [grid-figure stage](../docs/grid-shapes.md) consumes that JSON and writes `fingerprint/feature-grid.json`: hull orientation, nearest nodes at measured ridge width R, fixed padding, and every positional variant. An optional `fingerprint/feature-grid.png` illustrates the transformation and neighbor choices. These files follow the same report refresh lifecycle.

The [enrollment example](../docs/grx-packages.md) can write `enrollment/<merkle-root>.grx`. This separate package contains context and Merkle leaf hashes for later public-key membership lookup. Fingerprint report refresh does not remove enrollment packages.

The [fresh-image signing example](../docs/fingerprint-signing.md) keeps crop images, public authorizations and its [run record](enrollment/signing/results.json) under `enrollment/signing/`. It extracts each image again and signs with the first key found in the existing package. The current full-image and four crop runs all verified; these are crops of one synthetic fixture, not independent sensor captures. This directory also survives fingerprint report refreshes.

The [interactive fingerprint test stand](../docs/fingerprint-lab.md) keeps generated packages and public test records under `fingerprint-lab/`. Its active session is held in memory and its artifacts survive fingerprint-report refreshes.
