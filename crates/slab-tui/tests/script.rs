//! Headless script-mode integration tests: the binary is the unit under
//! test (`--script` + `--dump-after`), so these cover compile → kernel
//! dispatch → cell grid → dump end to end.

use std::{
	path::{Path, PathBuf},
	process::Command,
	sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_DUMP: AtomicUsize = AtomicUsize::new(0);

fn root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run slab-tui headless; return the dump text.
fn run(example: &str, args: &[&str]) -> String {
	let serial = NEXT_DUMP.fetch_add(1, Ordering::Relaxed);
	let dump =
		std::env::temp_dir().join(format!("slab-tui-test-{}-{serial}.txt", std::process::id()));
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_slab-tui"));
	cmd.arg(root().join(example))
		.args(args)
		.args(["--dump-after", dump.to_str().unwrap()]);
	let out = cmd.output().expect("spawn slab-tui");
	assert!(out.status.success(), "slab-tui failed: {}", String::from_utf8_lossy(&out.stderr));
	let text = std::fs::read_to_string(&dump).expect("read dump");
	let _ = std::fs::remove_file(&dump);
	text
}

fn signals_line(dump: &str) -> &str {
	dump.lines().last().expect("non-empty dump")
}

/// Tab order in 10-settings is document order: save, reset, sort, then
/// the field text node (field=draft implies focusable). TAB TAB ENTER
/// activates the reset button.
#[test]
fn tab_tab_enter_fires_reset() {
	let dump = run("examples/10-settings.slab", &["--script", "TAB TAB ENTER"]);
	assert_eq!(signals_line(&dump), "signals: reset");
}

/// A signal emitted by a repeated node retains the selected item's key.
#[test]
fn list_identity_is_preserved_on_activation() {
	let dump = run("conformance/cases/16-list.slab", &["--script", "TAB ENTER"]);
	assert_eq!(signals_line(&dump), "signals: pick[item=\"0\"]");
}

/// An unsupported wide glyph paints no fallback but preserves terminal-cell
/// advance, so the following combining cluster and caret remain aligned.
#[test]
fn missing_wide_glyph_preserves_caret_columns() {
	let dump = run("crates/slab-tui/tests/fixtures/edit-wide.slab", &["--script", "TAB"]);
	assert!(
		dump.lines().any(|line| line.contains("中e\u{301}▏")),
		"wide missing-glyph advance or combining caret is misplaced:\n{dump}"
	);
}

/// Keyboard-only field editing: four tabs land on the field, TYPE:hi
/// emits a Change signal with the full committed text, BACKSPACE deletes
/// one grapheme cluster and emits again; the grid shows the remaining
/// text with the caret cell right after it.
#[test]
fn typing_into_field_emits_draft_and_renders() {
	let dump = run("examples/10-settings.slab", &["--script", "TAB TAB TAB TAB TYPE:hi BACKSPACE"]);
	assert_eq!(signals_line(&dump), "signals: draft=\"hi\" draft=\"h\"");
	assert!(
		dump.lines().any(|l| l.contains("h\u{258F}")),
		"field text + caret cell missing from grid:\n{dump}"
	);
}

/// The kernel focus-visible state drives the doc's own when-patches:
/// tab traversal must change the rendered grid (the focus ring stroke
/// draws box cells around the focused button), with no driver-side focus
/// painting. The fixture's buttons sit on 8x16 cell multiples because the
/// tui suppresses stroke outlines lacking two fully covered rows/columns.
#[test]
fn tab_traversal_changes_the_grid() {
	let fixture = "crates/slab-tui/tests/fixtures/focus-ring.slab";
	let one = run(fixture, &["--script", "TAB"]);
	let two = run(fixture, &["--script", "TAB TAB"]);
	let strip = |s: &str| {
		s.lines()
			.filter(|l| !l.starts_with("signals:"))
			.collect::<Vec<_>>()
			.join("\n")
	};
	assert_ne!(strip(&one), strip(&two), "focus ring did not move");
	let ring_column = |dump: &str| {
		dump.lines().find_map(|line| {
			line
				.char_indices()
				.find_map(|(i, c)| (c == '╭').then_some(i))
		})
	};
	// Save spans cells 1..11, Reset 15..25; the ring's top-left corner
	// tracks the focused button.
	assert_eq!(ring_column(&one), Some(1), "ring not on Save after one Tab:\n{one}");
	assert_eq!(ring_column(&two), Some(15), "ring not on Reset after two Tabs:\n{two}");
}

/// Without a script, --dump-after is byte-identical to
/// `slab render FILE --client tui --plain` — frozen as the conformance
/// golden for t2-railyard (same doc, manifest sizing: width 800,
/// height unbounded).
#[test]
fn railyard_dump_matches_conformance_golden() {
	let dump = run("examples/05-railyard.slab", &["--width", "800"]);
	let golden = std::fs::read_to_string(root().join("conformance/expected/t2-railyard.cells.txt"))
		.expect("read golden");
	assert_eq!(dump, golden, "railyard dump differs from cells golden");
}
