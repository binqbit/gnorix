//! Enrollment orchestration, public commitments and user-side retained material.

use std::num::NonZeroU64;

use crate::{
    Error, Result,
    biometrics::{Capture, MeasurementEngine},
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
    pub(crate) fn encode(&self, limits: &Limits) -> Result<Vec<u8>> {
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
    let context = bind_context(measurement, crypto, spec, secret, limits)?;
    let model = prepare_model(measurement, captures, limits)?;
    let tree = build_commitment(measurement, crypto, &model, &context, secret, limits)?;

    Ok(assemble_enrollment(context, model, tree))
}

fn bind_context<M: MeasurementEngine, C: CryptoProvider>(
    measurement: &M,
    crypto: &C,
    spec: EnrollmentSpec,
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<EnrollmentContext> {
    spec.secret_policy.validate(secret)?;

    let context = EnrollmentContext {
        spec,
        profile_id: measurement.profile_id(),
        suite_id: crypto.suite_id().to_owned(),
    };

    context.encode(limits)?;

    Ok(context)
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

fn build_commitment<M: MeasurementEngine, C: CryptoProvider>(
    measurement: &M,
    crypto: &C,
    model: &M::Model,
    context: &EnrollmentContext,
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<MerkleTree> {
    let context = context.encode(limits)?;
    let candidates = measurement
        .enrollment_candidates(model, limits)
        .map_err(|error| algorithm(Stage::CandidateGeneration, error))?;
    let mut leaves = Vec::new();

    for (attempt, candidate) in candidates.enumerate() {
        check_attempt(attempt, limits)?;

        let candidate = candidate.map_err(|error| algorithm(Stage::CandidateGeneration, error))?;
        let key = derive_candidate(crypto, &candidate, &context, secret, limits)?;

        leaves.push(leaf_hash(&context, &key.public));
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
