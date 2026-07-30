//! Cross-field text-range dispatch, host composition, and paint contracts.

use crate::{dispatch, dumpjson, flatten, frame, layout, list, scene, slir, style, textm};

const ROOT: u32 = 0;
const A: u32 = 1;
const B: u32 = 2;
const C: u32 = 3;
const KEY_A: &str = "#root/#a";
const KEY_B: &str = "#root/#b";
const KEY_C: &str = "#root/#c";

fn add_fixed(doc: &mut slir::Doc, attr: u32, value: f64) {
	let value_index = u32::try_from(doc.aval_tag.len()).expect("fixture value count fits u32");
	doc.aval_tag.push(slir::T_SIZE_FIXED);
	doc.aval_lo.push(0);
	doc.aval_hi.push(0);
	doc.aval_num.push(value);
	doc.attr_id.push(attr);
	doc.attr_val.push(value_index);
}

fn finish_attrs(doc: &mut slir::Doc) {
	doc.attr_index
		.push(i32::try_from(doc.attr_id.len()).expect("fixture attr count fits i32"));
}

fn range_doc() -> slir::Doc {
	let mut doc = slir::doc_new();
	doc.ok = true;
	doc.strs
		.extend([String::new(), "#root".into(), KEY_A.into(), KEY_B.into(), KEY_C.into()]);
	doc.node_kind
		.extend([slir::K_COL, slir::K_TEXT, slir::K_TEXT, slir::K_TEXT]);
	doc.node_flags.extend([
		0,
		slir::F_FOCUSABLE | slir::F_NOWRAP,
		slir::F_FOCUSABLE | slir::F_NOWRAP,
		slir::F_FOCUSABLE | slir::F_NOWRAP,
	]);
	doc.node_parent.extend([slir::NONE, ROOT, ROOT, ROOT]);
	doc.node_first
		.extend([A, slir::NONE, slir::NONE, slir::NONE]);
	doc.node_next.extend([slir::NONE, B, C, slir::NONE]);
	doc.node_key.extend([1, 2, 3, 4]);
	doc.node_id.resize(4, 0);
	doc.node_line.resize(4, 1);

	doc.attr_index.push(0);
	add_fixed(&mut doc, slir::A_W, 140.0);
	finish_attrs(&mut doc);
	for _ in A..=C {
		add_fixed(&mut doc, slir::A_W, 120.0);
		add_fixed(&mut doc, slir::A_H, 24.0);
		finish_attrs(&mut doc);
	}

	doc.sign_name.extend([0, 0, 0]);
	doc.sign_node.extend([A, B, C]);
	doc.sign_trigger.extend([dispatch::TR_CHANGE; 3]);
	doc
}

fn instance() -> frame::Instance {
	let mut instance = frame::inst_shell();
	instance.doc = range_doc();
	frame::inst_init(&mut instance);
	frame::inst_set_env(&mut instance, 200.0, 200.0, 0, false, false);
	style::field_set(&mut instance.st, A, "alpha");
	style::field_set(&mut instance.st, B, "bravo");
	style::field_set(&mut instance.st, C, "charlie");
	frame::inst_frame(&mut instance, 0.0);
	instance
}

