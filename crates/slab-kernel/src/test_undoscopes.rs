//! Host structural-transaction snapshot and restore contracts.

use crate::{
	dispatch, edit,
	frame::{self, FieldRun, FieldRuns},
	slir, style,
};

const ROOT: u32 = 0;
const A: u32 = 1;
const B: u32 = 2;
const KEY_A: &str = "#root/#a";
const KEY_B: &str = "#root/#b";

fn add_fixed(doc: &mut slir::Doc, attr: u32, value: f64) {
	let value_index = u32::try_from(doc.aval_tag.len()).expect("fixture value count fits u32");
	doc.aval_tag.push(slir::T_SIZE_FIXED);
	doc.aval_lo.push(0);
	doc.aval_hi.push(0);
	doc.aval_num.push(value);
	doc.attr_id.push(attr);
	doc.attr_val.push(value_index);
}
fn add_str(doc: &mut slir::Doc, attr: u32, value: u32) {
	let value_index = u32::try_from(doc.aval_tag.len()).expect("fixture value count fits u32");
	doc.aval_tag.push(slir::T_STR);
	doc.aval_lo.push(value);
	doc.aval_hi.push(0);
	doc.aval_num.push(0.0);
	doc.attr_id.push(attr);
	doc.attr_val.push(value_index);
}

fn finish_attrs(doc: &mut slir::Doc) {
	doc.attr_index
		.push(i32::try_from(doc.attr_id.len()).expect("fixture attr count fits i32"));
}

fn transaction_doc() -> slir::Doc {
	let mut doc = slir::doc_new();
	doc.ok = true;
	doc.strs.extend([
		String::new(),
		"#root".into(),
		KEY_A.into(),
		KEY_B.into(),
		"z:host-undo".into(),
		"host-undo".into(),
	]);
	doc.node_kind
		.extend([slir::K_COL, slir::K_TEXT, slir::K_TEXT]);
	doc.node_flags.extend([
		0,
		slir::F_FOCUSABLE | slir::F_NOWRAP,
		slir::F_FOCUSABLE | slir::F_NOWRAP,
	]);
	doc.node_parent.extend([slir::NONE, ROOT, ROOT]);
	doc.node_first.extend([A, slir::NONE, slir::NONE]);
	doc.node_next.extend([slir::NONE, B, slir::NONE]);
	doc.node_key.extend([1, 2, 3]);
	doc.node_id.resize(3, 0);
	doc.node_line.resize(3, 1);

	doc.attr_index.push(0);
	add_fixed(&mut doc, slir::A_W, 140.0);
	add_str(&mut doc, slir::A_KEYS, 4);
	finish_attrs(&mut doc);
	for _ in A..=B {
		add_fixed(&mut doc, slir::A_W, 120.0);
		add_fixed(&mut doc, slir::A_H, 24.0);
		finish_attrs(&mut doc);
	}
	doc.sign_name.extend([0, 0, 5]);
	doc.sign_node.extend([A, B, ROOT]);
	doc.sign_trigger
		.extend([dispatch::TR_CHANGE, dispatch::TR_CHANGE, dispatch::TR_KEY_ACTIVATE]);
	doc
}

fn instance() -> frame::Instance {
	let mut instance = frame::inst_shell();
	instance.doc = transaction_doc();
	frame::inst_init(&mut instance);
	frame::inst_set_env(&mut instance, 200.0, 100.0, 0, false, false);
	style::field_set(&mut instance.st, A, "");
	style::field_set(&mut instance.st, B, "");
	frame::inst_frame(&mut instance, 0.0);
	instance
}

fn runs(entries: &[(u32, i32, i32)]) -> FieldRuns {
	FieldRuns {
		revision: 0,
		runs:     entries
			.iter()
			.map(|&(style, start, end)| FieldRun { style, start, end })
			.collect(),
	}
}

fn set_goal(instance: &mut frame::Instance, node: u32, goal_x: f64) {
	let edit_index = usize::try_from(dispatch::ed_ix(&instance.ds, node)).expect("field is bound");
	instance.ds.ed[edit_index].goal_x = goal_x;
}

fn edit_index(instance: &frame::Instance, node: u32) -> usize {
	usize::try_from(dispatch::ed_ix(&instance.ds, node)).expect("field is bound")
}

