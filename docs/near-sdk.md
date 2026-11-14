# NEAR contract integration

Gnorix can authorize a function inside a NEAR contract. The existing core
reconstructs an enrolled key and signs an exact action. The NEAR integration
checks its Merkle membership, signature, owner, network, revision, deadline and
next nonce. The application defines the signed data, permissions and any business
action; the SDK registers commitments and verifies authorizations.

The authorization is an argument to a normal NEAR transaction. A relayer still
needs a NEAR access key and gas. A Merkle root is not a NEAR account access key,
and this SDK does not create native transaction signatures from Merkle roots.

## Dependencies

Enable the single `near` feature on the public library:

```toml
# Contract: enrollment state and strict Ed25519 verification.
gnorix = { path = "../gnorix", default-features = false, features = ["near"] }
near-sdk = "=5.30.2"
```

Use `gnorix::near::contract::Enrollment` and the public `Action`, `Authorization`,
`Commitment` and `EnrollmentView` types under `gnorix::near`. The implementation
lives in `crates/near`; signing, biometric and Merkle algorithms remain in
`crates/core`. An application that also signs keeps the default `core` feature
and adds `features = ["near"]` when it needs these transport conversions.

The contract adapter requires NEAR protocol **85 or later**, because it obtains
the chain ID from the runtime. It binds `near:<chain_id>` and the current contract
account, rather than accepting an expected network from submitted arguments.
See the [NEAR host configuration](https://github.com/near/nearcore/blob/2.14.1/core/parameters/res/runtime_configs/85.yaml).

## Enrollment and lifecycle

1. Enroll offchain using the existing core pipeline and an explicitly selected
   Ed25519 `CryptoProvider`. Set the context network to `near:testnet`,
   `near:mainnet`, or the actual local chain ID; verifier to the destination
   contract account; subject to the owner's account; initial revision to `1`.
2. Retain the `.grx` package locally. Convert its public commitment with
   `Commitment::from_core(package.commitment())`.
3. The owner submits that commitment through an ordinary NEAR transaction.
   `Enrollment::register(commitment, accepted_suite_id)` checks the immediate
   predecessor, current contract/network, complete suite identifier and revision.
   The application rejects an existing owner slot and pays for storage growth.
4. Store the returned record under that owner. Expose `record.view()` to let a
   backend read the registered commitment, active status and next nonce.
5. Only the owner may call `replace` or `revoke`. Replacement preserves the owner
   and enrollment ID, advances the revision by exactly one, and resets the nonce
   in that new signed revision domain. Revocation retains the record and prevents
   further signed actions. Do not delete records or recreate occupied slots.

The contract stores the public commitment/context and replay state. Raw images,
feature points, candidate digests, private keys and the complete Merkle tree stay
offchain. The submitted proof carries only its public key, signature and path.
Registration stores an enrollment commitment, not each subsequent signature.

## Core signing and transport

Read `EnrollmentView` from the intended contract. The application checks that its
local package matches the active registered commitment and uses the current
nonce. Signing goes through the ordinary core API; the NEAR types only convert
the public request at the contract boundary:

```rust,ignore
use gnorix::{core::authorization, near::{Action, Authorization, Commitment}};

// Refuse revoked or stale registration before asking for action approval.
assert!(view.active, "enrollment is revoked");
assert_eq!(
    Commitment::from_core(package.commitment())?,
    view.commitment,
    "local package does not match the registered enrollment",
);

let action = Action {
    operation: "counter.increment".to_owned(),
    payload: delta.to_be_bytes().to_vec(),
    nonce: view.next_nonce,
    expires_at_ms,
};

// Obtain approval of this exact action before signing it.
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

`candidates` can be the existing fingerprint search iterator. It remains lazy and
stops at the first enrolled key. Exhaustion and search-budget exhaustion remain
distinct errors. Core `authorization::sign` also accepts a `MeasurementEngine`.
Transport, user approval and transaction submission belong to the application.

Send the authorization as JSON alongside the actual method arguments. All `u64`
fields use decimal strings, including nonce, revision, counts and expiration.
Byte fields use arrays. Expiration is Unix milliseconds, compared to block time;
an action is expired at `now >= expires_at_ms`. A concurrent successful action
can consume the nonce before submission: read fresh state and approve/sign again.

## Verification and optional one-time acceptance

Both methods return `Result<()>`: `Ok(())` means all checks passed; `Err(error)`
means verification failed. Neither executes a business action.

- `record.verify(operation, payload, &authorization)` checks the signed data
  against the current registration without changing state. Repeated checks can
  succeed while its nonce and deadline remain valid; verification does not reserve
  the nonce.
- `record.verify_and_consume(operation, payload, &authorization)` performs the
  same checks and advances the nonce on success. Use this when the application
  accepts an authorization once, and persist the updated record atomically with
  any application change.

Derive operation and canonical payload from the actual typed function arguments,
not from the submitted authorization. For example, a counter increment uses
`counter.increment` and the increment's unsigned 64-bit big-endian bytes:

```rust,ignore
// Read-only verification, for example in a contract view method:
state.enrollment.verify(
    "counter.increment",
    &delta.to_be_bytes(),
    &authorization,
)?;

// Alternatively, accept it once inside an application change method:
state.enrollment.verify_and_consume(
    "counter.increment",
    &delta.to_be_bytes(),
    &authorization,
)?;
// Enforce application permissions/bounds, then update application state.
// Abort the receipt on any error; persist both changes together on success.
```

The SDK reconstructs the expected nonce from persisted state and reuses core
verification. The signature also binds the stored context, root and leaf count.
Keep application permissions explicit: a valid signature does not bypass an
allowance or authorize an unrelated operation. An error after nonce consumption
must abort the receipt, not return a successful JSON error while saving state.

This interface covers actions within the verifying contract. Cross-contract
promises need a separate pending/execution/failure design: NEAR does not roll
back a successful caller when a later receipt fails. See
[NEAR cross-contract semantics](https://docs.near.org/smart-contracts/anatomy/crosscontract).
The [NEAR example](../examples/near/README.md) supplies that
application policy for standard token transfers: consume once when accepted,
retain a pending result, then record the token receipt's success or failure.
The SDK itself remains independent of transfers, custody and retry policy.

## Formats and verification policy

- Raw Ed25519 public keys are exactly 32 bytes; signatures exactly 64 bytes.
- The verifier uses Dalek `verify_strict`, matching the existing example backend.
  It deliberately does not substitute NEAR's less strict host verifier.
- Maximum payload: 4096 bytes; operation/suite: 128 bytes each; public salt: 64
  bytes; network: 64 bytes; Merkle path: 64 siblings. Decoded fields are bounded;
  callers should also bound the entire transport request.
- The NEAR sequential nonce is encoded in the final eight bytes of the core's
  32-byte nonce, big-endian, with the first 24 bytes zero. Noncanonical nonce
  conversions are rejected. Core signing and Merkle encodings are unchanged.
- The complete suite ID is selected by the application, not by the request.
  A provider must use Ed25519 with compatible strict verification and raw byte
  encodings. Key derivation remains its own explicit policy.

## Runnable example and evidence

The single [NEAR example](../examples/near/README.md) contains a smart wallet,
a standard token and their local sandbox tests. Run `./examples/near/test.sh`
from the repository root to build the contracts and exercise the complete flow.
The tests cover registration, relayed transfers, replay rejection, altered
requests, expiry, revocation, rotation, storage and asynchronous failure handling.

The wallet owns the token balance; an independent controller cannot spend tokens
belonging to a different account. Gnorix registration lives in the wallet and is
independent of the token's storage registration. The authorization tests use the
explicitly named example HKDF-SHA256/Ed25519 provider and synthetic candidate
digests to isolate protocol behavior from fingerprint repeatability.

The integration is not a claim of biometric security. The current fingerprint
descriptor has a recorded cross-subject match in local testing; correct Merkle
and signature verification cannot distinguish that collision from legitimate key
reconstruction. Real-capture accuracy, descriptor discrimination, secret/KDF
policy and production security qualification remain separate requirements.
No testnet or mainnet deployment is part of these local tests.
