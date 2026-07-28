//! `--app player` integration tests: the playlist app on top of
//! examples/00-player.slab. Signals mutate params through the kernel
//! (toggle starts the real-time ticker, next/prev rotate the playlist),
//! the new script verbs (TICK:ms, MOUSE:x,y, CLICK:x,y) prove it
//! headless, and the hover ease is asserted at the kernel frame level
//! (140ms ease-out transition on the SHUF button bg).

use std::{
	path::{Path, PathBuf},
	process::Command,
};

use slab_kernel::{
	flatten::{Frame, FrameOp},
	frame as kframe,
};

const PLAYER: &str = "examples/00-player.slab";

fn root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run `slab-tui examples/00-player.slab --width 360 --app player ARGS
/// --dump-after -` and return the dump text.
fn run_app(args: &[&str]) -> String {
	let out = Command::new(env!("CARGO_BIN_EXE_slab-tui"))
		.arg(root().join(PLAYER))
		.args(["--width", "360", "--app", "player"])
		.args(args)
		.args(["--dump-after", "-"])
		.output()
		.expect("spawn slab-tui");
	assert!(out.status.success(), "slab-tui failed: {}", String::from_utf8_lossy(&out.stderr));
	String::from_utf8(out.stdout).expect("utf8 dump")
}

/// Compile the player and return a live kernel instance at 360u wide.
fn player_instance() -> kframe::Instance {
	let file = root().join(PLAYER);
	let src = std::fs::read_to_string(&file).expect("read player");
	let opts = slab_compile::Options {
		embed_assets: true,
		base_dir: file.parent().unwrap().to_path_buf(),
		..slab_compile::Options::default()
	};
	let (slir, diags) = slab_compile::compile(&src, &opts);
	assert!(!diags.has_errors(), "player must compile clean");
	let bytes = slab_slir::write(&slir.expect("slir"));
	let (mut inst, _) = slab_slir::instance(&bytes).expect("host decode");
	kframe::inst_set_env(&mut inst, 360.0, 0.0, 2, false, false);
	inst
}

/// Center of a Text op by its string (x + `measured_w/2`, baseline - size/2)
/// — inside the button that wraps it, so it is both a pointer target and
/// a cell coordinate anchor.
fn text_center(fr: &Frame, s: &str) -> (f64, f64) {
	for op in &fr.ops {
		if let FrameOp::Text(t) = op
			&& fr.strings[t.str_ref as usize] == s
		{
			return (t.x + t.measured_w / 2.0, t.y_baseline - t.size / 2.0);
		}
	}
	panic!("text {s:?} not found in frame");
}

/// The play circle is the only 44x44 fully-round rect; return its center.
fn play_center(fr: &Frame) -> (f64, f64) {
	for op in &fr.ops {
		if let FrameOp::Rect(r) = op
			&& r.w == 44.0
			&& r.h == 44.0
			&& r.radius >= 22.0
		{
			return (r.x + 22.0, r.y + 22.0);
		}
	}
	panic!("play circle (44x44 round) not found in frame ops");
}

/// The bg paint word (0xRRGGBBAA) of the innermost Rect op enclosing
/// point (x,y) that carries a solid bg; 0 when nothing painted there.
fn bg_at(fr: &Frame, x: f64, y: f64) -> u32 {
	let mut bg = 0u32;
	for op in &fr.ops {
		if let FrameOp::Rect(r) = op
			&& r.bg_kind == 1
			&& x >= r.x
			&& x <= r.x + r.w
			&& y >= r.y
			&& y <= r.y + r.h
		{
			bg = r.bg;
		}
	}
	bg
}

/// Parse an ANSI dump (`--ansi`) into per-cell bg colors: bg[row][col] =
/// 0xRRGGBB or `NO_COLOR` for the terminal default. Only truecolor SGR
/// (38;2 / 48;2) appears in `cells_to_text` output.
const NO_COLOR: u32 = 0xff00_0000;
fn ansi_bg_grid(dump: &str) -> Vec<Vec<u32>> {
	let mut grid = Vec::new();
	for line in dump.lines() {
		if line.starts_with("signals:") {
			break;
		}
		let mut row = Vec::new();
		let mut bg = NO_COLOR;
		let mut chars = line.chars().peekable();
		while let Some(c) = chars.next() {
			if c == '\u{1b}' {
				assert_eq!(chars.next(), Some('['), "CSI expected");
				let mut params = String::new();
				for c in chars.by_ref() {
					if c == 'm' {
						break;
					}
					params.push(c);
				}
				let p: Vec<u32> = params.split(';').map(|s| s.parse().unwrap()).collect();
				let mut k = 0;
				while k < p.len() {
					match p[k] {
						0 => bg = NO_COLOR,
						48 => {
							assert_eq!(p[k + 1], 2, "truecolor bg expected");
							bg = (p[k + 2] << 16) | (p[k + 3] << 8) | p[k + 4];
							k += 4;
						},
						38 => k += 4, // fg: skip 2;r;g;b
						_ => {},
					}
					k += 1;
				}
			} else {
				row.push(bg);
			}
		}
		grid.push(row);
	}
	grid
}

