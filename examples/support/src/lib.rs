//! Shared adapters for repository examples, separate from the Gnorix library policy.

pub mod authorization_json;
pub mod enrollment_profile;
pub mod example_crypto;
pub mod fingerprint;
pub mod output;

/// Fallible input, output and protocol operations performed by example applications.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
