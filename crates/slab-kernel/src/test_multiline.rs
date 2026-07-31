//! Multiline dispatch integration tests covering Enter routing, visual
//! navigation, caret geometry, edit history, and caret-follow scrolling.

use crate::{
	dispatch::{self, DState, Effects, Event},
	edit,
	layout::{self, Lay},
	scene::{self, Scene},
	slir::{self, Doc},
	style::{self, St},
	textm,
};

/// Complete document, layout, scene, style, and dispatch state for one field.
#[derive(Clone, Debug)]
pub struct Fix {
	/// Minimal document containing the editable text node.
	pub d:   Doc,
	/// Mutable style and field state.
	pub st:  St,
	/// Measured text layout.
	pub lay: Lay,
	/// Solved scene geometry.
	pub sc:  Scene,
	/// Focus, editing, and dispatch state.
	pub ds:  DState,
}

/// Appends a text node with the supplied geometry to a test scene.
pub fn add_scene(
	sc: &mut Scene,
	node: u32,
	parent: i32,
	x: f64,
	y: f64,
	w: f64,
	h: f64,
	flags: u32,
) {
	sc.entries.push(crate::flatten::SceneNode {
		node,
		parent_ix: parent,
		kind: slir::K_TEXT,
		x,
		y,
		w,
		h,
		flags,
		src_line: 1,
		authored_order: node,
		..Default::default()
	});
}

/// Creates an empty fixture from fresh document, style, layout, scene, and
/// dispatch state.
pub fn fix_new() -> Fix {
	Fix {
		d:   slir::doc_new(),
		st:  style::st_new(),
		lay: layout::lay_new(),
		sc:  scene::scene_new(),
		ds:  dispatch::dstate_new(),
	}
}

/// Populates a fixture with one focused editable text node.
pub fn fill(f: &mut Fix, text: &str, multi: bool, submit: bool, width: f64) {
	f.d.strs
		.extend([String::new(), "change".into(), "submit".into()]);
	f.d.node_kind.push(slir::K_TEXT);
	let flags = slir::F_FOCUSABLE | if multi { slir::F_MULTILINE } else { 0 };
	f.d.node_flags.push(flags);
	f.d.node_parent.push(slir::NONE);
	f.d.node_first.push(slir::NONE);
	f.d.node_next.push(slir::NONE);
	f.d.node_key.push(0);
	f.d.node_id.push(0);
	f.d.node_line.push(1);
	f.d.attr_index.extend([0, 0]);
	f.d.sign_name.push(1);
	f.d.sign_node.push(0);
	f.d.sign_trigger.push(1);
	if submit {
		f.d.sign_name.push(2);
		f.d.sign_node.push(0);
		f.d.sign_trigger.push(2);
	}
	f.lay.tls.push(std::rc::Rc::new(textm::measure_text(
		&f.d, -1, 14.0, 1.2, 0.0, text, width, multi, false, -1,
	)));
	f.lay.p_node.push(0);
	f.lay.p_tl.push(0);
	add_scene(&mut f.sc, 0, -1, 10.0, 20.0, width, 16.8, flags);
	f.ds.fs.focus = 0;
	f.ds.ed_node.push(0);
	f.ds.ed.push(edit::es_new(0, text));
}
fn set_tab_size(f: &mut Fix, size: f64) {
	let value = u32::try_from(f.d.aval_tag.len()).expect("fixture value count");
	f.d.aval_tag.push(slir::T_NUM);
	f.d.aval_lo.push(0);
	f.d.aval_hi.push(0);
	f.d.aval_num.push(size);
	f.d.attr_id.push(slir::A_TAB_SIZE);
	f.d.attr_val.push(value);
	f.d.attr_index[1] = i32::try_from(f.d.attr_id.len()).expect("fixture attr count");
}

fn add_focus_target(f: &mut Fix) {
	f.d.node_kind.push(slir::K_TEXT);
	f.d.node_flags.push(slir::F_FOCUSABLE);
	f.d.node_parent.push(slir::NONE);
	f.d.node_first.push(slir::NONE);
	f.d.node_next.push(slir::NONE);
	f.d.node_key.push(0);
	f.d.node_id.push(0);
	f.d.node_line.push(1);
	f.d.attr_index
		.push(i32::try_from(f.d.attr_id.len()).expect("fixture attr count"));
	add_scene(&mut f.sc, 1, -1, 0.0, 50.0, 100.0, 20.0, slir::F_FOCUSABLE);
}

/// Constructs a keyboard or text event with neutral pointer fields.
pub fn event(etype: u32, key: &str, text: &str, mods: u32) -> Event {
	Event {
		etype,
		x: 0.0,
		y: 0.0,
		dx: 0.0,
		dy: 0.0,
		button: 0,
		clicks: 0,
		key: key.into(),
		text: text.into(),
		clauses: Vec::new(),
		mods,
	}
}