fn bg_cell(grid: &[Vec<u32>], col: usize, row: usize) -> u32 {
	grid
		.get(row)
		.and_then(|r| r.get(col).copied())
		.unwrap_or(NO_COLOR)
}

/// (a) Arrow keys walk the focus ring to the play button; ENTER emits
/// `toggle`, which flips the app into playing; TICK:2000 advances the
/// real clock 2s — the dumped grid's elapsed/remain text moved on from
/// the initial 2:37/-1:35 while the title row stayed on track 1. The
/// whole loop runs through kernel param re-solves (host only formats).
#[test]
fn enter_toggles_play_and_tick_advances_time() {
	let dump = run_app(&["--script", "RIGHT RIGHT RIGHT ENTER TICK:2000"]);
	assert_eq!(dump.lines().last().unwrap(), "signals: toggle");
	assert!(dump.contains("Pale Green Things"), "title row must stay on track 1:\n{dump}");
	assert!(
		dump.contains("2:39") && dump.contains("-1:33"),
		"elapsed/remain must advance 2s from 2:37/-1:35:\n{dump}"
	);
	assert!(!dump.contains("2:37"), "stale elapsed still shown:\n{dump}");

	// Paused (no ENTER): the same TICK does not move the clock.
	let paused = run_app(&["--script", "TICK:2000"]);
	assert!(
		paused.contains("2:37") && paused.contains("-1:35"),
		"paused clock must not advance:\n{paused}"
	);
}

/// (a2) Track end auto-advances: 2:37 into the 4:12 track 1, 96s of play
/// crosses the end and rolls into track 2 at 0:01.
#[test]
fn tick_past_track_end_auto_advances() {
	let dump = run_app(&["--script", "RIGHT RIGHT RIGHT ENTER TICK:96000"]);
	assert!(
		dump.contains("This Year") && dump.contains("0:01"),
		"track end must auto-advance into This Year @0:01:\n{dump}"
	);
}

/// (b) One more RIGHT reaches the next button: ENTER emits `next` and the
/// grid title flips to track 2 with reset times.
#[test]
fn next_rotates_playlist_title() {
	let dump = run_app(&["--script", "RIGHT RIGHT RIGHT RIGHT ENTER"]);
	assert_eq!(dump.lines().last().unwrap(), "signals: next");
	assert!(
		dump.contains("This Year") && !dump.contains("Pale Green Things"),
		"next must show track 2's title:\n{dump}"
	);
	assert!(
		dump.contains("0:00") && dump.contains("-4:05"),
		"next must reset elapsed/remain for the 4:05 track:\n{dump}"
	);

	// prev from track 1 wraps back to track 4.
	let dump = run_app(&["--script", "RIGHT RIGHT ENTER"]);
	assert_eq!(dump.lines().last().unwrap(), "signals: prev");
	assert!(
		dump.contains("Dance Music") && dump.contains("-1:59"),
		"prev must wrap to track 4:\n{dump}"
	);
}

/// (c) Mouse hover shows the moss highlight: MOUSE over the SHUF button
/// (center computed from kernel frame geometry, not hardcoded), TICK:500
/// to finish the 140ms ease — the SHUF cell bg in the ANSI dump is the
/// composited moss #1B2E22, and differs from the no-hover dump.
#[test]
fn mouse_hover_changes_shuf_cell_bg() {
	let mut inst = player_instance();
	let fr = kframe::inst_frame(&mut inst, 0.0);
	let (x, y) = text_center(&fr, "SHUF");
	let (col, row) = ((x / 8.0) as usize, (y / 16.0) as usize);

	let mouse = format!("MOUSE:{x},{y} TICK:500");
	let hovered = ansi_bg_grid(&run_app(&["--ansi", "--script", &mouse]));
	let plain = ansi_bg_grid(&run_app(&["--ansi", "--script", "TICK:500"]));
	let (hb, pb) = (bg_cell(&hovered, col, row), bg_cell(&plain, col, row));
	assert_ne!(hb, pb, "hover must recolor the SHUF cell bg at ({col},{row})");
	assert_eq!(hb, 0x1b2e22, "settled hover bg must be moss #1B2E22, got #{hb:06X}");
}

