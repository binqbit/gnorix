# Development

## Workspace and setup

The repository uses a Rust 2024 Cargo workspace with one reusable library at `crates/gnorix`. The package boundary supports consumption by applications and future integrations. Add another package only when it has a concrete independent responsibility.

Install Rust **1.99.1** with Cargo, rustfmt and Clippy. `rust-toolchain.toml` selects that version for rustup users. With rustup:

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
cargo fetch --locked
```

On a system-managed toolchain, including NixOS, provide the same tools through the system's package configuration and check `rustc --version` and `cargo --version`. The library needs no scanner, blockchain node, credentials or database to build and test. Keep `Cargo.lock` in version control for reproducible repository checks. Package publication is disabled during this foundation stage.

The production dependencies are [RustCrypto SHA-2](https://docs.rs/sha2/0.11.1/sha2/) for complete SHA-256 digests and [zeroize](https://docs.rs/zeroize/1.9.1/zeroize/) for owned sensitive buffers and signing-key disposal contracts. These dependencies do not select a biometric KDF or signature suite.

## Routine checks

Run from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
cargo test --workspace --release --locked
cargo doc --workspace --no-deps --locked
cargo run -p gnorix --example merkle --locked
```

Use `RUSTDOCFLAGS="-D warnings"` for documentation checks on a POSIX shell. Cargo writes build output to the ignored root `target/` directory. The GitHub Actions workflow checks formatting, Clippy and documentation, and declares test jobs for Linux, macOS and Windows. A configured workflow does not itself establish that those hosted runs have passed.

## Implemented responsibilities

| Component | Location | Responsibility |
|---|---|---|
| Acquisition input | `src/biometrics/capture.rs` | Owned grayscale pixels, optional valid-region mask and scale; shape and numeric validation; redacted diagnostics. |
| Grid mathematics | `src/biometrics/geometry.rs` | Explicit triangular positions, Euclidean-nearest nodes and six-neighbor coordinates, with overflow/finite-value checks. |
| Fingerprint extraction | `src/biometrics/fingerprint/` | Native per-capture image stages, topology-preserving skeleton tracing, curve geometry and diagnostic output. |
| Regional subsets | `src/biometrics/fingerprint/combinations/` | Surviving feature projection, nine expandable circular pools and bounded weighted point combinations. |
| Fresh-image search | `src/biometrics/fingerprint/combinations/search.rs` | Global ranked subsets of n–2n surviving scored points, lazy baseline grid candidates and bounded search work. |
| Grid figures | `src/biometrics/fingerprint/shapes/` | Convex-hull frames, measured-width snapping, fixed uncertainty padding and complete positional traversal. |
| Measurement boundary | `src/biometrics/mod.rs` | Algorithm-owned reconstruction model and fallible candidate streams. |
| Crypto boundary | `src/crypto.rs` | Explicit KDF/signature suite, secret input handling and candidate-key operations. |
| Enrollment | `src/enrollment.rs` | Bind profile and context, prepare a model, process the complete candidate set, retain its commitment/model/hash tree. |
| Enrollment packages | `src/package.rs`, `src/package/` | Bounded canonical `.grx` storage of enrollment context and leaf hashes, validated tree reconstruction and public-key membership lookup. |
| Signing and verification | `src/authorization.rs` | Reconstruct a member, sign an exact action, and verify against trusted application state. |
| Commitment | `src/merkle.rs` | Deterministic SHA-256 tree, indexed membership lookup and strict proof verification. |
| Limits and errors | `src/policy.rs`, `src/error.rs` | Caller-selected software bounds, per-enrollment secret policy and typed failures with algorithm error sources. |
| Byte framing | `src/encoding.rs` | Bounded, unambiguous field framing for contexts and signed messages. |

Locations in this table are relative to `crates/gnorix`. High-level functions call named mechanisms; the mechanisms delegate hashing, framing and algorithm operations to their respective components. The pipelines are `enrollment::enroll` / `enroll_candidates`, `authorization::sign` / `sign_candidates` and `authorization::verify`.

### Fingerprint implementation layout

The public entry points and types are exported by `src/biometrics/fingerprint/mod.rs`. Implementation modules own their data and operations together:

