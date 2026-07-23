//! Editing tests for cluster-bounded carets, ZWJ deletion, composition,
//! selection, and word operations.
//!
//! These cases were originally ported from the editing block in
//! `research/tests/test_app.py`.

use crate::{
    dispatch, edit, flatten, frame, layout,
    slir::{self, Doc},
    style, textm,
};

/// Verifies that caret movement and deletion respect grapheme clusters.
pub fn test_caret_respects_clusters() {
    // "ae\u{301}b"
    let mut es = edit::es_new(1, "ae\u{301}b");
    assert_eq!(es.caret, 4, "caret starts at end");
    edit::move_caret(&mut es, -1, false, false);
    assert_eq!(es.caret, 3, "left over b");
    edit::move_caret(&mut es, -1, false, false);
    assert_eq!(es.caret, 1, "never inside the cluster");
    assert!(edit::backspace(&mut es), "backspace changes text");
    assert_eq!(edit::text_str(&es), "e\u{301}b", "a removed");
    assert_eq!(es.caret, 0, "caret at 0");
}

/// Verifies that a family emoji joined with ZWJs is one caret stop.
pub fn test_zwj_emoji_is_one_stop() {
    // x + family ZWJ sequence + y
    let mut es = edit::es_new(1, "x\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}y");
    edit::move_caret(&mut es, -1, false, false);
    edit::move_caret(&mut es, -1, false, false);
    assert_eq!(es.caret, 1, "two stops back lands before the family");
    assert!(edit::del(&mut es), "delete consumes the family");
    assert_eq!(edit::text_str(&es), "xy", "family removed whole");
}

/// Verifies inline composition display and committed replacement text.
pub fn test_composition_update_then_commit() {
    let mut es = edit::es_new(1, "ab");
    es.caret = 1;
    es.anchor = 1;
    edit::composition_update(&mut es, "か");
    assert_eq!(edit::display_str(&es), "aかb", "inline compose");
    assert_eq!(edit::display_caret(&es), 2, "display caret after compose");
    let changed = edit::composition_end(&mut es, "漢字");
    assert!(changed, "commit changes text");
    assert_eq!(edit::text_str(&es), "a漢字b", "committed text");
    assert!(!es.composing, "composition cleared");
}

/// Verifies word movement and replacing a selection.
pub fn test_selection_and_words() {
    let mut es = edit::es_new(1, "hello brave world");
    edit::home(&mut es, false);
    edit::move_caret(&mut es, 1, false, true);
    assert_eq!(es.caret, 6, "word right");
    edit::move_caret(&mut es, -1, false, true);
    assert_eq!(es.caret, 0, "word left");
    edit::move_caret(&mut es, 1, false, true);
    assert_eq!(es.caret, 6, "word right after left");
    edit::end(&mut es, true);
    assert_eq!(
        (edit::sel_lo(&es), edit::sel_hi(&es)),
        (6, 17),
        "selection to end"
    );
    assert!(edit::insert(&mut es, "!"), "insert replaces selection");
    assert_eq!(edit::text_str(&es), "hello !");
}

/// Verifies that the editing core preserves inserted newlines.
pub fn test_insert_preserves_newlines() {
    let mut es = edit::es_new(1, "ab");
    es.caret = 1;
    es.anchor = 1;
    edit::insert(&mut es, "x\ny");
    assert_eq!(
        edit::text_str(&es),
        "ax\nyb",
        "edit core accepts newline input"
    );
    assert_eq!(es.caret, 4, "caret after insertion");
}

/// Verifies selection collapse, select-all, and selected deletion.
pub fn test_collapse_selection_on_move() {
    let mut es = edit::es_new(1, "abcd");
    edit::home(&mut es, false);
    edit::move_caret(&mut es, 1, true, false);
    edit::move_caret(&mut es, 1, true, false);
    assert_eq!(
        (edit::sel_lo(&es), edit::sel_hi(&es)),
        (0, 2),
        "shift-right selects"
    );
    edit::move_caret(&mut es, -1, false, false);
    assert_eq!((es.caret, es.anchor), (0, 0), "left collapses to lo");
    edit::select_all(&mut es);
    assert_eq!(edit::sel_hi(&es), 4, "select all");
    assert!(edit::backspace(&mut es), "backspace deletes selection");
    assert_eq!(edit::text_str(&es), "", "cleared");
}

