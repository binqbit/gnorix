# NEAR transaction signing demo

This example holds standard NEP-141 tokens on a smart-wallet account. Gnorix
approves an exact transfer; a relayer submits it, and the wallet verifies the
signature and Merkle proof before calling the token's `ft_transfer`. The token
needs no Gnorix-specific methods. Its balance belongs to the **wallet contract's
account**, not to a separate management account or an arbitrary user's wallet.

This is a disposable private NEAR sandbox. Successful execution does not prove
biometric identity; read the [recorded descriptor limitation](../../docs/near-sdk.md#biometric-security-boundary)
before adapting it for real assets. The page includes the prototype notice and
**How it works** explanation.

## Open the demo

Run all commands below from the repository root. For image analysis without
blockchain transactions, use the [Fingerprint Lab](../fingerprint-lab/README.md);
the [local testing guide](../../docs/local-testing.md) helps choose a workflow.

### One-time setup

Install the native build prerequisites for your OS from
[workspace setup](../../docs/development.md#workspace-and-setup), then install
and select the pinned Rust tools:

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
export PATH="$(dirname "$(rustup which --toolchain 1.99.1 rustc)"):${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
```

The first run needs internet access for dependencies, the Wasm target and the
NEAR sandbox binary. The app installs a missing Wasm target for Rust 1.99.1.
Automatic sandbox downloads support Linux x86_64/aarch64 and Apple Silicon macOS.
Use a Linux VM on Intel Macs and WSL on Windows. No Docker, Node.js, separate RPC
service or `cargo-near` is required for this web entry point.
A native build environment is required for the Cargo command; runtime contract
builds also require rustup unless you supply prebuilt contracts. Keep the same
toolchain selected in each new terminal.

### Start with one command

Use the same five-point profile as the supplied public service:

```sh
cargo run -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked -- --points-per-combination 5
```

Keep the terminal running. Once it prints the address, open
**http://127.0.0.1:8780**. The page shows startup progress while Rust builds both
contracts, starts one private sandbox and analyzes the two source images. Each
browser session then generates its own Merkle enrollments and receives two
funded wallets and a token contract. Wait for **Sandbox connected**, both wallets
and their **1,000 GXT** balances before signing. Account creation, contract
deployment and funding are automatic; no browser wallet, public-chain account,
faucet or separate deployment command is needed.

Omitting `--points-per-combination 5` uses the standalone **3–6** point range.
The flag fixes the size of each combination, not the total number of detected
features. It requires at least three points and changes both enrollment and
signing. Changing it requires a restart and fresh enrollments.

### Make and verify a transfer

1. Select **Wallet 1** and enter **10** as the amount. The recipient is
   **Wallet 2**. The fingerprint, eligible points, balance and registered root
   belong to the selected sending wallet.
2. Click **Approve & sign**. One new random circle covers at least a third of the
   confirmed pattern. Only its unchanged pixels enter fresh feature extraction;
   the surrounding blur is visual. A failed capture stays visible and requires
   another explicit click. No hidden crop retry or full-image fallback occurs.
3. A successful reconstruction highlights five matched points with the command
   above and shows **Signature ready** and the actual **Signing public key**.
   No tokens have moved yet. Inspect **Signature & Merkle proof**, then click
   **Submit to NEAR** once.
4. The page displays the transaction hash, receipt/callback outcome, actual gas,
   balances and a link back to the proof. On success, this first transfer leaves
   **990 GXT** in Wallet 1 and **1,010 GXT** in Wallet 2. The attached allowance is
   **100 Tgas**; measured consumption is shown separately. Switch to Wallet 2
   and repeat with 10 GXT to return both balances to 1,000, or edit the next
   transfer directly.

If observation is uncertain, **Check transaction status** queries the original
hash without broadcasting again. A new transfer stays blocked until that result
is resolved. A search-budget failure means **incomplete**, not a confirmed
fingerprint mismatch. Visible points alone do not guarantee an enrolled subset
exists inside the selected crop.

### Stop, reset and troubleshoot

Press **Ctrl+C** in the launching terminal and wait for shutdown. It stops the
service and its managed sandbox. Rerun the same command for a fresh chain,
wallets, enrollments and balances; a page reload alone resumes the current
browser session. Restarting discards all sessions on this local service.
Build artifacts stay cached under `target/`; no `cargo clean` is needed.

| What you see | What to do |
| --- | --- |
| Startup progress or disabled signing controls | Wait for contract builds, image analysis and enrollment. The first run takes longer than subsequent builds. |
| `cannot listen on http://127.0.0.1:8780` | Another process owns the port. Add `--port 8781` after the final `--` in the launch command and open that port. |
| `Sandbox setup failed` | Read the page's error and terminal output. Check the Rust/native prerequisites and first-run downloads, then restart after fixing the cause. |
| `Capture failed` | Read the reason. Too few usable points, no enrolled match and an exhausted search budget are different outcomes. Click **Approve & sign** for a new random attempt; a new attempt is not guaranteed to match. |
| `Signature ready`, balances unchanged | The transfer is only prepared. Click **Submit to NEAR** to execute it. |
| `Outcome unconfirmed` | Use **Check transaction status**; it checks the existing hash without sending another transfer. |
| `Session account cleanup failed` during Ctrl+C | The terminal signal can stop sandbox RPC before account retirement finishes. Shutdown still discards the disposable local chain; wait for the process to exit before restarting. |

### Runtime options

| Setting | Meaning |
| --- | --- |
| `--port 8781` | Change the loopback HTTP port; default is 8780. |
| `--public-origin https://gnorix.online` | Admit that exact public Host/Origin through an HTTPS reverse proxy. |
| `--points-per-combination 5` | Use exactly five points for enrollment and signing; integer ≥3. Without this flag the range is **3–6**. |
| `GNORIX_DEMO_CONTRACTS_DIR` | Directory containing `gnorix_near_wallet.wasm` and `gnorix_near_token.wasm`; skips startup compilation. |
| `NEAR_SANDBOX_BIN_PATH` | Select a compatible preinstalled NEAR sandbox binary; current version is **2.14.1**. |

See [deployment](deploy/README.md) for a self-contained release bundle, systemd
and nginx. Local accounts exist only in the managed sandbox, not on public
testnet or mainnet.

## Browser sessions and data lifetime

Only **GET `/api/state`** creates or resumes a session. Its opaque cookie is
`HttpOnly`, `SameSite=Strict`, host-only and scoped to `/api`; the configured
public HTTPS origin adds `Secure`. Local loopback HTTP remains usable. Another
tab in the same browser shares its session; another browser/profile receives
independent wallets, token contract, relayer, roots, nonces and active attempt.
The JSON CSRF token is separate from the cookie. Writes require both from the
same session, and capture URLs resolve within that session only.

The lifetime is **3,600 seconds from creation**; polling never renews it. Expired
cookies receive HTTP 410 on writes and capture reads. Missing cookies receive
403. A subsequent GET `/api/state` creates a new session; static assets and HEAD
requests never allocate wallets. At **16 registered sessions**, new allocations
receive 503 with `Retry-After: 60`; existing sessions are not evicted. Expired
entries awaiting the next cleanup tick can briefly count toward this limit.

Expiry is checked every second. Access stops at the deadline, while teardown
cancels the session task and clears its enrollment, capture and approval state.
An already running blocking computation releases owned data at its next
cancellation checkpoint; this is not instantaneous memory erasure. One permit
serializes CPU-heavy enrollment and signing jobs across all sessions. The two
public fixture analyses and their enrollment candidate descriptors are cached
once alongside the shared sandbox. Session-specific enrollments and fresh-scan
reconstruction are separate; private keys are not shared or cached.

Account retirement first revokes registrations and resolves accepted transfers
before deleting disposable accounts. Its operation has a **45-second limit**;
failed or unresolved cleanup is reported and accounts can remain on the private
chain. Process shutdown gives all session cleanup a shared **15-second grace**
before disposing the sandbox. Expiry cannot undo an already submitted transfer.
The chain retains transaction/receipt history until the sandbox is discarded.

The app writes no signature archive or per-attempt metadata files. Current
public enrollment material, capture, proof and result live in session memory;
a new approval replaces the prior attempt. Private signing keys are transient,
not cached. Fresh contexts and salts make roots session-specific even for the
same bundled image. Cookies isolate this demonstration's state; they are not
real-user authentication or a biometric identity guarantee.

## Fingerprint policy and fixtures

The two [synthetic scans](assets/README.md) are 241 × 298 pixels. They are test
inputs with no claimed real person, scanner calibration or uniqueness. The core
fixture and optional SOCOFing Lab gallery are separate inputs.

| Parameter | Standalone default | Supplied public service |
| --- | --- | --- |
| Points per combination | 3–6 | Exactly 5 |
| Minimum feature priority | 0.1 | 0.1 |
| Anchor count / pairwise distance | At least 3 / at least 5R | Same |
| Search-region radius | 30R | Same |
| Positional uncertainty | 1 neighbor step during signing | Same |
| Signing work budget | 300,000,000 work units | Same |

R is the measured mean dark-ridge width. Every anchor pair must satisfy the
minimum distance; other points lie inside or on the anchors' convex hull. Equal
distances are not required. Enrollment stores rounded baselines only. Signing
checks each ranked subset's positional variants lazily, stopping at the first
member. Work includes geometry and descriptor processing, not just key attempts.

Example profile **v10** binds these settings and the bounded, priority-weighted
subset selection with local coverage groups around features and between pairs of
features. The 5R anchor distance leaves more room for extraction variation than
the previous 6R demo setting; the comparison itself remains strict. See [feature combinations](../../docs/feature-combinations.md)
and [fresh-image signing](../../docs/fingerprint-signing.md) for the shared
algorithms. Earlier profiles need new enrollment. The Lab has different defaults
and optional image artifacts; its settings do not control this NEAR service.

The demonstration uses the example HKDF-SHA256/Ed25519 provider without an
independent secret. This is a concrete test configuration, not a recommended
production biometric key-derivation policy.

Each click chooses a center uniformly from confirmed-pattern pixels, then the
smallest pixel radius covering at least one third of them, including boundary
ties. This is pattern coverage, not anatomical finger area or frame area. A fresh
random seed drives each attempt. The preview keeps the original pixels, with
smooth browser blur outside the exact analysis circle and no extra artifacts.
Feature IDs belong to the new capture, not to enrollment's point list.

## Contract integration

The [SDK guide](../../docs/near-sdk.md) defines registration ownership, canonical
action signing, transport conversions and replay protection. The wallet-specific
interface below supplies transfer policy; the SDK itself does not move tokens.

Deploy and initialize together in a wallet-account-authorized transaction.
`new(owner_id)` is private and runs once. A full-access account key retains its
normal authority to replace code; Gnorix does not restrict it. Register the
wallet and recipient with the token's `storage_deposit`, fund the wallet's token
balance, and leave native NEAR for storage and outgoing one-yocto deposits.

Enroll offchain with network `near:<chain_id>`, verifier equal to the wallet
account, subject equal to `owner_id`, and initial revision 1. The owner registers
the public commitment, paying measured storage growth; excess NEAR is refunded
asynchronously. The contract receives no image, private key or full Merkle tree.

| Method | Behavior |
| --- | --- |
| `new(owner_id)` | Private initialization. |
| `register_enrollment(commitment)` | Owner only; payable storage registration. |
| `replace_enrollment(commitment)` | Owner only; next revision, same enrollment ID; refuses a pending transfer. |
| `revoke_enrollment()` | Owner only; disables new approvals and retains replay history. |
| `get_owner()`, `get_enrollment()`, `get_transfer_status()` | Read management identity, enrollment and latest transfer outcome. |
| `verify_transfer(transfer, authorization)` | Read-only verification; does not consume a nonce or check token balances. |
| `transfer(transfer, authorization)` | Any relayer; zero attached NEAR; accepts once and dispatches the token call. |
| `on_transfer(revision, nonce)` | Private runtime callback. |

`Transfer` contains `token_id: AccountId`, `receiver_id: AccountId`,
`amount: U128` and `memo: Option<String>`. The amount is a positive decimal JSON
string in base units; memo is at most 256 UTF-8 bytes. The wallet cannot be its
own token or recipient. Operation **`wallet.ft_transfer.v1`** signs the Borsh
encoding of `(token_id, receiver_id, amount, memo)` in that order. `None` and
`Some("")` differ. Use [`Transfer::signing_payload`](src/transfer.rs), then the
[core signing/conversion flow](../../docs/near-sdk.md#core-signing-and-transport).
The contract reconstructs this payload from the actual method arguments.

Token storage registration and Gnorix registration are independent. External
`storage_unregister` cannot reset the wallet's root, revision or nonce; the
standard token sends no notification to this wallet. Use explicit revoke, then
replacement with the next revision to reactivate.

### Asynchronous results

The relayer attaches **zero NEAR** and **100 Tgas**. The wallet pays the token
call's one yoctoNEAR from its own balance and reserves 20 Tgas for `ft_transfer`
and 10 Tgas for its callback. The accepting receipt checks the remaining gas
before dispatch. A token requiring more than the reserved gas can fail.

Only one transfer may be pending. Acceptance consumes the nonce and records
`pending`; the callback records `succeeded` or `failed` for that revision and
nonce. External failure does not restore a consumed nonce: correct the cause
and sign again with the next nonce. Rejection inside the accepting receipt rolls
back that receipt's changes instead.

A failed callback can leave `pending` even after the token call completed.
Inspect receipts and balances; do not resend blindly or reset the nonce. There
is no automatic pending-state reset. Revocation prevents new authorizations but
cannot undo dispatched promises; replacement waits for no pending transfer.
Success means the selected token contract's receipt succeeded, not that an
arbitrary contract has been audited as a trustworthy token.

## Verification

The commands below start and stop their own test environment. A separately
running web demo is not required. Ordinary workspace tests do not enable the
`demo` feature or run tests marked `ignored`.

For one automated end-to-end check of fresh fingerprint capture, signing and
token transfers in both directions, use:

```sh
cargo test -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked fingerprint_approvals_execute_real_transfers_between_both_wallets -- --ignored --nocapture --test-threads=1
```

This scenario uses repeatable capture seeds, checks the actual balances and
retains an expected failed capture. It automatically builds contracts and starts
a private sandbox, using the same prerequisites as the browser demo. It does not
need `cargo-near`.

For all ordinary demo tests, then the complete opt-in extraction/sandbox suite:

```sh
cargo test -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked
cargo test -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked -- --ignored --nocapture --test-threads=1
```

Ordinary tests cover HTTP cookie/CSRF boundaries and request validation without
starting a sandbox. The ignored suite builds contracts automatically unless supplied through
`GNORIX_DEMO_CONTRACTS_DIR`. It covers fresh fingerprint extraction, actual
transfers, default and five-point profiles, failed captures, replay/async failure,
isolated enrollments and expiry cleanup. Public random-capture tests must retain
failed attempts rather than replacing them silently. Passing tests establish
their specific software checks, not biometric false-acceptance rates.

### Standalone contract regression suites

These check wallet authorization, replay protection, asynchronous transfers and
standard token behavior using small deterministic key fixtures. They do not run
image extraction; use the demo check above to test the complete biometric path.
They also exercise a test-only token receiver. Unlike the web entry point, their
build script requires `cargo-near`.

On Ubuntu, install the native dependency used by `cargo-near` first:

```sh
sudo apt-get update
sudo apt-get install --no-install-recommends -y libudev-dev pkg-config
```

```sh
rustup target add --toolchain 1.99.1 wasm32-unknown-unknown
cargo install cargo-near --version 0.23.0 --locked
./examples/near/test.sh
```

The script emits `gnorix_near_wallet.wasm`, `gnorix_near_token.wasm` and
`gnorix_ft_test_receiver.wasm` under `target/near/`. Rerun an individual suite:

```sh
cargo test -p gnorix-near-wallet --features sandbox --locked --test wallet -- --ignored --nocapture
cargo test -p gnorix-near-wallet --features sandbox --locked --test fungible_token -- --ignored --nocapture
```

Rebuild only the wallet:

```sh
cargo near build non-reproducible-wasm --locked --no-abi --manifest-path examples/near/Cargo.toml --out-dir target/near --override-toolchain 1.99.1
```

## Files

- `demo/`: Rust HTTP service, session registry, fingerprint adapter and managed chain.
- `web/`: embedded English interface and public proof rendering.
- `assets/`: two synthetic fingerprint fixtures and provenance notes.
- `src/`: smart wallet and canonical transfer payload.
- `token/`: standard Gnorix Test Token (GXT), 24 decimals, without a Gnorix dependency.
- `tests/`: contract scenarios and the test-only transfer-call receiver.

The token uses `near-contract-standards` 5.30.2 for NEP-141 transfers, NEP-145
storage, NEP-148 metadata and events. Private initialization mints the fixed
supply to the initial holder; there is no later minting or admin transfer method.
The receiver fixture exercises normal `ft_transfer_call` refund paths. Neither
contract test fixture nor the web demo is a public-network deployment tool.
