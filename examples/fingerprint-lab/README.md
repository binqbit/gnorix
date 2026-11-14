# Fingerprint Lab

A local Rust application for inspecting fingerprint features, enrolling a Merkle set and testing signatures from circular or altered captures. The server embeds its web interface and requires no frontend installation.

Install the pinned toolchain, then run from the repository root:

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
export PATH="$(dirname "$(rustup which --toolchain 1.99.1 rustc)"):${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
cargo run -p gnorix-lab --release --locked
```

Or run `cargo run --release --locked` from this directory. Open **http://127.0.0.1:8765** and stop the application with Ctrl+C. Append `-- --port 8766` to choose another port.

The application has its own `Cargo.toml` and depends on the repository-root `gnorix` library with its default `core` feature and the `gnorix-example-support` package. It shares the workspace lockfile and `target/` directory. Build from the repository checkout: the bundled image comes from `crates/core/tests/assets/test-fingerprint.png`.

- `src/` contains startup, HTTP handling, application state, capture processing and diagnostics.
- `web/` contains the embedded HTML, CSS, JavaScript and logo.
- `gallery.json` pins the optional five-image SOCOFing gallery.

The optional gallery loads from repository-root `test-data/fingerprint-gallery/`;
`-- --gallery /path/to/gallery` overrides that location. Generated public
packages and test records go to repository-root `reports/fingerprint-lab/`,
regardless of the current directory. Dataset images are optional local imports;
the bundled synthetic fixture works without them.

This is one shared local workspace, not the NEAR demo's expiring browser-session
service. It starts with 3–6 points per combination, minimum priority 0.5,
three anchors at least 6R apart, radius 30R and one uncertainty step. The UI
adjusts these parameters and can distort the signing image while retaining a
clean enrollment. Restarting clears active in-memory state; saved reports stay
on disk. The Lab neither runs a blockchain nor inherits the NEAR demo's
priority 0.1 or deployed five-point configuration.

Run the application tests:

```sh
cargo test -p gnorix-lab --release --locked
```

See the [Lab guide](../../docs/fingerprint-lab.md) for controls, workflow and
artifact reproduction, the [gallery importer](../../tools/fingerprint-gallery/README.md)
for research images, and [toolchain setup](../../docs/development.md#workspace-and-setup)
if system Cargo/Clippy versions differ. The [biometric security boundary](../../docs/near-sdk.md#biometric-security-boundary)
applies to successful Lab signatures too.
