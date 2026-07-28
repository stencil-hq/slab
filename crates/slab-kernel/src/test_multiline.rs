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
#[allow(clippy::too_many_arguments)]
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
	sc.node.push(node);
	sc.parent.push(parent);
	sc.kind.push(slir::K_TEXT);
	sc.x.push(x);
	sc.y.push(y);
	sc.w.push(w);
	sc.h.push(h);
	sc.radius.push(0.0);
	sc.rot.push(0.0);
	sc.cx.push(x + w / 2.0);
	sc.cy.push(y + h / 2.0);
	sc.flags.push(flags);
	sc.content_main.push(0.0);
	sc.scroll_off.push(0.0);
	sc.is_row.push(false);
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
		mods,
	}
}

/// Dispatches an event through a fixture and returns its effects.
pub fn send(f: &mut Fix, ev: &Event) -> Effects {
	dispatch::dispatch(&f.d, &mut f.st, &f.lay, &f.sc, &mut f.ds, ev)
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
	let ri =
		style::build_rstyle(&f.d, &mut f.st, 0, 255, false, 0, 1, 0, 14.0, 400.0, 1.2, 0.0, false);
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
	g.sc.parent[0] = 1;
	g.sc.y[0] = 30.0;
	add_scene(&mut g.sc, 1, -1, 0.0, 0.0, 100.0, 20.0, slir::F_SCROLL | slir::F_CLIP);
	g.sc.content_main[1] = 100.0;
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
	f.sc.parent[0] = 1;
	f.sc.x[0] = 0.0;
	f.sc.y[0] = 0.0;
	add_scene(&mut f.sc, 1, -1, 0.0, 0.0, 20.0, 16.8, slir::F_SCROLL | slir::F_CLIP);
	f.sc.content_main[1] = 100.0;
	send(&mut f, &event(dispatch::E_TEXT, "", "bc", 0));
	assert_eq!(style::scroll_get(&f.st, 1), 0.0, "stale one-line layout does not predict wrap");
	f.lay.tls[0] = std::rc::Rc::new(textm::measure_text(
		&f.d,
		-1,
		14.0,
		1.2,
		0.0,
		&edit::display_str(&f.ds.ed[0]),
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
