use std::{
    cell::Cell,
    error::Error as StdError,
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use gnorix::{
    authorization::{self, Action, Authorization, EnrollmentStatus},
    biometrics::fingerprint::{
        self, CombinationError, CombinationPoint, ExtractionConfig, FeatureKind,
        GridBaselineConfig, GridBaselines, GridShapeError, SigningSearch, SigningSearchConfig,
    },
    crypto::SignatureVerifier,
    enrollment::{self, Domain, EnrollmentContext, EnrollmentSpec},
    package::EnrollmentPackage,
    policy::{Limits, SecretPolicy},
};
use image::{GrayImage, ImageEncoder};
use sha2::{Digest, Sha256};

use super::{
    artifacts, capture,
    model::*,
    resources::{DEFAULT_MEMORY_GIB, EnrollmentBudget},
};
use crate::{Result, authorization_json, enrollment_profile, output, provider::ExampleCrypto};

pub struct Engine {
    source: GrayImage,
    frame: ArtifactFrame,
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
    Enrollment {
        grid: GridBaselines,
        budget: EnrollmentBudget,
    },
    Signing(Box<SigningSearch>),
}

struct Enrolled {
    package: EnrollmentPackage,
    config: LabConfig,
    authorization: Option<Authorization>,
    unit_px: f64,
}

struct ArtifactFrame {
    view: ArtifactView,
    image: GrayImage,
    png: Arc<[u8]>,
}

impl Engine {
    pub fn new(source: GrayImage, output: PathBuf) -> Result<Self> {
        let frame = ArtifactFrame::new(source.clone(), 0, 0, artifact_seed(None)?, None)?;
        let data = LabData {
            artifact: frame.view.clone(),
            ..LabData::default()
        };
        Ok(Self {
            source,
            frame,
            output,
            next_id: 1,
            prepared: None,
            enrolled: None,
            data,
        })
    }

    pub fn view(&self) -> LabData {
        self.data.clone()
    }

    pub fn preview(&self) -> (u64, Arc<[u8]>) {
        (self.frame.view.revision, Arc::clone(&self.frame.png))
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.source.width(), self.source.height())
    }

    pub fn change_artifacts(
        &mut self,
        request: ArtifactRequest,
        phase: impl Fn(&str),
    ) -> Result<()> {
        if request.level > 100 {
            return Err("artifact level must be between 0 and 100".into());
        }
        let unit_px = self
            .enrolled
            .as_ref()
            .ok_or("create a clean enrollment before adding artifacts")?
            .unit_px;
        if request.level == self.frame.view.level && !request.randomize {
            return Ok(());
        }
        let seed = artifact_seed(Some(self.frame.view.seed))?;
        let revision = self
            .frame
            .view
            .revision
            .checked_add(1)
            .ok_or("artifact revision overflow")?;
        phase("Rendering scan artifacts");
        let image = artifacts::apply(&self.source, unit_px, request.level, u64::from(seed))?;
        let frame = ArtifactFrame::new(image, revision, request.level, seed, Some(unit_px))?;
        self.data.artifact = frame.view.clone();
        self.frame = frame;
        if self
            .prepared
            .as_ref()
            .is_some_and(|prepared| matches!(prepared.work, PreparedWork::Signing(_)))
        {
            self.prepared = None;
            self.data.prepared = None;
        }
        self.data.result = None;
        if let Some(enrolled) = self.enrolled.as_mut() {
            enrolled.authorization = None;
        }
        Ok(())
    }

    pub fn test(&mut self, request: TestRequest, phase: impl Fn(&str)) -> Result<()> {
        validate_payload(&request.payload)?;
        validate_attempts(request.attempt_limit)?;
        self.prepare(
            PrepareRequest {
                kind: "signing".into(),
                config: request.config,
                selection: Some(request.selection),
                work_limit: request.work_limit,
                artifact_revision: Some(request.artifact_revision),
                memory_budget_gib: DEFAULT_MEMORY_GIB,
            },
            &phase,
        )?;
        let prepared_id = self
            .prepared
            .as_ref()
            .ok_or("test preparation missing")?
            .view
            .id;
        self.sign(
            SignRequest {
                prepared_id,
                payload: request.payload,
                attempt_limit: request.attempt_limit,
                work_limit: request.work_limit,
            },
            phase,
        )
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
        if request.kind == "enrollment" && request.artifact_revision.is_some() {
            return Err("clean enrollment must not select an altered scan revision".into());
        }
        if request.kind == "signing" && request.selection.is_none() {
            return Err("signing preparation requires a selected circle".into());
        }
        if request.kind == "signing" {
            self.require_artifact_revision(request.artifact_revision)?;
            let enrolled = self
                .enrolled
                .as_ref()
                .ok_or("create an enrollment before preparing a signing fragment")?;
            require_config(enrolled.config, request.config)?;
        }

        let enrollment_budget = if request.kind == "enrollment" {
            Some(EnrollmentBudget::new(
                request.memory_budget_gib,
                self.retained_leaf_count(),
            )?)
        } else {
            None
        };

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
        let source = if request.kind == "enrollment" {
            &self.source
        } else {
            &self.frame.image
        };
        let selected = capture::select(source, request.selection)?;
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
            artifact_revision: request.artifact_revision,
            unit_px: extracted.scale.unit_px,
            points: Vec::new(),
            regions: Vec::new(),
            combination_count: 0,
            candidate_count: None,
            memory_budget_gib: None,
            candidate_limit: None,
            estimated_peak_bytes: None,
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
            let budget = enrollment_budget.ok_or("enrollment memory budget missing")?;
            let grid = prepare_enrollment_grid(
                &combinations.points,
                &combinations.combinations,
                view.unit_px,
                request.config.uncertainty_steps,
                request.work_limit,
                budget,
            )?;
            view.candidate_count = Some(grid.baseline_count());
            view.memory_budget_gib = Some(budget.memory_budget_gib());
            view.candidate_limit = Some(budget.candidate_limit());
            view.estimated_peak_bytes = Some(budget.estimated_peak_bytes(grid.baseline_count())?);
            PreparedWork::Enrollment { grid, budget }
        } else {
            let search = fingerprint::prepare_signing_search(
                &extracted,
                &SigningSearchConfig {
                    min_points: request.config.min_points,
                    radius_units: request.config.radius_units,
                    uncertainty_steps: request.config.uncertainty_steps,
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
            PreparedWork::Signing(Box::new(search))
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
        let retained_leaves = self.retained_leaf_count();
        let prepared = self
            .prepared
            .as_mut()
            .ok_or("prepare the full fingerprint first")?;
        require_prepared(prepared.view.id, request.prepared_id)?;
        let PreparedWork::Enrollment { grid, budget } = &mut prepared.work else {
            return Err("this preparation is a signing fragment".into());
        };
        let effective_budget = EnrollmentBudget::new(budget.memory_budget_gib(), retained_leaves)?;
        let estimated_peak = effective_budget.estimated_peak_bytes(grid.baseline_count())?;
        *budget = effective_budget;
        prepared.view.candidate_limit = Some(budget.candidate_limit());
        prepared.view.estimated_peak_bytes = Some(estimated_peak);
        self.data.prepared = Some(prepared.view.clone());
        phase("Deriving keys and building the Merkle tree");
        let limits = limits(usize::try_from(budget.candidate_limit())?)?;
        let package_limits = budget.package_limits()?;
        let context = enrollment_context(prepared.view.config)?;
        let enrolled = enrollment::enroll_candidates(
            &ExampleCrypto,
            grid.candidates(nonzero(1024 * 1024)?),
            context,
            None,
            &limits,
        )?;
        let package = EnrollmentPackage::from_enrollment(&enrolled, &limits)?;
        drop(enrolled);
        phase("Verifying and saving enrollment");
        let expected = package.commitment().clone();
        let bytes = package.encode(&package_limits, &limits)?;
        let path = self.output.join(package.filename());
        output::write_new(&path, &bytes)?;
        drop(bytes);
        drop(package);
        let bytes = output::read(&path, package_limits.max_bytes.get())?;
        let restored = EnrollmentPackage::decode(&bytes, &package_limits, &limits)?;
        drop(bytes);
        if restored.commitment() != &expected {
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
            unit_px: prepared.view.unit_px,
        });
        Ok(())
    }

    pub fn sign(&mut self, request: SignRequest, phase: impl Fn(&str)) -> Result<()> {
        validate_payload(&request.payload)?;
        validate_work(request.work_limit)?;
        validate_attempts(request.attempt_limit)?;
        let artifact_revision = self
            .prepared
            .as_ref()
            .ok_or("prepare the selected fragment first")?
            .view
            .artifact_revision;
        self.require_artifact_revision(artifact_revision)?;
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
                        "No match found after exhausting all regional combinations and their grid uncertainty variants.".into()
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
            &serde_json::json!({"prepared": prepared.view, "artifact": self.frame.view, "root": hex(&enrolled.package.commitment().root.0), "result": result}),
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

    fn require_artifact_revision(&self, revision: Option<u64>) -> Result<()> {
        if revision != Some(self.frame.view.revision) {
            return Err("scan revision changed; load the current preview and prepare again".into());
        }
        Ok(())
    }

    fn retained_leaf_count(&self) -> u64 {
        self.enrolled
            .as_ref()
            .map_or(0, |enrolled| enrolled.package.commitment().leaf_count)
    }
}

impl ArtifactFrame {
    fn new(
        image: GrayImage,
        revision: u64,
        level: u8,
        seed: u32,
        ridge_width_px: Option<f64>,
    ) -> Result<Self> {
        let image_sha256 = hex(&Sha256::digest(image.as_raw()));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::L8,
        )?;
        Ok(Self {
            view: ArtifactView {
                revision,
                level,
                seed,
                image_url: format!("/api/scan/{revision}.png"),
                generator_version: ARTIFACT_GENERATOR.into(),
                image_sha256,
                ridge_width_px,
            },
            image,
            png: png.into(),
        })
    }
}

/// Capture variation uses the render time; cryptographic keys use separate entropy.
fn artifact_seed(previous: Option<u32>) -> Result<u32> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let seed = (timestamp as u32)
        ^ ((timestamp >> 32) as u32)
        ^ ((timestamp >> 64) as u32)
        ^ ((timestamp >> 96) as u32);
    Ok(if previous == Some(seed) {
        seed.wrapping_add(1)
    } else {
        seed
    })
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

fn prepare_enrollment_grid(
    points: &[CombinationPoint],
    combinations: &[fingerprint::FeatureCombination],
    unit_px: f64,
    uncertainty_steps: u32,
    max_work: usize,
    budget: EnrollmentBudget,
) -> Result<GridBaselines> {
    let config = GridBaselineConfig {
        padding_steps: uncertainty_steps,
        max_work,
        max_baselines: budget.candidate_limit(),
    };
    match fingerprint::prepare_grid_baselines(points, combinations, unit_px, &config) {
        Ok(grid) => Ok(grid),
        Err(error @ GridShapeError::BaselineLimit { required, .. }) => {
            match budget.estimated_peak_bytes(required) {
                Err(reason) => Err(reason),
                Ok(_) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
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
                | GridShapeError::BaselineLimit { .. }
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
    fn repeated_enrollment_rechecks_memory_without_requiring_new_preparation() {
        let directory = std::env::temp_dir().join(format!(
            "gnorix-memory-enrollment-{}",
            hex(&random_bytes::<12>().unwrap())
        ));
        let mut engine = Engine::new(GrayImage::new(2, 2), directory.clone()).unwrap();
        let config = LabConfig {
            uncertainty_steps: 0,
            ..LabConfig::default()
        };
        let points: Vec<_> = [(0.0, 0.0), (16.0, 0.0), (5.0, 12.0)]
            .into_iter()
            .enumerate()
            .map(|(id, (x, y))| CombinationPoint {
                id,
                kind: FeatureKind::Ending,
                position: gnorix::biometrics::geometry::Point::new(x, y).unwrap(),
                priority: 1.0,
            })
            .collect();
        let combinations = [fingerprint::FeatureCombination {
            feature_ids: vec![0, 1, 2],
            score: 1.0,
            regions: vec![0],
        }];
        let budget = EnrollmentBudget::new(DEFAULT_MEMORY_GIB, 0).unwrap();
        let grid =
            prepare_enrollment_grid(&points, &combinations, 4.0, 0, MAX_WORK, budget).unwrap();
        let view = PreparedView {
            id: 1,
            kind: "enrollment".into(),
            config,
            selection: None,
            work_limit: MAX_WORK,
            artifact_revision: None,
            unit_px: 4.0,
            points: Vec::new(),
            regions: Vec::new(),
            combination_count: 1,
            candidate_count: Some(1),
            memory_budget_gib: Some(DEFAULT_MEMORY_GIB),
            candidate_limit: Some(budget.candidate_limit()),
            estimated_peak_bytes: Some(budget.estimated_peak_bytes(1).unwrap()),
        };
        engine.prepared = Some(Prepared {
            view: view.clone(),
            work: PreparedWork::Enrollment { grid, budget },
            consumed: false,
        });
        engine.data.prepared = Some(view);
        engine
            .enroll(PreparedId { prepared_id: 1 }, |_| {})
            .unwrap();
        let first = engine.data.enrollment.as_ref().unwrap().root.clone();
        let first_estimate = engine
            .data
            .prepared
            .as_ref()
            .unwrap()
            .estimated_peak_bytes
            .unwrap();
        engine
            .enroll(PreparedId { prepared_id: 1 }, |_| {})
            .unwrap();
        assert_ne!(engine.data.enrollment.as_ref().unwrap().root, first);
        assert_eq!(
            engine
                .data
                .prepared
                .as_ref()
                .unwrap()
                .estimated_peak_bytes
                .unwrap(),
            first_estimate + 96
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn enrolled_engine() -> Engine {
        let source =
            GrayImage::from_fn(64, 80, |x, y| image::Luma([((x * 7 + y * 13) % 256) as u8]));
        let mut engine = Engine::new(source, PathBuf::new()).unwrap();
        let config = LabConfig::default();
        let limits = limits(1).unwrap();
        let candidate = gnorix::biometrics::Candidate::new(vec![7; 32]).unwrap();
        let candidates = Box::new(std::iter::once(Ok(candidate)));
        let enrollment = enrollment::enroll_candidates(
            &ExampleCrypto,
            candidates,
            enrollment_context(config).unwrap(),
            None,
            &limits,
        )
        .unwrap();
        let package = EnrollmentPackage::from_enrollment(&enrollment, &limits).unwrap();
        engine.data.enrollment = Some(EnrollmentView {
            root: hex(&package.commitment().root.0),
            leaf_count: package.commitment().leaf_count,
            config,
        });
        engine.enrolled = Some(Enrolled {
            package,
            config,
            authorization: None,
            unit_px: 5.0,
        });
        engine
    }

    #[test]
    fn altered_preview_uses_exact_pixels_and_preserves_clean_enrollment() {
        let mut engine = enrolled_engine();
        assert_eq!(engine.data.artifact.ridge_width_px, None);
        let source = engine.source.clone();
        let root = engine.data.enrollment.as_ref().unwrap().root.clone();
        let mut previous_seed = engine.data.artifact.seed;
        for level in [35, 80, 0] {
            engine
                .change_artifacts(
                    ArtifactRequest {
                        level,
                        randomize: false,
                    },
                    |_| {},
                )
                .unwrap();
            let decoded = image::load_from_memory(&engine.preview().1)
                .unwrap()
                .to_luma8();
            assert_eq!(decoded, engine.frame.image);
            assert_eq!(engine.source, source);
            assert_eq!(engine.data.enrollment.as_ref().unwrap().root, root);
            assert_ne!(engine.data.artifact.seed, previous_seed);
            previous_seed = engine.data.artifact.seed;
            assert_eq!(engine.data.artifact.level, level);
            assert_eq!(engine.data.artifact.ridge_width_px, Some(5.0));
            let metadata = serde_json::to_value(&engine.data.artifact).unwrap();
            assert_eq!(metadata["ridge_width_px"], 5.0);
            let reproduced = artifacts::apply(
                &source,
                metadata["ridge_width_px"].as_f64().unwrap(),
                metadata["level"].as_u64().unwrap() as u8,
                metadata["seed"].as_u64().unwrap(),
            )
            .unwrap();
            assert_eq!(reproduced, decoded);
            assert_eq!(
                engine.data.artifact.image_sha256,
                hex(&Sha256::digest(decoded.as_raw()))
            );
            if level == 0 {
                assert_eq!(decoded, source);
            }
        }
        engine
            .change_artifacts(
                ArtifactRequest {
                    level: 0,
                    randomize: true,
                },
                |_| {},
            )
            .unwrap();
        assert_ne!(engine.data.artifact.seed, previous_seed);
        assert_eq!(engine.frame.image, source);
        assert_eq!(engine.data.enrollment.as_ref().unwrap().root, root);
    }

    #[test]
    fn stale_scan_revision_is_rejected_before_extraction() {
        let mut engine = enrolled_engine();
        engine
            .change_artifacts(
                ArtifactRequest {
                    level: 20,
                    randomize: false,
                },
                |_| {},
            )
            .unwrap();
        let mut request = PrepareRequest {
            kind: "signing".into(),
            config: LabConfig::default(),
            selection: Some(Circle {
                x: 30.0,
                y: 40.0,
                radius: 20.0,
            }),
            work_limit: MAX_WORK,
            artifact_revision: Some(0),
            memory_budget_gib: DEFAULT_MEMORY_GIB,
        };
        assert!(
            engine
                .prepare(request, |_| {})
                .unwrap_err()
                .to_string()
                .contains("scan revision changed")
        );
        request = PrepareRequest {
            kind: "enrollment".into(),
            config: LabConfig::default(),
            selection: None,
            work_limit: MAX_WORK,
            artifact_revision: Some(engine.data.artifact.revision),
            memory_budget_gib: DEFAULT_MEMORY_GIB,
        };
        assert!(
            engine
                .prepare(request, |_| {})
                .unwrap_err()
                .to_string()
                .contains("clean enrollment")
        );
    }

    #[test]
    fn unchanged_artifacts_preserve_the_current_result_and_revision() {
        let mut engine = enrolled_engine();
        engine
            .change_artifacts(
                ArtifactRequest {
                    level: 30,
                    randomize: false,
                },
                |_| {},
            )
            .unwrap();
        let revision = engine.data.artifact.revision;
        let bytes = Arc::clone(&engine.frame.png);
        engine.data.result = Some(RunResult {
            status: "incomplete".into(),
            message: "retained result".into(),
            attempts: 1,
            feature_ids: Vec::new(),
            authorization: None,
            verified: false,
        });
        engine
            .change_artifacts(
                ArtifactRequest {
                    level: 30,
                    randomize: false,
                },
                |_| panic!("must not render unchanged settings"),
            )
            .unwrap();
        assert_eq!(engine.data.artifact.revision, revision);
        assert!(Arc::ptr_eq(&engine.frame.png, &bytes));
        assert_eq!(
            engine.data.result.as_ref().unwrap().message,
            "retained result"
        );
        engine
            .change_artifacts(
                ArtifactRequest {
                    level: 31,
                    randomize: false,
                },
                |_| {},
            )
            .unwrap();
        assert!(engine.data.result.is_none());
        assert_eq!(engine.data.artifact.revision, revision + 1);
    }

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
            failure_status(&GridShapeError::BaselineLimit {
                required: 2,
                limit: 1,
            }),
            "incomplete"
        );
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
        let mut engine = Engine::new(GrayImage::new(1, 1), PathBuf::new()).unwrap();
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
