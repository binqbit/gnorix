# Regional feature combinations

`fingerprint::generate_combinations(&fingerprint, &config)` turns assessed features from one capture into a deterministic list of point subsets. It follows the [fingerprint extraction](fingerprint.md) stage. Each subset records original feature IDs, a ranking score and the regions that selected it.

## Regions and point pools

1. Keep source feature identities whose assessed type and degree survive cleanup and reconstruction. Count repeated identities once. Unknown priorities are excluded; measured zero remains eligible. The previous 30% illustration cutoff does not apply.
2. Bound the confirmed three-ridge pattern mask with a rectangle. Place nine centers at the centers of its 3×3 cells, in row-major order, IDs 0–8. This is a frame for this capture, not anatomical segmentation or alignment between captures.
3. Start each circle with diameter `diameter_fraction × min(width, height)` of that rectangle. The initial example fraction is 0.5.
4. The target pool size is `min_points × pool_multiplier`. If necessary, enlarge the circle to the distance of the target-th nearest eligible point. Boundary points are included. From all points inside the final circle, take the highest priorities, breaking ties by original ID.
5. If the whole capture has fewer points than the target but at least `min_points`, use all available points. Fewer than `min_points` produces an explicit error.

## Bounded weighted selection

For a pool of n ranked points and minimum m, subsets may have any size from m through n:

```text
point weight = 0.1 + 0.9 × priority²
subset score = mean(point weights)
```

First include `n − m + 1` coverage subsets: the strongest `m − 1` points together with each remaining point in turn. Then fill the per-region budget with the highest-scoring remaining distinct subsets. Ties use lexicographic point-rank order. A heap explores at most two successors per visited subset, so a small output budget does not require enumerating the complete combinatorial space.

Every pool point appears at least once. Within one pool, a stronger-ranked point appears no less often than a weaker-ranked point; integer counts can be equal. Coverage comes from the mandatory subsets. The constant 0.1 shifts mean scores but does not change their order. Weights are ranking controls, not random sampling probabilities or prescribed occurrence percentages. A small budget need not include every allowed subset size.

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
| `GNORIX_COMBINATION_POOL_MULTIPLIER` | 2 | Target pool size as a multiple of m. |
| `GNORIX_COMBINATION_DIAMETER_FRACTION` | 0.5 | Initial diameter relative to the shorter bounds side. |
| `GNORIX_COMBINATION_PER_REGION` | 12 | Maximum selected distinct subsets per region. |
| `GNORIX_COMBINATION_MAX_WORK` | 10000000 | Cumulative mask, topology, point-selection and subset-index work entries. |

These small defaults belong to the example. The library requires positive explicit controls and sufficient subset budget to cover the configured pool. Exhausting the possible distinct subsets returns fewer than the budget. Exhausting the work allowance fails the operation instead of returning a partial successful result.

The JSON contains the effective configuration, bounds, eligible point IDs/coordinates/priorities, all nine pools and radii, per-point occurrence counts and the unified `combinations` list. Other source measurements remain available through the original `Fingerprint` and its feature IDs. The checked-in test image with the example settings gives **299 scored features, nine pools of six and 108 unique combinations**.

## Scope

This stage supplies point subsets for later encoding. [Grid figures and positional variants](grid-shapes.md) apply hull-based orientation, measured-R snapping and complete local uncertainty traversal to those subsets. [Candidate hashing and enrollment packages](grx-packages.md) then encode integer positions independently of source IDs and use the selected crypto provider. Original IDs and region bounds belong to this capture. Repeated identical input and settings reproduce the output; partial new captures still require correspondence and evaluated biometric acceptance criteria.
