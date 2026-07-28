//! Runtime font tables must replace matching compiled metrics without changing
//! the authored family reference carried through style resolution.

use crate::{frame, slir, textm};

/// Builds the compiled font table used by the registration test.
pub fn compiled_doc() -> slir::Doc {
	let mut doc = slir::doc_new();
	doc.strs.push(String::new());
	doc.font_family.push(0);
	doc.font_class.push(0);
	doc.font_weight.push(400);
	doc.font_upem.push(1_000);
	doc.font_ascent.push(800);
	doc.font_descent.push(-200);
	doc.font_line_gap.push(0);
	doc.font_default_adv.push(500);
	doc.font_cmap_off.push(0);
	doc.font_cmap_len.push(1);
	doc.font_cmap_cp.push(65);
	doc.font_cmap_gid.push(1);
	doc.font_adv.push(500);
	doc
}

/// Verifies that a registered runtime font supersedes matching compiled
/// metrics.
pub fn test_runtime_font_register_overrides_matching_family() {
	let mut inst = frame::inst_shell();
	inst.doc = compiled_doc();
	inst.doc.ok = true;
	frame::inst_init(&mut inst);

	let family = u32::try_from(inst.doc.strs.len()).expect("font family index exceeds u32");
	inst.doc.strs.push("TEST MONO".to_owned());
	let registered = frame::inst_font_register(
		&mut inst,
		"Test Mono",
		400,
		1_000,
		800,
		-200,
		0,
		900,
		&[65],
		&[1],
		&[900],
	);

	assert_eq!(registered, 1, "appends after compiled table");
	assert_eq!(
		inst.doc.font_class[usize::try_from(registered).expect("nonnegative font index")],
		1,
		"mono fallback class"
	);
	assert_eq!(slir::font_select(&inst.doc, family, 400), registered, "family match wins");

	const WRAP: bool = true;
	const ELLIPSIS: bool = false;
	let layout =
		textm::measure_text(&inst.doc, registered, 10.0, 1.4, 0.0, "A", 100.0, WRAP, ELLIPSIS, -1);
	assert_eq!(layout.w, 9.0, "registered advance drives measurement");
}