| Location within `src/biometrics/fingerprint/` | Responsibility |
|---|---|
| `pipeline.rs` | Compose measurement, extraction, contour reconstruction and feature assessment; assemble the capture result. |
| `map.rs`, `graph.rs` | Binary maps, boundary provenance, traced graph data, shared incidence and connected-component traversal. |
| `scale.rs`, `scale/`, `full_scale.rs`, `full_scale/` | Bootstrap search, grayscale profiles, complete ridge-width measurement and local measurement support. |
| `enhancement/` | Local normalization, intensity filtering, orientation tensors and reusable directional responses. |
| `cleanup/`, `smoothing/`, `repair/` | Source-supported cleanup, constrained smoothing and gap reconstruction. Repair separates cubic construction, obstruction checks and graph assembly. |
| `features/` | Source probes, geometric evidence and stress correspondence. Ending assessment borrows unchanged paths and retains only the clipped remainder of its own edge. |
| `combinations.rs`, `combinations/` | Shared candidate projection and lazy weighted subset enumeration; regional enrollment selection/deduplication and global fresh-image signing search. |
| `shapes.rs`, `shapes/` | Prepare oriented figures, preserve staggered-grid phase, stream positional assignments and hash canonical integer-coordinate descriptors. |

Feature assessment separates local evidence (`features/evidence.rs`), extended source-backed structure (`features/context.rs` and its tracing/probe modules), threshold correspondence (`features/stability/`) and bounded crowding (`features/neighborhood.rs`). The orchestration combines these independent measurements; the report adapters only serialize and validate them. All source/graph observation preserves the original topology. Context traversal, sampling and neighborhood work have explicit bounds.

The fingerprint example is an adapter: `examples/fingerprint.rs` coordinates input and extraction, `examples/fingerprint/images.rs` exports image stages, and `examples/fingerprint/snapshot.rs` assembles the diagnostic JSON with the stage-specific exporters. Image serialization and report layout remain outside the library.

`examples/feature_combinations.rs` runs extraction and [regional subset generation](feature-combinations.md), with explicit example environment controls and JSON output. Both examples use `examples/support/image_input.rs` for bounded opaque-image decoding. Subsets contain capture-local feature IDs; the later grid and enrollment stages encode positions independently of these IDs and derive keys.

`examples/feature_grid.rs` orchestrates the [complete grid variants](grid-shapes.md) stage and its environment controls. Its `feature_grid/input.rs` module reads the bounded combination export; `feature_grid/output.rs` streams bounded JSON and removes partial outputs if traversal or writing fails. File I/O stays outside the library.

`examples/enroll_grid.rs` consumes that same combination input, streams grid candidate hashes through the shared enrollment pipeline, and writes a verified [`.grx` package](grx-packages.md). It supplies an explicit HKDF-SHA256/Ed25519 provider through development dependencies; the library retains its caller-selected crypto boundary. Both grid examples use `examples/support/grid_config.rs` for their controls.

`examples/fingerprint_signing.rs` extracts a fresh image, searches globally ranked point subsets and signs with the first member of an existing package. Its `verify` command checks the public authorization against a separately supplied trusted package and expected action. [The signing guide](fingerprint-signing.md) documents controls, evidence and crop results. Shared example modules under `examples/support/` own image decoding, combination/grid controls, enrollment profile, crypto provider, secret input, bounded JSON and exclusive file output; file I/O remains outside the library.

In `tools/fingerprint-reports/gnorix_reports/`, `snapshot.py` loads and validates native bundles, `validation.py` supplies shared JSON primitives, and stage modules validate their own evidence. `structure.py` coordinates rendering; the rendering modules draw already validated measurements. `cli.py` runs extraction and publishes the complete report directory. Report filenames and native raster filenames each have one maintained inventory shared by their consumers.

## Connecting algorithms

The [fingerprint extractor](fingerprint.md) is available as a per-capture component. Its common length unit is an arithmetic ridge-width mean from a full raw-centerline traversal, with explicit measured/unmeasured coverage. It returns observed ridge geometry, an intermediate graph with source-supported short-branch and narrow-loop decisions, and reconstructed smooth contours with source-pixel coordinates and explicit provenance. Separate fingerprint components select subsets, normalize them to the grid and encode candidates; registration of whole captures and calibrated biometric tolerances remain separate work.

