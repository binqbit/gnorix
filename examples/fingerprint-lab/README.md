# Fingerprint Lab

A local Rust application for inspecting fingerprint features, enrolling a Merkle set and testing signatures from circular or altered captures. The server embeds its web interface and requires no frontend installation.

From the repository root:

```sh
cargo run -p gnorix-lab --release --locked
```

Or run `cargo run --release --locked` from this directory. Open **http://127.0.0.1:8765** and stop the application with Ctrl+C. Append `-- --port 8766` to choose another port.

The application has its own `Cargo.toml` and depends on the local `gnorix` library and `gnorix-example-support` package. It shares the workspace lockfile and `target/` directory. Build from the repository checkout: the bundled image comes from `crates/gnorix/tests/assets/test-fingerprint.png`.

- `src/` contains startup, HTTP handling, application state, capture processing and diagnostics.
- `web/` contains the embedded HTML, CSS, JavaScript and logo.
- `gallery.json` pins the optional five-image SOCOFing gallery.

The optional gallery loads from repository-root `test-data/fingerprint-gallery/`; `-- --gallery /path/to/gallery` overrides that location. Generated public packages and test records go to repository-root `reports/fingerprint-lab/`, regardless of the current directory.

Run the application tests:

```sh
cargo test -p gnorix-lab --release --locked
```

See the [Lab guide](../../docs/fingerprint-lab.md) for controls, workflow and validation limits, and the [gallery importer](../../tools/fingerprint-gallery/README.md) for optional research images.
