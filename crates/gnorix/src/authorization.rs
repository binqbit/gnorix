//! Reconstruction/signing and cryptographic verification pipelines.

use std::num::{NonZeroU64, NonZeroUsize};

use crate::{
    Error, Result,
    biometrics::{Capture, MeasurementEngine},
    crypto::{
        CryptoProvider, DerivedKey, SecretInput, SignatureVerifier, derive_candidate,
        validate_public_key, validate_signature,
    },
    encoding,
    enrollment::{Enrollment, EnrollmentCommitment},
    error::{Resource, Stage, algorithm},
    merkle::{self, Hash, MerkleError, MerkleProof, leaf_hash},
    policy::{Limits, check_attempt},
};

/// Exact application intent. The application defines canonical payload bytes and
/// uses a consistent time unit for `expires_at` and the verifier's trusted clock.
/// A nonce is signed here; durable replay prevention belongs to the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub operation: String,
    pub payload: Vec<u8>,
    pub nonce: Hash,
    pub expires_at: u64,
}

impl Action {
    fn encode(&self, limits: &Limits) -> Result<Vec<u8>> {
        if self.operation.is_empty() {
            return Err(Error::InvalidInput("action operation is empty"));
        }

        encoding::encode(
            b"gnorix.action.v1",
            &[
                self.operation.as_bytes(),
                &self.payload,
                &self.nonce,
                &self.expires_at.to_be_bytes(),
            ],
            limits.action_bytes,
            Resource::ActionBytes,
        )
    }
}

/// Public proof package. It carries an enrollment reference, never a replacement
/// trusted root. Constructing this type does not establish authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorization {
    pub enrollment_id: Hash,
    pub revision: NonZeroU64,
    pub action: Action,
    pub public_key: Vec<u8>,
    pub proof: MerkleProof,
    pub signature: Vec<u8>,
}

/// Status supplied by the application from its trusted enrollment registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentStatus {
    Active,
    Revoked,
}

/// Evidence constructed only after all core checks succeed. This does not consume
/// a nonce or grant application permissions, execute an action, or prove liveness.
#[derive(Debug)]
pub struct VerifiedAuthorization<'a> {
    commitment: &'a EnrollmentCommitment,
    action: &'a Action,
}

impl<'a> VerifiedAuthorization<'a> {
    pub fn commitment(&self) -> &'a EnrollmentCommitment {
        self.commitment
    }

    pub fn action(&self) -> &'a Action {
        self.action
    }
}

/// Reconstruct an enrolled key, locate its proof and sign the exact action.
/// Call only after the application has obtained the user's approval of `action`.
pub fn sign<M: MeasurementEngine, C: CryptoProvider>(
    measurement: &M,
    crypto: &C,
    enrollment: &Enrollment<M::Model>,
    capture: &Capture,
    action: Action,
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<Authorization> {
    validate_signing_context(measurement, crypto, enrollment, secret)?;
    let message = signing_message(&enrollment.commitment, &action, limits)?;
    let (key, proof) =
        reconstruct_member(measurement, crypto, enrollment, capture, secret, limits)?;
    let signature = sign_message(crypto, &key, &message, limits)?;

    Ok(Authorization {
        enrollment_id: enrollment.commitment.context.spec.id,
        revision: enrollment.commitment.context.spec.revision,
        action,
        public_key: key.public,
        proof,
        signature,
    })
}

/// Verify against independently trusted registry state and the expected action.
/// Expiration is exclusive: `now >= expires_at` is rejected. The application must
/// check permissions and atomically reserve/consume replay state before execution.
pub fn verify<'a, C: SignatureVerifier>(
    crypto: &C,
    trusted: &'a EnrollmentCommitment,
    status: EnrollmentStatus,
    expected: &Action,
    authorization: &'a Authorization,
    now: u64,
    limits: &Limits,
) -> Result<VerifiedAuthorization<'a>> {
    validate_request(
        crypto,
        trusted,
        status,
        expected,
        authorization,
        now,
        limits,
    )?;
    verify_membership(trusted, authorization, limits)?;
    verify_signature(crypto, trusted, authorization, limits)?;

    Ok(VerifiedAuthorization {
        commitment: trusted,
        action: &authorization.action,
    })
}

fn validate_signing_context<M: MeasurementEngine, C: CryptoProvider>(
    measurement: &M,
    crypto: &C,
    enrollment: &Enrollment<M::Model>,
    secret: Option<&SecretInput>,
) -> Result<()> {
    let context = &enrollment.commitment.context;

    if measurement.profile_id() != context.profile_id {
        return Err(Error::ProfileMismatch);
    }
    if crypto.suite_id() != context.suite_id {
        return Err(Error::SuiteMismatch);
    }

    context.spec.secret_policy.validate(secret)
}

