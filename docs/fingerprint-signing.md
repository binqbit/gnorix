# Signing from a fresh fingerprint image

`fingerprint::signing_candidates(&fingerprint, &config)` searches the observed features of a newly extracted image. `authorization::sign_candidates(...)` derives each candidate key, looks it up in the retained [`.grx` package](grx-packages.md), and signs the exact approved action with the first member found. The enrollment and its accepted key set remain unchanged.

## Candidate search

1. Extract the new image and measure its own mean ridge width R. Keep scored feature identities whose type and degree survive cleanup; measured zero priority remains eligible, while unknown priority does not.
2. Use the same adaptive circle geometry and initial `radius_units × R` as enrollment. If needed, expand a circle to include up to 2n available points. Search all scored points inside each effective circle, without enrollment’s top-pool truncation. Subsets contain `n..=min(2n, regional_points)` points. There is no global fallback.
3. Merge the regional subset streams lazily by descending mean weight, deduplicating identical source-point subsets from overlaps before hashing or counting key attempts, using `weight = 0.1 + 0.9 × priority²`. Ties follow point rank, with source IDs breaking equal-priority ties. These are deterministic ranking weights, not random probabilities. A subset can omit the strongest point; limited searches may stop before reaching weaker subsets.
4. Apply the same hull orientation, nearest-grid snapping, phase and fixed L-step padding used at enrollment. For the current subset, try its rounded baseline first, then stream the full Cartesian product of the L-step neighborhoods of its points. Keep that frame fixed; do not rotate or recenter individual variants.
5. For **each yielded assignment**, hash the canonical integer coordinates, derive the key under the retained context and secret policy, and immediately test its context-bound public-key hash for membership. Stop at the first match and return its public key, Merkle proof and signature. The highest-priority subset and all its variants are searched before any lower-priority subset; the schedule is deterministic.

Both the library iterator and signing pipeline have explicit bounds. Natural exhaustion without a member returns `CandidateNotEnrolled` for the configured regional search space. This does not rule out an enrolled combination spanning different regions. An attempt limit, work limit or algorithm failure means the search is incomplete; it must not be reported as a complete no-match result. No such outcome changes the package.

Each region contributes `Σ C(N, k)` subsets for `k = n..min(2n, N)` before overlap deduplication. Each k-point subset has up to `(1 + 3L(L + 1))^k` positional assignments. The search neither collects them in advance nor requires their full product to fit a count limit. It holds the current traversal state and produces the next assignment on demand. A small search space is exhausted completely if both limits permit it; larger searches stop at the first member or explicitly reach a limit.

This flow uses combination JSON v2 and example profile v3: enrollment commits only baseline keys, while signing searches positional uncertainty. Earlier profiles require a new enrollment for this reconstruction flow. Already issued authorizations can still be verified against their trusted original package. The `.grx` container and coordinate-hash format are unchanged.

## Run the example

First create the package using the [enrollment example](grx-packages.md#run-the-complete-example). Use the package path printed by enrollment and the same settings:

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

Use the same `GNORIX_COMBINATION_MIN_POINTS`, `GNORIX_COMBINATION_POOL_MULTIPLIER`, `GNORIX_COMBINATION_RADIUS_UNITS`, `GNORIX_COMBINATION_PER_REGION` and `GNORIX_GRID_UNCERTAINTY_STEPS` values as enrollment. They identify the example profile, with radius measured in R for both stages. Defaults are 3, 2, 30, 12 and 1 respectively. Required-secret enrollments also need the same `GNORIX_ENROLL_SECRET`. The shared example provider remains HKDF-SHA256/Ed25519; the library accepts the caller's `CryptoProvider`.

| Search control | Example default | Meaning |
|---|---:|---|
| `GNORIX_SIGNING_MAX_ATTEMPTS` | 1000000 | Maximum candidate keys examined. |
| `GNORIX_SIGNING_MAX_WORK` | 1000000000 | Cumulative point, subset-frontier, geometry and descriptor work. |

These limits do not alter the enrolled profile or accepted set. The core may request one extra iterator item to detect attempt overflow; it does not derive a key from that item. The example also bounds input JSON to 256 KiB and framed actions to 64 KiB. Package reading uses `GNORIX_ENROLLMENT_MAX_CANDIDATES` as its leaf cap (default 1,000,000), with a corresponding byte allowance for leaf hashes and context. If enrollment used a larger cap, supply an equal or larger value when signing or verifying.

## Public evidence and verification

The `gnorix.authorization.v1` JSON contains the enrollment ID/revision, exact action, public key, proof (`leaf_index`, `leaf_count`, sibling hashes) and signature. It contains no replacement root/context, coordinates, image or private key. Binary fields use lowercase hexadecimal; decoding checks field sets, lengths and numeric bounds.

Verification uses the independently trusted enrollment commitment, expected action, enrollment status and current time. It checks both the Merkle path and the signature, with the signed message binding the full enrollment context, root, leaf count and action. In this CLI, the separately supplied `.grx` path is the trusted enrollment source and status is active; do not substitute a package supplied by the authorization sender. Applications obtain current status and commitment from their trusted registry and apply permissions and durable replay handling before execution.

## Checked local example

Fresh extraction and signing were run against the baseline enrollment on the full 1086×1448 image, three unscaled 760×1014 crops and one 512×512 crop. The package contains 48 unique baseline leaves and occupies 1,886 bytes. The radius multiplier is 30R in every case. Crop boxes use `(left, top, right, bottom)` with exclusive right/bottom coordinates. Candidate attempts include the baseline and searched positional variants.

| Input | Crop box | Measured R, px | Regions | Key attempts until match |
|---|---|---:|---:|---:|
| Full fixture | `(0, 0, 1086, 1448)` | 16.820793 | 4 | 1 |
| Top left | `(0, 0, 760, 1014)` | 16.743084 | 2 | 1 |
| Center | `(163, 217, 923, 1231)` | 16.195271 | 2 | 14 |
| Bottom right | `(326, 434, 1086, 1448)` | 16.353278 | 2 | 351 |
| Small center | `(287, 468, 799, 980)` | 16.331495 | 1 | 92 |

All five runs produced a signature and Merkle proof that passed separate verification. The [PNG overview](../reports/enrollment/signing/crop-signing.png), authorizations and [run record](../reports/enrollment/signing/results.json) are kept under `reports/enrollment/signing/`, separately from fingerprint report refreshes. The [verification record](../reports/enrollment/signing/verification.json) also covers changed signed data/key/proof/signature, expiration, incompatible profiles and an incomplete one-attempt search.

These are re-extracted crops of the **same synthetic test image**, not new sensor acquisitions. They demonstrate the implemented search/sign/verify path with small test subsets. They do not measure repeatability across real captures or establish biometric acceptance, rejection or key-entropy properties.
