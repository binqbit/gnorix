# Regional feature combinations

`fingerprint::generate_combinations(&features, &config)` turns a compact `FingerprintFeatures` analysis into a deterministic list of point subsets. Obtain it with `fingerprint::analyze`, or project an existing full report with `Fingerprint::features()`. The [fingerprint analysis](fingerprint.md) does not need to create report-only maps for this operation. Each subset records original feature IDs, a ranking score and the regions that selected it.

## Regions and point pools

1. Keep source feature identities whose assessed type and degree survive cleanup and reconstruction and whose final `priority >= min_priority`. Count repeated identities once. Unknown priorities are excluded; equality is included. Apply this cutoff before building regions or selecting pools. The examples and test stand use 0.5. The stand displays these eligible points without another cutoff; the 30% documentation illustration remains a separate static example.
2. Bound the confirmed three-ridge pattern mask with a rectangle, extending it to include retained eligible point centers where needed. Cover it with a regular grid of overlapping circles; use the fewest cells whose half diagonal fits the configured radius. A small fragment needs one circle, while larger captures use more. Nine is neither mandatory nor a maximum. This is a frame for the visible capture, not an estimate of an anatomical full finger.
3. Start each circle with radius `radius_units × R`, where R is the measured mean dark-ridge width. The example uses 30R. Re-measure R on each capture; cropping the image does not directly shrink the configured radius.
4. The target pool size is `min_points × pool_multiplier`. If necessary, enlarge the circle to the distance of the target-th nearest eligible point. Boundary points are included. Start with the highest priorities, breaking ties by original ID. Spatial admission can require more distant eligible points; the pool size and priority cutoff remain unchanged.
5. If the whole capture has fewer eligible points than the target but at least `min_points`, use all eligible points. Fewer than `min_points` produces an explicit error. Never refill a pool with points below `min_priority`.

## Spatial policy

Each selected subset must satisfy three independent controls:

- Its diameter (the largest distance between any two points) is at least **D**.
- It contains **at least k anchor points**, with every pair of anchors separated by at least **d**.
- Every other point lies inside or on the boundary of the anchors' convex hull.

Anchor count is a minimum: a figure may need more than k anchors. In particular, k = 2 still permits a polygon with three or more anchors; exactly two anchors enclose only their line segment. Collinear points use an inclusive segment. No extra containment margin is added.

Derive both distances once from the enrollment's confirmed-pattern pixel-edge rectangle, extended to include eligible point centers:

```text
S = min(bounds_width, bounds_height)
min_diameter_units = min_diameter_fraction × S / enrollment_R
min_anchor_separation_units = min_anchor_separation_fraction × S / enrollment_R
```

The examples and stand default to D = S/3, k = 3 and d = S/6. All three are configurable. `CombinationConfig::spatial` carries the declared `SpatialPolicy`; `FeatureCombinations::spatial` retains the resolved `SpatialRequirements`. Signing reuses these distances multiplied by the fresh capture's R, rather than weakening them to fit a crop. The declared policy and resolved values are authenticated inside the package.

These are source-coordinate admission rules, before rotation, snapping or hashing. Once admitted, a subset still receives its complete configured Cartesian uncertainty search. Neighbor movements are not filtered again. Anchor roles constrain admission only; they do not change coordinate encoding or add anchor IDs to the hash.

An exact check can use a useful property: every extreme vertex of the subset's hull must be an anchor. These vertices must satisfy pairwise separation; if there are fewer than k, compatible interior points must complete the anchor set. A short outside protrusion can therefore invalidate a subset even when its larger neighboring triangle is admissible.

## Bounded weighted selection

For a pool of n ranked points and minimum m, subsets may have any size from m through n:

```text
point weight = 0.1 + 0.9 × priority²
subset score = mean(point weights)
```

Traverse each selected pool once in quality order, retaining coverage witnesses and at most the per-region budget of best valid subsets. The final selection contains those witnesses plus the highest-scoring distinct admissible subsets that fit the remaining slots. Ties use lexicographic point-rank order. Invalid subsets consume search work, not output slots. Testing the full pool alone is insufficient: a pool can contain several valid subsets even when its complete hull fails separation.

