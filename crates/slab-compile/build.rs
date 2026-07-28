//! Embeds the git commit hash so every front end (`slab --version`,
//! `slab-native --version`, the WASM `compiler_version` export) can pin a
//! build. Falls back to `unknown` outside a git checkout.

use std::process::Command;

fn main() {
	let hash = Command::new("git")
		.args(["rev-parse", "--short=9", "HEAD"])
		.output()
		.ok()
		.filter(|out| out.status.success())
		.and_then(|out| String::from_utf8(out.stdout).ok())
		.map(|s| s.trim().to_owned())
		.filter(|s| !s.is_empty())
		.unwrap_or_else(|| "unknown".to_owned());
	println!("cargo:rustc-env=SLAB_GIT_HASH={hash}");
	// Re-embed when HEAD moves (best effort; .git may be absent in tarballs).
	println!("cargo:rerun-if-changed=../../.git/HEAD");
}