/// Builds the fixed-width font document used by editing layout tests.
pub fn metric_doc() -> Doc {
    let mut doc = slir::doc_new();
    doc.font_family.push(0);
    doc.font_class.push(0);
    doc.font_weight.push(400);
    doc.font_upem.push(1000);
    doc.font_ascent.push(800);
    doc.font_descent.push(0_i32.wrapping_sub(200));
    doc.font_line_gap.push(0);
    doc.font_default_adv.push(500);
    doc.font_cmap_off.push(0);
    doc.font_cmap_len.push(0);
    doc
}

/// Measures editable text with the fixed test metrics and width.
pub fn text_layout(text: &str, max_w: f64) -> textm::TextLayout {
    textm::measure_text(
        &metric_doc(),
        0,
        10.0,
        1.2,
        0.0,
        text,
        max_w,
        true,
        false,
        -1,
    )
}

/// Verifies visual-line kills and word deletion boundaries.
pub fn test_kills_and_word_deletes() {
    let mut end_es = edit::es_new(1, "hello world\nnext");
    end_es.caret = 2;
    end_es.anchor = 2;
    let end_tl = text_layout(&end_es.text, 1000.0);
    assert!(
        edit::kill_end(&mut end_es, &end_tl),
        "kill to visual line end changes"
    );
    assert_eq!(
        end_es.text, "he\nnext",
        "line-end kill preserves hard newline"
    );

    let mut wrapped = edit::es_new(1, "aaa bbb");
    wrapped.caret = 1;
    wrapped.anchor = 1;
    let wrapped_tl = text_layout(&wrapped.text, 20.0);
    assert!(
        edit::kill_end(&mut wrapped, &wrapped_tl),
        "kill uses wrapped visual line end"
    );
    assert_eq!(
        wrapped.text, "a bbb",
        "wrapped kill stops before break separator"
    );

    let mut start_es = edit::es_new(1, "hello\nnext");
    start_es.caret = 9;
    start_es.anchor = 9;
    let start_tl = text_layout(&start_es.text, 1000.0);
    assert!(
        edit::kill_start(&mut start_es, &start_tl),
        "kill to visual line start changes"
    );
    assert_eq!(
        start_es.text, "hello\nt",
        "line-start kill uses source offsets"
    );

    let mut back = edit::es_new(1, "one two three");
    back.caret = 7;
    back.anchor = 7;
    assert!(edit::word_back(&mut back), "word backward changes");
    assert_eq!(
        back.text, "one  three",
        "word backward removes one space-delimited unit"
    );

    let mut forward = edit::es_new(1, "one two three");
    forward.caret = 4;
    forward.anchor = 4;
    assert!(edit::word_forward(&mut forward), "word forward changes");
    assert_eq!(
        forward.text, "one three",
        "word forward consumes following separator"
    );
}

/// Verifies typing coalescence, undo/redo, invalidation, and history bounds.
pub fn test_coalesced_undo_and_redo_invalidation() {
    let mut es = edit::es_new(1, "");
    for text in ["a", "b", " ", "c", "d"] {
        edit::insert(&mut es, text);
    }
    assert_eq!(es.text, "ab cd", "typed text");
    assert!(edit::undo(&mut es), "undo second word");
    assert_eq!(es.text, "ab ", "word after whitespace is one undo group");
    assert!(edit::undo(&mut es), "undo first group");
    assert_eq!(es.text, "", "first group returns to focus text");
    assert!(edit::redo(&mut es), "redo first group");
    assert_eq!(es.text, "ab ", "redo restores");
    edit::insert(&mut es, "x");
    assert!(!edit::redo(&mut es), "new edit clears redo");

    let mut capped = edit::es_new(2, "");
    for _ in 0..110 {
        edit::history_barrier(&mut capped);
        edit::insert(&mut capped, "x");
    }
    assert_eq!(capped.u_text.len(), 100, "undo history capped at 100");
}

