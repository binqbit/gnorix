//! Real fingerprint extraction, enrollment and fresh key reconstruction for the
//! two bundled synthetic demo images. Only public enrollment material is kept.

use std::{
    cell::Cell,
    num::{NonZeroU64, NonZeroUsize},
};

use anyhow::{Context, Result, anyhow};
use gnorix::{
    core::{
        authorization,
        biometrics::{
            Capture,
            fingerprint::{
                self, CombinationConfig, CombinationPoint, ExtractionConfig, FingerprintFeatures,
                GridBaselineConfig, SigningSearchConfig, SpatialPolicy,
            },
        },
        enrollment::{self, Domain, EnrollmentContext, EnrollmentSpec},
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
const SEARCH_WORK: usize = 100_000_000;
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
    pub matched_feature_ids: Vec<usize>,
    pub attempts: usize,
}

/// No candidate, reconstructed key or source analysis is cached for signing.
pub struct FingerprintWallet {
    image_index: usize,
    package: EnrollmentPackage,
    parameters: ProfileParameters,
    view: FingerprintView,
    pattern: PatternGeometry,
}

impl FingerprintWallet {
    pub fn enroll(index: usize, network: &str, wallet_id: &str) -> Result<Self> {
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
        let config = combination_config();
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
        let context = enrollment_context(&parameters, network, wallet_id)?;
        let limits = limits(MAX_BASELINES);
        let enrolled = enrollment::enroll_candidates(
            &ExampleCrypto,
            grid.candidates(nonzero(MAX_DESCRIPTOR_BYTES)),
            context,
            None,
            &limits,
        )?;
        let package = EnrollmentPackage::from_enrollment(&enrolled, &limits)?
            .with_profile_parameters(parameters.encoded().to_vec(), &limits)?;
        // Validate the exact public transport representation before publishing it.
        Commitment::from_core(package.commitment())?;
        let points = feature_points(&combinations.points);
        let view = FingerprintView {
            image_url: format!("/api/fingerprints/{index}"),
            width: u32::try_from(features.width)?,
            height: u32::try_from(features.height)?,
            feature_count: points.len(),
            points,
        };

        Ok(Self {
            image_index: index,
            package,
            parameters,
            view,
            pattern,
        })
    }

    pub fn commitment(&self) -> Result<Commitment> {
        Ok(Commitment::from_core(self.package.commitment())?)
    }

    pub fn view(&self) -> FingerprintView {
        self.view.clone()
    }

    pub fn sample(&self, seed: u64) -> Result<SampledCapture> {
        self.pattern
            .sample_with_seed(&source_image(self.image_index)?, seed)
    }

    pub fn sign(
        &self,
        action: Action,
        sampled: &SampledCapture,
        on_features: impl FnOnce(Vec<FeaturePoint>),
    ) -> Result<SignedFingerprint> {
        let expected = action.to_core()?;
        // Only the cropped and masked observation enters extraction.
        let features = fingerprint::analyze(&sampled.capture, &ExtractionConfig::default())
            .context("the partial scan could not be analyzed")?;
        on_features(observed_points(&features, sampled.metadata.offset));
        let mut search = fingerprint::prepare_signing_search(&features, &self.search_config()?)?;
        let attempts = Cell::new(0_usize);
        let candidates = Box::new(search.by_ref().inspect(|candidate| {
            if candidate.is_ok() {
                attempts.set(attempts.get() + 1);
            }
        }));
        let signed = authorization::sign_candidates(
            &ExampleCrypto,
            candidates,
            self.parameters.profile_id(),
            &self.package,
            expected,
            None,
            &limits(SEARCH_WORK),
        )
        .with_context(|| {
            format!(
                "wallet {} fingerprint search failed after {} candidate attempts",
                self.image_index + 1,
                attempts.get()
            )
        })?;

        Ok(SignedFingerprint {
            authorization: Authorization::from_core(&signed)?,
            matched_feature_ids: search.last_feature_ids().to_vec(),
            attempts: attempts.get(),
        })
    }

    fn search_config(&self) -> Result<SigningSearchConfig> {
        let controls = self
            .parameters
            .controls(SEARCH_WORK)
            .map_err(anyhow::Error::from_boxed)?;
        Ok(SigningSearchConfig {
            min_points: controls.min_points,
            max_points: controls.max_points,
            min_priority: controls.min_priority,
            spatial: controls.spatial,
            radius_units: controls.radius_units,
            uncertainty_steps: self.parameters.uncertainty_steps(),
            max_work: SEARCH_WORK,
            max_descriptor_bytes: nonzero(MAX_DESCRIPTOR_BYTES),
        })
    }
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
            (priority >= combination_config().min_priority).then(|| FeaturePoint {
                id: feature.id,
                x: feature.position.x() + offset[0],
                y: feature.position.y() + offset[1],
                priority,
            })
        })
        .collect()
}

fn combination_config() -> CombinationConfig {
    CombinationConfig {
        min_points: 3,
        max_points: 6,
        min_priority: 0.1,
        spatial: SpatialPolicy {
            min_anchor_points: 3,
            min_anchor_distance_units: 6.0,
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
mod tests {
    use super::*;

    #[test]
    fn image_selection_rejects_unknown_sources() {
        assert!(image(0).is_some());
        assert!(image(1).is_some());
        assert!(image(2).is_none());
        assert!(FingerprintWallet::enroll(2, "near:local", "wallet.testnet").is_err());
    }

    #[test]
    #[ignore = "runs real extraction and an extended cryptographic candidate search"]
    fn reported_partial_capture_signs_with_enlarged_search_budget() {
        let wallet = FingerprintWallet::enroll(0, "near:regression", "wallet.fixture").unwrap();
        // This exact capture reached the former 1m work limit with 12 usable
        // features. Its selection remains independent of the signing outcome.
        let sampled = wallet.sample(457).unwrap();
        assert_eq!(sampled.metadata.circle.x, 2.0);
        assert_eq!(sampled.metadata.circle.y, 134.0);
        assert_eq!(sampled.metadata.circle.radius, 123.00406497347964);
        let action = Action {
            operation: "wallet.ft_transfer.v1".into(),
            payload: b"reported-partial-capture-regression".to_vec(),
            nonce: 0,
            expires_at_ms: 2_000_000_000_000,
        };

        let signed = wallet
            .sign(action.clone(), &sampled, |points| {
                assert_eq!(points.len(), 12)
            })
            .unwrap();
        assert!(signed.attempts > 0);
        assert!(!signed.matched_feature_ids.is_empty());

        let commitment = wallet.commitment().unwrap().to_core().unwrap();
        let authorization = signed.authorization.to_core().unwrap();
        let mut expected = action.to_core().unwrap();
        let verify = |expected: &gnorix::core::authorization::Action| {
            authorization::verify(
                &ExampleCrypto,
                &commitment,
                authorization::EnrollmentStatus::Active,
                expected,
                &authorization,
                1_900_000_000_000,
                &verification_limits(),
            )
        };
        // Verify descriptor membership and the action signature. This does not
        // assert correspondence to the same physical minutiae in both scans.
        verify(&expected).unwrap();
        expected.payload.push(0);
        assert!(verify(&expected).is_err());
    }
}
