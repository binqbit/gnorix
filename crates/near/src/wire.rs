//! Bounded public arguments and views for NEAR contract authorization.
//!
//! JSON represents all `u64` values as decimal strings to preserve precision.
//! Hashes, keys and payloads are byte arrays. Borsh is supported for contract state
//! and uses its normal binary integer encoding. Decoded fields are bounded;
//! Borsh rejects excessive length prefixes before allocating. A JSON parser may
//! still allocate scratch space for escaped input, so transports must also bound
//! the complete request size. Call `validate` or `to_core` for semantic validity.

mod codec;

use std::num::NonZeroU64;

use borsh::{BorshDeserialize, BorshSerialize};
use gnorix_core::{
    authorization,
    enrollment::{Domain, EnrollmentCommitment, EnrollmentContext, EnrollmentSpec},
    merkle::{MerkleProof, MerkleRoot},
    policy::SecretPolicy,
};
use serde::{Deserialize, Serialize};

use crate::{AccountId, Error, Hash, Result, verification_limits};

/// Public registry state used to prepare the next action offchain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentView {
    pub commitment: Commitment,
    pub active: bool,
    #[serde(with = "codec::decimal")]
    pub next_nonce: u64,
}

/// A public commitment registered through the application's authorized path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(deny_unknown_fields)]
pub struct Commitment {
    #[serde(deserialize_with = "codec::text::<_, 64>")]
    #[borsh(deserialize_with = "codec::read_text::<_, 64>")]
    pub network: String,
    #[serde(deserialize_with = "codec::account_id")]
    #[borsh(deserialize_with = "codec::read_account_id")]
    pub contract_id: AccountId,
    #[serde(deserialize_with = "codec::account_id")]
    #[borsh(deserialize_with = "codec::read_account_id")]
    pub owner_id: AccountId,
    pub enrollment_id: Hash,
    #[serde(with = "codec::decimal")]
    pub revision: u64,
    #[serde(deserialize_with = "codec::sequence::<_, u8, 64>")]
    #[borsh(deserialize_with = "codec::read_sequence::<_, u8, 64>")]
    pub public_salt: Vec<u8>,
    pub secret_required: bool,
    pub profile_id: Hash,
    #[serde(deserialize_with = "codec::text::<_, 128>")]
    #[borsh(deserialize_with = "codec::read_text::<_, 128>")]
    pub suite_id: String,
    pub root: Hash,
    #[serde(with = "codec::decimal")]
    pub leaf_count: u64,
}

impl Commitment {
    pub fn validate(&self) -> Result<()> {
        let network = self.network.strip_prefix("near:").unwrap_or_default();
        if self.network.len() > 64 || network.is_empty() || network.chars().any(char::is_control) {
            return Err(Error::InvalidInput("network must be near:<network-name>"));
        }
        if self.revision == 0 || self.leaf_count == 0 {
            return Err(Error::InvalidInput(
                "revision and leaf count must be positive",
            ));
        }
        if self.public_salt.len() > 64 {
            return Err(Error::InvalidInput("public salt exceeds 64 bytes"));
        }
        validate_text(&self.suite_id, 128, "invalid suite identifier")
    }

    pub fn from_core(commitment: &EnrollmentCommitment) -> Result<Self> {
        let context = &commitment.context;
        let value = Self {
            network: context.spec.domain.network.clone(),
            contract_id: context
                .spec
                .domain
                .verifier
                .parse()
                .map_err(|_| Error::InvalidInput("invalid verifying contract account"))?,
            owner_id: context
                .spec
                .subject
                .parse()
                .map_err(|_| Error::InvalidInput("invalid enrollment owner account"))?,
            enrollment_id: context.spec.id,
            revision: context.spec.revision.get(),
            public_salt: context.spec.public_salt.clone(),
            secret_required: context.spec.secret_policy == SecretPolicy::Required,
            profile_id: context.profile_id,
            suite_id: context.suite_id.clone(),
            root: commitment.root.0,
            leaf_count: commitment.leaf_count,
        };
        value.validate()?;
        context.encode(&verification_limits())?;

        Ok(value)
    }

    pub fn to_core(&self) -> Result<EnrollmentCommitment> {
        self.validate()?;
        let context = EnrollmentContext {
            spec: EnrollmentSpec {
                domain: Domain {
                    network: self.network.clone(),
                    verifier: self.contract_id.to_string(),
                },
                subject: self.owner_id.to_string(),
                id: self.enrollment_id,
                revision: NonZeroU64::new(self.revision)
                    .ok_or(Error::InvalidInput("revision must be positive"))?,
                public_salt: self.public_salt.clone(),
                secret_policy: if self.secret_required {
                    SecretPolicy::Required
                } else {
                    SecretPolicy::Absent
                },
            },
            profile_id: self.profile_id,
            suite_id: self.suite_id.clone(),
        };
        context.encode(&verification_limits())?;

        Ok(EnrollmentCommitment {
            context,
            root: MerkleRoot(self.root),
            leaf_count: self.leaf_count,
        })
    }
}

