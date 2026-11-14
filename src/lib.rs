//! Gnorix's application-facing biometric signing library.
//!
//! The `core` feature is enabled by default. It exposes the biometric,
//! enrollment, signing and verification APIs directly and under `gnorix::core`.
//! Disable default features to build without the core implementation.

/// Core biometric signing APIs, enabled by the default `core` feature.
#[cfg(feature = "core")]
pub use gnorix_core as core;

#[cfg(feature = "core")]
pub use gnorix_core::*;
