# Fingerprint report

The current analysis has one generated home: [fingerprint/](fingerprint/). Open [the overview](fingerprint/overview.png) or [the report index](fingerprint/README.md) for its detailed PNG figures.

Generate or refresh the complete report from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/gnorix/tests/assets/test-fingerprint.png
```

The command runs native Rust extraction and publishes one consistent bundle in `reports/fingerprint/`. The next successful run replaces it. Source images and measurements are retained under its `data/` directory. Generated content is ignored by Git; this README is maintained documentation.

The eight PNG figures include [local clarity](fingerprint/clarity-analysis.png), its native contrast/separation/repetition components and support mask. Contour opacity follows a fixed square-root display transfer from 40% to 90%, independent of other overlay settings. Unavailable path values are gray at 40%, while joins retain their acceptance colors at the same minimum opacity. These diagnostics do not change geometry or establish recognition accuracy.

See the [tool guide](../tools/fingerprint-reports/README.md) for setup, opacity, PNG resolution and recovery behavior.
