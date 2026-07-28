//! Focused gesture dispatch state-machine tests.

use crate::{
	dispatch::{self, Effects, Event, SigMeta},
	layout, list,
	scene::{self, Scene},
	slir::{self, Doc},
	style::{self, St},
	test_hit,
};

const ROOT: u32 = 0;
const SOURCE: u32 = 1;
const SOURCE_CHILD: u32 = 2;
const TARGET: u32 = 3;
const TARGET_INNER: u32 = 4;

struct Fixture {
	doc:      Doc,
	state:    St,
	layout:   layout::Lay,
	scene:    Scene,
	dispatch: dispatch::DState,
}

fn intern(doc: &mut Doc, value: &str) -> u32 {
	if let Some(index) = doc.strs.iter().position(|candidate| candidate == value) {
		return u32::try_from(index).expect("test string table fits u32");
	}
	let index = u32::try_from(doc.strs.len()).expect("test string table fits u32");
	doc.strs.push(value.to_owned());
	index
}

fn set_string_attr(fixture: &mut Fixture, node: u32, attr: u32, value: &str) {
	let string_ref = intern(&mut fixture.doc, value);
	let value_index =
		u32::try_from(fixture.doc.aval_tag.len()).expect("test attribute count fits u32");
	fixture.doc.aval_tag.push(slir::T_STR);
	fixture.doc.aval_lo.push(string_ref);
	fixture.doc.aval_hi.push(0);
	fixture.doc.aval_num.push(0.0);
	fixture.doc.attr_id.push(attr);
	fixture.doc.attr_val.push(value_index);
	for boundary in fixture
		.doc
		.attr_index
		.iter_mut()
		.skip(usize::try_from(node + 1).expect("node fits usize"))
	{
		*boundary += 1;
	}
}

fn fixture(signals: &[(u32, u32, &str)]) -> Fixture {
	let mut doc = slir::doc_new();
	doc.ok = true;
	doc.strs.extend([
		String::new(),
		"root".into(),
		"root/source".into(),
		"root/source/child".into(),
		"root/target".into(),
		"root/target/inner".into(),
		"pressed".into(),
		"hover".into(),
		"focus".into(),
		"focus-visible".into(),
		"disabled".into(),
		"dragging".into(),
		"drop".into(),
	]);
	for node in ROOT..=TARGET_INNER {
		doc.node_kind.push(slir::K_RECT);
		doc.node_flags
			.push(if node == SOURCE { slir::F_FOCUSABLE } else { 0 });
		doc.node_parent.push(match node {
			ROOT => slir::NONE,
			SOURCE | TARGET => ROOT,
			SOURCE_CHILD => SOURCE,
			TARGET_INNER => TARGET,
			_ => unreachable!(),
		});
		doc.node_first.push(slir::NONE);
		doc.node_next.push(slir::NONE);
		doc.node_key.push(node + 1);
		doc.node_id.push(0);
		doc.node_line.push(1);
	}
	doc.attr_index.resize(6, 0);
	for &(node, trigger, name) in signals {
		let name = intern(&mut doc, name);
		doc.sign_name.push(name);
		doc.sign_node.push(node);
		doc.sign_trigger.push(trigger);
	}

	let mut scene = scene::scene_new();
	test_hit::add(&mut scene, ROOT, -1, 0.0, 0.0, 300.0, 80.0, 0.0, 0.0, 0);
	test_hit::add(&mut scene, SOURCE, 0, 0.0, 0.0, 80.0, 80.0, 0.0, 0.0, slir::F_FOCUSABLE);
	test_hit::add(&mut scene, SOURCE_CHILD, 1, 8.0, 8.0, 64.0, 64.0, 0.0, 0.0, 0);
	test_hit::add(&mut scene, TARGET, 0, 120.0, 0.0, 100.0, 80.0, 0.0, 0.0, 0);
	test_hit::add(&mut scene, TARGET_INNER, 3, 132.0, 8.0, 76.0, 64.0, 0.0, 0.0, 0);

	let mut state = style::st_new();
	list::init(&doc, &mut state.lists);
	Fixture { doc, state, layout: layout::lay_new(), scene, dispatch: dispatch::dstate_new() }
}

