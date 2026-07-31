//! Text wrapping edge cases against a synthetic font table.
//!
//! The table uses 1000 units per em: mapped letters are 500 units wide, spaces
//! and NBSP 250, ellipsis 800, and default advance 600. At size 10, these
//! become 5, 2.5, 8, and 6 units.

use crate::{
	flatten::{self, FrameGlyph, FrameOp},
	frame, graphemes,
	slir::{self, Doc},
	test_cells,
	textm::{self, TextLayout},
};

/// Builds the synthetic font document used by the text measurement checks.
pub fn font_doc() -> Doc {
	let mut doc = slir::doc_new();
	// Font 0's family id references the interned name; rich bold/code
	// selection resolves through it.
	doc.strs.push("Sans".to_owned());
	doc.font_family.push(0);
	doc.font_class.push(0);
	doc.font_weight.push(400);
	doc.font_upem.push(1000);
	doc.font_ascent.push(800);
	doc.font_descent.push(0_i32.wrapping_sub(200));
	doc.font_line_gap.push(0);
	doc.font_default_adv.push(600);
	doc.font_cmap_off.push(0);

	// The cmap is sorted by code point: space, a through c, x, NBSP, ellipsis.
	let code_points = [32, 97, 98, 99, 120, 160, 0x2026];
	let advances = [250, 500, 500, 500, 500, 250, 800];
	doc.font_cmap_len
		.push(i32::try_from(code_points.len()).expect("synthetic cmap fits in i32"));
	for (index, (&code_point, &advance)) in code_points.iter().zip(&advances).enumerate() {
		doc.font_cmap_cp.push(code_point);
		doc.font_cmap_gid
			.push(u32::try_from(index + 1).expect("synthetic glyph identifier fits in u32"));
		doc.font_adv.push(advance);
	}
	doc
}

/// Measures text with the synthetic font and the standard test metrics.
pub fn measure(
	doc: &Doc,
	text: &str,
	max_width: f64,
	wrap: bool,
	ellipsis: bool,
	max_lines: i32,
) -> TextLayout {
	textm::measure_text(doc, 0, 10.0, 1.4, 0.0, text, max_width, wrap, ellipsis, max_lines)
}

/// Returns the string represented by one measured line.
pub fn line_str(layout: &TextLayout, line: i32) -> String {
	let line = usize::try_from(line).expect("line index must be nonnegative");
	let start = usize::try_from(layout.ls[line]).expect("line start must be nonnegative");
	let end = usize::try_from(layout.le[line]).expect("line end must be nonnegative");
	crate::rt::str_from_chars(&layout.chars[start..end])
}

/// Checks ordinary word wrapping and the resulting block metrics.
pub fn test_wrap_basic() {
	let doc = font_doc();
	// "aaa bbb ccc": words are 15 units and spaces are 2.5; a 33-unit line
	// fits two words exactly within 32.5 units.
	let layout = measure(&doc, "aaa bbb ccc", 33.0, true, false, -1);
	assert_eq!(layout.ls.len(), 2, "two lines");
	assert_eq!(line_str(&layout, 0), "aaa bbb", "first line words");
	assert_eq!(line_str(&layout, 1), "ccc", "second line word");
	assert_eq!(layout.line_w[0], 32.5, "line 0 width");
	assert_eq!(layout.line_w[1], 15.0, "line 1 width");
	assert_eq!(layout.w, 32.5, "block width");
	assert_eq!(layout.h, 28.0, "two lines at 14u");
	assert!(!layout.truncated, "not truncated");
}

/// Checks that a nonbreaking space does not create a wrapping opportunity.
pub fn test_wrap_nbsp_glue() {
	let doc = font_doc();
	// NBSP is not a break point: "aa\u{00A0}bb" is one 22.5-unit word.
	let layout = measure(&doc, "aa bb cc", 25.0, true, false, -1);
	assert_eq!(layout.ls.len(), 2, "nbsp glues");
	assert_eq!(line_str(&layout, 0), "aa bb", "glued word intact");
	assert_eq!(layout.line_w[0], 22.5, "glued width");
}

/// Checks that an explicit newline starts a new measured line.
pub fn test_hard_newline() {
	let doc = font_doc();
	let layout = measure(&doc, "aa\nbb", 1000.0, true, false, -1);
	assert_eq!(layout.ls.len(), 2, "newline splits");
	assert_eq!(line_str(&layout, 0), "aa", "first hard line");
	assert_eq!(line_str(&layout, 1), "bb", "second hard line");
}