Every selected pool point must appear in at least one valid subset. If any selected point cannot participate, or the requested budget cannot cover the pool, the entire enrollment preparation fails explicitly; it does not silently omit that point or return a partial enrollment. Spatial validity can require a weaker distant point more often than a stronger compact point, so occurrence counts need not follow priority order. The constant 0.1 shifts mean scores but does not change their order. Weights are ranking controls, not random sampling probabilities or prescribed occurrence percentages. A small budget need not include every allowed subset size.

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
| `GNORIX_COMBINATION_MIN_DIAMETER_FRACTION` | 0.3333333333333333 | Minimum subset diameter as a fraction of the enrolled shorter side. |
| `GNORIX_COMBINATION_MIN_ANCHOR_POINTS` | 3 | Minimum number of pairwise-separated anchors. |
| `GNORIX_COMBINATION_MIN_ANCHOR_SEPARATION_FRACTION` | 0.16666666666666666 | Minimum distance between every anchor pair, as a fraction of the enrolled shorter side. |
| `GNORIX_COMBINATION_POOL_MULTIPLIER` | 2 | Target pool size as a multiple of m. |
| `GNORIX_COMBINATION_RADIUS_UNITS` | 30 | Initial radius in measured dark-ridge widths R. |
| `GNORIX_COMBINATION_PER_REGION` | 12 | Maximum selected distinct subsets per region. |
| `GNORIX_COMBINATION_MAX_WORK` | 10000000 | Cumulative feature validation, point-selection, spatial-check and subset-index work entries. |

These small defaults belong to the example. The library requires finite priority/fractions in `[0, 1]`, positive counts/work controls, an anchor count that fits the allowed pool, and sufficient subset budget to cover it. Enrollment and signing use the same retained priority and spatial policy. Changing that policy requires a new enrollment. The weighting curve cannot restore rejected points. Exhausting the possible valid subsets returns fewer than the budget. Exhausting work fails the operation instead of returning a partial successful result. The work count tracks actual processing; avoiding repeated traversal can let the same limit complete more selections without changing their ranking or spatial rules.

The `gnorix.feature-combinations.v5` JSON contains the configuration and declared spatial policy, resolved spatial requirements, bounds, eligible point IDs/coordinates/priorities, all adaptive pools and radii, per-point occurrence counts and the unified `combinations` list. Other source measurements remain available through the original `Fingerprint` and its feature IDs. The number of regions follows the observed extent and R.

The checked fixture with these initial settings supplies **30 eligible points, four regions and 48 combinations**: 25 triples, 16 quadruples, five quintuples and two six-point sets. D = **362 px = 21.520984 R** and d = **181 px = 10.760492 R**. Independent enumeration of possible anchor subsets confirmed every exported combination and coverage of every selected pool point.

Spatial settings control the admitted geometry and the amount of search. They are not a measured key-entropy estimate or a cryptographic strength in bits. Increasing a restriction can reduce the accepted set and partial-capture coverage; stronger geometry does not by itself establish stronger cryptography.

## Scale and coverage

R is the thickness of a dark ridge, not the period from one ridge to the next. The same radius multiplier is used for enrollment and reconstruction. The algorithm does not assume a fixed “full-finger ridge count”: ridge dimensions vary by person and location, and plain versus rolled acquisition covers different skin areas. See the [NIST-hosted ridge-width discussion](https://www.nist.gov/system/files/documents/2016/12/19/p19_jain_chen_nistlevel3.pdf) and [FBI acquisition definitions](https://le.fbi.gov/science-and-lab/biometrics-and-fingerprints/biometrics/recording-legible-fingerprints).

Circle coverage ensures that this rectangle, including every eligible point, is covered. It does not ensure that every previously enrolled multi-point subset lies together inside one circle of a new crop. [Signing](fingerprint-signing.md) deliberately searches only its regional subsets; a spatially excluded combination is not recovered by a global fallback.

## Scope

This stage supplies point subsets for later encoding. [Grid figures](grid-shapes.md) apply hull-based orientation and measured-R snapping to those subsets. [Enrollment](grx-packages.md) encodes one baseline per subset independently of source IDs; [signing](fingerprint-signing.md) searches the local uncertainty variants on demand. Original IDs and region bounds belong to this capture. Repeated identical input and settings reproduce the output; partial new captures still require correspondence and evaluated biometric acceptance criteria.