/// Enter split and host undo preserve the complete original rich-field state.
pub fn test_enter_split_restore_is_exact_and_resets_local_history() {
	let mut instance = instance();
	assert!(frame::inst_set_field_text(&mut instance, KEY_A, "abcdef"));
	assert!(frame::inst_set_field_runs(
		&mut instance,
		KEY_A,
		&runs(&[(edit::STYLE_BOLD, 1, 5), (edit::STYLE_CODE, 2, 4)]),
	));
	assert!(frame::inst_set_caret(&mut instance, KEY_A, 3, 1));
	set_goal(&mut instance, A, 37.5);
	let snapshot = frame::inst_snapshot_fields(&instance, &[KEY_A]).expect("bound field");
	let captured = snapshot.fields[0].clone();

	assert!(frame::inst_set_field_text(&mut instance, KEY_A, "abc"));
	assert!(frame::inst_set_field_runs(
		&mut instance,
		KEY_A,
		&runs(&[(edit::STYLE_BOLD, 1, 3), (edit::STYLE_CODE, 2, 3)]),
	));
	assert!(frame::inst_set_field_text(&mut instance, KEY_B, "def"));
	assert!(frame::inst_set_field_runs(
		&mut instance,
		KEY_B,
		&runs(&[(edit::STYLE_BOLD, 0, 2), (edit::STYLE_CODE, 0, 1)]),
	));
	assert!(frame::inst_commit_fields(&mut instance, &[KEY_A, KEY_B]));

	assert!(frame::inst_restore_fields(&mut instance, &snapshot));
	assert_eq!(frame::inst_field_text(&instance, KEY_A).as_deref(), Some("abcdef"));
	assert_eq!(frame::inst_field_runs(&instance, KEY_A), Some(captured.runs.clone()));
	let caret = frame::inst_get_caret(&instance, KEY_A).expect("restored caret");
	assert_eq!((caret.caret, caret.anchor, caret.goal_x), (3, 1, 37.5));
	let state = &instance.ds.ed[edit_index(&instance, A)];
	assert_eq!(state.revision, captured.runs.revision, "restore adopts captured revision");
	assert!(state.undo.is_empty(), "restored state is the new undo baseline");
	assert!(state.redo.is_empty(), "restore discards redo history");

	let mut undo = crate::test_edit::host_field_event(dispatch::E_KEY_DOWN, "z", "");
	undo.mods = dispatch::M_CTRL;
	let effects = frame::inst_dispatch(&mut instance, &undo);
	assert_eq!(frame::inst_field_text(&instance, KEY_A).as_deref(), Some("abcdef"));
	assert!(
		effects.sig_name.contains(&5),
		"empty kernel history bubbles Ctrl+Z to the host structural undo binding"
	);
}

/// Backspace merge and host undo restore both styled fields independently.
pub fn test_backspace_merge_restore_restores_both_fields() {
	let mut instance = instance();
	assert!(frame::inst_set_field_text(&mut instance, KEY_A, "one"));
	assert!(frame::inst_set_field_runs(&mut instance, KEY_A, &runs(&[(edit::STYLE_BOLD, 0, 3)]),));
	assert!(frame::inst_set_caret(&mut instance, KEY_A, 2, 2));
	assert!(frame::inst_set_field_text(&mut instance, KEY_B, "two"));
	assert!(frame::inst_set_field_runs(&mut instance, KEY_B, &runs(&[(edit::STYLE_CODE, 0, 3)]),));
	assert!(frame::inst_set_caret(&mut instance, KEY_B, 1, 0));
	let snapshot =
		frame::inst_snapshot_fields(&instance, &[KEY_A, KEY_B]).expect("both fields bound");

	assert!(frame::inst_set_field_text(&mut instance, KEY_A, "onetwo"));
	assert!(frame::inst_set_field_runs(
		&mut instance,
		KEY_A,
		&runs(&[(edit::STYLE_BOLD, 0, 3), (edit::STYLE_CODE, 3, 6)]),
	));
	assert!(frame::inst_set_field_text(&mut instance, KEY_B, ""));
	assert!(frame::inst_set_field_runs(&mut instance, KEY_B, &runs(&[])));
	assert!(frame::inst_commit_fields(&mut instance, &[KEY_A]));

	assert!(frame::inst_restore_fields(&mut instance, &snapshot));
	for field in &snapshot.fields {
		assert_eq!(frame::inst_field_text(&instance, &field.locator), Some(field.text.clone()));
		assert_eq!(frame::inst_field_runs(&instance, &field.locator), Some(field.runs.clone()));
		let caret = frame::inst_get_caret(&instance, &field.locator).expect("restored caret");
		assert_eq!(
			(caret.caret, caret.anchor, caret.goal_x),
			(field.caret, field.anchor, field.goal_x)
		);
	}
	for node in [A, B] {
		let state = &instance.ds.ed[edit_index(&instance, node)];
		assert!(state.undo.is_empty(), "restored field has no local undo");
		assert!(state.redo.is_empty(), "restored field has no local redo");
	}
	let mut undo = crate::test_edit::host_field_event(dispatch::E_KEY_DOWN, "z", "");
	undo.mods = dispatch::M_CTRL;
	let effects = frame::inst_dispatch(&mut instance, &undo);
	assert_eq!(frame::inst_field_text(&instance, KEY_A).as_deref(), Some("one"));
	assert_eq!(frame::inst_field_text(&instance, KEY_B).as_deref(), Some("two"));
	assert!(effects.sig_name.contains(&5), "empty kernel history bubbles merge undo to the host");
}

