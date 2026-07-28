//! `slab gen rust FILE -o OUT.rs` — emit a typed Rust module for a compiled
//! .slab document, targeting the native gpu client (`slab-native`). Thin
//! front end over [`slab_compile::rustgen::generate`]; this module keeps only
//! arg parsing and the filesystem write.

use std::{
	path::{Path, PathBuf},
	process::ExitCode,
};

use slab_compile::{Options, rustgen::generate};

const GEN_USAGE: &str = "usage: slab gen rust FILE -o OUT.rs";

fn usage_err(msg: &str) -> ExitCode {
	eprintln!("error: {msg}");
	eprintln!("{GEN_USAGE}");
	ExitCode::from(2)
}

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
	let (module, diags) = generate(&src, &copts, &name);
	for d in &diags.0 {
		eprintln!("{}", d.format(&name));
	}
	let Some(module) = module else {
		return ExitCode::FAILURE;
	};
	if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty())
		&& let Err(e) = std::fs::create_dir_all(parent)
	{
		eprintln!("error: cannot create {}: {e}", parent.display());
		return ExitCode::FAILURE;
	}
	if let Err(e) = std::fs::write(&out, &module) {
		eprintln!("error: {}: {e}", out.display());
		return ExitCode::FAILURE;
	}
	eprintln!("wrote {} ({} bytes)", out.display(), module.len());
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
		let _ = std::fs::remove_dir_all(&dir);
	}
}
