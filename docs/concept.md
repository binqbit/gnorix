# Concept

## What Gnorix is

Gnorix is an independent biometric-cryptographic library for reconstructing signing keys from biometric measurements and verifying the resulting signatures. Its central mechanism connects a person's variable biometric readings to an enrolled set of acceptable signing keys. Applications decide what those signatures authorize.

The library is independent of a particular blockchain or account model. NEAR is a separate integration for applications that need on-chain verification and permission-controlled actions.

## The problem: measurements vary, signatures need exact keys

Digital signatures depend on exact cryptographic keys. Biometric measurements do not reproduce as identical data every time: a finger can be placed differently, some features may be missing, and acquisition conditions can alter the reading. Simply hashing each fresh scan produces unrelated results when even a small part of the input changes.

Reducing every measurement to a very coarse representation can improve repeatability, but it can also discard distinctions between people. The challenge is to accommodate legitimate variation without making the accepted measurements so broad that unrelated inputs become interchangeable.

There is also a disclosure problem. A person may want to sign data or authorize an action without uploading their raw fingerprint to the verifier or publishing it on a blockchain. The verifying application needs evidence of a valid authorization, not necessarily the biometric sample itself.

## Core principle: accept an enrolled set rather than one exact reading

Gnorix's enrollment model accounts for a defined range of measurement variation. Instead of requiring every later capture to recover one identical representative, the process derives candidate signing keys from the variations accepted for that enrollment.

Each candidate has a private signing key and a corresponding public verification key. The allowed public keys are collected into a Merkle tree, a structure that combines their hashes into one compact commitment called the root. The root represents the enrolled set without requiring the verifier to store or search every key individually.

On a later capture, the library attempts to reconstruct one candidate private key. If its public key belongs to the enrolled set, the person can provide a membership proof and sign the intended action. Matching one enrolled candidate is sufficient; reconstructing every possible key is not necessary during ordinary signing.

This replaces an exact-reading requirement with an accepted-set requirement. It does not mean that every future capture will succeed, or that accepting more variations inherently improves security. The accepted range remains a meaningful part of the enrollment policy.

## What the signature package means

An authorization package combines three elements:

- A candidate public key, used to check the action signature.
- A Merkle membership path, used to check that this key belongs to the enrolled set.
- A signature over the particular data or action being authorized.

The Merkle root serves as the enrollment's public commitment. It is not the public key of the underlying signature algorithm. Membership and signature validity answer different questions: whether this key was enrolled, and whether the corresponding private key signed this action.

The full candidate set and raw biometric reading need not be submitted for this check. The disclosed candidate key, proof and action context are still visible to the verifier; avoiding raw-sample disclosure is not a claim of complete anonymity.

## Optional secrets and retained proof material

An enrollment can incorporate a PIN or password as an additional secret input. It need not be limited to four digits. This secret is different from public diversification data and contributes according to its actual unpredictability, not a fixed promised multiplier.

Biometric reconstruction and proof availability are separate requirements. A person may recover a valid candidate key yet still need retained tree or proof information to demonstrate membership. A compact public root alone does not recreate all of that information. Backup, protection and recovery of the retained material therefore belong to the user experience.

## Applications and authority

Gnorix can provide a signing mechanism for arbitrary data and a supplementary authorization mechanism for protected operations. The application retains responsibility for enrollment ownership, consent, permissions, revocation and preventing unwanted reuse.

In the NEAR integration, an application could allow a signed operation only within an owner's existing authorization. A valid signature would not bypass a spending limit or give control over arbitrary assets in another account.

The signature establishes a cryptographic relationship to an enrollment, not by itself a legal identity, a unique person, or proof of a live capture. These distinctions define where biometric signing fits within an application's broader protection model.

Continue with the full [enrollment and signing lifecycle](how-it-works.md) and [component architecture](architecture.md).
