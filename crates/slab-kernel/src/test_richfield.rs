//! Rich-field span, editing, layout, and host round-trip tests.

use crate::{
	dispatch, edit,
	flatten::FrameOp,
	frame::{self, FieldRun, FieldRuns, ParamValue},
	textm,
};

fn field() -> frame::Instance {
	crate::test_edit::host_field_instance()
}

fn runs(revision: u64, entries: &[(u32, i32, i32)]) -> FieldRuns {
	FieldRuns {
		revision,
		runs: entries
			.iter()
			.map(|&(style, start, end)| FieldRun { style, start, end })
			.collect(),
	}
}

fn split_runs(source: &FieldRuns, at: i32) -> (FieldRuns, FieldRuns) {
	let mut left = Vec::new();
	let mut right = Vec::new();
	for run in &source.runs {
		if run.start < at {
			left.push(FieldRun { style: run.style, start: run.start, end: run.end.min(at) });
		}
		if run.end > at {
			right.push(FieldRun {
				style: run.style,
				start: run.start.max(at) - at,
				end:   run.end - at,
			});
		}
	}
	(FieldRuns { revision: source.revision, runs: left }, FieldRuns {
		revision: source.revision,
		runs:     right,
	})
}

/// A host can read, split, and write two independently normalized run sets.
pub fn test_host_split_round_trip_preserves_runs() {
	let mut source = field();
	assert!(frame::inst_set_field_text(&mut source, "field-key", "abcdef"));
	assert!(frame::inst_set_caret(&mut source, "field-key", 5, 1));
	assert!(frame::inst_toggle_style(&mut source, "field-key", edit::STYLE_BOLD));
	assert!(frame::inst_set_caret(&mut source, "field-key", 4, 2));
	assert!(frame::inst_toggle_style(&mut source, "field-key", edit::STYLE_CODE));
	let source_runs = frame::inst_field_runs(&source, "field-key").expect("source runs");
	let (left_runs, right_runs) = split_runs(&source_runs, 3);

	let mut left = field();
	assert!(frame::inst_set_field_text(&mut left, "field-key", "abc"));
	assert!(frame::inst_set_field_runs(&mut left, "field-key", &left_runs));
	let mut right = field();
	assert!(frame::inst_set_field_text(&mut right, "field-key", "def"));
	assert!(frame::inst_set_field_runs(&mut right, "field-key", &right_runs));

	assert_eq!(
		frame::inst_field_runs(&left, "field-key"),
		Some(runs(2, &[(edit::STYLE_BOLD, 1, 3), (edit::STYLE_CODE, 2, 3)]))
	);
	assert_eq!(
		frame::inst_field_runs(&right, "field-key"),
		Some(runs(2, &[(edit::STYLE_BOLD, 0, 2), (edit::STYLE_CODE, 0, 1)]))
	);
}

/// Range-tail typing inherits style, while head typing shifts it away.
pub fn test_typing_span_boundary_semantics() {
	for (caret, expected) in [(3, (1, 4)), (1, (2, 4)), (2, (1, 4))] {
		let mut state = edit::es_new(0, "abcd");
		state.spans.bold.0.push((1, 3));
		state.caret = caret;
		state.anchor = caret;
		assert!(edit::insert(&mut state, "x"));
		assert_eq!(state.spans.bold.0, [expected]);
	}
}

/// Fully covered selections remove style; partial selections extend it.
pub fn test_toggle_covered_removes_partial_extends() {
	let mut ranges = edit::Ranges(vec![(1, 4)]);
	ranges.toggle(2, 3);
	assert_eq!(ranges.0, [(1, 2), (3, 4)]);
	let mut partial = edit::Ranges(vec![(1, 3)]);
	partial.toggle(2, 5);
	assert_eq!(partial.0, [(1, 5)]);
}

/// Undo and redo restore text and all style spans atomically.
pub fn test_styled_delete_undo_redo() {
	let mut state = edit::es_new(0, "abcdef");
	state.spans.bold.0.push((1, 5));
	state.spans.code.0.push((2, 4));
	state.anchor = 2;
	state.caret = 4;
	assert!(edit::backspace(&mut state));
	assert_eq!(state.text, "abef");
	assert_eq!(state.spans.bold.0, [(1, 3)]);
	assert!(state.spans.code.0.is_empty());
	assert!(edit::undo(&mut state));
	assert_eq!(state.text, "abcdef");
	assert_eq!(state.spans.bold.0, [(1, 5)]);
	assert_eq!(state.spans.code.0, [(2, 4)]);
	assert!(edit::redo(&mut state));
	assert_eq!(state.text, "abef");
	assert_eq!(state.spans.bold.0, [(1, 3)]);
	assert!(state.spans.code.0.is_empty());
}

