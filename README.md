# Gnorix

Gnorix is an independent biometric-cryptographic library for signing data and digital actions with keys reconstructed from biometric measurements. It separates the signing mechanism from the application that verifies a signature and decides what it authorizes.

Repeated biometric captures are not identical. Gnorix accounts for an accepted range of measurement variation during enrollment and derives a set of candidate signing keys. A Merkle root, a compact commitment to their public keys, represents the enrollment without requiring the verifier to hold the entire set.

For a later signature, the library reconstructs one candidate key from a fresh capture and any required PIN or password. The authorization combines that key's public counterpart, evidence that it belongs to the enrolled set, and a signature over the approved action. Verification does not require the raw biometric sample or every other candidate key.

The application retains control over enrollment ownership, permitted actions, reuse, revocation and recovery. Gnorix's signing mechanism is independent of a particular blockchain; the NEAR integration connects it to on-chain verification and permission-controlled operations. A valid signature establishes authorization evidence, not unlimited account access or proof of a live biometric capture.

## Documentation

- [Concept](docs/concept.md) explains the biometric-stability problem, accepted-key-set principle, Merkle commitment and intended role of the library.
- [How it works](docs/how-it-works.md) follows enrollment, retained proof material, fresh-capture reconstruction, signing, verification and recovery.
- [Architecture](docs/architecture.md) describes component responsibilities, the user/verifier boundary, independent library and NEAR integration.
- [Development](docs/development.md) covers the Rust workspace, setup, test commands, algorithm interfaces and implementation boundaries.

## Development

The Rust workspace contains the independent `gnorix` library in `crates/gnorix`. The foundation provides capture and geometry types, enrollment/signing/verification pipelines, bounded candidate processing, and a working SHA-256 Merkle commitment and proof implementation. Applications supply the measurement engine and KDF/signature provider explicitly.

Use the pinned Rust 1.99.1 toolchain, then run:

```sh
cargo test --workspace --all-targets --locked
cargo run -p gnorix --example merkle --locked
```

The example exercises membership proofs with illustrative public-key bytes. Biometric algorithms, numerical acceptance criteria, production KDF/signature providers, protected persistence and the NEAR execution adapter remain development work. See [Development](docs/development.md) for the complete workflow and test scope.
