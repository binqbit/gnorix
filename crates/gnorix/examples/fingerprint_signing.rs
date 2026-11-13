//! Sign with the first reconstructed member, or verify a public authorization.

#[path = "support/authorization_json.rs"]
mod authorization_json;
#[path = "support/enrollment_profile.rs"]
mod enrollment_profile;
#[path = "support/image_input.rs"]
mod image_input;
#[path = "support/output.rs"]
mod output;
#[path = "support/example_crypto.rs"]
mod provider;
#[path = "support/secret_input.rs"]
mod secret_input;
#[path = "fingerprint_signing/settings.rs"]
mod settings;

use std::{
    cell::Cell,
    env,
    error::Error,
    fs::File,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use gnorix::{
    authorization::{self, Action, Authorization, EnrollmentStatus},
    biometrics::{
        Capture,
        fingerprint::{self, ExtractionConfig, Fingerprint},
    },
    crypto::SignatureVerifier,
    package::EnrollmentPackage,
    policy::Limits,
};

use provider::ExampleCrypto;
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const MAX_PUBLIC_JSON_BYTES: usize = 256 * 1024;

fn main() -> Result<()> {
    match arguments()? {
        Command::Sign {
            image,
            package,
            action,
            output,
        } => sign(&image, &package, &action, &output),
        Command::Verify {
            package,
            action,
            authorization,
            now,
        } => verify(&package, &action, &authorization, now),
    }
}

fn sign(image: &Path, package_path: &Path, action_path: &Path, output_path: &Path) -> Result<()> {
    require_new_output(output_path)?;
    let limits = settings::limits(settings::attempt_limit()?)?;
    let package = read_package(package_path, &limits)?;
    let controls = settings::read(&package)?;
    let action = read_action(action_path, &limits)?;
    validate_profile(&package, controls.profile_id)?;
    let secret = secret_input::read_secret(package.commitment().context.spec.secret_policy)?;
    let fingerprint = extract(image)?;
    let unit_px = fingerprint.scale.unit_px;
    let candidates = fingerprint::prepare_signing_search(&fingerprint, &controls.search)?;
    let point_count = candidates.points().len();
    let region_count = candidates.regions().len();
    let spatial = controls.search.spatial;
    let min_diameter_units = spatial.min_diameter_units;
    let min_anchor_points = spatial.min_anchor_points;
    let min_anchor_separation_units = spatial.min_anchor_separation_units;
    let attempted = Cell::new(0_usize);
    let counted = Box::new(candidates.inspect(|result| {
        if result.is_ok() {
            attempted.set(attempted.get() + 1);
        }
    }));
    let signed = authorization::sign_candidates(
        &ExampleCrypto,
        counted,
        controls.profile_id,
        &package,
        action.clone(),
        secret.as_ref(),
        &limits,
    );
    let signed = match signed {
        Ok(signed) => signed,
        Err(error) => {
            eprintln!(
                "search failed after generating {} candidates; {point_count} eligible features; {region_count} regions; minimum diameter {min_diameter_units:.6}R; at least {min_anchor_points} anchors separated by at least {min_anchor_separation_units:.6}R: {error}",
                attempted.get(),
            );
            return Err(error.into());
        }
    };
    publish_verified(&package, &action, &signed, output_path, &limits)?;
    println!(
        "first matching key after {} candidates; {point_count} eligible features; {region_count} regions; minimum diameter {min_diameter_units:.6}R; at least {min_anchor_points} anchors separated by at least {min_anchor_separation_units:.6}R; R={unit_px:.6}px; signature and Merkle proof verified\n{}",
        attempted.get(),
        output_path.display(),
    );
    Ok(())
}

fn verify(
    package_path: &Path,
    action_path: &Path,
    authorization_path: &Path,
    now: u64,
) -> Result<()> {
    let limits = settings::limits(1)?;
    let package = read_package(package_path, &limits)?;
    let expected = read_action(action_path, &limits)?;
    let signed = read_authorization(authorization_path, &limits)?;
    verify_authorization(&package, &expected, &signed, now, &limits)?;
    println!(
        "signature and Merkle membership verified against the trusted enrollment and expected action"
    );
    Ok(())
}

fn extract(path: &Path) -> Result<Fingerprint> {
    let controls = ExtractionConfig::default();
    let image = image_input::read(path, controls.max_pixels)?;
    let capture = Capture::new(image.width(), image.height(), image.into_raw(), None, None)?;
    Ok(fingerprint::extract(&capture, &controls)?)
}

fn validate_profile(package: &EnrollmentPackage, expected: gnorix::merkle::Hash) -> Result<()> {
    if package.commitment().context.profile_id != expected {
        return Err(gnorix::Error::ProfileMismatch.into());
    }
    if package.commitment().context.suite_id != ExampleCrypto.suite_id() {
        return Err(gnorix::Error::SuiteMismatch.into());
    }
    Ok(())
}

fn read_package(path: &Path, limits: &Limits) -> Result<EnrollmentPackage> {
    let capacity = settings::package_limits()?;
    let bytes = output::read(path, capacity.max_bytes.get())?;
    let package = EnrollmentPackage::decode(&bytes, &capacity, limits)?;
    settings::parameters(&package)?;
    Ok(package)
}

fn read_action(path: &Path, limits: &Limits) -> Result<Action> {
    authorization_json::read_action(&mut File::open(path)?, MAX_PUBLIC_JSON_BYTES, limits)
}

fn read_authorization(path: &Path, limits: &Limits) -> Result<Authorization> {
    let bytes = output::read(path, MAX_PUBLIC_JSON_BYTES)?;
    authorization_json::decode_authorization(&serde_json::from_slice(&bytes)?, limits)
}

fn publish_verified(
    package: &EnrollmentPackage,
    expected: &Action,
    signed: &Authorization,
    path: &Path,
    limits: &Limits,
) -> Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    verify_authorization(package, expected, signed, now, limits)?;
    let bytes = serde_json::to_vec_pretty(&authorization_json::encode_authorization(signed))?;
    if bytes.len() > MAX_PUBLIC_JSON_BYTES {
        return Err("authorization JSON exceeds the example byte limit".into());
    }
    output::write_new(path, &bytes)?;
    let restored = read_authorization(path, limits)?;
    verify_authorization(package, expected, &restored, now, limits)
}

