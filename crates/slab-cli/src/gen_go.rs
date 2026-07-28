//! `slab gen go FILE -o OUT.go [--package NAME]` — emit a typed Go binding for
//! a compiled `.slab` document, targeting the Go client runtime (`clients/go`)
//! (`github.com/stencil-hq/slab/clients/go/slab`). Thin front end over
//! [`slab_compile::gogen::generate`]; this module keeps only arg parsing, the
//! package-name choice, and the filesystem write.

use slab_compile::Options;
use slab_compile::gogen::generate;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const GEN_USAGE: &str = "usage: slab gen go FILE -o OUT.go [--package NAME]";

/// Package name used when the output directory yields no usable identifier.
const FALLBACK_PACKAGE: &str = "slabdoc";

/// Reserved words that cannot name a Go package.
const GO_KEYWORDS: [&str; 25] = [
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

fn usage_err(msg: &str) -> ExitCode {
    eprintln!("error: {msg}");
    eprintln!("{GEN_USAGE}");
    ExitCode::from(2)
}

/// Whether `name` is a legal, non-reserved Go package name.
fn valid_package(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !GO_KEYWORDS.contains(&name)
}

/// Derive the default package name from the output file's directory, keeping
/// only identifier characters and lowercasing them. Falls back to
/// [`FALLBACK_PACKAGE`] when nothing usable remains.
fn default_package(out: &Path) -> String {
    let raw = out
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut name = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            name.extend(c.to_lowercase());
        } else if c == '_' && !name.is_empty() {
            name.push('_');
        }
    }
    if valid_package(&name) {
        name
    } else {
        FALLBACK_PACKAGE.to_string()
    }
}

/// Run `slab gen go`, writing the generated binding and reporting diagnostics.
pub fn cmd_gen_go(args: &[String]) -> ExitCode {
    if args == ["--help"] || args == ["-h"] {
        println!("{GEN_USAGE}");
        return ExitCode::SUCCESS;
    }
    let mut file: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut package: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => match it.next() {
                Some(v) => out = Some(PathBuf::from(v)),
                None => return usage_err("missing value for -o"),
            },
            "--package" => match it.next() {
                Some(v) => package = Some(v.clone()),
                None => return usage_err("missing value for --package"),
            },
            other if other.starts_with('-') => {
                return usage_err(&format!("unknown flag {other}"));
            }
            _ if file.is_none() => file = Some(PathBuf::from(a)),
            other => return usage_err(&format!("unexpected argument '{other}'")),
        }
    }
    let (Some(file), Some(out)) = (file, out) else {
        return usage_err("gen go needs FILE and -o OUT.go");
    };
    let package = match package {
        Some(name) if !valid_package(&name) => {
            return usage_err(&format!("'{name}' is not a valid Go package name"));
        }
        Some(name) => name,
        None => default_package(&out),
    };
    let src = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", file.display());
            return ExitCode::from(2);
        }
    };
    let copts = Options {
        embed_assets: true,
        base_dir: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
        assets: None,
        sources: None,
        fonts: std::collections::HashMap::new(),
    };
    let name = file.display().to_string();
    let (module, diags) = generate(&src, &copts, &name, &package);
    for d in &diags.0 {
        eprintln!("{}", d.format(&name));
    }
    let Some(module) = module else {
        return ExitCode::FAILURE;
    };
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty())
        && let Err(e) = std::fs::create_dir_all(parent) {
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
