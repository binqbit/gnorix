use std::{
    cell::Cell,
    error::Error as StdError,
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use gnorix::{
    authorization::{self, Action, Authorization, EnrollmentStatus},
    biometrics::fingerprint::{
        self, CombinationError, CombinationPoint, ExtractionConfig, FeatureKind, GridShapeError,
        GridShapes, SigningSearch, SigningSearchConfig,
    },
    crypto::SignatureVerifier,
    enrollment::{self, Domain, EnrollmentContext, EnrollmentSpec},
    package::{EnrollmentPackage, PackageLimits},
    policy::{Limits, SecretPolicy},
};
use image::GrayImage;

use super::{capture, model::*};
use crate::{Result, authorization_json, enrollment_profile, output, provider::ExampleCrypto};

pub struct Engine {
    source: GrayImage,
    output: PathBuf,
    next_id: u64,
    prepared: Option<Prepared>,
    enrolled: Option<Enrolled>,
    data: LabData,
}

struct Prepared {
    view: PreparedView,
    work: PreparedWork,
    consumed: bool,
}

enum PreparedWork {
    Enrollment(GridShapes),
    Signing(SigningSearch),
}

struct Enrolled {
    package: EnrollmentPackage,
    config: LabConfig,
    authorization: Option<Authorization>,
}

impl Engine {
    pub fn new(source: GrayImage, output: PathBuf) -> Self {
        Self {
            source,
            output,
            next_id: 1,
            prepared: None,
            enrolled: None,
            data: LabData::default(),
        }
    }

    pub fn view(&self) -> LabData {
        self.data.clone()
    }

    pub fn prepare(&mut self, request: PrepareRequest, phase: impl Fn(&str)) -> Result<()> {
        request.config.validate()?;
        validate_work(request.work_limit)?;
        if request.kind != "enrollment" && request.kind != "signing" {
            return Err("preparation kind must be enrollment or signing".into());
        }
        if request.kind == "enrollment" && request.selection.is_some() {
            return Err("enrollment preparation uses the full source image".into());
        }
        if request.kind == "signing" && request.selection.is_none() {
            return Err("signing preparation requires a selected circle".into());
        }
        if request.kind == "signing" {
            let enrolled = self
                .enrolled
                .as_ref()
                .ok_or("create an enrollment before preparing a signing fragment")?;
            require_config(enrolled.config, request.config)?;
        }

        self.prepared = None;
        self.data.prepared = None;
        self.data.result = None;
        if self
            .enrolled
            .as_ref()
            .is_some_and(|enrolled| enrolled.config != request.config)
        {
            self.enrolled = None;
            self.data.enrollment = None;
        }
        phase("Analyzing fingerprint");
        let selected = capture::select(&self.source, request.selection)?;
        let extracted = fingerprint::extract(&selected.capture, &ExtractionConfig::default())?;
        phase("Preparing regions and features");
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("preparation identifier overflow")?;
        let mut view = PreparedView {
            id,
            kind: request.kind.clone(),
            config: request.config,
            selection: request.selection,
            work_limit: request.work_limit,
            unit_px: extracted.scale.unit_px,
            points: Vec::new(),
            regions: Vec::new(),
            combination_count: 0,
            variant_count: None,
        };
        let work = if request.kind == "enrollment" {
            let mut combination_config = request.config.combinations()?;
            combination_config.max_work = request.work_limit;
            let combinations = fingerprint::generate_combinations(&extracted, &combination_config)?;
            view.points = point_views(&combinations.points, selected.offset);
            view.regions = combinations
                .regions
                .iter()
                .map(|region| RegionView {
                    id: region.id,
                    x: region.center.x() + selected.offset[0],
                    y: region.center.y() + selected.offset[1],
                    initial_radius: region.initial_radius,
                    radius: region.radius,
                    pool: region.pool.clone(),
                    available_points: region.available_points,
                })
                .collect();
            view.combination_count = combinations.combinations.len();
            let mut grid_config = request.config.grid();
            grid_config.max_work = request.work_limit;
            let grid = fingerprint::prepare_grid_shapes(
                &combinations.points,
                &combinations.combinations,
                view.unit_px,
                &grid_config,
            )?;
            view.variant_count = Some(grid.variant_count());
            PreparedWork::Enrollment(grid)
        } else {
            let search = fingerprint::prepare_signing_search(
                &extracted,
                &SigningSearchConfig {
                    min_points: request.config.min_points,
                    radius_units: request.config.radius_units,
                    enrollment_padding: request.config.uncertainty_steps,
                    max_work: request.work_limit,
                    max_descriptor_bytes: nonzero(1024 * 1024)?,
                },
            )?;
            view.points = point_views(search.points(), selected.offset);
            view.regions = search
                .regions()
                .iter()
                .map(|region| RegionView {
                    id: region.id,
                    x: region.center.x() + selected.offset[0],
                    y: region.center.y() + selected.offset[1],
                    initial_radius: region.initial_radius,
                    radius: region.radius,
                    pool: region.feature_ids.clone(),
                    available_points: region.feature_ids.len(),
                })
                .collect();
            PreparedWork::Signing(search)
        };
        self.data.prepared = Some(view.clone());
        self.prepared = Some(Prepared {
            view,
            work,
            consumed: false,
        });
        Ok(())
    }

    pub fn enroll(&mut self, request: PreparedId, phase: impl Fn(&str)) -> Result<()> {
        let prepared = self
            .prepared
            .as_ref()
            .ok_or("prepare the full fingerprint first")?;
        require_prepared(prepared.view.id, request.prepared_id)?;
        let PreparedWork::Enrollment(grid) = &prepared.work else {
            return Err("this preparation is a signing fragment".into());
        };
        phase("Deriving keys and building the Merkle tree");
        let limits = limits(usize::try_from(MAX_VARIANTS)?)?;
        let context = enrollment_context(prepared.view.config)?;
        let enrolled = enrollment::enroll_candidates(
            &ExampleCrypto,
            grid.candidates(nonzero(1024 * 1024)?),
            context,
            None,
            &limits,
        )?;
        let package = EnrollmentPackage::from_enrollment(&enrolled, &limits)?;
        phase("Verifying and saving enrollment");
        let bytes = package.encode(&package_limits()?, &limits)?;
        let path = self.output.join(package.filename());
        output::write_new(&path, &bytes)?;
        let restored = EnrollmentPackage::decode(
            &output::read(&path, 64 * 1024 * 1024)?,
            &package_limits()?,
            &limits,
        )?;
        if restored.commitment() != package.commitment() {
            return Err("retained package differs after readback".into());
        }
        let view = EnrollmentView {
            root: hex(&restored.commitment().root.0),
            leaf_count: restored.commitment().leaf_count,
            config: prepared.view.config,
        };
        persist_json(
            &self.output,
            "enrollment",
            &serde_json::json!({"prepared": prepared.view, "enrollment": view}),
        )?;
        self.data.enrollment = Some(view);
        self.data.result = None;
        self.enrolled = Some(Enrolled {
            package: restored,
            config: prepared.view.config,
            authorization: None,
        });
        Ok(())
    }

    pub fn sign(&mut self, request: SignRequest, phase: impl Fn(&str)) -> Result<()> {
        validate_payload(&request.payload)?;
        validate_work(request.work_limit)?;
        validate_attempts(request.attempt_limit)?;
        let prepared = self
            .prepared
            .as_mut()
            .ok_or("prepare the selected fragment first")?;
        require_prepared(prepared.view.id, request.prepared_id)?;
        if request.work_limit != prepared.view.work_limit {
            return Err("work limit changed; prepare the fragment again".into());
        }
        let enrolled = self.enrolled.as_mut().ok_or("create an enrollment first")?;
        require_config(enrolled.config, prepared.view.config)?;
        let PreparedWork::Signing(search) = &mut prepared.work else {
            return Err("prepare a circular signing fragment first".into());
        };
        // A stream is consumed once; callers must prepare again before retrying.
        if prepared.consumed {
            return Err("this signing search was already consumed; prepare again".into());
        }
        phase("Searching enrollment for a matching key");
        let limits = limits(request.attempt_limit)?;
        let action = action(request.payload)?;
        prepared.consumed = true;
        let generated = Cell::new(0usize);
        let candidates = Box::new(search.by_ref().inspect(|candidate| {
            if candidate.is_ok() {
                generated.set(generated.get() + 1);
            }
        }));
        let outcome = authorization::sign_candidates(
            &ExampleCrypto,
            candidates,
            enrolled.package.commitment().context.profile_id,
            &enrolled.package,
            action.clone(),
            None,
            &limits,
        );
        let attempted = generated.get().min(request.attempt_limit);
        let result = match outcome {
            Ok(signed) => {
                phase("Verifying signature and Merkle proof");
                authorization::verify(
                    &ExampleCrypto,
                    enrolled.package.commitment(),
                    EnrollmentStatus::Active,
                    &action,
                    &signed,
                    now()?,
                    &limits,
                )?;
                let encoded = authorization_json::encode_authorization(&signed);
                let restored = authorization_json::decode_authorization(&encoded, &limits)?;
                authorization::verify(
                    &ExampleCrypto,
                    enrolled.package.commitment(),
                    EnrollmentStatus::Active,
                    &action,
                    &restored,
                    now()?,
                    &limits,
                )?;
                let result = RunResult {
                    status: "matched".into(),
                    message: "Key found. Signature and Merkle proof verified.".into(),
                    attempts: attempted,
                    feature_ids: search.last_feature_ids().to_vec(),
                    authorization: Some(encoded),
                    verified: true,
                };
                enrolled.authorization = Some(restored);
                result
            }
            Err(error) => {
                let status = failure_status(&error);
                let message = match status {
                    "no_match" => {
                        "No match found after exhausting all permitted regional combinations."
                            .into()
                    }
                    "incomplete" => format!("Search incomplete: {}", error_detail(&error)),
                    _ => format!("Computation failed: {}", error_detail(&error)),
                };
                enrolled.authorization = None;
                RunResult {
                    status: status.into(),
                    message,
                    attempts: attempted,
                    feature_ids: Vec::new(),
                    authorization: None,
                    verified: false,
                }
            }
        };
        persist_json(
            &self.output,
            "signing",
            &serde_json::json!({"prepared": prepared.view, "root": hex(&enrolled.package.commitment().root.0), "result": result}),
        )?;
        self.data.result = Some(result);
        Ok(())
    }

    pub fn verify(&mut self, request: VerifyRequest) -> Result<()> {
        if let Some(result) = self.data.result.as_mut() {
            result.verified = false;
            result.message = "New verification has not completed.".into();
        }
        validate_payload(&request.payload)?;
        let enrolled = self.enrolled.as_ref().ok_or("create an enrollment first")?;
        let signed = enrolled
            .authorization
            .as_ref()
            .ok_or("no successful signature is available")?;
        let mut expected = signed.action.clone();
        expected.payload = request.payload.into_bytes();
        let verification = authorization::verify(
            &ExampleCrypto,
            enrolled.package.commitment(),
            EnrollmentStatus::Active,
            &expected,
            signed,
            now()?,
            &limits(1)?,
        );
        let result = self
            .data
            .result
            .as_mut()
            .ok_or("no signing result is available")?;
        result.verified = verification.is_ok();
        result.message = match verification {
            Ok(_) => "Signature and Merkle proof verified for this message.".into(),
            Err(error) => format!("Verification failed: {error}"),
        };
        Ok(())
    }
}

fn point_views(points: &[CombinationPoint], offset: [f64; 2]) -> Vec<PointView> {
    points
        .iter()
        .map(|point| PointView {
            id: point.id,
            x: point.position.x() + offset[0],
            y: point.position.y() + offset[1],
            priority: point.priority,
            kind: match point.kind {
                FeatureKind::Ending => "ending",
                FeatureKind::Bifurcation => "bifurcation",
                FeatureKind::Junction => "junction",
                FeatureKind::Isolated => "isolated",
                FeatureKind::Cycle => "cycle",
            }
            .into(),
        })
        .collect()
}

fn require_prepared(actual: u64, requested: u64) -> Result<()> {
    if actual != requested {
        return Err("preparation is stale; prepare again".into());
    }
    Ok(())
}

fn require_config(enrolled: LabConfig, current: LabConfig) -> Result<()> {
    if enrolled != current {
        return Err("parameters changed; create a new enrollment".into());
    }
    Ok(())
}

fn validate_work(work: usize) -> Result<()> {
    if !(1..=MAX_WORK).contains(&work) {
        return Err("work limit must be between 1 and 1000000000".into());
    }
    Ok(())
}

fn validate_attempts(attempts: usize) -> Result<()> {
    if !(1..=MAX_ATTEMPTS).contains(&attempts) {
        return Err(format!("attempt limit must be between 1 and {MAX_ATTEMPTS}").into());
    }
    Ok(())
}

fn validate_payload(payload: &str) -> Result<()> {
    if payload.len() > 16 * 1024 {
        return Err("payload exceeds 16 KiB".into());
    }
    Ok(())
}

fn nonzero(value: usize) -> Result<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "resource limit must be nonzero".into())
}