fn register_metric_font(
	instance: &mut frame::Instance,
	family: &str,
	weight: u32,
	advance: u32,
) -> i32 {
	frame::inst_font_register(
		instance,
		family,
		weight,
		1000,
		800,
		-200,
		0,
		-100,
		50,
		advance,
		&[],
		&[],
		&[],
		&[],
	)
}

/// Field span boundaries lower to independently styled shaped TEXT operations.
pub fn test_layout_emits_styled_segments_with_total_advance() {
	let mut instance = field();
	let regular = register_metric_font(&mut instance, "Test Sans", 400, 500);
	let bold = register_metric_font(&mut instance, "Test Sans", 700, 600);
	let mono = register_metric_font(&mut instance, "Test Mono", 400, 700);
	assert_eq!((regular, bold, mono), (0, 1, 2));
	assert!(frame::inst_set_field_text(&mut instance, "field-key", "abcd"));
	assert!(frame::inst_set_field_runs(
		&mut instance,
		"field-key",
		&runs(0, &[(edit::STYLE_BOLD, 0, 2), (edit::STYLE_CODE, 2, 4)]),
	));
	let painted = frame::inst_frame(&mut instance, 0.0);
	let text_ops: Vec<_> = painted
		.ops
		.iter()
		.filter_map(|op| match op {
			FrameOp::Text(text) => Some(text),
			_ => None,
		})
		.collect();
	assert_eq!(text_ops.len(), 2, "one TEXT op per rich segment");
	assert_eq!(text_ops[0].weight, 700);
	assert_eq!(text_ops[0].font, bold);
	assert_eq!(text_ops[1].font, mono);
	let expected: f64 = text_ops
		.iter()
		.map(|op| {
			let text = &painted.strings[usize::try_from(op.str_ref).expect("string ref")];
			textm::str_slice_w(
				instance.doc(),
				op.font,
				op.size,
				op.tracking,
				text,
				0,
				crate::rt::str_len(text),
			)
		})
		.sum();
	let actual: f64 = text_ops.iter().map(|op| op.measured_w).sum();
	assert_eq!(actual, expected);
	assert_eq!(instance.lay.tls[0].line_w[0], actual);

	let spans = instance.ds.ed[0].spans.clone();
	let size = text_ops[0].size;
	let tracking = text_ops[0].tracking;
	let mut wrapped = textm::measure_text(
		instance.doc(),
		regular,
		size,
		1.2,
		tracking,
		"abcd",
		20.0,
		true,
		false,
		-1,
	);
	let mut shape_cache = textm::ShapeCache::default();
	textm::rewrap_rich_layout(
		instance.doc(),
		regular,
		size,
		tracking,
		"abcd",
		20.0,
		-1,
		&spans,
		&mut wrapped,
		&mut shape_cache,
	);
	textm::shape_rich_layout(
		instance.doc(),
		regular,
		size,
		tracking,
		&spans,
		&mut wrapped,
		&mut shape_cache,
	);
	assert_eq!(wrapped.src_le, [2, 4], "rich advances drive wrapping");
	assert!(wrapped.line_w.iter().all(|width| *width <= 20.0));
}

fn code_paint_field(code_color: u32, code_bg: u32) -> frame::Instance {
	let mut instance = field();
	let color_value = u32::try_from(instance.doc.aval_tag.len()).expect("attribute value count");
	let bg_value = color_value + 1;
	instance
		.doc
		.aval_tag
		.extend([crate::slir::T_COLOR, crate::slir::T_COLOR]);
	instance.doc.aval_lo.extend([code_color, code_bg]);
	instance.doc.aval_hi.extend([0, 0]);
	instance.doc.aval_num.extend([0.0, 0.0]);
	instance
		.doc
		.attr_id
		.extend([crate::slir::A_CODE_COLOR, crate::slir::A_CODE_BG]);
	instance.doc.attr_val.extend([color_value, bg_value]);
	instance.doc.attr_index[1] = 3;
	frame::inst_init(&mut instance);
	frame::inst_set_env(&mut instance, 500.0, 500.0, 0, false, false);
	frame::inst_frame(&mut instance, 0.0);
	instance
}

