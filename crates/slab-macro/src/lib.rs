//! `slab-macro` — compile `.slab` documents into typed Rust modules at build time.
//!
//! The [`include_doc!`] macro runs the slab compiler at macro expansion time and
//! splices the generated typed module (a `Doc` wrapper over
//! `slab_kernel::frame::Instance`, `PARAM_*` consts, and a `Signal` enum) into the
//! invoking crate. Compile failures in the document surface as real slab
//! diagnostics at the macro callsite. The invoking crate must depend on
//! `slab-kernel` and `slab-slir`.
//!
//! Trailing `"Family" = "font.ttf"` pairs compile the document's FONT tables
//! from those byte-backed faces, so glyph ids and advances agree with the faces
//! the host registers at render time. Families without a pair use slab's
//! bundled faces.
//!
//! ```ignore
//! slab_macro::include_doc!(settings, "docs/settings.slab");
//! slab_macro::include_doc!(
//!     app,
//!     "ui/app.slab",
//!     "Instrument Sans" = "assets/fonts/InstrumentSans.ttf",
//! );
//!
//! let mut doc = settings::Doc::new();
//! doc.set_env(800.0, 600.0, false, false);
//! doc.set_title("Hello");
//! let frame = doc.frame(0.0);
//! ```

use proc_macro::{TokenStream, TokenTree};
use std::path::Path;

/// Compile a `.slab` file (relative to `CARGO_MANIFEST_DIR`) into a typed module:
/// `include_doc!("path.slab")` names the module after the file stem;
/// `include_doc!(name, "path.slab")` names it explicitly. Trailing
/// `"Family" = "font.ttf"` pairs (paths relative to `CARGO_MANIFEST_DIR`)
/// compile that family's FONT tables from the given face.
#[proc_macro]
pub fn include_doc(input: TokenStream) -> TokenStream {
    let (name, path, fonts) = match parse_args(input) {
        Ok(v) => v,
        Err(msg) => return compile_error(&msg),
    };
    match expand(&path, name.as_deref(), &fonts) {
        Ok(src) => src.parse().unwrap_or_else(|e| {
            compile_error(&format!(
                "slab-macro internal error: generated module for `{path}` failed to parse: {e}"
            ))
        }),
        Err(msg) => compile_error(&msg),
    }
}

/// Build a `compile_error!(..)` invocation carrying `msg`.
fn compile_error(msg: &str) -> TokenStream {
    format!("compile_error!({msg:?});")
        .parse()
        .expect("compile_error! literal always parses")
}

type ParsedArgs = (Option<String>, String, Vec<(String, String)>);

/// Parse `([name,] "path" [, "Family" = "font.ttf"]... [,])` macro arguments.
fn parse_args(input: TokenStream) -> Result<ParsedArgs, String> {
    const USAGE: &str =
        "expected `include_doc!([name,] \"relative/path.slab\" [, \"Family\" = \"font.ttf\"]...)`";
    let mut tokens = input.into_iter().peekable();
    let name = match tokens.peek() {
        Some(TokenTree::Ident(ident)) => {
            let name = ident.to_string();
            tokens.next();
            match tokens.next() {
                Some(TokenTree::Punct(p)) if p.as_char() == ',' => {}
                _ => return Err(USAGE.to_string()),
            }
            Some(name)
        }
        _ => None,
    };
    let path = match tokens.next() {
        Some(TokenTree::Literal(lit)) => string_literal(&lit)?,
        _ => return Err(USAGE.to_string()),
    };
    let mut fonts = Vec::new();
    loop {
        match tokens.next() {
            None => break,
            Some(TokenTree::Punct(p)) if p.as_char() == ',' => {}
            _ => return Err(USAGE.to_string()),
        }
        let family = match tokens.next() {
            None => break, // trailing comma
            Some(TokenTree::Literal(lit)) => string_literal(&lit)?,
            _ => return Err(USAGE.to_string()),
        };
        match tokens.next() {
            Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
            _ => return Err(USAGE.to_string()),
        }
        let font_path = match tokens.next() {
            Some(TokenTree::Literal(lit)) => string_literal(&lit)?,
            _ => return Err(USAGE.to_string()),
        };
        fonts.push((family, font_path));
    }
    Ok((name, path, fonts))
}

/// Extract the contents of a plain `"..."` string literal token.
fn string_literal(lit: &proc_macro::Literal) -> Result<String, String> {
    let repr = lit.to_string();
    let inner = repr
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| format!("expected a string literal path, found `{repr}`"))?;
    unescape(inner)
}

/// Decode the escape sequences a path literal can legally contain.
fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('\'') => out.push('\''),
            other => {
                return Err(format!(
                    "unsupported escape `\\{}` in path literal",
                    other.map(String::from).unwrap_or_default()
                ));
            }
        }
    }
    Ok(out)
}

