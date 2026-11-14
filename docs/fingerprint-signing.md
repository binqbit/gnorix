# Signing from a fresh fingerprint image

`fingerprint::analyze(&capture, &extraction_config)` returns compact `FingerprintFeatures`; `fingerprint::signing_candidates(&features, &config)` searches those observed features without generating report-only maps or curve descriptors. `authorization::sign_candidates(...)` derives each candidate key, looks it up in the retained [`.grx` package](grx-packages.md), and signs the exact approved action with the first member found. The enrollment and its accepted key set remain unchanged.

## Candidate search

1. Extract the new image and measure its own mean ridge width R. Keep feature identities whose type and degree survive cleanup and whose final `priority >= min_priority`, using the same cutoff as enrollment. Unknown priorities are excluded. Apply this filter before building regions; fewer than the required minimum eligible points is an explicit insufficient-data error.
2. Use the same adaptive circles and initial `radius_units × R` as enrollment. If needed, expand a circle to include `max_points` eligible points, or all available points if fewer exist. Search all eligible points in each effective circle. Subsets contain `min_points..=min(max_points, regional_points)` points and must satisfy the saved [spatial policy](feature-combinations.md#spatial-policy): at least k pairwise-separated anchors, with the remaining points inside their hull. Multiply the retained anchor distance in R by the fresh R. There is no global fallback.
3. Assign subsets by their strongest included priority: H `> 0.9`, M `0.5..=0.9`, L `< 0.5`. Schedule valid unique subsets in the deterministic cycle **H, M, H, L**, skipping exhausted bands. Within each band, merge the regional streams by descending mean weight, using `weight = 0.1 + 0.9 × priority²`; ties follow point rank, with source IDs breaking equal-priority ties. Deduplicate overlapping regions and reject failed source-space geometry before consuming a band turn. These checks consume work, not key attempts. The cycle gives 50%/25%/25% of subset turns while all bands remain available; it does not promise those proportions of point occurrences or key attempts. The minimum-priority cutoff still applies, so some bands can be empty.
4. Apply the same hull orientation, nearest-grid snapping, phase and fixed L-step padding used at enrollment. For the current admitted subset, try its rounded baseline first, then stream the full Cartesian product of the L-step neighborhoods of its points. Keep that frame fixed; do not rotate or recenter individual variants. Spatial admission is not checked again after snapping or neighbor movement.
5. For **each yielded assignment**, hash the canonical integer coordinates, derive the key under the retained context and secret policy, and immediately test its context-bound public-key hash for membership. Stop at the first match and return its public key, Merkle proof and signature. All positional variants of the current subset are searched before the next subset in the band schedule. Signing has no enrollment-style 3N subset cap; only the configured work bound and first successful match stop an otherwise complete regional traversal.

Both the library iterator and signing pipeline have explicit bounds. Natural exhaustion without a member returns `CandidateNotEnrolled` for the configured regional search space. This does not rule out an enrolled combination spanning different regions. Exhausting the work budget or encountering an algorithm failure means the search is incomplete; it must not be reported as a complete no-match result. No such outcome changes the package.

Each region contributes `Σ C(N, k)` subsets for `k = min_points..min(max_points, N)` before overlap deduplication. Each k-point subset has up to `(1 + 3L(L + 1))^k` positional assignments. The search neither collects them in advance nor requires their full product to fit a count limit. It holds the current traversal state and produces the next assignment on demand. A small search space is exhausted completely if the work budget permits it; larger searches stop at the first member or explicitly reach a limit.

This flow uses combination JSON v7, example profile v8 and `.grx` container v2. Enrollment commits only baseline keys, while signing searches positional uncertainty; both enforce the retained priority and spatial policy. Earlier fingerprint profiles must be recreated. The current CLI rejects old container files. Existing authorizations remain checkable through the generic verifier with their retained original trusted commitment. Coordinate hashing, context encoding and signature framing are unchanged.

## Run the example

First create the package using the [enrollment example](grx-packages.md#run-the-complete-example). Use the package path printed by enrollment; signing loads its reconstruction policy automatically:

```sh
grx_package=reports/enrollment/REPLACE_WITH_MERKLE_ROOT.grx

cargo run -p gnorix-core --example fingerprint_signing --release --locked -- \
  sign crates/core/tests/assets/test-fingerprint.png "$grx_package" \
  crates/core/examples/fingerprint_signing/action.json \
  reports/enrollment/signing/example-authorization.json

cargo run -p gnorix-core --example fingerprint_signing --release --locked -- \
  verify "$grx_package" \
  crates/core/examples/fingerprint_signing/action.json \
  reports/enrollment/signing/example-authorization.json "$(date +%s)"
```

Replace the image path with a new capture or crop to run fresh extraction. The output must not exist. Signing verifies the result before writing, then reloads and verifies the saved JSON. The verifier needs no image, private key or secret.

The action JSON contains exactly `operation` (nonempty text), `payload` (lowercase hexadecimal bytes), `nonce` (32 bytes in lowercase hexadecimal) and `expires_at` (unsigned Unix seconds in this example). The included action is a local fixture with a fixed nonce; applications create their own approved actions and manage nonce reuse.

The package retains explicit minimum/maximum point counts, priority cutoff, radius in R, anchor count and distance in R, and uncertainty steps. Signing loads these values automatically. Explicit combination-policy environment overrides and `GNORIX_GRID_UNCERTAINTY_STEPS` must equal the saved values; mismatches are rejected. Changing policy requires a new enrollment. Required-secret enrollments also need the same `GNORIX_ENROLL_SECRET`. The shared example provider remains HKDF-SHA256/Ed25519; the library accepts the caller's `CryptoProvider`.

| Search control | Example default | Meaning |
|---|---:|---|
| `GNORIX_SIGNING_MAX_WORK` | 1000000000 | Cumulative point, subset-frontier, geometry and descriptor work budget. |

There is one user-facing search budget. Actual key attempts remain reported statistics. For this fingerprint stream, the generic authorization API receives the same value as its candidate-attempt safety cap; every yielded descriptor already consumes positive work, so that cap does not independently stop this path. The generic API retains its cap for other candidate providers. Work units are accounting units, not milliseconds or a promise of constant processing cost.

The local work limit does not alter the enrolled profile or accepted set. The example also bounds input JSON to 256 KiB and framed actions to 64 KiB. Package reading uses `GNORIX_ENROLLMENT_MAX_CANDIDATES` as its leaf cap (default 1,000,000), with a corresponding byte allowance for leaf hashes, context and public parameters. If enrollment used a larger cap, supply an equal or larger value when signing or verifying.

## Public evidence and verification

The `gnorix.authorization.v1` JSON contains the enrollment ID/revision, exact action, public key, proof (`leaf_index`, `leaf_count`, sibling hashes) and signature. It contains no replacement root/context, coordinates, image or private key. Binary fields use lowercase hexadecimal; decoding checks field sets, lengths and numeric bounds.

Verification uses the independently trusted enrollment commitment, expected action, enrollment status and current time. It checks both the Merkle path and the signature, with the signed message binding the full enrollment context, root, leaf count and action. In this CLI, the separately supplied `.grx` path is the trusted enrollment source and status is active; do not substitute a package supplied by the authorization sender. Applications obtain current status and commitment from their trusted registry and apply permissions and durable replay handling before execution.

## Validation scope

Recreate packages and crop checks after changing the fingerprint profile. Earlier profile-v7 leaf counts, first-match counts and crop outcomes are historical; they do not describe the current profile-v8 schedule. A successful same-image or crop test verifies the implemented extraction/search/signature/Merkle path; it does not establish repeatability across independent sensor acquisitions.

Historical profile-v7 negative controls produced cross-finger matches with the coordinate-only descriptor. A valid signature and Merkle proof establish membership of the derived key, not that the scan belongs to the enrolled person. Changing the subset schedule does not change coordinate hashing or resolve that biometric ambiguity; new enrollments need fresh positive and negative controls.