const fn pointer(etype: u32, x: f64, y: f64, button: u32, clicks: u32, mods: u32) -> Event {
	Event {
		etype,
		x,
		y,
		dx: 0.0,
		dy: 0.0,
		button,
		clicks,
		key: String::new(),
		text: String::new(),
		mods,
	}
}

fn key_event(key: &str) -> Event {
	let mut event = pointer(dispatch::E_KEY_DOWN, 0.0, 0.0, 0, 0, 0);
	event.key = key.into();
	event
}

fn send(fixture: &mut Fixture, event: &Event) -> Effects {
	dispatch::dispatch(
		&fixture.doc,
		&mut fixture.state,
		&fixture.layout,
		&fixture.scene,
		&mut fixture.dispatch,
		event,
	)
}

fn signal_names(fixture: &Fixture, effects: &Effects) -> Vec<String> {
	effects
		.sig_name
		.iter()
		.map(|&name| slir::str_at(&fixture.doc, name).to_owned())
		.collect()
}

/// Primary Press precedes capture, while secondary Context never presses or
/// focuses.
pub fn test_press_and_context_button_semantics() {
	let mut primary = fixture(&[
		(SOURCE, dispatch::TR_PRESS, "pressed-signal"),
		(SOURCE, dispatch::TR_CONTEXT, "context-signal"),
	]);
	let down =
		send(&mut primary, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, dispatch::M_CTRL));
	assert_eq!(signal_names(&primary, &down), ["pressed-signal"]);
	assert_eq!(primary.dispatch.pressed, SOURCE);
	assert_eq!(primary.dispatch.fs.focus, SOURCE);
	assert!(style::node_state_on(&primary.doc, &primary.state, SOURCE, "pressed"));
	assert_eq!(down.sig_meta, [SigMeta {
		cancelled:   false,
		drag_dx:     0.0,
		drag_dy:     0.0,
		dropped:     false,
		dx:          0.0,
		dy:          0.0,
		x:           20.0,
		y:           20.0,
		mods:        dispatch::M_CTRL,
		button:      0,
		clicks:      1,
		key:         "root/source".into(),
		hit_key:     "root/source/child".into(),
		pressed_key: String::new(),
		src_key:     String::new(),
		src_item:    String::new(),
	}]);
	let up = send(&mut primary, &pointer(dispatch::E_POINTER_UP, 20.0, 20.0, 0, 0, 0));
	assert!(up.sig_name.is_empty());
	assert_eq!(primary.dispatch.pressed, slir::NONE);

	let mut secondary = fixture(&[
		(SOURCE, dispatch::TR_CONTEXT, "context-signal"),
		(SOURCE, dispatch::TR_DRAG_START, "drag-start"),
	]);
	let context = send(&mut secondary, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 2, 1, 0));
	assert_eq!(signal_names(&secondary, &context), ["context-signal"]);
	assert_eq!(secondary.dispatch.pressed, slir::NONE);
	assert_eq!(secondary.dispatch.fs.focus, slir::NONE);
	assert_eq!(secondary.dispatch.drag_source, slir::NONE);
	assert!(!style::node_state_on(&secondary.doc, &secondary.state, SOURCE, "pressed"));
	assert_eq!(context.sig_meta[0].button, 2);
	assert_eq!(context.sig_meta[0].key, "root/source");

	let mut field = fixture(&[
		(SOURCE, dispatch::TR_CONTEXT, "field-context"),
		(SOURCE, dispatch::TR_CHANGE, "field-change"),
		(SOURCE, dispatch::TR_DRAG_START, "field-drag"),
	]);
	let field_context =
		send(&mut field, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 2, 1, dispatch::M_ALT));
	assert_eq!(signal_names(&field, &field_context), ["field-context"]);
	assert_eq!(field.dispatch.fs.focus, SOURCE);
	assert_eq!(field.dispatch.pressed, slir::NONE);
	assert_eq!(field.dispatch.drag_source, slir::NONE);
	assert_eq!((field_context.sig_meta[0].x, field_context.sig_meta[0].y), (20.0, 20.0));
	assert_eq!(field_context.sig_meta[0].mods, dispatch::M_ALT);
}

