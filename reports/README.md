# Generated reports and test artifacts

Generated content below this directory is ignored by Git. This README is maintained documentation.

| Location | Contents | Lifecycle |
| --- | --- | --- |
| `fingerprint/` | Nine diagnostic PNGs, generated index and native measurements in `data/`. | The report command replaces the managed bundle after successful extraction/rendering. |
| `fingerprint/feature-combinations.json` | Regional point subsets and their controls. | Capture-specific; generate after extraction. |
| `fingerprint/feature-grid.json` | Grid normalization and positional variants. | Derived from the combination export. |
| `enrollment/` | Public `<merkle-root>.grx` packages. | Separate from fingerprint report refreshes. |
| `enrollment/signing/` | Outputs written there by CLI signing examples. | Caller-managed; not removed by report refreshes. |
| `fingerprint-lab/` | Lab packages and public preparation/signing records. | Survive report refreshes; active Lab sessions are in memory. |

Generate the main report from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/core/tests/assets/test-fingerprint.png
```

Then open `fingerprint/overview.png` or the generated `fingerprint/README.md`. Keep unrelated files outside this managed directory. Combination/grid outputs must be regenerated when their source capture changes; see the [refresh boundary and current schema limitation](../tools/fingerprint-reports/README.md#refresh-and-recovery).

For subsequent stages, see [feature combinations](../docs/feature-combinations.md), [grid figures](../docs/grid-shapes.md), [enrollment packages](../docs/grx-packages.md), [fresh-capture signing](../docs/fingerprint-signing.md) and the [Fingerprint Lab](../docs/fingerprint-lab.md). The separate [NEAR demo](../examples/near/README.md) holds its visitor-session data in memory rather than writing these report directories.
