use std::num::{NonZeroU64, NonZeroUsize};

use sha2::{Digest, Sha256};

use super::*;
use crate::{
    enrollment::{Domain, EnrollmentContext, EnrollmentSpec},
    merkle::{self, Hash, MerkleError},
    policy::SecretPolicy,
};

fn limits() -> Limits {
    Limits {
        candidate_attempts: NonZeroUsize::new(100).unwrap(),
        candidate_bytes: NonZeroUsize::new(256).unwrap(),
        public_key_bytes: NonZeroUsize::new(128).unwrap(),
        signature_bytes: NonZeroUsize::new(128).unwrap(),
        action_bytes: NonZeroUsize::new(4096).unwrap(),
        context_bytes: NonZeroUsize::new(4096).unwrap(),
    }
}

fn capacity() -> PackageLimits {
    PackageLimits {
        max_bytes: NonZeroUsize::new(1024 * 1024).unwrap(),
        max_leaves: NonZeroUsize::new(100).unwrap(),
    }
}

fn context() -> EnrollmentContext {
    EnrollmentContext {
        spec: EnrollmentSpec {
            domain: Domain {
                network: "test-network".into(),
                verifier: "test-verifier".into(),
            },
            subject: "test-subject".into(),
            id: [7; 32],
            revision: NonZeroU64::new(3).unwrap(),
            public_salt: b"test-salt".to_vec(),
            secret_policy: SecretPolicy::Absent,
        },
        profile_id: [8; 32],
        suite_id: "fixture-suite".into(),
    }
}

fn package(keys: &[&[u8]]) -> EnrollmentPackage {
    package_with_context(keys, context())
}

fn package_with_context(keys: &[&[u8]], context: EnrollmentContext) -> EnrollmentPackage {
    let encoded = context.encode(&limits()).unwrap();
    let tree =
        MerkleTree::from_leaves(keys.iter().map(|key| leaf_hash(&encoded, key)).collect()).unwrap();
    let enrollment = Enrollment {
        commitment: EnrollmentCommitment {
            context,
            root: tree.root(),
            leaf_count: tree.leaf_count() as u64,
        },
        model: b"this-reconstruction-model-must-not-be-stored",
        tree,
    };
    EnrollmentPackage::from_enrollment(&enrollment, &limits()).unwrap()
}

fn encoded(package: &EnrollmentPackage) -> Vec<u8> {
    package.encode(&capacity(), &limits()).unwrap()
}

fn root_offset(bytes: &[u8]) -> usize {
    let parameters = parameters_offset(bytes);
    parameters
        + 8
        + u64::from_be_bytes(bytes[parameters..parameters + 8].try_into().unwrap()) as usize
}

fn parameters_offset(bytes: &[u8]) -> usize {
    20 + u64::from_be_bytes(bytes[12..20].try_into().unwrap()) as usize
}

fn parameterized_package(parameters: &[u8]) -> EnrollmentPackage {
    let context = EnrollmentContext {
        profile_id: Sha256::digest(parameters).into(),
        ..context()
    };
    package_with_context(&[b"pub-A", b"pub-B", b"pub-C"], context)
        .with_profile_parameters(parameters.to_vec(), &limits())
        .unwrap()
}

#[test]
fn round_trip_restores_paths_from_hashes_for_singleton_even_and_odd_trees() {
    for count in [1, 2, 3, 5, 16, 65] {
        let keys: Vec<_> = (0..count)
            .map(|index| format!("public-key-{index}").into_bytes())
            .collect();
        let references: Vec<_> = keys.iter().map(Vec::as_slice).collect();
        let original = package(&references);
        let bytes = encoded(&original);
        let loaded = EnrollmentPackage::decode(&bytes, &capacity(), &limits()).unwrap();
        assert_eq!(loaded.commitment(), original.commitment());
        assert_eq!(loaded.profile_parameters(), None);
        assert_eq!(encoded(&loaded), bytes);
        let context = loaded.commitment.context.encode(&limits()).unwrap();
        for key in keys {
            let proof = loaded.proof_for(&key, &limits()).unwrap();
            assert_eq!(proof, original.proof_for(&key, &limits()).unwrap());
            merkle::verify(original.commitment.root, leaf_hash(&context, &key), &proof).unwrap();
        }
    }
}