/// A handled multi-click emits Dblclick on down and suppresses that gesture's
/// Activate.
pub fn test_double_click_suppresses_activate() {
	for clicks in [2, 3] {
		let mut fixture = fixture(&[
			(SOURCE, dispatch::TR_ACTIVATE, "activate"),
			(SOURCE, dispatch::TR_DBLCLICK, "double"),
		]);
		let down = send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, clicks, 0));
		assert_eq!(signal_names(&fixture, &down), ["double"]);
		assert_eq!(down.sig_meta[0].clicks, clicks);
		assert_eq!(down.sig_meta[0].key, "root/source");
		assert_eq!(down.sig_meta[0].hit_key, "root/source/child");

		let up = send(&mut fixture, &pointer(dispatch::E_POINTER_UP, 20.0, 20.0, 0, 0, 0));
		assert!(up.sig_name.is_empty(), "Activate must be suppressed");
		assert_eq!(fixture.dispatch.pressed, slir::NONE);
		assert!(!fixture.dispatch.suppress_activate);
	}
}

/// Drag starts strictly beyond four units, targets the deepest external Drop,
/// and cleans up.
pub fn test_drag_threshold_deepest_drop_and_source_metadata() {
	let mut fixture = fixture(&[
		(SOURCE, dispatch::TR_ACTIVATE, "activate"),
		(SOURCE, dispatch::TR_DRAG_START, "drag-start"),
		(SOURCE_CHILD, dispatch::TR_DROP, "inside-drop"),
		(TARGET, dispatch::TR_DROP, "outer-drop"),
		(TARGET_INNER, dispatch::TR_DROP, "inner-drop"),
	]);
	let down = send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
	assert!(down.sig_name.is_empty());

	let threshold_boundary =
		send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 24.0, 20.0, 0, 0, 0));
	assert!(threshold_boundary.sig_name.is_empty());
	assert!(!fixture.dispatch.drag_active);

	let start = send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 28.0, 20.0, 0, 0, 0));
	assert_eq!(signal_names(&fixture, &start), ["drag-start"]);
	assert!(fixture.dispatch.drag_active);
	assert_eq!(fixture.dispatch.drop_target, slir::NONE);
	assert!(style::node_state_on(&fixture.doc, &fixture.state, SOURCE, "dragging"));
	assert_eq!(start.sig_meta[0].key, "root/source");
	assert_eq!((start.sig_meta[0].x, start.sig_meta[0].y), (28.0, 20.0));

	let over_target = send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, 0));
	assert!(over_target.sig_name.is_empty());
	assert_eq!(fixture.dispatch.drop_target, TARGET_INNER);
	assert!(style::node_state_on(&fixture.doc, &fixture.state, TARGET_INNER, "drop"));
	assert!(!style::node_state_on(&fixture.doc, &fixture.state, SOURCE_CHILD, "drop"));

	let dropped =
		send(&mut fixture, &pointer(dispatch::E_POINTER_UP, 150.0, 20.0, 0, 0, dispatch::M_SHIFT));
	assert_eq!(signal_names(&fixture, &dropped), ["inner-drop"]);
	assert_eq!(dropped.sig_item, [""]);
	assert_eq!(dropped.sig_meta[0].key, "root/target/inner");
	assert_eq!(dropped.sig_meta[0].src_key, "root/source");
	assert_eq!(dropped.sig_meta[0].src_item, "");
	assert_eq!(dropped.sig_meta[0].mods, dispatch::M_SHIFT);
	assert_eq!(fixture.dispatch.drag_source, slir::NONE);
	assert_eq!(fixture.dispatch.drop_target, slir::NONE);
	assert!(!fixture.dispatch.drag_active);
	assert!(!style::node_state_on(&fixture.doc, &fixture.state, SOURCE, "dragging"));
	assert!(!style::node_state_on(&fixture.doc, &fixture.state, TARGET_INNER, "drop"));
}

