# NEAR transaction signing demo

This example keeps a standard NEP-141 token balance on a NEAR smart-wallet
account. Its owner registers a Gnorix Merkle commitment and approves each exact
transfer offchain. Any relayer can submit that authorization; the wallet verifies
it and calls the token's ordinary `ft_transfer` method.

The token needs no Gnorix integration. The token balance belongs to the **wallet
contract's account**, not to a separate management account named by `owner_id`.
A standalone controller cannot spend tokens held by another NEAR account merely
by verifying that account's signature. Funding this wallet moves tokens into its
own balance. The included token demonstrates this without adding delegation
methods to the token.

## Open the demo

From the repository root, run:

```sh
cargo run -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked
```

Open **http://127.0.0.1:8780**. Rust starts the web server immediately and the page
shows preparation progress. It builds both contracts, starts a private NEAR
sandbox, deploys the token and two smart wallets, analyzes the two bundled test
fingerprints and registers their Merkle commitments. Each wallet starts with
**1,000 GXT**, checked against the token contract's actual state.

The machine needs Rust/rustup and a native build environment. The repository pins
Rust 1.99.1; the app installs its missing Wasm target automatically. The first run
needs internet access for Cargo dependencies, the Wasm target and the sandbox
binary. Use Linux or macOS with a supported NEAR sandbox binary; native Windows
users should run this example in WSL. No Docker, Compose, Node.js, separate RPC
service or `cargo-near` installation is required for this web demo.

1. Select **Wallet 1** or **Wallet 2**. Its fingerprint, eligible feature points
   and on-chain token balance appear alongside the registered Merkle root.
2. Enter the amount and approve signing. Each click selects a **new random
   circular sample**. The circle covers at least
   one third of the enrollment's confirmed pattern pixels. Everything
   outside it is blurred in the page and excluded from analysis. The backend
   extracts fresh features only from this unchanged partial sample, reconstructs
   a signing key, and checks the signature and Merkle proof through the deployed
   wallet's read-only verifier. Matched fresh points receive four rounded lime arcs
   with a thin dark outline. Their center color still shows priority;
   the markers keep a readable minimum size on small screens.
3. Review the public key, signature and Merkle membership path, then submit to NEAR. A separate local relayer sends
   one transaction. The page shows its hash, receipt/callback result, gas and
   balances read from the token contract.
4. A capture can fail. Its image, softly revealed sample and observed features stay visible;
   click **Approve & sign** again to make another independent attempt. The server
   never secretly chooses another crop or falls back to the full image. Reaching
   the search budget means the search is incomplete, not a confirmed mismatch.
5. After the transfer finishes, the form is immediately ready for the next one.
   Its previous amount, recipient, receipt and proof remain visible while you
   edit the next transfer. Each approval creates a fresh signature; an accepted
   authorization cannot be reused. You can also switch wallets to send tokens back.

Use `-- --port 8781` to change the web port. For an HTTPS reverse proxy, set
`-- --public-origin https://gnorix.online`; the listener stays on loopback and
accepts the explicitly configured Host/Origin. See [public deployment](deploy/README.md)
for the release bundle, systemd service and nginx configuration. Public visitors
share the same disposable demo session.

Set `GNORIX_DEMO_CONTRACTS_DIR` to a directory containing both prebuilt Wasm
contracts to skip startup compilation. Without it, the local build behavior is
unchanged. Ctrl+C stops the app and its managed
sandbox. Every launch creates fresh disposable accounts, roots and balances;
this demo does not persist a wallet or reconnect to testnet/mainnet. Both source
images and signing run locally. Fingerprint extraction can take tens of seconds;
use the documented release build. The HTTP page remains responsive during it.

The web service does not write a signature archive or per-attempt metadata files.
It retains public enrollment data (Merkle hashes, context and parameters) and the
current capture, proof and transaction result in memory. Starting a new approval
replaces the previous attempt; reconstructed signing keys are not cached. Keys
are derived deterministically within that enrollment, while restarting creates
a new enrollment context. Submitted transactions remain in the disposable
sandbox's chain history, as required for actual NEAR execution.

The fingerprint, transfer controls and execution panels keep stable dimensions
through signing, submission and receipt updates. Long status messages and transfer
details scroll within their panels. The public evidence panel stays open: hashes,
public key and signature use hexadecimal text, and every membership-path level
shows its left/right inputs and parent hash, followed by the count-bound root.
The diagram represents the selected path; sibling hashes represent the other
subtrees. It is a presentation of the proof, not a replacement transport format.