fn event(etype: u32, key: &str, text: &str, mods: u32) -> dispatch::Event {
	dispatch::Event {
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

fn pointer(instance: &frame::Instance, node: u32, offset: i32, mods: u32) -> dispatch::Event {
	let scene_index = usize::try_from(crate::scene::index_of(&instance.sc, node))
		.expect("field is in retained scene");
	let text_layout_index = usize::try_from(layout::text_layout_ix(&instance.lay, node))
		.expect("field has retained text layout");
	let entry = &instance.sc.entries[scene_index];
	let text_layout = &instance.lay.tls[text_layout_index];
	let mut event = event(dispatch::E_POINTER_DOWN, "", "", mods);
	event.x = entry.x
		+ textm::caret_x(
			textm::Shaper { d: &instance.doc, cache: &instance.lay.shape_cache },
			text_layout,
			0,
			offset,
		);
	event.y = entry.y + entry.h / 2.0;
	event.clicks = 1;
	event
}

fn release(instance: &mut frame::Instance, down: &dispatch::Event) {
	let mut up = down.clone();
	up.etype = dispatch::E_POINTER_UP;
	frame::inst_dispatch(instance, &up);
}

fn band_widths(frame: &flatten::Frame, node: u32) -> Vec<f64> {
	frame
		.ops
		.iter()
		.filter_map(|op| match op {
			flatten::FrameOp::Rect(rect) if rect.node == node && rect.bg >> 24 == 0x80 => Some(rect.w),
			_ => None,
		})
		.collect()
}

fn text_width(frame: &flatten::Frame, node: u32) -> f64 {
	frame
		.ops
		.iter()
		.filter_map(|op| match op {
			flatten::FrameOp::Text(text) if text.node == node => Some(text.measured_w),
			_ => None,
		})
		.sum()
}

fn establish_pointer_range(instance: &mut frame::Instance) {
	assert!(frame::inst_set_caret(instance, KEY_A, 2, 2));
	let down = pointer(instance, C, 2, dispatch::M_SHIFT);
	frame::inst_dispatch(instance, &down);
	release(instance, &down);
}

/// Shift-click preserves the source anchor, places the active endpoint at the
/// hit caret, and bands both partial endpoints plus every intervening field.
pub fn test_shift_click_cross_field_range_paints_endpoints_and_middle() {
	let mut instance = instance();
	establish_pointer_range(&mut instance);

	let (anchor, head) = frame::inst_get_range(&instance).expect("cross-field range is retained");
	assert_eq!((anchor.key.as_str(), anchor.offset), (KEY_A, 2));
	assert_eq!((head.key.as_str(), head.offset), (KEY_C, 2));
	let source = frame::inst_get_caret(&instance, KEY_A).expect("source edit state");
	let target = frame::inst_get_caret(&instance, KEY_C).expect("target edit state");
	assert_eq!((source.anchor, source.caret), (2, 5), "source selects anchor to end");
	assert_eq!((target.anchor, target.caret), (0, 2), "target selects start to hit");

	let painted = frame::inst_frame(&mut instance, 1.0);
	let a = band_widths(&painted, A);
	let b = band_widths(&painted, B);
	let c = band_widths(&painted, C);
	assert_eq!((a.len(), b.len(), c.len()), (1, 1, 1));
	assert!(a[0] < text_width(&painted, A), "source band is partial");
	assert_eq!(b[0], text_width(&painted, B), "middle field is fully banded");
	assert!(c[0] < text_width(&painted, C), "head band is partial");
}

/// Cross-field mutation requests stay byte-intact for host structural editing;
/// an ordinary primary click still collapses and invalidates the range.
pub fn test_range_edits_defer_to_host_and_plain_click_clears() {
	let mut typed = instance();
	establish_pointer_range(&mut typed);
	let effect = frame::inst_dispatch(&mut typed, &event(dispatch::E_TEXT, "", "X", 0));
	let json = dumpjson::dump_effects(&typed.doc, &typed.st, &effect);
	assert!(json.contains("\"range_edit\":{\"kind\":0"));
	let request = effect.range_edit.expect("typing emits a host range edit");
	assert_eq!(request.kind, dispatch::RANGE_EDIT_TEXT);
	assert_eq!((request.anchor.key.as_str(), request.anchor.offset), (KEY_A, 2));
	assert_eq!((request.head.key.as_str(), request.head.offset), (KEY_C, 2));
	assert_eq!(request.text, "X");
	assert_eq!(frame::inst_field_text(&typed, KEY_A).as_deref(), Some("alpha"));
	assert_eq!(frame::inst_field_text(&typed, KEY_B).as_deref(), Some("bravo"));
	assert_eq!(frame::inst_field_text(&typed, KEY_C).as_deref(), Some("charlie"));
	assert!(frame::inst_get_range(&typed).is_some(), "host edit retains range metadata");

	let effect = frame::inst_dispatch(&mut typed, &event(dispatch::E_KEY_DOWN, "Backspace", "", 0));
	let request = effect
		.range_edit
		.expect("Backspace emits a host range deletion");
	assert_eq!(request.kind, dispatch::RANGE_EDIT_BACKSPACE);
	assert!(request.text.is_empty());
	assert_eq!(frame::inst_field_text(&typed, KEY_C).as_deref(), Some("charlie"));
	assert!(frame::inst_get_range(&typed).is_some());
	for (etype, key, text, kind) in [
		(dispatch::E_PASTE, "", "YZ", dispatch::RANGE_EDIT_PASTE),
		(dispatch::E_CUT, "", "", dispatch::RANGE_EDIT_CUT),
		(dispatch::E_KEY_DOWN, "Delete", "", dispatch::RANGE_EDIT_DELETE),
		(dispatch::E_COPY, "", "", dispatch::RANGE_EDIT_COPY),
		(dispatch::E_COMPOSITION_START, "", "", dispatch::RANGE_EDIT_COMPOSITION),
		(dispatch::E_COMPOSITION_UPDATE, "", "候", dispatch::RANGE_EDIT_COMPOSITION),
		(dispatch::E_COMPOSITION_END, "", "候補", dispatch::RANGE_EDIT_COMPOSITION),
	] {
		let effect = frame::inst_dispatch(&mut typed, &event(etype, key, text, 0));
		let request = effect.range_edit.expect("range event defers to host");
		assert_eq!((request.kind, request.text.as_str()), (kind, text));
		assert_eq!(frame::inst_field_text(&typed, KEY_A).as_deref(), Some("alpha"));
		assert_eq!(frame::inst_field_text(&typed, KEY_B).as_deref(), Some("bravo"));
		assert_eq!(frame::inst_field_text(&typed, KEY_C).as_deref(), Some("charlie"));
		assert!(frame::inst_get_range(&typed).is_some());
	}

	let mut local = instance();
	assert!(frame::inst_set_caret(&mut local, KEY_C, 2, 2));
	let effect = frame::inst_dispatch(&mut local, &event(dispatch::E_TEXT, "", "X", 0));
	assert!(effect.range_edit.is_none());
	assert_eq!(frame::inst_field_text(&local, KEY_C).as_deref(), Some("chXarlie"));

	let mut clicked = instance();
	establish_pointer_range(&mut clicked);
	let down = pointer(&clicked, B, 3, 0);
	frame::inst_dispatch(&mut clicked, &down);
	release(&mut clicked, &down);
	assert!(frame::inst_get_range(&clicked).is_none(), "plain click clears range metadata");
	let caret = frame::inst_get_caret(&clicked, KEY_B).expect("clicked field is bound");
	assert_eq!((caret.caret, caret.anchor), (3, 3), "plain click collapses selection");
	let painted = frame::inst_frame(&mut clicked, 1.0);
	assert!(band_widths(&painted, A).is_empty());
	assert!(band_widths(&painted, B).is_empty());
	assert!(band_widths(&painted, C).is_empty());
}

/// After a Shift+Arrow boundary no-op, the host can focus the next field with
/// an edge-anchored local selection; the kernel composes it with the source.
pub fn test_host_composed_boundary_range_preserves_source_band() {
	let mut instance = instance();
	assert!(frame::inst_set_caret(&mut instance, KEY_A, 5, 2));
	frame::inst_dispatch(
		&mut instance,
		&event(dispatch::E_KEY_DOWN, "ArrowRight", "", dispatch::M_SHIFT),
	);
	assert!(frame::inst_set_caret(&mut instance, KEY_B, 2, 0));

	let (anchor, head) = frame::inst_get_range(&instance).expect("host-composed range");
	assert_eq!((anchor.key.as_str(), anchor.offset), (KEY_A, 2));
	assert_eq!((head.key.as_str(), head.offset), (KEY_B, 2));
	assert_eq!(
		frame::inst_get_caret(&instance, KEY_A).map(|state| (state.anchor, state.caret)),
		Some((2, 5)),
		"source endpoint persists after destination focus"
	);
	assert_eq!(
		frame::inst_get_caret(&instance, KEY_B).map(|state| (state.anchor, state.caret)),
		Some((0, 2)),
		"destination paints its edge-anchored partial selection"
	);
	let painted = frame::inst_frame(&mut instance, 1.0);
	assert_eq!(band_widths(&painted, A).len(), 1);
	assert_eq!(band_widths(&painted, B).len(), 1);

	assert!(frame::inst_clear_range(&mut instance));
	assert!(!frame::inst_clear_range(&mut instance), "clear is idempotent");
}

fn virtual_field_doc() -> slir::Doc {
	let mut doc = crate::test_list::virtual_list_doc();
	doc.node_kind[2] = slir::K_TEXT;
	doc.node_flags[2] = slir::F_DETACHED | slir::F_FOCUSABLE | slir::F_NOWRAP;
	doc.node_first[2] = slir::NONE;
	doc.attr_index[3] = 6;
	doc.attr_index[4] = 6;
	doc.sign_name.push(0);
	doc.sign_node.push(2);
	doc.sign_trigger.push(dispatch::TR_CHANGE);
	doc
}

fn text_value(text: &str) -> frame::ParamValue {
	frame::ParamValue { kind: 0, num: 0.0, s: text.into(), rgba: 0, sym: String::new() }
}

fn virtual_field_by_item(instance: &frame::Instance, item_key: &str) -> u32 {
	instance
		.sc
		.entries
		.iter()
		.map(|entry| entry.node)
		.find(|&node| {
			list::base(&instance.st.lists, &instance.doc, node) == 2
				&& list::item_key_ref(&instance.st.lists, &instance.doc, node) == item_key
		})
		.unwrap_or_else(|| {
			panic!(
				"keyed virtual field '{item_key}' is absent; scene={:?}",
				instance
					.sc
					.entries
					.iter()
					.map(|entry| (
						list::base(&instance.st.lists, &instance.doc, entry.node),
						list::item_key(&instance.st.lists, &instance.doc, entry.node),
						scene::key_of(&instance.doc, &instance.st.lists, entry.node),
					))
					.collect::<Vec<_>>()
			)
		})
}

/// Canonical endpoint keys survive keyed-list reorder and temporary virtual
/// de-materialization; paint resolves the retained identity against each frame.
pub fn test_keyed_list_reorder_and_virtualization_keep_range_identity() {
	let mut instance = frame::inst_shell();
	instance.doc = virtual_field_doc();
	frame::inst_init(&mut instance);
	frame::inst_set_env(&mut instance, 120.0, 500.0, 0, false, false);
	assert!(frame::inst_set_list_len(&mut instance, 0, "", 20));
	for index in 0..20 {
		let key = if index == 2 {
			"k/02".to_owned()
		} else {
			format!("k{index:02}")
		};
		let text = format!("item{index:02}");
		assert!(frame::inst_set_list_key(&mut instance, 0, "", index, &key));
		assert!(
			frame::inst_set_list_field(&mut instance, 0, "", index, "label", &text_value(&text),)
		);
	}
	frame::inst_frame(&mut instance, 0.0);

	let anchor_node = virtual_field_by_item(&instance, "k/02");
	let head_node = virtual_field_by_item(&instance, "k06");
	let anchor_key = scene::key_of(&instance.doc, &instance.st.lists, anchor_node);
	let head_key = scene::key_of(&instance.doc, &instance.st.lists, head_node);
	assert!(anchor_key.contains("%2F"), "canonical locator escapes item-key separators");
	assert!(frame::inst_set_caret(&mut instance, &anchor_key, 6, 1));
	assert!(frame::inst_set_caret(&mut instance, &head_key, 2, 0));
	let range = frame::inst_get_range(&instance).expect("initial keyed range");
	assert_eq!((range.0.offset, range.1.offset), (1, 2));
	assert_eq!((&range.0.key, &range.1.key), (&anchor_key, &head_key));

	// Swap stable identities through the host API's permitted transient key
	// duplication; the final key set is unique.
	assert!(frame::inst_set_list_key(&mut instance, 0, "", 2, "__swap"));
	assert!(frame::inst_set_list_key(&mut instance, 0, "", 6, "k/02"));
	assert!(frame::inst_set_list_key(&mut instance, 0, "", 2, "k06"));
	let reordered = frame::inst_frame(&mut instance, 1.0);
	let anchor_node = scene::node_by_key(&instance.doc, &instance.st.lists, &anchor_key);
	let head_node = scene::node_by_key(&instance.doc, &instance.st.lists, &head_key);
	assert_eq!(list::item_ix(&instance.st.lists, &instance.doc, anchor_node), 6);
	assert_eq!(list::item_ix(&instance.st.lists, &instance.doc, head_node), 2);
	let range = frame::inst_get_range(&instance).expect("range survives keyed reorder");
	assert_eq!((&range.0.key, &range.1.key), (&anchor_key, &head_key));
	assert_eq!((range.0.offset, range.1.offset), (1, 2));
	assert!(!band_widths(&reordered, anchor_node).is_empty());
	assert!(!band_widths(&reordered, head_node).is_empty());

	// Move only the anchor identity outside the retained virtual window.
	assert!(frame::inst_set_list_key(&mut instance, 0, "", 6, "__window"));
	assert!(frame::inst_set_list_key(&mut instance, 0, "", 15, "k/02"));
	frame::inst_set_env(&mut instance, 120.0, 60.0, 0, false, false);
	assert!(frame::inst_set_list_key(&mut instance, 0, "", 6, "k15"));
	let windowed = frame::inst_frame(&mut instance, 2.0);
	let anchor_node = scene::node_by_key(&instance.doc, &instance.st.lists, &anchor_key);
	assert!(scene::index_of(&instance.sc, anchor_node) < 0);
	assert!(scene::index_of(&instance.sc, head_node) >= 0);
	let range = frame::inst_get_range(&instance).expect("de-windowed identity remains retained");
	assert_eq!((&range.0.key, &range.1.key), (&anchor_key, &head_key));
	assert_eq!((range.0.offset, range.1.offset), (1, 2));
	assert!(band_widths(&windowed, anchor_node).is_empty());
	assert!(!band_widths(&windowed, head_node).is_empty());

	assert!(frame::inst_set_list_len(&mut instance, 0, "", 3));
	frame::inst_frame(&mut instance, 3.0);
	assert!(
		frame::inst_get_range(&instance).is_none(),
		"truncating an endpoint identity invalidates the range"
	);
}
