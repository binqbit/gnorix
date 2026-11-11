# Generated reports

The fingerprint report tool writes PNG reports here by default, regardless of the working directory when launched from this source checkout. Generated report files and bundles are ignored by Git; this README is maintained documentation.

From the repository root:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report sample \
  --image crates/gnorix/tests/assets/test-fingerprint.png
```

Use `--output reports/<name>` to keep separate runs. Sample bundles contain PNG images and a `snapshot.json` with the measurements used to render them. To render a saved bundle again:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report render \
  reports/snapshot.json --output reports/replay
```

See the [tool guide](../tools/fingerprint-reports/README.md) for point/tolerance overlays, crops, opacity controls and the snapshot format. Generated files may contain the source fingerprint and its measurements; share them intentionally.
