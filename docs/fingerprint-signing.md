# Signing from a fresh fingerprint image

`fingerprint::analyze(&capture, &extraction_config)` returns compact features without report-only maps. `authorization::sign_candidates(...)` consumes a lazy fingerprint candidate stream, derives each key and searches the retained [`.grx` package](grx-packages.md). The first member signs the exact approved action. Enrollment and its accepted key set remain unchanged.

## Candidate search

1. Analyze the new image and measure its own mean ridge width R. Keep surviving features with known final priority at or above the retained cutoff. Fewer than `max(min_points, min_anchor_points)` eligible points is an insufficient-data error.
2. Build the same [adaptive circles](feature-combinations.md#regions-and-eligible-points), using the saved radius in R. Sparse circles expand to `max_points` eligible points, or all available points if fewer exist. There is no global fallback.
3. Traverse regional subsets within the saved point-count range and [spatial policy](feature-combinations.md#spatial-policy). Deduplicate overlapping regions and schedule valid subsets in **H, M, H, L** order, according to their strongest included priority. Within each band, merge regional streams by descending mean point weight. Rejected geometry consumes work, not a key attempt or a band turn. Signing has no enrollment-style 3N subset cap and does not require every observed point to have a coverage witness before starting.
4. Apply the [same hull orientation, snapping, phase and L-step padding](grid-shapes.md). Try the current subset's baseline, then its positional variants in the fixed frame. Complete that subset's neighborhood before advancing; do not recheck source-space anchor distances after moving nodes.
5. For each yielded assignment, hash the canonical coordinates, derive the key under the retained context/secret policy and immediately test Merkle membership. Stop at the first member and return its public key, membership proof and action signature.

`signing_candidates(...)` and `prepare_signing_search(...)` traverse the full positional product. The latter also exposes observed points, search circles and the most recently yielded feature IDs for diagnostics.

For an enrollment known to contain **only canonical baselines**, `prepare_baseline_signing_search(...)` skips assignments whose top/right extrema cannot equal an enrolled baseline's padding boundaries. Rejected placements consume work but avoid hashing and key derivation. Possible matches keep their original order and coordinates. Use the generic search for packages containing arbitrary positional candidates. The CLI below uses the generic search; the NEAR demonstration uses the baseline-only optimization.

For N regional points, the unfiltered subset count is `Σ C(N, k)` over the permitted sizes. Each k-point subset has `(1 + 3L(L + 1))^k` positional assignments. The iterator does not collect this product or require its total to fit a counter before starting. Spatial admission, overlap removal and optional baseline-bound checks reduce the work reaching key derivation.

| Outcome | Meaning |
|---|---|
| First matching member | Sign and return its public evidence. |
| `CandidateNotEnrolled` after natural exhaustion | No member in the configured regional search space. |
| Work/attempt limit or algorithm error | Search did not complete; this is not a definitive no-match result. |
| Insufficient eligible points | Capture cannot supply the configured minimum. |

A completed regional no-match does not rule out an enrolled subset spanning different search circles. None of these outcomes changes the package.

## Run the example

First create a package using the [enrollment example](grx-packages.md#run-the-complete-example). Use the path printed by enrollment:

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

Replace the image with a fresh capture or crop to repeat extraction. The output must not exist. Signing verifies before writing, then reloads and verifies the saved JSON. Verification needs no image, private key or secret.

The action JSON contains exactly `operation` (nonempty text), `payload` (lowercase hexadecimal bytes), `nonce` (32 bytes in lowercase hexadecimal) and `expires_at` (unsigned Unix seconds). The included action is a local fixture with a fixed nonce. Applications must create approved actions and durably prevent nonce reuse; the standalone CLI is not a replay registry.

Signing loads point-count bounds, cutoff, radius, anchors and uncertainty from the package's canonical fingerprint profile. Explicit combination-policy environment overrides and `GNORIX_GRID_UNCERTAINTY_STEPS` must equal those retained values. Required-secret enrollments need the same `GNORIX_ENROLL_SECRET`. The examples use HKDF-SHA256/Ed25519; the core accepts the application's `CryptoProvider`.

| Search control | Example default | Meaning |
|---|---:|---|
| `GNORIX_SIGNING_MAX_WORK` | 1000000000 | Cumulative point, frontier, geometry, placement and descriptor work. |

This local budget does not alter the enrolled policy. Actual key attempts are reported separately. The CLI passes the same value to the generic authorization API's candidate-attempt cap; each fingerprint candidate already consumes positive work, so this cap does not independently stop this path. Work units are accounting units, not elapsed time or constant-cost operations.

Input/authorization JSON is limited to 256 KiB and framed actions to 64 KiB. `GNORIX_ENROLLMENT_MAX_CANDIDATES` sets the package leaf cap (default 1,000,000) and its corresponding byte allowance. If a package was enrolled with a larger limit, supply a sufficient value when signing or verifying.

## Public evidence and verification

The `gnorix.authorization.v1` JSON contains enrollment ID/revision, exact action, public key, proof (`leaf_index`, `leaf_count`, sibling hashes) and signature. Binary fields use lowercase hexadecimal; decoding validates fields, lengths and numeric bounds. It contains no replacement root/context, feature coordinates, image or private key.

Verification takes an independently trusted commitment, expected action, current enrollment status and current time. It checks the Merkle path and the signature; the signed message binds the full enrollment context, root, leaf count and action. In this CLI, the separately supplied `.grx` file is the trusted enrollment source and status is active. Do not accept a sender-selected package as that trust source. Applications must provide their trusted registry, permissions and replay handling before executing an action; the [NEAR integration](near-sdk.md) demonstrates that boundary.

## Validation scope

A successful same-image or crop test validates the implemented extraction, search, signature and Merkle path. It does not establish repeatability across independent sensor captures or prove a person's identity. The descriptor contains coordinates, not feature kinds or a full biometric template.

Earlier prototype negative controls produced cross-finger matches with this coordinate-only encoding; see the recorded [biometric security boundary](near-sdk.md#biometric-security-boundary). The current subset schedule does not resolve that ambiguity. Evaluate each enrollment profile with fresh positive and negative controls; a cryptographically valid member can still be the result of a biometric false match.