/// Checks per-character hard breaks when a word exceeds the line width.
pub fn test_hard_break_long_word() {
	let doc = font_doc();
	// "aaaaa" is 25 units; a 12-unit maximum breaks it as aa|aa|a.
	let layout = measure(&doc, "aaaaa", 12.0, true, false, -1);
	assert_eq!(layout.ls.len(), 3, "hard-broken into 3");
	assert_eq!(line_str(&layout, 0), "aa", "chunk 1");
	assert_eq!(line_str(&layout, 1), "aa", "chunk 2");
	assert_eq!(line_str(&layout, 2), "a", "chunk 3");
}
/// Checks CJK opportunities while preserving kinsoku-prohibited punctuation.
pub fn test_wrap_cjk_kinsoku() {
	let doc = font_doc();
	let text = "天地。玄黄、宇宙」洪荒！開闢）以前";
	let layout = measure(&doc, text, 12.0, true, false, -1);
	assert!(layout.ls.len() > 1, "CJK paragraph wraps");
	let lines: Vec<String> = (0..i32::try_from(layout.ls.len()).expect("line count fits i32"))
		.map(|line| line_str(&layout, line))
		.collect();
	assert_eq!(lines.concat(), text, "wrapping preserves CJK text");
	for line in lines.iter().skip(1) {
		assert!(
			!line.starts_with(['。', '、', '」', '！', '）']),
			"closing punctuation must not open a line: {line:?}"
		);
	}
}

/// Checks that CJK and Latin runs use their respective wrapping behavior.
pub fn test_wrap_mixed_cjk_latin() {
	let doc = font_doc();
	let layout = measure(&doc, "ab天地cd", 12.0, true, false, -1);
	let lines: Vec<String> = (0..i32::try_from(layout.ls.len()).expect("line count fits i32"))
		.map(|line| line_str(&layout, line))
		.collect();
	assert_eq!(lines, ["ab", "天地", "cd"], "mixed break classes wrap greedily");
}
/// Checks the UAX #14 HY opportunity in otherwise ordinary Latin text.
pub fn test_wrap_latin_hyphen_opportunity() {
	let doc = font_doc();
	let layout = measure(&doc, "aa foo-bar", 45.0, true, false, -1);
	assert_eq!(line_str(&layout, 0), "aa foo-", "HY opportunity fills the current line");
	assert_eq!(line_str(&layout, 1), "bar", "whole token would also fit the next line");
}

/// Checks that zero-width space supplies a UAX #14 opportunity.
pub fn test_wrap_zero_width_space_opportunity() {
	let doc = font_doc();
	let layout = measure(&doc, "a aa\u{200b}bb", 30.0, true, false, -1);
	assert_eq!(line_str(&layout, 0), "a aa\u{200b}", "ZWSP opportunity fills the current line");
	assert_eq!(line_str(&layout, 1), "bb", "whole token would also fit the next line");
}

/// Checks that an ideographic space supplies a UAX #14 opportunity.
pub fn test_wrap_ideographic_space_opportunity() {
	let doc = font_doc();
	let layout = measure(&doc, "a aa\u{3000}bb", 30.0, true, false, -1);
	assert_eq!(
		line_str(&layout, 0),
		"a aa\u{3000}",
		"ideographic-space opportunity fills the current line"
	);
	assert_eq!(line_str(&layout, 1), "bb", "whole token would also fit the next line");
}

/// Checks that GL-class nonbreaking space remains attached even when over
/// width.
pub fn test_wrap_nbsp_overlong_glue() {
	let doc = font_doc();
	let layout = measure(&doc, "a b c", 12.0, true, false, -1);
	assert_eq!(line_str(&layout, 0), "a b", "no fallback break around NBSP");
	assert_eq!(line_str(&layout, 1), "c", "following ASCII word wraps normally");
}

/// Checks that an overlong ASCII URL retains legacy grapheme hard breaks.
pub fn test_wrap_overlong_latin_url() {
	let doc = font_doc();
	let text = "https://example.test";
	let layout = measure(&doc, text, 12.0, true, false, -1);
	let joined: String = (0..i32::try_from(layout.ls.len()).expect("line count fits i32"))
		.map(|line| line_str(&layout, line))
		.collect();
	assert_eq!(joined, text, "hard breaks preserve every URL cluster");
	assert!(layout.ls.len() > 1, "overlong URL wraps");
}

/// Checks SA-class fallback without dictionary segmentation or stalled scans.
pub fn test_wrap_thai_grapheme_fallback() {
	let doc = font_doc();
	let text = "ภาษาไทย";
	let layout = measure(&doc, text, 12.0, true, false, -1);
	let joined: String = (0..i32::try_from(layout.ls.len()).expect("line count fits i32"))
		.map(|line| line_str(&layout, line))
		.collect();
	assert_eq!(joined, text, "Thai fallback preserves clusters");
	assert!(layout.ls.len() > 1, "Thai zero-opportunity run wraps");
}

/// Checks clipping and ellipsis insertion when wrapping is disabled.
pub fn test_nowrap_ellipsis() {
	let doc = font_doc();
	// "abcabc" is 30 units; within a 20-unit maximum, "ab" (10) plus the
	// ellipsis (8) fits, but adding "c" would require 23 units.
	let layout = measure(&doc, "abcabc", 20.0, false, true, -1);
	assert_eq!(layout.ls.len(), 1, "one line");
	assert!(layout.truncated, "truncated");
	assert_eq!(line_str(&layout, 0), "ab…", "ellipsis cut");
	assert_eq!(layout.line_w[0], 18.0, "cut width 10 + 8");
}