/// Releasing away from a target and Blur cancellation never synthesize Drop or
/// Activate.
pub fn test_drag_cancel_and_blur_clear_all_gesture_state() {
	let mut fixture = fixture(&[
		(SOURCE, dispatch::TR_ACTIVATE, "activate"),
		(SOURCE, dispatch::TR_DRAG_START, "drag-start"),
		(TARGET_INNER, dispatch::TR_DROP, "drop-finished"),
	]);
	send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
	let started = send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 28.0, 20.0, 0, 0, 0));
	assert_eq!(signal_names(&fixture, &started), ["drag-start"]);
	send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, 0));
	assert_eq!(fixture.dispatch.drop_target, TARGET_INNER);
	send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 270.0, 20.0, 0, 0, 0));
	assert_eq!(fixture.dispatch.drop_target, slir::NONE);
	let released = send(&mut fixture, &pointer(dispatch::E_POINTER_UP, 270.0, 20.0, 0, 0, 0));
	assert!(released.sig_name.is_empty());
	assert_eq!(fixture.dispatch.pressed, slir::NONE);
	assert_eq!(fixture.dispatch.drag_source, slir::NONE);
	assert!(!fixture.dispatch.drag_active);

	send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
	send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 28.0, 20.0, 0, 0, 0));
	send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, 0));
	let blurred = send(&mut fixture, &pointer(dispatch::E_BLUR, 0.0, 0.0, 0, 0, 0));
	assert!(blurred.sig_name.is_empty());
	assert_eq!(fixture.dispatch.pressed, slir::NONE);
	assert_eq!(fixture.dispatch.drag_source, slir::NONE);
	assert_eq!(fixture.dispatch.drop_target, slir::NONE);
	assert!(!style::node_state_on(&fixture.doc, &fixture.state, SOURCE, "dragging"));
	assert!(!style::node_state_on(&fixture.doc, &fixture.state, TARGET_INNER, "drop"));
}

/// Release cancels an active drag whose source became disabled after the final
/// move.
pub fn test_drag_release_revalidates_source() {
	let mut fixture = fixture(&[
		(SOURCE, dispatch::TR_DRAG_START, "drag-start"),
		(TARGET_INNER, dispatch::TR_DROP, "drop-finished"),
	]);
	send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
	send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, 0));
	assert_eq!(fixture.dispatch.drop_target, TARGET_INNER);
	assert!(style::set_node_state(&fixture.doc, &mut fixture.state, SOURCE, "disabled", true));

	let released = send(&mut fixture, &pointer(dispatch::E_POINTER_UP, 150.0, 20.0, 0, 0, 0));
	assert!(released.sig_name.is_empty());
	assert_eq!(fixture.dispatch.drag_source, slir::NONE);
	assert_eq!(fixture.dispatch.drop_target, slir::NONE);
	assert!(!style::node_state_on(&fixture.doc, &fixture.state, TARGET_INNER, "drop"));
}

/// Pruning a synthetic source clears Drop state on a surviving real target.
pub fn test_pruned_drag_source_clears_surviving_drop_state() {
	let mut fixture = fixture(&[]);
	let source = list::synthetic(&fixture.doc, &mut fixture.state.lists, ROOT, SOURCE, "gone");
	assert!(style::set_node_state(&fixture.doc, &mut fixture.state, source, "dragging", true));
	assert!(style::set_node_state(&fixture.doc, &mut fixture.state, TARGET_INNER, "drop", true));
	fixture.dispatch.drag_source = source;
	fixture.dispatch.drop_target = TARGET_INNER;
	fixture.dispatch.drag_active = true;
	fixture.state.lists.sy_id.clear();
	fixture.state.lists.sy_each.clear();
	fixture.state.lists.sy_tpl.clear();
	fixture.state.lists.sy_key.clear();

	assert!(dispatch::prune_vanished(&fixture.doc, &mut fixture.state, &mut fixture.dispatch));
	assert_eq!(fixture.dispatch.drag_source, slir::NONE);
	assert_eq!(fixture.dispatch.drop_target, slir::NONE);
	assert!(!style::node_state_on(&fixture.doc, &fixture.state, TARGET_INNER, "drop"));
}

