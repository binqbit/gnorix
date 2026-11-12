//! Acquisition data and the boundary for an explicitly chosen measurement model.

pub mod capture;
pub mod fingerprint;
pub mod geometry;

use std::fmt;

use zeroize::Zeroizing;

use crate::{Error, Result, error::AlgorithmError, merkle::Hash, policy::Limits};

pub use capture::{Capture, CaptureError};

/// Sensitive canonical variant bytes; not a raw image or a derived private key.
/// The measurement implementation owns quantization, point order and missingness.
pub struct Candidate(Zeroizing<Vec<u8>>);

impl Candidate {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let bytes = Zeroizing::new(bytes);

        if bytes.is_empty() {
            return Err(Error::InvalidInput("candidate encoding is empty"));
        }

        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Candidate([REDACTED])")
    }
}

/// A fallible stream avoids materializing every candidate before enforcing limits.
pub type Candidates<'a> =
    Box<dyn Iterator<Item = std::result::Result<Candidate, AlgorithmError>> + 'a>;

/// A complete measurement strategy, including its processing order.
///
/// Implementations own alignment, feature extraction/fusion, uncertainty,
/// quantization, candidate encoding and any reference/helper material. The core
/// deliberately does not choose their ordering or numerical acceptance criteria.
/// `profile_id` must identify all settings that affect reconstruction. Models
/// must preserve those settings for subsequent captures. Implementations must
/// honor supplied work limits internally; the core can bound yielded candidates
/// but cannot interrupt arbitrary computation inside `prepare` or `next`.
/// Errors must not include captures, candidate bytes or sensitive model data.
pub trait MeasurementEngine {
    type Model;

    fn profile_id(&self) -> Hash;

    fn prepare(
        &self,
        captures: &[Capture],
        limits: &Limits,
    ) -> std::result::Result<Self::Model, AlgorithmError>;

    /// Exhaustion must mean the entire accepted set has been enumerated.
    /// A budget or internal failure must be returned as an error, never hidden
    /// by ending an incomplete stream.
    fn enrollment_candidates<'a>(
        &'a self,
        model: &'a Self::Model,
        limits: &Limits,
    ) -> std::result::Result<Candidates<'a>, AlgorithmError>;

    /// Yield candidates under the enrolled policy without widening its tolerance.
    fn reconstruct<'a>(
        &'a self,
        capture: &'a Capture,
        model: &'a Self::Model,
        limits: &Limits,
    ) -> std::result::Result<Candidates<'a>, AlgorithmError>;
}