/// Checks that no-wrap truncation retains text when ellipsis is disabled.
pub fn test_nowrap_clipped_no_ellipsis() {
	let doc = font_doc();
	let layout = measure(&doc, "abcabc", 20.0, false, false, -1);
	assert!(layout.truncated, "flagged truncated");
	assert_eq!(line_str(&layout, 0), "abcabc", "text kept without ellipsis");
}

/// Checks that wrapping respects the maximum line count.
pub fn test_max_lines() {
	let doc = font_doc();
	let layout = measure(&doc, "aaa bbb ccc", 16.0, true, false, 2);
	assert_eq!(layout.ls.len(), 2, "clamped to 2 lines");
	assert!(layout.truncated, "truncated by max_lines");
}

/// Checks that max-line truncation appends an ellipsis when it fits.
pub fn test_max_lines_ellipsis_appends() {
	let doc = font_doc();
	// Each word occupies its own line. With one line allowed, the ellipsis
	// appends to "aaa" because 15 + 8 = 23 is at most 26.
	let layout = measure(&doc, "aaa bbb", 26.0, true, true, 1);
	assert_eq!(layout.ls.len(), 1, "one line kept");
	assert_eq!(line_str(&layout, 0), "aaa…", "ellipsis appended");
	assert_eq!(layout.line_w[0], 23.0, "appended width");
}

/// Checks fallback advance and tracking for measured glyphs.
pub fn test_default_advance() {
	let doc = font_doc();
	// 'z' (U+007A) is absent from the cmap, so the 600-unit default advance
	// becomes 6 units at size 10.
	assert_eq!(textm::char_w(&doc, 0, 10.0, 0.0, 122), 6.0, "default advance");
	assert_eq!(
		textm::char_w(&doc, 0, 10.0, 1.5, graphemes::ZWJ),
		0.0,
		"joiners do not consume a fallback advance or tracking"
	);
	assert_eq!(
		textm::char_w(&doc, 0, 10.0, 1.5, graphemes::VS15),
		0.0,
		"variation selectors do not consume a fallback advance or tracking"
	);
	assert_eq!(
		textm::char_w(&doc, 0, 10.0, 1.5, 0x0301),
		7.5,
		"missing spacing marks retain the fallback advance and tracking"
	);
	assert_eq!(
		textm::char_w(&doc, 0, 10.0, 1.5, 0x0001),
		7.5,
		"ordinary controls retain the fallback advance and tracking"
	);
	assert_eq!(textm::char_w(&doc, 0, 10.0, 1.5, 97), 6.5, "tracking after glyph");

	let mut modifier_doc = font_doc();
	modifier_doc.font_cmap_cp.insert(6, graphemes::ZWJ);
	modifier_doc.font_cmap_gid.insert(6, 91);
	modifier_doc.font_adv.insert(6, 600);
	modifier_doc.font_cmap_cp.push(graphemes::VS15);
	modifier_doc.font_cmap_gid.push(92);
	modifier_doc.font_adv.push(600);
	modifier_doc.font_cmap_len[0] += 2;
	let mut instance = frame::inst_shell();
	instance.doc = modifier_doc;
	let mut output = flatten::frame_new();
	output.strings.push("\u{200D}\u{FE0E}".to_owned());
	output.glyphs.extend([
		FrameGlyph {
			font:    0,
			gid:     0,
			cluster: 0,
			x:       10.0,
			y:       12.0,
			size:    10.0,
		},
		FrameGlyph {
			font:    0,
			gid:     0,
			cluster: 1,
			x:       10.0,
			y:       12.0,
			size:    10.0,
		},
	]);
	let mut op = test_cells::text_op(10.0, 12.0, 0, 0);
	op.font = 0;
	op.glyph_len = 2;
	output.ops.push(FrameOp::Text(op));
	let glyphs = frame::text_glyphs(&instance, &output, 0);
	assert_eq!(glyphs.len(), 2, "both modifiers remain addressable");
	assert!(
		glyphs.iter().all(|glyph| glyph.gid == 0 && glyph.x == 10.0),
		"glyph modifiers neither paint nor advance: {glyphs:?}"
	);
}

/// Checks line metrics, half-leading ascent, and empty-text dimensions.
pub fn test_metrics() {
	let doc = font_doc();
	assert_eq!(textm::line_h(10.0, 1.4), 14.0, "line height");
	// Ascent is 8 + (14 - 10) / 2 = 10: half-leading over the 1000-upem box.
	assert_eq!(textm::ascent(&doc, 0, 10.0, 1.4), 10.0, "half-leading ascent");
	// Empty text still occupies one line.
	let layout = measure(&doc, "", 100.0, true, false, -1);
	assert_eq!(layout.ls.len(), 1, "empty text one line");
	assert_eq!(layout.h, 14.0, "empty text height");
	assert_eq!(layout.w, 0.0, "empty text width");
}

