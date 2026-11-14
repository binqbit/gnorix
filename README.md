# Gnorix

Gnorix is an independent biometric-cryptographic library for signing data and digital actions with keys reconstructed from biometric measurements. It separates the signing mechanism from the application that verifies a signature and decides what it authorizes.

Repeated biometric captures are not identical. The native fingerprint path selects features meeting a configurable minimum priority and enrolls one canonical grid position set per selected combination. Reconstruction applies the same minimum and searches bounded positional variations lazily, checking each candidate immediately. A Merkle root, a compact commitment to the enrolled public keys, represents the enrollment without requiring the verifier to hold the entire set.

Each combination must satisfy the configured anchor count and minimum pairwise separation in ridge-width units R; its other points lie inside the anchors' convex hull. The retained policy is reused when signing from a partial capture.

For a later signature, the library reconstructs one candidate key from a fresh capture and any required PIN or password. The authorization combines that key's public counterpart, evidence that it belongs to the enrolled set, and a signature over the approved action. Verification does not require the raw biometric sample or every other candidate key.

The application retains control over enrollment ownership, permitted actions, reuse, revocation and recovery. Gnorix's signing mechanism is independent of a particular blockchain; the NEAR integration connects it to on-chain verification and permission-controlled operations. A valid signature establishes authorization evidence, not unlimited account access or proof of a live biometric capture.

## Documentation

- [Concept](docs/concept.md) explains the biometric-stability problem, accepted-key-set principle, Merkle commitment and intended role of the library.
- [How it works](docs/how-it-works.md) follows enrollment, retained proof material, fresh-capture reconstruction, signing, verification and recovery.
- [Architecture](docs/architecture.md) describes component responsibilities, the user/verifier boundary, independent library and NEAR integration.
- [Development](docs/development.md) covers the Rust workspace, setup, test commands, algorithm interfaces and implementation boundaries.
- [Fingerprint extraction](docs/fingerprint.md#feature-summary) summarizes feature measurements with an illustrated example and describes native ridge processing, graph geometry and reproducible PNG diagnostics.
- [Fingerprint signing](docs/fingerprint-signing.md) covers regional point-subset search, signing against an existing `.grx` package, public verification and checked crop examples.
- [Fingerprint test stand](docs/fingerprint-lab.md) provides a local Rust web interface for preparing regions, selecting circular captures, testing spatially varying capture artifacts and inspecting real signatures and matching feature points.
- [NEAR contract integration](docs/near-sdk.md) covers owner-controlled enrollment, contract verification, core authorization conversion and the runnable local sandbox example.
- [NEAR example](examples/near/README.md) demonstrates Gnorix-authorized transfers of a standard fungible token through a smart wallet, with one command to build and test the complete flow.

## Development

The repository-root `gnorix` package is the public library entry point. Its default `core` feature exposes the implementation in `crates/core` (`gnorix-core`) through both `gnorix::core::*` and the direct `gnorix::*` API. It provides native fingerprint extraction, ranked point subsets, canonical grid candidates, fresh-image signing search, enrollment packages, bounded signing/verification pipelines and a working SHA-256 Merkle commitment. Applications can use these fingerprint components or supply a `MeasurementEngine`; the KDF/signature provider remains an explicit choice. See [Development](docs/development.md#using-the-library) for dependency and feature selection.

The [Fingerprint Lab application](examples/fingerprint-lab/README.md) is a separate workspace package with its own manifest and web dependencies. Run it with `cargo run -p gnorix-lab --release --locked`.

Use the pinned Rust 1.99.1 toolchain, then run:

```sh
cargo test --workspace --all-targets --locked
cargo run -p gnorix-core --example merkle --locked
```

The `merkle` example exercises membership proofs with illustrative public-key bytes. The [fingerprint signing example](docs/fingerprint-signing.md) also derives keys and verifies actual signatures against an existing enrollment, including re-extracted crops of the synthetic fixture. The optional `near` feature adds contract verification and enrollment lifecycle management, with a separate local sandbox test; signing uses the existing core API. Real-capture repeatability, calibrated acceptance criteria, production KDF/signature selection and protected persistence remain development work. See [Development](docs/development.md) for the complete workflow and test scope.
