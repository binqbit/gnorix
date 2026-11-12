# Fingerprint report

The current analysis has one generated home: [fingerprint/](fingerprint/). Open [the overview](fingerprint/overview.png) or [the report index](fingerprint/README.md) for its detailed PNG figures.

Generate or refresh the complete report from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/gnorix/tests/assets/test-fingerprint.png
```

The command runs native Rust extraction and publishes one consistent bundle in `reports/fingerprint/`. The next successful run replaces it. Source images and measurements are retained under its `data/` directory. Generated content is ignored by Git; this README is maintained documentation.

The [contour comparison](fingerprint/contour-comparison.png) shows observed structure alongside cleaned and reconstructed lines. It marks removed short terminal branches and collapsed narrow loops. Original-image evidence, retained alternatives and the original paths remain in `data/features.json`.

Cleanup allows a fixed ±20% width variation around the measured mean R: short branches and narrow loops must be smaller than 1.2R and still require original-image confirmation. The lower bound 0.8R does not exclude smaller artifacts. R remains the shared measurement unit.

The nine PNG figures include [local clarity](fingerprint/clarity-analysis.png), its native contrast/separation/repetition components and support mask. Contour opacity follows a fixed square-root display transfer from 40% to 90%, independent of other overlay settings. Unavailable path values are gray at 40%, while joins retain their acceptance colors at the same minimum opacity.

[Feature strength](fingerprint/feature-strength.png) retains every original node and isolated cycle. It separates original-image support (P), geometric support (G) and persistence across eight controlled threshold variants (T). The combined rank is `min(P, G) × (0.5 + 0.5 × T)`. Missing evidence remains unknown. A low rank does not delete a feature, and a high rank is not a calibrated recognition probability.

See the [tool guide](../tools/fingerprint-reports/README.md) for setup, opacity, PNG resolution and recovery behavior.
