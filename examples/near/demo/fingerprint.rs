//! Shared preparation of the two public synthetic demo images, session-specific
//! enrollment, and fresh partial-scan key reconstruction. Cached descriptors are
//! used only for enrollment; reconstructed private keys are never retained.

use std::{
    cell::Cell,
    num::{NonZeroU64, NonZeroUsize},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, anyhow, ensure};
use gnorix::{
    core::{
        authorization,
        biometrics::{
            Candidate, Candidates, Capture,
            fingerprint::{
                self, CombinationConfig, CombinationPoint, ExtractionConfig, FingerprintFeatures,
                GridBaselineConfig, SigningSearchConfig, SpatialPolicy,
            },
        },
        enrollment::{self, Domain, EnrollmentContext, EnrollmentSpec},
        error::AlgorithmError,
        merkle,
        package::EnrollmentPackage,
        policy::{Limits, SecretPolicy},
    },
    near::{Action, Authorization, Commitment, verification_limits},
};
use gnorix_example_support::{
    enrollment_profile::ProfileParameters,
    example_crypto::{ExampleCrypto, SUITE_ID},
};
use serde::Serialize;

use super::capture::{PatternGeometry, SampledCapture};

const ENROLLMENT_WORK: usize = 1_000_000;
const SEARCH_WORK: usize = 300_000_000;
const MAX_BASELINES: usize = 100_000;
const UNCERTAINTY_STEPS: u32 = 1;
const MAX_DESCRIPTOR_BYTES: usize = 1024 * 1024;

