//! Explicit key-derivation and signature provider boundary.

use std::fmt;

use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::{
    Error, Result,
    biometrics::Candidate,
    error::{AlgorithmError, Resource, Stage, algorithm},
    policy::{Limits, check_size},
};

/// An independently supplied PIN/password. No length or strength default is chosen.
pub struct SecretInput(Zeroizing<Vec<u8>>);

impl SecretInput {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let bytes = Zeroizing::new(bytes);

        if bytes.is_empty() {
            return Err(Error::InvalidInput("secret input is empty"));
        }

        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretInput([REDACTED])")
    }
}

/// Public verification without access to key derivation or signing operations.
/// The suite identifier includes the enrollment's KDF/signature configuration,
/// even when the verifier implements only its public signature algorithm.
pub trait SignatureVerifier {
    fn suite_id(&self) -> &str;

    fn verify(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> std::result::Result<bool, AlgorithmError>;
}

/// A selected, internally consistent KDF/signature suite for user-side operations.
///
/// `suite_id` must identify both algorithms, their parameters and key/ signature
/// encodings. Derivation must bind the supplied enrollment context and secret.
/// Identical candidate, context and secret inputs must deterministically produce
/// the same key and canonical public-key bytes. Suite settings and identifiers
/// must remain stable for an enrollment. Signature randomness is suite-specific.
/// Key generation and signature validation belong to established crypto libraries;
/// the orchestration code does not implement them. There is no default provider.
/// Implementations are responsible for copies and intermediate secret material.
/// Errors must not contain secret or candidate bytes: their sources are exposed
/// for diagnosis and can be formatted by application code.
pub trait CryptoProvider: SignatureVerifier {
    type SigningKey: ZeroizeOnDrop;

    fn derive_key(
        &self,
        candidate: &Candidate,
        context: &[u8],
        secret: Option<&SecretInput>,
    ) -> std::result::Result<Self::SigningKey, AlgorithmError>;

    fn public_key(&self, key: &Self::SigningKey) -> std::result::Result<Vec<u8>, AlgorithmError>;

    fn sign(
        &self,
        key: &Self::SigningKey,
        message: &[u8],
    ) -> std::result::Result<Vec<u8>, AlgorithmError>;
}

pub(crate) struct DerivedKey<C: CryptoProvider> {
    pub secret: C::SigningKey,
    pub public: Vec<u8>,
}

pub(crate) fn derive_candidate<C: CryptoProvider>(
    crypto: &C,
    candidate: &Candidate,
    context: &[u8],
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<DerivedKey<C>> {
    check_size(
        candidate.as_bytes().len(),
        limits.candidate_bytes,
        Resource::CandidateBytes,
    )?;

    let key = crypto
        .derive_key(candidate, context, secret)
        .map_err(|error| algorithm(Stage::KeyDerivation, error))?;
    let public = crypto
        .public_key(&key)
        .map_err(|error| algorithm(Stage::PublicKey, error))?;

    validate_public_key(&public, limits)?;

    Ok(DerivedKey {
        secret: key,
        public,
    })
}

pub(crate) fn validate_public_key(public: &[u8], limits: &Limits) -> Result<()> {
    if public.is_empty() {
        return Err(Error::InvalidInput("public key is empty"));
    }

    check_size(
        public.len(),
        limits.public_key_bytes,
        Resource::PublicKeyBytes,
    )
}

pub(crate) fn validate_signature(signature: &[u8], limits: &Limits) -> Result<()> {
    if signature.is_empty() {
        return Err(Error::InvalidInput("signature is empty"));
    }

    check_size(
        signature.len(),
        limits.signature_bytes,
        Resource::SignatureBytes,
    )
}
