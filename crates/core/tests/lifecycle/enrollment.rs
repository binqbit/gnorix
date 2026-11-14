//! Enrollment, reconstruction, secret policy and complete candidate enumeration.

use std::num::NonZeroUsize;

use gnorix_core::{
    Error,
    authorization::{self, EnrollmentStatus},
    crypto::SecretInput,
    enrollment,
    error::Resource,
    merkle::MerkleError,
    policy::SecretPolicy,
};

use crate::support::{
    Fixture, MeasurementFault, SyntheticCrypto, SyntheticMeasurements, action, capture, failure,
    limits, spec,
};

#[test]
fn empty_capture_input_is_rejected_before_measurement_preparation() {
    let mut measurement = SyntheticMeasurements::new();
    measurement.fault = Some(MeasurementFault::Prepare);
    let crypto = SyntheticCrypto::new();
    let result = enrollment::enroll(
        &measurement,
        &crypto,
        &[],
        spec(SecretPolicy::Absent),
        None,
        &limits(),
    );

    // If preparation were called, the injected algorithm error would replace
    // this structural input rejection.
    assert!(matches!(
        result,
        Err(Error::InvalidInput("enrollment captures are empty"))
    ));
    assert_eq!(measurement.enrollment_yielded.get(), 0);
    assert_eq!(crypto.keys.created.get(), 0);
}

#[test]
fn complete_empty_candidate_stream_cannot_create_an_enrollment() {
    let mut measurement = SyntheticMeasurements::new();
    measurement.empty_enrollment = true;
    let crypto = SyntheticCrypto::new();
    let result = enrollment::enroll(
        &measurement,
        &crypto,
        &[capture(&[10, 20])],
        spec(SecretPolicy::Absent),
        None,
        &limits(),
    );

    assert!(matches!(result, Err(Error::Merkle(MerkleError::EmptyTree))));
    assert_eq!(measurement.enrollment_yielded.get(), 0);
    assert_eq!(crypto.keys.created.get(), 0);
}

#[test]
fn fresh_capture_can_reconstruct_sign_and_verify_an_enrolled_candidate() {
    let fixture = Fixture::new();

    assert_eq!(fixture.enrollment.commitment().leaf_count, 3);
    assert_eq!(fixture.enrollment.proof_material().leaf_count(), 3);
    assert_eq!(fixture.crypto.keys.created.get(), 3);
    fixture.crypto.assert_keys_dropped();

    // This fresh input differs from enrollment and has a nonmember before the
    // matching identifier. It is a test of search orchestration, not fingerprint noise.
    let authorization = fixture.sign(&[99, 20]).unwrap();
    let verified = fixture.verify(&authorization).unwrap();

    assert_eq!(verified.commitment(), fixture.enrollment.commitment());
    assert_eq!(verified.action(), &fixture.action);
    assert_eq!(fixture.measurement.reconstruction_yielded.get(), 2);
    assert_eq!(fixture.crypto.keys.created.get(), 5);
    assert_eq!(fixture.crypto.signing_calls.get(), 1);
    assert_eq!(fixture.crypto.verification_calls.get(), 1);
    fixture.crypto.assert_keys_dropped();
}

#[test]
fn mismatched_measurement_profile_and_crypto_suite_reject_signing() {
    let mut fixture = Fixture::new();
    fixture.measurement.profile[0] ^= 1;

    assert!(matches!(fixture.sign(&[20]), Err(Error::ProfileMismatch)));
    assert_eq!(fixture.measurement.reconstruction_yielded.get(), 0);

    fixture.measurement.profile[0] ^= 1;
    let signed = fixture.sign(&[20]).unwrap();
    fixture.crypto.suite = "different-test-suite";

    assert!(matches!(fixture.sign(&[20]), Err(Error::SuiteMismatch)));
    assert!(matches!(fixture.verify(&signed), Err(Error::SuiteMismatch)));
    assert_eq!(fixture.measurement.reconstruction_yielded.get(), 1);
    fixture.crypto.assert_keys_dropped();
}

#[test]
fn required_secret_is_bound_to_keys_and_secret_presence_is_enforced() {
    let measurement = SyntheticMeasurements::new();
    let crypto = SyntheticCrypto::new();
    let limits = limits();
    let right_secret = SecretInput::new(b"test-secret-one".to_vec()).unwrap();
    let wrong_secret = SecretInput::new(b"test-secret-two".to_vec()).unwrap();

    for (policy, supplied) in [
        (SecretPolicy::Required, None),
        (SecretPolicy::Absent, Some(&right_secret)),
    ] {
        assert!(matches!(
            enrollment::enroll(
                &measurement,
                &crypto,
                &[capture(&[10])],
                spec(policy),
                supplied,
                &limits
            ),
            Err(Error::SecretPolicyMismatch)
        ));
    }

    let enrollment = enrollment::enroll(
        &measurement,
        &crypto,
        &[capture(&[10, 20])],
        spec(SecretPolicy::Required),
        Some(&right_secret),
        &limits,
    )
    .unwrap();

    let missing = authorization::sign(
        &measurement,
        &crypto,
        &enrollment,
        &capture(&[20]),
        action(),
        None,
        &limits,
    );
    let incorrect = authorization::sign(
        &measurement,
        &crypto,
        &enrollment,
        &capture(&[20]),
        action(),
        Some(&wrong_secret),
        &limits,
    );

    assert!(matches!(missing, Err(Error::SecretPolicyMismatch)));
    assert!(matches!(incorrect, Err(Error::CandidateNotEnrolled)));
    assert_eq!(crypto.signing_calls.get(), 0);

    let signed = authorization::sign(
        &measurement,
        &crypto,
        &enrollment,
        &capture(&[20]),
        action(),
        Some(&right_secret),
        &limits,
    )
    .unwrap();

    assert!(
        authorization::verify(
            &crypto,
            enrollment.commitment(),
            EnrollmentStatus::Active,
            &action(),
            &signed,
            10,
            &limits,
        )
        .is_ok()
    );
    crypto.assert_keys_dropped();

    let fixture = Fixture::new();
    let unexpected = authorization::sign(
        &fixture.measurement,
        &fixture.crypto,
        &fixture.enrollment,
        &capture(&[20]),
        action(),
        Some(&right_secret),
        &fixture.limits,
    );

    assert!(matches!(unexpected, Err(Error::SecretPolicyMismatch)));
    assert_eq!(fixture.measurement.reconstruction_yielded.get(), 0);
}