/// Dispatches an event through a fixture and returns its effects.
pub fn send(f: &mut Fix, ev: &Event) -> Effects {
	dispatch::dispatch(&f.d, &mut f.st, &f.lay, &f.sc, &mut f.ds, ev)
}
fn pointer_at(f: &Fix, etype: u32, line: usize, at: i32) -> Event {
	let text_layout = &f.lay.tls[0];
	let mut ev = event(etype, "", "", 0);
	ev.x = f.sc.entries[0].x
		+ textm::caret_x(textm::Shaper { d: &f.d, cache: &f.lay.shape_cache }, text_layout, line, at);
	ev.y = (line as f64 + 0.5).mul_add(text_layout.line_h, f.sc.entries[0].y);
	ev.clicks = 1;
	ev
}

fn drag(f: &mut Fix, from_line: usize, from: i32, to_line: usize, to: i32) {
	let down = pointer_at(f, dispatch::E_POINTER_DOWN, from_line, from);
	send(f, &down);
	let moved = pointer_at(f, dispatch::E_POINTER_MOVE, to_line, to);
	send(f, &moved);
	let up = pointer_at(f, dispatch::E_POINTER_UP, to_line, to);
	send(f, &up);
}

/// Verifies opt-in Tab insertion, history, Change delivery, and traversal
/// fallbacks.
pub fn test_tab_size_insertion_and_traversal() {
	let mut opted_in = fix_new();
	fill(&mut opted_in, "ab", true, false, 100.0);
	set_tab_size(&mut opted_in, 3.0);
	add_focus_target(&mut opted_in);
	opted_in.ds.ed[0].caret = 1;
	opted_in.ds.ed[0].anchor = 1;

	let inserted = send(&mut opted_in, &event(dispatch::E_KEY_DOWN, "Tab", "", 0));
	assert_eq!(edit::text_str(&opted_in.ds.ed[0]), "a   b", "Tab inserts exactly tab-size spaces");
	assert_eq!(opted_in.ds.fs.focus, 0, "Tab insertion retains field focus");
	assert_eq!(inserted.sig_name, vec![1], "Tab insertion emits Change");
	assert_eq!(inserted.sig_text, vec!["a   b"], "Change carries the edited text");

	let undone = send(&mut opted_in, &event(dispatch::E_KEY_DOWN, "z", "", dispatch::M_META));
	assert_eq!(edit::text_str(&opted_in.ds.ed[0]), "ab", "one undo removes the Tab insertion");
	assert_eq!(undone.sig_text, vec!["ab"], "undo emits Change with restored text");

	let mut shifted = fix_new();
	fill(&mut shifted, "ab", true, false, 100.0);
	set_tab_size(&mut shifted, 3.0);
	add_focus_target(&mut shifted);
	send(&mut shifted, &event(dispatch::E_KEY_DOWN, "Tab", "", dispatch::M_SHIFT));
	assert_eq!(shifted.ds.fs.focus, 1, "Shift+Tab keeps backward focus traversal");
	assert_eq!(edit::text_str(&shifted.ds.ed[0]), "ab", "Shift+Tab does not edit");

	let mut default_field = fix_new();
	fill(&mut default_field, "ab", true, false, 100.0);
	add_focus_target(&mut default_field);
	send(&mut default_field, &event(dispatch::E_KEY_DOWN, "Tab", "", 0));
	assert_eq!(default_field.ds.fs.focus, 1, "a field without tab-size keeps Tab traversal");
	assert_eq!(edit::text_str(&default_field.ds.ed[0]), "ab", "default Tab does not edit");

	let mut single_line = fix_new();
	fill(&mut single_line, "ab", false, false, 100.0);
	set_tab_size(&mut single_line, 3.0);
	add_focus_target(&mut single_line);
	send(&mut single_line, &event(dispatch::E_KEY_DOWN, "Tab", "", 0));
	assert_eq!(single_line.ds.fs.focus, 1, "single-line tab-size keeps Tab traversal");
	assert_eq!(edit::text_str(&single_line.ds.ed[0]), "ab", "single-line Tab does not edit");
}