#[test]
fn constant_profile_package_is_canonical_and_retains_only_context_and_hashes() {
    let a = package(&[b"pub-A", b"pub-B", b"pub-C"]);
    let b = package(&[b"pub-C", b"pub-B", b"pub-A", b"pub-A"]);
    assert_eq!(encoded(&a), encoded(&b));
    assert_eq!(a.filename(), b.filename());
    assert_eq!(a.filename().len(), 68);
    assert!(a.filename().ends_with(".grx"));
    assert!(
        a.filename()[..64]
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    );
    let bytes = encoded(&a);
    let leaves = &bytes[root_offset(&bytes) + 40..];
    assert_eq!(leaves, a.tree.leaves().concat());
    assert_eq!(leaves.len(), 3 * 32);
    assert!(!bytes.windows(5).any(|window| window == b"pub-A"));
    let model_marker = b"this-reconstruction-model-must-not-be-stored";
    assert!(
        !bytes
            .windows(model_marker.len())
            .any(|window| window == model_marker)
    );
}

#[test]
fn package_bytes_match_an_independent_python_hashlib_vector() {
    let bytes = encoded(&package(&[b"pub-A", b"pub-B", b"pub-C"]));
    // Derived independently from the documented framing and Merkle domains.
    let expected: Hash = [
        29, 21, 230, 120, 76, 188, 207, 38, 20, 18, 233, 105, 40, 104, 106, 6, 2, 124, 9, 152, 5,
        247, 71, 155, 1, 66, 149, 24, 165, 101, 23, 35,
    ];
    assert_eq!(Hash::from(Sha256::digest(&bytes)), expected);

    let parameters = b"gnorix.test.profile.v1\0minimum_pair_span_r=21.5";
    let with_parameters = encoded(&parameterized_package(parameters));
    let expected: Hash = [
        144, 252, 157, 41, 29, 108, 224, 48, 128, 133, 219, 126, 77, 127, 186, 128, 132, 85, 34,
        54, 118, 122, 160, 7, 157, 115, 184, 119, 58, 126, 164, 94,
    ];
    assert_eq!(Hash::from(Sha256::digest(&with_parameters)), expected);
}

#[test]
fn rejects_every_truncation_and_trailing_data() {
    let bytes = encoded(&package(&[b"pub-A", b"pub-B", b"pub-C"]));
    for length in 0..bytes.len() {
        assert!(
            EnrollmentPackage::decode(&bytes[..length], &capacity(), &limits()).is_err(),
            "length {length}"
        );
    }
    let mut extended = bytes;
    extended.push(0);
    assert!(EnrollmentPackage::decode(&extended, &capacity(), &limits()).is_err());
}

#[test]
fn rejects_changed_magic_version_root_count_and_noncanonical_leaves() {
    let bytes = encoded(&package(&[b"pub-A", b"pub-B", b"pub-C"]));
    let root = root_offset(&bytes);
    let count = root + 32;
    let leaves = count + 8;
    let mut corruptions = Vec::new();
    for index in [0, 11, root, leaves] {
        let mut changed = bytes.clone();
        changed[index] ^= 1;
        corruptions.push(changed);
    }
    for wrong_count in [0_u64, 2, 4, u64::MAX] {
        let mut changed = bytes.clone();
        changed[count..leaves].copy_from_slice(&wrong_count.to_be_bytes());
        corruptions.push(changed);
    }
    let mut duplicate = bytes.clone();
    duplicate.copy_within(leaves..leaves + 32, leaves + 32);
    corruptions.push(duplicate);
    let mut unsorted = bytes.clone();
    for index in 0..32 {
        unsorted.swap(leaves + index, leaves + 32 + index);
    }
    corruptions.push(unsorted);
    for changed in corruptions {
        assert!(EnrollmentPackage::decode(&changed, &capacity(), &limits()).is_err());
    }
}

