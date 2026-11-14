# NEAR example: smart wallet and standard token

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

This is one runnable example, organized around the two contracts it exercises:

```text
examples/near/
├── src/                         Smart wallet and canonical transfer payload
├── token/src/                   Standard token, without a Gnorix dependency
├── tests/                       Wallet and token sandbox scenarios
│   └── contracts/ft-receiver/    Test-only transfer-call callback receiver
└── test.sh                      Build all Wasm artifacts and run the scenarios
```

The token is **Gnorix Test Token (GXT)** with **24 decimals**, implemented using
`near-contract-standards` 5.30.2. It follows NEP-141 transfers, NEP-145 storage,
NEP-148 metadata and standard token events. Private `new(owner_id, total_supply)`
registers the initial holder and mints the complete supply to that account.
There is no later minting method or administrative transfer path. Normal
transfer-call refunds use the standard resolver; forced storage unregistration
burns the account's remaining tokens and emits a burn event. The receiver under
`tests/` exists only to exercise successful, partial, full and failed refunds.

## Build and run locally

Install the pinned tools once:

```sh
rustup toolchain install 1.99.1 --profile minimal
rustup target add --toolchain 1.99.1 wasm32-unknown-unknown
cargo +1.99.1 install cargo-near --version 0.23.0 --locked
```

Then run the entire example from the repository root:

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

The sandbox tests use synthetic candidate bytes to isolate signing and contract
behavior from fingerprint recognition. Read the [NEAR SDK documentation and
recorded biometric limitations](../../docs/near-sdk.md) before adapting this
example for real funds. Successful protocol tests do not establish biometric
security.
