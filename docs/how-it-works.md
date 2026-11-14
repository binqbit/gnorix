# How it works

This page follows the implemented fingerprint path from one image to a verified authorization. The generic core also accepts a caller-supplied `MeasurementEngine`; multi-capture registration and fusion are not implemented by the native fingerprint path.

## Enrollment

1. **Measure the image.** Estimate the mean dark-ridge thickness R from supported ridge segments, normalize local brightness, extract ridge contours and identify feature candidates. Image evidence, geometry and controlled threshold variations contribute to feature strength; nearby feature crowding reduces final priority. [Fingerprint extraction](fingerprint.md) defines these measurements.
2. **Select point combinations.** Apply the minimum final priority and configured point-count range. Circular regions use radius in R and expand automatically when necessary. Each combination must have enough pairwise-separated anchors, with all remaining points inside their convex hull. Quality-group weights and compact local groups determine the selected set. See [feature combinations](feature-combinations.md).
3. **Build canonical candidates.** Orient each combination using its convex hull's longest edge, align it to the top and right boundaries, and snap every point to the nearest triangular-grid node with spacing R. Realign the snapped bounds, preserve row phase and add the fixed uncertainty margin. Hash the sorted integer-coordinate sequence with its phase, retaining repeated coordinates. Feature IDs, scores and pixel-valued R are excluded from this encoding. See [grid shapes](grid-shapes.md) and [candidate encoding](grx-packages.md).
4. **Derive and commit keys.** The application's `CryptoProvider` derives each candidate key from the digest, enrollment context and any required secret. The examples use HKDF-SHA256 and Ed25519. The Merkle tree contains sorted, deduplicated, context-bound public-key hashes; its root also binds the distinct-leaf count. Enrollment derives keys only from baselines, without expanding their positional uncertainty or retaining their coordinates.
5. **Retain proof material and register the commitment.** The `.grx` package holds public reconstruction parameters, enrollment context and leaf hashes from which the in-memory tree and membership paths can be rebuilt. It contains no private keys, source images or feature coordinates. The application registers the commitment through an owner-authorized path. See [GRX packages](grx-packages.md).

The native path must finish constructing the selected enrollment set or fail. It does not return a silently truncated Merkle commitment after reaching a resource limit.

## Signing from a later capture

The application first obtains approval for the exact action: its operation, payload, nonce and expiry. Gnorix then processes the new image under the retained profile and applies the same eligibility and anchor constraints. Distances remain in R; a crop does not lower the stored minimum separation.

The search visits point combinations lazily in priority-weighted order. For each combination it checks the rounded baseline first, followed by the allowed neighboring-position assignments in the fixed grid frame. Every derived public key is checked for membership immediately. A match stops the search and signs the approved action; otherwise all positional variants of the current combination are exhausted before the next combination is visited.

A shared work budget bounds fingerprint subset enumeration and candidate checks. Budget exhaustion means **search incomplete**, which differs from exhausting the candidate stream without finding a member. Coverage, geometry, ranking changes and quantization can prevent reconstruction even when a crop contains visible features. The [signing guide](fingerprint-signing.md) describes these outcomes and reproducible examples.

The signer needs the retained package and any required secret. A fresh image alone is not a guaranteed backup of the previous root or its membership data. Changing the profile, secret, context or accepted key set requires a new enrollment rather than an invisible change to an existing one.

## Verification and execution

The authorization supplies the matching public key, Merkle proof and action signature. The verifier compares it with the application's independently trusted commitment and expected action, then checks enrollment status, context, revision, expiry, membership and signature validity.

The core returns verification evidence. It does not consume replay state or execute the action. The application must enforce permissions and reserve or consume the nonce together with its protected state transition.

In the [NEAR SDK](near-sdk.md), owner-controlled commitment registration and monotonic nonce state supply that lifecycle. The example wallet consumes a nonce when accepting an authorization, then makes an asynchronous token transfer. Acceptance is distinct from transfer completion; a failed transfer needs a new signature. The [NEAR demo](../examples/near/README.md) shows the entire flow in a private sandbox with a new random circular capture on each attempt.

## What this establishes

Membership proves that the supplied public key was enrolled; the signature proves that its private counterpart signed the bound action. Neither proves sensor provenance, liveness or reliable distinction between people. Feature priority is not a calibrated recognition probability, and the number of combinations is not an entropy estimate. See the [biometric security boundary](near-sdk.md#biometric-security-boundary).