/// A fresh scene that omits an active real source cancels capture and Drop
/// styling.
pub fn test_fresh_scene_cancels_missing_drag_source() {
	let mut fixture = fixture(&[]);
	assert!(style::set_node_state(&fixture.doc, &mut fixture.state, SOURCE, "dragging", true));
	assert!(style::set_node_state(&fixture.doc, &mut fixture.state, TARGET_INNER, "drop", true));
	fixture.dispatch.pressed = SOURCE;
	fixture.dispatch.drag_source = SOURCE;
	fixture.dispatch.drop_target = TARGET_INNER;
	fixture.dispatch.drag_active = true;

	let mut fresh_scene = scene::scene_new();
	test_hit::add(&mut fresh_scene, ROOT, -1, 0.0, 0.0, 300.0, 80.0, 0.0, 0.0, 0);
	test_hit::add(&mut fresh_scene, TARGET, 0, 120.0, 0.0, 100.0, 80.0, 0.0, 0.0, 0);
	test_hit::add(&mut fresh_scene, TARGET_INNER, 1, 132.0, 8.0, 76.0, 64.0, 0.0, 0.0, 0);

	assert!(dispatch::cancel_invalid_drag(
		&fixture.doc,
		&mut fixture.state,
		&fresh_scene,
		&mut fixture.dispatch
	));
	assert_eq!(fixture.dispatch.pressed, slir::NONE);
	assert_eq!(fixture.dispatch.drag_source, slir::NONE);
	assert_eq!(fixture.dispatch.drop_target, slir::NONE);
	assert!(!style::node_state_on(&fixture.doc, &fixture.state, SOURCE, "dragging"));
	assert!(!style::node_state_on(&fixture.doc, &fixture.state, TARGET_INNER, "drop"));
}

/// Continuous gestures emit pointer, drag-update, Drop, and `DragEnd` in order.
pub fn test_continuous_drag_signals_and_release_metadata() {
	let mut fixture = fixture(&[
		(SOURCE, dispatch::TR_POINTER_MOVE, "pointer-move"),
		(SOURCE, dispatch::TR_POINTER_UP, "pointer-up"),
		(SOURCE, dispatch::TR_DRAG_START, "drag-start"),
		(SOURCE, dispatch::TR_DRAG_UPDATE, "drag-update"),
		(SOURCE, dispatch::TR_DRAG_END, "drag-end"),
		(TARGET_INNER, dispatch::TR_DROP, "drop"),
	]);
	send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
	let mut move_event = pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, dispatch::M_CTRL);
	move_event.dx = 130.0;
	let moved = send(&mut fixture, &move_event);
	assert_eq!(signal_names(&fixture, &moved), ["pointer-move", "drag-start", "drag-update"]);
	for meta in &moved.sig_meta {
		assert_eq!(meta.key, "root/source");
		assert_eq!(meta.dx, 130.0);
		assert_eq!(meta.dy, 0.0);
		assert_eq!(meta.drag_dx, 130.0);
		assert_eq!(meta.drag_dy, 0.0);
		assert!(!meta.cancelled);
		assert!(!meta.dropped);
	}

	let released =
		send(&mut fixture, &pointer(dispatch::E_POINTER_UP, 150.0, 20.0, 0, 0, dispatch::M_SHIFT));
	assert_eq!(signal_names(&fixture, &released), ["pointer-up", "drop", "drag-end"]);
	assert_eq!(released.sig_meta[0].drag_dx, 130.0);
	assert!(!released.sig_meta[0].dropped);
	assert_eq!(released.sig_meta[1].key, "root/target/inner");
	assert_eq!(released.sig_meta[1].src_key, "root/source");
	assert!(released.sig_meta[1].dropped);
	assert_eq!(released.sig_meta[2].key, "root/source");
	assert!(!released.sig_meta[2].cancelled);
	assert!(released.sig_meta[2].dropped);
}

/// `PointerUp` routes for every button while primary gesture cleanup remains
/// primary-only.
pub fn test_secondary_pointer_up_routes_without_releasing_primary_capture() {
	let mut fixture = fixture(&[(SOURCE, dispatch::TR_POINTER_UP, "released")]);
	send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
	let released =
		send(&mut fixture, &pointer(dispatch::E_POINTER_UP, 270.0, 20.0, 2, 1, dispatch::M_ALT));
	assert_eq!(signal_names(&fixture, &released), ["released"]);
	assert_eq!(released.sig_meta[0].button, 2);
	assert_eq!(released.sig_meta[0].mods, dispatch::M_ALT);
	assert_eq!(released.sig_meta[0].key, "root/source");
	assert_eq!(fixture.dispatch.pressed, SOURCE);
	send(&mut fixture, &pointer(dispatch::E_POINTER_UP, 270.0, 20.0, 0, 0, 0));
	assert_eq!(fixture.dispatch.pressed, slir::NONE);
}

