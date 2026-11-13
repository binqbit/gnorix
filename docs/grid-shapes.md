# Grid figures and positional variants

`fingerprint::prepare_grid_shapes(points, combinations, R, &config)` consumes the [selected point combinations](feature-combinations.md). R is the measured **mean dark-ridge width from the same capture**. It is the distance between neighboring triangular-grid nodes; there is no independent pixel-spacing setting.

## One fixed frame per combination

1. Construct the convex hull, like a band around the exterior points. Keep every interior point for the remaining stages.
2. Rotate the longest boundary edge horizontally above the figure, without reflection. Its two endpoints establish the top boundary. Set `Y = 0` there and `X = 0` at the rightmost point of the entire rotated figure. This virtual top-right corner need not be an actual feature.
3. Snap every point to its Euclidean-nearest triangular-grid node. Signed axial coordinates `(q, r)` remain allowed. The basis is `(R, 0)` and `(R/2, √3 R/2)`; exact floating-point distance ties choose the smaller `(q, r)` lexicographically.
4. Align the snapped figure's top and right extrema. Preserve the alternating half-step between rows through an explicit `phase` bit. This is a change of the grid's coordinate frame, not independent deformation of its points.
5. For uncertainty level L, add **L rows downward and L full horizontal steps leftward**. This fixed margin accommodates every allowed movement. Variants keep that frame: they are not individually rotated or recentered.

Output cells use nonnegative `(column, row)` indices: columns grow **left** from the right boundary and rows grow **down**. For a shape's `phase` (0 or 1):

```text
leftward distance / R = column + ((row + phase) mod 2) / 2
downward distance / R = √3 × row / 2
```

Thus the two Cartesian boundaries can both be zero before padding even when the top-right corner is between grid nodes. Subtracting the independent axial minima would not accomplish this.

Equal longest edges are compared by their normalized, sorted grid geometry, without using priority or capture-local IDs. Machine-rounding guards cover arithmetic collinearity and equal lengths; they are not extra biometric tolerances. A point or collinear subset remains a point/line rather than disappearing. Rounding can also turn a thin polygon into a line. Coincident features keep their separate IDs and appear in explicit collision groups.

## Complete positional traversal

Level L includes every node at most L steps away through the six nearest neighbors. Each point has `1 + 3L(L + 1)` choices: 1, 7, 19 and 37 at levels 0–3. These are hexagonal graph neighborhoods, not Euclidean circular cutoffs.

For n points the complete Cartesian product contains `(1 + 3L(L + 1))^n` assignments. Every point moves independently, including points that established the frame. Neighborhood order is center first, then successive rings ordered by `(q, r)`; the last source-ID-ordered feature varies fastest. Collisions, common translations and repeated unlabeled geometries remain in the traversal. No extra subset omission or geometry deduplication occurs here.

`GridShapes::variants()` is a fallible iterator; it does not hold the product in memory. A checked total count rejects a request exceeding `max_variants` before traversal. Preparation and iteration share a work allowance. Exhausting it yields an error and stops, rather than reporting an incomplete traversal as successful.

## Interpretation

This is per-subset geometric normalization, not registration of whole captures or a canonical key encoding. Near-equal longest edges can switch under measurement noise; local neighbor movements do not cover an arbitrary frame rotation. Symmetric or equally quantized figures can have ambiguous source-ID correspondences despite identical unlabeled geometry. Boundary rounding and changes in measured R remain sources of variability that require repeated-capture evaluation.
