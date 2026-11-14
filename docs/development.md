# Development

## Workspace and setup

The repository uses a Rust 2024 Cargo workspace. The root `gnorix` package is the public library facade; `crates/core` contains the `gnorix-core` implementation and `crates/near` the optional NEAR SDK. `examples/fingerprint-lab` is the independent `gnorix-lab` application. `examples/support` provides the shared crypto, enrollment-profile, file/JSON and test-image capture/artifact adapters used by examples. The single NEAR example lives in `examples/near`: its main package is the smart wallet, `token/` builds the separate standard token, and `tests/` contains sandbox checks and their receiver fixture. Its `demo` feature starts a Rust web interface and manages the local NEAR sandbox with real fingerprint signing. Run the single command in its README for the web demo or `./examples/near/test.sh` for the contract regression suite; there is no separate integration-test package. Each package has its own manifest and dependencies; the workspace shares `Cargo.lock`, build output and quality checks.

Install Rust **1.99.1** with Cargo, rustfmt and Clippy. `rust-toolchain.toml` selects that version for rustup users. With rustup:

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
# POSIX shells: keep Cargo and its subcommands on the same pinned toolchain.
export PATH="$(dirname "$(rustup which --toolchain 1.99.1 rustc)"):${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
rustc --version
cargo --version
cargo clippy --version
cargo fmt --version
cargo fetch --locked
```

On NixOS or another system-managed installation, an earlier `cargo-clippy` on
`PATH` can come from a different release even when `rustc` is pinned. Use the
toolchain-bin prefix above for the whole shell, or provide the complete 1.99.1
toolchain through the system configuration. Check for conflicting `RUSTC` and
`RUSTDOC` overrides as well. Mixing system 1.100 tools with the pinned 1.99
compiler can cause incompatible compiler-plugin errors. Once the toolchain-bin
prefix is active, use plain `cargo`; `cargo +VERSION` requires the rustup proxy.

The library needs no scanner, blockchain node, credentials or database to build
and test. Keep `Cargo.lock` in version control. Package publication is disabled
during this prerelease stage.

The core implementation dependencies are [RustCrypto SHA-2](https://docs.rs/sha2/0.11.1/sha2/) for complete SHA-256 digests and [zeroize](https://docs.rs/zeroize/1.9.1/zeroize/) for owned sensitive buffers and signing-key disposal contracts. These dependencies do not select a biometric KDF or signature suite.

## Using the library

Consumers depend on the repository-root `gnorix` package. For a local checkout, use a path relative to the consuming application's `Cargo.toml`:

```toml
[dependencies]
gnorix = { path = "../gnorix" }
```

The `core` feature is enabled by default. Select the component explicitly when desired:

```toml
[dependencies]
gnorix = { path = "../gnorix", default-features = false, features = ["core"] }
```

Both imports expose the same implementation and types:

```rust
use gnorix::core::biometrics::fingerprint;
// The direct API is also available: use gnorix::biometrics::fingerprint;
```

With no features selected, `gnorix` has no implementation dependencies. `core` exposes the biometric and signing APIs. The optional `near` feature adds `gnorix::near::contract::Enrollment` for persistent contract authorization state and strict Ed25519 verification, together with bounded public transport types. NEAR uses the core implementation internally even when its direct facade exports are disabled. Signing remains in the core API. See the [NEAR contract guide](near-sdk.md) for dependencies, ownership and replay rules.

The CLI examples belong to the implementation package at `crates/core/examples/`, so their commands use `-p gnorix-core`. Application consumers continue to use `gnorix`.

## Routine checks

Run from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
cargo test --workspace --release --locked
cargo doc --workspace --no-deps --locked
cargo check -p gnorix --no-default-features --locked
cargo check -p gnorix --no-default-features --features core --locked
cargo check -p gnorix --no-default-features --features near --locked
cargo test -p gnorix-near --locked
cargo run -p gnorix-core --example merkle --locked
```