/// Blur and Close each terminate an active drag exactly once from cached
/// pointer data.
pub fn test_blur_and_close_emit_cancelled_drag_end_once() {
	for terminal in [dispatch::E_BLUR, dispatch::E_CLOSE] {
		let mut fixture = fixture(&[
			(SOURCE, dispatch::TR_DRAG_START, "drag-start"),
			(SOURCE, dispatch::TR_DRAG_END, "drag-end"),
			(TARGET_INNER, dispatch::TR_DROP, "drop"),
		]);
		send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
		let mut moved = pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, dispatch::M_META);
		moved.dx = 130.0;
		send(&mut fixture, &moved);
		let ended = send(&mut fixture, &pointer(terminal, 0.0, 0.0, 0, 0, 0));
		assert_eq!(signal_names(&fixture, &ended), ["drag-end"]);
		assert_eq!(ended.sig_meta.len(), 1);
		let meta = &ended.sig_meta[0];
		assert_eq!(meta.key, "root/source");
		assert_eq!(meta.x, 150.0);
		assert_eq!(meta.y, 20.0);
		assert_eq!(meta.dx, 130.0);
		assert_eq!(meta.drag_dx, 130.0);
		assert_eq!(meta.mods, dispatch::M_META);
		assert!(meta.cancelled);
		assert!(!meta.dropped);
		assert_eq!(fixture.dispatch.drag_source, slir::NONE);
		assert_eq!(fixture.dispatch.drop_target, slir::NONE);
		assert!(!fixture.dispatch.drag_active);
	}
}

/// Escape cancels armed and active drags before authored key activation.
pub fn test_escape_cancels_drag_and_consumes_activation() {
	for active in [false, true] {
		let mut fixture = fixture(&[
			(SOURCE, dispatch::TR_ACTIVATE, "activate"),
			(SOURCE, dispatch::TR_DRAG_START, "drag-start"),
			(SOURCE, dispatch::TR_DRAG_END, "drag-end"),
			(TARGET_INNER, dispatch::TR_DROP, "drop"),
		]);
		set_string_attr(&mut fixture, SOURCE, slir::A_KEYS, "Escape");
		fixture.dispatch.fs.focus = SOURCE;
		send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
		if active {
			send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, 0));
			assert_eq!(fixture.dispatch.drop_target, TARGET_INNER);
		}
		let escaped = send(&mut fixture, &key_event("Escape"));
		assert_eq!(signal_names(&fixture, &escaped), ["drag-end"]);
		assert!(escaped.sig_meta[0].cancelled);
		assert!(!escaped.sig_meta[0].dropped);
		assert_eq!(escaped.sig_meta[0].key, "root/source");
		assert_eq!(fixture.dispatch.drag_source, slir::NONE);
		assert_eq!(fixture.dispatch.drop_target, slir::NONE);
		assert!(!fixture.dispatch.drag_active);
	}
}

/// Zero local deltas fall back to successive coordinates; supplied deltas
/// remain authoritative.
pub fn test_pointer_move_delta_fallback_and_authority() {
	let mut fixture = fixture(&[
		(SOURCE, dispatch::TR_POINTER_MOVE, "pointer-move"),
		(SOURCE, dispatch::TR_DRAG_START, "drag-start"),
	]);
	send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
	let fallback = send(&mut fixture, &pointer(dispatch::E_POINTER_MOVE, 22.0, 23.0, 0, 0, 0));
	assert_eq!(signal_names(&fixture, &fallback), ["pointer-move"]);
	assert_eq!((fallback.sig_meta[0].dx, fallback.sig_meta[0].dy), (2.0, 3.0));

	let mut supplied = pointer(dispatch::E_POINTER_MOVE, 30.0, 32.0, 0, 0, 0);
	supplied.dx = 90.0;
	supplied.dy = -7.0;
	let authoritative = send(&mut fixture, &supplied);
	assert_eq!(signal_names(&fixture, &authoritative), ["pointer-move", "drag-start"]);
	for meta in &authoritative.sig_meta {
		assert_eq!((meta.dx, meta.dy), (90.0, -7.0));
	}
}