const IMAGES: [&[u8]; 2] = [
    include_bytes!("../assets/wallet-1-fingerprint.png"),
    include_bytes!("../assets/wallet-2-fingerprint.png"),
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FeaturePoint {
    pub id: usize,
    pub x: f64,
    pub y: f64,
    pub priority: f64,
}

#[derive(Clone, Serialize)]
pub struct FingerprintView {
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    /// Features that pass the demo's shared minimum priority, not raw candidates.
    pub feature_count: usize,
    pub points: Vec<FeaturePoint>,
}

pub struct SignedFingerprint {
    pub authorization: Authorization,
    pub leaf_hash: merkle::Hash,
    pub matched_feature_ids: Vec<usize>,
    pub attempts: usize,
}

/// Image analysis and enrollment descriptors shared by visitor sessions.
/// These bundled synthetic sources are public demo fixtures, not user captures.
pub struct FingerprintTemplate {
    fingerprint: Arc<PreparedFingerprint>,
    candidates: Vec<Candidate>,
}

struct PreparedFingerprint {
    image_index: usize,
    parameters: ProfileParameters,
    view: FingerprintView,
    pattern: PatternGeometry,
}

/// Each visitor has a separate public enrollment package. The shared capture
/// metadata contains no signing candidates or reconstructed private keys.
pub struct FingerprintWallet {
    fingerprint: Arc<PreparedFingerprint>,
    package: EnrollmentPackage,
}

impl FingerprintTemplate {
    pub fn prepare(index: usize, points_per_combination: Option<usize>) -> Result<Self> {
        let source = source_image(index)?;
        let capture = Capture::new(
            source.width(),
            source.height(),
            source.into_raw(),
            None,
            None,
        )?;
        let extracted =
            fingerprint::extract(&capture, &ExtractionConfig::default()).with_context(|| {
                format!(
                    "wallet {} fingerprint enrollment analysis failed",
                    index + 1
                )
            })?;
        let features = extracted.features()?;
        let pattern = PatternGeometry::new(
            capture.width(),
            capture.height(),
            &extracted.scale.pattern_support,
        )?;
        drop(extracted);
        let config = combination_config(points_per_combination);
        let combinations =
            fingerprint::generate_combinations(&features, &config).with_context(|| {
                format!(
                    "wallet {} fingerprint combination selection failed",
                    index + 1
                )
            })?;
        let parameters = ProfileParameters::new(&config, UNCERTAINTY_STEPS)
            .map_err(anyhow::Error::from_boxed)?;
        let grid = fingerprint::prepare_grid_baselines(
            &combinations.points,
            &combinations.combinations,
            features.unit_px,
            &GridBaselineConfig {
                padding_steps: UNCERTAINTY_STEPS,
                max_baselines: MAX_BASELINES as u64,
                max_work: ENROLLMENT_WORK,
            },
        )?;
        let candidates = grid
            .candidates(nonzero(MAX_DESCRIPTOR_BYTES))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(anyhow::Error::from_boxed)?;
        let points = feature_points(&combinations.points);
        let view = FingerprintView {
            image_url: format!("/api/fingerprints/{index}"),
            width: u32::try_from(features.width)?,
            height: u32::try_from(features.height)?,
            feature_count: points.len(),
            points,
        };

        Ok(Self {
            fingerprint: Arc::new(PreparedFingerprint {
                image_index: index,
                parameters,
                view,
                pattern,
            }),
            candidates,
        })
    }

    /// Derive session-specific keys from fresh enrollment randomness. Only the
    /// resulting public hash tree is retained; the template never stores keys.
    pub fn enroll(&self, network: &str, wallet_id: &str) -> Result<FingerprintWallet> {
        let parameters = &self.fingerprint.parameters;
        let context = enrollment_context(parameters, network, wallet_id)?;
        let limits = limits(MAX_BASELINES);
        let candidates =
            Box::new(self.candidates.iter().map(|candidate| {
                Candidate::new(candidate.as_bytes().to_vec()).map_err(Into::into)
            }));
        let enrolled =
            enrollment::enroll_candidates(&ExampleCrypto, candidates, context, None, &limits)?;
        let package = EnrollmentPackage::from_enrollment(&enrolled, &limits)?
            .with_profile_parameters(parameters.encoded().to_vec(), &limits)?;
        // Validate the exact public transport representation before publishing it.
        Commitment::from_core(package.commitment())?;

        Ok(FingerprintWallet {
            fingerprint: Arc::clone(&self.fingerprint),
            package,
        })
    }
}

impl FingerprintWallet {
    pub fn commitment(&self) -> Result<Commitment> {
        Ok(Commitment::from_core(self.package.commitment())?)
    }

    pub fn view(&self) -> FingerprintView {
        self.fingerprint.view.clone()
    }

    pub fn sample(&self, seed: u64) -> Result<SampledCapture> {
        self.fingerprint
            .pattern
            .sample_with_seed(&source_image(self.fingerprint.image_index)?, seed)
    }

    pub fn sign(
        &self,
        action: Action,
        sampled: &SampledCapture,
        on_features: impl FnOnce(Vec<FeaturePoint>),
        cancelled: &AtomicBool,
    ) -> Result<SignedFingerprint> {
        check_signing_cancelled(cancelled)?;
        let expected = action.to_core()?;
        // Only the cropped and masked observation enters extraction.
        let features = fingerprint::analyze(&sampled.capture, &ExtractionConfig::default())
            .context("the partial scan could not be analyzed")?;
        check_signing_cancelled(cancelled)?;
        on_features(observed_points(&features, sampled.metadata.offset));
        check_signing_cancelled(cancelled)?;
        let mut search =
            fingerprint::prepare_baseline_signing_search(&features, &self.search_config()?)?;
        let attempts = Cell::new(0_usize);
        let candidates = signing_candidates(search.by_ref(), cancelled, &attempts);
        let signed = authorization::sign_candidates(
            &ExampleCrypto,
            candidates,
            self.fingerprint.parameters.profile_id(),
            &self.package,
            expected,
            None,
            &limits(SEARCH_WORK),
        )
        .with_context(|| {
            format!(
                "wallet {} fingerprint search failed after {} candidate attempts",
                self.fingerprint.image_index + 1,
                attempts.get()
            )
        })?;

        check_signing_cancelled(cancelled)?;
        let context = self
            .package
            .commitment()
            .context
            .encode(&limits(SEARCH_WORK))?;
        let leaf_hash = merkle::leaf_hash(&context, &signed.public_key);

        let authorization = Authorization::from_core(&signed)?;
        check_signing_cancelled(cancelled)?;

        Ok(SignedFingerprint {
            authorization,
            leaf_hash,
            matched_feature_ids: search.last_feature_ids().to_vec(),
            attempts: attempts.get(),
        })
    }

    fn search_config(&self) -> Result<SigningSearchConfig> {
        let controls = self
            .fingerprint
            .parameters
            .controls(SEARCH_WORK)
            .map_err(anyhow::Error::from_boxed)?;
        Ok(SigningSearchConfig {
            min_points: controls.min_points,
            max_points: controls.max_points,
            min_priority: controls.min_priority,
            spatial: controls.spatial,
            radius_units: controls.radius_units,
            uncertainty_steps: self.fingerprint.parameters.uncertainty_steps(),
            max_work: SEARCH_WORK,
            max_descriptor_bytes: nonzero(MAX_DESCRIPTOR_BYTES),
        })
    }
}

/// Cancellation is checked around each generation step, before the yielded
/// descriptor reaches key derivation. An error terminates this stream.
fn signing_candidates<'a>(
    mut source: impl Iterator<Item = std::result::Result<Candidate, AlgorithmError>> + 'a,
    cancelled: &'a AtomicBool,
    attempts: &'a Cell<usize>,
) -> Candidates<'a> {
    let mut finished = false;
    Box::new(std::iter::from_fn(move || {
        if finished {
            return None;
        }
        let result = (|| {
            check_signing_cancelled(cancelled).map_err(anyhow::Error::into_boxed_dyn_error)?;
            let Some(candidate) = source.next() else {
                return Ok(None);
            };
            check_signing_cancelled(cancelled).map_err(anyhow::Error::into_boxed_dyn_error)?;
            candidate.map(Some)
        })();
        match result {
            Ok(Some(candidate)) => {
                attempts.set(attempts.get() + 1);
                Some(Ok(candidate))
            }
            Ok(None) => {
                finished = true;
                None
            }
            Err(error) => {
                finished = true;
                Some(Err(error))
            }
        }
    }))
}

