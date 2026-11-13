# Regional feature combinations

`fingerprint::generate_combinations(&fingerprint, &config)` turns assessed features from one capture into a deterministic list of point subsets. It follows the [fingerprint extraction](fingerprint.md) stage. Each subset records original feature IDs, a ranking score and the regions that selected it.

## Regions and point pools

1. Keep source feature identities whose assessed type and degree survive cleanup and reconstruction. Count repeated identities once. Unknown priorities are excluded; measured zero remains eligible. The previous 30% illustration cutoff does not apply.
2. Bound the confirmed three-ridge pattern mask with a rectangle, extending it to include retained eligible point centers where needed. Cover it with a regular grid of overlapping circles; use the fewest cells whose half diagonal fits the configured radius. A small fragment needs one circle, while larger captures use more. Nine is neither mandatory nor a maximum. This is a frame for the visible capture, not an estimate of an anatomical full finger.
3. Start each circle with radius `radius_units × R`, where R is the measured mean dark-ridge width. The example uses 30R. Re-measure R on each capture; cropping the image does not directly shrink the configured radius.
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
| `GNORIX_COMBINATION_RADIUS_UNITS` | 30 | Initial radius in measured dark-ridge widths R. |
| `GNORIX_COMBINATION_PER_REGION` | 12 | Maximum selected distinct subsets per region. |
| `GNORIX_COMBINATION_MAX_WORK` | 10000000 | Cumulative mask, topology, point-selection and subset-index work entries. |

These small defaults belong to the example. The library requires positive explicit controls and sufficient subset budget to cover the configured pool. Exhausting the possible distinct subsets returns fewer than the budget. Exhausting the work allowance fails the operation instead of returning a partial successful result.

The JSON contains the effective configuration, bounds, eligible point IDs/coordinates/priorities, all adaptive pools and radii, per-point occurrence counts and the unified `combinations` list. Other source measurements remain available through the original `Fingerprint` and its feature IDs. The number of regions follows the observed extent and R; it is exported with their centers, initial/effective radii and selected pools. At 30R the checked-in fixture yields **299 scored features, four pools of six and 48 unique combinations**. A freshly extracted 512×512 central crop needs one circle.

## Scale and coverage

R is the thickness of a dark ridge, not the period from one ridge to the next. The same radius multiplier is used for enrollment and reconstruction. The algorithm does not assume a fixed “full-finger ridge count”: ridge dimensions vary by person and location, and plain versus rolled acquisition covers different skin areas. See the [NIST-hosted ridge-width discussion](https://www.nist.gov/system/files/documents/2016/12/19/p19_jain_chen_nistlevel3.pdf) and [FBI acquisition definitions](https://le.fbi.gov/science-and-lab/biometrics-and-fingerprints/biometrics/recording-legible-fingerprints).

Circle coverage ensures that this rectangle, including every eligible point, is covered. It does not ensure that every previously enrolled multi-point subset lies together inside one circle of a new crop. [Signing](fingerprint-signing.md) deliberately searches only its regional subsets; a spatially excluded combination is not recovered by a global fallback.

## Scope

This stage supplies point subsets for later encoding. [Grid figures](grid-shapes.md) apply hull-based orientation and measured-R snapping to those subsets. [Enrollment](grx-packages.md) encodes one baseline per subset independently of source IDs; [signing](fingerprint-signing.md) searches the local uncertainty variants on demand. Original IDs and region bounds belong to this capture. Repeated identical input and settings reproduce the output; partial new captures still require correspondence and evaluated biometric acceptance criteria.
