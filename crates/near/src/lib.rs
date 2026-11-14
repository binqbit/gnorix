//! Gnorix authorization verification inside NEAR contracts.
//!
//! [`contract`] owns authenticated enrollment and replay state. [`wire`] defines
//! public contract arguments and conversions from core authorizations. Offchain
//! key reconstruction and signing use `gnorix::core` directly. Applications supply
//! their own permissions and execute approved actions in the verifying receipt.

pub mod contract;
pub mod wire;

mod error;

pub use error::{Error, Result};
pub use gnorix_core::merkle::Hash;
pub use near_account_id::AccountId;
pub use wire::{Action, Authorization, Commitment, EnrollmentView};

/// Bounds for public verification. Candidate fields are unused by verification;
/// offchain reconstruction receives its own explicit search limits in core.
pub fn verification_limits() -> gnorix_core::policy::Limits {
    use std::num::NonZeroUsize;

    gnorix_core::policy::Limits {
        candidate_attempts: NonZeroUsize::new(1).unwrap(),
        candidate_bytes: NonZeroUsize::new(32).unwrap(),
        public_key_bytes: NonZeroUsize::new(32).unwrap(),
        signature_bytes: NonZeroUsize::new(64).unwrap(),
        action_bytes: NonZeroUsize::new(8192).unwrap(),
        context_bytes: NonZeroUsize::new(4096).unwrap(),
    }
}
