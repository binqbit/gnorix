use super::*;

#[test]
fn image_selection_rejects_unknown_sources() {
    assert!(image(0).is_some());
    assert!(image(1).is_some());
    assert!(image(2).is_none());
    assert!(FingerprintTemplate::prepare(2, None).is_err());
}

#[test]
#[ignore = "runs real extraction and an extended cryptographic candidate search"]
fn reported_partial_capture_signs_with_enlarged_search_budget() {
    let wallet = FingerprintTemplate::prepare(0, None)
        .unwrap()
        .enroll("near:regression", "wallet.fixture")
        .unwrap();
    // This exact capture reached the former 1m work limit with 12 usable
    // features. Its selection remains independent of the signing outcome.
    let sampled = wallet.sample(457).unwrap();
    assert_eq!(sampled.metadata.circle.x, 2.0);
    assert_eq!(sampled.metadata.circle.y, 134.0);
    assert_eq!(sampled.metadata.circle.radius, 123.00406497347964);
    let action = Action {
        operation: "wallet.ft_transfer.v1".into(),
        payload: b"reported-partial-capture-regression".to_vec(),
        nonce: 0,
        expires_at_ms: 2_000_000_000_000,
    };

    let signed = wallet
        .sign(
            action.clone(),
            &sampled,
            |points| assert_eq!(points.len(), 12),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(signed.attempts > 0);
    assert!(!signed.matched_feature_ids.is_empty());

    assert_signed_action(&wallet, action, signed);
}

#[test]
#[ignore = "extracts both the enrollment and reported partial scan and searches real keys"]
fn reported_five_point_capture_signs_with_local_coverage() {
    let wallet = FingerprintTemplate::prepare(1, Some(5))
        .unwrap()
        .enroll("near:regression", "wallet.fixture")
        .unwrap();
    // Seed reproduces the reported circle, selected by its center rather
    // than by a successful signing outcome. This crop previously had no
    // registered descriptor even after its complete positional traversal.
    let sampled = wallet.sample(118_278).unwrap();
    assert_eq!(sampled.metadata.circle.x, 235.0);
    assert_eq!(sampled.metadata.circle.y, 131.0);
    assert_eq!(sampled.metadata.circle.radius, 119.26860441876563);
    let action = Action {
        operation: "wallet.ft_transfer.v1".into(),
        payload: b"reported-five-point-capture".to_vec(),
        nonce: 0,
        expires_at_ms: 2_000_000_000_000,
    };
    let signed = wallet
        .sign(
            action.clone(),
            &sampled,
            |points| assert_eq!(points.len(), 13),
            &AtomicBool::new(false),
        )
        .unwrap();
    // These five observed points correspond to a compact enrolled group.
    // Midpoint coverage makes this group available earlier in the search.
    assert_eq!(signed.matched_feature_ids, [29, 36, 43, 47, 50]);
    assert_signed_action(&wallet, action, signed);
}

#[test]
#[ignore = "extracts the exact hosted partial scan and searches real signing keys"]
fn reported_eight_feature_capture_reconstructs_a_five_point_key() {
    let wallet = FingerprintTemplate::prepare(0, Some(5))
        .unwrap()
        .enroll("near:regression", "wallet.fixture")
        .unwrap();
    // Reproduce the reported center, independently of the signing outcome.
    let sampled = wallet.sample(67_294).unwrap();
    assert_eq!(sampled.metadata.circle.x, 103.0);
    assert_eq!(sampled.metadata.circle.y, 153.0);
    assert_eq!(sampled.metadata.circle.radius, 82.3286098510111);
    let action = Action {
        operation: "wallet.ft_transfer.v1".into(),
        payload: b"reported-eight-feature-capture".to_vec(),
        nonce: 0,
        expires_at_ms: 2_000_000_000_000,
    };
    let signed = wallet
        .sign(
            action.clone(),
            &sampled,
            |points| {
                assert_eq!(
                    points.iter().map(|point| point.id).collect::<Vec<_>>(),
                    [6, 11, 13, 14, 32, 38, 43, 45]
                );
            },
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(signed.matched_feature_ids, [11, 13, 38, 43, 45]);
    assert_signed_action(&wallet, action, signed);
}

fn assert_signed_action(wallet: &FingerprintWallet, action: Action, signed: SignedFingerprint) {
    let commitment = wallet.commitment().unwrap().to_core().unwrap();
    let authorization = signed.authorization.to_core().unwrap();
    let mut expected = action.to_core().unwrap();
    let verify = |expected: &gnorix::core::authorization::Action| {
        authorization::verify(
            &ExampleCrypto,
            &commitment,
            authorization::EnrollmentStatus::Active,
            expected,
            &authorization,
            1_900_000_000_000,
            &verification_limits(),
        )
    };
    // Verify descriptor membership and the action signature. This does not
    // assert correspondence to the same physical minutiae in both scans.
    verify(&expected).unwrap();
    expected.payload.push(0);
    assert!(verify(&expected).is_err());
}

/// A small canonical grid fixture keeps context-isolation checks independent of
/// expensive image extraction. Capture metadata still refers to the real PNG.
fn template_fixture() -> FingerprintTemplate {
    use fingerprint::{GridCell, grid_candidate};
    let source = source_image(0).unwrap();
    let view = FingerprintView {
        image_url: "/api/fingerprints/0".into(),
        width: source.width(),
        height: source.height(),
        feature_count: 0,
        points: Vec::new(),
    };
    FingerprintTemplate {
        fingerprint: Arc::new(PreparedFingerprint {
            image_index: 0,
            parameters: ProfileParameters::new(&combination_config(None), UNCERTAINTY_STEPS)
                .unwrap(),
            pattern: PatternGeometry::new(
                source.width(),
                source.height(),
                &vec![true; source.as_raw().len()],
            )
            .unwrap(),
            view,
        }),
        candidates: vec![
            grid_candidate(
                &[
                    GridCell { column: 1, row: 1 },
                    GridCell { column: 9, row: 1 },
                    GridCell { column: 5, row: 8 },
                ],
                0,
                nonzero(MAX_DESCRIPTOR_BYTES),
            )
            .unwrap(),
        ],
    }
}

#[test]
fn shared_template_produces_isolated_session_commitments() {
    let template = template_fixture();
    let first = template
        .enroll("near:regression", "wallet.fixture")
        .unwrap();
    let second = template
        .enroll("near:regression", "wallet.fixture")
        .unwrap();
    assert_shared_capture_and_distinct_enrollment(&template, &first, &second);

    let action = Action {
        operation: "wallet.ft_transfer.v1".into(),
        payload: b"session-isolation".to_vec(),
        nonce: 0,
        expires_at_ms: 2_000_000_000_000,
    }
    .to_core()
    .unwrap();
    let candidate = Candidate::new(template.candidates[0].as_bytes().to_vec()).unwrap();
    let signed = authorization::sign_candidates(
        &ExampleCrypto,
        Box::new(std::iter::once(Ok(candidate))),
        first.fingerprint.parameters.profile_id(),
        &first.package,
        action.clone(),
        None,
        &limits(1),
    )
    .unwrap();
    let verify = |wallet: &FingerprintWallet| {
        authorization::verify(
            &ExampleCrypto,
            wallet.package.commitment(),
            authorization::EnrollmentStatus::Active,
            &action,
            &signed,
            1_900_000_000_000,
            &verification_limits(),
        )
        .map(|_| ())
    };
    verify(&first).unwrap();
    assert!(verify(&second).is_err());
}

#[test]
fn cancelled_signing_does_not_extract_or_publish_features() {
    let wallet = template_fixture()
        .enroll("near:regression", "wallet.fixture")
        .unwrap();
    let sample = wallet.sample(17).unwrap();
    let action = Action {
        operation: "wallet.ft_transfer.v1".into(),
        payload: b"cancelled".to_vec(),
        nonce: 0,
        expires_at_ms: 2_000_000_000_000,
    };
    let result = wallet.sign(
        action,
        &sample,
        |_| panic!("cancelled capture must not publish features"),
        &AtomicBool::new(true),
    );
    assert!(result.err().unwrap().to_string().contains("cancelled"));
}

#[test]
#[ignore = "runs real enrollment extraction once and derives two session packages"]
fn real_fingerprint_preparation_is_reused_between_sessions() {
    let template = FingerprintTemplate::prepare(0, Some(5)).unwrap();
    let first = template
        .enroll("near:regression", "wallet.fixture")
        .unwrap();
    let second = template
        .enroll("near:regression", "wallet.fixture")
        .unwrap();
    assert!(first.view().feature_count > 0);
    assert!(first.package.commitment().leaf_count > 1);
    assert_shared_capture_and_distinct_enrollment(&template, &first, &second);
}

fn assert_shared_capture_and_distinct_enrollment(
    template: &FingerprintTemplate,
    first: &FingerprintWallet,
    second: &FingerprintWallet,
) {
    assert!(Arc::ptr_eq(&template.fingerprint, &first.fingerprint));
    assert!(Arc::ptr_eq(&first.fingerprint, &second.fingerprint));
    let first = first.package.commitment();
    let second = second.package.commitment();
    assert_eq!(first.context.profile_id, second.context.profile_id);
    assert_eq!(first.leaf_count, second.leaf_count);
    assert_ne!(first.context.spec.id, second.context.spec.id);
    assert_ne!(
        first.context.spec.public_salt,
        second.context.spec.public_salt
    );
    assert_ne!(first.root, second.root);
}

#[test]
fn cancellation_stops_the_stream_before_more_candidates_are_evaluated() {
    let cancelled = AtomicBool::new(false);
    let generated = Cell::new(0);
    let attempts = Cell::new(0);
    let source = std::iter::from_fn(|| {
        generated.set(generated.get() + 1);
        Some(Ok(Candidate::new(vec![1]).unwrap()))
    });
    let mut candidates = signing_candidates(source, &cancelled, &attempts);
    assert!(candidates.next().unwrap().is_ok());
    cancelled.store(true, Ordering::Relaxed);
    assert!(
        candidates
            .next()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert!(candidates.next().is_none());
    assert_eq!(attempts.get(), 1);
    assert_eq!(generated.get(), 1);
}

#[test]
fn cancellation_during_generation_does_not_release_the_descriptor() {
    let cancelled = AtomicBool::new(false);
    let attempts = Cell::new(0);
    let source = std::iter::once_with(|| {
        cancelled.store(true, Ordering::Relaxed);
        Ok(Candidate::new(vec![1]).unwrap())
    });
    let mut candidates = signing_candidates(source, &cancelled, &attempts);
    assert!(
        candidates
            .next()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert!(candidates.next().is_none());
    assert_eq!(attempts.get(), 0);
}