/// Keyboard Activate keeps the emitter node key and reports the pressed key.
pub fn test_keyboard_activate_metadata_keeps_node_key() {
	let mut fixture = fixture(&[(SOURCE, dispatch::TR_ACTIVATE, "activate")]);
	fixture.dispatch.fs.focus = SOURCE;
	let keyboard = send(&mut fixture, &key_event("Enter"));
	assert_eq!(signal_names(&fixture, &keyboard), ["activate"]);
	assert_eq!(keyboard.sig_meta[0].key, "root/source");
	assert_eq!(keyboard.sig_meta[0].pressed_key, "Enter");
	assert_eq!(keyboard.sig_meta[0].hit_key, "");
	assert_eq!((keyboard.sig_meta[0].x, keyboard.sig_meta[0].y), (-1.0, -1.0));

	send(&mut fixture, &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0));
	let pointer = send(&mut fixture, &pointer(dispatch::E_POINTER_UP, 20.0, 20.0, 0, 1, 0));
	assert_eq!(signal_names(&fixture, &pointer), ["activate"]);
	assert_eq!(pointer.sig_meta[0].key, "root/source");
	assert_eq!(pointer.sig_meta[0].pressed_key, "");
	assert_eq!(pointer.sig_meta[0].hit_key, "root/source/child");
}

/// Escape-blur fires a `cancel=` binder with the retained committed buffer.
pub fn test_cancel_binder_fires_on_escape_blur() {
	let mut fx = self::fixture(&[
		(SOURCE, dispatch::TR_CHANGE, "field-change"),
		(SOURCE, dispatch::TR_CANCEL, "field-cancel"),
	]);
	fx.doc.node_flags[usize::try_from(SOURCE).expect("node fits usize")] |= slir::F_ESCAPE_BLUR;
	fx.dispatch.fs.focus = SOURCE;
	dispatch::ensure_edit(&fx.doc, &mut fx.state, &mut fx.dispatch, SOURCE);
	let mut typed = pointer(dispatch::E_TEXT, 0.0, 0.0, 0, 0, 0);
	typed.text = "draft".into();
	let changed = send(&mut fx, &typed);
	assert_eq!(signal_names(&fx, &changed), ["field-change"]);

	let escaped = send(&mut fx, &key_event("Escape"));
	assert_eq!(signal_names(&fx, &escaped), ["field-cancel"]);
	assert_eq!(escaped.sig_text, ["draft"]);
	assert_eq!(escaped.sig_meta[0].key, "root/source");
	assert_eq!(fx.dispatch.fs.focus, slir::NONE);
	let edit_index =
		usize::try_from(dispatch::ed_ix(&fx.dispatch, SOURCE)).expect("field keeps edit state");
	assert_eq!(fx.dispatch.ed[edit_index].text, "draft");

	// Without a cancel binder the escape-blur stays silent.
	let mut silent = fixture(&[(SOURCE, dispatch::TR_CHANGE, "field-change")]);
	silent.doc.node_flags[usize::try_from(SOURCE).expect("node fits usize")] |= slir::F_ESCAPE_BLUR;
	silent.dispatch.fs.focus = SOURCE;
	dispatch::ensure_edit(&silent.doc, &mut silent.state, &mut silent.dispatch, SOURCE);
	let escaped = send(&mut silent, &key_event("Escape"));
	assert!(escaped.sig_name.is_empty());
	assert_eq!(silent.dispatch.fs.focus, slir::NONE);
}

/// With empty focus, key dispatch starts at the document root `keys=` map,
/// while printable keys stay with a focused editor.
pub fn test_root_keys_map_reachable_without_focus() {
	let mut fx = self::fixture(&[(ROOT, dispatch::TR_ACTIVATE, "root-activate")]);
	set_string_attr(&mut fx, ROOT, slir::A_KEYS, "F1,a");
	assert_eq!(fx.dispatch.fs.focus, slir::NONE);
	let fired = send(&mut fx, &key_event("F1"));
	assert_eq!(signal_names(&fx, &fired), ["root-activate"]);
	assert_eq!(fired.sig_meta[0].pressed_key, "F1");

	// A focused editor consumes printable keys before any `keys=` map.
	let mut editing = fixture(&[
		(ROOT, dispatch::TR_ACTIVATE, "root-activate"),
		(SOURCE, dispatch::TR_CHANGE, "field-change"),
	]);
	set_string_attr(&mut editing, ROOT, slir::A_KEYS, "F1,a");
	editing.dispatch.fs.focus = SOURCE;
	dispatch::ensure_edit(&editing.doc, &mut editing.state, &mut editing.dispatch, SOURCE);
	let printable = send(&mut editing, &key_event("a"));
	assert!(printable.sig_name.is_empty(), "printable keys never leave a focused editor");
	let function_key = send(&mut editing, &key_event("F1"));
	assert_eq!(signal_names(&editing, &function_key), ["root-activate"]);
}

