//! Gnorix's independent enrollment, signing and verification foundation.
//!
//! Applications supply a [`biometrics::MeasurementEngine`] and a
//! [`crypto::CryptoProvider`]. No biometric interpretation, tolerance, key
//! derivation or signature algorithm is selected by this crate.
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
