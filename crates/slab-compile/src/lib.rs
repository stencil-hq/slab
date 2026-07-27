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
