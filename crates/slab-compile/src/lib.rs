//! `slab-compile` — resolve, expand, and emit `.slab` sources as SLIR.
//! Layout, when/anim evaluation, and everything env-dependent live in the
//! kernel (P3); this crate is the pure compile-time half.

pub mod capsnote;
pub mod color;
pub mod emit;
pub mod expand;
pub mod export;
pub mod fonts;
pub mod gogen;
pub mod import;
pub mod input;
pub mod raster;
pub mod react;
pub mod render;
pub mod rustgen;
pub mod svg;
pub mod wc;

use std::path::PathBuf;

use slab_slir::Slir;
use slab_syntax::diag::Diagnostics;

/// Build identification for every front end: the workspace semver plus the
/// git commit hash embedded by `build.rs` (`0.1.0 (72e5ca758)`), or
/// `(unknown)` when built outside a git checkout.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("SLAB_GIT_HASH"), ")");

#[derive(Debug, Clone)]
pub struct Options {
	/// Embed image bytes (`--no-embed-assets` clears this).
	pub embed_assets: bool,
	/// Directory image `src` paths resolve against (the .slab file's dir).
	pub base_dir:     PathBuf,
	/// In-memory image assets keyed by the `src` string as written in the
	/// document. `Some` → the filesystem is never touched (wasm hosts);
	/// `None` → read `base_dir.join(src)` as before.
	pub assets:       Option<std::collections::HashMap<String, Vec<u8>>>,
	/// In-memory Slab sources keyed by normalized `base_dir`-relative paths.
	/// `Some` prevents all filesystem access for imports.
	pub sources:      Option<std::collections::HashMap<String, String>>,
	/// Host-supplied sfnt bytes keyed by family name (matched
	/// case-insensitively). A matching family's FONT tables are built from
	/// these bytes instead of the bundled class fallback, so compiled glyph
	/// ids agree with the face the host registers at render time.
	pub fonts:        std::collections::HashMap<String, Vec<u8>>,
}

impl Default for Options {
	fn default() -> Self {
		Self {
			embed_assets: true,
			base_dir:     PathBuf::from("."),
			assets:       None,
			sources:      None,
			fonts:        std::collections::HashMap::new(),
		}
	}
}

/// Compile slab source. Always returns the diagnostics; the SLIR document is
/// `None` when errors make the output meaningless.
pub fn compile(src: &str, opts: &Options) -> (Option<Slir>, Diagnostics) {
	let mut diags = Diagnostics::new();
	let units = import::closure(src, opts, &mut diags);
	let slir = compile_units(&units, opts, &mut diags);
	(slir, diags)
}

/// Compile an already-loaded source closure without loading it again.
///
/// Hosts that maintain virtual source maps use this after [`import::closure`].
pub fn compile_units(
	units: &[import::Unit],
	opts: &Options,
	diags: &mut Diagnostics,
) -> Option<Slir> {
	let expanded = expand::expand(units, diags);
	if diags.has_errors() {
		return None;
	}
	let slir = emit::emit(&expanded, opts, diags);
	if diags.has_errors() {
		return None;
	}
	Some(slir)
}

/// Compile the document and every exported definition as a standalone document.
///
/// Standalone diagnostics include the exported definition name in their
/// message.
pub fn compile_with_exports(src: &str, opts: &Options) -> (Option<Slir>, Diagnostics) {
	let mut diags = Diagnostics::new();
	let units = import::closure(src, opts, &mut diags);
	let slir = compile_units_with_exports(&units, opts, &mut diags);
	(slir, diags)
}

pub(crate) fn compile_units_with_exports(
	units: &[import::Unit],
	opts: &Options,
	diags: &mut Diagnostics,
) -> Option<Slir> {
	let slir = compile_units(units, opts, diags)?;
	// One authored site reports once: an exported def's body is recompiled
	// per export, so any diagnostic already reported for the same code, file,
	// line, and message by the document compile (or an earlier export) is a
	// duplicate, not new information.
	let mut seen = diags
		.0
		.iter()
		.map(|diagnostic| {
			(diagnostic.code, diagnostic.file.clone(), diagnostic.line, diagnostic.msg.clone())
		})
		.collect::<std::collections::BTreeSet<_>>();
	for def in export::exported_def_names(units) {
		let (exported, export_diags, _) = export::compile_export(units, &def, opts);
		for mut diagnostic in export_diags.0 {
			if !seen.insert((
				diagnostic.code,
				diagnostic.file.clone(),
				diagnostic.line,
				diagnostic.msg.clone(),
			)) {
				continue;
			}
			diagnostic.msg = format!("in export {def}: {}", diagnostic.msg);
			diags.0.push(diagnostic);
		}
		exported.as_ref()?;
	}
	Some(slir)
}

#[cfg(test)]
mod version_tests {
	#[test]
	fn version_embeds_semver_and_a_hash_annotation() {
		assert!(super::VERSION.starts_with(env!("CARGO_PKG_VERSION")));
		// `0.1.0 (<hash>)`: hash is a short git rev or the `unknown` fallback
		let hash = super::VERSION
			.split_once(" (")
			.and_then(|(_, rest)| rest.strip_suffix(')'))
			.expect("VERSION carries a parenthesized build hash");
		assert!(!hash.is_empty());
		assert!(hash == "unknown" || hash.chars().all(|c| c.is_ascii_hexdigit()));
	}
}