/// Verifies that movement, selection, and word kills split undo runs.
pub fn test_movement_selection_and_word_kill_break_undo_runs() {
    let mut moved = edit::es_new(1, "");
    edit::insert(&mut moved, "a");
    edit::insert(&mut moved, "b");
    edit::move_caret(&mut moved, -1, false, false);
    edit::insert(&mut moved, "x");
    assert_eq!(moved.text, "axb", "insert after caret movement");
    assert!(
        edit::undo(&mut moved) && moved.text == "ab",
        "movement starts a distinct undo run"
    );
    assert!(
        edit::undo(&mut moved) && moved.text.is_empty(),
        "prior typing remains its own run"
    );

    let mut selected = edit::es_new(2, "");
    edit::insert(&mut selected, "abc");
    edit::select_all(&mut selected);
    edit::insert(&mut selected, "x");
    assert!(edit::undo(&mut selected), "selection replacement undoes");
    assert_eq!(selected.text, "abc", "select-all breaks the typing run");

    let mut killed = edit::es_new(3, "");
    edit::insert(&mut killed, "one two");
    edit::history_barrier(&mut killed); // Ctrl+W routing barrier
    assert!(edit::word_back(&mut killed), "Ctrl+W word deletion changes");
    assert!(
        edit::undo(&mut killed),
        "word kill has its own undo snapshot"
    );
    assert_eq!(killed.text, "one two", "word kill undo restores text");
    assert!(
        edit::undo(&mut killed) && killed.text.is_empty(),
        "typing before word kill stays separate"
    );
}

/// Verifies source-line maps and the last-solved layout lookup rule.
pub fn test_source_line_maps_and_layout_lookup() {
    let tl = text_layout("aaa bbb\ncc", 20.0);
    assert_eq!(
        (tl.src_ls.len(), tl.src_le.len()),
        (3, 3),
        "source map parallels visual lines"
    );
    assert_eq!(
        (tl.src_ls[0], tl.src_le[0]),
        (0, 3),
        "wrapped first source line"
    );
    assert_eq!(
        (tl.src_ls[1], tl.src_le[1]),
        (4, 7),
        "wrapped second source line"
    );
    assert_eq!(
        (tl.src_ls[2], tl.src_le[2]),
        (8, 10),
        "hard newline advances source offset"
    );

    let mut lay = layout::lay_new();
    lay.p_node.extend([900, 900]);
    lay.p_tl.extend([0, 1]);
    assert_eq!(
        layout::text_layout_ix(&lay, 900),
        1,
        "node lookup returns last solved text layout"
    );
}

/// Verifies that vertical movement retains its goal through a short line.
pub fn test_visual_goal_x_survives_short_line() {
    let doc = metric_doc();
    let tl = textm::measure_text(
        &doc,
        0,
        10.0,
        1.2,
        0.0,
        "abcd\nx\nabcd",
        1000.0,
        true,
        false,
        -1,
    );
    let mut es = edit::es_new(1, "abcd\nx\nabcd");
    es.caret = 3;
    es.anchor = 3;
    edit::visual_move(&doc, &mut es, &tl, 0, 10.0, 0.0, 1, false);
    assert_eq!(es.caret, 6, "down clamps to short visual line end");
    assert_eq!(es.goal_x, 15.0, "first vertical move captures goal x");
    edit::visual_move(&doc, &mut es, &tl, 0, 10.0, 0.0, 1, false);
    assert_eq!(es.caret, 10, "second down restores goal column");
    edit::visual_move(&doc, &mut es, &tl, 0, 10.0, 0.0, -1, false);
    assert_eq!(es.caret, 6, "up clamps but retains goal");
    edit::visual_home(&mut es, &tl, false);
    assert!(
        es.caret == 5 && es.goal_x < 0.0,
        "visual Home and goal reset"
    );
    edit::visual_end(&mut es, &tl, false);
    assert_eq!(es.caret, 6, "visual End");
}

