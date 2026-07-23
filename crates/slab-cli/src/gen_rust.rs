//! `slab gen rust FILE -o OUT.rs` — emit a typed Rust module for a compiled
//! .slab document, targeting the native gpu client (`slab-native`). Thin
//! front end over [`slab_compile::rustgen::generate`]; this module keeps only
//! arg parsing and the filesystem write.

use slab_compile::Options;
use slab_compile::rustgen::generate;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const GEN_USAGE: &str = "usage: slab gen rust FILE -o OUT.rs";

fn usage_err(msg: &str) -> ExitCode {
    eprintln!("error: {msg}");
    eprintln!("{GEN_USAGE}");
    ExitCode::from(2)
}

pub fn cmd_gen_rust(args: &[String]) -> ExitCode {
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
            }
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
        }
    };
    let copts = Options {
        embed_assets: true,
        base_dir: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
        assets: None,
    };
    let name = file.display().to_string();
    let (module, diags) = generate(&src, &copts, &name);
    for d in &diags.0 {
        eprintln!("{}", d.format(&name));
    }
    let Some(module) = module else {
        return ExitCode::FAILURE;
    };
    if let Err(e) = std::fs::write(&out, &module) {
        eprintln!("error: {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {} ({} bytes)", out.display(), module.len());
    ExitCode::SUCCESS
}