/// Verifies the multiline, submit-bound, modified, and single-line Enter
/// matrix.
pub fn test_enter_matrix_and_submit_payload() {
	let mut a = fix_new();
	fill(&mut a, "a", true, false, 100.0);
	let ea = send(&mut a, &event(dispatch::E_KEY_DOWN, "Enter", "", 0));
	assert_eq!(edit::text_str(&a.ds.ed[0]), "a\n", "multiline Enter inserts newline");
	assert!(ea.sig_name.len() == 1 && ea.sig_name[0] == 1, "newline emits Change");

	let mut b = fix_new();
	fill(&mut b, "draft", true, true, 100.0);
	let eb = send(&mut b, &event(dispatch::E_KEY_DOWN, "Enter", "", 0));
	assert_eq!(edit::text_str(&b.ds.ed[0]), "draft", "plain Enter with submit does not insert");
	assert!(eb.sig_name.len() == 1 && eb.sig_name[0] == 2, "trigger 2 emitted");
	assert_eq!(eb.sig_text[0], "draft", "Submit carries committed text");
	assert!(eb.sig_item.len() == 1 && eb.sig_item[0].is_empty(), "real Submit item empty");

	let mut c = fix_new();
	fill(&mut c, "a", true, true, 100.0);
	let ec = send(&mut c, &event(dispatch::E_KEY_DOWN, "Enter", "", dispatch::M_SHIFT));
	assert_eq!(edit::text_str(&c.ds.ed[0]), "a\n", "Shift+Enter inserts with submit binding");
	assert!(ec.sig_name.len() == 1 && ec.sig_name[0] == 1, "modified Enter emits Change only");

	let mut ca = fix_new();
	fill(&mut ca, "a", true, true, 100.0);
	let eca = send(&mut ca, &event(dispatch::E_KEY_DOWN, "Enter", "", dispatch::M_ALT));
	assert_eq!(edit::text_str(&ca.ds.ed[0]), "a\n", "Alt+Enter inserts with submit binding");
	assert!(eca.sig_name.len() == 1 && eca.sig_name[0] == 1, "Alt+Enter emits Change only");

	let mut d = fix_new();
	fill(&mut d, "one", false, true, 100.0);
	let ed = send(&mut d, &event(dispatch::E_KEY_DOWN, "Enter", "", 0));
	assert!(ed.sig_name.len() == 1 && ed.sig_name[0] == 2, "single-line submit fires");

	let mut e = fix_new();
	fill(&mut e, "one", false, false, 100.0);
	let ee = send(&mut e, &event(dispatch::E_KEY_DOWN, "Enter", "", 0));
	assert!(
		edit::text_str(&e.ds.ed[0]) == "one" && ee.sig_name.is_empty() && !ee.repaint,
		"single no-submit Enter is fully inert"
	);
}

/// Verifies that single-line input maps incoming newlines to spaces.
pub fn test_single_line_text_prefilters_newlines() {
	let mut f = fix_new();
	fill(&mut f, "a", false, false, 100.0);
	send(&mut f, &event(dispatch::E_TEXT, "", "\nb", 0));
	assert_eq!(edit::text_str(&f.ds.ed[0]), "a b", "single line maps newline to space");
}

/// Verifies visual arrow, Home, and End movement and caret geometry.
pub fn test_visual_arrows_home_end_and_caret_geometry() {
	let mut f = fix_new();
	fill(&mut f, "ab\ncde", true, false, 100.0);
	let up = send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowUp", "", 0));
	assert_eq!(f.ds.ed[0].caret, 2, "Up preserves goal x and reaches prior visual line end");
	assert!((up.caret_x - 26.8).abs() < 0.01, "caret x is line-relative");
	assert!(
		(up.caret_y - 20.0).abs() < 0.01 && (up.caret_h - 16.8).abs() < 0.01,
		"caret y/h use visual line"
	);
	assert!(up.has_ime && up.ime_x == up.caret_x && up.ime_y == up.caret_y, "IME rect equals caret");
	send(&mut f, &event(dispatch::E_KEY_DOWN, "Home", "", 0));
	assert_eq!(f.ds.ed[0].caret, 0, "Home reaches visual line start");
	send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowDown", "", 0));
	assert_eq!(f.ds.ed[0].caret, 3, "Down reaches next visual line at goal x");
	send(&mut f, &event(dispatch::E_KEY_DOWN, "End", "", 0));
	assert_eq!(f.ds.ed[0].caret, 6, "End reaches visual line end");
}

/// Verifies that caret geometry includes text padding and alignment.
pub fn test_caret_geometry_honors_padding_and_alignment() {
	let mut f = fix_new();
	fill(&mut f, "ab", true, false, 100.0);
	let ri = style::build_rstyle(
		&f.d, &mut f.st, 0, 255, false, 0, 1, 0, 14.0, 400.0, 1.2, 0.0, false, false, false,
	);
	let ri = usize::try_from(ri).expect("render style index must be non-negative");
	f.st.rs[ri].pad_l = 10.0;
	f.st.rs[ri].pad_r = 20.0;
	f.st.rs[ri].pad_t = 5.0;
	f.st.rs[ri].talign = 2;
	let eff = send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowLeft", "", 0));
	assert!((eff.caret_x - 81.6).abs() < 0.01, "right-aligned padded caret x");
	assert!((eff.caret_y - 25.0).abs() < 0.01, "padded caret y");
}

