use gnorix_near::{Action, Authorization, Commitment, EnrollmentView};
use serde_json::json;

fn commitment() -> Commitment {
    Commitment {
        network: "near:testnet".into(),
        contract_id: "verifier.testnet".parse().unwrap(),
        owner_id: "owner.testnet".parse().unwrap(),
        enrollment_id: [1; 32],
        revision: u64::MAX,
        public_salt: vec![2; 32],
        secret_required: true,
        profile_id: [3; 32],
        suite_id: "application.hkdf-ed25519.v1".into(),
        root: [4; 32],
        leaf_count: u64::MAX,
    }
}

fn action() -> Action {
    Action {
        operation: "set_message".into(),
        payload: b"approved payload".to_vec(),
        nonce: u64::MAX,
        expires_at_ms: u64::MAX,
    }
}

fn authorization() -> Authorization {
    Authorization {
        enrollment_id: [1; 32],
        revision: u64::MAX,
        action: action(),
        public_key: [3; 32],
        signature: vec![4; 64],
        leaf_index: u64::MAX - 1,
        leaf_count: u64::MAX,
        siblings: vec![[5; 32]; 64],
    }
}

#[test]
fn core_roundtrips_preserve_every_signed_field() {
    let commitment = commitment();
    let authorization = authorization();
    assert_eq!(
        Commitment::from_core(&commitment.to_core().unwrap()).unwrap(),
        commitment
    );
    assert_eq!(
        Authorization::from_core(&authorization.to_core().unwrap()).unwrap(),
        authorization
    );
}

#[test]
fn json_preserves_large_integers_as_decimal_strings() {
    let authorization = authorization();
    let encoded = serde_json::to_value(&authorization).unwrap();
    assert_eq!(encoded["revision"], json!(u64::MAX.to_string()));
    assert_eq!(encoded["action"]["nonce"], json!(u64::MAX.to_string()));
    assert_eq!(encoded["leaf_count"], json!(u64::MAX.to_string()));
    assert_eq!(
        serde_json::from_value::<Authorization>(encoded).unwrap(),
        authorization
    );

    let view = EnrollmentView {
        commitment: commitment(),
        active: true,
        next_nonce: u64::MAX,
    };
    let encoded = serde_json::to_value(&view).unwrap();
    assert_eq!(encoded["next_nonce"], json!(u64::MAX.to_string()));
    assert_eq!(
        serde_json::from_value::<EnrollmentView>(encoded).unwrap(),
        view
    );
}

#[test]
fn json_rejects_noncanonical_integer_representations() {
    for invalid in [
        json!(1),
        json!("01"),
        json!("+1"),
        json!("-1"),
        json!("18446744073709551616"),
    ] {
        let mut encoded = serde_json::to_value(action()).unwrap();
        encoded["nonce"] = invalid;
        assert!(serde_json::from_value::<Action>(encoded).is_err());
    }
}

#[test]
fn json_rejects_oversized_arrays_and_strings() {
    for (field, value) in [
        ("signature", json!(vec![0; 65])),
        ("siblings", json!(vec![[0; 32]; 65])),
        ("public_key", json!(vec![0; 33])),
    ] {
        let mut encoded = serde_json::to_value(authorization()).unwrap();
        encoded[field] = value;
        assert!(
            serde_json::from_value::<Authorization>(encoded).is_err(),
            "{field}"
        );
    }
    for (field, value) in [
        ("payload", json!(vec![0; 4097])),
        ("operation", json!("o".repeat(129))),
    ] {
        let mut encoded = serde_json::to_value(action()).unwrap();
        encoded[field] = value;
        assert!(
            serde_json::from_value::<Action>(encoded).is_err(),
            "{field}"
        );
    }
    for (field, value) in [
        ("public_salt", json!(vec![0; 65])),
        ("network", json!("n".repeat(65))),
        ("suite_id", json!("s".repeat(129))),
        ("owner_id", json!("a".repeat(65))),
        ("contract_id", json!("INVALID ACCOUNT")),
    ] {
        let mut encoded = serde_json::to_value(commitment()).unwrap();
        encoded[field] = value;
        assert!(
            serde_json::from_value::<Commitment>(encoded).is_err(),
            "{field}"
        );
    }
}

#[test]
fn json_rejects_unknown_fields() {
    let mut encoded = serde_json::to_value(authorization()).unwrap();
    encoded["root"] = json!(vec![0; 32]);
    assert!(serde_json::from_value::<Authorization>(encoded).is_err());
}

#[test]
fn borsh_roundtrips_and_rejects_oversized_length_before_reading_content() {
    let authorization = authorization();
    assert_eq!(
        borsh::from_slice::<Authorization>(&borsh::to_vec(&authorization).unwrap()).unwrap(),
        authorization
    );
    let commitment = commitment();
    assert_eq!(
        borsh::from_slice::<Commitment>(&borsh::to_vec(&commitment).unwrap()).unwrap(),
        commitment
    );

    let error = borsh::from_slice::<Action>(&u32::MAX.to_le_bytes()).unwrap_err();
    assert!(error.to_string().contains("size limit"));

    let mut payload_prefix = borsh::to_vec(&"operation".to_string()).unwrap();
    payload_prefix.extend_from_slice(&u32::MAX.to_le_bytes());
    let error = borsh::from_slice::<Action>(&payload_prefix).unwrap_err();
    assert!(error.to_string().contains("size limit"));
}

#[test]
fn core_conversion_rejects_foreign_nonce_and_ed25519_encodings() {
    let mut core = authorization().to_core().unwrap();
    core.action.nonce[0] = 1;
    assert!(Authorization::from_core(&core).is_err());
    core.action.nonce[0] = 0;
    core.public_key.pop();
    assert!(Authorization::from_core(&core).is_err());
    core.public_key.push(3);
    core.signature.pop();
    assert!(Authorization::from_core(&core).is_err());
}

#[test]
fn semantic_validation_rejects_invalid_domains_and_merkle_metadata() {
    let mut commitment = commitment();
    commitment.network = "testnet".into();
    assert!(commitment.to_core().is_err());
    commitment.network = "near:".into();
    assert!(commitment.to_core().is_err());
    commitment.network = "near:testnet".into();
    commitment.revision = 0;
    assert!(commitment.to_core().is_err());
    commitment.revision = 1;
    commitment.leaf_count = 0;
    assert!(commitment.to_core().is_err());

    let mut authorization = authorization();
    authorization.leaf_index = authorization.leaf_count;
    assert!(authorization.to_core().is_err());
    authorization.leaf_index = 0;
    authorization.signature.truncate(63);
    assert!(authorization.to_core().is_err());
}

#[test]
fn chain_ids_preserve_runtime_case_without_normalization() {
    let mut commitment = commitment();
    commitment.network = "near:test-chain-PxxRd".into();
    let core = commitment.to_core().unwrap();
    assert_eq!(core.context.spec.domain.network, "near:test-chain-PxxRd");
    assert_eq!(Commitment::from_core(&core).unwrap(), commitment);
    commitment.network.push('\n');
    assert!(commitment.to_core().is_err());
}
