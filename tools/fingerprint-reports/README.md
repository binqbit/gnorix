# Fingerprint reports

Generate static PNG diagnostics from a processing snapshot. Each report brings together the source image, coordinate-aligned points and grid, a metric distribution, and any supplied raster or line series. Separate overlay figures show points and supplied tolerance distances directly over the same image coordinates. The tool runs locally and accepts snapshots from any language or pipeline; it does not call Gnorix's Rust library or select its biometric algorithms.

## Setup and first report

Run these commands from the repository root. Install [uv](https://docs.astral.sh/uv/) first; this Python 3.11+ project locks its matplotlib, Pillow and NumPy dependencies in `uv.lock`.

```sh
uv sync --project tools/fingerprint-reports --locked

uv run --project tools/fingerprint-reports --locked gnorix-report sample \
  --image crates/gnorix/tests/assets/test-fingerprint.png \
  --spacing 36 \
  --radius 12
```

The default output is the repository-root `reports/` directory. It is discovered from the project location even when the command is launched from another working directory. To group a run separately, use an explicit path such as `--output reports/fingerprint` from the repository root; relative overrides resolve from your working directory.

The sample command prepares a snapshot bundle and renders reports into the output directory. It writes `snapshot.json`, a grayscale `capture.png`, and PNG versions of an input overview plus the two metric reports and their separate overlays. It reads the supplied image, places a triangular preview grid over it, and computes local mean intensity and standard deviation. These are descriptive image measurements, not a biometric weight, matching score, acceptance criterion or key-reconstruction algorithm. The repository image is synthetic; it is not a capture from a scanner or a real person's finger.

`--spacing` and `--radius` are pixel dimensions for this sample calculation. The radius is an integer from 1 to 2048; the square window is centered on the containing pixel and clipped at image boundaries. `--rotation` rotates the preview grid in degrees about the image center. All image pixels are eligible, including background; no fingerprint segmentation is inferred. The sample also supplies a reduced intensity raster and a center-row intensity profile for the report panels. Transparent sample inputs must be explicitly composited before use.

These controls are local inspection choices; they do not configure the Rust core or establish biometric criteria. Defaults are spacing 36 pixels, radius 12 pixels, rotation 0 degrees and DPI 160. Figures are exported as PNG. `--dpi` accepts values from 50 to 600 and controls export resolution.

To render a snapshot from your own pipeline:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report render snapshot.json \
  --output reports/custom \
  --dpi 160
```

The resulting files are static images that can be opened directly in an image viewer or placed in a report. No server or browser application is required.

To select metrics, repeat `--metric` with exact keys on `render`, for example `--metric intensity --metric contrast` for the sample snapshot. An unknown metric key is an error. Without this filter every stage and metric is rendered, including an overview for stages without metrics.

Existing matching output files cause an error. Add `--overwrite` to explicitly replace them; unrelated files in the output directory are retained. The sample bundle can be rendered again by passing its `snapshot.json` to `render` and choosing another output directory.

## Translucent point and distance overlays

Both `sample` and `render` accept these display controls:

| Option | Meaning |
|---|---|
| `--overlay-alpha 0.5` | Opacity of overlay geometry, from 0 to 1; default 0.5. The source image remains at full opacity. |
| `--point-radius 3` | Positive radius of point-center markers in source-image pixels; default 3. |
| `--tolerance-radius 12` | Optional nonnegative circular distance in source-image pixels, overriding each point's supplied `toleranceRadius`. No tolerance is inferred when neither is supplied. |
| `--crop X Y WIDTH HEIGHT` | Show a rectangle measured in the original image's pixels, retaining its original coordinate labels. The rectangle must have positive size and lie within every selected frame. |
| `--overlay-only` | Export dedicated overlay figures instead of the full multipanel reports. The sample command still prepares its image and snapshot bundle. |

For a detailed view of the repository's actual synthetic image:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report sample \
  --image crates/gnorix/tests/assets/test-fingerprint.png \
  --spacing 36 \
  --radius 12 \
  --tolerance-radius 12 \
  --overlay-alpha 0.5 \
  --overlay-only \
  --crop 360 420 320 320 \
  --output reports/fingerprint-overlay
```

Here `--radius 12` selects the square intensity-measurement window, while `--tolerance-radius 12` draws a circular distance around each point. They have separate meanings. Both values are demonstration settings; neither selects a real biometric tolerance or acceptance criterion.

The small center marks the exact point coordinate. Its surrounding circle shows the supplied distance from that center. Center markers and circles use the same pixel units and equal axis scale as the image; the image and overlays scale together when displayed or exported at another size. PNG overlays retain source-coordinate rulers. Cropping changes the displayed region without translating point coordinates or modifying the snapshot. Geometry crossing an image or crop boundary is clipped visually. A cropped full report's histogram still summarizes the entire stage, not only the visible points.

Crop ranges that cannot be represented accurately by the plotting axes are rejected, so the renderer never silently expands a numerically degenerate crop.

Circles provide an isotropic distance visualization only. They do not implement an error model, infer direction-dependent uncertainty, or change which observations are accepted by the biometric pipeline.

## Snapshot format

The schema identifier is `gnorix.inspection.v1`. Save the JSON file and optional image files together as a bundle. This minimal example renders without an image file:

```json
{
  "schema": "gnorix.inspection.v1",
  "title": "Provided point weights",
  "description": "Illustrative values supplied by a pipeline, not calculated by the renderer.",
  "frames": [
    {
      "id": "capture",
      "label": "Capture coordinates",
      "width": 100,
      "height": 80
    }
  ],
  "stages": [
    {
      "id": "weights",
      "label": "Point weights",
      "frame": "capture",
      "metrics": [
        { "key": "weight", "label": "Provided weight", "domain": [0, 1] }
      ],
      "points": [
        { "id": "a", "x": 20, "y": 20, "toleranceRadius": 12, "values": { "weight": 0 } },
        { "id": "b", "x": 70, "y": 50, "toleranceRadius": null, "values": { "weight": 1 } }
      ]
    }
  ]
}
```

The renderer displays provided weights; it does not derive a weight formula. A metric's optional domain sets its display scale, not a pass/fail threshold.

| Object | Fields and meaning |
|---|---|
| Root | `schema`, `title`, optional `description`, `frames`, `stages`. |
| Frame | Unique `id`, `label`, positive pixel `width` and `height`, optional `image`. |
| Stage | Unique `id`, `label`, optional `description`, a `frame` ID, `metrics`, and optional `points`, `raster`, `series`. |
| Metric | `key`, `label`, optional `unit`, optional numeric `domain: [min, max]`. |
| Point | `id`, pixel coordinates `x` and `y`, optional paired integer lattice coordinates `q` and `r`, optional `valid`, optional `toleranceRadius`, and `values` keyed by metric name. |
| Raster | `label`, pixel rectangle `x`, `y`, `width`, `height`, positive `columns` and `rows`, row-major `values`, optional `domain`. |
| Series | `id`, `label`, `xLabel`, `yLabel`, and `points` containing numeric `x` and `y`. |

All spatial coordinates use the referenced frame: origin at its top-left, x increasing rightward and y downward, in pixels. Points must satisfy `0 <= x < width` and `0 <= y < height`. A transformed stage should reference a separate frame when its coordinate system changes. `q` and `r` identify triangular-lattice neighbors for grid edges; supply both together when that relationship exists. They do not replace the point's pixel coordinates, and duplicate lattice coordinates within a stage are rejected.

`toleranceRadius` is optional: omit it or use `null` for an unspecified distance, or supply a finite nonnegative pixel radius, including zero. It is retained in the snapshot and describes display geometry only. `--tolerance-radius` supplies a render-time override for all points; it does not turn that distance into a biometric acceptance rule.

An image is optional. For a background image, add a relative path such as `"image": "scan.png"` to the frame. PNG and JPEG files must stay within the snapshot bundle. Absolute paths, URLs, parent-directory traversal and symlink escapes are rejected. Image dimensions must agree with the frame; report background images are limited to 128 MiB per file.

Supported images use 8-bit `L`, `LA`, `P`, `RGB` or `RGBA` modes. Higher-depth inputs are rejected rather than silently reduced. Sample calculation requires opaque input and explicitly converts it to grayscale. Extremely large finite values that cannot be plotted with safe floating-point arithmetic are rejected with an error.

Metric values are finite numbers or `null`. Zero is a measured value and remains visible in statistics. A `null` or absent metric value is missing, not zero. Points marked `"valid": false` are identified as rejected and excluded from metric statistics. The report distinguishes missing and rejected observations instead of silently counting them as successful measurements.

A raster's rectangle defines its placement and extent in frame pixels; its value count must equal `columns * rows`. It can describe a dense response map without inventing point-grid relationships. Supplied line series are plotted under their own axis labels; they are not inferred from point order or interpreted as additional biometric criteria.

The validator accepts at most 24 frames, 100 stages, 256 metrics and 100,000 points per stage, 32 series per stage, 20,000 points per series, and 1,000,000 cells per raster. Frame dimensions are limited to 16,384 pixels per side and 16,777,216 pixels total; the JSON file limit is 64 MiB. These are inspection-tool resource bounds, not biometric acceptance settings.

## Reports and integration

By default, rendering produces one multipanel report and one dedicated overlay per stage and metric. The reports include optional raster and series panels. PNG file names contain stage and metric indexes plus sanitized identifiers, for example `01-weights-01-weight.png`; separate overlays append `-overlay`, such as `01-weights-01-weight-overlay.png`. `--overlay-only` produces only those overlay figures. Indexes keep distinct stages or metrics from overwriting one another when their identifiers sanitize to the same name. A stage without metrics produces an overview such as `01-input-overview.png` and its overlay when no metric filter is active.

Export snapshots at the stages you want to inspect. Keep stage names, metric labels, units and coordinate frames explicit so reports can be compared meaningfully. A producer can be a Rust experiment, Python processing script or another program; the renderer only consumes the JSON contract. There is no required implementation inside the biometric core and no automatic export from signing or enrollment.

The tool reads explicitly selected local files and writes requested bundles and PNG reports to the repository's Git-ignored `reports/` directory by default, or to the explicit `--output` location. It performs no telemetry or remote upload. A snapshot or image report can include its source image, point locations and measurements; sharing those files is a separate user action. Owned-buffer clearing in the Rust library does not remove files exported by this tool.

## Tests

Run the tool's test suite with Python's standard-library `unittest` runner from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked ruff check tools/fingerprint-reports
uv run --project tools/fingerprint-reports --locked ruff format --check tools/fingerprint-reports
uv run --project tools/fingerprint-reports --locked python -m unittest discover \
  -s tools/fingerprint-reports/tests -v
```

These checks cover snapshot and report behavior. Descriptive sample calculations and static figures do not establish real-scan repeatability, uniqueness, false-acceptance rates or cryptographic security.
