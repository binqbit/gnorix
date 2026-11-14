//! Trusted registry state, exact signed intent and proof verification.

use std::num::NonZeroU64;

use gnorix::{
    Error,
    authorization::{self, Action, EnrollmentStatus},
    crypto::SignatureVerifier,
    enrollment::{self, EnrollmentCommitment},
    error::AlgorithmError,
    merkle::MerkleError,
    policy::SecretPolicy,
};

use crate::support::{Fixture, SyntheticCrypto, capture, spec};

type Change<T> = (&'static str, fn(&mut T));

fn action_changes() -> [Change<Action>; 4] {
    [
        ("operation", |action| action.operation.push('!')),
        ("payload", |action| action.payload.push(1)),
        ("nonce", |action| action.nonce[0] ^= 1),
        ("expiry", |action| action.expires_at += 1),
    ]
}

#[test]
fn every_expected_action_field_must_match_before_signature_verification() {
    let fixture = Fixture::new();
    let signed = fixture.sign(&[20]).unwrap();

    for (name, change) in action_changes() {
        let mut expected = fixture.action.clone();
        change(&mut expected);

        let result = authorization::verify(
            &fixture.crypto,
            fixture.enrollment.commitment(),
            EnrollmentStatus::Active,
            &expected,
            &signed,
            10,
            &fixture.limits,
        );

        assert!(matches!(result, Err(Error::ActionMismatch)), "{name}");
    }

    assert_eq!(fixture.crypto.verification_calls.get(), 0);
}

#[test]
fn changed_signed_action_fails_even_when_the_expected_action_matches_the_tampering() {
    let fixture = Fixture::new();
    let signed = fixture.sign(&[20]).unwrap();

    for (name, change) in action_changes() {
        let mut tampered = signed.clone();
        change(&mut tampered.action);

        let result = authorization::verify(
            &fixture.crypto,
            fixture.enrollment.commitment(),
            EnrollmentStatus::Active,
            &tampered.action,
            &tampered,
            10,
            &fixture.limits,
        );

        assert!(matches!(result, Err(Error::InvalidSignature)), "{name}");
    }

    assert_eq!(fixture.crypto.verification_calls.get(), 4);
}

#[test]
fn expiration_uses_an_exclusive_boundary() {
    let fixture = Fixture::new();
    let signed = fixture.sign(&[20]).unwrap();

    for (now, accepted) in [(99, true), (100, false), (101, false)] {
        let result = authorization::verify(
            &fixture.crypto,
            fixture.enrollment.commitment(),
            EnrollmentStatus::Active,
            &fixture.action,
            &signed,
            now,
            &fixture.limits,
        );

        if accepted {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(Error::ActionExpired)));
        }
    }
}

#[test]
fn revocation_and_stale_enrollment_references_reject_before_crypto_verification() {
    let fixture = Fixture::new();
    let signed = fixture.sign(&[20]).unwrap();
    let revoked = authorization::verify(
        &fixture.crypto,
        fixture.enrollment.commitment(),
        EnrollmentStatus::Revoked,
        &fixture.action,
        &signed,
        10,
        &fixture.limits,
    );

    assert!(matches!(revoked, Err(Error::EnrollmentRevoked)));

    let mut renewed = fixture.enrollment.commitment().clone();
    renewed.context.spec.revision = NonZeroU64::new(2).unwrap();
    let mut replaced = fixture.enrollment.commitment().clone();
    replaced.context.spec.id[0] ^= 1;

    for trusted in [&renewed, &replaced] {
        let result = authorization::verify(
            &fixture.crypto,
            trusted,
            EnrollmentStatus::Active,
            &fixture.action,
            &signed,
            10,
            &fixture.limits,
        );

        assert!(matches!(result, Err(Error::EnrollmentMismatch)));
    }

    assert_eq!(fixture.crypto.verification_calls.get(), 0);
}