/// Failed capture or restore is all-or-nothing, including history barriers.
pub fn test_snapshot_and_restore_reject_unresolvable_locator_atomically() {
	let mut instance = instance();
	assert!(frame::inst_set_field_text(&mut instance, KEY_A, "alpha"));
	assert!(frame::inst_set_field_text(&mut instance, KEY_B, "bravo"));
	let a = edit_index(&instance, A);
	assert!(edit::insert(&mut instance.ds.ed[a], "!"));
	let before_text = instance.ds.ed[a].text.clone();
	let before_undo = instance.ds.ed[a].undo.clone();
	assert!(frame::inst_snapshot_fields(&instance, &[KEY_A, "#root/#missing"]).is_none());
	assert_eq!(instance.ds.ed[a].text, before_text);
	assert_eq!(instance.ds.ed[a].undo, before_undo, "failed capture writes no barrier");

	let mut snapshot =
		frame::inst_snapshot_fields(&instance, &[KEY_A, KEY_B]).expect("valid capture");
	snapshot.fields[1].locator = "#root/#missing".into();
	let before_a = frame::inst_field_text(&instance, KEY_A);
	let before_b = frame::inst_field_text(&instance, KEY_B);
	instance.dirty = false;
	assert!(!frame::inst_restore_fields(&mut instance, &snapshot));
	assert_eq!(frame::inst_field_text(&instance, KEY_A), before_a);
	assert_eq!(frame::inst_field_text(&instance, KEY_B), before_b);
	assert!(!instance.dirty, "failed restore does not request a frame");
}
/// Pure capture permits a host to abort without damaging ordinary field undo.
pub fn test_snapshot_abort_preserves_local_history() {
	let mut instance = instance();
	assert!(frame::inst_set_field_text(&mut instance, KEY_A, "alpha"));
	let a = edit_index(&instance, A);
	assert!(edit::insert(&mut instance.ds.ed[a], "!"));
	let before_undo = instance.ds.ed[a].undo.clone();
	let before_redo = instance.ds.ed[a].redo.clone();

	let snapshot = frame::inst_snapshot_fields(&instance, &[KEY_A]).expect("bound field");
	assert_eq!(snapshot.fields[0].text, "alpha!");
	assert_eq!(instance.ds.ed[a].undo, before_undo);
	assert_eq!(instance.ds.ed[a].redo, before_redo);
	assert!(edit::undo(&mut instance.ds.ed[a]), "pre-snapshot edit remains undoable");
	assert_eq!(instance.ds.ed[a].text, "alpha");
}

/// Commit is a hard barrier even when the host ultimately changed no structure.
pub fn test_commit_without_structural_change_barriers_history() {
	let mut instance = instance();
	assert!(frame::inst_set_field_text(&mut instance, KEY_A, "alpha"));
	assert!(frame::inst_set_caret(&mut instance, KEY_A, 5, 5));
	let text = crate::test_edit::host_field_event(dispatch::E_TEXT, "", "!");
	frame::inst_dispatch(&mut instance, &text);
	assert!(!instance.ds.ed[edit_index(&instance, A)].undo.is_empty());

	assert!(frame::inst_commit_fields(&mut instance, &[KEY_A]));
	let state = &instance.ds.ed[edit_index(&instance, A)];
	assert!(state.undo.is_empty());
	assert!(state.redo.is_empty());
	let mut undo = crate::test_edit::host_field_event(dispatch::E_KEY_DOWN, "z", "");
	undo.mods = dispatch::M_CTRL;
	let effects = frame::inst_dispatch(&mut instance, &undo);
	assert_eq!(frame::inst_field_text(&instance, KEY_A).as_deref(), Some("alpha!"));
	assert!(effects.sig_name.contains(&5), "barrier makes Ctrl+Z bubble to host undo");
}

/// A transaction snapshots only committed text and restoration cancels preedit.
pub fn test_restore_clears_active_composition() {
	let mut instance = instance();
	assert!(frame::inst_set_field_text(&mut instance, KEY_A, "seed"));
	assert!(frame::inst_set_focus(&mut instance, KEY_A, true));
	let start = crate::test_edit::host_field_event(dispatch::E_COMPOSITION_START, "", "");
	frame::inst_dispatch(&mut instance, &start);
	let update = crate::test_edit::host_field_event(dispatch::E_COMPOSITION_UPDATE, "", "候補");
	frame::inst_dispatch(&mut instance, &update);
	assert!(
		frame::inst_get_caret(&instance, KEY_A)
			.expect("active field")
			.composing
	);
	let snapshot = frame::inst_snapshot_fields(&instance, &[KEY_A]).expect("active field binds");
	assert_eq!(snapshot.fields[0].text, "seed", "preedit is excluded from committed text");

	assert!(frame::inst_restore_fields(&mut instance, &snapshot));
	let caret = frame::inst_get_caret(&instance, KEY_A).expect("restored field");
	assert!(!caret.composing);
	let state = &instance.ds.ed[edit_index(&instance, A)];
	assert!(state.compose.is_empty());
	assert!(state.compose_clauses.is_empty());
	assert_eq!(frame::inst_field_text(&instance, KEY_A).as_deref(), Some("seed"));
}