/// Builds a solvable single-text-node instance around the synthetic font.
fn text_instance(content: &str) -> frame::Instance {
	let mut doc = font_doc();
	doc.ok = true;
	doc.strs.push(String::new());
	let content_ref = u32::try_from(doc.strs.len()).expect("string pool fits u32");
	doc.strs.push(content.to_owned());
	doc.node_kind.push(slir::K_TEXT);
	doc.node_flags.push(slir::F_NOWRAP);
	doc.node_parent.push(slir::NONE);
	doc.node_first.push(slir::NONE);
	doc.node_next.push(slir::NONE);
	doc.node_key.push(0);
	doc.node_id.push(0);
	doc.node_line.push(3);
	doc.attr_index.push(0);
	doc.aval_tag.push(slir::T_STR);
	doc.aval_lo.push(content_ref);
	doc.aval_hi.push(0);
	doc.aval_num.push(0.0);
	doc.attr_id.push(slir::A_CONTENT);
	doc.attr_val.push(0);
	doc.attr_index.push(1);
	let mut inst = frame::inst_shell();
	inst.doc = doc;
	frame::inst_init(&mut inst);
	frame::inst_set_env(&mut inst, 500.0, 500.0, 0, false, false);
	inst
}

/// Verifies deterministic East-Asian-Width fallback advances (C-16).
///
/// Checks codepoints the cmap does not cover: mono-class families charge two
/// cells for wide codepoints; vector families keep the single replacement
/// advance.
pub fn test_fallback_advance_eaw() {
	let mut doc = font_doc();
	// 日 (U+65E5) is EAW-wide and absent from the synthetic cmap.
	assert_eq!(
		textm::char_w(&doc, 0, 10.0, 0.0, 0x65e5),
		6.0,
		"vector families charge one replacement advance"
	);
	doc.font_class[0] = 1;
	assert_eq!(
		textm::char_w(&doc, 0, 10.0, 0.0, 0x65e5),
		12.0,
		"mono families charge two cells for wide uncovered codepoints"
	);
	assert_eq!(
		textm::char_w(&doc, 0, 10.0, 0.0, 122),
		6.0,
		"mono families charge one cell for narrow uncovered codepoints"
	);
	assert_eq!(
		textm::char_w(&doc, 0, 10.0, 0.0, 97),
		5.0,
		"covered advances ignore the family class"
	);
}

/// Verifies visual bidi reordering while source clusters remain logical.
pub fn test_bidi_visual_order() {
	let doc = font_doc();
	let chars: Vec<u32> = "abc אבג".chars().map(u32::from).collect();
	let shaped = textm::shape_line(&doc, 0, 10.0, 0.0, &chars);
	let starts: Vec<i32> = shaped
		.clusters
		.iter()
		.map(|cluster| cluster.start)
		.collect();
	assert_eq!(starts, [0, 1, 2, 3, 6, 5, 4], "RTL clusters paint in visual order");
	assert!(shaped.runs.iter().any(|run| run.rtl), "frame splits an RTL shaped run");
	let layout = measure(&doc, "abc אבג", 200.0, false, false, -1);
	let cache = std::cell::RefCell::new(textm::ShapeCache::default());
	let shaper = textm::Shaper { d: &doc, cache: &cache };
	let mut editor = crate::edit::es_new(0, "abc אבג");
	// Arrow stops are logical grapheme boundaries: monotone even across the
	// bidi run seam, so no stop can be visited twice or skipped.
	editor.caret = 4;
	editor.anchor = 4;
	crate::edit::visual_step(shaper, &mut editor, &layout, 1, false);
	assert_eq!(editor.caret, 5, "ArrowRight advances one logical grapheme in RTL text");
	crate::edit::visual_step(shaper, &mut editor, &layout, -1, false);
	assert_eq!(editor.caret, 4, "ArrowLeft reverses the same logical step");
	editor.caret = 7;
	editor.anchor = 7;
	crate::edit::visual_step(shaper, &mut editor, &layout, 1, false);
	assert_eq!(editor.caret, 7, "ArrowRight at the logical end stays put");
}

/// Verifies grapheme clusters are indivisible for wrapping and caret geometry.
pub fn test_cluster_aware_wrap_and_caret() {
	let doc = font_doc();
	let layout = measure(&doc, "a\u{301}b", 6.0, true, false, -1);
	let cache = std::cell::RefCell::new(textm::ShapeCache::default());
	let shaper = textm::Shaper { d: &doc, cache: &cache };
	assert_eq!(layout.ls.len(), 2, "oversized grapheme stays intact");
	assert_eq!(line_str(&layout, 0), "a\u{301}", "hard wrap does not split combining marks");
	let first = shaper.line(&layout, 0).expect("line 0 shapes");
	assert_eq!(
		first
			.clusters
			.iter()
			.map(|cluster| (cluster.start, cluster.end))
			.collect::<Vec<_>>(),
		[(0, 2)],
		"base and combining mark expose one caret cluster"
	);
	assert_eq!(
		textm::caret_x(shaper, &layout, 0, 1),
		first.width,
		"an interior source offset snaps past the cluster"
	);
	assert_eq!(
		textm::selection_bands(shaper, &layout, 0, 1, 2),
		[(0.0, first.width)],
		"partial logical selection paints the whole cluster"
	);
}