If a transaction result cannot be observed, **Check transaction status** queries
the same hash. It does not resend, and another transfer is blocked until that
outcome is resolved. A failed token call consumes its accepted nonce, so retrying
requires a new approval and signature. A successful transfer with an unavailable
balance refresh is reported separately from confirmed balance data.

The app listens only on loopback, restricts requests to local origins and an
explicitly configured public HTTPS origin, and requires a per-session token for
mutations. It uses public test fingerprints and temporary NEAR keys; public
deployment does not change the private, disposable nature of the underlying chain.

This is one example with a single entry point:

```text
examples/near/
├── demo/                        Rust HTTP service, fingerprint adapter, managed chain
├── web/                         English UI and embedded local assets
├── assets/                      Two compact synthetic fingerprint fixtures
├── src/                         Smart wallet and canonical transfer payload
├── token/src/                   Standard token, without a Gnorix dependency
├── tests/                       Wallet and token sandbox scenarios
│   └── contracts/ft-receiver/    Test-only transfer-call callback receiver
└── test.sh                      Contract regression suite
```

The token is **Gnorix Test Token (GXT)** with **24 decimals**, implemented using
`near-contract-standards` 5.30.2. It follows NEP-141 transfers, NEP-145 storage,
NEP-148 metadata and standard token events. Private `new(owner_id, total_supply)`
registers the initial holder and mints the complete supply to that account.
There is no later minting method or administrative transfer path. Normal
transfer-call refunds use the standard resolver; forced storage unregistration
burns the account's remaining tokens and emits a burn event. The receiver under
`tests/` exists only to exercise successful, partial, full and failed refunds.

## Run the contract regression suite

Install the pinned tools once:

```sh
rustup toolchain install 1.99.1 --profile minimal
rustup target add --toolchain 1.99.1 wasm32-unknown-unknown
cargo +1.99.1 install cargo-near --version 0.23.0 --locked
```

Then run the contract regression suite from the repository root:

```sh
./examples/near/test.sh
```

The script builds the wallet, token and test receiver into `target/near/`, then
runs both sandbox test suites. The tests start local nearcore 2.14.1 instances,
create temporary accounts, deploy the Wasm, fund the wallet, sign through Gnorix
core and exercise relayed transfers and standard token behavior.
The sandbox binary is downloaded automatically; `NEAR_SANDBOX_BIN_PATH` can
select a preinstalled compatible binary. Nothing here deploys to testnet or
mainnet.

After building, rerun just the wallet or standard-token scenarios with:

```sh
cargo +1.99.1 test -p gnorix-near-wallet --features sandbox \
  --locked --test wallet -- --ignored --nocapture
cargo +1.99.1 test -p gnorix-near-wallet --features sandbox \
  --locked --test fungible_token -- --ignored --nocapture
```

The default wallet manifest is `examples/near/Cargo.toml`. To rebuild only that
contract while working on it:

```sh
cargo near build non-reproducible-wasm --locked --no-abi \
  --manifest-path examples/near/Cargo.toml --out-dir target/near \
  --override-toolchain 1.99.1
```

It produces `target/near/gnorix_near_wallet.wasm`; the accompanying artifacts are
`gnorix_near_token.wasm` and `gnorix_ft_test_receiver.wasm` in the same directory.
Sandbox tests are opt-in and ignored by ordinary workspace test runs.

## Setup and contract interface

Deploy and initialize together in a transaction authorized by the wallet
account. `new(owner_id)` is private: a caller on another account cannot initialize
an uninitialized wallet. `owner_id` can be the wallet account itself or a separate
management account. An existing account's contract must not be overwritten
without understanding its code and state. Keeping full-access keys also keeps
their authority to replace code or otherwise control the account; Gnorix does
not restrict those keys.

Token registration and Gnorix registration are separate:

1. Register the wallet and the intended recipient with the token's
   `storage_deposit`, paying the token's required storage deposit. Fund the wallet
   with an ordinary token transfer. Also leave native NEAR on the wallet for its
   own storage and outgoing one-yoctoNEAR deposits.
2. Enroll offchain with the selected example HKDF-SHA256/Ed25519 suite. The
   context's verifier is the wallet account, subject is `owner_id`, network is
   `near:<chain_id>`, and initial revision is `1`. Retain the `.grx` package.
