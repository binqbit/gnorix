//! Streaming reconstruction preserves the existing signing and verification contract.

use std::{cell::Cell, error::Error as _, num::NonZeroUsize};

use gnorix_core::{
    Error,
    authorization::{self, EnrollmentStatus},
    biometrics::{Candidate, Candidates, MeasurementEngine},
    crypto::SecretInput,
    enrollment,
    error::{AlgorithmError, Resource, Stage},
    package::{EnrollmentPackage, PackageLimits},
    policy::SecretPolicy,
};

use crate::support::{
    CryptoFault, Fixture, FixtureError, MeasurementFault, SyntheticCrypto, SyntheticMeasurements,
    action, capture, failure, limits, spec,
};

fn candidates<'a>(symbols: &'a [u8], yielded: &'a Cell<usize>) -> Candidates<'a> {
    Box::new(symbols.iter().map(|symbol| {
        yielded.set(yielded.get() + 1);
        Ok(Candidate::new(vec![*symbol]).unwrap())
    }))
}

fn package(fixture: &Fixture) -> EnrollmentPackage {
    let package = EnrollmentPackage::from_enrollment(&fixture.enrollment, &fixture.limits).unwrap();
    let capacity = PackageLimits {
        max_bytes: NonZeroUsize::new(4096).unwrap(),
        max_leaves: NonZeroUsize::new(8).unwrap(),
    };
    let bytes = package.encode(&capacity, &fixture.limits).unwrap();
    EnrollmentPackage::decode(&bytes, &capacity, &fixture.limits).unwrap()
}

#[test]
fn restored_package_signing_matches_existing_signing_and_stops_at_the_first_member() {
    let mut fixture = Fixture::new();
    let package = package(&fixture);
    fixture.limits.candidate_attempts = NonZeroUsize::new(2).unwrap();
    let expected = fixture.sign(&[20]).unwrap();
    let before = fixture.crypto.keys.created.get();
    let yielded = Cell::new(0);
    let entries = [
        Ok(Candidate::new(vec![99]).unwrap()),
        Ok(Candidate::new(vec![20]).unwrap()),
        Err(Box::new(FixtureError("must not read past first member")) as AlgorithmError),
        Ok(Candidate::new(vec![10]).unwrap()),
    ];
    let stream = Box::new(
        entries
            .into_iter()
            .inspect(|_| yielded.set(yielded.get() + 1)),
    );
    let signed = authorization::sign_candidates(
        &fixture.crypto,
        stream,
        fixture.measurement.profile_id(),
        &package,
        fixture.action.clone(),
        None,
        &fixture.limits,
    )
    .unwrap();

    assert_eq!(signed, expected);
    assert_eq!(yielded.get(), 2);
    assert_eq!(fixture.crypto.keys.created.get() - before, 2);
    fixture.crypto.assert_keys_dropped();
    fixture.verify(&signed).unwrap();
}

#[test]
fn profile_suite_secret_context_and_action_fail_before_consuming_candidates() {
    for case in 0..6 {
        let mut fixture = Fixture::new();
        let package = package(&fixture);
        let mut profile = fixture.measurement.profile_id();
        let independent_secret = SecretInput::new(b"unexpected-test-secret".to_vec()).unwrap();
        let mut secret = None;
        match case {
            0 => profile[0] ^= 1,
            1 => fixture.crypto.suite = "different-suite",
            2 => secret = Some(&independent_secret),
            3 => fixture.limits.context_bytes = NonZeroUsize::new(1).unwrap(),
            4 => fixture.limits.action_bytes = NonZeroUsize::new(1).unwrap(),
            5 => fixture.action.operation.clear(),
            _ => unreachable!(),
        }
        let yielded = Cell::new(0);
        let result = authorization::sign_candidates(
            &fixture.crypto,
            candidates(&[20], &yielded),
            profile,
            &package,
            fixture.action.clone(),
            secret,
            &fixture.limits,
        );
        match case {
            0 => assert!(matches!(result, Err(Error::ProfileMismatch))),
            1 => assert!(matches!(result, Err(Error::SuiteMismatch))),
            2 => assert!(matches!(result, Err(Error::SecretPolicyMismatch))),
            3 => assert!(matches!(
                result,
                Err(Error::LimitExceeded(Resource::ContextBytes))
            )),
            4 => assert!(matches!(
                result,
                Err(Error::LimitExceeded(Resource::ActionBytes))
            )),
            5 => assert!(matches!(result, Err(Error::InvalidInput(_)))),
            _ => unreachable!(),
        }
        assert_eq!(yielded.get(), 0);
        assert_eq!(fixture.crypto.keys.created.get(), 3);
        assert_eq!(fixture.crypto.signing_calls.get(), 0);
        fixture.crypto.assert_keys_dropped();
    }
}

