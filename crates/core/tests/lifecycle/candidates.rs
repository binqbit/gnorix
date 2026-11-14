//! Already-generated candidate streams use the same enrollment and proof contracts.

use std::{cell::Cell, error::Error as _, num::NonZeroUsize};

use gnorix::{
    Error,
    biometrics::{Candidate, Candidates, MeasurementEngine},
    crypto::{CryptoProvider, SecretInput, SignatureVerifier},
    enrollment::{self, EnrollmentContext},
    error::{AlgorithmError, Resource, Stage},
    merkle::{self, MerkleError},
    policy::SecretPolicy,
};

use crate::support::{
    CryptoFault, FixtureError, SyntheticCrypto, SyntheticMeasurements, capture, failure, limits,
    spec,
};

fn context(crypto: &SyntheticCrypto, policy: SecretPolicy) -> EnrollmentContext {
    EnrollmentContext {
        spec: spec(policy),
        profile_id: SyntheticMeasurements::new().profile_id(),
        suite_id: crypto.suite_id().into(),
    }
}

fn candidates<'a>(symbols: &'a [u8], yielded: &'a Cell<usize>) -> Candidates<'a> {
    Box::new(symbols.iter().map(|symbol| {
        yielded.set(yielded.get() + 1);
        Ok(Candidate::new(vec![*symbol]).unwrap())
    }))
}

#[test]
fn prepared_stream_matches_measurement_enrollment_and_external_proof_lookup() {
    let measurement = SyntheticMeasurements::new();
    let crypto = SyntheticCrypto::new();
    let limits = limits();
    let secret = SecretInput::new(b"stream-test-secret".to_vec()).unwrap();
    let measured = enrollment::enroll(
        &measurement,
        &crypto,
        &[capture(&[30, 10, 20, 10])],
        spec(SecretPolicy::Required),
        Some(&secret),
        &limits,
    )
    .unwrap();
    let context = measured.commitment().context.clone();
    let yielded = Cell::new(0);
    let streamed = enrollment::enroll_candidates(
        &crypto,
        candidates(&[10, 20, 10, 30], &yielded),
        context.clone(),
        Some(&secret),
        &limits,
    )
    .unwrap();

    assert_eq!(yielded.get(), 4);
    assert_eq!(streamed.reconstruction_model(), &());
    assert_eq!(streamed.commitment(), measured.commitment());
    assert_eq!(streamed.commitment().leaf_count, 3);
    assert_eq!(crypto.keys.created.get(), 8);
    crypto.assert_keys_dropped();

    // An external consumer uses the public encoder, selected provider and tree
    // exactly as enrollment does, without duplicating context serialization.
    let encoded = context.encode(&limits).unwrap();
    for symbol in [10, 20, 30] {
        let candidate = Candidate::new(vec![symbol]).unwrap();
        let key = crypto
            .derive_key(&candidate, &encoded, Some(&secret))
            .unwrap();
        let public = crypto.public_key(&key).unwrap();
        let leaf = merkle::leaf_hash(&encoded, &public);
        let proof = streamed.proof_material().proof_for(&leaf).unwrap();
        assert_eq!(proof, measured.proof_material().proof_for(&leaf).unwrap());
        merkle::verify(streamed.commitment().root, leaf, &proof).unwrap();
    }
    crypto.assert_keys_dropped();
}

#[test]
fn context_suite_and_secret_policy_fail_before_consuming_candidates() {
    let crypto = SyntheticCrypto::new();
    let secret = SecretInput::new(b"test-secret".to_vec()).unwrap();
    let yielded = Cell::new(0);
    let valid = context(&crypto, SecretPolicy::Absent);
    let mut wrong_suite = valid.clone();
    wrong_suite.suite_id = "different-suite".into();
    let mut invalid = valid.clone();
    invalid.spec.subject.clear();
    let mut bounded = limits();
    bounded.context_bytes = NonZeroUsize::new(1).unwrap();

    let cases = [
        (wrong_suite, None, limits()),
        (context(&crypto, SecretPolicy::Required), None, limits()),
        (valid.clone(), Some(&secret), limits()),
        (invalid, None, limits()),
        (valid, None, bounded),
    ];
    for (index, (context, secret, limits)) in cases.into_iter().enumerate() {
        let result = enrollment::enroll_candidates(
            &crypto,
            candidates(&[10], &yielded),
            context,
            secret,
            &limits,
        );
        match index {
            0 => assert!(matches!(result, Err(Error::SuiteMismatch))),
            1 | 2 => assert!(matches!(result, Err(Error::SecretPolicyMismatch))),
            3 => assert!(matches!(result, Err(Error::InvalidInput(_)))),
            4 => assert!(matches!(
                result,
                Err(Error::LimitExceeded(Resource::ContextBytes))
            )),
            _ => unreachable!(),
        }
        assert_eq!(yielded.get(), 0);
        assert_eq!(crypto.keys.created.get(), 0);
    }
}

