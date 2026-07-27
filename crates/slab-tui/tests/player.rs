//! Player-card integration tests (examples/00-player.slab): the binary's
//! `--script`/`--set`/`--dump-after` surface end to end, plus kernel
//! frame-geometry assertions for the visuals the cell medium cannot
//! express (opacity glyph swap, the sub-cell playhead knob).

use std::path::{Path, PathBuf};
use std::process::Command;

const PLAYER: &str = "examples/00-player.slab";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run slab-tui headless on the player; return the dump text.
fn run(name: &str, args: &[&str]) -> String {
    let dump =
        std::env::temp_dir().join(format!("slab-tui-player-{}-{name}.txt", std::process::id()));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_slab-tui"));
    cmd.arg(root().join(PLAYER))
        .args(["--width", "360"])
        .args(args)
        .args(["--dump-after", dump.to_str().unwrap()]);
    let out = cmd.output().expect("spawn slab-tui");
    assert!(
        out.status.success(),
        "slab-tui failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&dump).expect("read dump");
    let _ = std::fs::remove_file(&dump);
    text
}

/// Run slab-tui expecting failure; return (exit code, stderr).
fn run_err(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_slab-tui"))
        .arg(root().join(PLAYER))
        .args(args)
        .args(["--dump-after", "-"])
        .output()
        .expect("spawn slab-tui");
    assert!(!out.status.success(), "expected failure");
    (
        out.status.code().expect("exit code"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Compile the player and build a kernel instance with param overrides,
/// mirroring the binary's --set path (name lookup + type-checked
/// inst_set_param), then return the settled frame at t=0.
fn player_frame(sets: &[(&str, slab_kernel::frame::ParamValue)]) -> slab_kernel::flatten::Frame {
    let file = root().join(PLAYER);
    let src = std::fs::read_to_string(&file).expect("read player");
    let opts = slab_compile::Options { embed_assets: true, base_dir: file.parent().unwrap().to_path_buf(), ..slab_compile::Options::default() };
    let (slir, diags) = slab_compile::compile(&src, &opts);
    assert!(!diags.has_errors(), "player must compile clean");
    let bytes = slab_slir::write(&slir.expect("slir"));
    let (mut inst, _) = slab_slir::instance(&bytes).expect("host decode");
    slab_kernel::frame::inst_set_env(&mut inst, 360.0, 0.0, 2, false, false);
    for (name, v) in sets {
        let p = (0..inst.doc.parm_name.len())
            .position(|p| inst.doc.strs[inst.doc.parm_name[p] as usize] == *name)
            .unwrap_or_else(|| panic!("unknown param '{name}'"));
        assert!(
            slab_kernel::frame::inst_set_param(&mut inst, p as u32, v),
            "inst_set_param rejected {name}"
        );
    }
    slab_kernel::frame::inst_frame(&mut inst, 0.0)
}

fn pv_num(kind: u32, num: f64) -> slab_kernel::frame::ParamValue {
    slab_kernel::frame::ParamValue {
        kind,
        num,
        s: String::new(),
        rgba: 0,
        sym: String::new(),
    }
}

/// Effective group opacity over each Text op, keyed by its string.
/// The glyph swap in the player is pure opacity (when playing {...}),
/// which flatten expresses as GroupPush around the text op.
fn text_opacities(fr: &slab_kernel::flatten::Frame) -> Vec<(String, f64)> {
    use slab_kernel::flatten::FrameOp;
    let mut stack: Vec<f64> = vec![1.0];
    let mut out = Vec::new();
    for op in &fr.ops {
        match op {
            FrameOp::GroupPush(g) => stack.push(stack.last().unwrap() * g.opacity),
            FrameOp::GroupPop => {
                stack.pop();
            }
            FrameOp::Text(t) => out.push((
                fr.strings[t.str_ref as usize].clone(),
                *stack.last().unwrap(),
            )),
            _ => {}
        }
    }
    out
}

/// The playhead knob is the only 8x8 fully-round rect in the doc
/// (rect w=8 h=8 radius=999 inside the pack=end progress row).
fn knob_x(fr: &slab_kernel::flatten::Frame) -> f64 {
    use slab_kernel::flatten::FrameOp;
    for op in &fr.ops {
        if let FrameOp::Rect(r) = op
            && r.w == 8.0
            && r.h == 8.0
            && r.radius >= 4.0
        {
            return r.x;
        }
    }
    panic!("knob rect (8x8 round) not found in frame ops");
}

/// (a) Tab order is document order — shuffle, prev, toggle — so
/// TAB TAB TAB ENTER activates the play circle and emits `toggle`;
/// the same dump carries the transport labels and the default title.
#[test]
fn tab_tab_tab_enter_fires_toggle() {
    let dump = run("toggle", &["--script", "TAB TAB TAB ENTER"]);
    assert_eq!(dump.lines().last().unwrap(), "signals: toggle");
    for needle in ["SHUF", "LOOP", "<<", ">>", "Pale Green Things"] {
        assert!(dump.contains(needle), "dump missing {needle:?}:\n{dump}");
    }
}

/// (b) `--set` overrides land in the grid: the title param is re-rendered,
/// text/bool/pct coercions all pass through the kernel type check.
#[test]
fn set_overrides_change_the_grid() {
    let dump = run(
        "settitle",
        &[
            "--set",
            "title=This Year",
            "--set",
            "playing=false",
            "--set",
            "elapsed=0:00",
        ],
    );
    assert!(dump.contains("This Year"), "override missing:\n{dump}");
    assert!(
        !dump.contains("Pale Green Things"),
        "old title still present"
    );
    assert!(dump.contains("0:00"), "elapsed override missing");
}

/// (b) The |>/|| swap is pure opacity, which the cell medium cannot
/// express (`slab_kernel::cells`: GroupPush is ignored — both glyph texts land in
/// the grid regardless of `playing`; documented degradation). The swap is
/// asserted where it lives: the frame-op group opacities around the two
/// glyph text ops, driven through the same type-checked inst_set_param
#[test]
fn playing_swaps_glyph_opacity_in_frame_ops() {
    let opacity_of = |fr: &slab_kernel::flatten::Frame, glyph: &str| -> f64 {
        text_opacities(fr)
            .into_iter()
            .find(|(s, _)| s == glyph)
            .unwrap_or_else(|| panic!("glyph {glyph:?} not in frame"))
            .1
    };
    let paused = player_frame(&[("playing", pv_num(4, 0.0))]);
    assert_eq!(opacity_of(&paused, "|>"), 1.0, "paused: |> must be visible");
    assert_eq!(opacity_of(&paused, "||"), 0.0, "paused: || must be hidden");
    let playing = player_frame(&[("playing", pv_num(4, 1.0))]);
    assert_eq!(
        opacity_of(&playing, "|>"),
        0.0,
        "playing: |> must be hidden"
    );
    assert_eq!(
        opacity_of(&playing, "||"),
        1.0,
        "playing: || must be visible"
    );

    // Cell-medium ground truth: group opacity multiplies through the cell
    // rasterizer, so the plain grid shows exactly one transport glyph and
    // the swap is visible at the binary level too.
    let g_pause = run("glyphpause", &["--set", "playing=false"]);
    let g_play = run("glyphplay", &["--set", "playing=true"]);
    assert_ne!(
        g_pause, g_play,
        "plain grid must swap the transport glyph with `playing`"
    );
    assert!(
        g_pause.contains("|>"),
        "paused grid missing |> glyph:\n{g_pause}"
    );
    assert!(
        g_play.contains("||") && !g_play.contains("|>"),
        "playing grid must show || only:\n{g_play}"
    );
}

/// (c) progress drives the playhead: the knob (8x8 round rect, pack=end
/// in a w=param.progress row) moves right between 20% and 80% by ~60% of
/// the 316u waveform span. Grid cells can't show the knob (colored rect,
/// sub-cell), so the assertion reads frame geometry through the kernel.
#[test]
fn progress_moves_the_playhead_knob() {
    let x20 = knob_x(&player_frame(&[("progress", pv_num(2, 20.0))]));
    let x80 = knob_x(&player_frame(&[("progress", pv_num(2, 80.0))]));
    assert!(x80 > x20, "knob must move right: x20={x20} x80={x80}");
    let delta = x80 - x20;
    assert!(
        (150.0..230.0).contains(&delta),
        "knob delta {delta} out of range for a 316u waveform"
    );
    // The binary accepts the same values through --set pct coercion
    // ('%' suffix optional, exit 0).
    run("prog20", &["--set", "progress=20%"]);
    run("prog80", &["--set", "progress=80"]);
}

/// Bad --set input is a clear error with exit code 2: unknown param,
/// un-coercible value, and a bool that isn't one.
#[test]
fn bad_set_exits_two() {
    let (code, err) = run_err(&["--set", "progress=banana"]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(
        err.contains("'banana' is not a percentage"),
        "stderr: {err}"
    );
    let (code, err) = run_err(&["--set", "nope=1"]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown param 'nope'"), "stderr: {err}");
    let (code, err) = run_err(&["--set", "playing=maybe"]);
    assert_eq!(code, 2);
    assert!(err.contains("'maybe' is not a bool"), "stderr: {err}");
}

/// (d) The queue hole: UP NEXT header renders and the hole region below
/// carries no host content (holes are none:cap-hole on tui — only the
/// card's own outline passes through; nothing crashes).
#[test]
fn queue_hole_renders_blank() {
    let dump = run("queue", &["--script", ""]);
    let mut lines = dump.lines().rev();
    let signals = lines.next().unwrap();
    assert_eq!(signals, "signals:", "empty script must emit no signals");
    let up_next = dump.lines().position(|l| l.contains("UP NEXT")).unwrap();
    for l in dump.lines().skip(up_next + 1) {
        if l.starts_with("signals:") {
            break;
        }
        assert!(
            !l.chars().any(|c| c.is_ascii_alphanumeric()),
            "hole region must carry no host content, got {l:?}"
        );
    }
}
