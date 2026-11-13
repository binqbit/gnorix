//! Gnorix's independent enrollment, signing and verification foundation.
//!
//! Applications supply a [`biometrics::MeasurementEngine`] and a
//! [`crypto::CryptoProvider`]. Per-capture ridge extraction is available in
//! [`biometrics::fingerprint`], including normalized grid candidate hashing.
//! Registration, matching tolerances, key derivation and signature algorithms
//! remain explicit choices.
//! Public verification alone requires only [`crypto::SignatureVerifier`].
//!
//! [`authorization::verify`] checks evidence against an independently trusted
//! enrollment. The application still owns registration, permissions, replay
//! protection, revocation authority, recovery and execution.

pub mod authorization;
pub mod biometrics;
pub mod crypto;
pub mod enrollment;
pub mod error;
pub mod merkle;
pub mod policy;

mod encoding;

pub use error::{Error, Result};
