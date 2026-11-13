# Signing from a fresh fingerprint image

`fingerprint::signing_candidates(&fingerprint, &config)` searches the observed features of a newly extracted image. `authorization::sign_candidates(...)` derives each candidate key, looks it up in the retained [`.grx` package](grx-packages.md), and signs the exact approved action with the first member found. The enrollment and its accepted key set remain unchanged.

## Candidate search

1. Extract the new image and measure its own mean ridge width R. Keep scored feature identities whose type and degree survive cleanup; measured zero priority remains eligible, while unknown priority does not.
2. Use the same adaptive circle geometry and initial `radius_units × R` as enrollment. If needed, expand a circle to include up to 2n available points. Search all scored points inside each effective circle, without enrollment’s top-pool truncation. Subsets contain `n..=min(2n, regional_points)` points. There is no global fallback.
3. Merge the regional subset streams lazily by descending mean weight, deduplicating identical source-point subsets from overlaps before hashing or counting key attempts, using `weight = 0.1 + 0.9 × priority²`. Ties follow point rank, with source IDs breaking equal-priority ties. These are deterministic ranking weights, not random probabilities. A subset can omit the strongest point; limited searches may stop before reaching weaker subsets.
4. Apply the same hull orientation, nearest-grid snapping, phase and fixed L-step padding used at enrollment. Produce **one baseline assignment per subset**. Enrollment already committed the allowed positional alternatives; signing does not add a second uncertainty neighborhood or widen L.
5. Hash the canonical integer coordinates, derive the key under the retained context and secret policy, and test its context-bound public-key hash for membership. Stop at the first match and return its public key, Merkle proof and signature.

Both the library iterator and signing pipeline have explicit bounds. Natural exhaustion without a member returns `CandidateNotEnrolled` for the configured regional search space. This does not rule out an enrolled combination spanning different regions. An attempt limit, work limit or algorithm failure means the search is incomplete; it must not be reported as a complete no-match result. No such outcome changes the package.

Baseline-only search removes positional variants. Each region still contributes `Σ C(N, k)` subsets for `k = n..min(2n, N)` before overlap deduplication. With n = 3, even 31 regional points give 942,152 subsets. A small regional search space is exhausted completely if both limits permit it; larger searches stop at the first member or explicitly reach a limit.

The adaptive-region policy uses combination JSON v2 and example profile v2. Earlier fractional-radius packages are not compatible with this profile: regenerate enrollment with the new settings. The `.grx` container and coordinate-hash format are unchanged.

## Run the example

First create the package using the [enrollment example](grx-packages.md#run-the-complete-example). The following uses its default-settings fixture package; for another enrollment, substitute its trusted path and matching settings:

```sh
grx_package=reports/enrollment/76af87f11cc25e45757a1e8ff08b8223dd691f8a385d421a8d07d50eb510d597.grx

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

Use the same `GNORIX_COMBINATION_MIN_POINTS`, `GNORIX_COMBINATION_POOL_MULTIPLIER`, `GNORIX_COMBINATION_RADIUS_UNITS`, `GNORIX_COMBINATION_PER_REGION` and `GNORIX_GRID_UNCERTAINTY_STEPS` values as enrollment. They identify the example profile, with radius measured in R for both stages. Defaults are 3, 2, 30, 12 and 1 respectively. Required-secret enrollments also need the same `GNORIX_ENROLL_SECRET`. The shared example provider remains HKDF-SHA256/Ed25519; the library accepts the caller's `CryptoProvider`.

| Search control | Example default | Meaning |
|---|---:|---|
| `GNORIX_SIGNING_MAX_ATTEMPTS` | 1000000 | Maximum candidate keys examined. |
| `GNORIX_SIGNING_MAX_WORK` | 1000000000 | Cumulative point, subset-frontier, geometry and descriptor work. |

These limits do not alter the enrolled profile or accepted set. The core may request one extra iterator item to detect attempt overflow; it does not derive a key from that item. The example also bounds input JSON to 256 KiB, framed actions to 64 KiB and packages to 64 MiB / one million leaves.

## Public evidence and verification

The `gnorix.authorization.v1` JSON contains the enrollment ID/revision, exact action, public key, proof (`leaf_index`, `leaf_count`, sibling hashes) and signature. It contains no replacement root/context, coordinates, image or private key. Binary fields use lowercase hexadecimal; decoding checks field sets, lengths and numeric bounds.

Verification uses the independently trusted enrollment commitment, expected action, enrollment status and current time. It checks both the Merkle path and the signature, with the signed message binding the full enrollment context, root, leaf count and action. In this CLI, the separately supplied `.grx` path is the trusted enrollment source and status is active; do not substitute a package supplied by the authorization sender. Applications obtain current status and commitment from their trusted registry and apply permissions and durable replay handling before execution.