/// Verifies missing graphemes select the next registered font table.
pub fn test_font_fallback_splits_shaped_runs() {
	let mut doc = font_doc();
	doc.font_family.push(0);
	doc.font_class.push(0);
	doc.font_weight.push(400);
	doc.font_upem.push(1000);
	doc.font_ascent.push(800);
	doc.font_descent.push(-200);
	doc.font_line_gap.push(0);
	doc.font_default_adv.push(600);
	doc.font_cmap_off
		.push(i32::try_from(doc.font_cmap_cp.len()).expect("cmap fits i32"));
	doc.font_cmap_len.push(1);
	doc.font_cmap_cp.push(0x2715);
	doc.font_cmap_gid.push(99);
	doc.font_adv.push(500);

	let chars: Vec<u32> = "a✕b".chars().map(u32::from).collect();
	let shaped = textm::shape_line(&doc, 0, 10.0, 0.0, &chars);
	assert_eq!(
		shaped.runs.iter().map(|run| run.font).collect::<Vec<_>>(),
		[0, 1, 0],
		"fallback face becomes a distinct positioned run"
	);
	assert_eq!(shaped.runs[1].glyphs[0].gid, 99, "fallback cmap supplies the emitted glyph");
}

/// Verifies uncovered-glyph run marking on text operations (C-16).
///
/// Runs are half-open codepoint ranges into the op string, coalesced across
/// adjacent uncovered clusters, and absent for fully covered strings.
pub fn test_uncovered_runs_marked() {
	let mut inst = text_instance("ab\u{2715}\u{2715}xa");
	let output = frame::inst_frame(&mut inst, 0.0);
	let text = output
		.ops
		.iter()
		.find_map(|op| match op {
			FrameOp::Text(text) => Some(text),
			_ => None,
		})
		.expect("text op present");
	assert_eq!(
		(text.uncov_off, text.uncov_len),
		(0, 1),
		"adjacent uncovered clusters coalesce into one run"
	);
	assert_eq!(output.uncovered, [2, 4], "run bounds are codepoint offsets into the op string");

	let mut covered = text_instance("abxa");
	let output = frame::inst_frame(&mut covered, 0.0);
	let text = output
		.ops
		.iter()
		.find_map(|op| match op {
			FrameOp::Text(text) => Some(text),
			_ => None,
		})
		.expect("text op present");
	assert_eq!(text.uncov_len, 0, "covered strings mark no runs");
	assert!(output.uncovered.is_empty(), "covered frame pool stays empty");
}

/// Verifies the cumulative per-instance diagnostic set (C-17).
///
/// The per-solve stream stays one-shot, while [`frame::inst_diags`] retains
/// every note until a new document initializes.
pub fn test_cumulative_diagnostics_survive_resolves() {
	let mut inst = text_instance("a\u{2715}b");
	let first = frame::inst_frame(&mut inst, 0.0);
	assert!(
		first
			.diagnostics
			.iter()
			.any(|diagnostic| diagnostic.code == "glyph-missing"),
		"first solve streams the glyph note"
	);
	inst.dirty = true;
	let second = frame::inst_frame(&mut inst, 1.0);
	assert!(
		second
			.diagnostics
			.iter()
			.all(|diagnostic| diagnostic.code != "glyph-missing"),
		"intermediate solves consume the one-shot stream"
	);
	assert!(
		frame::inst_diags(&inst)
			.iter()
			.any(|diagnostic| diagnostic.code == "glyph-missing" && diagnostic.line == 3),
		"cumulative set stays queryable after any solve"
	);
	frame::inst_init(&mut inst);
	assert!(
		frame::inst_diags(&inst).is_empty(),
		"new document assignment resets the cumulative set"
	);
}

fn assert_splice_layout_eq(actual: &TextLayout, expected: &TextLayout, context: &str) {
	assert_eq!(actual.chars, expected.chars, "{context}: chars");
	assert_eq!(actual.ls, expected.ls, "{context}: line starts");
	assert_eq!(actual.le, expected.le, "{context}: line ends");
	assert_eq!(actual.src_ls, expected.src_ls, "{context}: source line starts");
	assert_eq!(actual.src_le, expected.src_le, "{context}: source line ends");
	assert_eq!(
		actual
			.line_w
			.iter()
			.map(|width| width.to_bits())
			.collect::<Vec<_>>(),
		expected
			.line_w
			.iter()
			.map(|width| width.to_bits())
			.collect::<Vec<_>>(),
		"{context}: line widths"
	);
	assert_eq!(actual.hard_lines, expected.hard_lines, "{context}: hard lines");
	assert_eq!(actual.hard_src, expected.hard_src, "{context}: hard source offsets");
	assert_eq!(actual.w.to_bits(), expected.w.to_bits(), "{context}: width");
	assert_eq!(actual.h.to_bits(), expected.h.to_bits(), "{context}: height");
	assert_eq!(actual.truncated, expected.truncated, "{context}: truncation");
	assert_eq!(actual.spans, expected.spans, "{context}: spans");
}

