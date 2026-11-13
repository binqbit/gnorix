# Regional feature combinations

`fingerprint::generate_combinations(&fingerprint, &config)` turns assessed features from one capture into a deterministic list of point subsets. It follows the [fingerprint extraction](fingerprint.md) stage. Each subset records original feature IDs, a ranking score and the regions that selected it.

## Regions and point pools

1. Keep source feature identities whose assessed type and degree survive cleanup and reconstruction and whose final `priority >= min_priority`. Count repeated identities once. Unknown priorities are excluded; equality is included. Apply this cutoff before building regions or selecting pools. The examples and test stand use 0.5. The stand displays these eligible points without another cutoff; the 30% documentation illustration remains a separate static example.
2. Bound the confirmed three-ridge pattern mask with a rectangle, extending it to include retained eligible point centers where needed. Cover it with a regular grid of overlapping circles; use the fewest cells whose half diagonal fits the configured radius. A small fragment needs one circle, while larger captures use more. Nine is neither mandatory nor a maximum. This is a frame for the visible capture, not an estimate of an anatomical full finger.
3. Start each circle with radius `radius_units × R`, where R is the measured mean dark-ridge width. The example uses 30R. Re-measure R on each capture; cropping the image does not directly shrink the configured radius.
4. The target pool size is `min_points × pool_multiplier`. If necessary, enlarge the circle to the distance of the target-th nearest eligible point. Boundary points are included. Start with the highest priorities, breaking ties by original ID. If that pool is too compact, choose the strongest ranked pool of the same size containing a pair that meets the span requirement below. No priority cutoff is relaxed.
5. If the whole capture has fewer eligible points than the target but at least `min_points`, use all eligible points. Fewer than `min_points` produces an explicit error. Never refill a pool with points below `min_priority`.

## Minimum spatial span

Each selected subset must contain **at least one pair** whose source-coordinate distance is at least D. Derive D once from the enrollment's confirmed-pattern pixel-edge rectangle, extended to include eligible point centers:

```text
D = min_span_fraction × min(bounds_width, bounds_height)
min_pair_span_units = D / enrollment_R
```

The examples and test stand fix `min_span_fraction = 1/3`. `FeatureCombinations::min_pair_span_units` retains the resulting dimensionless span. Later signing uses that same stored value times the fresh capture's R; it does not derive a smaller threshold from a partial crop. A region without a qualifying pair contributes no subsets; if no region qualifies, preparation reports insufficient span.

This is admission in the original source positions, before rotation, snapping or hashing. Once admitted, a subset still receives its complete configured Cartesian uncertainty search. Neighbor movements are not filtered again by span.

## Bounded weighted selection

For a pool of n ranked points and minimum m, subsets may have any size from m through n:

```text
point weight = 0.1 + 0.9 × priority²
subset score = mean(point weights)
```

Build coverage subsets from the strongest `m − 1` points together with each remaining point in turn. Replace compact seeds with the best minimal-size valid subset containing the required point, then cover any remaining uncovered pool point with another valid subset. Fill the remaining per-region budget with the highest-scoring distinct subsets that meet the same span requirement. Ties use lexicographic point-rank order. Invalid subsets consume search work, not output slots.

Every pool point appears in at least one valid subset. Spatial validity can require a weaker distant point more often than a stronger compact point, so occurrence counts need not follow priority order. If the requested budget cannot cover the pool with valid subsets, preparation fails explicitly. The constant 0.1 shifts mean scores but does not change their order. Weights are ranking controls, not random sampling probabilities or prescribed occurrence percentages. A small budget need not include every allowed subset size.

Identical point sets from overlapping regions are merged into one global entry with all contributing region IDs. Point IDs inside each entry are ascending; the global list is ordered by descending score, then IDs. Regional and global occurrence counts are exported separately: different region memberships can change the global frequency order.

## Run the small example