Implement `MeasurementEngine` for the selected measurement strategy. It supplies a profile identifier, builds its own reconstruction model, enumerates all accepted enrollment candidates, and reconstructs candidates from a new capture under that same model. Its identifier must cover every setting that affects interpretation or candidate bytes.

The engine owns alignment, descriptor extraction, fusion, quantization, local uncertainty, missing-point rules and point ordering. It can organize these internally once the algorithms are selected. The core does not fix whether images are combined before feature extraction or features are extracted before alignment/fusion. It does not derive tolerances from the grid's mathematical spacing.

Implement `CryptoProvider` for the selected KDF and signature suite. Its identifier must cover the algorithms, parameters and key/signature encodings. `derive_key` receives canonical candidate bytes, the full framed enrollment context and the optional independent secret. The implementation must bind all required inputs, deterministically reproduce the same key/public-key encoding for identical inputs, and follow its crypto library's valid-key rules. Signing keys implement `ZeroizeOnDrop`; implementations remain responsible for intermediate buffers and copies. There is no default or placeholder production provider.

`SignatureVerifier` is the narrower public verification interface. `CryptoProvider` extends it, while `authorization::verify` requires only `SignatureVerifier`. A verifier or future NEAR adapter therefore implements no key-derivation or signing stubs.

The enrollment chooses `SecretPolicy::Absent` or `SecretPolicy::Required`. Supplying a secret to an absent-secret enrollment, or omitting a required one, is rejected. An empty secret is rejected; no minimum entropy or password-strength rule is inferred from byte length. `Candidate`, `SecretInput` and pixel buffers clear their owned bytes on drop. That does not erase caller copies, arbitrary algorithm models, registers or the whole device's memory.

Use these public entry points in order:

1. Call `enrollment::enroll` with the engine, provider, captures, explicit `EnrollmentSpec`, optional secret and `Limits`.
2. Register `enrollment.commitment()` through the application's owner-authorized registration flow. Protect the local enrollment and algorithm-owned reconstruction model separately.
3. Obtain approval for the exact `Action`, then call `authorization::sign` with a fresh capture.
4. Read the expected enrollment commitment/status from a trusted registry and call `authorization::verify` with the expected action and trusted current time.
5. Apply the application's permissions and durable replay/execution policy before performing the action.

The [lifecycle integration tests](../crates/gnorix/tests/lifecycle/main.rs) demonstrate this wiring with deliberately nonproduction measurement and crypto doubles. The suite groups enrollment, verification and failure behavior into cohesive modules with shared fixtures. They are test fixtures, not algorithms to copy into an application.

For an existing fingerprint `.grx` package, the alternative path is `fingerprint::extract` → `fingerprint::signing_candidates` → `authorization::sign_candidates`, followed by the same trusted verification. This path uses the enrolled profile and padding, stops at the first matching public key and leaves the accepted set unchanged.

## Bounds and failure semantics

`Limits` requires explicit nonzero maxima for candidate attempts, candidate bytes, public-key bytes, signature bytes, framed action bytes and framed context bytes. These are software resource limits, not biometric acceptance criteria. Context/action limits include their domain and length framing. No scanner thresholds, candidate-count defaults or quality cutoffs are embedded in the library.

Every candidate processed counts toward the attempt limit, including duplicates. Enrollment must exhaust the complete stream before committing it. Exceeding the limit fails the whole enrollment; the library never commits a truncated set. One extra iterator item may be requested to determine that a stream exceeds the limit; it is not passed to key derivation. Algorithm implementations must also bound their own preparation and iterator work, because the core cannot interrupt arbitrary synchronous computation inside a provider.

Signing stops after finding one enrolled member. Exhaustion without a match is `CandidateNotEnrolled`; exceeding the search budget is a distinct `LimitExceeded` error. Neither result changes the enrolled set. Provider failures preserve their operation stage and original error source. Provider errors and application logging must not contain secrets.

## Commitment and action binding