/// Verifies routed word and line kills together with undo and redo.
pub fn test_kills_undo_and_redo() {
	let mut f = fix_new();
	fill(&mut f, "ab cd", true, false, 100.0);
	send(&mut f, &event(dispatch::E_KEY_DOWN, "Home", "", 0));
	send(&mut f, &event(dispatch::E_KEY_DOWN, "Delete", "", dispatch::M_CTRL));
	assert_eq!(edit::text_str(&f.ds.ed[0]), "cd", "Ctrl+Delete removes word forward");
	send(&mut f, &event(dispatch::E_KEY_DOWN, "z", "", dispatch::M_CTRL));
	assert_eq!(edit::text_str(&f.ds.ed[0]), "ab cd", "undo restores deletion");
	send(&mut f, &event(dispatch::E_KEY_DOWN, "Z", "", dispatch::M_CTRL | dispatch::M_SHIFT));
	assert_eq!(edit::text_str(&f.ds.ed[0]), "cd", "redo restores deletion");

	let mut k = fix_new();
	fill(&mut k, "ab\ncd", true, false, 100.0);
	send(&mut k, &event(dispatch::E_KEY_DOWN, "u", "", dispatch::M_CTRL));
	assert_eq!(edit::text_str(&k.ds.ed[0]), "ab\n", "Ctrl+U kills to visual line start");
	send(&mut k, &event(dispatch::E_KEY_DOWN, "z", "", dispatch::M_CTRL));
	send(&mut k, &event(dispatch::E_KEY_DOWN, "Home", "", 0));
	send(&mut k, &event(dispatch::E_KEY_DOWN, "k", "", dispatch::M_CTRL));
	assert_eq!(edit::text_str(&k.ds.ed[0]), "ab\n", "Ctrl+K kills to visual line end");
}

/// Verifies that `E_PASTE` inserts wholesale and undoes as one step
/// (the paste-side history barrier keeps it out of the typing run).
pub fn test_paste_undoes_in_one_step() {
	let mut f = fix_new();
	fill(&mut f, "", true, false, 100.0);
	send(&mut f, &event(dispatch::E_TEXT, "", "ab", 0));
	send(&mut f, &event(dispatch::E_PASTE, "", "one\ntwo", 0));
	assert_eq!(edit::text_str(&f.ds.ed[0]), "abone\ntwo", "paste inserts both lines wholesale");
	send(&mut f, &event(dispatch::E_KEY_DOWN, "z", "", dispatch::M_CTRL));
	assert_eq!(edit::text_str(&f.ds.ed[0]), "ab", "one undo restores the pre-paste text");
}

/// Verifies horizontal field scrolling and nearest-ancestor scroll following.
pub fn test_horizontal_and_ancestor_scroll_follow() {
	let mut f = fix_new();
	fill(&mut f, "abcdefgh", false, false, 20.0);
	send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowLeft", "", 0));
	assert!(f.ds.ed[0].scroll_x > 0.0, "narrow single line follows caret horizontally");
	assert_eq!(
		style::field_scroll_x(&f.st, 0),
		f.ds.ed[0].scroll_x,
		"flatten scroll mirror updated"
	);

	let mut g = fix_new();
	fill(&mut g, "a\nb", true, false, 100.0);
	g.d.node_kind.push(slir::K_COL);
	g.d.node_flags.push(slir::F_SCROLL);
	g.d.node_parent.push(slir::NONE);
	g.d.node_first.push(slir::NONE);
	g.d.node_next.push(slir::NONE);
	g.d.node_key.push(0);
	g.d.node_id.push(0);
	g.d.node_line.push(1);
	g.d.attr_index.push(0);
	g.sc.entries[0].parent_ix = 1;
	g.sc.entries[0].y = 30.0;
	add_scene(&mut g.sc, 1, -1, 0.0, 0.0, 100.0, 20.0, slir::F_SCROLL | slir::F_CLIP);
	g.sc.entries[1].content_main = 100.0;
	send(&mut g, &event(dispatch::E_KEY_DOWN, "ArrowLeft", "", 0));
	assert!(style::scroll_get(&g.st, 1) > 0.0, "nearest scroll ancestor follows multiline caret");
}

