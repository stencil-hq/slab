//! Builds the host-agnostic ABI module and publishes it to the Go and Python
//! clients.
//!
//! Both clients embed the same bytes, gzip-compressed to keep the checked-in
//! artifact near two megabytes instead of five, and decompress once at load.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Client copies of the compressed module, relative to the repo root.
const TARGETS: &[&str] = &[
    "clients/go/slab/slab_abi.wasm.gz",
    "packages/pyslab/src/slab/slab_abi.wasm.gz",
];

/// Builds `slab_abi.wasm` and writes the compressed copies both clients embed.
pub fn run(root: &Path) -> Result<(), Box<dyn Error>> {
    run_command(Command::new("cargo").current_dir(root).args([
        "build",
        "-p",
        "slab-abi",
        "--target",
        "wasm32-unknown-unknown",
        "--profile",
        "wasm-release",
    ]))?;

    let wasm = root.join("target/wasm32-unknown-unknown/wasm-release/slab_abi.wasm");
    if !wasm.is_file() {
        return Err(format!("ABI build did not produce {}", wasm.display()).into());
    }
    let packed = gzip(&wasm)?;

    for target in TARGETS {
        let path: PathBuf = root.join(target);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &packed)?;
    }
    eprintln!(
        "abi-wasm: {} bytes -> {} gzipped, published to {} clients",
        fs::metadata(&wasm)?.len(),
        packed.len(),
        TARGETS.len()
    );
    Ok(())
}

/// Compresses one file with a fixed configuration so regeneration is
/// byte-stable: `-9` fixes the level and `-n` drops the name and timestamp.
fn gzip(path: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
    let output = Command::new("gzip")
        .args(["-9", "-n", "-c"])
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(format!("gzip failed with {}", output.status).into());
    }
    Ok(output.stdout)
}

fn run_command(command: &mut Command) -> Result<(), Box<dyn Error>> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("{command:?} failed with {status}").into());
    }
    Ok(())
}