/// snake_case module identifier from a file stem (`10-settings` -> `_10_settings`).
fn module_name(stem: &str) -> String {
    let mut out = String::with_capacity(stem.len() + 1);
    for c in stem.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// Compile `path` (relative to `CARGO_MANIFEST_DIR`) with `fonts` overriding the
/// bundled faces, and assemble the wrapping `pub mod` source, or a diagnostic
/// message on failure.
fn expand(path: &str, name: Option<&str>, fonts: &[(String, String)]) -> Result<String, String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|e| format!("include_doc!: CARGO_MANIFEST_DIR is unavailable: {e}"))?;
    let abs = Path::new(&manifest).join(path);
    let src = std::fs::read_to_string(&abs)
        .map_err(|e| format!("include_doc!: cannot read `{}`: {e}", abs.display()))?;
    let name =
        match name {
            Some(n) => n.to_string(),
            None => module_name(abs.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
                format!("include_doc!: `{}` has no usable file stem", abs.display())
            })?),
        };
    // Track every input file so rustc recompiles the caller when one changes.
    let mut tracked = vec![abs.display().to_string()];
    let mut font_bytes = std::collections::HashMap::new();
    for (family, font_path) in fonts {
        let font_abs = Path::new(&manifest).join(font_path);
        let bytes = std::fs::read(&font_abs).map_err(|e| {
            format!(
                "include_doc!: cannot read font `{}` for family `{family}`: {e}",
                font_abs.display()
            )
        })?;
        tracked.push(font_abs.display().to_string());
        font_bytes.insert(family.clone(), bytes);
    }
    let opts = slab_compile::Options {
        embed_assets: true,
        base_dir: abs.parent().map(Path::to_path_buf).unwrap_or_default(),
        assets: None,
        sources: None,
        fonts: font_bytes,
    };
    let (module, diags, imports) =
        slab_compile::rustgen::generate_with_import_paths(&src, &opts, path);
    tracked.extend(imports.iter().map(|import| import.display().to_string()));
    let module = match module {
        Some(m) if !diags.has_errors() => {
            // Surface lint-level diagnostics on stderr; stable proc macros have
            // no warning channel, and silence would hide document lints.
            for diagnostic in &diags.0 {
                eprintln!("warning: {}", diagnostic.format(path));
            }
            m
        }
        _ => {
            let lines: Vec<String> = diags.0.iter().map(|d| d.format(path)).collect();
            return Err(if lines.is_empty() {
                format!("include_doc!: `{path}` failed to compile")
            } else {
                lines.join("\n")
            });
        }
    };
    let tracked: String = tracked
        .iter()
        .map(|p| format!("const _: &[u8] = include_bytes!({p:?});\n"))
        .collect();
    // Generated code is not the caller's to lint; shield it from strict
    // workspace lint levels (clippy::all alone leaves pedantic active).
    Ok(format!(
        "pub mod {name} {{\n#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]\n{module}\n{tracked}}}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::{expand, module_name};

    #[test]
    fn missing_file_reports_path() {
        let err = expand("tests/fixtures/missing.slab", None, &[]).unwrap_err();
        assert!(err.contains("missing.slab"), "{err}");
        assert!(err.contains("cannot read"), "{err}");
    }

    #[test]
    fn broken_document_reports_slab_diagnostics() {
        let err = expand("tests/fixtures/broken.slab", None, &[]).unwrap_err();
        assert!(err.contains("error["), "{err}");
        assert!(err.contains("tests/fixtures/broken.slab"), "{err}");
    }

    #[test]
    fn success_wraps_generated_module_and_tracks_document_bytes() {
        let src = expand("tests/fixtures/counter.slab", Some("counter"), &[]).unwrap();
        assert!(src.starts_with("pub mod counter {"), "{src}");
        assert!(src.contains("pub struct Doc"), "{src}");
        assert!(src.contains("include_bytes!"), "{src}");
        assert!(src.contains("tests/fixtures/counter.slab"), "{src}");
    }

    #[test]
    fn success_tracks_every_imported_module() {
        let src = expand("tests/fixtures/modular.slab", Some("modular"), &[]).unwrap();
        assert!(src.contains("tests/fixtures/modular.slab"), "{src}");
        assert!(src.contains("tests/fixtures/module.slab"), "{src}");
    }

    #[test]
    fn derived_module_name_matches_stem() {
        let src = expand("tests/fixtures/counter.slab", None, &[]).unwrap();
        assert!(src.starts_with("pub mod counter {"), "{src}");
    }

    #[test]
    fn custom_font_compiles_and_is_tracked() {
        let fonts = [(
            "Inter".to_owned(),
            "../../assets/fonts/Inter-Regular.ttf".to_owned(),
        )];
        let src = expand("tests/fixtures/counter.slab", Some("counter"), &fonts).unwrap();
        assert!(src.contains("Inter-Regular.ttf"), "{src}");
    }

    #[test]
    fn missing_font_reports_family_and_path() {
        let fonts = [("Inter".to_owned(), "tests/fixtures/missing.ttf".to_owned())];
        let err = expand("tests/fixtures/counter.slab", None, &fonts).unwrap_err();
        assert!(err.contains("missing.ttf"), "{err}");
        assert!(err.contains("`Inter`"), "{err}");
    }

    #[test]
    fn module_name_sanitizes_stems() {
        assert_eq!(module_name("10-settings"), "_10_settings");
        assert_eq!(module_name("Counter Card"), "counter_card");
    }
}