fn splice_differential(wrap: bool, rich: bool) {
	const EDITS: usize = 256;
	let doc = font_doc();
	let mut cache = textm::ShapeCache::default();
	let mut text = (0..200)
		.map(|line| format!("abc x line{line:03} cab xxx"))
		.collect::<Vec<_>>()
		.join("\n");
	let mut state = 0x4d59_5df4_d0f3_3173_u64;
	let mut random = || {
		state = state
			.wrapping_mul(6_364_136_223_846_793_005)
			.wrapping_add(1);
		state
	};
	let max_w = if wrap { 31.0 } else { 10_000.0 };

	for edit in 0..EDITS {
		let old_chars: Vec<char> = text.chars().collect();
		let old_len = old_chars.len();
		let newline_positions: Vec<usize> = old_chars
			.iter()
			.enumerate()
			.filter_map(|(index, &ch)| (ch == '\n').then_some(index))
			.collect();
		let (at, removed, inserted) = match edit % 8 {
			0 => (0, 0, vec![if edit % 16 == 0 { '\n' } else { 'a' }]),
			1 => (old_len, 0, vec![' ', 'f', 'f', 'i']),
			2 => {
				let boundary = newline_positions
					.get((random() as usize) % newline_positions.len())
					.map_or(old_len, |position| position + 1);
				(boundary, 0, vec!['b', 'c'])
			},
			3 => {
				let newline = newline_positions[(random() as usize) % newline_positions.len()];
				let start = newline.saturating_sub(2);
				let count = (old_len - start).min(5);
				(start, count, Vec::new())
			},
			4 => {
				let at = (random() as usize) % (old_len + 1);
				let choices = ['x', '\n', ' ', 'c', '\u{301}'];
				(at, 0, vec![choices[(random() as usize) % choices.len()]])
			},
			5 => {
				let at = (random() as usize) % (old_len + 1);
				let choices =
					[vec!['a', 'b', 'c'], vec!['x', '\n', 'x'], vec![' ', 'f', 'i', ' '], vec![
						'c', 'a',
					]];
				(at, 0, choices[(random() as usize) % choices.len()].clone())
			},
			6 => {
				let at = (random() as usize) % old_len;
				let count = ((random() as usize) % 9 + 1).min(old_len - at);
				(at, count, Vec::new())
			},
			_ => {
				let at = (random() as usize) % old_len;
				let count = ((random() as usize) % 6 + 1).min(old_len - at);
				(at, count, vec!['c', '\n', 'a'])
			},
		};
		let spans_old = if rich {
			rich_test_spans(&text)
		} else {
			crate::edit::InlineSpans::empty()
		};
		let text_cps: Vec<u32> = text.chars().map(u32::from).collect();
		let full_prev = textm::measure_text_cached(
			&doc, 0, 10.0, 1.4, 0.0, &text_cps, max_w, wrap, false, -1, &spans_old, &mut cache,
		);
		let mut new_chars = old_chars;
		new_chars.splice(at..at + removed, inserted.iter().copied());
		let new_text: String = new_chars.iter().collect();
		let new_cps: Vec<u32> = new_text.chars().map(u32::from).collect();
		let delta = textm::TextDelta {
			at:       i32::try_from(at).expect("test text fits i32"),
			removed:  i32::try_from(removed).expect("test edit fits i32"),
			inserted: i32::try_from(inserted.len()).expect("test edit fits i32"),
		};
		// The positional transform edits apply to spans; `follows_splice`
		// must certify exactly this shape of change.
		let mut spans_new = spans_old.clone();
		for style in 0..=crate::edit::STYLE_CODE {
			let ranges = spans_new.get_mut(style).expect("known style id");
			ranges.delete(delta.at, delta.at.wrapping_add(delta.removed));
			ranges.insert(delta.at, delta.inserted);
		}
		assert!(
			spans_old.follows_splice(&spans_new, delta),
			"positionally shifted spans certify edit {edit}"
		);
		let spliced = textm::measure_text_spliced(
			&doc, 0, 10.0, 0.0, &full_prev, &new_cps, delta, max_w, wrap, &spans_new, &mut cache,
		)
		.unwrap_or_else(|| panic!("splice rejected valid edit {edit}, wrap={wrap}, rich={rich}"));
		let full_new = textm::measure_text_cached(
			&doc, 0, 10.0, 1.4, 0.0, &new_cps, max_w, wrap, false, -1, &spans_new, &mut cache,
		);
		assert_splice_layout_eq(
			&spliced,
			&full_new,
			&format!("edit {edit}, wrap={wrap}, rich={rich}"),
		);
		text = new_text;
	}
}

/// Differentially checks delta-spliced measurement against a full measure.
pub fn test_splice_differential_wrapped() {
	splice_differential(true, false);
}

/// Differentially checks delta-spliced measurement without soft wrapping.
pub fn test_splice_differential_nowrap() {
	splice_differential(false, false);
}

/// Differentially checks rich-span spliced measurement with soft wrapping.
pub fn test_splice_differential_rich_wrapped() {
	splice_differential(true, true);
}