fn check_signing_cancelled(cancelled: &AtomicBool) -> Result<()> {
    ensure!(
        !cancelled.load(Ordering::Relaxed),
        "fingerprint signing was cancelled"
    );
    Ok(())
}

/// Preserve incomplete-search semantics instead of describing a resource stop
/// as a biometric mismatch. The full typed cause remains in server logs.
pub fn attempt_failure(error: &anyhow::Error) -> Option<&'static str> {
    use fingerprint::{CombinationError, FingerprintError, GridShapeError};
    use gnorix::core::{Error, error::Resource};
    for cause in error.chain() {
        if matches!(
            cause.downcast_ref::<GridShapeError>(),
            Some(GridShapeError::WorkLimit)
        ) || matches!(
            cause.downcast_ref::<CombinationError>(),
            Some(CombinationError::WorkLimit)
        ) || matches!(
            cause.downcast_ref::<Error>(),
            Some(Error::LimitExceeded(Resource::CandidateAttempts))
        ) {
            return Some(
                "The search budget was reached before completion. This does not establish a mismatch. Try a new random capture.",
            );
        }
        if matches!(
            cause.downcast_ref::<Error>(),
            Some(Error::CandidateNotEnrolled)
        ) {
            return Some(
                "This capture did not reconstruct an enrolled key. Try a new random capture.",
            );
        }
        if matches!(
            cause.downcast_ref::<CombinationError>(),
            Some(
                CombinationError::InsufficientPoints { .. } | CombinationError::PatternUnavailable
            )
        ) {
            return Some(
                "This capture has too few usable features for signing. Try a new random capture.",
            );
        }
        if matches!(
            cause.downcast_ref::<FingerprintError>(),
            Some(FingerprintError::WorkLimit)
        ) {
            return Some(
                "This capture exceeded the image-analysis budget. No signature was created. Try a new random capture.",
            );
        }
    }
    None
}

pub fn image(index: usize) -> Option<&'static [u8]> {
    IMAGES.get(index).copied()
}

fn source_image(index: usize) -> Result<image::GrayImage> {
    let bytes = image(index).ok_or_else(|| anyhow!("unknown fingerprint image index {index}"))?;
    Ok(
        image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .context("cannot decode the bundled fingerprint PNG")?
            .to_luma8(),
    )
}

fn observed_points(features: &FingerprintFeatures, offset: [f64; 2]) -> Vec<FeaturePoint> {
    features
        .features
        .iter()
        .filter_map(|feature| {
            let priority = feature.priority?;
            (priority >= combination_config(None).min_priority).then(|| FeaturePoint {
                id: feature.id,
                x: feature.position.x() + offset[0],
                y: feature.position.y() + offset[1],
                priority,
            })
        })
        .collect()
}

fn combination_config(points_per_combination: Option<usize>) -> CombinationConfig {
    CombinationConfig {
        min_points: points_per_combination.unwrap_or(3),
        max_points: points_per_combination.unwrap_or(6),
        min_priority: 0.1,
        spatial: SpatialPolicy {
            min_anchor_points: 3,
            min_anchor_distance_units: 5.0,
        },
        radius_units: 30.0,
        max_work: ENROLLMENT_WORK,
    }
}

fn enrollment_context(
    parameters: &ProfileParameters,
    network: &str,
    wallet_id: &str,
) -> Result<EnrollmentContext> {
    Ok(EnrollmentContext {
        spec: EnrollmentSpec {
            domain: Domain {
                network: network.to_owned(),
                verifier: wallet_id.to_owned(),
            },
            subject: wallet_id.to_owned(),
            id: random_bytes()?,
            revision: NonZeroU64::new(1).unwrap(),
            public_salt: random_bytes()?.to_vec(),
            secret_policy: SecretPolicy::Absent,
        },
        profile_id: parameters.profile_id(),
        suite_id: SUITE_ID.to_owned(),
    })
}

fn random_bytes() -> Result<[u8; 32]> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|error| anyhow!("random source failed: {error}"))?;
    Ok(bytes)
}

fn limits(candidate_attempts: usize) -> Limits {
    Limits {
        candidate_attempts: nonzero(candidate_attempts),
        ..verification_limits()
    }
}

fn nonzero(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("fixed resource limit is nonzero")
}

fn feature_points(points: &[CombinationPoint]) -> Vec<FeaturePoint> {
    points
        .iter()
        .map(|point| FeaturePoint {
            id: point.id,
            x: point.position.x(),
            y: point.position.y(),
            priority: point.priority,
        })
        .collect()
}

#[cfg(test)]
mod tests;
