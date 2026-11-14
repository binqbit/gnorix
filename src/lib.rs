//! Gnorix's application-facing biometric signing library.
//!
//! The `core` feature is enabled by default. It exposes the biometric,
//! enrollment, signing and verification APIs directly and under `gnorix::core`.
//! With no features selected, the facade has no implementation dependencies.
//! The optional `near` feature adds verification inside NEAR contracts.

/// Core biometric signing APIs, enabled by the default `core` feature.
#[cfg(feature = "core")]
pub use gnorix_core as core;

#[cfg(feature = "core")]
pub use gnorix_core::*;

/// NEAR contract authorization state, verification and public argument formats.
#[cfg(feature = "near")]
pub use gnorix_near as near;
