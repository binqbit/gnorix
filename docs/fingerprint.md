# Fingerprint extraction

`gnorix::biometrics::fingerprint` processes one grayscale capture into a ridge skeleton, an observed graph, reconstructed smooth contours and geometric measurements. It is implemented in native Rust. It does not call Python, SourceAFIS, a scanner SDK or a network service. Its output is sensitive biometric information intended for local inspection and later measurement-engine development.

## Run on an image

From the repository root:

```sh
cargo run -p gnorix --release --example fingerprint --locked -- \
  crates/gnorix/tests/assets/test-fingerprint.png reports/fingerprint
uv run --project tools/fingerprint-reports --locked gnorix-report structure \
  reports/fingerprint/features.json --output reports/fingerprint/visuals
```

The Rust example requires an opaque PNG and a new output directory. It estimates ridge thickness and period from the image before scale-dependent extraction; there is no manual period argument or fallback to a sample-specific value. Transparent input is rejected instead of interpreting hidden RGB as scanner evidence; applications with missing pixels should supply a `Capture` validity mask. The PNG adapter and JSON export use development-only `image` and `serde_json` dependencies; the library itself accepts an existing `Capture` and adds no runtime dependencies.

The bundle contains the source grayscale image, local dark/light references, raw measurement skeleton and confirmed-pattern support, final image stages and `features.json`. The report command produces five PNG figures: an overview, a structural overlay, primary `scale-analysis.png`, provisional `bootstrap-analysis.png` and `normalization-analysis.png`. Overlay geometry is semitransparent and retains source pixel coordinates. The JSON retains every full-pass width observation, its represented length and original grayscale mean.

## Full-source measurement and common unit

`estimate_scale(&capture, &config)` performs a full raster-based measurement pass before final directional enhancement. Local source grayscale and the adaptive palette form a raw dark mask. That mask is thinned without the final denoising or directional ridge filter. Every unique undirected link of the resulting measurement skeleton is visited once, including links inside junction clusters and cycle-closing links. No final observation budget selects a subset of these links.

Each link represents its actual raster length: one source pixel for an orthogonal link, or `sqrt(2)` for a diagonal. A transverse profile of the original image measures only the dark span containing that link's midpoint. Its normal comes from the local axial orientation field, not the stair-step direction of a digital skeleton link. A nearby ridge never replaces a missing own-ridge span.

Three complete dark ridges establish a local pattern region. The region extends across the measured triple and half its measured period along the tangent on each side; overlapping regions form a union, clipped by the acquisition validity mask. Confirmation does not flood an entire connected component. Once its region is confirmed, a visible own-ridge width can contribute even when that individual cross-section has fewer than three complete neighboring ridges. Both skeleton-link endpoint pixels must be inside confirmed support. Raw width sampling independently requires observed source pixels.

The common unit is the arithmetic mean over measurable ridge length:

```text
unit_px = sum(own_width_px × represented_length_px) / sum(represented_length_px)
length_in_units = source_length_px / unit_px
```

This is equivalent to equal weighting of equal-length portions along all measured lines. Long lines contribute in proportion to their length; graph-edge or connected-component counts do not determine their weight. Neighboring ridges confirm context without adding repeated width votes. No median, dominant-mode selection or post-mean width rejection changes this unit.

`FullScale.coverage` reports all traversed link length/count and the measured subset. Unreliable normals, missing width boundaries, unsupported source pixels and unconfirmed regions remain in the unmeasured length. Isolated skeleton pixels have no defined arc length and are counted separately. Complete traversal is not a claim that every physical ridge in an imperfect capture is observable. No measurable length produces an explicit failure rather than substituting the provisional scale.

`Fingerprint.scale.unit_px` is the final common unit. Curve sampling uses one quarter of this unit and smoothing sigma one half. Source coordinates remain in pixels for image overlays; exported geometric lengths also include `arc_length_units` and `chord_length_units`. Pattern period remains a separately measured dimension; the implementation does not assume `period = 2 × dark width`.

After the unit is known, `scale.width_consistency()` counts all full-pass own widths within 10%, 20% and 30% of it, with inclusive endpoints and only a floating-point arithmetic allowance. Fractions count observations; the separate coverage fields measure source length. These are descriptive consistency diagnostics, not recognition probabilities or a calibrated capture-quality score.

## Provisional bootstrap diagnostics