#[test]
fn trusted_root_and_full_enrollment_context_bind_membership() {
    let fixture = Fixture::new();
    let signed = fixture.sign(&[20]).unwrap();
    let changes: [Change<EnrollmentCommitment>; 7] = [
        ("root", |trusted| trusted.root.0[0] ^= 1),
        ("network", |trusted| {
            trusted.context.spec.domain.network.push('!')
        }),
        ("verifier", |trusted| {
            trusted.context.spec.domain.verifier.push('!')
        }),
        ("subject", |trusted| trusted.context.spec.subject.push('!')),
        ("salt", |trusted| trusted.context.spec.public_salt.push(1)),
        ("profile", |trusted| trusted.context.profile_id[0] ^= 1),
        ("secret policy", |trusted| {
            trusted.context.spec.secret_policy = SecretPolicy::Required
        }),
    ];

    for (name, change) in changes {
        let mut trusted = fixture.enrollment.commitment().clone();
        change(&mut trusted);

        let result = authorization::verify(
            &fixture.crypto,
            &trusted,
            EnrollmentStatus::Active,
            &fixture.action,
            &signed,
            10,
            &fixture.limits,
        );

        assert!(
            matches!(result, Err(Error::Merkle(MerkleError::InvalidProof))),
            "{name}"
        );
    }

    assert_eq!(fixture.crypto.verification_calls.get(), 0);
}

#[test]
fn tampered_public_key_path_count_and_signature_cannot_authorize() {
    let fixture = Fixture::new();
    let signed = fixture.sign(&[20]).unwrap();
    let mut wrong_key = signed.clone();
    wrong_key.public_key[0] ^= 1;
    let mut wrong_path = signed.clone();
    wrong_path.proof.siblings[0][0] ^= 1;

    for tampered in [&wrong_key, &wrong_path] {
        assert!(matches!(
            fixture.verify(tampered),
            Err(Error::Merkle(MerkleError::InvalidProof))
        ));
    }

    let mut wrong_count = signed.clone();
    wrong_count.proof.leaf_count += 1;
    assert!(matches!(
        fixture.verify(&wrong_count),
        Err(Error::EnrollmentMismatch)
    ));

    let mut wrong_signature = signed;
    wrong_signature.signature[0] ^= 1;
    assert!(matches!(
        fixture.verify(&wrong_signature),
        Err(Error::InvalidSignature)
    ));
}

#[test]
fn verification_leaves_durable_replay_consumption_to_the_application() {
    let fixture = Fixture::new();
    let signed = fixture.sign(&[20]).unwrap();

    assert!(fixture.verify(&signed).is_ok());
    assert!(fixture.verify(&signed).is_ok());
    assert_eq!(fixture.crypto.verification_calls.get(), 2);
}

#[test]
fn valid_membership_in_another_commitment_cannot_reuse_the_original_signature() {
    let fixture = Fixture::new();
    let original = fixture.sign(&[20]).unwrap();
    let changed_enrollment = enrollment::enroll(
        &fixture.measurement,
        &fixture.crypto,
        &[capture(&[20, 40])],
        spec(SecretPolicy::Absent),
        None,
        &fixture.limits,
    )
    .unwrap();
    let mut changed_authorization = authorization::sign(
        &fixture.measurement,
        &fixture.crypto,
        &changed_enrollment,
        &capture(&[20]),
        fixture.action.clone(),
        None,
        &fixture.limits,
    )
    .unwrap();

    assert_eq!(original.public_key, changed_authorization.public_key);
    assert_ne!(
        fixture.enrollment.commitment().root,
        changed_enrollment.commitment().root
    );
    assert!(
        authorization::verify(
            &fixture.crypto,
            changed_enrollment.commitment(),
            EnrollmentStatus::Active,
            &fixture.action,
            &changed_authorization,
            10,
            &fixture.limits,
        )
        .is_ok()
    );

    changed_authorization.signature = original.signature;

    assert!(matches!(
        authorization::verify(
            &fixture.crypto,
            changed_enrollment.commitment(),
            EnrollmentStatus::Active,
            &fixture.action,
            &changed_authorization,
            10,
            &fixture.limits,
        ),
        Err(Error::InvalidSignature)
    ));
}

#[test]
fn verification_accepts_a_provider_with_no_key_derivation_or_signing_interface() {
    struct ReadOnlyVerifier<'a>(&'a SyntheticCrypto);

    impl SignatureVerifier for ReadOnlyVerifier<'_> {
        fn suite_id(&self) -> &str {
            self.0.suite_id()
        }

        fn verify(
            &self,
            public_key: &[u8],
            message: &[u8],
            signature: &[u8],
        ) -> Result<bool, AlgorithmError> {
            self.0.verify(public_key, message, signature)
        }
    }

    let fixture = Fixture::new();
    let signed = fixture.sign(&[20]).unwrap();
    let verifier = ReadOnlyVerifier(&fixture.crypto);

    assert!(
        authorization::verify(
            &verifier,
            fixture.enrollment.commitment(),
            EnrollmentStatus::Active,
            &fixture.action,
            &signed,
            10,
            &fixture.limits,
        )
        .is_ok()
    );
}