/// Verifies that fresh wrapped layout performs the settling caret-follow solve.
pub fn test_fresh_wrapped_layout_scroll_follow_settles() {
	let mut f = fix_new();
	fill(&mut f, "a", true, false, 10.0);
	f.d.node_kind.push(slir::K_COL);
	f.d.node_flags.push(slir::F_SCROLL);
	f.d.node_parent.push(slir::NONE);
	f.d.node_first.push(slir::NONE);
	f.d.node_next.push(slir::NONE);
	f.d.node_key.push(0);
	f.d.node_id.push(0);
	f.d.node_line.push(1);
	f.d.attr_index.push(0);
	f.sc.entries[0].parent_ix = 1;
	f.sc.entries[0].x = 0.0;
	f.sc.entries[0].y = 0.0;
	add_scene(&mut f.sc, 1, -1, 0.0, 0.0, 20.0, 16.8, slir::F_SCROLL | slir::F_CLIP);
	f.sc.entries[1].content_main = 100.0;
	send(&mut f, &event(dispatch::E_TEXT, "", "bc", 0));
	assert_eq!(style::scroll_get(&f.st, 1), 0.0, "stale one-line layout does not predict wrap");
	f.lay.tls[0] = std::rc::Rc::new(textm::measure_text(
		&f.d,
		-1,
		14.0,
		1.2,
		0.0,
		&edit::display_text(&f.ds.ed[0]).to_utf8(),
		10.0,
		true,
		false,
		-1,
	));
	assert!(
		dispatch::follow_caret_fresh(&f.d, &mut f.st, &f.lay, &f.sc, &mut f.ds),
		"fresh wrapped layout requests settle solve"
	);
	assert!(style::scroll_get(&f.st, 1) > 0.0, "fresh wrapped caret scrolls ancestor");
}
/// Primary field drags retain their down-caret anchor, replace the selected
/// span on typing, and leave sub-threshold clicks collapsed.
pub fn test_pointer_drag_selects_replaces_and_preserves_plain_click() {
	let mut forward = fix_new();
	fill(&mut forward, "abcdef", false, false, 120.0);
	drag(&mut forward, 0, 1, 0, 4);
	assert_eq!(
		(forward.ds.ed[0].anchor, forward.ds.ed[0].caret),
		(1, 4),
		"forward drag selects from the press caret to the move caret"
	);
	let stray_move = pointer_at(&forward, dispatch::E_POINTER_MOVE, 0, 0);
	send(&mut forward, &stray_move);
	assert_eq!(
		(forward.ds.ed[0].anchor, forward.ds.ed[0].caret),
		(1, 4),
		"pointer-up ends capture without collapsing the retained selection"
	);
	send(&mut forward, &event(dispatch::E_TEXT, "", "X", 0));
	assert_eq!(edit::text_str(&forward.ds.ed[0]), "aXef", "typing replaces the dragged span");

	let mut reverse = fix_new();
	fill(&mut reverse, "abcdef", false, false, 120.0);
	drag(&mut reverse, 0, 4, 0, 1);
	assert_eq!(
		(reverse.ds.ed[0].anchor, reverse.ds.ed[0].caret),
		(4, 1),
		"right-to-left drag mirrors the active and fixed endpoints"
	);

	let mut clicked = fix_new();
	fill(&mut clicked, "abcdef", false, false, 120.0);
	edit::set_selection(&mut clicked.ds.ed[0], 5, 0);
	let down = pointer_at(&clicked, dispatch::E_POINTER_DOWN, 0, 2);
	send(&mut clicked, &down);
	let mut jitter = pointer_at(&clicked, dispatch::E_POINTER_MOVE, 0, 2);
	jitter.x += 2.0;
	send(&mut clicked, &jitter);
	jitter.etype = dispatch::E_POINTER_UP;
	send(&mut clicked, &jitter);
	assert_eq!(
		(clicked.ds.ed[0].anchor, clicked.ds.ed[0].caret),
		(2, 2),
		"a plain click with sub-threshold jitter remains collapsed"
	);
}

