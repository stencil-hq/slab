//! Builds the host-agnostic ABI module and publishes it to the Go, Python, and
//! Swift clients.
//!
//! Go and Python embed deterministic gzip bytes. Swift embeds the raw module
//! because `WasmKit` accepts uncompressed bytes and Foundation has no portable
//! gzip decoder.

use std::{
	error::Error,
	fs,
	path::{Path, PathBuf},
	process::Command,
};

/// Compressed client copies, relative to the repository root.
const COMPRESSED_TARGETS: &[&str] =
	&["clients/go/slab/slab_abi.wasm.gz", "packages/pyslab/src/slab/slab_abi.wasm.gz"];

/// Raw client copies, relative to the repository root.
const RAW_TARGETS: &[&str] = &["clients/swift/Sources/Slab/Resources/slab_abi.wasm"];

/// Builds `slab_abi.wasm` and writes every client copy.
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
	let module = fs::read(&wasm)?;
	for target in COMPRESSED_TARGETS {
		let path: PathBuf = root.join(target);
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent)?;
		}
		fs::write(&path, &packed)?;
	}
	for target in RAW_TARGETS {
		let path: PathBuf = root.join(target);
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent)?;
		}
		fs::write(&path, &module)?;
	}
	eprintln!(
		"abi-wasm: {} bytes -> {} gzipped, published to {} clients",
		module.len(),
		packed.len(),
		COMPRESSED_TARGETS.len() + RAW_TARGETS.len()
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