The bootstrap works directly from grayscale and the validity mask. A deterministic initial grid searches for transverse profiles with three complete dark ridges and intervening light valleys. If this grid misses every valid patch, an exhaustive source-site fallback searches for initial support; a small pattern between grid nodes is not rejected solely because the grid missed it. Its provisional period initializes raw normalization and orientation windows. Bootstrap observations never contribute directly to the final common-unit mean.

On the original image, refinement uses a short tangent projection and one-dimensional Gaussian smoothing with sigma equal to one thirty-second of the coarse period. Each dark ridge is bounded by subpixel crossings at half of its local contrast against the adjacent light maxima. This makes the operational definition of width explicit for blurred grayscale boundaries. It measures the sampled image; very narrow subpixel ridges do not reveal their exact latent physical width. A triple yields three ridge widths, two valley widths and two distances between ridge centers. Clipped ridges and invalid mask crossings cannot supply a complete width.

`Fingerprint.bootstrap` retains independent medians of its ridge widths, valley widths and center-to-center periods, plus unscaled median absolute deviations. These remain provisional diagnostics for comparing the initialization with the full measurement. They do not become the final unit.

Arithmetic mean ridge and valley widths are reported alongside these medians. Each width contributes once to its corresponding arithmetic mean. Each triple also retains three mean dark-span intensities and two mean light-gap intensities from the original capture, on the scale `0 = black`, `255 = white`. These are grayscale brightness measurements, not physical color temperatures. The intensity measurement integrates the bilinear source image along each complete span, including its half-contrast boundaries. Splitting at pixel-cell boundaries and applying Simpson integration within each cell makes the integral exact for that interpolant. It does not use normalized, denoised or binary image values.

Global dark and light brightness means weight each span mean by that span's source-pixel length. This gives the mean over the observed cross-sections, rather than giving a short and a long span equal brightness weight. Only dark spans and the two intervening light gaps of confirmed triples contribute; exterior blank background is excluded. Revisited physical ridges remain repeated observations, so these are profile statistics rather than averages over unique fingerprint pixels.

The bootstrap report labels its own width-consistency fractions around its provisional median. Its profile intersections are repeated source observations, not unique biological ridge identities. The primary scale report instead uses the complete raw measurement pass and the final arithmetic mean.

## Local dark and light levels

Normalization uses overlapping windows centered at each source pixel, with radius `ceil(provisional period / 2)`. Period selects image-filter support; R remains the common geometric measurement unit. The image is not partitioned into independent rectangular tiles. Each location has its own dark and light reference, allowing the references to follow gradual illumination changes.

The first pass calculates masked local mean and variance. A pixel with sufficient contrast and a complete validity-mask window supplies a dark seed when it is below its own local mean, or a light seed when it is above it; equality supplies neither class. The second pass averages original grayscale values of each seed class in each window. These two local class means define the dark reference `D` and the light reference `L`. A supported center requires both classes and `D < L`. Missing references remain unsupported rather than borrowing a distant zone's palette.

The detection intensity is `(I - (D + L) / 2) / ((L - D) / 2)`, clipped to `[-3, 3]`. Thus the measured local dark reference maps to `-1`, the light reference to `+1`, and their midpoint to zero. The same absolute grayscale value can be dark in one zone and light in another. This corrects local brightness and contrast without changing the source-coordinate system. Orientation tensors continue to use the first-pass standardized intensity; spatial denoising and ridge detection use the palette-normalized intensity.

`Fingerprint.palette` retains floating-point reference values in `0..255` and an explicit support mask. Exported `ridge-level.png` and `valley-level.png` round the references to 8-bit grayscale; `normalization-support.png` is `255` for supported pixels and `0` elsewhere. Unsupported reference slots are stored as zero and must be interpreted with that mask. The report masks them visually. Palette support indicates sufficient local normalization evidence, not fingerprint segmentation or anatomical certainty.

## Processing stages

