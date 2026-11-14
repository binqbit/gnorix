# Fingerprint report

Generate the current native fingerprint analysis and its PNG report with one command from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/core/tests/assets/test-fingerprint.png
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
  clarity-analysis.png
  feature-strength.png
  data/
    features.json
    source.png
    ... native grayscale processing stages
```

| Figure | Content |
|---|---|
| `overview.png` | Source, normalization, denoising, coherence, usable region, binary ridges and skeleton, with measurement summaries. |
| `structure-overlay.png` | Cleaned, reconstructed paths with surviving feature markers and their original priority; changed feature types remain explicitly unassessed. |
| `scale-analysis.png` | Full-source length-weighted mean ridge width R, measured/unmeasured coverage and width consistency. |
| `bootstrap-analysis.png` | Initial ridge triples, provisional widths and period, and original grayscale measurements. |
| `normalization-analysis.png` | Local dark/light reference maps, supported regions and normalization behavior. |
| `contour-comparison.png` | Observed and reconstructed paths on identical source coordinates, marked terminal-spur and duplicate-loop-arm removals, and a matched detail. |
| `contour-overlay.png` | Cleaned, smooth contours and explicit accepted or unaccepted gap continuations. |
| `clarity-analysis.png` | Source-aligned local clarity, native contrast/separation/repetition components and explicit measurement support, on fixed 0–1 scales. |
| `feature-strength.png` | Every original candidate colored by final priority, with intrinsic Q, context at 2R/4R, neighborhood evidence and selected detail examples. |

The full-source width mean is the common unit R. Bootstrap measurements initialize the pipeline and remain separate diagnostics. Width consistency at ±10%, ±20% and ±30% describes measured variation; it is not a recognition probability. The scale overlay displays a spatial subset for readability, while every measurement contributes to its statistics.

Observed and reconstructed geometry remain distinct. Cleanup uses a fixed ±20% tolerance around the global mean width R: 0.8R–1.2R. R remains the common unit. Every original junction-to-ending branch whose arc length is strictly less than the upper bound 1.2R is assessed, including branches below 0.8R; the lower reference is not an exclusion threshold. Removal requires the original image to confirm a termination with a complete light barrier ahead of the tip. A measured barrier alone is insufficient when connected dark source structure can continue around it. Processing-boundary flags alone do not decide whether a ridge continues. Missing source support, possible continuations and other unconfirmed cases are retained. Decisions use one pass over the original graph; an all-short component is retained rather than erased. A branch of length exactly 1.2R is retained.

A split/rejoin pair consists of exactly two observed arms between two degree-three junctions. It collapses only when the observed arms remain less than 1.2R apart and the original image supports one ridge: a dark interior, a first light boundary on each side within 1.2R, and no bright valley enclosed by dark masses inside the candidate arms for at least R/3 along the loop. The terminal light-barrier lookahead also uses 1.2R; other source controls remain derived from R. The source-preferred observed arm remains; the duplicate arm is removed explicitly. Cleanup does not invent a center path or erase unsupported loops. Green bridge segments are accepted source-supported joins; amber dashed segments remain disconnected. Reconstruction does not establish anatomical correctness.

The native export retains an explicit cleaned graph in `cleanup`, between the observed graph and final contours. `spur_assessments` and `loops` retain source decisions and measurements, including rejected candidates. Terminal retention lists distinguish missing support, unconfirmed evidence and the all-short component safeguard. Cleanup provenance accounts for every removed and retained source path and observed route through junction clusters. Connected components are preserved; each applied loop collapse removes exactly one independent cycle. Final smoothing records, join node IDs and path origins refer to the cleaned graph. The renderer validates both transitions before publishing; it does not classify source pixels or decide topology itself.

Vermillion dashed paths in the comparison show removed artifacts: terminal spurs or duplicate loop arms. The final overlay shows retained geometry. The comparison detail selects the widest source-confirmed collapsed loop, with removed edge ID breaking ties, and includes both observed arms. Otherwise it selects the longest removed spur, then the longest accepted gap join, then the path with the largest smoothing displacement. Both panels use identical source coordinates and scale. Labels show the mean cursor diameter R, its ±20% range and the strict 1.2R branch-length or loop-width threshold from the native policy.

Clarity comes from the native source-image analysis: grid-anchor scores are contrast × separation × repetition, then bilinearly interpolated. The three component maps interpolate independently; their dense pixelwise product is not the clarity score. It does not filter paths, change geometry or establish recognition probability. Every observed/refined path and proposed join carries native per-segment values in `segment_clarity`; open paths have one value per consecutive point pair, and isolated cycles additionally include the last-to-first pair. A cycle with a repeated closing point still carries that final zero-length segment's value. `null` means missing source-map support, not a measured zero.

Contour opacity is `0.4 + 0.5 × sqrt(clarity)`: a fixed **40–90%** range, independent of `--overlay-alpha`. This square-root display transfer is identical for every image; native scores and linear heatmap scales remain unchanged. High-clarity segments appear stronger, while low-clarity geometry remains inspectable. Missing clarity on raw/refined paths is gray at 40%. Joins retain their green/amber status colors, at 40% when clarity is unavailable. Faint dotted originals behind the final comparison detail use 55% of this opacity as a reference layer. The source image stays at full opacity. Dense score/component PNGs round native values to bytes; `clarity-support.png` distinguishes unavailable pixels from measured zero. The color scale is fixed across images. The native supported-pixel mean may include flat background and is not an overall fingerprint accuracy score.

`data/features.json` retains all measured widths, source-coordinate paths, local-reference metadata, reconstruction provenance and processing controls. The adjacent images are the extractor's original 8-bit grayscale PNG stages. The renderer checks dimensions, image containment, finite values, scale summaries, path origins, endpoint preservation and join evidence before publishing. Image processing is local; fingerprints and measurements are not uploaded.

## Feature strength and priority

The native assessment keeps every original ending, bifurcation, higher-degree junction, isolated node and isolated cycle. `feature-strength.png` includes zero scores, unknown evidence and candidates later removed by cleanup. The main `structure-overlay.png` follows final contours and both provenance transitions to show surviving features. Changed feature types/degrees remain gray and unassessed; their old scores are not reassigned to a new shape.

Extended source-confirmed context C is the main evidence. It follows required arms over up to 4R, with 2R context shown for comparison, and checks actual ridge/flank separation, continuity and feature-specific valleys or termination. A weak local contrast does not veto a consistently supported structure. Source-confirmed continuation can cross an incidental graph node; ambiguous or missing continuation stays explicit. These observations do not edit the extracted or reconstructed paths.

Ending details show the graded light-cap score, source-backed forward continuation within 3R, curved continuation, and their combined maximum. The cap uses complete R-wide, R/4-deep windows within the existing 1.2R lookahead. A weak or uneven cap reduces its contrast/noise score continuously; a single pixel does not veto it. Ending context cannot exceed either its cap score or one minus the combined continuation. Missing required evidence remains unknown. These ranking measurements do not relax the strict source checks used to remove or join geometry.

Local source P, local geometry G and eight-case extraction stability T remain separate:

```text
Q = C × (0.75 + 0.25 × min(P, G)) × (0.5 + 0.5 × T)
N = 0.5 + 0.5 × observed_fraction / (1 + weighted_count)
priority = Q × N
```

The neighborhood is a disk of radius 3R. Other original candidates contribute squared taper weights `(1 - distance/(3R))²` inside that radius. Source-mask and frame coverage limit isolation credit; N remains between 0.5 and 1. Crowding affects priority, not intrinsic strength Q, and cannot turn a lone artifact into strong evidence. Missing required local/context evidence yields unknown Q and priority. These are inspectable engineering scores, not calibrated recognition probabilities.

Both figures use distinct candidate shapes with a fixed 0–1 scale for final priority: red → orange → yellow → bright green, from weak to strong. The continuous palette increases in lightness and avoids the cyan path colors; markers and colorbars share the same scale across images. Opacity is `0.4 + 0.5 × sqrt(priority)`, within 40–90%, independent of `--overlay-alpha`. Gray at 40% means unavailable priority; measured zero is red. The structural overlay retains hollow markers with × for unknown continuation. Path visibility follows regional clarity, while the original image stays fully opaque.

The diagnostic figure shows C/P/G/T/Q/priority distributions, C2R/C4R, per-arm extent and source separation, continuity, stop reasons, graded ending components, neighborhood evidence and the final priority. Exact native measurements remain in `data/features.json`. Python validates the versioned contract, graph references, formulas and neighborhood counts against exported positions. It does not resample source pixels or compute feature strength.

## Display controls

- `--overlay-alpha 0.5`: opacity of maps, measurement overlays and removed-artifact annotations in `[0, 1]`. Clarity-colored lines and priority-colored feature markers use their fixed 40–90% range. Default: 0.5.
- `--dpi 160`: PNG export resolution from 50 to 600. Default: 160.

These options change presentation only. They do not change ridge extraction, matching tolerances or the source-coordinate system. The command generates PNG figures only.

## Refresh and recovery

Extraction and rendering finish in a temporary sibling directory before publication. The previous report is then moved aside, the completed report takes its place, and the previous generated tree is removed. A handled publication failure restores the previous report; the swap is not a guarantee of uninterrupted directory visibility to concurrent readers. If restoring the previous directory also fails, the command retains its backup and reports its location.

The command refuses symlink destinations and unrecognized files in the managed report tree. Keep personal files outside `reports/fingerprint/`. An exclusive `reports/.fingerprint.lock` prevents simultaneous writers. A process killed without cleanup can leave the lock; remove it only after confirming that no report generation is running. Successful runs leave only the canonical report directory and the maintained root `reports/README.md`.

Previously generated native bundles may omit newer diagnostic files and still be refreshed. The command recognizes their native schema, core files and allowed generated filenames; it never renders an old data format or removes unknown personal files.

## Implementation responsibilities

`cli.py` runs native extraction and manages the report lock, replacement and recovery.
`structure.py` orchestrates validation, staged figure generation and publication;
`snapshot.py` owns bounded JSON loading and the validated snapshot model. Scalar
JSON checks live in `validation.py`, while graph and provenance checks remain
with their corresponding measurement modules.

`structure_render.py`, `contour_render.py` and `feature_strength_render.py` draw
already validated observations, reconstructed contours and feature evidence.
They do not select biometric thresholds or change native measurements. Report
filenames are declared once in `structure.py`; native raster names follow the
snapshot image contract. Adding a figure must preserve the report sequence and
complete-output checks.

## Verification

```sh
uv run --project tools/fingerprint-reports --locked ruff check tools/fingerprint-reports
uv run --project tools/fingerprint-reports --locked ruff format --check tools/fingerprint-reports
uv run --project tools/fingerprint-reports --locked python -Werror -m unittest discover \
  -s tools/fingerprint-reports/tests -v
```

CI also executes the one-command report generation against the checked-in fingerprint. Tests exercise native measurement validation, geometry and provenance, image decoding, complete report publication and failure recovery. They do not establish real-scanner recognition accuracy.
