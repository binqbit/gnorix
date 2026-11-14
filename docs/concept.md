# Concept

Gnorix connects variable biometric measurements to exact signing keys. It supplies the signing mechanism; the application decides who may enroll and what a valid signature permits.

## Why one hash of a scan is insufficient

Digital signatures need an exact key. Fingerprint captures vary with placement, coverage and image quality, so hashing a fresh image usually produces a different result. Making the representation coarser can improve repeatability while also discarding distinctions between fingerprints.

Gnorix explores an accepted-set approach: enroll keys from several feature combinations, then try to reconstruct any one of those keys from a later capture. A partial capture can succeed when it contains enough compatible features; success is not guaranteed merely because features are visible.

## What is enrolled

The current fingerprint path extracts and ranks features from one image. A shared minimum priority determines point eligibility at enrollment and signing. Each combination contains a configured number of points and enough pairwise-separated anchors to enclose its remaining points. Distances use the measured mean dark-ridge thickness, R, so a smaller later capture does not weaken the policy.

Each admitted combination is oriented, rounded to a triangular grid and encoded into a candidate digest. The selected cryptographic provider derives a signing key using that digest, enrollment context and any required additional secret. Enrollment commits only the rounded baseline of each combination; positional variations are searched later.

Context-bound hashes of candidate public keys are sorted, deduplicated and combined into a Merkle tree. Its root is the enrollment's public commitment. It is separate from the individual public key used to verify an action signature.

## What is presented when signing

For a new capture, Gnorix tests combinations in priority-weighted order. Each combination's baseline and allowed neighboring grid positions are checked immediately against the retained enrollment. The first matching key signs the exact approved action.

The resulting authorization contains:

- The matching public key.
- A Merkle path proving that key belongs to the enrolled set.
- A signature over the action and its enrollment context.

The verifier needs a trusted enrollment commitment and this authorization. It does not need the raw image or all enrolled keys. The signer still needs retained membership material: a root alone cannot produce arbitrary membership paths.

## Scope and limits

The library is independent of a blockchain. Its NEAR adapter registers commitments and verifies authorizations inside contracts; the example smart wallet transfers tokens it owns after verification. Ownership, permissions, replay protection, revocation and recovery remain application responsibilities.

An optional PIN or password is an additional secret, while a public salt only diversifies enrollments. Neither a 256-bit output nor many combinations establishes biometric entropy. A valid authorization proves possession of an enrolled key, not a live capture, a unique person or legal identity. Current biometric limitations are recorded in the [security boundary](near-sdk.md#biometric-security-boundary).

Continue with the [implemented flow](how-it-works.md) or [mechanism architecture](architecture.md).
