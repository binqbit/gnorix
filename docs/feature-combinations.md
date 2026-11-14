# Regional feature combinations

`fingerprint::generate_combinations(&features, &config)` selects deterministic point subsets from `FingerprintFeatures`. Obtain them with `fingerprint::analyze`, or project a full analysis with `Fingerprint::features()`. Each subset retains capture-local feature IDs, a score and the regions that selected it. It is input to [grid normalization](grid-shapes.md), not yet a key.

## Regions and eligible points

Only surviving features with a known final `priority >= min_priority` participate. Equality is included; unknown scores and weaker points never refill a region. Priority is an algorithmic ranking, not a measured probability of correct recognition.

R is the capture's measured **mean dark-ridge width**. The initial radius is `radius_units × R`. A rectangular grid of overlapping circles covers the confirmed-pattern bounds, extended to include eligible point centers. The layout minimizes the required cell count; it does not assume nine regions or a fixed full-finger size.

If a circle contains fewer than `max_points` eligible points, it expands directly to the nearest point needed to reach that count. If fewer exist in the entire capture, it uses all available points. Every eligible point inside the effective circle participates; `max_points` limits subset size, not the regional point pool. Fewer than `max(min_points, min_anchor_points)` eligible points is an explicit error.

Subset sizes run from `min_points` through `min(max_points, regional_point_count)`, also satisfying the minimum anchor count. Enrollment and signing use the same retained size range.

## Spatial policy

Each subset must admit at least `min_anchor_points` anchors, with **every anchor pair** separated by at least `min_anchor_distance_units × R`. All remaining points must lie inside or on the anchors' convex hull. More anchors are allowed; the setting is a minimum, not an exact hull-vertex count. Every extreme hull vertex is therefore mandatory, and compatible interior points can complete the required count.

Admission uses observed coordinates before rotation or snapping. Signing applies the saved distance in R to the fresh capture's measured R. Neighbor movements are not checked against the spatial policy again. Anchor roles do not enter the coordinate hash.

These are geometric constraints, not a cryptographic strength estimate. A valid local subset need not be one of the subsets selected during enrollment.

## Automatic weighted selection

Order regional points by descending priority, breaking ties by source ID. A valid subset belongs to the absolute priority band of its **strongest included point**:

| Band | Strongest priority | Relative weight |
|---|---|---:|
| H | `> 0.9` | 2 |
| M | `0.5..=0.9` | 1 |
| L | `< 0.5` | 1 |

Select distinct valid subsets in the cycle **H, M, H, L**. Empty or exhausted bands release their turns. Geometry rejections and duplicates do not consume a turn. Within each band, order subsets by mean `0.1 + 0.9 × priority²`; ties use lexicographic point-rank order. The cutoff still applies: with `min_priority = 0.5`, band L is empty.

For N regional points, enrollment targets **3N subsets**, or all admissible subsets if fewer exist. It then adds:

- Coverage witnesses for any regional point not represented yet. If a point cannot belong to any valid subset in the configured size range, the entire operation fails with its region and source ID.
- At most one compact valid group around each regional point, including that point.
- At most one compact valid group around the midpoint of each regional point pair, without requiring either endpoint. These centers cover partial scans whose useful group lies between features.

Both local searches prefer the smallest farthest-neighbor distance, then the smallest total distance, and expand until the same spatial constraints can be met. Duplicate groups merge. Pair centers add at most N(N−1)/2 witnesses per region, and their search shares the work budget; no unrestricted subset enumeration is performed.

These additions improve point coverage and partial-capture coverage; they can exceed 3N and change the overall band proportions. They do not guarantee a match for every crop. Selection and feasibility checks share the work budget; exhaustion is an incomplete computation, not proof that a point is geometrically unusable.

Identical point sets selected by overlapping regions merge into one entry retaining all contributing regions. The exported list uses descending score, then ascending feature IDs. The 2:1:1 weights describe subset allocation, not the frequency of individual points or key attempts.

[Signing](fingerprint-signing.md#candidate-search) uses the same band schedule, but traverses admissible regional subsets lazily without the 3N enrollment target. It completes the current subset's positional search before scheduling another subset.

## Run the small example

From the repository root:

```sh
cargo run -p gnorix-core --example feature_combinations --release --locked -- \
  crates/core/tests/assets/test-fingerprint.png \
  reports/fingerprint/feature-combinations.json
```

The output must not already exist. This adapter reads the environment; the library receives an explicit `CombinationConfig`.

| Environment variable | Example default | Meaning |
|---|---:|---|
| `GNORIX_COMBINATION_MIN_POINTS` | 3 | Minimum subset size. |
| `GNORIX_COMBINATION_MAX_POINTS` | 6 | Maximum subset size and sparse-circle expansion target. |
| `GNORIX_COMBINATION_MIN_PRIORITY` | 0.5 | Inclusive minimum final feature priority. |
| `GNORIX_COMBINATION_MIN_ANCHOR_POINTS` | 3 | Minimum pairwise-separated anchor count. |
| `GNORIX_COMBINATION_MIN_ANCHOR_DISTANCE_UNITS` | 6 | Minimum anchor-pair distance in R. |
| `GNORIX_COMBINATION_RADIUS_UNITS` | 30 | Initial circle radius in R. |
| `GNORIX_COMBINATION_MAX_WORK` | 10000000 | Cumulative validation, selection and geometry work. |

These are CLI example defaults, not library defaults; the [NEAR demo](../examples/near/README.md) has its own explicit profile. Counts must be positive and ordered, and the anchor count must fit the maximum. Priority must be finite in `[0, 1]`, distance finite and nonnegative, and radius finite and positive.

The `gnorix.feature-combinations.v7` export records controls, measured R, eligible points, actual circles, selected subsets and occurrence counts. [Enrollment](grx-packages.md) validates the imported geometry and retains the reconstruction policy. Regenerate this export when the capture changes. The report tool's [refresh boundary](../tools/fingerprint-reports/README.md#refresh-and-recovery) describes replacement behavior and its current v7 schema limitation.

Circle coverage covers the pattern rectangle; it cannot guarantee that every enrolled multi-point subset lies together inside a circle of a new crop. Signing deliberately has no global fallback. Source IDs are local to a capture; deterministic output for identical inputs does not establish correspondence across independent scans.
