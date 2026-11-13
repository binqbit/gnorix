//! Enroll one baseline per imported feature combination with the example crypto suite.

#[path = "support/enrollment_profile.rs"]
mod enrollment_profile;
#[path = "feature_grid/input.rs"]
mod input;
#[path = "support/output.rs"]
mod output;
#[path = "support/example_crypto.rs"]
mod provider;
#[path = "support/secret_input.rs"]
mod secret_input;
#[path = "enroll_grid/settings.rs"]
mod settings;

use std::{
    env,
    error::Error,
    fs::File,
    path::{Path, PathBuf},
};

use gnorix::{
    biometrics::fingerprint::{self, GridBaselines},
    crypto::{CryptoProvider, SecretInput, SignatureVerifier},
    enrollment::{self, EnrollmentContext},
    merkle,
    package::{EnrollmentPackage, PackageLimits},
    policy::Limits,
};

use enrollment_profile::ProfileParameters;
use provider::ExampleCrypto;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const MAX_JSON_BYTES: usize = 64 * 1024 * 1024;

fn main() -> Result<()> {
    let (input_path, context_path, output_directory) = arguments()?;
    let prepared = prepare(&input_path, &context_path)?;
    let package = enroll(&prepared)?;
    let (path, bytes) = publish(&package, &prepared, &output_directory)?;
    println!(
        "{} baseline candidates; {} distinct Merkle leaves; {} bytes; membership and signature readback verified\n{}",
        prepared.grid.baseline_count(),
        package.commitment().leaf_count,
        bytes,
        path.display(),
    );
    Ok(())
}

struct PreparedEnrollment {
    parameters: ProfileParameters,
    grid: GridBaselines,
    context: EnrollmentContext,
    secret: Option<SecretInput>,
    limits: Limits,
    package_limits: PackageLimits,
}

fn prepare(input_path: &Path, context_path: &Path) -> Result<PreparedEnrollment> {
    let config = settings::baseline_config()?;
    let limits = settings::limits(&config)?;
    let package_limits = settings::package_limits(&config)?;
    let spec = settings::read_context(&mut File::open(context_path)?)?;
    let secret = secret_input::read_secret(spec.secret_policy)?;
    let input = input::read(&mut File::open(input_path)?, MAX_JSON_BYTES)?;
    let parameters = settings::parameters(&input, &config)?;
    let context = EnrollmentContext {
        spec,
        profile_id: parameters.profile_id(),
        suite_id: ExampleCrypto.suite_id().to_owned(),
    };
    context.encode(&limits)?;
    let grid = fingerprint::prepare_grid_baselines(
        &input.points,
        &input.combinations,
        input.unit_px,
        &config,
    )?;
    Ok(PreparedEnrollment {
        parameters,
        grid,
        context,
        secret,
        limits,
        package_limits,
    })
}

fn enroll(prepared: &PreparedEnrollment) -> Result<EnrollmentPackage> {
    let candidates = prepared
        .grid
        .candidates(settings::nonzero(settings::MAX_DESCRIPTOR_BYTES)?);
    let enrollment = enrollment::enroll_candidates(
        &ExampleCrypto,
        candidates,
        prepared.context.clone(),
        prepared.secret.as_ref(),
        &prepared.limits,
    )?;
    Ok(
        EnrollmentPackage::from_enrollment(&enrollment, &prepared.limits)?
            .with_profile_parameters(prepared.parameters.encoded().to_vec(), &prepared.limits)?,
    )
}

fn publish(
    package: &EnrollmentPackage,
    prepared: &PreparedEnrollment,
    output_directory: &Path,
) -> Result<(PathBuf, usize)> {
    let bytes = package.encode(&prepared.package_limits, &prepared.limits)?;
    let path = output_directory.join(package.filename());
    output::write_new(&path, &bytes)?;
    let readback = output::read(&path, prepared.package_limits.max_bytes.get())?;
    let restored =
        EnrollmentPackage::decode(&readback, &prepared.package_limits, &prepared.limits)?;
    if restored.commitment() != package.commitment() {
        return Err("package readback changed the enrollment commitment".into());
    }
    verify_member(
        &prepared.grid,
        &restored,
        prepared.secret.as_ref(),
        &prepared.limits,
    )?;
    Ok((path, bytes.len()))
}

fn arguments() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 3 {
        return Err(
            "usage: enroll_grid <combinations.json> <context.json> <output-directory>".into(),
        );
    }
    Ok((
        PathBuf::from(&arguments[0]),
        PathBuf::from(&arguments[1]),
        PathBuf::from(&arguments[2]),
    ))
}

fn verify_member(
    prepared: &GridBaselines,
    package: &EnrollmentPackage,
    secret: Option<&SecretInput>,
    limits: &Limits,
) -> Result<()> {
    let candidate = prepared
        .candidates(settings::nonzero(settings::MAX_DESCRIPTOR_BYTES)?)
        .next()
        .ok_or("no candidate for package readback")??;
    let context = package.commitment().context.encode(limits)?;
    let key = ExampleCrypto.derive_key(&candidate, &context, secret)?;
    let public = ExampleCrypto.public_key(&key)?;
    let proof = package.proof_for(&public, limits)?;
    merkle::verify(
        package.commitment().root,
        merkle::leaf_hash(&context, &public),
        &proof,
    )?;
    let message = b"gnorix.example.package-readback.v1";
    let signature = ExampleCrypto.sign(&key, message)?;
    if !ExampleCrypto.verify(&public, message, &signature)? {
        return Err("example signature readback failed".into());
    }
    Ok(())
}