```sh
cargo run -p gnorix --example feature_combinations --release --locked -- \
  crates/gnorix/tests/assets/test-fingerprint.png \
  reports/fingerprint/feature-combinations.json
```

Choose an output file that does not already exist. The adapter reads environment variables; the library receives an explicit `CombinationConfig` and does not read the environment. The canonical report folder can contain this JSON and an optional `feature-combinations.png` inspection figure. Refreshing the main fingerprint report replaces the folder and removes these previous-capture outputs; rerun this example for the new capture.

| Environment variable | Example default | Meaning |
|---|---:|---|
| `GNORIX_COMBINATION_MIN_POINTS` | 3 | Minimum subset size m. |
| `GNORIX_COMBINATION_MIN_PRIORITY` | 0.5 | Inclusive minimum final priority for every selected point. |
| `GNORIX_COMBINATION_POOL_MULTIPLIER` | 2 | Target pool size as a multiple of m. |
| `GNORIX_COMBINATION_RADIUS_UNITS` | 30 | Initial radius in measured dark-ridge widths R. |
| `GNORIX_COMBINATION_PER_REGION` | 12 | Maximum selected distinct subsets per region. |
| `GNORIX_COMBINATION_MAX_WORK` | 10000000 | Cumulative mask, topology, point-selection and subset-index work entries. |

These small defaults belong to the example. The library requires explicit finite `min_priority` and `min_span_fraction` in `[0, 1]`, positive size/work controls and sufficient subset budget to cover the configured pool. Zero span fraction disables the core span rule; the fingerprint examples require 1/3 and expose no environment override for it. Enrollment and signing use the same retained priority and span policy. Changing that policy requires a new enrollment. The weighting curve ranks only points that passed the cutoff; it cannot restore rejected points. Exhausting the possible valid subsets returns fewer than the budget. Exhausting the work allowance fails the operation instead of returning a partial successful result.

The `gnorix.feature-combinations.v4` JSON contains the effective configuration including `min_priority` and `min_span_fraction`, derived `min_pair_span_units`, bounds, eligible point IDs/coordinates/priorities, all adaptive pools and radii, per-point occurrence counts and the unified `combinations` list. Other source measurements remain available through the original `Fingerprint` and its feature IDs. The number of regions follows the observed extent and R; it is exported with their centers, initial/effective radii and selected pools.

At 30R, `min_priority = 0.5` and the one-third span rule, the checked-in fixture supplies **30 eligible features, four regions and 48 valid combinations**. Its enrolled span is **362 px**, or **21.520984 R**. Every selected subset contains a pair meeting that distance.

## Scale and coverage

R is the thickness of a dark ridge, not the period from one ridge to the next. The same radius multiplier is used for enrollment and reconstruction. The algorithm does not assume a fixed “full-finger ridge count”: ridge dimensions vary by person and location, and plain versus rolled acquisition covers different skin areas. See the [NIST-hosted ridge-width discussion](https://www.nist.gov/system/files/documents/2016/12/19/p19_jain_chen_nistlevel3.pdf) and [FBI acquisition definitions](https://le.fbi.gov/science-and-lab/biometrics-and-fingerprints/biometrics/recording-legible-fingerprints).

Circle coverage ensures that this rectangle, including every eligible point, is covered. It does not ensure that every previously enrolled multi-point subset lies together inside one circle of a new crop. [Signing](fingerprint-signing.md) deliberately searches only its regional subsets; a spatially excluded combination is not recovered by a global fallback.

## Scope

This stage supplies point subsets for later encoding. [Grid figures](grid-shapes.md) apply hull-based orientation and measured-R snapping to those subsets. [Enrollment](grx-packages.md) encodes one baseline per subset independently of source IDs; [signing](fingerprint-signing.md) searches the local uncertainty variants on demand. Original IDs and region bounds belong to this capture. Repeated identical input and settings reproduce the output; partial new captures still require correspondence and evaluated biometric acceptance criteria.