fn verify_authorization(
    package: &EnrollmentPackage,
    expected: &Action,
    signed: &Authorization,
    now: u64,
    limits: &Limits,
) -> Result<()> {
    authorization::verify(
        &ExampleCrypto,
        package.commitment(),
        EnrollmentStatus::Active,
        expected,
        signed,
        now,
        limits,
    )?;
    Ok(())
}

fn require_new_output(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err("authorization output already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

enum Command {
    Sign {
        image: PathBuf,
        package: PathBuf,
        action: PathBuf,
        output: PathBuf,
    },
    Verify {
        package: PathBuf,
        action: PathBuf,
        authorization: PathBuf,
        now: u64,
    },
}

fn arguments() -> Result<Command> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 5 {
        return Err("usage: fingerprint_signing sign <fresh.png> <trusted.grx> <action.json> <new-authorization.json> | verify <trusted.grx> <expected-action.json> <authorization.json> <unix-time>".into());
    }
    match args[0].to_str() {
        Some("sign") => Ok(Command::Sign {
            image: PathBuf::from(&args[1]),
            package: PathBuf::from(&args[2]),
            action: PathBuf::from(&args[3]),
            output: PathBuf::from(&args[4]),
        }),
        Some("verify") => Ok(Command::Verify {
            package: PathBuf::from(&args[1]),
            action: PathBuf::from(&args[2]),
            authorization: PathBuf::from(&args[3]),
            now: args[4].to_str().ok_or("time must be UTF-8")?.parse()?,
        }),
        _ => Err("expected sign or verify subcommand".into()),
    }
}