#[test]
fn exact_enrollment_attempt_cap_is_accepted_and_duplicate_keys_share_one_leaf() {
    let measurement = SyntheticMeasurements::new();
    let crypto = SyntheticCrypto::new();
    let mut limits = limits();
    limits.candidate_attempts = NonZeroUsize::new(3).unwrap();
    let enrollment = enrollment::enroll(
        &measurement,
        &crypto,
        &[capture(&[10, 20, 10])],
        spec(SecretPolicy::Absent),
        None,
        &limits,
    )
    .unwrap();

    assert_eq!(measurement.enrollment_yielded.get(), 3);
    assert_eq!(crypto.keys.created.get(), 3);
    assert_eq!(enrollment.commitment().leaf_count, 2);
    crypto.assert_keys_dropped();

    let distinct = enrollment::enroll(
        &measurement,
        &crypto,
        &[capture(&[20, 10])],
        spec(SecretPolicy::Absent),
        None,
        &limits,
    )
    .unwrap();

    assert_eq!(enrollment.commitment(), distinct.commitment());

    for member in [10, 20] {
        let signed = authorization::sign(
            &measurement,
            &crypto,
            &enrollment,
            &capture(&[member]),
            action(),
            None,
            &limits,
        )
        .unwrap();

        assert!(
            authorization::verify(
                &crypto,
                enrollment.commitment(),
                EnrollmentStatus::Active,
                &action(),
                &signed,
                10,
                &limits,
            )
            .is_ok()
        );
    }
}

#[test]
fn enrollment_overflow_including_duplicate_attempts_never_returns_a_partial_commitment() {
    for symbols in [&[10, 20, 30][..], &[10, 10, 10][..]] {
        let measurement = SyntheticMeasurements::new();
        let crypto = SyntheticCrypto::new();
        let mut limits = limits();
        limits.candidate_attempts = NonZeroUsize::new(2).unwrap();
        let result = enrollment::enroll(
            &measurement,
            &crypto,
            &[capture(symbols)],
            spec(SecretPolicy::Absent),
            None,
            &limits,
        );

        assert!(matches!(
            result,
            Err(Error::LimitExceeded(Resource::CandidateAttempts))
        ));
        // One additional stream item is needed to distinguish exact exhaustion
        // from overflow. It is never passed to key derivation.
        assert_eq!(measurement.enrollment_yielded.get(), 3);
        assert_eq!(crypto.keys.created.get(), 2);
        crypto.assert_keys_dropped();
    }
}

#[test]
fn reconstruction_accepts_the_last_allowed_match_and_stops_after_membership() {
    let mut fixture = Fixture::new();
    fixture.limits.candidate_attempts = NonZeroUsize::new(2).unwrap();
    let signed = fixture.sign(&[99, 20, 98]).unwrap();

    assert!(fixture.verify(&signed).is_ok());
    assert_eq!(fixture.measurement.reconstruction_yielded.get(), 2);
    fixture.crypto.assert_keys_dropped();
}

#[test]
fn reconstruction_distinguishes_complete_miss_from_attempt_overflow() {
    let cases = [(&[99, 98][..], false), (&[99, 99, 20][..], true)];

    for (symbols, overflows) in cases {
        let mut fixture = Fixture::new();
        fixture.limits.candidate_attempts = NonZeroUsize::new(2).unwrap();
        let error = failure(fixture.sign(symbols));

        if overflows {
            assert!(matches!(
                error,
                Error::LimitExceeded(Resource::CandidateAttempts)
            ));
            assert_eq!(fixture.measurement.reconstruction_yielded.get(), 3);
        } else {
            assert!(matches!(error, Error::CandidateNotEnrolled));
            assert_eq!(fixture.measurement.reconstruction_yielded.get(), 2);
        }

        assert_eq!(fixture.crypto.signing_calls.get(), 0);
        assert_eq!(fixture.crypto.keys.created.get(), 5);
        fixture.crypto.assert_keys_dropped();
    }
}

#[test]
fn maximum_context_and_action_limits_do_not_overflow_small_actual_messages() {
    let measurement = SyntheticMeasurements::new();
    let crypto = SyntheticCrypto::new();
    let mut limits = limits();
    limits.context_bytes = NonZeroUsize::MAX;
    limits.action_bytes = NonZeroUsize::MAX;
    let enrollment = enrollment::enroll(
        &measurement,
        &crypto,
        &[capture(&[10, 20])],
        spec(SecretPolicy::Absent),
        None,
        &limits,
    )
    .unwrap();
    let signed = authorization::sign(
        &measurement,
        &crypto,
        &enrollment,
        &capture(&[20]),
        action(),
        None,
        &limits,
    )
    .unwrap();

    assert!(
        authorization::verify(
            &crypto,
            enrollment.commitment(),
            EnrollmentStatus::Active,
            &action(),
            &signed,
            10,
            &limits,
        )
        .is_ok()
    );
    crypto.assert_keys_dropped();
}
