//! Enrollment orchestration, public commitments and user-side retained material.

use std::num::NonZeroU64;

use crate::{
    Error, Result,
    biometrics::{Candidates, Capture, MeasurementEngine},
    crypto::{CryptoProvider, SecretInput, derive_candidate},
    encoding,
    error::{Resource, Stage, algorithm},
    merkle::{Hash, MerkleRoot, MerkleTree, leaf_hash},
    policy::{Limits, SecretPolicy, check_attempt},
};

/// The application/network in which an enrollment can be used.
/// For a chain integration, `network` identifies the chain and network and
/// `verifier` identifies the verifying application or contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    pub network: String,
    pub verifier: String,
}

/// Caller-selected enrollment identity and policy. No registration authority is
/// inferred from constructing this value or deriving a commitment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentSpec {
    pub domain: Domain,
    pub subject: String,
    pub id: Hash,
    pub revision: NonZeroU64,
    pub public_salt: Vec<u8>,
    pub secret_policy: SecretPolicy,
}

/// The exact interpretation used for deriving and committing candidate keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentContext {
    pub spec: EnrollmentSpec,
    pub profile_id: Hash,
    pub suite_id: String,
}

impl EnrollmentContext {
    /// Encode the exact context used for candidate-key derivation and Merkle leaves.
    ///
    /// The versioned, length-prefixed framing is shared by enrollment and signing.
    /// Callers can reproduce provider inputs and membership leaf hashes without
    /// rebuilding this encoding themselves. The context is validated and bounded
    /// by `limits.context_bytes`; its bytes contain public enrollment metadata.
    pub fn encode(&self, limits: &Limits) -> Result<Vec<u8>> {
        validate_spec(&self.spec)?;

        if self.suite_id.is_empty() {
            return Err(Error::InvalidInput(
                "cryptographic suite identifier is empty",
            ));
        }

        encoding::encode(
            b"gnorix.enrollment.v1",
            &[
                self.spec.domain.network.as_bytes(),
                self.spec.domain.verifier.as_bytes(),
                self.spec.subject.as_bytes(),
                &self.spec.id,
                &self.spec.revision.get().to_be_bytes(),
                &self.spec.public_salt,
                &[self.spec.secret_policy.tag()],
                &self.profile_id,
                self.suite_id.as_bytes(),
            ],
            limits.context_bytes,
            Resource::ContextBytes,
        )
    }
}

/// Public enrollment state that must be registered through an authorized path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentCommitment {
    pub context: EnrollmentContext,
    pub root: MerkleRoot,
    pub leaf_count: u64,
}

/// User-side enrollment: public commitment, algorithm-owned reconstruction model
/// and retained hash tree. Transient candidate/private keys are not retained.
/// The model and tree require a separately selected protection/backup policy.
pub struct Enrollment<Model> {
    pub(crate) commitment: EnrollmentCommitment,
    pub(crate) model: Model,
    pub(crate) tree: MerkleTree,
}

impl<Model> Enrollment<Model> {
    pub fn commitment(&self) -> &EnrollmentCommitment {
        &self.commitment
    }

    pub fn reconstruction_model(&self) -> &Model {
        &self.model
    }

    pub fn proof_material(&self) -> &MerkleTree {
        &self.tree
    }
}

/// Prepare a model, enumerate its complete accepted set, and build a commitment.
/// A failed or exhausted enumeration never returns a partially enrolled set.
pub fn enroll<M: MeasurementEngine, C: CryptoProvider>(
    measurement: &M,
    crypto: &C,
    captures: &[Capture],
    spec: EnrollmentSpec,
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<Enrollment<M::Model>> {
    let context = bind_context(measurement, crypto, spec);
    let encoded_context = validate_context(crypto, &context, secret, limits)?;
    let model = prepare_model(measurement, captures, limits)?;
    let candidates = prepare_candidates(measurement, &model, limits)?;
    let tree = build_commitment(crypto, candidates, &encoded_context, secret, limits)?;

    Ok(assemble_enrollment(context, model, tree))
}

/// Enroll a complete stream of already encoded candidates using an explicit context.
///
/// This entry point shares key derivation, limits and tree construction with
/// [`enroll`]. It retains no reconstruction model: the caller owns candidate
/// generation and must identify its exact interpretation through `profile_id`.
/// Context validity, the provider's suite and secret policy are checked before
/// consuming the stream. Any stream, derivation or resource failure rejects the
/// entire enrollment; transient candidate/private keys are not retained.
pub fn enroll_candidates<C: CryptoProvider>(
    crypto: &C,
    candidates: Candidates<'_>,
    context: EnrollmentContext,
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<Enrollment<()>> {
    let encoded_context = validate_context(crypto, &context, secret, limits)?;
    let tree = build_commitment(crypto, candidates, &encoded_context, secret, limits)?;

    Ok(assemble_enrollment(context, (), tree))
}

fn bind_context<M: MeasurementEngine, C: CryptoProvider>(
    measurement: &M,
    crypto: &C,
    spec: EnrollmentSpec,
) -> EnrollmentContext {
    EnrollmentContext {
        spec,
        profile_id: measurement.profile_id(),
        suite_id: crypto.suite_id().to_owned(),
    }
}

fn validate_context<C: CryptoProvider>(
    crypto: &C,
    context: &EnrollmentContext,
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<Vec<u8>> {
    context.spec.secret_policy.validate(secret)?;
    if context.suite_id != crypto.suite_id() {
        return Err(Error::SuiteMismatch);
    }

    context.encode(limits)
}

fn prepare_model<M: MeasurementEngine>(
    measurement: &M,
    captures: &[Capture],
    limits: &Limits,
) -> Result<M::Model> {
    if captures.is_empty() {
        return Err(Error::InvalidInput("enrollment captures are empty"));
    }

    measurement
        .prepare(captures, limits)
        .map_err(|error| algorithm(Stage::EnrollmentPreparation, error))
}

fn prepare_candidates<'a, M: MeasurementEngine>(
    measurement: &'a M,
    model: &'a M::Model,
    limits: &Limits,
) -> Result<Candidates<'a>> {
    measurement
        .enrollment_candidates(model, limits)
        .map_err(|error| algorithm(Stage::CandidateGeneration, error))
}

fn build_commitment<C: CryptoProvider>(
    crypto: &C,
    candidates: Candidates<'_>,
    context: &[u8],
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<MerkleTree> {
    let mut leaves = Vec::new();

    for (attempt, candidate) in candidates.enumerate() {
        check_attempt(attempt, limits)?;

        let candidate = candidate.map_err(|error| algorithm(Stage::CandidateGeneration, error))?;
        let key = derive_candidate(crypto, &candidate, context, secret, limits)?;

        leaves.push(leaf_hash(context, &key.public));
    }

    Ok(MerkleTree::from_leaves(leaves)?)
}

fn assemble_enrollment<Model>(
    context: EnrollmentContext,
    model: Model,
    tree: MerkleTree,
) -> Enrollment<Model> {
    let commitment = EnrollmentCommitment {
        context,
        root: tree.root(),
        leaf_count: tree.leaf_count() as u64,
    };

    Enrollment {
        commitment,
        model,
        tree,
    }
}

fn validate_spec(spec: &EnrollmentSpec) -> Result<()> {
    if spec.domain.network.is_empty() || spec.domain.verifier.is_empty() || spec.subject.is_empty()
    {
        return Err(Error::InvalidInput(
            "network, verifier and subject must be nonempty",
        ));
    }

    Ok(())
}
