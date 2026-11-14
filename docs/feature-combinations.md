# Regional feature combinations

`fingerprint::generate_combinations(&features, &config)` turns a compact `FingerprintFeatures` analysis into a deterministic list of point subsets. Obtain it with `fingerprint::analyze`, or project an existing full report with `Fingerprint::features()`. The [fingerprint analysis](fingerprint.md) does not need to create report-only maps for this operation. Each subset records original feature IDs, a ranking score and the regions that selected it.

## Regions and eligible points

Keep surviving feature identities with known final `priority >= min_priority`; equality is included. The examples use 0.5. Unknown priorities and weaker points never refill a region. Bound the confirmed pattern, extending it to include eligible point centers, and cover that rectangle with a regular grid of overlapping circles. A small capture can need one circle; nine is neither mandatory nor a maximum.

The initial radius is `radius_units × R`, where R is the measured mean dark-ridge width. If a circle contains fewer than `max_points` eligible points, expand directly to the required nearest point. If the entire capture has fewer than that maximum, use all available points. Fewer than `max(min_points, min_anchor_points)` is an explicit error. There is no expansion-step setting and no truncated point pool: every eligible point inside the effective circle participates in selection.

Both enrollment and signing admit subset sizes from **min_points through max_points**, capped by the available point count. The maximum limits a single combination, not the number of eligible regional points.

## Spatial policy

`SpatialPolicy` has two controls:

- **min_anchor_points:** at least this many anchor points must exist in the subset.
- **min_anchor_distance_units:** every pair of anchors must be separated by at least this distance, measured in R.

Every other point must lie inside or on the anchors' convex hull. Anchor count is a minimum: a polygon may require more anchors; two anchors enclose only their line segment. No extra containment margin is added. Every extreme hull vertex must be an anchor, and compatible interior points can complete the required count.

The examples start with three anchors separated by at least **6R**. Enrollment retains this policy; signing multiplies the same distance by the fresh capture's R. There is no fraction of image bounds, separate overall diameter, or second derived spatial-policy record. These settings govern geometry, not measured cryptographic strength.

Check source coordinates before rotation, snapping or hashing. An admitted subset retains its complete configured Cartesian uncertainty search; neighbor movements are not filtered again. Anchor roles do not add identities to the coordinate hash.

## Automatic weighted selection

Sort the N regional points by descending priority, using source ID to break ties. Partition valid subsets by their **strongest included point**: band 0 contains the strongest point, band 1 omits it and contains the next, and so on. Lower bands therefore provide alternatives without stronger points.

The first band receives up to N distinct subsets. Each subsequent band receives half the previous allowance, rounded up, with a minimum of one. Empty or exhausted bands contribute only their available valid subsets. Within a band, select in descending score:

```text
point weight = 0.1 + 0.9 × priority²
subset score = mean(point weights)
```

Ties use lexicographic point-rank order. The allowance is automatic, not a user-controlled per-region budget. It is a preference among bands, not a promise of exact per-point occurrence ratios: weaker points also occur alongside stronger points, and geometry restricts possible subsets.

After band selection, add valid coverage witnesses for any still-unrepresented regional points. If a point cannot participate in **any** admissible subset within the configured size range, fail the whole preparation with its region and source ID. Do not silently drop it. If the work budget prevents proving coverage or completing selection, report budget exhaustion instead of claiming geometric impossibility. Coverage witnesses can increase the output beyond the sum of band allowances.

Merge identical point sets from overlapping regions, retaining all contributing region IDs. IDs inside each entry are ascending; the global list is ordered by descending score, then IDs. Identical inputs and controls reproduce the same list. Signing searches all admissible regional subsets lazily in priority order, rather than applying enrollment's finite band selection to the new capture.

## Run the small example

```sh
cargo run -p gnorix --example feature_combinations --release --locked -- \
  crates/gnorix/tests/assets/test-fingerprint.png \
  reports/fingerprint/feature-combinations.json
```

Choose an output file that does not already exist. The adapter reads environment variables; the library receives an explicit `CombinationConfig` and does not read the environment. The canonical report folder can contain this JSON and an optional `feature-combinations.png` inspection figure. Refreshing the main fingerprint report replaces the folder and removes these previous-capture outputs; rerun this example for the new capture.

| Environment variable | Example default | Meaning |
|---|---:|---|
| `GNORIX_COMBINATION_MIN_POINTS` | 3 | Minimum subset size. |
| `GNORIX_COMBINATION_MAX_POINTS` | 6 | Maximum subset size and sparse-circle expansion target. |
| `GNORIX_COMBINATION_MIN_PRIORITY` | 0.5 | Inclusive minimum final feature priority. |
| `GNORIX_COMBINATION_MIN_ANCHOR_POINTS` | 3 | Minimum number of pairwise-separated anchors. |
| `GNORIX_COMBINATION_MIN_ANCHOR_DISTANCE_UNITS` | 6 | Minimum distance between every anchor pair, in R. |
| `GNORIX_COMBINATION_RADIUS_UNITS` | 30 | Initial circle radius in R. |
| `GNORIX_COMBINATION_MAX_WORK` | 10000000 | Cumulative validation, selection and geometry work budget. |

The library receives an explicit `CombinationConfig`; these defaults belong to the examples. Counts must be positive with `min_points <= max_points` and an anchor count that fits the maximum. Priority is finite in `[0, 1]`; distance is finite and nonnegative; radius is finite and positive. Changing the retained selection policy requires a new enrollment.

The export uses `gnorix.feature-combinations.v6`. It records controls, the direct spatial policy, eligible points, actual circles with their `feature_ids`, unique combinations and occurrence diagnostics. Imported enrollment validates these records before hashing. Old combination exports and fingerprint profiles require regeneration.

## Scale and coverage

R is the thickness of a dark ridge, not the period from one ridge to the next. The same radius multiplier is used for enrollment and reconstruction. The algorithm does not assume a fixed “full-finger ridge count”: ridge dimensions vary by person and location, and plain versus rolled acquisition covers different skin areas. See the [NIST-hosted ridge-width discussion](https://www.nist.gov/system/files/documents/2016/12/19/p19_jain_chen_nistlevel3.pdf) and [FBI acquisition definitions](https://le.fbi.gov/science-and-lab/biometrics-and-fingerprints/biometrics/recording-legible-fingerprints).

Circle coverage ensures that this rectangle, including every eligible point, is covered. It does not ensure that every previously enrolled multi-point subset lies together inside one circle of a new crop. [Signing](fingerprint-signing.md) deliberately searches only its regional subsets; a spatially excluded combination is not recovered by a global fallback.

## Scope

This stage supplies point subsets for later encoding. [Grid figures](grid-shapes.md) apply hull-based orientation and measured-R snapping to those subsets. [Enrollment](grx-packages.md) encodes one baseline per subset independently of source IDs; [signing](fingerprint-signing.md) searches the local uncertainty variants on demand. Original IDs and region bounds belong to this capture. Repeated identical input and settings reproduce the output; partial new captures still require correspondence and evaluated biometric acceptance criteria.