/// Field drag hit-testing follows visual lines, clamps outside line extents,
/// and reports codepoint offsets only at shaped emoji boundaries.
pub fn test_pointer_drag_multiline_clamps_and_uses_emoji_boundaries() {
	let mut multiline = fix_new();
	fill(&mut multiline, "abc\ndef", true, false, 120.0);
	multiline.sc.entries[0].h = multiline.lay.tls[0].line_h * 2.0;
	drag(&mut multiline, 0, 1, 1, 6);
	assert_eq!(
		(multiline.ds.ed[0].anchor, multiline.ds.ed[0].caret),
		(1, 6),
		"dragging onto the second visual line selects across the hard break"
	);

	let mut left = fix_new();
	fill(&mut left, "abc", false, false, 120.0);
	let down = pointer_at(&left, dispatch::E_POINTER_DOWN, 0, 1);
	send(&mut left, &down);
	let mut moved = pointer_at(&left, dispatch::E_POINTER_MOVE, 0, 0);
	moved.x = left.sc.entries[0].x - 100.0;
	send(&mut left, &moved);
	moved.etype = dispatch::E_POINTER_UP;
	send(&mut left, &moved);
	assert_eq!((left.ds.ed[0].anchor, left.ds.ed[0].caret), (1, 0));

	let mut right = fix_new();
	fill(&mut right, "abc", false, false, 120.0);
	let down = pointer_at(&right, dispatch::E_POINTER_DOWN, 0, 1);
	send(&mut right, &down);
	let mut moved = pointer_at(&right, dispatch::E_POINTER_MOVE, 0, 3);
	moved.x = right.sc.entries[0].x + 1000.0;
	send(&mut right, &moved);
	moved.etype = dispatch::E_POINTER_UP;
	send(&mut right, &moved);
	assert_eq!((right.ds.ed[0].anchor, right.ds.ed[0].caret), (1, 3));

	let mut emoji = fix_new();
	fill(&mut emoji, "a😀b", false, false, 120.0);
	let down = pointer_at(&emoji, dispatch::E_POINTER_DOWN, 0, 0);
	send(&mut emoji, &down);
	let before = pointer_at(&emoji, dispatch::E_POINTER_MOVE, 0, 1).x;
	let after = pointer_at(&emoji, dispatch::E_POINTER_MOVE, 0, 2).x;
	let mut moved = pointer_at(&emoji, dispatch::E_POINTER_MOVE, 0, 2);
	moved.x = f64::midpoint(before, after) + 0.1;
	send(&mut emoji, &moved);
	moved.etype = dispatch::E_POINTER_UP;
	send(&mut emoji, &moved);
	assert_eq!(
		(emoji.ds.ed[0].anchor, emoji.ds.ed[0].caret),
		(0, 2),
		"emoji hit returns a grapheme-boundary codepoint offset rather than a UTF-8 byte offset"
	);
}

/// Active IME preedit keeps ownership of the caret during a captured drag.
pub fn test_pointer_drag_does_not_extend_active_composition() {
	let mut f = fix_new();
	fill(&mut f, "abcdef", false, false, 120.0);
	send(&mut f, &event(dispatch::E_COMPOSITION_START, "", "", 0));
	let down = pointer_at(&f, dispatch::E_POINTER_DOWN, 0, 1);
	send(&mut f, &down);
	let moved = pointer_at(&f, dispatch::E_POINTER_MOVE, 0, 5);
	send(&mut f, &moved);
	let up = pointer_at(&f, dispatch::E_POINTER_UP, 0, 5);
	send(&mut f, &up);
	assert!(f.ds.ed[0].composing);
	assert_eq!(
		(f.ds.ed[0].anchor, f.ds.ed[0].caret),
		(1, 1),
		"captured pointer movement cannot extend an active composition"
	);
}
/// Escape, host blur, and close all cancel field-selection capture.
pub fn test_pointer_drag_capture_clears_on_cancel_paths() {
	for cancel in [dispatch::E_KEY_DOWN, dispatch::E_BLUR, dispatch::E_CLOSE] {
		let mut f = fix_new();
		fill(&mut f, "abcdef", false, false, 120.0);
		let down = pointer_at(&f, dispatch::E_POINTER_DOWN, 0, 1);
		send(&mut f, &down);
		let canceled = if cancel == dispatch::E_KEY_DOWN {
			event(cancel, "Escape", "", 0)
		} else {
			event(cancel, "", "", 0)
		};
		send(&mut f, &canceled);
		let moved = pointer_at(&f, dispatch::E_POINTER_MOVE, 0, 5);
		send(&mut f, &moved);
		assert_eq!(
			(f.ds.ed[0].anchor, f.ds.ed[0].caret),
			(1, 1),
			"cancel event {cancel} prevents later moves from extending selection"
		);
	}
}

/// Appends one string to the doc pool, returning its index.
fn push_str(f: &mut Fix, text: &str) -> u32 {
	let index = u32::try_from(f.d.strs.len()).expect("str index fits u32");
	f.d.strs.push(text.to_string());
	index
}

/// Appends a doc node with one string attr and returns its id.
fn push_node(f: &mut Fix, kind: u32, attr: u32, value: u32) -> u32 {
	let node = u32::try_from(f.d.node_kind.len()).expect("node id fits u32");
	f.d.node_kind.push(kind);
	f.d.node_flags.push(0);
	f.d.node_parent.push(slir::NONE);
	f.d.node_first.push(slir::NONE);
	f.d.node_next.push(slir::NONE);
	f.d.node_key.push(0);
	f.d.node_id.push(node);
	f.d.node_line.push(1);
	let attr_start = *f.d.attr_index.last().expect("attr_index terminator");
	f.d.attr_index.push(attr_start + 1);
	f.d.attr_id.push(attr);
	let aval = push_aval_str(f, value);
	f.d.attr_val.push(aval);
	node
}