/// Editable code spans use author-supplied text and background paints only on
/// code runs, including text inherited by typing at the styled tail.
pub fn test_code_run_paints_color_and_background() {
	let code_color = 0xff55_22ff;
	let code_bg = 0x3322_11ff;
	let mut styled = code_paint_field(code_color, code_bg);
	assert!(frame::inst_set_field_text(&mut styled, "field-key", "ab"));
	frame::inst_take_signals(&mut styled);
	assert!(frame::inst_set_caret(&mut styled, "field-key", 1, 0));
	assert!(frame::inst_toggle_style(&mut styled, "field-key", edit::STYLE_CODE));
	assert!(frame::inst_set_caret(&mut styled, "field-key", 1, 1));
	frame::inst_dispatch(
		&mut styled,
		&crate::test_edit::host_field_event(dispatch::E_TEXT, "", "x"),
	);
	let painted = frame::inst_frame(&mut styled, 0.0);
	assert_eq!(
		(styled.st.rs[0].code_color, styled.st.rs[0].code_color_kind),
		(code_color, 1),
		"code-color resolves on the field rule",
	);
	let text_index = painted
		.ops
		.iter()
		.position(
			|op| matches!(op, FrameOp::Text(text) if text.color == code_color && text.str_ref >= 0),
		)
		.expect("code TEXT uses code-color after tail typing");
	let background_index = painted
		.ops
		.iter()
		.position(
			|op| matches!(op, FrameOp::Rect(rect) if rect.bg == code_bg && rect.bg_kind == 1 && rect.w > 0.0),
		)
		.expect("code run emits its background rectangle");
	assert!(background_index < text_index, "code background paints before its glyphs");

	let mut plain = code_paint_field(code_color, code_bg);
	assert!(frame::inst_set_field_text(&mut plain, "field-key", "ab"));
	let plain_frame = frame::inst_frame(&mut plain, 0.0);
	assert!(
		plain_frame
			.ops
			.iter()
			.all(|op| !matches!(op, FrameOp::Text(text) if text.color == code_color)),
		"non-code text keeps the node color",
	);
	assert!(
		plain_frame
			.ops
			.iter()
			.all(|op| !matches!(op, FrameOp::Rect(rect) if rect.bg == code_bg)),
		"non-code text emits no code background",
	);
}

/// Change signals carry the new revision; host parameter sync does not bump it.
pub fn test_change_signal_runs_revision_and_param_reset() {
	let mut instance = field();
	assert!(frame::inst_set_field_text(&mut instance, "field-key", "ab"));
	frame::inst_take_signals(&mut instance);
	assert!(frame::inst_set_caret(&mut instance, "field-key", 2, 0));
	assert!(frame::inst_toggle_style(&mut instance, "field-key", edit::STYLE_BOLD));
	frame::inst_take_signals(&mut instance);
	assert!(frame::inst_set_caret(&mut instance, "field-key", 2, 2));
	let effect = frame::inst_dispatch(
		&mut instance,
		&crate::test_edit::host_field_event(dispatch::E_TEXT, "", "x"),
	);
	assert_eq!(effect.sig_text, ["abx"]);
	assert_eq!(effect.sig_runs, ["{\"rev\":3,\"runs\":[{\"style\":0,\"start\":0,\"end\":3}]}"],);
	let before = frame::inst_field_runs(&instance, "field-key")
		.expect("runs")
		.revision;
	assert!(frame::inst_set_param(&mut instance, 0, &ParamValue {
		kind: 0,
		num:  0.0,
		s:    "host".into(),
		rgba: 0,
		sym:  String::new(),
	},));
	assert_eq!(
		frame::inst_field_runs(&instance, "field-key")
			.expect("runs")
			.revision,
		before,
		"host parameter synchronization is not a local committed edit",
	);
}

fn composition_event(text: &str, clauses: &[(i32, i32)]) -> dispatch::Event {
	let mut event = crate::test_edit::host_field_event(dispatch::E_COMPOSITION_UPDATE, "", text);
	event.clauses.extend_from_slice(clauses);
	event
}

fn painted_preedit(clauses: &[(i32, i32)]) -> (frame::Instance, crate::flatten::Frame) {
	let mut instance = field();
	assert!(frame::inst_set_field_text(&mut instance, "field-key", ""));
	assert!(frame::inst_set_focus(&mut instance, "field-key", true));
	frame::inst_dispatch(&mut instance, &composition_event("にほんご", clauses));
	let painted = frame::inst_frame(&mut instance, 0.0);
	(instance, painted)
}