#[test]
fn existing_measurement_reconstruction_stays_deferred_until_action_validation() {
    let mut fixture = Fixture::new();
    fixture.measurement.fault = Some(MeasurementFault::ReconstructionStart);
    fixture.action.operation.clear();
    assert!(matches!(fixture.sign(&[20]), Err(Error::InvalidInput(_))));
    assert_eq!(fixture.measurement.reconstruction_yielded.get(), 0);
    assert_eq!(fixture.crypto.keys.created.get(), 3);
}

#[test]
fn exhaustion_and_attempt_overflow_remain_distinct_and_neither_signs() {
    for (symbols, cap, overflow, expected_yielded, expected_new_keys) in [
        (vec![], 8, false, 0, 0),
        (vec![98, 99], 8, false, 2, 2),
        (vec![98, 99], 2, false, 2, 2),
        (vec![98, 99, 20], 2, true, 3, 2),
    ] {
        let mut fixture = Fixture::new();
        let package = package(&fixture);
        fixture.limits.candidate_attempts = NonZeroUsize::new(cap).unwrap();
        let yielded = Cell::new(0);
        let result = authorization::sign_candidates(
            &fixture.crypto,
            candidates(&symbols, &yielded),
            fixture.measurement.profile_id(),
            &package,
            fixture.action.clone(),
            None,
            &fixture.limits,
        );
        if overflow {
            assert!(matches!(
                result,
                Err(Error::LimitExceeded(Resource::CandidateAttempts))
            ));
        } else {
            assert!(matches!(result, Err(Error::CandidateNotEnrolled)));
        }
        assert_eq!(yielded.get(), expected_yielded);
        assert_eq!(fixture.crypto.keys.created.get(), 3 + expected_new_keys);
        assert_eq!(fixture.crypto.signing_calls.get(), 0);
        fixture.crypto.assert_keys_dropped();
    }
}