/// Exact approved operation. The application defines the payload's canonical
/// bytes. Expiration uses Unix milliseconds; nonce is an application sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    #[serde(deserialize_with = "codec::text::<_, 128>")]
    #[borsh(deserialize_with = "codec::read_text::<_, 128>")]
    pub operation: String,
    #[serde(deserialize_with = "codec::sequence::<_, u8, 4096>")]
    #[borsh(deserialize_with = "codec::read_sequence::<_, u8, 4096>")]
    pub payload: Vec<u8>,
    #[serde(with = "codec::decimal")]
    pub nonce: u64,
    #[serde(with = "codec::decimal")]
    pub expires_at_ms: u64,
}

impl Action {
    pub fn validate(&self) -> Result<()> {
        validate_text(&self.operation, 128, "invalid operation identifier")?;
        if self.payload.len() > 4096 {
            return Err(Error::InvalidInput("action payload exceeds 4096 bytes"));
        }
        Ok(())
    }

    pub fn to_core(&self) -> Result<authorization::Action> {
        self.validate()?;
        let mut nonce = [0; 32];
        nonce[24..].copy_from_slice(&self.nonce.to_be_bytes());

        Ok(authorization::Action {
            operation: self.operation.clone(),
            payload: self.payload.clone(),
            nonce,
            expires_at: self.expires_at_ms,
        })
    }

    pub fn from_core(action: &authorization::Action) -> Result<Self> {
        if action.nonce[..24] != [0; 24] {
            return Err(Error::InvalidInput(
                "nonce is not a canonical NEAR sequence",
            ));
        }
        let mut sequence = [0; 8];
        sequence.copy_from_slice(&action.nonce[24..]);
        let value = Self {
            operation: action.operation.clone(),
            payload: action.payload.clone(),
            nonce: u64::from_be_bytes(sequence),
            expires_at_ms: action.expires_at,
        };
        value.validate()?;
        Ok(value)
    }
}

/// Public signature and membership evidence; no caller-supplied trusted root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub enrollment_id: Hash,
    #[serde(with = "codec::decimal")]
    pub revision: u64,
    pub action: Action,
    pub public_key: [u8; 32],
    #[serde(deserialize_with = "codec::sequence::<_, u8, 64>")]
    #[borsh(deserialize_with = "codec::read_sequence::<_, u8, 64>")]
    pub signature: Vec<u8>,
    #[serde(with = "codec::decimal")]
    pub leaf_index: u64,
    #[serde(with = "codec::decimal")]
    pub leaf_count: u64,
    #[serde(deserialize_with = "codec::sequence::<_, Hash, 64>")]
    #[borsh(deserialize_with = "codec::read_sequence::<_, Hash, 64>")]
    pub siblings: Vec<Hash>,
}

impl Authorization {
    pub fn validate(&self) -> Result<()> {
        self.action.validate()?;
        if self.revision == 0 {
            return Err(Error::InvalidInput("revision must be positive"));
        }
        if self.signature.len() != 64 {
            return Err(Error::InvalidInput(
                "Ed25519 signature must contain 64 bytes",
            ));
        }
        if self.leaf_count == 0 || self.leaf_index >= self.leaf_count {
            return Err(Error::InvalidInput("invalid Merkle leaf index or count"));
        }
        if self.siblings.len() > 64 {
            return Err(Error::InvalidInput("Merkle path exceeds 64 siblings"));
        }
        Ok(())
    }

    pub fn from_core(authorization: &authorization::Authorization) -> Result<Self> {
        let value = Self {
            enrollment_id: authorization.enrollment_id,
            revision: authorization.revision.get(),
            action: Action::from_core(&authorization.action)?,
            public_key: authorization
                .public_key
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidInput("Ed25519 public key must contain 32 bytes"))?,
            signature: authorization.signature.clone(),
            leaf_index: authorization.proof.leaf_index,
            leaf_count: authorization.proof.leaf_count,
            siblings: authorization.proof.siblings.clone(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn to_core(&self) -> Result<authorization::Authorization> {
        self.validate()?;
        Ok(authorization::Authorization {
            enrollment_id: self.enrollment_id,
            revision: NonZeroU64::new(self.revision)
                .ok_or(Error::InvalidInput("revision must be positive"))?,
            action: self.action.to_core()?,
            public_key: self.public_key.to_vec(),
            signature: self.signature.clone(),
            proof: MerkleProof {
                leaf_index: self.leaf_index,
                leaf_count: self.leaf_count,
                siblings: self.siblings.clone(),
            },
        })
    }
}

fn validate_text(value: &str, maximum: usize, reason: &'static str) -> Result<()> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(Error::InvalidInput(reason));
    }
    Ok(())
}
