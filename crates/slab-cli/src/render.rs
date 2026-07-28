//! `slab render FILE -o OUT.{svg,png,apng,txt}` — compile, then call the
//! lib-side renderer (`slab_compile::render`), and write / print the result.
//! The output kind comes from the extension; the `when`-client class defaults
//! per kind (svg -> svg, png/apng -> png, txt -> tui) and can be overridden
//! with `--client`.

use std::{
	path::{Path, PathBuf},
	process::ExitCode,
};

use slab_compile::{
	Options, compile,
	render::{RenderKind, RenderOpts, render},
};
use slab_syntax::diag::Diagnostics;

pub const USAGE: &str = "\
usage: slab render FILE [-o OUT.{svg,png,apng,txt}] [--client web|gpu|tui|svg|png]
                   [--width N] [--height N] [--scale N] [--t MS]
                   [--dur S] [--fps N] [--state a,b] [--env portrait,dark,coarse] [--theme NAME]
                   [--set param=value]... [--font NAME=PATH]... [--plain] [-v]

  --scale   png/apng device scale factor (default 1)
  --dur     apng duration in seconds (default 2)
  --fps     apng frames per second (default 20)
  --t       motion clock in ms (renders one instant)
  --env     dark/coarse media flags; portrait picks a 16:9 portrait height
            when --height is not given
  --state   enable document-global preview states (not per-node state)
  --set     override a scalar param, or a list with a JSON array of objects
  --plain   tui: no ANSI colors (the conformance golden format)
  --font    register a font face from PATH under NAME (repeatable)
  -v        print every capability note (default: one summary line)
  (txt output with no -o writes to stdout)
";

struct Args {
	file:    PathBuf,
	out:     Option<PathBuf>,
	client:  Option<String>,
	theme:   Option<String>,
	width:   f64,
	height:  f64,
	scale:   f64,
	t:       f64,
	dur:     f64,
	fps:     f64,
	states:  Vec<String>,
	env:     Vec<String>,
	sets:    Vec<(String, String)>,
	fonts:   Vec<(String, PathBuf)>,
	plain:   bool,
	verbose: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
	let mut a = Args {
		file:    PathBuf::new(),
		out:     None,
		client:  None,
		width:   800.0,
		theme:   None,
		height:  0.0,
		scale:   1.0,
		t:       0.0,
		dur:     2.0,
		fps:     20.0,
		states:  Vec::new(),
		env:     Vec::new(),
		sets:    Vec::new(),
		fonts:   Vec::new(),
		plain:   false,
		verbose: false,
	};
	let mut file = None;
	let mut it = args.iter();
	while let Some(arg) = it.next() {
		let mut val = |name: &str| {
			it.next()
				.cloned()
				.ok_or_else(|| format!("missing value for {name}"))
		};
		match arg.as_str() {
			"-o" | "--out" => a.out = Some(PathBuf::from(val("-o")?)),
			"--client" => a.client = Some(val("--client")?),
			"--theme" => a.theme = Some(val("--theme")?),
			"--width" => a.width = val("--width")?.parse().map_err(|_| "bad --width")?,
			"--height" => a.height = val("--height")?.parse().map_err(|_| "bad --height")?,
			"--scale" => a.scale = val("--scale")?.parse().map_err(|_| "bad --scale")?,
			"--t" => a.t = val("--t")?.parse().map_err(|_| "bad --t")?,
			"--dur" => a.dur = val("--dur")?.parse().map_err(|_| "bad --dur")?,
			"--fps" => a.fps = val("--fps")?.parse().map_err(|_| "bad --fps")?,
			"--state" => a
				.states
				.extend(val("--state")?.split(',').map(str::to_string)),
			"--env" => a.env.extend(val("--env")?.split(',').map(str::to_string)),
			"--set" => {
				let v = val("--set")?;
				let (k, v) = v.split_once('=').ok_or("--set needs param=value")?;
				a.sets.push((k.to_string(), v.to_string()));
			},
			"--font" => {
				let spec = val("--font")?;
				let (name, path) = spec.split_once('=').ok_or("--font needs NAME=PATH")?;
				if name.is_empty() || path.is_empty() {
					return Err("--font needs NAME=PATH".into());
				}
				a.fonts.push((name.to_string(), PathBuf::from(path)));
			},
			"--plain" => a.plain = true,
			"-v" | "--verbose" => a.verbose = true,
			other if other.starts_with('-') => return Err(format!("unknown flag {other}")),
			other if file.is_none() => file = Some(PathBuf::from(other)),
			other => return Err(format!("unexpected argument '{other}'")),
		}
	}
	a.file = file.ok_or("render needs a FILE")?;
	Ok(a)
}

/// Loads runtime font files for CLI render and drive sessions.
pub(crate) fn load_registered_fonts(
	fonts: &[(String, PathBuf)],
) -> Result<Vec<slab_compile::render::RegisteredFont>, String> {
	fonts
		.iter()
		.map(|(name, path)| {
			let bytes =
				std::fs::read(path).map_err(|e| format!("cannot read font {}: {e}", path.display()))?;
			let metrics = slab_fonts::parse_metrics(&bytes)
				.ok_or_else(|| format!("cannot parse font {}", path.display()))?;
			Ok(slab_compile::render::RegisteredFont { name: name.clone(), bytes, metrics })
		})
		.collect()
}