fn reconstruct_member<M: MeasurementEngine, C: CryptoProvider>(
    measurement: &M,
    crypto: &C,
    enrollment: &Enrollment<M::Model>,
    capture: &Capture,
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<(DerivedKey<C>, MerkleProof)> {
    let context = enrollment.commitment.context.encode(limits)?;
    let candidates = measurement
        .reconstruct(capture, &enrollment.model, limits)
        .map_err(|error| algorithm(Stage::Reconstruction, error))?;

    for (attempt, candidate) in candidates.enumerate() {
        check_attempt(attempt, limits)?;

        let candidate = candidate.map_err(|error| algorithm(Stage::Reconstruction, error))?;
        let key = derive_candidate(crypto, &candidate, &context, secret, limits)?;
        let leaf = leaf_hash(&context, &key.public);

        match enrollment.tree.proof_for(&leaf) {
            Ok(proof) => return Ok((key, proof)),
            Err(MerkleError::MissingLeaf) => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(Error::CandidateNotEnrolled)
}

fn sign_message<C: CryptoProvider>(
    crypto: &C,
    key: &DerivedKey<C>,
    message: &[u8],
    limits: &Limits,
) -> Result<Vec<u8>> {
    let signature = crypto
        .sign(&key.secret, message)
        .map_err(|error| algorithm(Stage::Signing, error))?;

    validate_signature(&signature, limits)?;

    Ok(signature)
}

fn validate_request<C: SignatureVerifier>(
    crypto: &C,
    trusted: &EnrollmentCommitment,
    status: EnrollmentStatus,
    expected: &Action,
    authorization: &Authorization,
    now: u64,
    limits: &Limits,
) -> Result<()> {
    if status != EnrollmentStatus::Active {
        return Err(Error::EnrollmentRevoked);
    }
    if crypto.suite_id() != trusted.context.suite_id {
        return Err(Error::SuiteMismatch);
    }
    if authorization.enrollment_id != trusted.context.spec.id
        || authorization.revision != trusted.context.spec.revision
    {
        return Err(Error::EnrollmentMismatch);
    }
    if &authorization.action != expected {
        return Err(Error::ActionMismatch);
    }
    if now >= authorization.action.expires_at {
        return Err(Error::ActionExpired);
    }
    if authorization.proof.leaf_count != trusted.leaf_count {
        return Err(Error::EnrollmentMismatch);
    }

    validate_public_key(&authorization.public_key, limits)?;
    validate_signature(&authorization.signature, limits)
}

fn verify_membership(
    trusted: &EnrollmentCommitment,
    authorization: &Authorization,
    limits: &Limits,
) -> Result<()> {
    let context = trusted.context.encode(limits)?;
    let leaf = leaf_hash(&context, &authorization.public_key);

    Ok(merkle::verify(trusted.root, leaf, &authorization.proof)?)
}

fn verify_signature<C: SignatureVerifier>(
    crypto: &C,
    trusted: &EnrollmentCommitment,
    authorization: &Authorization,
    limits: &Limits,
) -> Result<()> {
    let message = signing_message(trusted, &authorization.action, limits)?;
    let valid = crypto
        .verify(
            &authorization.public_key,
            &message,
            &authorization.signature,
        )
        .map_err(|error| algorithm(Stage::SignatureVerification, error))?;

    if !valid {
        return Err(Error::InvalidSignature);
    }

    Ok(())
}

fn signing_message(
    commitment: &EnrollmentCommitment,
    action: &Action,
    limits: &Limits,
) -> Result<Vec<u8>> {
    let context = commitment.context.encode(limits)?;
    let action = action.encode(limits)?;
    // Components have already been bounded independently. The final allocation
    // needs their actual sizes, plus five length prefixes, domain, root and count.
    let domain = b"gnorix.signature.v1";
    let overhead = 5 * 8 + domain.len() + 32 + 8;
    let maximum = context
        .len()
        .checked_add(action.len())
        .and_then(|size| size.checked_add(overhead))
        .and_then(NonZeroUsize::new)
        .ok_or(Error::LimitExceeded(Resource::ActionBytes))?;

    encoding::encode(
        domain,
        &[
            &context,
            &commitment.root.0,
            &commitment.leaf_count.to_be_bytes(),
            &action,
        ],
        maximum,
        Resource::ActionBytes,
    )
}
