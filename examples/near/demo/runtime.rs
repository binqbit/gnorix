//! Use supplied contract artifacts or build them with the repository's pinned
//! Rust target and `near` cfg. Cargo checks local build freshness each time.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Output, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use tokio::process::Command;

const TOOLCHAIN: &str = "1.99.1";
const WASM_TARGET: &str = "wasm32-unknown-unknown";
const CONTRACT_ARTIFACTS: [&str; 2] = ["gnorix_near_wallet.wasm", "gnorix_near_token.wasm"];
const WASM_HEADER: [u8; 8] = [0, b'a', b's', b'm', 1, 0, 0, 0];

/// Cancellation kills build processes instead of leaving a blocking startup job.
/// The returned directory contains both contracts ready for sandbox deployment.
pub async fn prepare_contracts(progress: impl Fn(&str) + Send + Sync) -> Result<PathBuf> {
    if let Some(directory) = std::env::var_os("GNORIX_DEMO_CONTRACTS_DIR") {
        ensure!(
            !directory.is_empty(),
            "GNORIX_DEMO_CONTRACTS_DIR must name a directory"
        );
        let contracts_dir = validate_contracts_dir(Path::new(&directory))?;
        progress("Bundled contracts are ready; starting the local NEAR sandbox");
        return Ok(contracts_dir);
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .context("cannot locate the Gnorix repository from the demo crate")?;
    let target_dir = root.join("target/near-demo/contracts");

    progress("Checking the pinned Rust toolchain and WebAssembly target");
    prepare_wasm_target(&progress).await?;
    let rustc = pinned_compiler().await?;

    progress("Building the smart-wallet contract (the first build may take a few minutes)");
    build_contract(&root, &target_dir, &rustc, "examples/near/Cargo.toml").await?;

    progress("Building the standard token contract");
    build_contract(&root, &target_dir, &rustc, "examples/near/token/Cargo.toml").await?;

    let contracts_dir = validate_contracts_dir(&target_dir.join(WASM_TARGET).join("release"))?;
    progress("Contracts are ready; starting the local NEAR sandbox");
    Ok(contracts_dir)
}

fn validate_contracts_dir(directory: &Path) -> Result<PathBuf> {
    let directory = directory
        .canonicalize()
        .with_context(|| format!("cannot locate contract directory {}", directory.display()))?;
    ensure!(directory.is_dir(), "contract path must be a directory");
    for artifact in CONTRACT_ARTIFACTS {
        let path = directory.join(artifact);
        let metadata = std::fs::metadata(&path)
            .with_context(|| format!("cannot inspect contract {artifact}"))?;
        ensure!(
            metadata.is_file(),
            "contract {artifact} must be a regular file"
        );
        let mut header = [0_u8; 8];
        std::fs::File::open(&path)
            .and_then(|mut file| file.read_exact(&mut header))
            .with_context(|| format!("cannot read WebAssembly header of {artifact}"))?;
        ensure!(
            header == WASM_HEADER,
            "contract {artifact} must be a WebAssembly version 1 module"
        );
    }
    Ok(directory)
}

async fn prepare_wasm_target(progress: &impl Fn(&str)) -> Result<()> {
    let installed = run(
        Command::new("rustup").args(["target", "list", "--installed", "--toolchain", TOOLCHAIN]),
        "check Rust 1.99.1 targets; install the repository's Rust toolchain first",
    )
    .await?;
    if String::from_utf8_lossy(&installed.stdout)
        .lines()
        .any(|target| target == WASM_TARGET)
    {
        return Ok(());
    }

    progress("Installing Rust's WebAssembly target for the pinned toolchain");
    run(
        Command::new("rustup").args(["target", "add", "--toolchain", TOOLCHAIN, WASM_TARGET]),
        "install the WebAssembly target",
    )
    .await?;
    Ok(())
}

async fn pinned_compiler() -> Result<PathBuf> {
    let output = run(
        Command::new("rustup").args(["which", "--toolchain", TOOLCHAIN, "rustc"]),
        "locate the pinned Rust compiler",
    )
    .await?;
    let path = PathBuf::from(
        String::from_utf8(output.stdout)
            .context("Rust compiler path is not UTF-8")?
            .trim(),
    );
    ensure!(path.is_file(), "pinned Rust compiler does not exist");
    Ok(path)
}

async fn build_contract(
    root: &Path,
    target_dir: &Path,
    rustc: &Path,
    manifest: &str,
) -> Result<()> {
    let mut command = Command::new("rustup");
    command
        .current_dir(root)
        .args([
            "run",
            TOOLCHAIN,
            "cargo",
            "build",
            "--locked",
            "--release",
            "--target",
            WASM_TARGET,
            "--lib",
            "--no-default-features",
            "--manifest-path",
            manifest,
            "--target-dir",
        ])
        .arg(target_dir)
        // Match cargo-near's contract configuration, without its optional
        // wasm-opt size pass. Keep host/demo features out of contract builds.
        .env("RUSTFLAGS", "-C link-arg=-s --cfg near")
        // System Cargo/Rust packages can coexist with rustup. Cargo must not
        // discover a different system compiler through PATH.
        .env("RUSTC", rustc)
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    run(&mut command, &format!("build {manifest}")).await?;
    Ok(())
}

async fn run(command: &mut Command, action: &str) -> Result<Output> {
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .spawn()
        .with_context(|| format!("could not {action}"))?;
    #[cfg(unix)]
    let mut group = ProcessGroup::new(child.id().context("spawned child has no process ID")?)?;
    let output = child
        .wait_with_output()
        .await
        .with_context(|| format!("could not {action}"))?;
    #[cfg(unix)]
    group.disarm();
    if !output.status.success() {
        bail!(
            "could not {action}: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(output)
}

/// Cargo launches rustc/linkers. Killing only the direct child would leave those
/// descendants running after shutdown. Each command owns its own process group.
#[cfg(unix)]
struct ProcessGroup(Option<rustix::process::Pid>);

#[cfg(unix)]
impl ProcessGroup {
    fn new(id: u32) -> Result<Self> {
        let id = i32::try_from(id).context("child process ID is out of range")?;
        ensure!(id > 1, "invalid child process group");
        Ok(Self(rustix::process::Pid::from_raw(id)))
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(group) = self.0.take() {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::time::Duration;

    use tokio::time::{sleep, timeout};

    use super::*;

    fn contract_fixture() -> Result<PathBuf> {
        let mut nonce = [0_u8; 8];
        getrandom::fill(&mut nonce)?;
        let directory =
            std::env::temp_dir().join(format!("gnorix-contracts-{}", u64::from_le_bytes(nonce)));
        std::fs::create_dir(&directory)?;
        for artifact in CONTRACT_ARTIFACTS {
            std::fs::write(directory.join(artifact), WASM_HEADER)?;
        }
        Ok(directory)
    }

    #[test]
    fn supplied_contract_directory_accepts_complete_wasm_bundle() -> Result<()> {
        let directory = contract_fixture()?;
        assert_eq!(
            validate_contracts_dir(&directory)?,
            directory.canonicalize()?
        );
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn supplied_contract_directory_rejects_missing_or_nonregular_artifacts() -> Result<()> {
        let directory = contract_fixture()?;
        let wallet = directory.join(CONTRACT_ARTIFACTS[0]);
        std::fs::remove_file(&wallet)?;
        let error = validate_contracts_dir(&directory).unwrap_err().to_string();
        assert!(error.contains(CONTRACT_ARTIFACTS[0]));
        std::fs::create_dir(&wallet)?;
        let error = validate_contracts_dir(&directory).unwrap_err().to_string();
        assert!(error.contains("regular file"));
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn supplied_contract_directory_rejects_truncated_or_invalid_wasm() -> Result<()> {
        let directory = contract_fixture()?;
        let token = directory.join(CONTRACT_ARTIFACTS[1]);
        for bytes in [b"\0asm".as_slice(), b"not wasm", b"\0asm\x02\0\0\0"] {
            std::fs::write(&token, bytes)?;
            let error = validate_contracts_dir(&directory).unwrap_err().to_string();
            assert!(error.contains(CONTRACT_ARTIFACTS[1]));
        }
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn canceling_build_kills_the_child_and_its_descendants() -> Result<()> {
        let mut nonce = [0_u8; 8];
        getrandom::fill(&mut nonce)?;
        let directory =
            std::env::temp_dir().join(format!("gnorix-build-cancel-{}", u64::from_le_bytes(nonce)));
        std::fs::create_dir(&directory)?;
        let parent_path = directory.join("parent");
        let child_path = directory.join("child");
        let files = [parent_path.clone(), child_path.clone()];
        let task = tokio::spawn(async move {
            run(
                Command::new("sh")
                    .args([
                        "-c",
                        "echo $$ > \"$1\"; sleep 120 & echo $! > \"$2\"; wait",
                        "fixture",
                    ])
                    .arg(parent_path)
                    .arg(child_path),
                "run cancellation fixture",
            )
            .await
        });
        let pids = timeout(Duration::from_secs(5), async {
            loop {
                let observed = files.each_ref().map(|path| {
                    std::fs::read_to_string(path)
                        .ok()?
                        .trim()
                        .parse::<u32>()
                        .ok()
                });
                if let [Some(parent), Some(child)] = observed {
                    break [parent, child];
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        timeout(Duration::from_secs(5), async {
            while pids.iter().any(|pid| process_is_running(*pid)) {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("canceled startup left a build process running")?;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    fn process_is_running(pid: u32) -> bool {
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        // A terminated orphan may briefly remain a zombie until PID 1 reaps it.
        status
            .rsplit_once(") ")
            .is_some_and(|(_, status)| !status.starts_with(['Z', 'X']))
    }
}
