//! `slab gen rust FILE -o OUT.rs` — emit a typed Rust module for a compiled
//! .slab document, targeting the native gpu client (`slab-native`). Thin
//! front end over [`slab_compile::rustgen::generate`]; this module keeps only
//! arg parsing and the filesystem write.

use std::{
	io::{self, Write as _},
	path::{Path, PathBuf},
	process::{Command, ExitCode, Stdio},
};

use slab_compile::{Options, rustgen::generate};

const GEN_USAGE: &str = "usage: slab gen rust FILE -o OUT.rs";

fn usage_err(msg: &str) -> ExitCode {
	eprintln!("error: {msg}");
	eprintln!("{GEN_USAGE}");
	ExitCode::from(2)
}
fn format_rust(module: &str) -> io::Result<Vec<u8>> {
	let mut child = Command::new("rustfmt")
		.args(["--edition", "2024", "--emit", "stdout"])
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.spawn()?;
	let mut stdin = child
		.stdin
		.take()
		.ok_or_else(|| io::Error::other("rustfmt stdin unavailable"))?;
	stdin.write_all(module.as_bytes())?;
	drop(stdin);
	let output = child.wait_with_output()?;
	if !output.status.success() {
		return Err(io::Error::other(format!("rustfmt exited with {}", output.status)));
	}
	Ok(output.stdout)
}

fn write_generated(path: &Path, bytes: &[u8]) -> io::Result<bool> {
	match std::fs::read(path) {
		Ok(current) if current == bytes => return Ok(false),
		Ok(_) => {},
		Err(error) if error.kind() == io::ErrorKind::NotFound => {},
		Err(error) => return Err(error),
	}
	std::fs::write(path, bytes)?;
	Ok(true)
}

fn write_generated_rust(path: &Path, module: &str) -> io::Result<(bool, usize)> {
	let next = format_rust(module)?;
	let len = next.len();
	Ok((write_generated(path, &next)?, len))
}

/// Generates one typed Rust module from a Slab document.
pub fn cmd_gen_rust(args: &[String]) -> ExitCode {
	if args == ["--help"] || args == ["-h"] {
		println!("{GEN_USAGE}");
		return ExitCode::SUCCESS;
	}
	let mut file: Option<PathBuf> = None;
	let mut out: Option<PathBuf> = None;
	let mut it = args.iter();
	while let Some(a) = it.next() {
		match a.as_str() {
			"-o" | "--out" => match it.next() {
				Some(v) => out = Some(PathBuf::from(v)),
				None => return usage_err("missing value for -o"),
			},
			other if other.starts_with('-') => {
				return usage_err(&format!("unknown flag {other}"));
			},
			_ if file.is_none() => file = Some(PathBuf::from(a)),
			other => return usage_err(&format!("unexpected argument '{other}'")),
		}
	}
	let (Some(file), Some(out)) = (file, out) else {
		return usage_err("gen rust needs FILE and -o OUT.rs");
	};
	let slir_out = out.with_extension("slir");
	let Some(slir_name) = slir_out.file_name().and_then(|name| name.to_str()) else {
		return usage_err("OUT.rs must have a UTF-8 file name");
	};
	let src = match std::fs::read_to_string(&file) {
		Ok(s) => s,
		Err(e) => {
			eprintln!("error: cannot read {}: {e}", file.display());
			return ExitCode::from(2);
		},
	};
	let copts = Options {
		embed_assets: true,
		base_dir:     file
			.parent()
			.unwrap_or_else(|| Path::new("."))
			.to_path_buf(),
		assets:       None,
		sources:      None,
		fonts:        std::collections::HashMap::new(),
	};
	let name = file.display().to_string();
	let (output, diags) = generate(&src, &copts, &name, slir_name);
	for d in &diags.0 {
		eprintln!("{}", d.format(&name));
	}
	let Some(output) = output else {
		return ExitCode::FAILURE;
	};
	if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty())
		&& let Err(e) = std::fs::create_dir_all(parent)
	{
		eprintln!("error: cannot create {}: {e}", parent.display());
		return ExitCode::FAILURE;
	}
	let (rust_changed, rust_bytes) = match write_generated_rust(&out, &output.module) {
		Ok(result) => result,
		Err(e) => {
			eprintln!("error: {}: {e}", out.display());
			return ExitCode::FAILURE;
		},
	};
	let slir_changed = match write_generated(&slir_out, &output.slir) {
		Ok(changed) => changed,
		Err(e) => {
			eprintln!("error: {}: {e}", slir_out.display());
			return ExitCode::FAILURE;
		},
	};
	let status = if rust_changed || slir_changed {
		"wrote"
	} else {
		"up to date"
	};
	eprintln!(
		"{status} {} + {} ({rust_bytes} + {} bytes)",
		out.display(),
		slir_out.display(),
		output.slir.len()
	);
	ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
	use std::process::ExitCode;

	use super::cmd_gen_rust;

	#[test]
	fn gen_rust_creates_missing_output_directories() {
		let dir = std::env::temp_dir().join(format!("slab-gen-rust-mkdir-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let src = dir.join("doc.slab");
		std::fs::write(&src, "col { text \"hi\" }\n").unwrap();
		let out = dir.join("nested/deeper/out.rs");
		let args = [src.display().to_string(), "-o".to_string(), out.display().to_string()];
		assert_eq!(cmd_gen_rust(&args), ExitCode::SUCCESS);
		assert!(out.is_file(), "gen rust must create parent directories");
		let slir = out.with_extension("slir");
		assert!(slir.is_file(), "gen rust must create the SLIR sidecar");
		let permissions = std::fs::metadata(&out).unwrap().permissions();
		let mut readonly = permissions.clone();
		readonly.set_readonly(true);
		std::fs::set_permissions(&out, readonly).unwrap();
		let slir_permissions = std::fs::metadata(&slir).unwrap().permissions();
		let mut slir_readonly = slir_permissions.clone();
		slir_readonly.set_readonly(true);
		std::fs::set_permissions(&slir, slir_readonly).unwrap();
		assert_eq!(cmd_gen_rust(&args), ExitCode::SUCCESS, "unchanged output must not be rewritten");
		std::fs::set_permissions(&out, permissions).unwrap();
		std::fs::set_permissions(&slir, slir_permissions).unwrap();
		let _ = std::fs::remove_dir_all(&dir);
	}
}