3. Have the owner call `register_enrollment` with the public commitment and enough
   attached NEAR to pay measured storage growth. Excess payment is refunded
   asynchronously. Neither the token nor the wallet receives biometric images,
   private keys or the complete Merkle tree.

Removing or recreating the wallet's balance registration in an external token
does not change its Gnorix root, revision or nonce. A standard token sends no
notification about `storage_unregister` to this wallet. The owner disables Gnorix
explicitly with `revoke_enrollment`; its record is retained, and reactivation
requires `replace_enrollment` with the next revision.

| Method | Behavior |
| --- | --- |
| `new(owner_id: AccountId)` | Private, one-time initialization. |
| `register_enrollment(commitment: Commitment)` | Owner only; payable storage registration. |
| `replace_enrollment(commitment: Commitment)` | Owner only; same enrollment ID, next revision; payable if storage grows; refuses a pending transfer. |
| `revoke_enrollment()` | Owner only; disables new authorizations and retains replay history. |
| `get_owner()` | Returns the management account. |
| `get_enrollment()` | Returns optional `EnrollmentView`: commitment, active status, next nonce. |
| `get_transfer_status()` | Returns the latest accepted transfer's revision, nonce and status. |
| `verify_transfer(transfer: Transfer, authorization: Authorization)` | Read-only cryptographic check; does not consume a nonce or check token balances. |
| `transfer(transfer: Transfer, authorization: Authorization)` | Any relayer, zero attached deposit; verifies, consumes the nonce and dispatches the token call. |
| `on_transfer(revision: U64, nonce: U64)` | Private runtime callback; not a relayer endpoint. |

`Transfer` contains `token_id: AccountId`, `receiver_id: AccountId`,
`amount: U128` and `memo: Option<String>`. Amount is a decimal JSON string in the
token's smallest units. The amount must be positive; the wallet cannot be its
own token or recipient; a memo is limited to 256 UTF-8 bytes. Revision and nonce
in the returned outcome are also decimal strings.

## What the owner signs

The operation is exactly `wallet.ft_transfer.v1`. The payload is a Borsh tuple
in this order: token account, receiver account, raw `u128` amount, optional UTF-8
memo. Strings have their Borsh length prefixes, the amount is 16 little-endian
bytes, and the memo carries its option tag. `None` and `Some("")` differ. JSON
property order has no effect. See [`Transfer::signing_payload`](src/transfer.rs)
for the implementation and byte-level test.

The signing application first fetches the wallet's enrollment and checks that it
matches the retained package. The following fragment uses the ordinary core API;
`transfer` is the typed request whose `signing_payload` is described above:

```rust,ignore
use gnorix::{core::authorization, near::{Action, Authorization, Commitment}};

assert!(view.active, "enrollment is revoked");
assert_eq!(Commitment::from_core(package.commitment())?, view.commitment);

let action = Action {
    operation: "wallet.ft_transfer.v1".to_owned(),
    payload: transfer.signing_payload(),
    nonce: view.next_nonce,
    expires_at_ms, // Unix milliseconds, compared with chain block time.
};

// Obtain the owner's approval of these exact transfer arguments before signing.
let signed = authorization::sign_candidates(
    &crypto,
    candidates,
    profile_id,
    &package,
    action.to_core()?,
    secret.as_ref(),
    &search_limits,
)?;
let authorization = Authorization::from_core(&signed)?;
```

Submit the same `transfer` and converted `authorization` to the wallet's
`transfer` method. The contract reconstructs the payload from its actual typed
arguments. The SDK additionally binds the owner, wallet account, network, root,
revision, nonce and expiry. This is authorization data inside a normal NEAR
transaction; the relayer still signs that transaction with its NEAR access key.

## Asynchronous results and retries

The relayer attaches **zero NEAR** and should provide **100 Tgas**. The wallet
funds the standard token call's **one yoctoNEAR** deposit from its own balance.
It allocates 20 Tgas to `ft_transfer` and 10 Tgas to its callback; the accepting
receipt checks that enough gas remains to schedule both. A token whose transfer
needs more than this configured allowance can fail.

Only one transfer may be pending. A successful accepting receipt consumes the
nonce and records `pending` before dispatch. The callback records `succeeded` or
`failed` for the matching revision and nonce. `succeeded` means the **signed
token contract's receipt succeeded**; it is not an independent audit that an
arbitrary contract really implements the token standard. Use the intended,
trusted token contract.

