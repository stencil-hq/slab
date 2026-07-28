//! `slab gen wc FILE -o DIR [--tag NAME] [--separate-ir]` — emit a web
//! component module, declarations, the text web client, and its single Rust
//! kernel WASM sidecar for a compiled `.slab` document. Thin front end over
//! [`slab_compile::wc::generate`]; this module keeps only argument parsing and
//! filesystem writes.

use std::{
	path::{Path, PathBuf},
	process::ExitCode,
};

use slab_compile::{
	Options,
	wc::{WcFile, WcOptions, generate},
};

const GEN_USAGE: &str = "usage: slab gen wc FILE -o DIR [--tag NAME] [--separate-ir]";

fn usage_err(msg: &str) -> ExitCode {
	eprintln!("error: {msg}");
	eprintln!("{GEN_USAGE}");
	ExitCode::from(2)
}

fn write_output(out: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
	let path = out.join(name);
	let parent = path
		.parent()
		.ok_or_else(|| format!("cannot determine parent directory for {}", path.display()))?;
	std::fs::create_dir_all(parent).map_err(|error| {
		format!("cannot create parent directory {} for {}: {error}", parent.display(), path.display())
	})?;
	std::fs::write(&path, bytes).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

pub fn cmd_gen_wc(args: &[String]) -> ExitCode {
	if args == ["--help"] || args == ["-h"] {
		println!("{GEN_USAGE}");
		return ExitCode::SUCCESS;
	}
	let mut file: Option<PathBuf> = None;
	let mut out: Option<PathBuf> = None;
	let mut tag: Option<String> = None;
	let mut separate_ir = false;
	let mut it = args.iter();
	while let Some(a) = it.next() {
		match a.as_str() {
			"-o" | "--out" => match it.next() {
				Some(v) => out = Some(PathBuf::from(v)),
				None => return usage_err("missing value for -o"),
			},
			"--tag" => match it.next() {
				Some(v) => tag = Some(v.clone()),
				None => return usage_err("missing value for --tag"),
			},
			"--separate-ir" => separate_ir = true,
			other if other.starts_with('-') => {
				return usage_err(&format!("unknown flag {other}"));
			},
			_ if file.is_none() => file = Some(PathBuf::from(a)),
			other => return usage_err(&format!("unexpected argument '{other}'")),
		}
	}
	let (Some(file), Some(out)) = (file, out) else {
		return usage_err("gen wc needs FILE and -o DIR");
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
		base_dir:     file.parent().unwrap_or(Path::new(".")).to_path_buf(),
		assets:       None,
		sources:      None,
		fonts:        std::collections::HashMap::new(),
	};
	let name = file.display().to_string();
	let stem = file
		.file_stem()
		.map_or_else(|| "slab".into(), |s| s.to_string_lossy().into_owned());
	let wopts = WcOptions { tag, separate_ir };

	let (files, diags) = generate(&src, &copts, &wopts, &stem);
	for d in &diags.0 {
		eprintln!("{}", d.format(&name));
	}
	let Some(files) = files else {
		return ExitCode::FAILURE;
	};

	let mut n_elems = 0u32;
	for WcFile { name, bytes, text: _ } in &files {
		if let Err(error) = write_output(&out, name, bytes) {
			eprintln!("error: {error}");
			return ExitCode::from(2);
		}
		// The module defines one custom element per DocSpec (main + exports).
		if name.as_str() == format!("{stem}.js") {
			n_elems = bytes
				.windows(21)
				.filter(|w| w == b"customElements.define")
				.count() as u32;
		}
	}
	eprintln!(
		"wrote {} + .d.ts + slab-runtime.js + wasm/slab_kernel_bg.wasm (single Rust/WASM runtime; \
		 {n_elems} element{})",
		out.join(format!("{stem}.js")).display(),
		if n_elems == 1 { "" } else { "s" }
	);
	ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
	use std::time::{SystemTime, UNIX_EPOCH};

	use super::write_output;

	#[test]
	fn writer_creates_nested_output_from_an_empty_destination() {
		let nonce = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.expect("system clock should follow the Unix epoch")
			.as_nanos();
		let out = std::env::temp_dir().join(format!("slab-gen-wc-{}-{nonce}", std::process::id()));
		let bytes = [0, 97, 115, 109, 1, 0, 0, 0];

		write_output(&out, "wasm/slab_kernel_bg.wasm", &bytes)
			.expect("nested sidecar write should succeed");
		assert_eq!(
			std::fs::read(out.join("wasm/slab_kernel_bg.wasm"))
				.expect("written sidecar should be readable")
				.as_slice(),
			bytes.as_slice()
		);

		std::fs::remove_dir_all(out).expect("temporary output should be removable");
	}
}
