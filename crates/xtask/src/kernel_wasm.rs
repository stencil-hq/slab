//! Builds wasm-bindgen artifacts for the browser kernel and Node conformance.

use std::{
	error::Error,
	fs,
	path::Path,
	process::{Command, ExitStatus},
};

/// Builds the optimized kernel WASM and emits web and Node bindings.
pub fn run(root: &Path) -> Result<(), Box<dyn Error>> {
	let bindgen_version = bindgen_version(&root.join("Cargo.lock"))?;
	ensure_bindgen(&bindgen_version)?;

	run_command(Command::new("cargo").current_dir(root).args([
		"build",
		"-p",
		"slab-kernel-wasm",
		"--profile",
		"wasm-release",
		"--target",
		"wasm32-unknown-unknown",
	]))?;

	let wasm = root.join("target/wasm32-unknown-unknown/wasm-release/slab_kernel_wasm.wasm");
	if !wasm.is_file() {
		return Err(format!("kernel WASM build did not produce {}", wasm.display()).into());
	}

	let web_dir = root.join("clients/web/wasm");
	recreate_dir(&web_dir)?;
	bindgen(&wasm, &web_dir, "web")?;

	let node_dir = root.join("target/kernel-wasm-node");
	recreate_dir(&node_dir)?;
	bindgen(&wasm, &node_dir, "nodejs")?;
	fs::write(node_dir.join("package.json"), "{ \"type\": \"commonjs\" }\n")?;

	eprintln!("kernel-wasm: wrote {} and {}", web_dir.display(), node_dir.display());
	Ok(())
}

fn bindgen_version(lock_path: &Path) -> Result<String, Box<dyn Error>> {
	let lock = fs::read_to_string(lock_path)?;
	let mut lines = lock.lines();
	while let Some(line) = lines.next() {
		if line == "name = \"wasm-bindgen\"" {
			for line in lines.by_ref() {
				if let Some(version) = line
					.strip_prefix("version = \"")
					.and_then(|value| value.strip_suffix('"'))
				{
					return Ok(version.to_owned());
				}
				if line == "[[package]]" {
					break;
				}
			}
		}
	}
	Err("Cargo.lock does not contain wasm-bindgen".into())
}

fn ensure_bindgen(version: &str) -> Result<(), Box<dyn Error>> {
	let installed = Command::new("wasm-bindgen").arg("--version").output();
	let matches = installed.is_ok_and(|output| {
		output.status.success()
			&& String::from_utf8_lossy(&output.stdout)
				.split_whitespace()
				.any(|part| part == version)
	});
	if matches {
		return Ok(());
	}

	eprintln!("kernel-wasm: installing wasm-bindgen-cli {version}");
	run_command(Command::new("cargo").args([
		"install",
		"wasm-bindgen-cli",
		"--version",
		version,
		"--locked",
	]))
}

fn bindgen(wasm: &Path, out_dir: &Path, target: &str) -> Result<(), Box<dyn Error>> {
	run_command(
		Command::new("wasm-bindgen")
			.arg("--target")
			.arg(target)
			.arg("--out-name")
			.arg("slab_kernel")
			.arg("--out-dir")
			.arg(out_dir)
			.arg(wasm),
	)
}

fn recreate_dir(path: &Path) -> Result<(), Box<dyn Error>> {
	match fs::remove_dir_all(path) {
		Ok(()) => {},
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
		Err(error) => return Err(error.into()),
	}
	fs::create_dir_all(path)?;
	Ok(())
}

fn run_command(command: &mut Command) -> Result<(), Box<dyn Error>> {
	let description = format!("{command:?}");
	let status: ExitStatus = command.status()?;
	if status.success() {
		Ok(())
	} else {
		Err(format!("command failed ({status}): {description}").into())
	}
}

#[cfg(test)]
mod tests {
	use std::{fs, time::SystemTime};

	use super::bindgen_version;

	#[test]
	fn resolves_wasm_bindgen_package_version() {
		let suffix = SystemTime::now()
			.duration_since(SystemTime::UNIX_EPOCH)
			.expect("system clock follows Unix epoch")
			.as_nanos();
		let path = std::env::temp_dir().join(format!("slab-lock-{suffix}.toml"));
		fs::write(
			&path,
			"[[package]]\nname = \"other\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \
			 \"wasm-bindgen\"\nversion = \"0.2.100\"\n",
		)
		.expect("write temporary lockfile");
		assert_eq!(bindgen_version(&path).expect("resolve bindgen version"), "0.2.100");
		fs::remove_file(path).expect("remove temporary lockfile");
	}
}
