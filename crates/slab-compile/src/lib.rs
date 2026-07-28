//! `slab-compile` — resolve, expand, and emit `.slab` sources as SLIR.
//! Layout, when/anim evaluation, and everything env-dependent live in the
//! kernel (P3); this crate is the pure compile-time half.

pub mod capsnote;
pub mod color;
pub mod emit;
pub mod expand;
pub mod export;
pub mod fonts;
pub mod input;
pub mod raster;
pub mod react;
pub mod render;
pub mod rustgen;
pub mod svg;
pub mod wc;

use slab_slir::Slir;
use slab_syntax::diag::Diagnostics;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Options {
    /// Embed image bytes (`--no-embed-assets` clears this).
    pub embed_assets: bool,
    /// Directory image `src` paths resolve against (the .slab file's dir).
    pub base_dir: PathBuf,
    /// In-memory image assets keyed by the `src` string as written in the
    /// document. `Some` → the filesystem is never touched (wasm hosts);
    /// `None` → read `base_dir.join(src)` as before.
    pub assets: Option<std::collections::HashMap<String, Vec<u8>>>,
    /// Host-supplied sfnt bytes keyed by family name (matched
    /// case-insensitively). A matching family's FONT tables are built from
    /// these bytes instead of the bundled class fallback, so compiled glyph
    /// ids agree with the face the host registers at render time.
    pub fonts: std::collections::HashMap<String, Vec<u8>>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            embed_assets: true,
            base_dir: PathBuf::from("."),
            assets: None,
            fonts: std::collections::HashMap::new(),
        }
    }
}

/// Compile slab source. Always returns the diagnostics; the SLIR document is
/// `None` when errors make the output meaningless.
pub fn compile(src: &str, opts: &Options) -> (Option<Slir>, Diagnostics) {
    let mut diags = Diagnostics::new();
    let doc = slab_syntax::parse(src, &mut diags);
    let expanded = expand::expand(&doc, &mut diags);
    if diags.has_errors() {
        return (None, diags);
    }
    let slir = emit::emit(&expanded, opts, &mut diags);
    if diags.has_errors() {
        return (None, diags);
    }
    (Some(slir), diags)
}

/// Compile the document and every exported definition as a standalone document.
///
/// Standalone diagnostics include the exported definition name in their message.
pub fn compile_with_exports(src: &str, opts: &Options) -> (Option<Slir>, Diagnostics) {
    let (slir, mut diags) = compile(src, opts);
    if slir.is_none() {
        return (None, diags);
    }
    for def in export::exported_def_names(src) {
        let (exported, mut export_diags, _) = export::compile_export(src, &def, opts);
        for diagnostic in &mut export_diags.0 {
            diagnostic.msg = format!("in export {def}: {}", diagnostic.msg);
        }
        diags.0.extend(export_diags.0);
        if exported.is_none() {
            return (None, diags);
        }
    }
    (slir, diags)
}
