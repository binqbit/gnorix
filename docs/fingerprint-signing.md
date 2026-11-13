# Signing from a fresh fingerprint image

`fingerprint::analyze(&capture, &extraction_config)` returns compact `FingerprintFeatures`; `fingerprint::signing_candidates(&features, &config)` searches those observed features without generating report-only maps or curve descriptors. `authorization::sign_candidates(...)` derives each candidate key, looks it up in the retained [`.grx` package](grx-packages.md), and signs the exact approved action with the first member found. The enrollment and its accepted key set remain unchanged.

## Candidate search

1. Extract the new image and measure its own mean ridge width R. Keep feature identities whose type and degree survive cleanup and whose final `priority >= min_priority`, using the same cutoff as enrollment. Unknown priorities are excluded. Apply this filter before building regions; fewer than n eligible points is an explicit insufficient-data error.
2. Use the same adaptive circle geometry and initial `radius_units × R` as enrollment. If needed, expand a circle to include up to 2n eligible points. Search all eligible points inside each effective circle, without enrollment's top-pool truncation. Never refill from below the cutoff. Subsets contain `n..=min(2n, regional_points)` points and must satisfy the saved [spatial policy](feature-combinations.md#spatial-policy): minimum diameter, at least k pairwise-separated anchors, and containment of the remaining points in their hull. Multiply the retained distances in R by the fresh R; do not recompute them from the crop's smaller bounds. There is no global fallback.
3. Merge the regional subset streams lazily by descending mean weight, deduplicating identical source-point subsets from overlaps before hashing or counting key attempts, using `weight = 0.1 + 0.9 × priority²`. Ties follow point rank, with source IDs breaking equal-priority ties. Reject subsets that fail source-space spatial admission before grid preparation: these checks consume work, not key attempts. These are deterministic ranking weights, not random probabilities. A subset can omit the strongest point; limited searches may stop before reaching weaker subsets.
4. Apply the same hull orientation, nearest-grid snapping, phase and fixed L-step padding used at enrollment. For the current admitted subset, try its rounded baseline first, then stream the full Cartesian product of the L-step neighborhoods of its points. Keep that frame fixed; do not rotate or recenter individual variants. Spatial admission is not checked again after snapping or neighbor movement.
5. For **each yielded assignment**, hash the canonical integer coordinates, derive the key under the retained context and secret policy, and immediately test its context-bound public-key hash for membership. Stop at the first match and return its public key, Merkle proof and signature. The highest-priority subset and all its variants are searched before any lower-priority subset; the schedule is deterministic.

Both the library iterator and signing pipeline have explicit bounds. Natural exhaustion without a member returns `CandidateNotEnrolled` for the configured regional search space. This does not rule out an enrolled combination spanning different regions. An attempt limit, work limit or algorithm failure means the search is incomplete; it must not be reported as a complete no-match result. No such outcome changes the package.

Each region contributes `Σ C(N, k)` subsets for `k = n..min(2n, N)` before overlap deduplication. Each k-point subset has up to `(1 + 3L(L + 1))^k` positional assignments. The search neither collects them in advance nor requires their full product to fit a count limit. It holds the current traversal state and produces the next assignment on demand. A small search space is exhausted completely if both limits permit it; larger searches stop at the first member or explicitly reach a limit.

This flow uses combination JSON v5, example profile v6 and `.grx` container v2. Enrollment commits only baseline keys, while signing searches positional uncertainty; both enforce the retained priority and spatial policy. Earlier fingerprint profiles must be recreated. The current CLI rejects old container files. Existing authorizations remain checkable through the generic verifier with their retained original trusted commitment. Coordinate hashing, context encoding and signature framing are unchanged.

## Run the example