1. Validate controls and source budget; obtain a provisional period from raw profiles, with exhaustive fallback if the initial grid misses all support.
2. Build the raw adaptive measurement mask, traverse its entire skeleton, confirm local regions and compute the length-weighted arithmetic width unit and its coverage.
3. Estimate local mean and variance using masked summed-area statistics. Form local dark/light classes, measure their original brightness means, and normalize around their midpoint and contrast.
4. Estimate ridge orientation and coherence from locally averaged first-pass standardized image-gradient tensors. Exclude unsupported and low-contrast regions.
5. Apply a spatial Gaussian to normalized intensity with sigma equal to one twentieth of the measured ridge period. Exclude unusable samples and renormalize supported weights. This suppresses fine grain before thresholding; it does not change the contrast, orientation or coherence assessments or introduce new usable pixels.
6. Densely average along the local ridge direction with quadrature spacing at most one source pixel and compare the center with two parallel lines offset across the ridge. This is an explicit three-line contrast filter, not a Gabor implementation or an exact reproduction of SourceAFIS.
7. Thin the binary ridges using Zhang–Suen iterations with a safeguard against deleting an entire component. Transfer clipped-boundary provenance to its nearest retained support rather than spreading it along a connected ridge.
8. Trace the skeleton deterministically. Preserve disconnected components, merge acyclic adjacent junction pixels, suppress redundant diagonal/cell links, and separately trace isolated degree-two cycles. A filled 2x2 pixel cell is not a hole; junction clusters containing genuine holes retain their individual vertices.
9. Measure each path's length, endpoint distance and half-length point. Resample by arc length, apply Gaussian smoothing and calculate an unwrapped tangent-turning profile.
10. Construct constrained smooth paths, evaluate short endpoint continuations against the original image, merge accepted joins and measure the resulting contours. Preserve the observed graph and rejected proposals separately.

