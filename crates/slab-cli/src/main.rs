#![recursion_limit = "256"]

//! `slab` — reference CLI. P2 subcommands: `check`, `build`, `dump`;
//! P3 adds `conformance`. Later phases add `render` and `gen`.

mod conformance;
mod drive;
mod gen_rust;
mod gen_wc;
mod render;
mod traces;

use slab_syntax::diag::Diagnostics;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
usage: slab <command> [args]

commands:
  check FILE [--width N] [--height N] [--state a,b] [--env portrait,dark]
             [--client gpu]                print diagnostics (exit 1 on errors)
  build FILE -o OUT.slir [--no-embed-assets]   compile to SLIR
  dump FILE.slir                           print the canonical slir-dump text
  fmt FILE... [--check]                    reformat sources in place ('-' filters
                                           stdin to stdout; --check: diff exit 1)
  conformance [--update] [--emit-slir DIR] compile cases, drive slab-kernel,
                                           byte-compare frame.json + cells goldens
  render FILE -o OUT.{svg,png,apng,txt}    static export (see `slab render --help`)
  drive [FILE] [--port N] [--width N] [--height N]   NDJSON automation protocol (SDP)
  lsp                                      LSP server over stdio (editors)
  gen wc FILE -o DIR [--tag NAME] [--separate-ir]   emit a web-component module
  gen rust FILE -o OUT.rs                  emit a typed Rust module (native client)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    match cmd.as_str() {
        "check" => cmd_check(&args[1..]),
        "build" => cmd_build(&args[1..]),
        "dump" => cmd_dump(&args[1..]),
        "fmt" => cmd_fmt(&args[1..]),
        "conformance" => conformance::cmd_conformance(&args[1..]),
        "render" => render::cmd_render(&args[1..]),
        "drive" => drive::cmd_drive(&args[1..]),
        "lsp" => {
            let code = slab_lsp::serve(std::io::stdin().lock(), std::io::stdout().lock());
            ExitCode::from(code.clamp(0, 255) as u8)
        }
        "gen" => match args.get(1).map(String::as_str) {
            Some("wc") => gen_wc::cmd_gen_wc(&args[2..]),
            Some("rust") => gen_rust::cmd_gen_rust(&args[2..]),
            Some(other) => {
                eprintln!("error: unknown gen target '{other}'");
                ExitCode::from(2)
            }
            None => {
                eprintln!("error: gen needs a target (wc)");
                ExitCode::from(2)
            }
        },
        "--help" | "-h" | "help" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("error: unknown command '{other}'");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

struct Parsed {
    file: Option<PathBuf>,
    out: Option<PathBuf>,
    embed_assets: bool,
}

/// Tiny flag parser: known value-flags are consumed, unknown flags rejected.
fn parse_args(args: &[String], value_flags: &[&str]) -> Result<Parsed, String> {
    let mut p = Parsed {
        file: None,
        out: None,
        embed_assets: true,
    };
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => {
                let v = it.next().ok_or("missing value for -o")?;
                p.out = Some(PathBuf::from(v));
            }
            "--no-embed-assets" => p.embed_assets = false,
            flag if flag.starts_with("--") => {
                let name = flag.trim_start_matches("--");
                if value_flags.contains(&name) {
                    // accepted for CLI compatibility; env-dependent evaluation
                    // happens in the kernel, not at compile time
                    it.next()
                        .ok_or_else(|| format!("missing value for {flag}"))?;
                } else {
                    return Err(format!("unknown flag {flag}"));
                }
            }
            _ if p.file.is_none() => p.file = Some(PathBuf::from(a)),
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    Ok(p)
}

fn print_diags(diags: &Diagnostics, file: &str) {
    for d in &diags.0 {
        eprintln!("{}", d.format(file));
    }
}

/// Compiles one source file for CLI front ends and reports all diagnostics.
pub(crate) fn compile_file(
    path: &Path,
    embed_assets: bool,
) -> (Option<slab_slir::Slir>, Diagnostics) {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            let mut d = Diagnostics::new();
            d.error("parse", format!("cannot read {}: {e}", path.display()), 0);
            return (None, d);
        }
    };
    let opts = slab_compile::Options {
        embed_assets,
        base_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        assets: None,
        fonts: std::collections::HashMap::new(),
    };
    slab_compile::compile(&src, &opts)
}

fn cmd_check(args: &[String]) -> ExitCode {
    let p = match parse_args(
        args,
        &["width", "height", "state", "env", "client", "renderer"],
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let Some(file) = p.file else {
        eprintln!("error: check needs a FILE");
        return ExitCode::from(2);
    };
    let (_, diags) = compile_file(&file, false);
    let name = file.display().to_string();
    print_diags(&diags, &name);
    if diags.has_errors() {
        return ExitCode::FAILURE;
    }
    if diags.is_empty() {
        eprintln!("ok");
    } else {
        eprintln!("ok with warnings");
    }
    ExitCode::SUCCESS
}

/// `slab fmt FILE... [--check]` — canonical formatting via `slab_syntax::format`.
/// `-` reads stdin and writes the result to stdout. `--check` writes nothing
/// and exits 1 when any file would change.
fn cmd_fmt(args: &[String]) -> ExitCode {
    let mut check = false;
    let mut files: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            f if f.starts_with("--") => {
                eprintln!("error: unknown flag {f}");
                return ExitCode::from(2);
            }
            f => files.push(f.to_string()),
        }
    }
    if files.is_empty() {
        eprintln!("error: fmt needs FILE(s), or '-' for stdin");
        return ExitCode::from(2);
    }
    let mut dirty = false;
    let mut failed = false;
    for f in &files {
        if f == "-" {
            let mut src = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut src) {
                eprintln!("error: cannot read stdin: {e}");
                return ExitCode::from(2);
            }
            let out = slab_syntax::format(&src);
            dirty |= out != src;
            if !check {
                print!("{out}");
            }
            continue;
        }
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: cannot read {f}: {e}");
                failed = true;
                continue;
            }
        };
        let out = slab_syntax::format(&src);
        if out == src {
            continue;
        }
        dirty = true;
        if check {
            println!("would reformat {f}");
        } else if let Err(e) = std::fs::write(f, &out) {
            eprintln!("error: cannot write {f}: {e}");
            failed = true;
        } else {
            eprintln!("formatted {f}");
        }
    }
    if failed {
        ExitCode::from(2)
    } else if check && dirty {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn cmd_build(args: &[String]) -> ExitCode {
    let p = match parse_args(args, &[]) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let (Some(file), Some(out)) = (p.file, p.out) else {
        eprintln!("error: build needs FILE and -o OUT.slir");
        return ExitCode::from(2);
    };
    let (slir, diags) = compile_file(&file, p.embed_assets);
    let name = file.display().to_string();
    print_diags(&diags, &name);
    let Some(slir) = slir else {
        return ExitCode::FAILURE;
    };
    let bytes = slab_slir::write(&slir);
    if let Err(e) = std::fs::write(&out, &bytes) {
        eprintln!("error: {}: {e}", out.display());
        return ExitCode::from(2);
    }
    eprintln!("wrote {} ({} bytes)", out.display(), bytes.len());
    ExitCode::SUCCESS
}

fn cmd_dump(args: &[String]) -> ExitCode {
    let p = match parse_args(args, &[]) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let Some(file) = p.file else {
        eprintln!("error: dump needs a FILE.slir");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {}: {e}", file.display());
            return ExitCode::from(2);
        }
    };
    match slab_slir::read(&bytes) {
        Ok(slir) => {
            print!("{}", slab_slir::dump(&slir));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