The Merkle baseline sorts and deduplicates context-bound leaf hashes, duplicates the last node at odd levels, and uses distinct leaf/node/root hash domains. The root commits to both the top digest and the number of distinct leaves. Proof validation checks the count, index, exact depth, duplicated siblings and root. The retained tree contains hashes rather than a stored list of private keys or raw public-key bytes. Hashes/indexes still require an explicit privacy and backup policy.

The enrollment context binds network, verifier, subject, enrollment identifier and revision, public salt, secret policy, measurement profile and crypto suite. The signing message additionally binds the root, leaf count and complete action: operation, payload bytes, nonce and expiration. All variable-length fields use explicit lengths. These internal versioned encodings are a changeable prerelease baseline, not a stable storage or network wire format.

The application defines canonical operation payloads and a consistent unit for expiration/current time. `now >= expires_at` is expired. `verify` takes the expected action separately, rejects revoked/stale enrollments and never obtains its trusted root from the submitted package. A successful `VerifiedAuthorization` reports cryptographic evidence only. It does not consume a nonce, grant a spending allowance, or prove that a live capture or trustworthy approval screen was used.

## Integration and remaining work

The core is independent of NEAR. A future adapter must supply trusted registry state, canonical actions, permissions, replay reservation and asynchronous pending/success/failure handling. Root registration, replacement and recovery need their own authority. No contract API, custody model, callback/retry policy or deployment is implied by the library interfaces.

Canonical `.grx` persistence covers public context and retained Merkle hashes. Protected model storage, encrypted backups and recovery authority remain application work. The package does not serialize an arbitrary `MeasurementEngine` model; applications retain the required reconstruction settings separately.

Tests cover software invariants and library composition. They do not measure real-scan repeatability, false acceptance, entropy, resistance to biometric attacks, or on-chain gas/execution. Those require the separate measurement and integration validation plans.

## Static fingerprint report

The [fingerprint report tool](../tools/fingerprint-reports/README.md) runs the native Rust extractor and publishes its actual measurements and geometry as one PNG report bundle. Install `uv` alongside the pinned Rust toolchain, then run from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/gnorix/tests/assets/test-fingerprint.png
```

The complete report lives in `reports/fingerprint/`. Each successful run refreshes this same directory. `overview.png` provides the main overview; detailed PNGs show observed structure, full-source width and coverage, bootstrap measurements, local normalization, source-based clarity, reconstructed contours and feature strength. Geometry visibility follows the native segment clarity while unsupported measurements remain explicit. `feature-strength.png` shows all primary candidates, including weak ones, using extended source-confirmed context, local source/geometry and eight controlled extraction variants. Intrinsic strength and crowding-adjusted priority are separate; marker brightness follows final priority. These diagnostic ranks do not remove candidates or choose contour repairs. The generated index links every figure. Source rasters and `features.json` remain together under `data/`.

`structure-overlay.png` is the main view after cleanup and reconstruction; `feature-strength.png` retains every original candidate for diagnosis. Surviving unchanged feature types display their original evidence priority, while converted types or degrees remain unassessed. Both figures use the same fixed 0–1 priority color scale: stronger candidates are brighter, with opacity `0.4 + 0.5 × sqrt(priority)` in the 40–90% range. Unknown priority is gray at 40%; the structural overlay preserves hollow markers with × for unknown continuation. Line visibility follows local clarity in its own fixed 40–90% range. Other maps and annotations default to 50% opacity through `--overlay-alpha`; `--dpi` controls PNG resolution. Both options affect presentation only. The tool has a single native pipeline; measurement algorithms remain in Rust. The report guide explains publication safeguards, failures and checks. Generated content is ignored by Git.

## Change discipline

Keep pipeline, mechanism and primitive responsibilities distinct. Prefer clear function names and cohesive modules, and use blank lines between logical segments. Refactor when current requirements outgrow a structure; remove unused code and obsolete fields. Before the first release, correct architecture takes priority over compatibility with old internal APIs. Add abstractions and package boundaries only when current responsibilities justify them.

Keep maintained tests next to the behavior they verify or in the crate's `tests/` directory. Add regressions for actual failure modes. Do not add production fallbacks that silently accept invalid proofs, widen enrollment, bypass missing algorithms or hide budget failures. Update the relevant API documentation and tests together with changed contracts.