#[test]
fn checks_limits_before_parsing_or_rebuilding_the_tree() {
    let original = package(&[b"pub-A", b"pub-B", b"pub-C"]);
    let bytes = encoded(&original);
    let mut constrained = capacity();
    constrained.max_bytes = NonZeroUsize::new(bytes.len() - 1).unwrap();
    assert!(matches!(
        original.encode(&constrained, &limits()),
        Err(PackageError::ByteLimit)
    ));
    assert!(matches!(
        EnrollmentPackage::decode(&bytes, &constrained, &limits()),
        Err(PackageError::ByteLimit)
    ));
    constrained.max_bytes = NonZeroUsize::new(bytes.len()).unwrap();
    assert_eq!(original.encode(&constrained, &limits()).unwrap(), bytes);
    constrained.max_leaves = NonZeroUsize::new(2).unwrap();
    assert!(matches!(
        original.encode(&constrained, &limits()),
        Err(PackageError::LeafLimit)
    ));
    assert!(matches!(
        EnrollmentPackage::decode(&bytes, &constrained, &limits()),
        Err(PackageError::LeafLimit)
    ));
    let mut small_context = limits();
    small_context.context_bytes = NonZeroUsize::new(8).unwrap();
    assert!(original.encode(&capacity(), &small_context).is_err());
    assert!(EnrollmentPackage::decode(&bytes, &capacity(), &small_context).is_err());
}

#[test]
fn malformed_context_is_rejected_and_changed_context_cannot_find_original_keys() {
    let original = package(&[b"pub-A", b"pub-B", b"pub-C"]);
    let bytes = encoded(&original);
    let network = 20 + 8 + b"gnorix.enrollment.v1".len() + 8;
    let mut invalid_text = bytes.clone();
    invalid_text[network] = 255;
    assert!(EnrollmentPackage::decode(&invalid_text, &capacity(), &limits()).is_err());
    let mut excessive_context = bytes.clone();
    excessive_context[12..20].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(EnrollmentPackage::decode(&excessive_context, &capacity(), &limits()).is_err());

    // Hash-only material cannot independently authenticate context metadata.
    // A changed context cannot reproduce an enrolled public key's leaf.
    let mut changed_context = bytes;
    changed_context[network] = b'T';
    let loaded = EnrollmentPackage::decode(&changed_context, &capacity(), &limits()).unwrap();
    assert_ne!(loaded.commitment(), original.commitment());
    assert!(matches!(
        loaded.proof_for(b"pub-A", &limits()),
        Err(PackageError::Core(Error::Merkle(MerkleError::MissingLeaf)))
    ));
}

#[test]
fn rejects_absent_empty_and_oversized_public_keys() {
    let package = package(&[b"pub-A"]);
    assert!(matches!(
        package.proof_for(b"pub-B", &limits()),
        Err(PackageError::Core(Error::Merkle(MerkleError::MissingLeaf)))
    ));
    assert!(package.proof_for(b"", &limits()).is_err());
    assert!(package.proof_for(&[0; 129], &limits()).is_err());
}

#[test]
fn authenticated_public_profile_parameters_round_trip_without_changing_proofs() {
    let parameters = b"gnorix.test.profile.v1\0minimum_pair_span_r=21.5";
    let original = parameterized_package(parameters);
    assert_eq!(original.profile_parameters(), Some(parameters.as_slice()));
    let bytes = encoded(&original);
    let loaded = EnrollmentPackage::decode(&bytes, &capacity(), &limits()).unwrap();
    assert_eq!(loaded.commitment(), original.commitment());
    assert_eq!(loaded.profile_parameters(), Some(parameters.as_slice()));
    assert_eq!(encoded(&loaded), bytes);
    assert_eq!(
        loaded.proof_for(b"pub-B", &limits()).unwrap(),
        original.proof_for(b"pub-B", &limits()).unwrap()
    );
    assert_eq!(loaded.filename(), original.filename());
}

