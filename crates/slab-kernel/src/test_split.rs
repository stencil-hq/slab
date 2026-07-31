//! Focused n-ary split layout, persistence, sash, and dispatch contracts.

use crate::{dispatch, flatten::FrameOp, frame, scene, slir};

fn add_num(doc: &mut slir::Doc, attr: u32, tag: u32, num: f64) {
	let value = u32::try_from(doc.aval_tag.len()).expect("fixture value count");
	doc.aval_tag.push(tag);
	doc.aval_lo.push(0);
	doc.aval_hi.push(0);
	doc.aval_num.push(num);
	doc.attr_id.push(attr);
	doc.attr_val.push(value);
}

fn add_paint(doc: &mut slir::Doc, attr: u32, paint: u32) {
	let value = u32::try_from(doc.aval_tag.len()).expect("fixture value count");
	doc.aval_tag.push(slir::T_PAINT_SOLID);
	doc.aval_lo.push(paint);
	doc.aval_hi.push(0);
	doc.aval_num.push(0.0);
	doc.attr_id.push(attr);
	doc.attr_val.push(value);
}

fn finish_attrs(doc: &mut slir::Doc) {
	doc.attr_index
		.push(i32::try_from(doc.attr_id.len()).expect("fixture attr count"));
}

fn split_doc(seeds: [Option<f64>; 3], max: [f64; 3], paint: bool) -> slir::Doc {
	let mut doc = slir::doc_new();
	doc.ok = true;
	doc.strs.extend([
		String::new(),
		"#root".into(),
		"#root/#a".into(),
		"#root/#b".into(),
		"#root/#c".into(),
		"resized".into(),
	]);
	doc.node_kind
		.extend([slir::K_ROW, slir::K_COL, slir::K_COL, slir::K_COL]);
	doc.node_flags.extend([slir::F_SPLITS, 0, 0, 0]);
	doc.node_parent.extend([slir::NONE, 0, 0, 0]);
	doc.node_first
		.extend([1, slir::NONE, slir::NONE, slir::NONE]);
	doc.node_next.extend([slir::NONE, 2, 3, slir::NONE]);
	doc.node_key.extend([1, 2, 3, 4]);
	doc.node_id.resize(4, 0);
	doc.node_line.resize(4, 1);

	doc.attr_index.push(0);
	add_num(&mut doc, slir::A_W, slir::T_SIZE_FIXED, 300.0);
	add_num(&mut doc, slir::A_H, slir::T_SIZE_FIXED, 80.0);
	add_num(&mut doc, slir::A_SPLIT_W, slir::T_NUM, 4.0);
	if paint {
		add_paint(&mut doc, slir::A_SPLIT_FG, 0xff00_00ff);
	}
	finish_attrs(&mut doc);
	for index in 0..3 {
		if let Some(seed) = seeds[index] {
			add_num(&mut doc, slir::A_W, slir::T_SIZE_FIXED, seed);
		}
		add_num(&mut doc, slir::A_MIN_W, slir::T_NUM, 40.0);
		if max[index].is_finite() {
			add_num(&mut doc, slir::A_MAX_W, slir::T_NUM, max[index]);
		}
		finish_attrs(&mut doc);
	}
	doc.sign_name.push(5);
	doc.sign_node.push(0);
	doc.sign_trigger.push(dispatch::TR_RESIZE);
	doc
}