/// Appends one string AVAL entry, returning its pool index.
fn push_aval_str(f: &mut Fix, value: u32) -> u32 {
	let aval = u32::try_from(f.d.aval_tag.len()).expect("aval index fits u32");
	f.d.aval_tag.push(slir::T_STR);
	f.d.aval_lo.push(value);
	f.d.aval_hi.push(0);
	f.d.aval_num.push(0.0);
	aval
}

/// Binds a typed `keys=` string attr directly on the fixture's field node.
fn set_field_keys(f: &mut Fix, keys: u32) {
	f.d.attr_index[1] = 1;
	f.d.attr_id.push(slir::A_KEYS);
	let aval = push_aval_str(f, keys);
	f.d.attr_val.push(aval);
}

/// Declares a typed `keys=` signal on `node`, returning its str index.
fn push_key_signal(f: &mut Fix, node: u32, signal: &str) -> u32 {
	let sig = push_str(f, signal);
	f.d.sign_name.push(sig);
	f.d.sign_node.push(node);
	f.d.sign_trigger.push(dispatch::TR_KEY_ACTIVATE);
	sig
}

/// Verifies that a field's own `keys=` map preempts kernel editing for plain
/// keys while modified keys still reach the editor.
pub fn test_field_keys_preempt_plain_only() {
	let mut f = fix_new();
	fill(&mut f, "a", true, false, 100.0);
	let keys = push_str(&mut f, "Enter:split");
	set_field_keys(&mut f, keys);
	let split = push_key_signal(&mut f, 0, "split");

	let plain = send(&mut f, &event(dispatch::E_KEY_DOWN, "Enter", "", 0));
	assert!(plain.sig_name.contains(&split), "plain Enter fires the field binding");
	assert_eq!(edit::text_str(&f.ds.ed[0]), "a", "preempted Enter inserts nothing");

	let shifted = send(&mut f, &event(dispatch::E_KEY_DOWN, "Enter", "", dispatch::M_SHIFT));
	assert!(!shifted.sig_name.contains(&split), "Shift+Enter bypasses the binding");
	assert_eq!(edit::text_str(&f.ds.ed[0]), "a\n", "Shift+Enter inserts the soft break");
}

/// Verifies that boundary no-op edit commands bubble to an ancestor's `keys=`
/// map while effective edits stay with the editor.
pub fn test_boundary_noop_bubbles_to_ancestor_keys() {
	let mut f = fix_new();
	fill(&mut f, "ab", true, false, 100.0);
	let keys = push_str(&mut f, "Backspace:merge,ArrowLeft:prev");
	let parent = push_node(&mut f, slir::K_COL, slir::A_KEYS, keys);
	let merge = push_key_signal(&mut f, parent, "merge");
	let prev = push_key_signal(&mut f, parent, "prev");
	f.d.node_parent[0] = parent;
	f.sc.entries[0].parent_ix = 1;
	add_scene(&mut f.sc, parent, -1, 0.0, 0.0, 100.0, 100.0, 0);

	send(&mut f, &event(dispatch::E_KEY_DOWN, "Home", "", 0));
	let at_start = send(&mut f, &event(dispatch::E_KEY_DOWN, "Backspace", "", 0));
	assert!(at_start.sig_name.contains(&merge), "Backspace at the start bubbles");
	assert_eq!(
		at_start
			.sig_meta
			.last()
			.expect("bubble carries metadata")
			.pressed_key,
		"Backspace",
		"bubble reports the pressed key"
	);
	assert_eq!(edit::text_str(&f.ds.ed[0]), "ab", "boundary Backspace deletes nothing");

	let left_edge = send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowLeft", "", 0));
	assert!(left_edge.sig_name.contains(&prev), "ArrowLeft at the edge bubbles");
	assert_eq!(f.ds.ed[0].caret, 0, "edge arrow leaves the caret");

	send(&mut f, &event(dispatch::E_TEXT, "", "x", 0));
	let mid = send(&mut f, &event(dispatch::E_KEY_DOWN, "Backspace", "", 0));
	assert!(!mid.sig_name.contains(&merge), "mid-text Backspace stays in the editor");
	assert_eq!(edit::text_str(&f.ds.ed[0]), "ab", "mid-text Backspace deletes");

	let moved = send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowRight", "", 0));
	assert!(!moved.sig_name.contains(&prev), "interior arrow stays in the editor");
	assert_eq!(f.ds.ed[0].caret, 1, "interior arrow moves the caret");
}

