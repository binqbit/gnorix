# Gnorix

Gnorix is a Rust library for reconstructing signing keys from biometric features. It enrolls keys from multiple feature combinations, represents their accepted set with one Merkle root, and searches for a matching key when signing from a later capture. Applications verify the resulting signature and membership proof before authorizing an action.

The repository includes native fingerprint processing, PNG diagnostics, a fingerprint lab and a NEAR smart-wallet demonstration.

## Try it

Start with [Run and test locally](docs/local-testing.md) for one-time setup, expected results and test commands. After setup, choose an application and run its command from the repository root.

**Fingerprint Lab** — inspect features, create a Merkle set and test signatures without a blockchain. Open **http://127.0.0.1:8765**:

```sh
cargo run -p gnorix-lab --release --locked
```

**NEAR transfer demo** — the five-point configuration used by the hosted demo. Open **http://127.0.0.1:8780**:

```sh
cargo run -p gnorix-near-wallet --bin gnorix-near-demo --features demo --release --locked -- --points-per-combination 5
```

Each command runs in the foreground; stop it with Ctrl+C, or use separate terminals to run both. The NEAR command builds the contracts and starts a local sandbox automatically. No Docker, frontend build, scanner or downloaded dataset is needed. See the [NEAR setup requirements and options](examples/near/README.md) before the first run. The [hosted demo](https://gnorix.online) also executes transfers in a private sandbox; it does not move public-network assets. Each browser session has its own wallets, token balances and enrollments, expiring after one hour.

## Library

The public package is `gnorix`. Its default `core` feature exposes the implementation in `crates/core` through both `gnorix::*` and `gnorix::core::*`. The optional `near` feature adds contract-side verification and enrollment lifecycle management through `gnorix::near::*`.

The core provides fingerprint features, ranked combinations, canonical grid candidates, enrollment packages, signing search and SHA-256 Merkle proofs. Applications choose the cryptographic provider; the examples explicitly use HKDF-SHA256 and Ed25519. See [library setup](docs/development.md#using-the-library) for dependency declarations and feature selection.

## Documentation

| Topic | Guide |
| --- | --- |
| Purpose and mechanism | [Concept](docs/concept.md), [How it works](docs/how-it-works.md), [Architecture](docs/architecture.md) |
| Run and test | [Local testing: Lab, NEAR and reports](docs/local-testing.md), [Development and CI checks](docs/development.md) |
| Image processing and diagnostics | [Fingerprint extraction](docs/fingerprint.md), [Testing and datasets](docs/fingerprint-testing.md) |
| Key candidates | [Feature combinations](docs/feature-combinations.md), [Grid shapes](docs/grid-shapes.md) |
| Enrollment and reconstruction | [GRX packages](docs/grx-packages.md), [Fingerprint signing](docs/fingerprint-signing.md) |
| Interactive lab | [Fingerprint Lab](docs/fingerprint-lab.md) |
| NEAR | [Contract SDK](docs/near-sdk.md), [Runnable example](examples/near/README.md), [Deployment](examples/near/deploy/README.md) |

## Validation and limits

```sh
cargo test --workspace --all-targets --locked
cargo run -p gnorix-core --example merkle --locked
```

These checks cover software behavior; the [development guide](docs/development.md) separates ordinary tests from opt-in sandbox tests. Fingerprint scores are engineering rankings, not recognition probabilities or security bits. Repeat-capture reliability, resistance to unrelated fingerprints and production key protection remain unqualified. A cross-subject descriptor match has been observed; see the [biometric security boundary](docs/near-sdk.md#biometric-security-boundary) before using this prototype for authorization.

## License

[MIT](LICENSE). Third-party dependencies and external datasets retain their own licenses.

The author's other NEAR Hackathon project: [Apodect](https://github.com/binqbit/apodect).