/// Builds the single editable text-node document used by paint tests.
pub fn paint_doc() -> Doc {
    let mut doc = slir::doc_new();
    doc.ok = true;
    doc.strs.push(String::new());
    doc.node_kind.push(slir::K_TEXT);
    doc.node_flags.push(slir::F_NOWRAP);
    doc.node_parent.push(slir::NONE);
    doc.node_first.push(slir::NONE);
    doc.node_next.push(slir::NONE);
    doc.node_key.push(0);
    doc.node_id.push(0);
    doc.node_line.push(1);
    doc.attr_index.push(0);
    doc.aval_tag.push(slir::T_SIZE_FIXED);
    doc.aval_lo.push(0);
    doc.aval_hi.push(0);
    doc.aval_num.push(20.0);
    doc.attr_id.push(slir::A_W);
    doc.attr_val.push(0);
    doc.attr_index.push(1);
    doc.sign_name.push(0);
    doc.sign_node.push(0);
    doc.sign_trigger.push(1);
    doc
}

/// Verifies that field scrolling offsets text and forces clipping.
pub fn test_field_scroll_offsets_text_and_forces_clip() {
    let doc = paint_doc();
    let mut st = style::st_new();
    style::field_set(&mut st, 0, "abcdef");
    style::field_scroll_set(&mut st, 0, 8.0);
    style::begin_solve(&doc, &mut st);
    let mut lay = layout::lay_new();
    let root = layout::solve(&doc, &mut st, &mut lay, 100.0, 100.0, true);
    let frame = flatten::flatten(&doc, &st, &lay, &dispatch::dstate_new(), root);
    assert_ne!(
        frame.scene[0].flags & slir::F_CLIP,
        0,
        "horizontal edit scrolling forces node clip"
    );
    assert_eq!(frame.ops.len(), 3, "clip wraps the field text only");
    let text_x = match &frame.ops[1] {
        flatten::FrameOp::Text(text) => text.x,
        _ => 1000.0,
    };
    assert_eq!(text_x, -8.0, "field paint subtracts horizontal edit scroll");
}

/// Verifies that an overwide field remains clipped at zero scroll.
pub fn test_overwide_field_stays_clipped_at_zero_scroll() {
    let doc = paint_doc();
    let mut st = style::st_new();
    style::field_set(&mut st, 0, "abcdef");
    style::begin_solve(&doc, &mut st);
    let mut lay = layout::lay_new();
    let root = layout::solve(&doc, &mut st, &mut lay, 100.0, 100.0, true);
    let frame = flatten::flatten(&doc, &st, &lay, &dispatch::dstate_new(), root);
    assert_eq!(
        style::field_scroll_x(&st, 0),
        0.0,
        "field starts at zero horizontal scroll"
    );
    assert_ne!(
        frame.scene[0].flags & slir::F_CLIP,
        0,
        "editable field remains clipped at Home"
    );
    assert_eq!(
        frame.ops.len(),
        3,
        "zero-scroll field clip still wraps overwide text"
    );
}