/// Verifies that Enter-submit emits exactly its own signal and does not fall
/// through to ancestor `keys=` maps.
pub fn test_enter_submit_does_not_bubble() {
	let mut f = fix_new();
	fill(&mut f, "one", false, true, 100.0);
	let keys = push_str(&mut f, "Enter:save");
	let parent = push_node(&mut f, slir::K_COL, slir::A_KEYS, keys);
	let save = push_key_signal(&mut f, parent, "save");
	f.d.node_parent[0] = parent;
	f.sc.entries[0].parent_ix = 1;
	add_scene(&mut f.sc, parent, -1, 0.0, 0.0, 100.0, 100.0, 0);

	let submit = u32::try_from(
		f.d.strs
			.iter()
			.position(|s| s == "submit")
			.expect("submit string declared"),
	)
	.expect("submit str index fits u32");
	let eff = send(&mut f, &event(dispatch::E_KEY_DOWN, "Enter", "", 0));
	assert!(eff.sig_name.contains(&submit), "submit fires");
	assert!(!eff.sig_name.contains(&save), "submit does not bubble into keys=");
}

/// Verifies that modified printable shortcuts bubble with their modifier
/// bitset while unmodified typing stays with the editor.
pub fn test_modified_printable_bubbles_with_mods() {
	let mut f = fix_new();
	fill(&mut f, "a", true, false, 100.0);
	let keys = push_str(&mut f, "b:bold");
	set_field_keys(&mut f, keys);
	let bold = push_key_signal(&mut f, 0, "bold");

	let typed = send(&mut f, &event(dispatch::E_KEY_DOWN, "b", "", 0));
	assert!(!typed.sig_name.contains(&bold), "plain b is not a shortcut");
	send(&mut f, &event(dispatch::E_TEXT, "", "b", 0));
	assert_eq!(edit::text_str(&f.ds.ed[0]), "ab", "plain b inserts text");

	let chord = send(&mut f, &event(dispatch::E_KEY_DOWN, "b", "", dispatch::M_META));
	assert!(chord.sig_name.contains(&bold), "Cmd+B fires the field binding");
	let meta = chord.sig_meta.last().expect("shortcut carries metadata");
	assert_eq!(meta.mods, dispatch::M_META, "shortcut reports its modifiers");
	assert_eq!(meta.pressed_key, "b", "shortcut reports its key");
	assert_eq!(edit::text_str(&f.ds.ed[0]), "ab", "Cmd+B changes no text");
}

/// Verifies that vertical arrows clamped at the first/last visual line leave
/// the caret untouched, retain `goal_x`, and bubble to ancestor `keys=` maps.
pub fn test_vertical_boundary_bubbles_with_goal_x() {
	let mut f = fix_new();
	fill(&mut f, "ab\ncde", true, false, 100.0);
	let keys = push_str(&mut f, "ArrowUp:up,ArrowDown:down");
	let parent = push_node(&mut f, slir::K_COL, slir::A_KEYS, keys);
	let up = push_key_signal(&mut f, parent, "up");
	let down = push_key_signal(&mut f, parent, "down");
	f.d.node_parent[0] = parent;
	f.sc.entries[0].parent_ix = 1;
	add_scene(&mut f.sc, parent, -1, 0.0, 0.0, 100.0, 100.0, 0);

	// Mid first line: Up stays put, keeps goal_x, and bubbles.
	send(&mut f, &event(dispatch::E_KEY_DOWN, "Home", "", dispatch::M_META));
	send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowRight", "", 0));
	assert_eq!(f.ds.ed[0].caret, 1, "caret mid first line");
	let first = send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowUp", "", 0));
	assert!(first.sig_name.contains(&up), "Up on the first line bubbles");
	assert_eq!(f.ds.ed[0].caret, 1, "Up on the first line leaves the caret");
	assert!(f.ds.ed[0].goal_x >= 0.0, "Up on the first line retains goal_x");

	// Mid last line: Down stays put, keeps goal_x, and bubbles.
	send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowDown", "", 0));
	assert_eq!(f.ds.ed[0].caret, 4, "interior Down moves to the last line");
	let last = send(&mut f, &event(dispatch::E_KEY_DOWN, "ArrowDown", "", 0));
	assert!(last.sig_name.contains(&down), "Down on the last line bubbles");
	assert_eq!(f.ds.ed[0].caret, 4, "Down on the last line leaves the caret");
	assert!(f.ds.ed[0].goal_x >= 0.0, "Down on the last line retains goal_x");
}