/// Differentially checks rich-span spliced measurement without soft wrapping.
pub fn test_splice_differential_rich_nowrap() {
	splice_differential(false, true);
}

/// Builds deterministic spans over `text`: leading bold and inner code runs
/// on a line cadence, plus periodic bold ranges crossing a hard boundary.
fn rich_test_spans(text: &str) -> crate::edit::InlineSpans {
	let mut spans = crate::edit::InlineSpans::empty();
	let mut start = 0i32;
	for (line, content) in text.split('\n').enumerate() {
		let end = start.wrapping_add(i32::try_from(content.chars().count()).expect("line fits i32"));
		if line % 3 == 0 {
			spans
				.get_mut(crate::edit::STYLE_BOLD)
				.expect("bold style")
				.0
				.push((start, (start + 8).min(end)));
		}
		if line % 11 == 3 && start >= 4 {
			// Crosses the hard boundary before this line.
			spans
				.get_mut(crate::edit::STYLE_BOLD)
				.expect("bold style")
				.0
				.push((start - 4, (start + 5).min(end)));
		}
		if line % 5 == 0 {
			spans
				.get_mut(crate::edit::STYLE_CODE)
				.expect("code style")
				.0
				.push(((start + 4).min(end), (start + 12).min(end)));
		}
		start = end + 1;
	}
	for style in 0..=crate::edit::STYLE_CODE {
		spans.get_mut(style).expect("known style id").normalize();
	}
	spans
}

/// Checks that `follows_splice` certifies positional shifts and rejects
/// non-positional span changes.
pub fn test_spans_follows_splice() {
	let mut old = crate::edit::InlineSpans::empty();
	old.get_mut(crate::edit::STYLE_BOLD)
		.expect("bold style")
		.0
		.push((2, 6));
	let delta = textm::TextDelta { at: 3, removed: 0, inserted: 2 };
	let mut positional = old.clone();
	positional
		.get_mut(crate::edit::STYLE_BOLD)
		.expect("bold style")
		.insert(3, 2);
	assert!(old.follows_splice(&positional, delta), "positional shift certifies");
	let mut toggled = positional.clone();
	toggled
		.get_mut(crate::edit::STYLE_CODE)
		.expect("code style")
		.0
		.push((0, 1));
	assert!(!old.follows_splice(&toggled, delta), "an extra span range forces a full measure");
}

/// Asserts pins and rich slots form a bijection: every queued pin indexes a
/// distinct in-bounds `Rich` slot, and every `Rich` slot is queued.
fn assert_rich_pins_consistent(layout: &TextLayout, context: &str) {
	let store = layout.shaped.borrow();
	let pins = layout.rich_pins.borrow();
	let mut seen = vec![false; store.len()];
	for &pin in pins.iter() {
		assert!(pin < store.len(), "{context}: pin {pin} within {} lines", store.len());
		assert!(!seen[pin], "{context}: pin {pin} queued once");
		seen[pin] = true;
		assert!(
			matches!(store[pin], textm::LineShape::Rich(_)),
			"{context}: pin {pin} points at a rich slot"
		);
	}
	for (line, slot) in store.iter().enumerate() {
		if matches!(slot, textm::LineShape::Rich(_)) {
			assert!(seen[line], "{context}: rich line {line} is queued");
		}
	}
}

