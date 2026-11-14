//! Typed failures at the library and algorithm boundaries.

use std::{error::Error as StdError, fmt};

use crate::merkle::MerkleError;

/// An algorithm implementation's original failure, without discarding its cause.
pub type AlgorithmError = Box<dyn StdError + Send + Sync + 'static>;

/// A library operation result.
pub type Result<T> = std::result::Result<T, Error>;

/// The stage that failed inside an application-supplied algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    EnrollmentPreparation,
    CandidateGeneration,
    Reconstruction,
    KeyDerivation,
    PublicKey,
    Signing,
    SignatureVerification,
}

/// Resources bounded by the caller, independently of biometric acceptance rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    CandidateAttempts,
    CandidateBytes,
    PublicKeyBytes,
    SignatureBytes,
    ActionBytes,
    ContextBytes,
}

/// Failures never imply successful enrollment or authorization.
#[derive(Debug)]
pub enum Error {
    InvalidInput(&'static str),
    LimitExceeded(Resource),
    SecretPolicyMismatch,
    ProfileMismatch,
    SuiteMismatch,
    EnrollmentMismatch,
    EnrollmentRevoked,
    ActionMismatch,
    ActionExpired,
    CandidateNotEnrolled,
    InvalidSignature,
    Merkle(MerkleError),
    Algorithm {
        stage: Stage,
        source: AlgorithmError,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(reason) => write!(f, "invalid input: {reason}"),
            Self::LimitExceeded(resource) => write!(f, "resource limit exceeded: {resource:?}"),
            Self::SecretPolicyMismatch => {
                f.write_str("secret input does not match enrollment policy")
            }
            Self::ProfileMismatch => f.write_str("measurement profile does not match enrollment"),
            Self::SuiteMismatch => f.write_str("cryptographic suite does not match enrollment"),
            Self::EnrollmentMismatch => f.write_str("authorization belongs to another enrollment"),
            Self::EnrollmentRevoked => f.write_str("enrollment is revoked"),
            Self::ActionMismatch => f.write_str("authorization does not match the expected action"),
            Self::ActionExpired => f.write_str("action has expired"),
            Self::CandidateNotEnrolled => {
                f.write_str("no reconstructed candidate belongs to enrollment")
            }
            Self::InvalidSignature => f.write_str("signature verification failed"),
            Self::Merkle(error) => write!(f, "membership error: {error}"),
            Self::Algorithm { stage, .. } => write!(f, "algorithm failed during {stage:?}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Merkle(error) => Some(error),
            Self::Algorithm { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<MerkleError> for Error {
    fn from(error: MerkleError) -> Self {
        Self::Merkle(error)
    }
}

pub(crate) fn algorithm(stage: Stage, source: AlgorithmError) -> Error {
    Error::Algorithm { stage, source }
}