First create the package using the [enrollment example](grx-packages.md#run-the-complete-example). Use the package path printed by enrollment; signing loads its reconstruction policy automatically:

```sh
grx_package=reports/enrollment/REPLACE_WITH_MERKLE_ROOT.grx

cargo run -p gnorix --example fingerprint_signing --release --locked -- \
  sign crates/gnorix/tests/assets/test-fingerprint.png "$grx_package" \
  crates/gnorix/examples/fingerprint_signing/action.json \
  reports/enrollment/signing/example-authorization.json

cargo run -p gnorix --example fingerprint_signing --release --locked -- \
  verify "$grx_package" \
  crates/gnorix/examples/fingerprint_signing/action.json \
  reports/enrollment/signing/example-authorization.json "$(date +%s)"
```

Replace the image path with a new capture or crop to run fresh extraction. The output must not exist. Signing verifies the result before writing, then reloads and verifies the saved JSON. The verifier needs no image, private key or secret.

The action JSON contains exactly `operation` (nonempty text), `payload` (lowercase hexadecimal bytes), `nonce` (32 bytes in lowercase hexadecimal) and `expires_at` (unsigned Unix seconds in this example). The included action is a local fixture with a fixed nonce; applications create their own approved actions and manage nonce reuse.

The package retains the point minimum, priority cutoff, pool multiplier, radius in R, per-region budget, declared spatial policy, resolved distances in R and uncertainty steps. Signing does not require these environment variables again. Explicit combination-policy overrides (including `GNORIX_COMBINATION_MIN_DIAMETER_FRACTION`, `GNORIX_COMBINATION_MIN_ANCHOR_POINTS` and `GNORIX_COMBINATION_MIN_ANCHOR_SEPARATION_FRACTION`) and `GNORIX_GRID_UNCERTAINTY_STEPS` must equal the saved values; mismatches are rejected. Changing policy requires a new enrollment. Required-secret enrollments also need the same `GNORIX_ENROLL_SECRET`. The shared example provider remains HKDF-SHA256/Ed25519; the library accepts the caller's `CryptoProvider`.

| Search control | Example default | Meaning |
|---|---:|---|
| `GNORIX_SIGNING_MAX_ATTEMPTS` | 1000000 | Maximum candidate keys examined. |
| `GNORIX_SIGNING_MAX_WORK` | 1000000000 | Cumulative point, subset-frontier, geometry and descriptor work. |

These local resource limits do not alter the enrolled profile or accepted set. The core may request one extra iterator item to detect attempt overflow; it does not derive a key from that item. The example also bounds input JSON to 256 KiB and framed actions to 64 KiB. Package reading uses `GNORIX_ENROLLMENT_MAX_CANDIDATES` as its leaf cap (default 1,000,000), with a corresponding byte allowance for leaf hashes, context and public parameters. If enrollment used a larger cap, supply an equal or larger value when signing or verifying.

## Public evidence and verification

The `gnorix.authorization.v1` JSON contains the enrollment ID/revision, exact action, public key, proof (`leaf_index`, `leaf_count`, sibling hashes) and signature. It contains no replacement root/context, coordinates, image or private key. Binary fields use lowercase hexadecimal; decoding checks field sets, lengths and numeric bounds.

Verification uses the independently trusted enrollment commitment, expected action, enrollment status and current time. It checks both the Merkle path and the signature, with the signed message binding the full enrollment context, root, leaf count and action. In this CLI, the separately supplied `.grx` path is the trusted enrollment source and status is active; do not substitute a package supplied by the authorization sender. Applications obtain current status and commitment from their trusted registry and apply permissions and durable replay handling before execution.

## Checked local example

This **profile v6, `min_priority = 0.5`** run used the full 1086×1448 image, three unscaled 760×1014 crops and one 512×512 crop. The container-v2 package has 48 baseline leaves and occupies **2,384 bytes**, including its public parameters. Every crop reused **D = 21.520984 R**, **k = 3** and **d = 10.760492 R** automatically; none recomputed the distances from its crop bounds. The radius multiplier was 30R throughout. Crop boxes use `(left, top, right, bottom)` with exclusive right/bottom coordinates. Attempts include baselines and searched positional variants.

| Input | Crop box | Measured R, px | Eligible points | Regions | Key attempts | Outcome |
|---|---|---:|---:|---:|---:|---|
| Full fixture | `(0, 0, 1086, 1448)` | 16.820793 | 30 | 4 | 1 | Matched |
| Top left | `(0, 0, 760, 1014)` | 16.743084 | 12 | 2 | 1 | Matched |
| Center | `(163, 217, 923, 1231)` | 16.195271 | 13 | 2 | 14 | Matched |
| Bottom right | `(326, 434, 1086, 1448)` | 16.353278 | 16 | 2 | 351 | Matched |
| Small center | `(287, 468, 799, 980)` | 16.331495 | 4 | 1 | 3,087 | Complete no-match |

The four matches passed separate signature/Merkle verification. The small-center crop exhausted its admitted regional candidates without a member; no signature was written. This demonstrates that stricter geometry can reduce partial-capture coverage. The [PNG overview](../reports/enrollment/signing/crop-signing.png), authorizations and [run record](../reports/enrollment/signing/results.json) are kept under `reports/enrollment/signing/`, separately from fingerprint report refreshes. The [verification record](../reports/enrollment/signing/verification.json) includes conflicting spatial overrides and tampered public geometry, keys and signatures.

These are re-extracted crops of the **same synthetic test image**, not new sensor acquisitions. They demonstrate the implemented search/sign/verify path with small test subsets. They do not measure repeatability across real captures or establish biometric acceptance, rejection or key-entropy properties.