fn limits(attempts: usize) -> Result<Limits> {
    Ok(Limits {
        candidate_attempts: nonzero(attempts)?,
        candidate_bytes: nonzero(32)?,
        public_key_bytes: nonzero(32)?,
        signature_bytes: nonzero(64)?,
        action_bytes: nonzero(64 * 1024)?,
        context_bytes: nonzero(64 * 1024)?,
    })
}

fn package_limits() -> Result<PackageLimits> {
    Ok(PackageLimits {
        max_bytes: nonzero(64 * 1024 * 1024)?,
        max_leaves: nonzero(usize::try_from(MAX_VARIANTS)?)?,
    })
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut value = [0; N];
    getrandom::fill(&mut value).map_err(|error| format!("random source failed: {error}"))?;
    Ok(value)
}

fn enrollment_context(config: LabConfig) -> Result<EnrollmentContext> {
    Ok(EnrollmentContext {
        spec: EnrollmentSpec {
            domain: Domain {
                network: "local".into(),
                verifier: "gnorix-fingerprint-lab".into(),
            },
            subject: "synthetic-test-fingerprint".into(),
            id: random_bytes()?,
            revision: NonZeroU64::new(1).expect("nonzero revision"),
            public_salt: random_bytes::<32>()?.to_vec(),
            secret_policy: SecretPolicy::Absent,
        },
        profile_id: enrollment_profile::profile_id(
            &config.combinations()?,
            config.uncertainty_steps,
        )?,
        suite_id: ExampleCrypto.suite_id().into(),
    })
}

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn action(payload: String) -> Result<Action> {
    Ok(Action {
        operation: "fingerprint-lab-test".into(),
        payload: payload.into_bytes(),
        nonce: random_bytes()?,
        expires_at: now()?.checked_add(3600).ok_or("clock overflow")?,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn persist_json(
    directory: &std::path::Path,
    prefix: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let path = directory.join(format!("{prefix}-{}.json", hex(&random_bytes::<12>()?)));
    output::write_new(&path, &serde_json::to_vec_pretty(value)?)
}

fn failure_status(error: &(dyn StdError + 'static)) -> &'static str {
    if matches!(
        error.downcast_ref::<gnorix::Error>(),
        Some(gnorix::Error::CandidateNotEnrolled)
    ) {
        return "no_match";
    }
    if matches!(
        error.downcast_ref::<gnorix::Error>(),
        Some(gnorix::Error::LimitExceeded(_))
    ) || matches!(
        error.downcast_ref::<CombinationError>(),
        Some(CombinationError::WorkLimit)
    ) || matches!(
        error.downcast_ref::<GridShapeError>(),
        Some(
            GridShapeError::WorkLimit
                | GridShapeError::VariantLimit { .. }
                | GridShapeError::CountOverflow
        )
    ) {
        return "incomplete";
    }
    error.source().map_or("error", failure_status)
}

fn error_detail(error: &(dyn StdError + 'static)) -> String {
    let mut details = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source {
        let message = error.to_string();
        if details.last() != Some(&message) {
            details.push(message);
        }
        source = error.source();
    }
    details.join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_exhaustion_limits_and_actual_failures() {
        assert_eq!(
            failure_status(&gnorix::Error::CandidateNotEnrolled),
            "no_match"
        );
        assert_eq!(
            failure_status(&gnorix::Error::LimitExceeded(
                gnorix::error::Resource::CandidateAttempts
            )),
            "incomplete"
        );
        assert_eq!(failure_status(&CombinationError::WorkLimit), "incomplete");
        assert_eq!(
            failure_status(&gnorix::Error::Algorithm {
                stage: gnorix::error::Stage::Reconstruction,
                source: Box::new(GridShapeError::WorkLimit)
            }),
            "incomplete"
        );
        assert_eq!(failure_status(&gnorix::Error::InvalidSignature), "error");
    }

    #[test]
    fn identifiers_config_and_bounds_reject_stale_or_invalid_operations() {
        assert!(require_prepared(3, 3).is_ok());
        assert!(require_prepared(3, 2).is_err());
        assert!(
            require_config(
                LabConfig::default(),
                LabConfig {
                    radius_units: 31.0,
                    ..LabConfig::default()
                }
            )
            .is_err()
        );
        assert!(validate_work(0).is_err());
        assert!(validate_work(MAX_WORK + 1).is_err());
        assert!(validate_payload(&"a".repeat(16 * 1024 + 1)).is_err());
    }

    #[test]
    fn accepts_a_billion_attempts_without_allocating_or_running_the_search() {
        assert_eq!(MAX_ATTEMPTS, 1_000_000_000);
        for count in [1, 1_000_001, MAX_ATTEMPTS] {
            validate_attempts(count).unwrap();
            assert_eq!(limits(count).unwrap().candidate_attempts.get(), count);
        }
        for count in [0, MAX_ATTEMPTS + 1, usize::MAX] {
            let error = validate_attempts(count).unwrap_err().to_string();
            assert_eq!(
                error,
                format!("attempt limit must be between 1 and {MAX_ATTEMPTS}")
            );
        }
        assert_eq!(MAX_WORK, 1_000_000_000);
    }

    #[test]
    fn rejected_verification_does_not_leave_a_previous_success_visible() {
        let mut engine = Engine::new(GrayImage::new(1, 1), PathBuf::new());
        engine.data.result = Some(RunResult {
            status: "matched".into(),
            message: "Previously verified".into(),
            attempts: 1,
            feature_ids: vec![3, 7, 9],
            authorization: None,
            verified: true,
        });
        assert!(
            engine
                .verify(VerifyRequest {
                    payload: "a".repeat(16 * 1024 + 1)
                })
                .is_err()
        );
        let result = engine.view().result.unwrap();
        assert!(!result.verified);
        assert_eq!(result.feature_ids, [3, 7, 9]);
        assert_eq!(result.message, "New verification has not completed.");
    }
}