/// Adjacent IME clauses paint as distinct font-derived underline rectangles.
pub fn test_composition_clauses_paint_distinct_underlines() {
	let (instance, painted) = painted_preedit(&[(0, 2), (2, 4)]);
	let text = painted
		.ops
		.iter()
		.find_map(|op| match op {
			FrameOp::Text(text) => Some(text),
			_ => None,
		})
		.expect("preedit text operation");
	let underlines: Vec<_> = painted
		.ops
		.iter()
		.filter_map(|op| match op {
			FrameOp::Rect(rect)
				if rect.bg == text.color
					&& (rect.h - text.underline_thickness).abs() < f64::EPSILON =>
			{
				Some(rect)
			},
			_ => None,
		})
		.collect();
	assert_eq!(underlines.len(), 2, "one underline rectangle per adjacent clause");
	let layout = &instance.lay.tls[0];
	let expected = [
		(textm::caret_x(layout, 0, 0), textm::caret_x(layout, 0, 2)),
		(textm::caret_x(layout, 0, 2), textm::caret_x(layout, 0, 4)),
	];
	for (underline, (start, end)) in underlines.iter().zip(expected) {
		assert!((underline.x - (text.x + start.min(end))).abs() < 0.001);
		assert!((underline.w - (end - start).abs()).abs() < 0.001);
	}
}

/// Missing or single-clause metadata degrades to one whole-preedit underline.
pub fn test_composition_clause_fallback_underlines_whole_preedit() {
	for clauses in [&[][..], &[(1, 2)][..]] {
		let (instance, painted) = painted_preedit(clauses);
		let text = painted
			.ops
			.iter()
			.find_map(|op| match op {
				FrameOp::Text(text) => Some(text),
				_ => None,
			})
			.expect("preedit text operation");
		let underlines: Vec<_> = painted
			.ops
			.iter()
			.filter_map(|op| match op {
				FrameOp::Rect(rect)
					if rect.bg == text.color
						&& (rect.h - text.underline_thickness).abs() < f64::EPSILON =>
				{
					Some(rect)
				},
				_ => None,
			})
			.collect();
		assert_eq!(underlines.len(), 1, "fallback has one underline");
		let layout = &instance.lay.tls[0];
		let width = (textm::caret_x(layout, 0, 4) - textm::caret_x(layout, 0, 0)).abs();
		assert!((underlines[0].w - width).abs() < 0.001, "fallback covers full preedit");
	}
}

/// Composition end commits through dispatch and removes all marked-text state.
pub fn test_composition_end_clears_clause_overlay() {
	let (mut instance, _) = painted_preedit(&[(0, 2), (2, 4)]);
	let end = crate::test_edit::host_field_event(dispatch::E_COMPOSITION_END, "", "日本語");
	frame::inst_dispatch(&mut instance, &end);
	let state = &instance.ds.ed[0];
	assert_eq!(state.text, "日本語");
	assert!(!state.composing);
	assert!(state.compose.is_empty());
	assert!(state.compose_clauses.is_empty());
}

/// Clause ranges clamp to preedit bounds without merging adjacent clauses.
pub fn test_composition_clause_ranges_clamp() {
	let mut state = edit::es_new(0, "");
	assert!(!edit::composition_update_clauses(&mut state, "abc", &[(-4, 2), (2, 99), (8, 9)]));
	assert_eq!(state.compose_clauses, [(0, 2), (2, 3)]);
}

/// IME replacement uses the same splice algebra as keyboard input.
pub fn test_composition_commit_preserves_spans() {
	let mut state = edit::es_new(0, "abc");
	state.spans.bold.0.push((0, 2));
	state.anchor = 1;
	state.caret = 2;
	assert!(edit::composition_update_clauses(&mut state, "か", &[(0, 1)]));
	assert_eq!(state.spans.bold.0, [(0, 1)]);
	assert_eq!(state.compose_clauses, [(0, 1)]);
	assert!(edit::composition_end(&mut state, "漢"));
	assert!(state.compose_clauses.is_empty(), "commit clears clause overlay");
	assert_eq!(state.text, "a漢c");
	assert_eq!(state.spans.bold.0, [(0, 2)]);
}
