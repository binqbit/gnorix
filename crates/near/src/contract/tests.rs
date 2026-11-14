use std::num::{NonZeroU64, NonZeroUsize};

use gnorix_core::{
    biometrics::Candidate,
    enrollment::{Domain, EnrollmentContext, EnrollmentSpec, enroll_candidates},
    package::EnrollmentPackage,
    policy::SecretPolicy,
};
use gnorix_example_support::example_crypto::{ExampleCrypto, SUITE_ID};

use super::*;

fn account(name: &str) -> AccountId {
    name.parse().unwrap()
}

fn runtime() -> Runtime {
    Runtime {
        network: "near:testnet".into(),
        contract_id: account("counter.testnet"),
        now_ms: 1_000,
    }
}

fn package(revision: u64) -> EnrollmentPackage {
    let mut limits = verification_limits();
    limits.candidate_attempts = NonZeroUsize::new(3).unwrap();
    let context = EnrollmentContext {
        spec: EnrollmentSpec {
            domain: Domain {
                network: "near:testnet".into(),
                verifier: "counter.testnet".into(),
            },
            subject: "alice.testnet".into(),
            id: [7; 32],
            revision: NonZeroU64::new(revision).unwrap(),
            public_salt: vec![8; 32],
            secret_policy: SecretPolicy::Absent,
        },
        profile_id: [9; 32],
        suite_id: SUITE_ID.into(),
    };
    let candidates = Box::new((1..=3).map(|value| Ok(Candidate::new(vec![value; 32]).unwrap())));
    let enrollment = enroll_candidates(&ExampleCrypto, candidates, context, None, &limits).unwrap();
    EnrollmentPackage::from_enrollment(&enrollment, &limits).unwrap()
}

fn signed(package: &EnrollmentPackage, nonce: u64) -> Authorization {
    signed_action(
        package,
        Action {
            operation: "counter.increment".into(),
            payload: 2_u64.to_be_bytes().to_vec(),
            nonce,
            expires_at_ms: 2_000,
        },
    )
}

fn signed_action(package: &EnrollmentPackage, action: Action) -> Authorization {
    let candidates = Box::new(std::iter::once(Ok(Candidate::new(vec![2; 32]).unwrap())));
    let authorization = authorization::sign_candidates(
        &ExampleCrypto,
        candidates,
        [9; 32],
        package,
        action.to_core().unwrap(),
        None,
        &verification_limits(),
    )
    .unwrap();
    Authorization::from_core(&authorization).unwrap()
}

fn record(package: &EnrollmentPackage) -> Enrollment {
    Enrollment::register_at(
        Commitment::from_core(package.commitment()).unwrap(),
        SUITE_ID,
        &runtime(),
        &account("alice.testnet"),
    )
    .unwrap()
}

#[test]
fn verification_accepts_application_defined_operations_and_binary_payloads() {
    let package = package(1);
    let mut record = record(&package);
    let payload = b"Terms version 2\0\xff";
    let authorization = signed_action(
        &package,
        Action {
            operation: "document.approve".into(),
            payload: payload.to_vec(),
            nonce: 0,
            expires_at_ms: 2_000,
        },
    );
    let original = borsh::to_vec(&record).unwrap();

    assert_eq!(
        record
            .verify_at("document.approve", payload, &authorization, &runtime())
            .unwrap(),
        1,
    );
    assert!(
        record
            .verify_at("document.reject", payload, &authorization, &runtime())
            .is_err()
    );
    assert!(
        record
            .verify_at(
                "document.approve",
                b"Terms version 2\0",
                &authorization,
                &runtime()
            )
            .is_err()
    );
    assert_eq!(borsh::to_vec(&record).unwrap(), original);

    record
        .verify_and_consume_at("document.approve", payload, &authorization, &runtime())
        .unwrap();
    assert_eq!(record.view().next_nonce, 1);
}

