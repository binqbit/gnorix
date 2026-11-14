# NEAR authorization example

This counter demonstrates signing with Gnorix core and verification inside the
contract that executes the action. An account registers its own Merkle commitment;
any relayer can submit its signed authorization to increment that account's
counter. The relayer pays transaction gas and needs its own NEAR access key.
The Gnorix signature is a contract argument, not a native transaction signature.

The contract accepts the repository's explicit example HKDF-SHA256/Ed25519 suite.
Applications must select and review their own derivation/secret policy. This
example is not a claim about fingerprint entropy or recognition reliability.

## Build and test locally

Install Rust 1.99.1, its `wasm32-unknown-unknown` target, and `cargo-near` 0.23.0.
Run from the repository root:

```sh
rustup target add --toolchain 1.99.1 wasm32-unknown-unknown
cargo near build non-reproducible-wasm --no-abi \
  --manifest-path examples/near-contract/Cargo.toml --out-dir target/near
cargo test -p gnorix-near-integration-tests --features sandbox \
  --test authorization -- --ignored --nocapture
```

The build emits `target/near/gnorix_near_counter.wasm`. The test starts a local
NEAR sandbox pinned to nearcore 2.14.1, deploys this Wasm, creates temporary
accounts, and calls the deployed contract. It does not access testnet/mainnet.
The sandbox binary is downloaded automatically. Use `NEAR_SANDBOX_BIN_PATH` to
supply a preinstalled compatible binary, or `GNORIX_NEAR_CONTRACT_WASM` to choose
another build of this contract.

The test produces enrollments from deterministic 32-byte candidate fixtures,
signs them through `gnorix::core::authorization::sign_candidates`, converts the
result with `gnorix::near::Authorization::from_core`, and verifies it in Wasm.
Fixtures intentionally isolate protocol behavior from image processing.
Assertions cover successful relaying, payload/signature/proof tampering, replay,
expiration, network/contract binding, caller authorization, storage payment,
application permissions, revision replacement, and revocation.

## Contract interface

| Method | Caller and behavior |
| --- | --- |
| `new()` | Initialize once. |
| `register_enrollment(commitment)` | Immediate caller must own the commitment; attach storage payment. |
| `replace_enrollment(commitment)` | Owner only; next revision of the same enrollment; attach payment if storage grows. |
| `revoke_enrollment()` | Owner revokes its existing enrollment. |
| `get_enrollment(owner_id)` | Read commitment, active status, and next nonce. |
| `get_counter(owner_id)` | Read that owner's counter as a decimal string. |
| `verify_authorization(owner_id, operation, payload, authorization)` | Read-only check of arbitrary signed application data; returns a boolean. |
| `increment(owner_id, delta, authorization)` | Any relayer; `delta` is a decimal string within `1..=100`. |

`verify_authorization` checks the signature, Merkle membership, registered
domain/status, expiry, and current nonce without consuming anything. Repeating
the same valid query succeeds until the authorization is consumed or otherwise
invalidated. It does not enforce counter permissions: a correctly signed
increment of 101 passes this check, but `increment` rejects it and rolls back
nonce consumption. Only the application defines and executes business actions.
A successful view is not a reservation; the transaction must verify again.

The signed operation is `counter.increment`. Its payload is exactly the eight
big-endian bytes of `delta`, reconstructed by the contract from the actual call.
Use the current enrollment's `next_nonce` and an expiry in Unix milliseconds.
Before signing, compare the retained package with the active registered
commitment. Construct the approved `gnorix::near::Action`, convert it with
`to_core`, and pass it to the core signer. The NEAR module supplies contract
verification and shared transport types; key reconstruction and signing remain
in the core library.
The SDK binds the signature to the owner's registered root, revision, current
contract, and host-provided chain ID (`near:<chain_id>`). Replay consumption and
the counter update occur in one receipt; an error rolls both back.

The caller prepays measured registration/replacement storage. Any excess and
released storage stake are refunded to that caller using a NEAR transfer promise.
A refund transfer is asynchronous: failure does not undo the successful enrollment.
The example retains each owner's record after revocation so its enrollment
history and counter cannot be silently reset by registering again.

To adapt this pattern, replace `increment` with an explicit application operation,
validate its permissions and arguments, reconstruct its canonical payload, call
`verify_and_consume`, then mutate local application state. Cross-contract effects
need separate callback and failure handling; this example performs none.