fn kind_of(out: Option<&Path>, client: Option<&str>) -> Result<RenderKind, String> {
	if let Some(out) = out {
		return match out.extension().and_then(|e| e.to_str()).unwrap_or("") {
			"svg" => Ok(RenderKind::Svg),
			"png" => Ok(RenderKind::Png),
			"apng" => Ok(RenderKind::Apng),
			"txt" | "ansi" => Ok(RenderKind::Tui),
			other => Err(format!("cannot infer output kind from extension '.{other}'")),
		};
	}
	match client {
		Some("tui") => Ok(RenderKind::Tui),
		None | Some(_) => Err("render needs -o OUT (or --client tui for stdout)".into()),
	}
}

fn write_out(out: &Path, bytes: &[u8], note: &str) -> Result<(), String> {
	std::fs::write(out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
	eprintln!("wrote {} ({} bytes){note}", out.display(), bytes.len());
	Ok(())
}

/// Prints renderer capability notes: everything under `-v`, otherwise one
/// summary line naming the distinct note codes (N18: seven stderr lines per
/// static export is noise, but the information must stay discoverable).
fn print_notes(notes: &[String], verbose: bool) {
	if verbose || notes.len() <= 1 {
		for n in notes {
			eprintln!("{n}");
		}
		return;
	}
	eprintln!("{}", note_summary(notes));
}

/// One-line digest of a multi-note render: count plus distinct note codes.
fn note_summary(notes: &[String]) -> String {
	let mut codes: Vec<&str> = Vec::new();
	for n in notes {
		// note lines read `note <code>: …` (capsnote + tui grid diagnostics)
		let code = n
			.strip_prefix("note ")
			.and_then(|rest| rest.split(':').next())
			.unwrap_or(n.as_str());
		if !codes.contains(&code) {
			codes.push(code);
		}
	}
	format!(
		"note: {} capability notes ({}); rerun with -v for details",
		notes.len(),
		codes.join(", ")
	)
}

fn print_diags(diags: &Diagnostics, file: &str) {
	for d in &diags.0 {
		eprintln!("{}", d.format(file));
	}
}

pub fn cmd_render(args: &[String]) -> ExitCode {
	if args == ["--help"] || args == ["-h"] {
		print!("{USAGE}");
		return ExitCode::SUCCESS;
	}
	let a = match parse(args) {
		Ok(a) => a,
		Err(e) => {
			eprintln!("error: {e}");
			eprint!("{USAGE}");
			return ExitCode::from(2);
		},
	};
	match run(&a) {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		},
	}
}

fn run(a: &Args) -> Result<(), String> {
	let kind = kind_of(a.out.as_deref(), a.client.as_deref())?;
	let src = std::fs::read_to_string(&a.file).map_err(|e| format!("{}: {e}", a.file.display()))?;
	let base_dir = a.file.parent().unwrap_or(Path::new(".")).to_path_buf();
	let opts = Options {
		embed_assets: true,
		base_dir:     base_dir.clone(),
		assets:       None,
		sources:      None,
		fonts:        std::collections::HashMap::new(),
	};
	let (slir, diags) = compile(&src, &opts);
	print_diags(&diags, &a.file.display().to_string());
	let slir = slir.ok_or("compile failed")?;
	let registered_fonts = load_registered_fonts(&a.fonts)?;

	let ropts = RenderOpts {
		kind,
		client: a.client.clone(),
		theme: a.theme.clone(),
		width: a.width,
		height: a.height,
		scale: a.scale,
		t: a.t,
		dur: a.dur,
		fps: a.fps,
		states: a.states.clone(),
		env: a.env.clone(),
		sets: a.sets.clone(),
		plain: a.plain,
		registered_fonts,
	};
	let out = render(&slir, &ropts, &base_dir)?;
	print_notes(&out.notes, a.verbose);
	match &a.out {
		Some(path) => write_out(path, &out.bytes, &out.summary)?,
		None => {
			// TUI-to-stdout: `out.text` is true; print as UTF-8.
			let text = std::str::from_utf8(&out.bytes).map_err(|_| "non-utf8 tui output")?;
			print!("{text}");
		},
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::process::ExitCode;

	use super::{cmd_render, note_summary};

	#[test]
	fn render_help_is_a_successful_command() {
		assert_eq!(cmd_render(&["--help".to_string()]), ExitCode::SUCCESS);
		assert_eq!(cmd_render(&["-h".to_string()]), ExitCode::SUCCESS);
	}

	#[test]
	fn note_summary_counts_and_dedupes_codes() {
		let notes: Vec<String> = [
			"note cap-anim: 'transition' is not supported by the svg renderer",
			"note cap-anim: 'text-keyframes' is degraded by the svg renderer",
			"note cap-hover: 'hover' is not supported by the svg renderer",
		]
		.iter()
		.map(|s| s.to_string())
		.collect();
		assert_eq!(
			note_summary(&notes),
			"note: 3 capability notes (cap-anim, cap-hover); rerun with -v for details"
		);
	}

	#[test]
	fn note_summary_keeps_unprefixed_lines_whole() {
		let notes = vec!["grid oddity".to_string(), "grid oddity".to_string()];
		assert_eq!(
			note_summary(&notes),
			"note: 2 capability notes (grid oddity); rerun with -v for details"
		);
	}
}