fn instance(seeds: [Option<f64>; 3], max: [f64; 3], paint: bool) -> frame::Instance {
	let mut instance = frame::inst_shell();
	instance.doc = split_doc(seeds, max, paint);
	frame::inst_init(&mut instance);

	frame::inst_set_env(&mut instance, 300.0, 80.0, 0, false, false);
	frame::inst_frame(&mut instance, 0.0);
	instance
}
fn each_split_doc() -> slir::Doc {
	let mut doc = slir::doc_new();
	doc.ok = true;
	doc.strs.extend([
		String::new(),
		"#root".into(),
		"#root/panes".into(),
		"pane".into(),
		"panes".into(),
	]);
	let list_default = crate::test_list::aval(&mut doc, slir::T_LIST_DEFAULT, 0, 3, 0.0);
	let each = crate::test_list::aval(&mut doc, slir::T_NUM, 0, 0, 0.0);
	doc.parm_name.push(4);
	doc.parm_type.push(6);
	doc.parm_default.push(list_default);
	doc.parm_enum_off.push(0);
	doc.parm_enum_len.push(0);
	doc.parm_site_off.push(0);
	doc.parm_site_len.push(0);
	doc.list_param.push(0);
	doc.list_field_off.push(0);
	doc.list_field_len.push(0);
	doc.list_item_field_off.extend([0, 0, 0]);
	doc.list_item_field_len.extend([0, 0, 0]);
	doc.node_kind
		.extend([slir::K_ROW, slir::K_EACH, slir::K_COL]);
	doc.node_flags.extend([slir::F_SPLITS, 0, slir::F_DETACHED]);
	doc.node_parent.extend([slir::NONE, 0, 1]);
	doc.node_first.extend([1, 2, slir::NONE]);
	doc.node_next.extend([slir::NONE, slir::NONE, slir::NONE]);
	doc.node_key.extend([1, 2, 3]);
	doc.node_id.resize(3, 0);
	doc.node_line.resize(3, 1);
	doc.attr_index.push(0);
	add_num(&mut doc, slir::A_W, slir::T_SIZE_FIXED, 300.0);
	add_num(&mut doc, slir::A_H, slir::T_SIZE_FIXED, 80.0);
	finish_attrs(&mut doc);
	doc.attr_id.push(slir::A_EACH);
	doc.attr_val.push(each);
	finish_attrs(&mut doc);
	add_num(&mut doc, slir::A_MIN_W, slir::T_NUM, 40.0);
	finish_attrs(&mut doc);
	doc
}

fn nested_each_split_doc() -> slir::Doc {
	let mut doc = slir::doc_new();
	doc.ok = true;
	doc.strs.extend([
		String::new(),
		"#root".into(),
		"#root/groups".into(),
		"split".into(),
		"kids".into(),
		"pane".into(),
		"groups".into(),
		"children".into(),
	]);
	let groups_default = crate::test_list::aval(&mut doc, slir::T_LIST_DEFAULT, 0, 1, 0.0);
	let children_default = crate::test_list::aval(&mut doc, slir::T_LIST_DEFAULT, 1, 2, 0.0);
	let groups_each = crate::test_list::aval(&mut doc, slir::T_NUM, 0, 0, 0.0);
	let children_each = crate::test_list::aval(&mut doc, slir::T_NUM, 0, 0, 1.0);
	doc.parm_name.extend([6, 7]);
	doc.parm_type.extend([6, 6]);
	doc.parm_default.extend([groups_default, children_default]);
	doc.parm_enum_off.extend([0, 0]);
	doc.parm_enum_len.extend([0, 0]);
	doc.parm_site_off.extend([0, 0]);
	doc.parm_site_len.extend([0, 0]);
	doc.list_param.extend([0, 1]);
	doc.list_field_off.extend([0, 0]);
	doc.list_field_len.extend([0, 0]);
	doc.list_item_field_off.extend([0, 0, 0]);
	doc.list_item_field_len.extend([0, 0, 0]);
	doc.node_kind
		.extend([slir::K_ROW, slir::K_EACH, slir::K_ROW, slir::K_EACH, slir::K_COL]);
	doc.node_flags
		.extend([0, 0, slir::F_DETACHED | slir::F_SPLITS, 0, slir::F_DETACHED]);
	doc.node_parent.extend([slir::NONE, 0, 1, 2, 3]);
	doc.node_first.extend([1, 2, 3, 4, slir::NONE]);
	doc.node_next.resize(5, slir::NONE);
	doc.node_key.extend([1, 2, 3, 4, 5]);
	doc.node_id.resize(5, 0);
	doc.node_line.resize(5, 1);
	doc.attr_index.push(0);
	add_num(&mut doc, slir::A_W, slir::T_SIZE_FIXED, 300.0);
	add_num(&mut doc, slir::A_H, slir::T_SIZE_FIXED, 80.0);
	finish_attrs(&mut doc);
	doc.attr_id.push(slir::A_EACH);
	doc.attr_val.push(groups_each);
	finish_attrs(&mut doc);
	add_num(&mut doc, slir::A_W, slir::T_SIZE_FIXED, 300.0);
	add_num(&mut doc, slir::A_H, slir::T_SIZE_FIXED, 80.0);
	finish_attrs(&mut doc);
	doc.attr_id.push(slir::A_EACH);
	doc.attr_val.push(children_each);
	finish_attrs(&mut doc);
	finish_attrs(&mut doc);
	doc
}