#[test]
fn complete_empty_stream_fails_and_exact_duplicate_attempt_cap_succeeds() {
    let crypto = SyntheticCrypto::new();
    let yielded = Cell::new(0);
    let context = context(&crypto, SecretPolicy::Absent);
    let mut limits = limits();
    limits.candidate_attempts = NonZeroUsize::new(2).unwrap();
    let empty = enrollment::enroll_candidates(
        &crypto,
        candidates(&[], &yielded),
        context.clone(),
        None,
        &limits,
    );
    assert!(matches!(empty, Err(Error::Merkle(MerkleError::EmptyTree))));

    let complete = enrollment::enroll_candidates(
        &crypto,
        candidates(&[10, 10], &yielded),
        context,
        None,
        &limits,
    )
    .unwrap();
    assert_eq!(yielded.get(), 2);
    assert_eq!(crypto.keys.created.get(), 2);
    assert_eq!(complete.commitment().leaf_count, 1);
    crypto.assert_keys_dropped();
}

#[test]
fn duplicate_attempt_overflow_rejects_the_whole_stream_and_drops_prior_keys() {
    let crypto = SyntheticCrypto::new();
    let yielded = Cell::new(0);
    let mut limits = limits();
    limits.candidate_attempts = NonZeroUsize::new(2).unwrap();
    let result = enrollment::enroll_candidates(
        &crypto,
        candidates(&[10, 10, 10], &yielded),
        context(&crypto, SecretPolicy::Absent),
        None,
        &limits,
    );

    assert!(matches!(
        result,
        Err(Error::LimitExceeded(Resource::CandidateAttempts))
    ));
    assert_eq!(yielded.get(), 3);
    assert_eq!(crypto.keys.created.get(), 2);
    crypto.assert_keys_dropped();
}

#[test]
fn late_stream_error_preserves_its_stage_and_cause_and_discards_partial_work() {
    let crypto = SyntheticCrypto::new();
    let yielded = Cell::new(0);
    let entries = [
        Ok(Candidate::new(vec![10]).unwrap()),
        Err(Box::new(FixtureError("stream failure")) as AlgorithmError),
        Ok(Candidate::new(vec![30]).unwrap()),
    ];
    let stream = Box::new(
        entries
            .into_iter()
            .inspect(|_| yielded.set(yielded.get() + 1)),
    );
    let error = failure(enrollment::enroll_candidates(
        &crypto,
        stream,
        context(&crypto, SecretPolicy::Absent),
        None,
        &limits(),
    ));

    assert!(matches!(
        error,
        Error::Algorithm {
            stage: Stage::CandidateGeneration,
            ..
        }
    ));
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<FixtureError>()
            .unwrap()
            .0,
        "stream failure"
    );
    assert_eq!(yielded.get(), 2);
    assert_eq!(crypto.keys.created.get(), 1);
    crypto.assert_keys_dropped();
}

#[test]
fn provider_failures_preserve_error_stages_and_drop_transient_keys() {
    for (fault, expected_stage, created) in [
        (CryptoFault::Derive, Stage::KeyDerivation, 0),
        (CryptoFault::PublicKey, Stage::PublicKey, 1),
    ] {
        let mut crypto = SyntheticCrypto::new();
        crypto.fault = Some(fault);
        let yielded = Cell::new(0);
        let result = enrollment::enroll_candidates(
            &crypto,
            candidates(&[10, 20], &yielded),
            context(&crypto, SecretPolicy::Absent),
            None,
            &limits(),
        );

        assert!(matches!(
            result,
            Err(Error::Algorithm { stage, .. }) if stage == expected_stage
        ));
        assert_eq!(yielded.get(), 1);
        assert_eq!(crypto.keys.created.get(), created);
        crypto.assert_keys_dropped();
    }
}

#[test]
fn candidate_and_public_key_bounds_apply_to_prepared_streams() {
    for candidate_overflow in [true, false] {
        let crypto = SyntheticCrypto::new();
        let mut limits = limits();
        let (candidate, expected) = if candidate_overflow {
            (vec![10; 33], Resource::CandidateBytes)
        } else {
            limits.public_key_bytes = NonZeroUsize::new(31).unwrap();
            (vec![10], Resource::PublicKeyBytes)
        };
        let stream = Box::new(std::iter::once(Ok(Candidate::new(candidate).unwrap())));
        let result = enrollment::enroll_candidates(
            &crypto,
            stream,
            context(&crypto, SecretPolicy::Absent),
            None,
            &limits,
        );

        assert!(matches!(result, Err(Error::LimitExceeded(resource)) if resource == expected));
        assert_eq!(crypto.keys.created.get(), usize::from(!candidate_overflow));
        crypto.assert_keys_dropped();
    }
}
