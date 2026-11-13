# Grid candidates and `.grx` packages

Enrollment turns the normalized, snapped baseline of each selected point subset into one candidate digest. The caller's `CryptoProvider` derives a private/public key pair; the shared enrollment pipeline commits the public-key hashes to a Merkle tree. Positional uncertainty is searched lazily during signing and does not multiply the enrolled set. The resulting `<merkle-root>.grx` stores the context and hashes needed to find a member and reconstruct its proof later.

## Coordinates to a candidate

`fingerprint::grid_candidate(positions, phase, max_descriptor_bytes)`:

1. Sort the final integer nodes by `(row, column)`: top to bottom, then right to left under our grid convention.
2. Preserve repeated coordinates when features coincide. Keep the previously aligned and padded frame.
3. Encode phase, point count and consecutive `(column, row)` records with the versioned framing below.
4. Hash the entire descriptor with SHA-256. The 32-byte digest becomes a sensitive `Candidate`.

Feature IDs, kinds, priorities, original pixels, R and hull diagnostics do not enter this descriptor. Equal positional multisets with the same phase produce the same digest regardless of their extraction numbering.

Every field below, including the domain, has an unsigned 64-bit big-endian byte-length prefix:

```text
"gnorix.grid-candidate.v1"
phase:u8
point_count:u64be
(column:u64be, row:u64be) repeated in sorted order
```

The complete descriptor occupies `65 + 16 × point_count` bytes. Its size is checked before sorting/allocation. `GridBaselines::candidates(...)` applies this operation once per prepared baseline and propagates failures; it does not silently truncate the enrolled set. The separate `GridShapes` API remains available for complete diagnostic exports of positional variants.

## Keys and membership

`enrollment::enroll_candidates(...)` accepts an existing candidate stream and an explicit `EnrollmentContext`. It shares context validation, secret policy, resource limits and key/Merkle construction with `enrollment::enroll(...)`. The supplied provider must deterministically bind the candidate, context and any required independent secret. Transient candidates and private keys are not retained by enrollment.

Each leaf is the existing context-bound `merkle::leaf_hash(context, public_key)`. The tree sorts and deduplicates these hashes. Equal baselines can therefore produce fewer distinct leaves; each supplied baseline still counts toward enrollment limits.

For a later public key, `EnrollmentPackage::proof_for(public_key, &limits)` computes the same leaf hash, searches the sorted leaves, and constructs the membership path. Verify it with `merkle::verify` against the application's trusted root. An absent leaf is an explicit error.

[Fresh-image signing](fingerprint-signing.md) generates candidate keys from newly extracted point subsets and searches this retained tree. It signs with the first member and returns its public key, membership proof and action signature; it does not modify or extend the package.

## Retained package

`EnrollmentPackage` contains the full enrollment context, optional public profile parameters, Merkle root, leaf count and retained hash tree. Its tree representation stores only canonical leaf hashes: all internal nodes and paths are rebuilt on load. It contains no public-key list, private keys, candidate digests, feature coordinates, source image or independent secret.

```text
magic/version:    "GNORIX-GRX" || 0x00 || 0x02
context_length:   u64be
context:          exact EnrollmentContext::encode bytes
parameter_length: u64be
parameters:       exact public profile parameter bytes; zero length means absent
root:             32 bytes
leaf_count:       u64be
leaves:           leaf_count × 32 bytes, strictly increasing
```

The context includes the domain, subject, enrollment ID/revision, public salt, secret policy, measurement profile ID and crypto suite ID. Its encoding remains the existing `gnorix.enrollment.v1` length-prefixed format. Context and signature framing have not changed.

`EnrollmentPackage::with_profile_parameters(...)` attaches a nonempty public record. SHA-256 of its exact bytes must equal `context.profile_id`; attachment, encoding and decoding check this binding. The record has its own `Limits::context_bytes` bound. Generic constant profiles can omit the record.

Loading enforces byte, context, parameter and leaf-count bounds, validates fields, rejects duplicate/unsorted hashes and trailing data, and checks the rebuilt root. These checks establish consistency of the hash material and its parameter binding. They do not independently authenticate the context or establish that the enrollment belongs to a particular user; the full trusted commitment remains application state. The hexadecimal filename is a locator.

## Run the complete example

```sh
cargo run -p gnorix --example enroll_grid --release --locked -- \
  reports/fingerprint/feature-combinations.json \
  crates/gnorix/examples/enroll_grid/context.json \
  reports/enrollment
```

The included context has fixed public values for the local test image. The combination export records its inclusive `min_priority` cutoff; the example default is 0.5. Signing must use the same cutoff, and changing it requires a new enrollment. `GNORIX_GRID_UNCERTAINTY_STEPS` selects the shared L-step padding and later signing neighborhood (default 1). `GNORIX_ENROLLMENT_MAX_CANDIDATES` limits baseline candidates (default 1,000,000); `GNORIX_GRID_MAX_WORK` limits preparation/iteration work (default 10,000,000). The diagnostic `GNORIX_GRID_MAX_VARIANTS` setting does not control enrollment. The context JSON explicitly supplies `network`, `verifier`, `subject`, 32-byte hexadecimal `id`, nonzero `revision`, hexadecimal `public_salt`, and `secret_policy` (`absent` or `required`). For `required`, supply `GNORIX_ENROLL_SECRET`; it is not written to the package or logs.

Imported combinations must use `gnorix.feature-combinations.v3`. Enrollment checks that every selected point has a known final priority at or above the declared cutoff; an inconsistent imported selection is rejected instead of silently repairing it.

The explicit example suite is `gnorix.example.hkdf-sha256-ed25519.v1`: [HKDF-SHA256](https://www.rfc-editor.org/rfc/rfc5869.html) derives a 32-byte seed, and [ed25519-dalek](https://docs.rs/ed25519-dalek/3.0.0/ed25519_dalek/) handles key generation and signatures. HKDF uses the encoded context as salt; its input frames the domain `gnorix.example.ed25519-input.v1`, candidate digest, secret-presence byte and secret bytes with `u64be` lengths. Expansion info is `gnorix.example.ed25519-seed.v1`. This is an example-selected suite, not a library default or a password-hardening policy.

The command traverses every selected baseline, writes a new package exclusively, reloads it and verifies a reconstructed member's Merkle proof and Ed25519 signature. Existing files are not overwritten. Package files stay separate from the fingerprint report refresh. The [signing example](fingerprint-signing.md#run-the-example) uses container v2, profile v4 and the same crypto suite. Container v1 is rejected by the current decoder, so earlier test enrollments must be recreated. Existing authorizations can still be checked through the generic verifier against their retained original trusted commitment; the current CLI does not load their old `.grx` files. Repeatability on independent sensor captures and biometric acceptance remain separate work.