fn extent(instance: &frame::Instance, node: u32) -> f64 {
	let index = usize::try_from(scene::index_of(&instance.sc, node)).expect("pane in scene");
	instance.sc.entries[index].w
}

fn extent_key(instance: &frame::Instance, key: &str) -> f64 {
	let node = scene::node_by_key(&instance.doc, &instance.st.lists, key);
	extent(instance, node)
}

fn sash(instance: &frame::Instance, left: &str) -> (u32, f64, f64) {
	let key = format!("{left}~sash");
	let node = scene::node_by_key(&instance.doc, &instance.st.lists, &key);
	let index = usize::try_from(scene::index_of(&instance.sc, node)).expect("sash in scene");
	let entry = &instance.sc.entries[index];
	(node, entry.x + entry.w / 2.0, entry.y + entry.h / 2.0)
}

const fn pointer(etype: u32, x: f64, y: f64, clicks: u32) -> dispatch::Event {
	dispatch::Event {
		etype,
		x,
		y,
		dx: 0.0,
		dy: 0.0,
		button: 0,
		clicks,
		key: String::new(),
		text: String::new(),
		clauses: Vec::new(),
		mods: 0,
	}
}

fn key(name: &str, mods: u32) -> dispatch::Event {
	dispatch::Event {
		etype: dispatch::E_KEY_DOWN,
		x: 0.0,
		y: 0.0,
		dx: 0.0,
		dy: 0.0,
		button: 0,
		clicks: 0,
		key: name.into(),
		text: String::new(),
		clauses: Vec::new(),
		mods,
	}
}

pub fn test_split_equal_default_and_ratio_seeding() {
	let equal = instance([None, None, None], [f64::INFINITY; 3], false);
	assert_eq!([extent(&equal, 1), extent(&equal, 2), extent(&equal, 3)], [100.0; 3]);
	let ratio = instance([Some(100.0), Some(200.0), Some(100.0)], [f64::INFINITY; 3], false);
	assert_eq!([extent(&ratio, 1), extent(&ratio, 2), extent(&ratio, 3)], [75.0, 150.0, 75.0]);
}

pub fn test_split_host_api_reorder_and_removal_persistence() {
	let mut instance = instance([None, None, None], [f64::INFINITY; 3], false);
	assert!(frame::inst_set_split(&mut instance, "#root/#a", 60.0));
	assert!(frame::inst_set_split(&mut instance, "#root/#b", 90.0));
	assert!(frame::inst_set_split(&mut instance, "#root/#c", 150.0));
	frame::inst_frame(&mut instance, 1.0);
	assert_eq!(frame::inst_get_split(&instance, "#root/#b"), 90.0);
	instance.doc.node_first[0] = 3;
	instance.doc.node_next[3] = 1;
	instance.doc.node_next[1] = 2;
	instance.doc.node_next[2] = slir::NONE;
	instance.dirty = true;
	frame::inst_frame(&mut instance, 2.0);
	assert_eq!([extent(&instance, 1), extent(&instance, 2), extent(&instance, 3)], [
		60.0, 90.0, 150.0
	]);
	instance.doc.node_first[0] = 1;
	instance.doc.node_next[1] = 3;
	instance.doc.node_next[3] = slir::NONE;
	instance.dirty = true;
	frame::inst_frame(&mut instance, 3.0);
	assert!((extent(&instance, 1) - 85.714_285).abs() < 0.001);
	assert!((extent(&instance, 3) - 214.285_714).abs() < 0.001);
}