/// Splicing a rich layout rebases the pinned-line FIFO.
///
/// Window pins drop, suffix pins shift with the line delta, and eviction
/// after newline insert/delete splices never unshapes a shifted line or
/// indexes past a shrunk layout.
pub fn test_rich_pins_rebase_across_splice() {
	let doc = font_doc();
	let cache = std::cell::RefCell::new(textm::ShapeCache::default());
	let lines = textm::RICH_PIN_CAP + 40;
	let text = vec!["abc cab"; lines].join("\n");
	let cps: Vec<u32> = text.chars().map(u32::from).collect();
	// Bold the first three codepoints of every 8-codepoint hard line so each
	// line shapes through the rich pin path.
	let mut spans = crate::edit::InlineSpans::empty();
	for line in 0..lines {
		let start = i32::try_from(line * 8).expect("test text fits i32");
		spans
			.get_mut(crate::edit::STYLE_BOLD)
			.expect("bold style")
			.0
			.push((start, start + 3));
	}
	let mut layout = textm::measure_text_cached(
		&doc,
		0,
		10.0,
		1.4,
		0.0,
		&cps,
		10_000.0,
		false,
		false,
		-1,
		&spans,
		&mut cache.borrow_mut(),
	);
	// Pin to the cap; earlier lines evict as later ones shape.
	for line in 0..lines {
		assert!(textm::line_shaped(&doc, &cache, &layout, line).is_some(), "shape line {line}");
	}
	assert_rich_pins_consistent(&layout, "after initial sweep");

	// Insert a newline inside hard line 0: one more line, suffix pins +1.
	let mut grown: Vec<u32> = cps;
	grown.insert(3, 10);
	let grow = textm::TextDelta { at: 3, removed: 0, inserted: 1 };
	let mut spans_grown = spans.clone();
	for style in 0..=crate::edit::STYLE_CODE {
		let ranges = spans_grown.get_mut(style).expect("known style id");
		ranges.delete(grow.at, grow.at);
		ranges.insert(grow.at, grow.inserted);
	}
	assert!(
		textm::measure_text_spliced_into(
			&doc,
			0,
			10.0,
			0.0,
			&mut layout,
			&grown,
			grow,
			10_000.0,
			false,
			&spans_grown,
			&mut cache.borrow_mut(),
		),
		"grow splice applies"
	);
	assert_rich_pins_consistent(&layout, "after newline insert");

	// Delete across three hard boundaries: line count shrinks by three and
	// every suffix pin shifts down past the merged window.
	let mut shrunk: Vec<u32> = grown.clone();
	shrunk.drain(2..20);
	let shrink = textm::TextDelta { at: 2, removed: 18, inserted: 0 };
	let mut spans_shrunk = spans_grown.clone();
	for style in 0..=crate::edit::STYLE_CODE {
		let ranges = spans_shrunk.get_mut(style).expect("known style id");
		ranges.delete(shrink.at, shrink.at.wrapping_add(shrink.removed));
		ranges.insert(shrink.at, 0);
	}
	assert!(
		textm::measure_text_spliced_into(
			&doc,
			0,
			10.0,
			0.0,
			&mut layout,
			&shrunk,
			shrink,
			10_000.0,
			false,
			&spans_shrunk,
			&mut cache.borrow_mut(),
		),
		"shrink splice applies"
	);
	assert_rich_pins_consistent(&layout, "after newline deletes");

	// The respliced window line is unshaped; shaping it at a full FIFO forces
	// an eviction against the rebased pins.
	assert!(textm::line_shaped(&doc, &cache, &layout, 0).is_some(), "reshape window line");
	assert_rich_pins_consistent(&layout, "after post-splice eviction");
}

/// Checks composition of contiguous edit deltas and rejection of disjoint
/// edits.
pub fn test_text_delta_merge() {
	let mut cancelled = textm::TextDelta { at: 4, removed: 0, inserted: 1 };
	assert!(cancelled.merge(textm::TextDelta { at: 4, removed: 1, inserted: 0 }));
	assert_eq!(cancelled, textm::TextDelta { at: 4, removed: 0, inserted: 0 });

	let mut overlap = textm::TextDelta { at: 3, removed: 3, inserted: 2 };
	assert!(overlap.merge(textm::TextDelta { at: 4, removed: 1, inserted: 3 }));
	assert_eq!(overlap, textm::TextDelta { at: 3, removed: 3, inserted: 4 });
	assert!(overlap.merge(textm::TextDelta { at: 2, removed: 2, inserted: 1 }));
	assert_eq!(overlap, textm::TextDelta { at: 2, removed: 4, inserted: 4 });

	let before = overlap;
	assert!(!overlap.merge(textm::TextDelta { at: 20, removed: 1, inserted: 0 }));
	assert_eq!(overlap, before, "a rejected merge leaves the accumulated delta intact");
}

/// Checks that shaped multi-font runs remain in normative FONT-advance space.
pub fn test_multifont_shaped_advance_contract() {
	let mut doc = font_doc();
	doc.font_family.push(0);
	doc.font_class.push(0);
	doc.font_weight.push(700);
	doc.font_upem.push(1000);
	doc.font_ascent.push(800);
	doc.font_descent.push(-200);
	doc.font_line_gap.push(0);
	doc.font_default_adv.push(600);
	doc.font_cmap_off
		.push(i32::try_from(doc.font_cmap_cp.len()).expect("cmap fits i32"));
	doc.font_cmap_len.push(1);
	doc.font_cmap_cp.push(0x2715);
	doc.font_cmap_gid.push(99);
	doc.font_adv.push(700);

	let mut cache = textm::ShapeCache::default();
	let layout = textm::measure_text_cached(
		&doc,
		0,
		10.0,
		1.4,
		0.0,
		&"a✕b\n✕ ab✕".chars().map(u32::from).collect::<Vec<u32>>(),
		200.0,
		false,
		false,
		-1,
		&crate::edit::InlineSpans::empty(),
		&mut cache,
	);
	let cache = std::cell::RefCell::new(cache);
	for line in 0..layout.line_w.len() {
		let shaped = textm::line_shaped(&doc, &cache, &layout, line).expect("line shapes");
		let run_width = shaped.runs.iter().fold(0.0, |sum, run| sum + run.width);
		assert_eq!(
			run_width.to_bits(),
			layout.line_w[line].to_bits(),
			"line {line}: shaped run widths stay in measured advance space"
		);
		let mut visual_end = f64::NEG_INFINITY;
		for cluster in &shaped.clusters {
			assert!(cluster.x0 <= cluster.x1, "line {line}: cluster extent is ordered");
			assert!(
				cluster.x0 >= visual_end,
				"line {line}: visual clusters are monotone and non-overlapping"
			);
			visual_end = cluster.x1;
		}
	}
}
