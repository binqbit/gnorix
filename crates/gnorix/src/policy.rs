//! Explicit resource limits and secret-input policy; no biometric defaults.

use std::num::NonZeroUsize;

use crate::{Error, Result, crypto::SecretInput, error::Resource};

/// Every limit is supplied by the caller. None is a security-strength estimate.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub candidate_attempts: NonZeroUsize,
    pub candidate_bytes: NonZeroUsize,
    pub public_key_bytes: NonZeroUsize,
    pub signature_bytes: NonZeroUsize,
    pub action_bytes: NonZeroUsize,
    pub context_bytes: NonZeroUsize,
}

/// Whether this enrollment incorporates an additional independent secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretPolicy {
    Absent,
    Required,
}

impl SecretPolicy {
    pub(crate) fn validate(self, secret: Option<&SecretInput>) -> Result<()> {
        match (self, secret) {
            (Self::Absent, None) | (Self::Required, Some(_)) => Ok(()),
            _ => Err(Error::SecretPolicyMismatch),
        }
    }

    pub(crate) fn tag(self) -> u8 {
        match self {
            Self::Absent => 0,
            Self::Required => 1,
        }
    }
}

pub(crate) fn check_size(size: usize, limit: NonZeroUsize, resource: Resource) -> Result<()> {
    if size > limit.get() {
        Err(Error::LimitExceeded(resource))
    } else {
        Ok(())
    }
}
