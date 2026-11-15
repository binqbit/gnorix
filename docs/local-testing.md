# Run and test locally

Choose the workflow by what you want to verify. Both web apps include test
fingerprints and their interface; neither needs a scanner, Docker, Node.js,
wallet credentials or a dataset download.

| Goal | Entry point | Result |
| --- | --- | --- |
| Inspect features, change thresholds, test fragments and artifacts | [Fingerprint Lab](#fingerprint-lab) | A local signature and Merkle proof, without a blockchain |
| Sign and execute token transfers | [NEAR demo](#near-transfers) | Real execution on a private local chain and changed test balances |
| Inspect image-processing stages | [PNG report](#png-report) | An overview and detailed images under `reports/fingerprint/` |
| Check code regressions | [Automated tests](#automated-tests) | Assertions with explicit coverage, separate from the interactive apps |

## One-time setup

Clone the repository, then complete the [toolchain and native dependency setup](development.md#workspace-and-setup):

```sh
git clone https://github.com/binqbit/gnorix.git
cd gnorix
```

The pinned Rust version is **1.99.1**. Linux needs a native build environment;
the NEAR dependency chain also uses CMake, pkg-config and OpenSSL headers.
The setup guide includes Ubuntu/WSL commands. NEAR's automatic sandbox download
supports Linux x86_64/aarch64 and Apple Silicon macOS; use WSL on Windows or a
Linux VM on Intel Macs. Python and uv are needed only for reports and dataset
tools; see [optional Python setup](development.md#optional-python-tools).

Run the following launch commands from the checkout root. The first build and
NEAR sandbox download need internet access and can take several minutes.
Each server keeps its terminal occupied. Stop it with **Ctrl+C**, or use
separate terminals to run both; they do not share enrollments or settings.

## Fingerprint Lab

```sh
cargo run -p gnorix-lab --release --locked
```

Open **http://127.0.0.1:8765** after the terminal prints the startup URL.
Start with the bundled image and defaults:

1. Select **Prepare features**, inspect the points, then **Create Merkle set**.
2. Open **Sign with a fragment**. Leave **Artifact severity** at **0** and move **Selected fragment radius** to its maximum for a full-image control.
3. Select **Prepare features → Sign message → Verify signature**. Inspect the matched points, member public key and proof.
4. Reduce or move the circle, prepare again and repeat. Add artifacts only after the clean control works. Change the message after signing and verify again to check that an altered message is rejected.

This app tests feature extraction and signing; it never sends a transaction.
It has one shared local workspace. Restarting clears its active state, while
generated public records and `.grx` packages remain in
`reports/fingerprint-lab/`; create a new enrollment after restart.
Defaults are **3–6 points, priority 0.5, three anchors separated by 6R**.
See [Lab controls and result meanings](fingerprint-lab.md) before changing them.

## NEAR transfers

Use the hosted demo's five-point configuration:

```sh
cargo run -p gnorix-near-wallet --bin gnorix-near-demo --features demo --release --locked -- --points-per-combination 5
```

After the terminal prints the startup URL, open **http://127.0.0.1:8780**.
The page opens while the backend builds both
contracts, starts the private sandbox, analyzes the bundled fingerprints and
prepares the browser's accounts and enrollments. Wait until both wallets show
**1,000 GXT** and **Approve & sign** is enabled. No manual contract deployment
or `cargo-near` installation is needed for this command.

1. Choose **Wallet 1**. The recipient is automatically **Wallet 2**; enter amount **10**.
2. Select **Approve & sign**. A new random circular capture is analyzed. On success, inspect the highlighted features and signing public key. Signing alone does not change balances.
3. Select **Submit to NEAR**. Wait for settlement: Wallet 1 should show **990 GXT**, Wallet 2 **1,010 GXT**. Open **Signature & Merkle proof** to inspect the authorization.
4. Switch to Wallet 2 and send 10 back through the same steps. Both balances return to 1,000 GXT.

Each approval is a new capture, and some fragments can fail. Visible points
alone do not guarantee an enrolled combination. A finished no-match, insufficient
geometry and an exhausted search budget are distinct outcomes; follow the
displayed reason. If submission is uncertain, use **Check transaction status**
to reconcile that transfer before creating another one.

This profile uses **five points, priority 0.1, three anchors
separated by 5R**. Without `--points-per-combination 5`, the standalone app uses
3–6 points. Different browser profiles have isolated wallets, balances and
enrollments in one sandbox; tabs in the same profile share a one-hour session.
Restarting resets the local chain and sessions. These are test tokens, not
public-network assets. See the [NEAR guide](../examples/near/README.md) for
session behavior, options and troubleshooting.

## PNG report

After [Python setup](development.md#optional-python-tools):

```sh
uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image crates/core/tests/assets/test-fingerprint.png
```

Open **`reports/fingerprint/overview.png`**. The same directory contains detailed
width, normalization, clarity, contour and feature images, plus diagnostics.
The command builds/runs the Rust extractor and renders PNGs; it starts no web
server and performs no transfer. Repeating it refreshes the managed report.
See [report outputs and checks](../tools/fingerprint-reports/README.md).
SOCOFing is an [optional research dataset](fingerprint-testing.md), not a setup
requirement.

## Automated tests

Use the smallest check covering what you changed. All commands run from the
checkout root; no manually running web server is required.

| Scope | Command | What it covers |
| --- | --- | --- |
| Core biometrics and signing | `cargo test -p gnorix-core --release --locked` | Native extraction, ranking, geometry, candidate search and protocol checks |
| Lab application | `cargo test -p gnorix-lab --release --locked` | HTTP, state, capture/artifact and resource behavior |
| Ordinary workspace | `cargo test --workspace --all-targets --locked` | Default-feature Rust tests; excludes opt-in sandbox and Python checks |
| NEAR demo without chain startup | `cargo test -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked` | Demo configuration, HTTP/session boundaries and supporting units; ignored chain tests are skipped |

To exercise the complete **fingerprint → signature → local NEAR transfer** path
in both directions with one command:

```sh
cargo test -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked fingerprint_approvals_execute_real_transfers_between_both_wallets -- --ignored --nocapture --test-threads=1
```

This starts and cleans up its own sandbox, builds contracts automatically and
checks balances. The remaining [NEAR demo integration tests](../examples/near/README.md#verification)
cover additional capture, session and failure scenarios. The separate
`./examples/near/test.sh` suite uses deterministic key fixtures to test contract
semantics; it requires the [additional cargo-near setup](../examples/near/README.md#standalone-contract-regression-suites)
and does not test fingerprint extraction.

For the complete validation set, follow [Development: routine checks](development.md#routine-checks),
the [report tool tests](../tools/fingerprint-reports/README.md#implementation-and-checks) and
[gallery importer tests](../tools/fingerprint-gallery/README.md#checks).
Passing these software tests does not establish biometric reliability or
cryptographic strength; see the [recorded boundary](near-sdk.md#biometric-security-boundary).

## Common first-run issues

| Symptom | Action |
| --- | --- |
| `cargo`, linker, CMake or OpenSSL build error | Complete [native/toolchain setup](development.md#workspace-and-setup); check that Cargo, rustc and Clippy use 1.99.1. |
| `Blocking waiting for file lock` | Another Cargo build is active. Wait for it rather than starting more copies. |
| Address already in use | Stop the old local process or append `-- --port 8766` for the Lab. For the NEAR command above, add `--port 8781` after its existing application flags. |
| Page opens but NEAR wallets are not ready | Keep the backend running; watch the displayed phase and terminal for first-build/download failures. |
| A fragment fails despite visible features | Check the reported outcome. Try the clean full-image control in the Lab; NEAR approvals use a fresh random capture on each attempt. Do not treat incomplete search as a completed mismatch. |
| Changing parameters invalidates preparation | Prepare again; changes to the enrollment policy require a new Merkle set. The Lab and NEAR demo have different defaults. |