pub fn test_split_each_keys_persist_across_reorder_and_removal() {
	let mut instance = frame::inst_shell();
	instance.doc = each_split_doc();
	frame::inst_init(&mut instance);
	frame::inst_set_env(&mut instance, 300.0, 80.0, 0, false, false);
	for (index, key) in ["A", "B", "C"].iter().enumerate() {
		assert!(frame::inst_set_list_key(&mut instance, 0, "", index as i32, key));
	}
	frame::inst_frame(&mut instance, 0.0);
	assert!(
		!frame::inst_set_split(&mut instance, "#root/panes", 10.0),
		"each wrapper is not a pane"
	);
	for (key, size) in
		[("#root/panes~A/pane", 60.0), ("#root/panes~B/pane", 90.0), ("#root/panes~C/pane", 150.0)]
	{
		assert!(frame::inst_set_split(&mut instance, key, size));
	}
	frame::inst_frame(&mut instance, 1.0);
	for (index, key) in [(0, "_a"), (2, "_c"), (0, "C"), (1, "_b"), (2, "B"), (1, "A")] {
		assert!(frame::inst_set_list_key(&mut instance, 0, "", index, key));
	}
	frame::inst_frame(&mut instance, 2.0);
	assert_eq!(extent_key(&instance, "#root/panes~A/pane"), 60.0);
	assert_eq!(extent_key(&instance, "#root/panes~B/pane"), 90.0);
	assert_eq!(extent_key(&instance, "#root/panes~C/pane"), 150.0);
	assert!(frame::inst_set_list_len(&mut instance, 0, "", 2));
	frame::inst_frame(&mut instance, 3.0);
	assert!((extent_key(&instance, "#root/panes~A/pane") - 85.714_285).abs() < 0.001);
	assert!((extent_key(&instance, "#root/panes~C/pane") - 214.285_714).abs() < 0.001);
}

pub fn test_split_inserted_unset_pane_takes_equal_share() {
	let mut instance = frame::inst_shell();
	instance.doc = each_split_doc();
	frame::inst_init(&mut instance);
	frame::inst_set_env(&mut instance, 300.0, 80.0, 0, false, false);
	assert!(frame::inst_set_list_len(&mut instance, 0, "", 2));
	for (index, key) in ["A", "B"].iter().enumerate() {
		assert!(frame::inst_set_list_key(&mut instance, 0, "", index as i32, key));
	}
	frame::inst_frame(&mut instance, 0.0);
	assert!(frame::inst_set_split(&mut instance, "#root/panes~A/pane", 120.0));
	assert!(frame::inst_set_split(&mut instance, "#root/panes~B/pane", 180.0));
	frame::inst_frame(&mut instance, 1.0);
	assert!(frame::inst_set_list_len(&mut instance, 0, "", 3));
	assert!(frame::inst_set_list_key(&mut instance, 0, "", 2, "C"));
	frame::inst_frame(&mut instance, 2.0);
	assert_eq!(extent_key(&instance, "#root/panes~A/pane"), 80.0);
	assert_eq!(extent_key(&instance, "#root/panes~B/pane"), 120.0);
	assert_eq!(extent_key(&instance, "#root/panes~C/pane"), 100.0);
}

pub fn test_split_nested_each_pane_host_api_roundtrip() {
	let mut instance = frame::inst_shell();
	instance.doc = nested_each_split_doc();
	frame::inst_init(&mut instance);
	frame::inst_set_env(&mut instance, 300.0, 80.0, 0, false, false);
	assert!(frame::inst_set_list_key(&mut instance, 0, "", 0, "G"));
	assert!(frame::inst_set_list_key(&mut instance, 1, "", 0, "A"));
	assert!(frame::inst_set_list_key(&mut instance, 1, "", 1, "B"));
	frame::inst_frame(&mut instance, 0.0);
	let pane = instance
		.sc
		.entries
		.iter()
		.find(|entry| crate::list::base(&instance.st.lists, &instance.doc, entry.node) == 4)
		.expect("nested each pane materialized");
	let key = scene::key_of(&instance.doc, &instance.st.lists, pane.node);
	assert!(key.contains("~G/") && key.contains("~A/"), "full nested item identity: {key}");
	assert!(frame::inst_set_split(&mut instance, &key, 123.0));
	assert_eq!(frame::inst_get_split(&instance, &key), 123.0);
	let mut replay = frame::inst_shell();
	replay.doc = nested_each_split_doc();
	frame::inst_init(&mut replay);
	assert!(frame::inst_set_list_key(&mut replay, 0, "", 0, "G"));
	assert!(frame::inst_set_list_key(&mut replay, 1, "", 0, "A"));
	assert!(frame::inst_set_list_key(&mut replay, 1, "", 1, "B"));
	assert!(frame::inst_set_split(&mut replay, &key, 77.0), "buffered restore before first solve");
	assert_eq!(frame::inst_get_split(&replay, &key), 77.0);
}

