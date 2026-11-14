# Grid figures and positional variants

`fingerprint::prepare_grid_baselines(points, combinations, R, &config)` prepares one normalized, snapped baseline per [selected point subset](feature-combinations.md). Enrollment uses those baselines; signing searches nearby positions. R is the same capture's measured mean dark-ridge width and the distance between adjacent triangular-grid nodes.

## One fixed frame per combination

1. Construct the convex hull, retaining interior points for later stages.
2. Rotate its longest edge horizontally above the figure, without reflection. Set Y to zero at that edge and X to zero at the rightmost point of the entire rotated figure. This virtual top-right corner need not be a feature.
3. Snap every point to its Euclidean-nearest node on the signed axial lattice `(q, r)`, with basis `(R, 0)` and `(R/2, √3 R/2)`. Exact distance ties choose the smaller `(q, r)` lexicographically.
4. Align the snapped top and right extrema, preserving the alternating half-step between rows through a `phase` bit.
5. Add L rows downward and L full horizontal steps leftward, where L is the uncertainty level. All later variants keep this padded frame: no individual recentering or rotation.

Output cells have nonnegative `(column, row)` indices. Columns grow **left**; rows grow **down**. Their positions are:

```text
leftward distance / R = column + ((row + phase) mod 2) / 2
downward distance / R = √3 × row / 2
```

Equal longest edges are compared by normalized, sorted grid geometry, independent of priority and source IDs. Arithmetic guards handle floating-point collinearity and equal lengths; they do not add biometric tolerance. Point and collinear inputs remain representable, and snapping may collapse a thin polygon into a line. Coincident features retain their multiplicity and explicit collision groups.

`GridBaselineConfig` controls `padding_steps`, `max_baselines` and `max_work`. `GridBaselines::candidates(...)` hashes one baseline per prepared subset. It does not preflight or enumerate a Cartesian uncertainty product.

## Complete positional traversal

Level L contains every node reachable in at most L steps through the six nearest neighbors: `1 + 3L(L + 1)` positions per point. Levels 0–3 therefore have 1, 7, 19 and 37 positions. These are graph neighborhoods, not Euclidean circular cutoffs.

For n points, the full product has `(1 + 3L(L + 1))^n` assignments. Every point can move independently, including the points that determined the frame. Offsets are center first, then rings ordered by `(q, r)`; the last source-ID-ordered feature varies fastest. Collisions, common translations and repeated unlabeled geometries remain present.

The generic signing search streams this product for its current subset and tests each candidate immediately, without requiring the full count to fit a counter. `prepare_baseline_signing_search` additionally skips assignments whose top/right boundaries cannot equal a canonical enrolled baseline. This saves hashing and key derivation without changing possible-match order; it is valid only for baseline-only enrollments. See [candidate search](fingerprint-signing.md#candidate-search).

For a complete diagnostic, `prepare_grid_shapes(...)` and `GridShapes::variants()` calculate a checked total and reject requests above `max_variants` before traversal. The fallible iterator streams output; preparation and iteration share `max_work`. Exhaustion is an error, never a successful partial export. These diagnostic limits do not determine enrollment size.

## Run on the existing combinations

After [exporting combinations](feature-combinations.md#run-the-small-example), inspect their snapped shapes with a small level-0 export:

```sh
GNORIX_GRID_UNCERTAINTY_STEPS=0 \
cargo run -p gnorix-core --example feature_grid --release --locked -- \
  reports/fingerprint/feature-combinations.json \
  reports/fingerprint/feature-grid.json
```

The output must not exist. The command reads measured R and the selected subsets from JSON; it does not repeat image analysis. Omit the environment override to include the default level-1 neighborhoods. This diagnostic command does not change an enrolled package's policy.

| Environment variable | Example default | Meaning |
|---|---:|---|
| `GNORIX_GRID_UNCERTAINTY_STEPS` | 1 | Neighbor steps L. |
| `GNORIX_GRID_MAX_VARIANTS` | 1000000 | Complete assignment limit across all subsets. |
| `GNORIX_GRID_MAX_WORK` | 10000000 | Preparation and traversal work. |

The JSON includes hulls, frames, signed snapped nodes, padded baselines, phase, variants and collisions. Input and output are bounded to 64 MiB; a failed write removes its new partial file. Regenerate these results when the source capture changes; see the report tool's [refresh boundary and current schema limitation](../tools/fingerprint-reports/README.md#refresh-and-recovery).

## Interpretation

This normalizes individual subsets, not entire captures. Near-equal longest edges can switch under measurement noise; neighbor movements do not cover arbitrary frame changes. Changes in measured R, boundary rounding and symmetric figures remain sources of variability. [Coordinate encoding](grx-packages.md#coordinates-to-a-candidate) deliberately ignores source IDs and feature kinds, so successful normalization alone does not establish biometric identity.