#[test]
fn stream_errors_before_a_match_keep_the_reconstruction_stage_and_cause() {
    let fixture = Fixture::new();
    let package = package(&fixture);
    let yielded = Cell::new(0);
    let entries = [
        Ok(Candidate::new(vec![99]).unwrap()),
        Err(Box::new(FixtureError("search work exhausted")) as AlgorithmError),
        Ok(Candidate::new(vec![20]).unwrap()),
    ];
    let stream = Box::new(
        entries
            .into_iter()
            .inspect(|_| yielded.set(yielded.get() + 1)),
    );
    let error = failure(authorization::sign_candidates(
        &fixture.crypto,
        stream,
        fixture.measurement.profile_id(),
        &package,
        fixture.action.clone(),
        None,
        &fixture.limits,
    ));
    assert!(matches!(
        error,
        Error::Algorithm {
            stage: Stage::Reconstruction,
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
        "search work exhausted"
    );
    assert_eq!(yielded.get(), 2);
    assert_eq!(fixture.crypto.keys.created.get(), 4);
    assert_eq!(fixture.crypto.signing_calls.get(), 0);
    fixture.crypto.assert_keys_dropped();
}

#[test]
fn provider_and_output_failures_drop_temporary_keys_without_searching_further() {
    for case in 0..6 {
        let mut fixture = Fixture::new();
        let package = package(&fixture);
        match case {
            0 => fixture.crypto.fault = Some(CryptoFault::Derive),
            1 => fixture.crypto.fault = Some(CryptoFault::PublicKey),
            2 => fixture.crypto.fault = Some(CryptoFault::Sign),
            3 => fixture.limits.candidate_bytes = NonZeroUsize::new(1).unwrap(),
            4 => fixture.limits.public_key_bytes = NonZeroUsize::new(31).unwrap(),
            5 => fixture.limits.signature_bytes = NonZeroUsize::new(31).unwrap(),
            _ => unreachable!(),
        }
        let yielded = Cell::new(0);
        let stream: Candidates<'_> = if case == 3 {
            Box::new(std::iter::once_with(|| {
                yielded.set(1);
                Ok(Candidate::new(vec![20, 21]).unwrap())
            }))
        } else {
            candidates(&[20, 10], &yielded)
        };
        let result = authorization::sign_candidates(
            &fixture.crypto,
            stream,
            fixture.measurement.profile_id(),
            &package,
            fixture.action.clone(),
            None,
            &fixture.limits,
        );
        match case {
            0 => assert!(matches!(
                result,
                Err(Error::Algorithm {
                    stage: Stage::KeyDerivation,
                    ..
                })
            )),
            1 => assert!(matches!(
                result,
                Err(Error::Algorithm {
                    stage: Stage::PublicKey,
                    ..
                })
            )),
            2 => assert!(matches!(
                result,
                Err(Error::Algorithm {
                    stage: Stage::Signing,
                    ..
                })
            )),
            3 => assert!(matches!(
                result,
                Err(Error::LimitExceeded(Resource::CandidateBytes))
            )),
            4 => assert!(matches!(
                result,
                Err(Error::LimitExceeded(Resource::PublicKeyBytes))
            )),
            5 => assert!(matches!(
                result,
                Err(Error::LimitExceeded(Resource::SignatureBytes))
            )),
            _ => unreachable!(),
        }
        assert_eq!(yielded.get(), 1);
        fixture.crypto.assert_keys_dropped();
    }
}

#[test]
fn a_required_secret_is_needed_for_membership_and_is_not_part_of_authorization() {
    let measurement = SyntheticMeasurements::new();
    let crypto = SyntheticCrypto::new();
    let limits = limits();
    let secret = SecretInput::new(b"correct-test-secret".to_vec()).unwrap();
    let wrong = SecretInput::new(b"wrong-test-secret".to_vec()).unwrap();
    let enrollment = enrollment::enroll(
        &measurement,
        &crypto,
        &[capture(&[20])],
        spec(SecretPolicy::Required),
        Some(&secret),
        &limits,
    )
    .unwrap();
    let package = EnrollmentPackage::from_enrollment(&enrollment, &limits).unwrap();
    let yielded = Cell::new(0);
    let missing = authorization::sign_candidates(
        &crypto,
        candidates(&[20], &yielded),
        measurement.profile_id(),
        &package,
        action(),
        None,
        &limits,
    );
    assert!(matches!(missing, Err(Error::SecretPolicyMismatch)));
    assert_eq!(yielded.get(), 0);
    let result = authorization::sign_candidates(
        &crypto,
        candidates(&[20], &yielded),
        measurement.profile_id(),
        &package,
        action(),
        Some(&wrong),
        &limits,
    );
    assert!(matches!(result, Err(Error::CandidateNotEnrolled)));
    let signed = authorization::sign_candidates(
        &crypto,
        candidates(&[20], &yielded),
        measurement.profile_id(),
        &package,
        action(),
        Some(&secret),
        &limits,
    )
    .unwrap();
    authorization::verify(
        &crypto,
        enrollment.commitment(),
        EnrollmentStatus::Active,
        &action(),
        &signed,
        10,
        &limits,
    )
    .unwrap();
    let diagnostic = format!("{signed:?}");
    assert!(!diagnostic.contains("correct-test-secret"));
    assert!(!diagnostic.contains(&format!("{:?}", secret.as_bytes())));
    crypto.assert_keys_dropped();
}

#[test]
fn changed_payload_public_key_proof_signature_and_trusted_root_are_rejected() {
    let fixture = Fixture::new();
    let package = package(&fixture);
    let yielded = Cell::new(0);
    let signed = authorization::sign_candidates(
        &fixture.crypto,
        candidates(&[20], &yielded),
        fixture.measurement.profile_id(),
        &package,
        fixture.action.clone(),
        None,
        &fixture.limits,
    )
    .unwrap();
    fixture.verify(&signed).unwrap();
    for field in 0..5 {
        let mut changed = signed.clone();
        let mut trusted = fixture.enrollment.commitment().clone();
        match field {
            0 => changed.action.payload.push(1),
            1 => changed.public_key[0] ^= 1,
            2 => changed.proof.siblings[0][0] ^= 1,
            3 => changed.signature[0] ^= 1,
            4 => trusted.root.0[0] ^= 1,
            _ => unreachable!(),
        }
        let expected = &changed.action;
        assert!(
            authorization::verify(
                &fixture.crypto,
                &trusted,
                EnrollmentStatus::Active,
                expected,
                &changed,
                10,
                &fixture.limits
            )
            .is_err()
        );
    }
}