An external failure does not roll back the already accepted nonce. After
`failed`, correct the cause, fetch the new nonce and approve/sign a new request.
A replay of the previous authorization is rejected. Rejection within the
accepting receipt, such as insufficient scheduling gas, rolls that receipt's
state changes back instead.

If the callback itself fails, the outcome can remain `pending`. Inspect the
transaction's receipts and token balances before taking recovery action; do not
blindly resend or reset the nonce, because the transfer may already have happened.
The example exposes no automatic pending-state reset. Revoking an enrollment
prevents new requests but cannot undo an already dispatched transfer. Replacement
waits until no transfer is pending.

The contract regression suites use synthetic candidate bytes to isolate signing
and contract behavior from fingerprint recognition. The web demo uses real
extraction and reconstruction from its two bundled synthetic fingerprint images. Read the [NEAR SDK documentation and
recorded biometric limitations](../../docs/near-sdk.md) before adapting this
example for real funds. Successful protocol tests do not establish biometric
security.

## Demo verification

```sh
cargo test -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked
cargo test -p gnorix-near-wallet --features demo --bin gnorix-near-demo --release --locked -- --ignored --nocapture --test-threads=1
```

The opt-in tests exercise the complete application path: startup, both source
images, enrollment, fresh reconstruction, deployed signature verification and
real transfers in both directions. Additional checks cover failed calls, retained nonces and uncertain transaction
observation without rebroadcasting. The contract Wasm is
built automatically for these demo tests. Ordinary unit tests cover exact token
amounts and request validation.

## Fingerprint fixtures

Both wallets use separate synthetic scans at 241 × 298 pixels:
[Wallet 1](assets/wallet-1-fingerprint.png) and [Wallet 2](assets/wallet-2-fingerprint.png).
Their dark ridges and light scanner background resemble the project's test
gallery; the [fixture notes](assets/README.md) record generation and format.
The core test image is unchanged. Both images are software-test inputs, with no claimed real person, sensor calibration
or biometric uniqueness. The web demo applies the same explicit policy to both:
3–6 points per combination, priority at least 0.1, three anchors separated by
at least 6R, region radius 30R, and one neighbor step during key reconstruction.
Here R is the measured average dark-ridge thickness. Enrollment keeps only grid
baselines; signing checks variants lazily and stops at the first matching key.
Combination selection uses the strongest included point's score: above 0.9
receives 50% of selections, 0.5–0.9 receives 25%, and below 0.5 receives 25%.
Empty or exhausted groups pass their allowance to the others. Enrollment targets
three times the eligible regional point count, then adds any required point
coverage witnesses; capacity and those witnesses can change the final shares.
Signing applies the same group preference to its lazy combination search.
The signing work budget is 100 million units, including geometry and descriptor
bytes, not 100 million key attempts. It preserves priority order and checks a
subset's complete positional neighborhood before moving to the next subset.
Eligible points in a crop do not guarantee that an enrolled combination lies
entirely within it: enrollment selects a bounded set of combinations.
These selection rules use example profile v8; earlier enrollments must be recreated.

The public enrollment packages stay in memory; private keys and extracted
candidate descriptors are not cached for subsequent signatures. Each signature
binds the exact transfer, chain, wallet, owner, revision, nonce and expiry.

For the precise biometric limits, including the recorded cross-subject descriptor
match, see [the SDK guide](../../docs/near-sdk.md#runnable-example-and-evidence).
These local transfers demonstrate working software and contract authorization;
they do not qualify fingerprint-only custody of real assets.

For sampling, the backend chooses a center uniformly from the confirmed pattern
pixels and computes the smallest pixel radius containing at least a third of
those pixels. Coverage includes ties at the boundary and counts confirmed pattern,
not frame area or anatomical finger area. A fresh random seed is used for every
click, and only that one crop is analyzed. The preview PNG preserves the original
pixels. The page blends the sharp sample into a blurred copy of the same frame,
without a boundary ring, additional image distortions or decorative particles.
This soft edge is only visual: the analysis mask
remains an exact circle, and pixels outside it never enter reconstruction.
Fresh feature IDs belong to this capture, never to the enrollment's point list.
The 0.1 priority threshold is shared by registration and reconstruction.