Local normalization and orientation estimation follow established fingerprint-enhancement ideas, including [Hong, Wan and Jain](https://doi.org/10.1109/34.709565). The directional filter is our own implementation. Thinning derives from [Zhang and Suen](https://doi.org/10.1145/357994.358023), with the documented component-preservation and provenance additions. These references do not establish the biometric accuracy of this pipeline.

Enhancement and thresholding can alter connectivity, so the extracted graph is evidence from the processed image, not verified anatomy. The separate contour stage described below repairs supported short gaps. There is no pore-removal or small-feature deletion stage. Invalid acquisition-mask regions are not bridged or filled with inferred pixels.

## Smooth contours and short continuations

`Fingerprint.graph` retains the traced pixel paths. `Fingerprint.contours.graph` contains the reconstructed geometry. Full-source width measurement and its common unit R happen before reconstruction, so inferred bridge lengths do not enter the measured ridge-width mean.

Smoothing samples each observed path uniformly along its arc, with spacing at most `min(R/8, 1 px)` and Gaussian sigma `R/2`. The target is interpolated back at the original vertices' arc positions. Open endpoint ports stay fixed; closed paths use periodic support, and a loop attached at a junction keeps its attachment port. A blend with the original path limits movement to `R/4`. The complete swept corridor must remain inside usable dark binary support, and checks reject new crossings, overlaps and jumps across another contour. Closed paths retain winding and at least half their original enclosed area. When a candidate violates these constraints, the blend is reduced; if no supported candidate is found, the original path and a retention reason remain in the result. These constraints preserve small loops rather than treating them as disposable noise.

Gap reconstruction considers two ending nodes with enough incident length to estimate stable directions over R and R/2. The gap must be at most R and both ends must face the continuation within 30 degrees. A cubic bridge follows the local endpoint tangents, with forward motion and curvature bounded by `1/R`. Existing paths and isolated nodes obstruct a bridge; unrelated paths must stay at least `R/4` away. Only mutually unique source-supported partners can merge, and conflicting proposed bridges are both rejected. This is an explicit processing policy, not a learned or calibrated biometric tolerance.

Source support is measured from the original grayscale and floating-point local references. Darkness is `(L - I)/(L - D)`, where D and L are the local dark and light levels. Bridge quadrature uses steps no longer than half a pixel. Acceptance requires length-weighted mean darkness at least 0.5, at least 70% dark length, mean darkness advantage over flanks at least 0.25, and no sampled light run longer than `R/4`. Brightness, dark fraction and light-run lengths use midpoint quadrature; exact cell traversal guarantees source validity rather than exact brightness-threshold crossings. Flanks lie `3R/4` to each side. Every source interpolation cell must have valid capture pixels and local palette support. These values describe image evidence; darkness can lie outside `[0, 1]` and is not a confidence probability.

The observed node's `touches_unobserved` flag also includes regions excluded by enhancement. It therefore does not alone prohibit a source-supported repair. Actual acquisition validity and local source support govern the bridge. A geometry-only continuation with insufficient source evidence stays a proposal; it does not remove the two ending nodes or change connectivity. Proposals explicitly distinguish missing source support, insufficient dark evidence, ambiguity and obstruction.

Accepted joins concatenate paths and remove only their consumed ending nodes. They can also close a disconnected path into a cycle. Every final path records its original edge/cycle IDs and accepted join IDs; every retained node maps to its original node. Each original path occurs exactly once, including paths that could not be smoothed. Junctions, isolated nodes and existing isolated cycles are retained. `edge_smoothing` and `cycle_smoothing` report actual maximum displacement, applied blend and any retention reason, indexed by the observed graph. The complete smoothing controls and gap policy are exported with the result.

Reconstructed lengths and midpoints are measured on the resulting polylines; their turning profiles are sampled without applying a second Gaussian. The original graph descriptors retain their previous meaning. A smooth continuation remains an image-based inference rather than proof that an anatomical ridge was continuous.

## Controls and bounds

`ExtractionConfig::default()` sets local standard deviation `0.035` on intensity divided by 255, tensor coherence `0.2`, a source budget of four million pixels and 4096 initial bootstrap-grid locations. The grid budget does not cap the fallback or the full measurement pass. Callers can explicitly change these controls. Invalid values fail before processing. Source periods from 4 to 128 pixels are supported; this is computational support, not an inferred sensor resolution. Resource-limit failures do not return a truncated mean.

Curve sampling and smoothing are processing settings, not biometric tolerance ranges. `describe_curve` also accepts its own positive finite spacing and sigma. Sampling, Gaussian support and arithmetic have explicit work/numerical limits.

## Output meaning

- **Ending:** one connected path in the extracted graph. Missing signal can also create an ending.
- **Junction:** an acyclic cluster of adjacent branch pixels, or an individual vertex where consolidation would destroy a genuine cycle. The representative position is an actual pixel; edge ports can be different pixels within that cluster. No line is invented between a port and the representative.
- **Isolated node:** a retained single-pixel component. It is not automatically a meaningful biometric feature.
- **Cycle:** a closed path without graph junctions. A self-edge attached to a junction is retained as an edge; node identity alone does not force its two distinct ports into a geometrically closed curve.
- **Unknown continuation:** frame/invalid-region contact, including contact inherited through thinning. This is conservative provenance, not a confidence score. Absence of the flag does not establish a true anatomical ending.

For the observed graph, lengths and midpoints are measured on unsmoothed source-coordinate paths and include rasterization error. Reconstructed contour descriptors instead measure their final geometric paths. A curve's midpoint is halfway along its length, not the center of a fitted circle. Turning is relative to the initial tangent and follows the stored traversal; rotation does not change that relative profile, but reversing a path or moving a closed seam changes its parameterization. The observed turning descriptor uses a Gaussian truncated at four sigma with renormalized open ends or periodic closed boundaries. Contour reconstruction uses the endpoint-preserving reflected extension described above.

A very short or numerically unresolved curve can fail geometric description. Its path remains in the graph with an explicit per-curve error; it is not silently removed. A closed shape is not forced to have a valid circular center. No fitted-circle stability or repeat-capture tolerance is implied.

The report distinguishes isolated cycles from total graph cycle rank (`edges - nodes + components + isolated cycles`). A graph with no isolated cycles can still contain loops attached to junctions. Neither count is a claim about anatomical pores or verified fingerprint patterns.

## Verification and integration boundary

Tests cover image and mask validation, blank captures, oriented stripes, local brightness changes, source-intensity integration, independence from masked pixel values, component and hole preservation, missing regions, diagonal topology, junction clusters, closed cycles, long iterative paths, geometric invariances, numerical limits and the checked-in fingerprint. Contour tests additionally cover endpoint preservation, small-loop retention, swept support, collision rejection, supported processing gaps, identical geometry with a white source gap, missing acquisition pixels, ambiguous partners and complete path provenance. The report tests cover summary reconstruction, malformed geometry, palette support, path containment, image dimensions, empty graphs and PNG output. The synthetic fixture verifies execution and visualization, not real-scanner recognition performance.

The extractor is a component for a future complete `MeasurementEngine`. Registration, cross-capture correspondence, feature selection, calibrated tolerances, quantization and candidate encoding are not selected here. It is deliberately not connected to signing as a placeholder measurement strategy. Those choices require repeat captures and evaluation before a biometric key can be reconstructed reliably.
