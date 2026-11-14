# Grid candidates and `.grx` packages

Enrollment encodes one normalized, snapped baseline per selected point subset. A caller-selected `CryptoProvider` derives its signing key and public key; the enrollment pipeline commits context-bound public-key hashes to a Merkle tree. Signing searches positional uncertainty later, without enlarging the enrolled set. The package is named `<merkle-root>.grx`.

## Coordinates to a candidate

`fingerprint::grid_candidate(positions, phase, max_descriptor_bytes)` sorts integer nodes by `(row, column)`, preserves repeated coordinates, encodes them and hashes the complete descriptor with SHA-256. The resulting 32-byte digest is a sensitive `Candidate`.

Under the grid convention, sorting visits rows from top to bottom and columns from right to left. The aligned, padded frame stays intact. Feature IDs, kinds, priorities, source pixels, R and hull diagnostics do not enter the descriptor. Equal coordinate multisets and phase produce the same digest regardless of extraction numbering.

Each of these **four fields** has its own unsigned 64-bit big-endian byte-length prefix:

```text
"gnorix.grid-candidate.v1"
phase:u8
point_count:u64be
consecutive (column:u64be, row:u64be) pairs in sorted order
```

The descriptor occupies `65 + 16 × point_count` bytes. Phase must be 0 or 1; the point list must be nonempty. The complete size is checked before sorting/allocation. `GridBaselines::candidates(...)` encodes each baseline and propagates errors rather than silently truncating enrollment.

## Keys and membership

`enrollment::enroll_candidates(...)` accepts a candidate stream and an explicit `EnrollmentContext`. It shares validation, secret policy, resource limits and key/Merkle construction with `enrollment::enroll(...)`. The provider must deterministically bind the candidate, context and any required independent secret. The enrollment retains no candidate list or private keys.

Leaves are `merkle::leaf_hash(context, public_key)`. The tree sorts and deduplicates them; repeated baseline keys can therefore yield fewer leaves than input candidates. Every supplied candidate still counts toward the enrollment limit. The root binds the tree's top hash and distinct leaf count; it is a set commitment, not the signature algorithm's public key.

`EnrollmentPackage::proof_for(public_key, &limits)` computes the leaf, searches retained hashes and reconstructs its membership path. An absent leaf is an error. [Fresh-image signing](fingerprint-signing.md) signs with the first reconstructed member; it neither modifies nor extends the package.

## Retained package

In memory, `EnrollmentPackage` holds the context, optional public profile parameters, root, leaf count and all Merkle levels. Its **serialized form stores only the sorted leaf hashes**; decoding rebuilds internal nodes and paths:

```text
magic/version:    "GNORIX-GRX" || 0x00 || 0x02
context_length:   u64be
context:          exact EnrollmentContext::encode bytes
parameter_length: u64be
parameters:       exact public profile bytes; zero length means absent
root:             32 bytes
leaf_count:       u64be
leaves:           leaf_count × 32 bytes, strictly increasing
```

Neither form retains a public-key list, private keys, candidate digests, feature coordinates, source images or an independent secret. The context identifies network/verifier, subject, enrollment ID/revision, public salt, secret policy, profile ID and crypto suite. It uses the length-prefixed `gnorix.enrollment.v1` encoding.

`with_profile_parameters(...)` attaches a nonempty public record whose SHA-256 equals `context.profile_id`. Attachment, encoding and decoding check this binding and the parameter byte limit. Generic profiles may omit it; the fingerprint examples require it.

Their canonical compact JSON uses domain `gnorix.example.imported-grid-profile.v9` and combination schema `gnorix.feature-combinations.v7`. It retains minimum/maximum point counts, priority cutoff, radius in R, anchor count/distance and uncertainty steps. The domain fixes the [selection policy](feature-combinations.md#automatic-weighted-selection), including local coverage additions. Unknown fields, unsupported domains and noncanonical bytes are rejected. Work budgets are local execution controls and do not enter the profile.

Core decoding checks byte and leaf bounds, canonical context encoding, parameter hash binding, sorted unique leaves, absence of trailing data and the rebuilt root. The fingerprint adapter additionally validates its supported profile domain and canonical JSON. These establish internal consistency, not ownership or trust. Applications must obtain the trusted enrollment commitment independently; the hexadecimal filename is only a locator.

## Run the complete example

First [export feature combinations](feature-combinations.md#run-the-small-example), then run:

```sh
cargo run -p gnorix-core --example enroll_grid --release --locked -- \
  reports/fingerprint/feature-combinations.json \
  crates/core/examples/enroll_grid/context.json \
  reports/enrollment
```

The example validates imported subset sizes, eligibility and source-space anchor geometry before preparing baselines. It writes a new package, reloads it and checks a member's Merkle proof and signature. Existing files are not overwritten. `reports/enrollment/` is separate from the fingerprint report refresh.

| Environment variable | Example default | Meaning |
|---|---:|---|
| `GNORIX_GRID_UNCERTAINTY_STEPS` | 1 | Shared padding and later signing neighborhood L. |
| `GNORIX_ENROLLMENT_MAX_CANDIDATES` | 1000000 | Maximum baseline candidates; also the package leaf cap. |
| `GNORIX_GRID_MAX_WORK` | 10000000 | Baseline preparation and iteration work. |

The combination controls come from the input export. `GNORIX_GRID_MAX_VARIANTS` belongs only to the full positional diagnostic, not enrollment. Descriptors are bounded to 1 MiB; context JSON and encoded context/profile records are each bounded to 64 KiB. Package capacity is `32 × max_candidates + 2 × 64 KiB + 128` bytes.

The context JSON supplies `network`, `verifier`, `subject`, a 32-byte hexadecimal `id`, nonzero `revision`, hexadecimal `public_salt` and `secret_policy` (`absent` or `required`). The included file has fixed public values for a local fixture. Required-secret enrollment also reads `GNORIX_ENROLL_SECRET`; it is not written to the package or logs and must be supplied again during signing.

The explicit example suite is `gnorix.example.hkdf-sha256-ed25519.v1`, implemented in [example_crypto.rs](../examples/support/src/example_crypto.rs). HKDF-SHA256 uses the encoded context as salt. Its input separately length-prefixes `gnorix.example.ed25519-input.v1`, the candidate digest, a secret-presence byte and secret bytes. Expansion info is `gnorix.example.ed25519-seed.v1`; the 32-byte output seeds Ed25519 through `ed25519-dalek`. This is an example provider, not a library default or password-hardening policy. Hashing and key derivation do not establish biometric entropy.

[Signing](fingerprint-signing.md#run-the-example) loads the retained policy automatically. Changing that policy requires a new enrollment; the current decoder accepts only the format shown here.