pub fn test_split_drag_cascades_clamps_and_emits_resize() {
	let mut instance = instance([None, None, None], [f64::INFINITY, 105.0, f64::INFINITY], false);
	let (_, x, y) = sash(&instance, "#root/#b");
	frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_DOWN, x, y, 1));
	let moved =
		frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_MOVE, x + 100.0, y, 1));
	assert!(moved.repaint);
	assert!(moved.sig_name.is_empty(), "split resize signals only at gesture end");
	frame::inst_frame(&mut instance, 1.0);
	assert_eq!([extent(&instance, 1), extent(&instance, 2), extent(&instance, 3)], [
		155.0, 105.0, 40.0
	]);
	let released =
		frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_UP, x + 100.0, y, 1));
	assert_eq!(released.sig_name, [5]);
	assert_eq!(released.sig_text, ["105"]);
	assert_eq!(released.sig_meta[0].key, "#root/#b~sash");
}

pub fn test_split_drag_clamps_to_snapshotted_bounds() {
	let mut instance = instance([None, None, None], [f64::INFINITY; 3], false);
	let (_, x, y) = sash(&instance, "#root/#b");
	frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_DOWN, x, y, 1));
	let resolved = instance
		.st
		.rs
		.iter_mut()
		.rev()
		.find(|rule| rule.node == 3)
		.expect("third pane style");
	resolved.min_w = 90.0;
	frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_MOVE, x + 30.0, y, 1));
	assert_eq!(
		frame::inst_get_split(&instance, "#root/#c"),
		70.0,
		"gesture uses the pointer-down minimum snapshot"
	);
}

pub fn test_split_double_click_evening_and_arrow_keys() {
	let mut instance = instance([None, None, None], [f64::INFINITY; 3], false);
	assert!(frame::inst_set_split(&mut instance, "#root/#a", 160.0));
	assert!(frame::inst_set_split(&mut instance, "#root/#b", 40.0));
	assert!(frame::inst_set_split(&mut instance, "#root/#c", 100.0));
	frame::inst_frame(&mut instance, 1.0);
	let (_, x, y) = sash(&instance, "#root/#a");
	let even = frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_DOWN, x, y, 2));
	assert_eq!(even.sig_text, ["100"]);
	frame::inst_frame(&mut instance, 2.0);
	assert_eq!([extent(&instance, 1), extent(&instance, 2)], [100.0, 100.0]);
	let adjusted = frame::inst_dispatch(&mut instance, &key("ArrowRight", 0));
	assert_eq!(adjusted.sig_text, ["108"]);
	assert_eq!(adjusted.sig_meta[0].key, "#root/#a~sash");
	let shifted = frame::inst_dispatch(&mut instance, &key("ArrowLeft", dispatch::M_SHIFT));
	assert_eq!(shifted.sig_text, ["107"]);
	let unbound = frame::inst_dispatch(&mut instance, &key("j", dispatch::M_CTRL));
	assert!(unbound.sig_name.is_empty(), "unbound shortcut from a synthetic sash is ignored");
}

pub fn test_split_sash_hover_cursor_and_paint() {
	let mut instance = instance([None, None, None], [f64::INFINITY; 3], true);
	let (sash, x, y) = sash(&instance, "#root/#a");
	let hover = frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_MOVE, x, y, 0));
	assert_eq!(hover.cursor, dispatch::CUR_COL_RESIZE);
	let painted = frame::inst_frame(&mut instance, 1.0);
	assert!(
		painted.ops.iter().any(
			|op| matches!(op, FrameOp::Rect(rect) if rect.node == sash && rect.bg == 0xff00_00ff)
		)
	);
	let sash_entry =
		&instance.sc.entries[usize::try_from(scene::index_of(&instance.sc, sash)).unwrap()];
	assert_eq!(sash_entry.w, 4.0);
	assert_eq!(sash_entry.x + sash_entry.w / 2.0, extent(&instance, 1));
}