/// Verifies focused-field selection bands: one half-alpha rect per
/// visual line, before the glyphs, sized to the selected extents.
pub fn test_selection_bands_for_wrapped_two_lines() {
    let mut doc = paint_doc();
    doc.node_flags[0] = slir::F_MULTILINE;
    let text = "ab cd";
    let mut st = style::st_new();
    style::field_set(&mut st, 0, text);
    let mut ds = dispatch::dstate_new();
    ds.fs.focus = 0;
    ds.ed_node.push(0);
    let mut es = edit::es_new(0, text);
    es.anchor = 0; // caret stays at the end: select-all
    ds.ed.push(es);
    style::begin_solve(&doc, &mut st);
    let mut lay = layout::lay_new();
    let root = layout::solve(&doc, &mut st, &mut lay, 100.0, 100.0, true);
    let frame = flatten::flatten(&doc, &st, &lay, &ds, root);

    let text_color = frame
        .ops
        .iter()
        .find_map(|op| match op {
            flatten::FrameOp::Text(t) => Some(t.color),
            _ => None,
        })
        .expect("field paints text");
    let band_color = (text_color & 0x00FF_FFFF) | 0x8000_0000;
    let bands: Vec<&flatten::OpRect> = frame
        .ops
        .iter()
        .filter_map(|op| match op {
            flatten::FrameOp::Rect(r) if r.bg == band_color => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(bands.len(), 2, "one selection band per visual line");

    let first_text = frame
        .ops
        .iter()
        .position(|op| matches!(op, flatten::FrameOp::Text(_)))
        .expect("text op present");
    let first_band = frame
        .ops
        .iter()
        .position(|op| matches!(op, flatten::FrameOp::Rect(r) if r.bg == band_color))
        .expect("band op present");
    assert!(first_band < first_text, "bands paint under the glyphs");

    let tl = &lay.tls[0];
    assert_eq!(tl.src_ls.len(), 2, "field wraps to two visual lines");
    let rule = &st.rs[0];
    let line0_w = textm::str_slice_w(
        &doc,
        rule.font,
        rule.size,
        rule.tracking,
        text,
        tl.src_ls[0],
        tl.src_le[0],
    );
    let line1_w = textm::str_slice_w(
        &doc,
        rule.font,
        rule.size,
        rule.tracking,
        text,
        tl.src_ls[1],
        tl.src_le[1],
    );
    assert_eq!(bands[0].x, 0.0, "first band starts at the line origin");
    assert_eq!(bands[0].w, line0_w, "first band spans the selected line");
    assert_eq!(bands[1].w, line1_w, "second band spans the wrapped tail");
    assert_eq!(bands[0].y, 0.0, "first band sits on line zero");
    assert_eq!(bands[1].y, tl.line_h, "second band sits one line down");
    assert_eq!(bands[0].h, tl.line_h, "band height is the line height");
}

/// Verifies host-driven focus: a field key binds edit state and caret
/// effects; inert or unknown keys are rejected without side effects.
pub fn test_host_focus_binds_field_and_rejects_inert() {
    let keyed_inst = |inert: bool| {
        let mut inst = frame::inst_shell();
        inst.doc = paint_doc();
        inst.doc.node_flags[0] |= slir::F_FOCUSABLE | if inert { slir::F_INERT } else { 0 };
        inst.doc.strs.push("field-key".into());
        inst.doc.node_key[0] = 1;
        frame::inst_init(&mut inst);
        frame::inst_set_env(&mut inst, 500.0, 500.0, 0, false, false);
        frame::inst_frame(&mut inst, 0.0);
        inst
    };

    let mut inst = keyed_inst(false);
    assert!(
        !frame::inst_set_focus(&mut inst, "nope", true),
        "unknown key is rejected"
    );
    assert!(
        frame::inst_set_focus(&mut inst, "field-key", true),
        "focusable field key accepted"
    );
    assert_eq!(inst.ds.fs.focus, 0, "focus lands on the field node");
    assert!(inst.ds.fs.visible, "visible=true selects the focus ring");
    assert!(
        dispatch::ed_ix(&inst.ds, 0) >= 0,
        "field binds edit state on host focus"
    );
    let mut eff = dispatch::effects_new();
    dispatch::caret_effects(&inst.doc, &inst.st, &inst.lay, &inst.sc, &inst.ds, &mut eff);
    assert!(eff.has_caret, "caret effects present after host focus");
    assert!(
        frame::inst_set_focus(&mut inst, "", false),
        "empty key clears focus"
    );
    assert_eq!(inst.ds.fs.focus, slir::NONE, "focus cleared");

    let mut inert = keyed_inst(true);
    assert!(
        !frame::inst_set_focus(&mut inert, "field-key", true),
        "inert node is rejected"
    );
    assert_eq!(
        inert.ds.fs.focus,
        slir::NONE,
        "rejection leaves focus unchanged"
    );
    assert!(
        dispatch::ed_ix(&inert.ds, 0) < 0,
        "rejection binds no edit state"
    );
}
