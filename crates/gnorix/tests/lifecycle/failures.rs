//! Algorithm failure propagation, resource bounds and safe diagnostics.

use std::{error::Error as _, num::NonZeroUsize};

use gnorix::{
    Error,
    biometrics::Candidate,
    crypto::SecretInput,
    enrollment,
    error::{Resource, Stage},
    policy::SecretPolicy,
};

use crate::support::{
    CryptoFault, Fixture, FixtureError, MeasurementFault, SyntheticCrypto, SyntheticMeasurements,
    capture, failure, limits, spec,
};

fn assert_algorithm_error<T>(
    result: gnorix::Result<T>,
    expected_stage: Stage,
    expected_source: &str,
) {
    let error = failure(result);

    assert!(matches!(&error, Error::Algorithm { stage, .. } if *stage == expected_stage));
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<FixtureError>()
            .unwrap()
            .0,
        expected_source
    );
}

#[test]
fn preparation_and_generation_errors_keep_their_cause_and_discard_partial_work() {
    let cases = [
        (
            MeasurementFault::Prepare,
            Stage::EnrollmentPreparation,
            "prepare",
            0,
        ),
        (
            MeasurementFault::EnrollmentStart,
            Stage::CandidateGeneration,
            "enrollment start",
            0,
        ),
        (
            MeasurementFault::EnrollmentItem(1),
            Stage::CandidateGeneration,
            "enrollment item",
            1,
        ),
    ];

    for (fault, stage, source, created_keys) in cases {
        let mut measurement = SyntheticMeasurements::new();
        measurement.fault = Some(fault);
        let crypto = SyntheticCrypto::new();
        let result = enrollment::enroll(
            &measurement,
            &crypto,
            &[capture(&[10, 20])],
            spec(SecretPolicy::Absent),
            None,
            &limits(),
        );

        assert_algorithm_error(result, stage, source);
        assert_eq!(crypto.keys.created.get(), created_keys);
        crypto.assert_keys_dropped();
    }
}

#[test]
fn reconstruction_start_and_stream_errors_keep_their_cause_and_drop_nonmembers() {
    for (fault, source, new_keys) in [
        (
            MeasurementFault::ReconstructionStart,
            "reconstruction start",
            0,
        ),
        (
            MeasurementFault::ReconstructionItem(1),
            "reconstruction item",
            1,
        ),
    ] {
        let mut fixture = Fixture::new();
        fixture.measurement.fault = Some(fault);

        assert_algorithm_error(fixture.sign(&[99, 20]), Stage::Reconstruction, source);
        assert_eq!(fixture.crypto.keys.created.get(), 3 + new_keys);
        assert_eq!(fixture.crypto.signing_calls.get(), 0);
        fixture.crypto.assert_keys_dropped();
    }
}

#[test]
fn crypto_errors_preserve_stage_and_source_and_drop_temporary_keys() {
    for (fault, stage, source, new_keys) in [
        (CryptoFault::Derive, Stage::KeyDerivation, "derive", 0),
        (CryptoFault::PublicKey, Stage::PublicKey, "public key", 1),
        (CryptoFault::Sign, Stage::Signing, "sign", 1),
    ] {
        let mut fixture = Fixture::new();
        fixture.crypto.fault = Some(fault);

        assert_algorithm_error(fixture.sign(&[20]), stage, source);
        assert_eq!(fixture.crypto.keys.created.get(), 3 + new_keys);
        fixture.crypto.assert_keys_dropped();
    }

    let mut fixture = Fixture::new();
    let signed = fixture.sign(&[20]).unwrap();
    fixture.crypto.fault = Some(CryptoFault::Verify);

    assert_algorithm_error(
        fixture.verify(&signed),
        Stage::SignatureVerification,
        "verify",
    );
    fixture.crypto.assert_keys_dropped();
}

#[test]
fn output_size_failures_also_drop_derived_keys() {
    let mut fixture = Fixture::new();
    fixture.limits.public_key_bytes = NonZeroUsize::new(31).unwrap();

    assert!(matches!(
        fixture.sign(&[20]),
        Err(Error::LimitExceeded(Resource::PublicKeyBytes))
    ));
    assert_eq!(fixture.crypto.signing_calls.get(), 0);
    fixture.crypto.assert_keys_dropped();

    fixture.limits.public_key_bytes = NonZeroUsize::new(32).unwrap();
    fixture.limits.signature_bytes = NonZeroUsize::new(31).unwrap();

    assert!(matches!(
        fixture.sign(&[20]),
        Err(Error::LimitExceeded(Resource::SignatureBytes))
    ));
    assert_eq!(fixture.crypto.signing_calls.get(), 1);
    fixture.crypto.assert_keys_dropped();
}

#[test]
fn oversized_context_and_action_reject_before_candidate_derivation() {
    let measurement = SyntheticMeasurements::new();
    let crypto = SyntheticCrypto::new();
    let mut limits = limits();
    limits.context_bytes = NonZeroUsize::new(1).unwrap();

    let result = enrollment::enroll(
        &measurement,
        &crypto,
        &[capture(&[10])],
        spec(SecretPolicy::Absent),
        None,
        &limits,
    );

    assert!(matches!(
        result,
        Err(Error::LimitExceeded(Resource::ContextBytes))
    ));
    assert_eq!(crypto.keys.created.get(), 0);
    assert_eq!(measurement.enrollment_yielded.get(), 0);

    let mut fixture = Fixture::new();
    fixture.limits.action_bytes = NonZeroUsize::new(1).unwrap();

    assert!(matches!(
        fixture.sign(&[20]),
        Err(Error::LimitExceeded(Resource::ActionBytes))
    ));
    assert_eq!(fixture.measurement.reconstruction_yielded.get(), 0);
    assert_eq!(fixture.crypto.keys.created.get(), 3);
}

#[test]
fn candidate_and_secret_debug_output_do_not_expose_their_contents() {
    let bytes = b"sensitive-test-material";
    let candidate = Candidate::new(bytes.to_vec()).unwrap();
    let secret = SecretInput::new(bytes.to_vec()).unwrap();
    let text = std::str::from_utf8(bytes).unwrap();
    let byte_dump = format!("{bytes:?}");

    for debug in [format!("{candidate:?}"), format!("{secret:?}")] {
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(text));
        assert!(!debug.contains(&byte_dump));
    }
}
