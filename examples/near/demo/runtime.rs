//! Prepare the two contracts with the same pinned Rust target and `near` cfg
//! used by the repository's cargo-near builds. Cargo checks freshness each time.

use std::{
    path::{Path, PathBuf},
    process::{Output, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use tokio::process::Command;

const TOOLCHAIN: &str = "1.99.1";
const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// Cancellation kills build processes instead of leaving a blocking startup job.
/// The returned directory contains both contracts ready for sandbox deployment.
pub async fn prepare_contracts(progress: impl Fn(&str) + Send + Sync) -> Result<PathBuf> {
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

    let contracts_dir = target_dir.join(WASM_TARGET).join("release");
    for artifact in ["gnorix_near_wallet.wasm", "gnorix_near_token.wasm"] {
        ensure!(
            contracts_dir.join(artifact).is_file(),
            "contract build did not produce {artifact}"
        );
    }
    progress("Contracts are ready; starting the local NEAR sandbox");
    Ok(contracts_dir)
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