/// PageUp/PageDown/Home/End scroll the nearest scroll ancestor of focus, or
/// the primary root scroller with empty focus, by exactly one viewport.
pub fn test_page_keys_scroll_nearest_scroll_ancestor() {
	let mut fixture = fixture(&[]);
	let target_index = usize::try_from(scene::index_of(&fixture.scene, TARGET))
		.expect("target present in test scene");
	fixture.scene.flags[target_index] |= slir::F_SCROLL;
	fixture.scene.content_main[target_index] = 400.0;
	fixture.dispatch.fs.focus = TARGET_INNER;

	let paged = send(&mut fixture, &key_event("PageDown"));
	assert_eq!(paged.scrolls.len(), 1);
	assert_eq!(paged.scrolls[0].key, "root/target");
	assert_eq!(paged.scrolls[0].off, 80.0, "page size equals the viewport");

	let ended = send(&mut fixture, &key_event("End"));
	assert_eq!(ended.scrolls[0].off, 320.0, "End clamps to the content edge");
	let homed = send(&mut fixture, &key_event("Home"));
	assert_eq!(homed.scrolls[0].off, 0.0);
	let upped = send(&mut fixture, &key_event("PageUp"));
	assert!(upped.scrolls.is_empty(), "already at the top");

	// Empty focus targets the primary root scroller.
	fixture.dispatch.fs.focus = slir::NONE;
	let paged = send(&mut fixture, &key_event("PageDown"));
	assert_eq!(paged.scrolls[0].key, "root/target");
	assert_eq!(paged.scrolls[0].off, 80.0);
}

/// A host write to a field-synced text param resets idle edit buffers while a
/// composing editor keeps kernel priority.
pub fn test_host_param_write_resets_idle_edit_buffer() {
	let mut fixture = fixture(&[(SOURCE, dispatch::TR_CHANGE, "field-change")]);
	let name = intern(&mut fixture.doc, "field-change");
	fixture.doc.parm_name.push(name);
	fixture.doc.parm_type.push(slir::PARAM_TEXT);
	fixture.doc.parm_enum_off.push(0);
	fixture.doc.parm_enum_len.push(0);
	fixture.doc.parm_default.push(0);
	style::init_params(&fixture.doc, &mut fixture.state);
	fixture.dispatch.fs.focus = SOURCE;
	dispatch::ensure_edit(&fixture.doc, &mut fixture.state, &mut fixture.dispatch, SOURCE);
	let mut typed = pointer(dispatch::E_TEXT, 0.0, 0.0, 0, 0, 0);
	typed.text = "draft".into();
	send(&mut fixture, &typed);

	assert!(dispatch::reset_synced_edits(
		&fixture.doc,
		&mut fixture.state,
		&mut fixture.dispatch,
		0,
		"server"
	));
	let edit_index =
		usize::try_from(dispatch::ed_ix(&fixture.dispatch, SOURCE)).expect("field keeps edit state");
	assert_eq!(fixture.dispatch.ed[edit_index].text, "server");
	assert_eq!(fixture.dispatch.ed[edit_index].caret, 6);
	assert!(crate::edit::undo(&mut fixture.dispatch.ed[edit_index]));
	assert_eq!(fixture.dispatch.ed[edit_index].text, "draft");
	assert!(crate::edit::redo(&mut fixture.dispatch.ed[edit_index]));
	assert_eq!(fixture.dispatch.ed[edit_index].text, "server");

	// Equal values are a no-op; a composing editor is never reset.
	assert!(!dispatch::reset_synced_edits(
		&fixture.doc,
		&mut fixture.state,
		&mut fixture.dispatch,
		0,
		"server"
	));
	crate::edit::composition_update(&mut fixture.dispatch.ed[edit_index], "かん");
	assert!(!dispatch::reset_synced_edits(
		&fixture.doc,
		&mut fixture.state,
		&mut fixture.dispatch,
		0,
		"host"
	));
	assert_eq!(fixture.dispatch.ed[edit_index].text, "server");
	assert!(fixture.dispatch.ed[edit_index].composing);
}
