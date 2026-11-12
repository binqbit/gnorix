# Regional feature combinations

`fingerprint::generate_combinations(&fingerprint, &config)` turns assessed features from one capture into a deterministic list of point subsets. It follows the [fingerprint extraction](fingerprint.md) stage. Each subset records original feature IDs, a ranking score and the regions that selected it.

## Regions and point pools

1. Keep source feature identities whose assessed type and degree survive cleanup and reconstruction. Count repeated identities once. Unknown priorities are excluded; measured zero remains eligible. The previous 30% illustration cutoff does not apply.
2. Bound the confirmed three-ridge pattern mask with a rectangle. Place nine centers at the centers of its 3×3 cells, in row-major order, IDs 0–8. This is a frame for this capture, not anatomical segmentation or alignment between captures.
3. Start each circle with diameter `diameter_fraction × min(width, height)` of that rectangle. For half-width coverage, set `diameter_fraction` to 0.5.
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

Identical point sets from overlapping regions are merged into one global entry with all contributing region IDs. Point IDs inside each entry are ascending; the global list is ordered by descending score, then IDs. Different region memberships can change the global frequency order.

## Scope

This stage supplies point subsets for later encoding. Original IDs and region bounds belong to this capture. Repeated identical input and settings reproduce the output; handling translated, rotated or partial new captures still requires registration and stable descriptor encoding. Key derivation, quantization and biometric acceptance criteria are separate stages.