#[test]
fn repeated_read_only_verification_does_not_change_enrollment_state() {
    let package = package(1);
    let record = record(&package);
    let authorization = signed(&package, 0);
    let original = borsh::to_vec(&record).unwrap();

    for _ in 0..2 {
        let next_nonce = record
            .verify_at(
                "counter.increment",
                &2_u64.to_be_bytes(),
                &authorization,
                &runtime(),
            )
            .unwrap();

        assert_eq!(next_nonce, 1);
        assert_eq!(record.view().next_nonce, 0);
        assert_eq!(borsh::to_vec(&record).unwrap(), original);
    }
}

#[test]
fn invalid_read_only_verification_does_not_change_enrollment_state() {
    let package = package(1);
    let record = record(&package);
    let mut authorization = signed(&package, 0);
    authorization.signature[0] ^= 1;
    let original = borsh::to_vec(&record).unwrap();

    assert!(
        record
            .verify_at(
                "counter.increment",
                &2_u64.to_be_bytes(),
                &authorization,
                &runtime(),
            )
            .is_err()
    );
    assert_eq!(borsh::to_vec(&record).unwrap(), original);
}

#[test]
fn consumed_authorization_fails_read_only_verification() {
    let package = package(1);
    let mut record = record(&package);
    let authorization = signed(&package, 0);
    record
        .verify_and_consume_at(
            "counter.increment",
            &2_u64.to_be_bytes(),
            &authorization,
            &runtime(),
        )
        .unwrap();
    let consumed = borsh::to_vec(&record).unwrap();

    assert!(
        record
            .verify_at(
                "counter.increment",
                &2_u64.to_be_bytes(),
                &authorization,
                &runtime(),
            )
            .is_err()
    );
    assert_eq!(record.view().next_nonce, 1);
    assert_eq!(borsh::to_vec(&record).unwrap(), consumed);
    assert_eq!(
        record
            .verify_at(
                "counter.increment",
                &2_u64.to_be_bytes(),
                &signed(&package, 1),
                &runtime(),
            )
            .unwrap(),
        2,
    );
    assert_eq!(borsh::to_vec(&record).unwrap(), consumed);
}

#[test]
fn relayer_authorizes_once_and_roundtrips_persistent_replay_state() {
    let package = package(1);
    let mut record = record(&package);
    let authorization = signed(&package, 0);
    let caller = runtime();
    record
        .verify_and_consume_at(
            "counter.increment",
            &2_u64.to_be_bytes(),
            &authorization,
            &caller,
        )
        .unwrap();
    assert_eq!(record.view().next_nonce, 1);

    let bytes = borsh::to_vec(&record).unwrap();
    let mut restored: Enrollment = borsh::from_slice(&bytes).unwrap();
    assert!(
        restored
            .verify_and_consume_at(
                "counter.increment",
                &2_u64.to_be_bytes(),
                &authorization,
                &caller
            )
            .is_err()
    );
    assert_eq!(restored.view().next_nonce, 1);
    restored
        .verify_and_consume_at(
            "counter.increment",
            &2_u64.to_be_bytes(),
            &signed(&package, 1),
            &caller,
        )
        .unwrap();
    assert_eq!(restored.view().next_nonce, 2);
}

#[test]
fn tampering_never_consumes_a_nonce() {
    let package = package(1);
    let mut record = record(&package);
    let original = signed(&package, 0);
    let mutate: &[fn(&mut Authorization)] = &[
        |value| value.signature[0] ^= 1,
        |value| value.public_key[0] ^= 1,
        |value| value.siblings[0][0] ^= 1,
        |value| value.leaf_count += 1,
        |value| value.leaf_index = value.leaf_count,
        |value| value.siblings.push([0; 32]),
        |value| value.enrollment_id[0] ^= 1,
        |value| value.revision += 1,
        |value| value.action.nonce += 1,
        |value| value.action.expires_at_ms += 1,
        |value| value.action.operation.push('x'),
        |value| value.action.payload[0] ^= 1,
    ];
    for mutation in mutate {
        let mut forged = original.clone();
        mutation(&mut forged);
        assert!(
            record
                .verify_and_consume_at(
                    "counter.increment",
                    &2_u64.to_be_bytes(),
                    &forged,
                    &runtime()
                )
                .is_err()
        );
        assert_eq!(record.next_nonce, 0);
    }
    assert!(
        record
            .verify_and_consume_at(
                "other.operation",
                &2_u64.to_be_bytes(),
                &original,
                &runtime()
            )
            .is_err()
    );
    assert!(
        record
            .verify_and_consume_at(
                "counter.increment",
                &3_u64.to_be_bytes(),
                &original,
                &runtime()
            )
            .is_err()
    );
}