#[test]
fn parameter_records_require_the_exact_committed_hash_and_reject_empty_records() {
    assert!(matches!(
        package(&[b"pub-A"]).with_profile_parameters(Vec::new(), &limits()),
        Err(PackageError::InvalidFormat(
            "empty profile parameter record"
        ))
    ));
    assert!(matches!(
        package(&[b"pub-A"]).with_profile_parameters(b"other-profile".to_vec(), &limits()),
        Err(PackageError::InvalidFormat(
            "profile parameters do not match the profile ID"
        ))
    ));

    let parameters = b"gnorix.test.profile.v1\0minimum_pair_span_r=21.5";
    let mut original = parameterized_package(parameters);
    let bytes = encoded(&original);
    let start = parameters_offset(&bytes) + 8;
    let mut corrupted = bytes.clone();
    corrupted[start] ^= 1;
    assert!(matches!(
        EnrollmentPackage::decode(&corrupted, &capacity(), &limits()),
        Err(PackageError::InvalidFormat(
            "profile parameters do not match the profile ID"
        ))
    ));
    original.profile_parameters.as_mut().unwrap()[0] ^= 1;
    assert!(original.encode(&capacity(), &limits()).is_err());

    for length in 0..bytes.len() {
        assert!(
            EnrollmentPackage::decode(&bytes[..length], &capacity(), &limits()).is_err(),
            "parameterized package truncation at {length}"
        );
    }
}

#[test]
fn parameter_presence_is_explicit_and_old_container_versions_are_rejected() {
    let parameters = b"gnorix.test.profile.v1\0minimum_pair_span_r=21.5";
    let original = parameterized_package(parameters);
    let mut bytes = encoded(&original);
    let start = parameters_offset(&bytes);
    bytes.splice(start..start + 8 + parameters.len(), 0_u64.to_be_bytes());
    let loaded = EnrollmentPackage::decode(&bytes, &capacity(), &limits()).unwrap();
    // Constant profiles do not need a record; consumers requiring retained
    // settings distinguish this absence through the accessor and reject it.
    assert_eq!(loaded.profile_parameters(), None);
    assert_eq!(loaded.commitment(), original.commitment());

    bytes[11] = 1;
    assert!(matches!(
        EnrollmentPackage::decode(&bytes, &capacity(), &limits()),
        Err(PackageError::InvalidFormat("unsupported magic or version"))
    ));
}

#[test]
fn parameter_and_total_byte_limits_apply_before_rebuilding_the_tree() {
    let parameters = vec![31; 1024];
    let original = parameterized_package(&parameters);
    let bytes = encoded(&original);
    let mut constrained = capacity();
    constrained.max_bytes = NonZeroUsize::new(bytes.len() - 1).unwrap();
    assert!(matches!(
        original.encode(&constrained, &limits()),
        Err(PackageError::ByteLimit)
    ));
    assert!(matches!(
        EnrollmentPackage::decode(&bytes, &constrained, &limits()),
        Err(PackageError::ByteLimit)
    ));
    constrained.max_bytes = NonZeroUsize::new(bytes.len()).unwrap();
    assert_eq!(original.encode(&constrained, &limits()).unwrap(), bytes);

    let mut small = limits();
    small.context_bytes = NonZeroUsize::new(parameters.len() - 1).unwrap();
    // The encoded enrollment context fits; only the parameter record exceeds
    // this independently applied public-metadata allowance.
    assert!(original.commitment.context.encode(&small).is_ok());
    for result in [
        original.encode(&capacity(), &small),
        EnrollmentPackage::decode(&bytes, &capacity(), &small).map(|_| Vec::new()),
    ] {
        assert!(matches!(
            result,
            Err(PackageError::Core(Error::LimitExceeded(
                crate::error::Resource::ContextBytes
            )))
        ));
    }
    let context = original.commitment.context.clone();
    assert!(
        package_with_context(&[b"pub-A"], context)
            .with_profile_parameters(parameters, &small)
            .is_err()
    );

    let mut oversized_length = bytes;
    let start = parameters_offset(&oversized_length);
    oversized_length[start..start + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(EnrollmentPackage::decode(&oversized_length, &capacity(), &limits()).is_err());
}