/// (d) Hover EASE at the kernel level, per the shared proof contract:
/// dispatch a pointer-move over SHUF, then sample `inst_frame` at t0,
/// t0+70, t0+500. A base-less color transition fades through the target
/// at alpha 0 (`slab_kernel::motion` CSS-transparent semantics), so the SHUF
/// rect bg paint (0xAABBGGRR) runs #00222E1B → #BF222E1B → #FF222E1B: three
/// DIFFERENT words, moss RGB throughout, alpha strictly between the
/// endpoints at the 140ms ease-out's midpoint sample.
#[test]
fn hover_ease_interpolates_shuf_bg_paint() {
	const MOSS_RGB: u32 = 0x0022_2e1b; // #1B2E22 as 0xAABBGGRR, alpha masked

	let mut inst = player_instance();
	let fr = kframe::inst_frame(&mut inst, 0.0);
	let (x, y) = text_center(&fr, "SHUF");

	let ev = slab_kernel::dispatch::Event {
		etype: 0, // pointer-move: hover flip at t0 = 0
		x,
		y,
		dx: 0.0,
		dy: 0.0,
		button: 0,
		clicks: 0,
		key: String::new(),
		text: String::new(),
		mods: 0,
	};
	kframe::inst_dispatch(&mut inst, &ev);

	let v0 = bg_at(&kframe::inst_frame(&mut inst, 0.0), x, y);
	let v1 = bg_at(&kframe::inst_frame(&mut inst, 70.0), x, y);
	let v2 = bg_at(&kframe::inst_frame(&mut inst, 500.0), x, y);
	assert!(
		v0 != v1 && v1 != v2 && v0 != v2,
		"bg paint must ease through three distinct values: t0=#{v0:08X} t0+70=#{v1:08X} \
		 t0+500=#{v2:08X}"
	);
	for (name, v) in [("t0", v0), ("t0+70", v1), ("t0+500", v2)] {
		assert_eq!(
			v & 0x00ff_ffff,
			MOSS_RGB,
			"{name}: fade must hold moss RGB #1B2E22, got #{v:08X}"
		);
	}
	let (a0, a1, a2) = (v0 >> 24, v1 >> 24, v2 >> 24);
	assert_eq!(a0, 0x00, "t0 must start transparent, got #{v0:08X}");
	assert_eq!(a2, 0xff, "t0+500 must settle opaque, got #{v2:08X}");
	assert!(
		a0 < a1 && a1 < a2,
		"midpoint alpha {a1:#04X} must sit strictly between {a0:#04X} and {a2:#04X} (t0+70 of a \
		 140ms ease-out)"
	);
}

/// (e) Mouse click on the play circle (center from frame geometry) emits
/// `toggle` — down+up through `inst_dispatch`, headless.
#[test]
fn click_play_circle_emits_toggle() {
	let mut inst = player_instance();
	let fr = kframe::inst_frame(&mut inst, 0.0);
	let (x, y) = play_center(&fr);
	let dump = run_app(&["--script", &format!("CLICK:{x},{y} TICK:1000")]);
	assert_eq!(dump.lines().last().unwrap(), "signals: toggle");
	assert!(dump.contains("2:38"), "click-started playback must advance the clock:\n{dump}");
}

/// Shuffle/loop badges: the signals land (headless proof that the
/// buttons are wired); badge rendering itself is the --debug footer.
#[test]
fn shuffle_and_loop_signals_fire() {
	let dump = run_app(&["--script", "RIGHT ENTER"]);
	assert_eq!(dump.lines().last().unwrap(), "signals: shuffle");
	let dump = run_app(&["--script", "LEFT ENTER"]);
	assert_eq!(dump.lines().last().unwrap(), "signals: loop");
}

/// --app rejects unknown app names; generic FILE mode stays untouched
/// (no --app: TICK:2000 must NOT advance the declared elapsed text).
#[test]
fn app_flag_is_opt_in() {
	let out = Command::new(env!("CARGO_BIN_EXE_slab-tui"))
		.arg(root().join(PLAYER))
		.args(["--app", "jukebox", "--dump-after", "-"])
		.output()
		.expect("spawn slab-tui");
	assert!(!out.status.success());
	assert!(
		String::from_utf8_lossy(&out.stderr).contains("unknown --app 'jukebox'"),
		"stderr: {}",
		String::from_utf8_lossy(&out.stderr)
	);

	let out = Command::new(env!("CARGO_BIN_EXE_slab-tui"))
		.arg(root().join(PLAYER))
		.args(["--width", "360", "--script", "TICK:2000", "--dump-after", "-"])
		.output()
		.expect("spawn slab-tui");
	assert!(out.status.success());
	let dump = String::from_utf8_lossy(&out.stdout);
	assert!(dump.contains("2:37"), "generic mode must keep the declared elapsed param:\n{dump}");
}