#[test]
fn network_contract_deadline_and_overflow_are_enforced() {
    let package = package(1);
    let mut record = record(&package);
    let authorization = signed(&package, 0);
    let mut wrong_network = runtime();
    wrong_network.network = "near:mainnet".into();
    let mut wrong_contract = runtime();
    wrong_contract.contract_id = "other.testnet".parse().unwrap();
    let mut expired = runtime();
    expired.now_ms = authorization.action.expires_at_ms;
    for caller in [wrong_network, wrong_contract, expired] {
        assert!(
            record
                .verify_and_consume_at(
                    "counter.increment",
                    &2_u64.to_be_bytes(),
                    &authorization,
                    &caller
                )
                .is_err()
        );
        assert_eq!(record.next_nonce, 0);
    }
    record.next_nonce = u64::MAX;
    assert!(
        record
            .verify_and_consume_at(
                "counter.increment",
                &2_u64.to_be_bytes(),
                &signed(&package, u64::MAX),
                &runtime()
            )
            .is_err()
    );
    assert_eq!(record.next_nonce, u64::MAX);
}

#[test]
fn only_owner_can_register_rotate_or_revoke_and_rotation_invalidates_old_proofs() {
    let initial = package(1);
    let replacement = package(2);
    let commitment = Commitment::from_core(initial.commitment()).unwrap();
    assert!(
        Enrollment::register_at(
            commitment.clone(),
            SUITE_ID,
            &runtime(),
            &account("mallory.testnet")
        )
        .is_err()
    );
    assert!(
        Enrollment::register_at(
            commitment.clone(),
            "unrelated-suite",
            &runtime(),
            &account("alice.testnet"),
        )
        .is_err()
    );
    let mut other_domain = commitment.clone();
    other_domain.contract_id = "other.testnet".parse().unwrap();
    assert!(
        Enrollment::register_at(
            other_domain,
            SUITE_ID,
            &runtime(),
            &account("alice.testnet")
        )
        .is_err()
    );
    let mut record = record(&initial);
    assert!(record.revoke_as(&account("mallory.testnet")).is_err());
    assert!(
        record
            .replace_at(
                Commitment::from_core(replacement.commitment()).unwrap(),
                &runtime(),
                &account("mallory.testnet"),
            )
            .is_err()
    );
    assert!(
        record
            .replace_at(commitment, &runtime(), &account("alice.testnet"))
            .is_err()
    );
    record.revoke_as(&account("alice.testnet")).unwrap();
    assert!(!record.view().active);
    assert!(
        record
            .verify_and_consume_at(
                "counter.increment",
                &2_u64.to_be_bytes(),
                &signed(&initial, 0),
                &runtime()
            )
            .is_err()
    );
    record
        .replace_at(
            Commitment::from_core(replacement.commitment()).unwrap(),
            &runtime(),
            &account("alice.testnet"),
        )
        .unwrap();
    assert!(record.view().active);
    assert!(
        record
            .verify_and_consume_at(
                "counter.increment",
                &2_u64.to_be_bytes(),
                &signed(&initial, 0),
                &runtime()
            )
            .is_err()
    );
    record
        .verify_and_consume_at(
            "counter.increment",
            &2_u64.to_be_bytes(),
            &signed(&replacement, 0),
            &runtime(),
        )
        .unwrap();
}

#[test]
fn strict_verifier_rejects_weak_identity_keys_and_malformed_signatures() {
    let verifier = Ed25519Verifier::new(SUITE_ID);
    let mut identity = [0; 32];
    identity[0] = 1;
    let mut signature = [0; 64];
    signature[0] = 1;
    assert!(
        !verifier
            .verify(&identity, b"any message", &signature)
            .unwrap()
    );
    assert!(!verifier.verify(&[0; 31], b"message", &signature).unwrap());
    assert!(!verifier.verify(&identity, b"message", &[0; 63]).unwrap());
}