The optional NEAR demo is not enabled by ordinary workspace checks. Check its
HTTP/session code separately; opt-in real-sandbox commands are in the
[NEAR example](../examples/near/README.md#verification):

```sh
cargo clippy -p gnorix-near-wallet --features demo --all-targets --locked -- -D warnings
cargo test -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked
```

Use `RUSTDOCFLAGS="-D warnings"` for documentation checks on a POSIX shell. Cargo writes build output to the ignored root `target/` directory. The GitHub Actions workflow checks formatting, Clippy and documentation, and declares test jobs for Linux, macOS and Windows. A configured workflow does not itself establish that those hosted runs have passed.

## Implemented responsibilities

| Component | Location | Responsibility |
|---|---|---|
| Acquisition input | `src/biometrics/capture.rs` | Owned grayscale pixels, optional valid-region mask and scale; shape and numeric validation; redacted diagnostics. |
| Grid mathematics | `src/biometrics/geometry.rs` | Explicit triangular positions, Euclidean-nearest nodes and six-neighbor coordinates, with overflow/finite-value checks. |
| Fingerprint extraction | `src/biometrics/fingerprint/` | Native per-capture image stages, topology-preserving skeleton tracing, curve geometry and diagnostic output. |
| Regional subsets | `src/biometrics/fingerprint/combinations/` | Adaptive overlapping circles, retained anchor-envelope requirements and bounded weighted valid combinations. |
| Fresh-image search | `src/biometrics/fingerprint/combinations/search.rs` | Ranked min–max subsets inside adaptive circles, overlap deduplication and lazy per-subset positional uncertainty search. |
| Grid figures | `src/biometrics/fingerprint/shapes/` | Convex-hull frames, measured-width snapping, baseline enrollment and fixed-frame positional traversal. |
| Measurement boundary | `src/biometrics/mod.rs` | Algorithm-owned reconstruction model and fallible candidate streams. |
| Crypto boundary | `src/crypto.rs` | Explicit KDF/signature suite, secret input handling and candidate-key operations. |
| Enrollment | `src/enrollment.rs` | Bind profile and context, prepare a model, process the complete candidate set, retain its commitment/model/hash tree. |
| Enrollment packages | `src/package.rs`, `src/package/` | Bounded canonical `.grx` storage of context, hash-bound public profile parameters and leaf hashes, validated tree reconstruction and membership lookup. |
| Signing and verification | `src/authorization.rs` | Reconstruct a member, sign an exact action, and verify against trusted application state. |
| Commitment | `src/merkle.rs` | Deterministic SHA-256 tree, indexed membership lookup and strict proof verification. |
| Limits and errors | `src/policy.rs`, `src/error.rs` | Caller-selected software bounds, per-enrollment secret policy and typed failures with algorithm error sources. |
| Byte framing | `src/encoding.rs` | Bounded, unambiguous field framing for contexts and signed messages. |

Locations in this table are relative to `crates/core`. High-level functions call named mechanisms; the mechanisms delegate hashing, framing and algorithm operations to their respective components. The pipelines are `enrollment::enroll` / `enroll_candidates`, `authorization::sign` / `sign_candidates` and `authorization::verify`.

### Fingerprint implementation layout

The public entry points and types are exported by `src/biometrics/fingerprint/mod.rs`. Implementation modules own their data and operations together:

| Location within `src/biometrics/fingerprint/` | Responsibility |
|---|---|
| `pipeline.rs`, `analysis.rs`, `analysis/` | Compose shared source analysis, project surviving features into a compact result, and separately enrich full report diagnostics. |
| `map.rs`, `graph.rs` | Binary maps, boundary provenance, traced graph data, shared incidence and connected-component traversal. |
| `scale.rs`, `scale/`, `full_scale.rs`, `full_scale/` | Bootstrap search, grayscale profiles, complete ridge-width measurement and local measurement support. |
| `enhancement/` | Local normalization, intensity filtering, orientation tensors and reusable directional responses. |
| `cleanup/`, `smoothing/`, `repair/` | Source-supported cleanup, constrained smoothing and gap reconstruction. Repair separates cubic construction, obstruction checks and graph assembly. |
| `features/` | Source probes, geometric evidence and stress correspondence. Ending assessment borrows unchanged paths and retains only the clipped remainder of its own edge. |
| `combinations.rs`, `combinations/` | Shared candidate projection and lazy weighted subset enumeration; regional enrollment selection and regional fresh-image signing search with shared circle geometry. |
| `shapes.rs`, `shapes/` | Prepare oriented figures, preserve staggered-grid phase, stream positional assignments and hash canonical integer-coordinate descriptors. |

Feature assessment separates local evidence (`features/evidence.rs`), extended source-backed structure (`features/context.rs` and its tracing/probe modules), threshold correspondence (`features/stability/`) and bounded crowding (`features/neighborhood.rs`). The orchestration combines these independent measurements; the report adapters only serialize and validate them. All source/graph observation preserves the original topology. Context traversal, sampling and neighborhood work have explicit bounds.

`fingerprint::analyze` returns `FingerprintFeatures`: measured R, image dimensions, confirmed-pattern bounds and surviving features with their known or unknown priorities. Combination and signing policies consume this compact result. They apply their cutoff afterward and extend the pattern bounds to cover the eligible points. Analysis preserves all eight stress variants, source checks and contour cleanup; it omits report-only clarity maps, width summaries and curve descriptors. `fingerprint::extract` runs the same mechanisms and adds those diagnostics for reports. `Fingerprint::features()` projects a diagnostic result into the same compact representation.

Failures in requested report diagnostics still fail `extract`. They cannot block `analyze`, which does not run those stages. Errors in the shared source analysis fail both entry points.

### Example and tool entry points

| Entry point | Role and guide |
| --- | --- |
| `crates/core/examples/fingerprint.rs` | Full extraction and diagnostic JSON/raster export; [fingerprint processing](fingerprint.md). |
| `crates/core/examples/feature_combinations.rs` | Compact analysis and regional subset export; [combinations](feature-combinations.md). |
| `crates/core/examples/feature_grid.rs` | Bounded, streamed grid diagnostics from that export; [grid shapes](grid-shapes.md). |
| `crates/core/examples/enroll_grid.rs` | Baseline key enrollment and verified package output; [GRX packages](grx-packages.md). |
| `crates/core/examples/fingerprint_signing.rs` | Fresh-image reconstruction and public authorization verification; [signing](fingerprint-signing.md). |
| `examples/fingerprint-lab/` | Shared local test workspace; [Lab](fingerprint-lab.md). |
| `examples/near/` | One sandbox, isolated browser sessions and public-fixture preparation shared across sessions; [NEAR demo](../examples/near/README.md). |
| `tools/fingerprint-reports/` | Native execution, diagnostic validation and PNG publication; [report tool](../tools/fingerprint-reports/README.md). |
| `tools/fingerprint-gallery/` | Pixel-preserving import of the optional fixed gallery; [gallery tool](../tools/fingerprint-gallery/README.md). |

CLI-only modules under `crates/core/examples/support/` own bounded image input,
combination/grid controls and secret entry. The shared `examples/support/`
package owns the example crypto provider, enrollment profile, authorization JSON,
exclusive file output and capture/artifact adapters. File I/O and report layout
remain outside the library.

The Lab caches one compact analyzed capture, keyed by source revision, circle and
extraction controls. Policy changes rebuild selection/search without reprocessing
pixels; switching gallery inputs clears capture-dependent state. The NEAR demo
shares immutable public-fixture analysis and baseline descriptors, while each
session keeps its own accounts, enrollments, balances and approvals. Neither
application behavior is part of the generic core API.

## Connecting algorithms

Use the native [fingerprint components](fingerprint.md) for per-image extraction
and candidate generation. Multi-capture registration/fusion and calibrated
biometric acceptance criteria remain separate work.

Implement `MeasurementEngine` for the selected measurement strategy. It supplies a profile identifier, builds its own reconstruction model, enumerates all accepted enrollment candidates, and reconstructs candidates from a new capture under that same model. Its identifier must cover every setting that affects interpretation or candidate bytes.

A custom engine owns any alignment, fusion, quantization, uncertainty and missing-point rules it requires. The core consumes its candidate stream; it does not infer those algorithms or tolerances from grid spacing.

Implement `CryptoProvider` for the selected KDF and signature suite. Its identifier must cover the algorithms, parameters and key/signature encodings. `derive_key` receives canonical candidate bytes, the full framed enrollment context and the optional independent secret. The implementation must bind all required inputs, deterministically reproduce the same key/public-key encoding for identical inputs, and follow its crypto library's valid-key rules. Signing keys implement `ZeroizeOnDrop`; implementations remain responsible for intermediate buffers and copies. There is no default or placeholder production provider.

`SignatureVerifier` is the narrower public verification interface. `CryptoProvider` extends it, while `authorization::verify` requires only `SignatureVerifier`. The NEAR adapter implements strict Ed25519 verification without key-derivation or signing stubs.

The enrollment chooses `SecretPolicy::Absent` or `SecretPolicy::Required`. Supplying a secret to an absent-secret enrollment, or omitting a required one, is rejected. An empty secret is rejected; no minimum entropy or password-strength rule is inferred from byte length. `Candidate`, `SecretInput` and pixel buffers clear their owned bytes on drop. That does not erase caller copies, arbitrary algorithm models, registers or the whole device's memory.

Use these public entry points in order:

1. Call `enrollment::enroll` with the engine, provider, captures, explicit `EnrollmentSpec`, optional secret and `Limits`.
2. Register `enrollment.commitment()` through the application's owner-authorized registration flow. Protect the local enrollment and algorithm-owned reconstruction model separately.
3. Obtain approval for the exact `Action`, then call `authorization::sign` with a fresh capture.
4. Read the expected enrollment commitment/status from a trusted registry and call `authorization::verify` with the expected action and trusted current time.
5. Apply the application's permissions and durable replay/execution policy before performing the action.

The [lifecycle integration tests](../crates/core/tests/lifecycle/main.rs) demonstrate this wiring with deliberately nonproduction measurement and crypto doubles. The suite groups enrollment, verification and failure behavior into cohesive modules with shared fixtures. They are test fixtures, not algorithms to copy into an application.

For an existing fingerprint `.grx` package, validate its retained public policy,
then call `fingerprint::analyze` → `fingerprint::signing_candidates` →
`authorization::sign_candidates`, followed by trusted verification. Example
profile **v10** binds the point-count range, shared priority cutoff, anchor count
and distance in R, baseline enrollment, grid padding and subset-selection policy.
Enrollment combines weighted selection with point-coverage witnesses and compact
local subsets. Signing searches regional subsets lazily and stops at the first
member. The [combination](feature-combinations.md) and
[signing](fingerprint-signing.md) guides define the algorithms and failure cases.
Earlier profiles require a new enrollment.

## Bounds and failure semantics

`Limits` requires explicit nonzero maxima for candidate attempts, candidate bytes, public-key bytes, signature bytes, framed action bytes and framed context bytes. These are software resource limits, not biometric acceptance criteria. Context/action limits include their domain and length framing. Fingerprint callers supply their priority cutoff and search policy explicitly; the examples' defaults are not security guarantees.

Every candidate processed counts toward the attempt limit, including duplicates. Enrollment must exhaust the complete stream before committing it. Exceeding the limit fails the whole enrollment; the library never commits a truncated set. One extra iterator item may be requested to determine that a stream exceeds the limit; it is not passed to key derivation. Algorithm implementations must also bound their own preparation and iterator work, because the core cannot interrupt arbitrary synchronous computation inside a provider.

Signing stops after finding one enrolled member. Exhaustion without a match is `CandidateNotEnrolled`; exceeding the search budget is a distinct `LimitExceeded` error. Neither result changes the enrolled set. Provider failures preserve their operation stage and original error source. Provider errors and application logging must not contain secrets.

## Commitment and action binding

The Merkle baseline sorts and deduplicates context-bound leaf hashes, duplicates the last node at odd levels, and uses distinct leaf/node/root hash domains. The root commits to both the top digest and the number of distinct leaves. Proof validation checks the count, index, exact depth, duplicated siblings and root. The retained tree contains hashes rather than a stored list of private keys or raw public-key bytes. Hashes/indexes still require an explicit privacy and backup policy.

The enrollment context binds network, verifier, subject, enrollment identifier and revision, public salt, secret policy, measurement profile and crypto suite. The signing message additionally binds the root, leaf count and complete action: operation, payload bytes, nonce and expiration. All variable-length fields use explicit lengths. These internal versioned encodings are a changeable prerelease baseline, not a stable storage or network wire format.

The application defines canonical operation payloads and a consistent unit for expiration/current time. `now >= expires_at` is expired. `verify` takes the expected action separately, rejects revoked/stale enrollments and never obtains its trusted root from the submitted package. A successful `VerifiedAuthorization` reports cryptographic evidence only. It does not consume a nonce, grant a spending allowance, or prove that a live capture or trustworthy approval screen was used.

## Integration and remaining work

The core is independent of NEAR. The [NEAR contract integration](near-sdk.md) supplies owner-authorized commitment registration/replacement/revocation, read-only signature verification and optional one-time acceptance with durable nonce state. Both verification methods return `Result<()>`; neither executes business actions. The application stores the registration, defines canonical signed data, enforces permissions and executes its own logic. The [NEAR example](../examples/near/README.md) demonstrates signed standard token transfers with a pending state and bounded callback; failed external calls retain the consumed nonce. Custody and retry policy belong to that example, not to the SDK. No deployment is implied.

Canonical container-v2 `.grx` persistence covers public context, optional hash-bound profile parameters and retained Merkle hashes. Fingerprint consumers require the canonical parameter record; generic constant profiles may omit it. Protected model storage, encrypted backups and recovery authority remain application work. The package does not serialize an arbitrary `MeasurementEngine` model or captured features. Container-v1 files are unsupported by the current decoder; generic authorization verification with a retained original trusted commitment remains separate from loading an old file.

Core tests cover software invariants and library composition. They do not measure real-scan repeatability, false acceptance, entropy or resistance to biometric attacks. See the [recorded descriptor limitation](near-sdk.md#biometric-security-boundary). The explicitly enabled NEAR sandbox test additionally exercises contract execution locally; it does not establish production network deployment or production gas limits. Those require separate measurement and integration validation.

## Static fingerprint report

Install `uv` alongside the pinned Rust toolchain, then run from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/core/tests/assets/test-fingerprint.png
```

The managed bundle is `reports/fingerprint/`. Start with `overview.png`; detailed
figures cover width, normalization, clarity, contours and feature evidence.
The [report guide](../tools/fingerprint-reports/README.md) defines its contents,
colors, display options and publication safeguards, including the current refresh
schema limitation. Generated content is ignored by Git.

## Change discipline

Maintain documentation, interface text, runtime messages and report captions in English.

Keep pipeline, mechanism and primitive responsibilities distinct. Prefer clear function names and cohesive modules, and use blank lines between logical segments. Refactor when current requirements outgrow a structure; remove unused code and obsolete fields. Before the first release, correct architecture takes priority over compatibility with old internal APIs. Add abstractions and package boundaries only when current responsibilities justify them.

Keep maintained tests next to the behavior they verify or in the crate's `tests/` directory. Add regressions for actual failure modes. Do not add production fallbacks that silently accept invalid proofs, widen enrollment, bypass missing algorithms or hide budget failures. Update the relevant API documentation and tests together with changed contracts.
