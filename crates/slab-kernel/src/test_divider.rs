//! Focused divider layout, dispatch, and persistent-overlay contracts.

use crate::{dispatch, frame, scene, slir, style};

fn add_attr(doc: &mut slir::Doc, attr: u32, tag: u32, num: f64) {
    let value = u32::try_from(doc.aval_tag.len()).expect("fixture value count fits u32");
    doc.aval_tag.push(tag);
    doc.aval_lo.push(0);
    doc.aval_hi.push(0);
    doc.aval_num.push(num);
    doc.attr_id.push(attr);
    doc.attr_val.push(value);
}

fn finish_attrs(doc: &mut slir::Doc) {
    doc.attr_index
        .push(i32::try_from(doc.attr_id.len()).expect("fixture attr count fits i32"));
}

fn divider_doc(row: bool) -> slir::Doc {
    let mut doc = slir::doc_new();
    doc.ok = true;
    doc.strs.extend([
        String::new(),
        "#root".into(),
        "#root/#before".into(),
        "#root/#split".into(),
        "#root/#after".into(),
        "resized".into(),
        "reset".into(),
        "disabled".into(),
    ]);
    doc.node_kind.extend([
        if row { slir::K_ROW } else { slir::K_COL },
        slir::K_RECT,
        slir::K_DIVIDER,
        slir::K_RECT,
    ]);
    doc.node_flags.extend([0, 0, slir::F_FOCUSABLE, 0]);
    doc.node_parent.extend([slir::NONE, 0, 0, 0]);
    doc.node_first
        .extend([1, slir::NONE, slir::NONE, slir::NONE]);
    doc.node_next.extend([slir::NONE, 2, 3, slir::NONE]);
    doc.node_key.extend([1, 2, 3, 4]);
    doc.node_id.resize(4, 0);
    doc.node_line.resize(4, 1);

    doc.attr_index.push(0);
    add_attr(
        &mut doc,
        if row { slir::A_W } else { slir::A_H },
        slir::T_SIZE_FIXED,
        300.0,
    );
    add_attr(
        &mut doc,
        if row { slir::A_H } else { slir::A_W },
        slir::T_SIZE_FIXED,
        80.0,
    );
    finish_attrs(&mut doc);

    add_attr(
        &mut doc,
        if row { slir::A_W } else { slir::A_H },
        slir::T_SIZE_FILL,
        1.0,
    );
    add_attr(
        &mut doc,
        if row { slir::A_MIN_W } else { slir::A_MIN_H },
        slir::T_NUM,
        80.0,
    );
    add_attr(
        &mut doc,
        if row { slir::A_MAX_W } else { slir::A_MAX_H },
        slir::T_NUM,
        220.0,
    );
    finish_attrs(&mut doc);

    add_attr(
        &mut doc,
        if row { slir::A_W } else { slir::A_H },
        slir::T_SIZE_FIXED,
        6.0,
    );
    add_attr(
        &mut doc,
        if row { slir::A_MIN_W } else { slir::A_MIN_H },
        slir::T_NUM,
        0.0,
    );
    let pad_value = u32::try_from(doc.aval_tag.len()).expect("fixture value count fits u32");
    let pad_offset = u32::try_from(doc.f64s.len()).expect("fixture tuple offset fits u32");
    doc.f64s.extend([0.0, 0.0, 0.0, 0.0]);
    doc.aval_tag.push(slir::T_TUPLE);
    doc.aval_lo.push(pad_offset);
    doc.aval_hi.push(4);
    doc.aval_num.push(0.0);
    doc.attr_id.push(slir::A_PAD);
    doc.attr_val.push(pad_value);
    finish_attrs(&mut doc);

    add_attr(
        &mut doc,
        if row { slir::A_W } else { slir::A_H },
        slir::T_SIZE_FILL,
        1.0,
    );
    add_attr(
        &mut doc,
        if row { slir::A_MIN_W } else { slir::A_MIN_H },
        slir::T_NUM,
        70.0,
    );
    finish_attrs(&mut doc);

    doc.sign_name.extend([5, 6]);
    doc.sign_node.extend([2, 2]);
    doc.sign_trigger
        .extend([dispatch::TR_RESIZE, dispatch::TR_DBLCLICK]);
    doc
}

fn unsolved_instance(row: bool) -> frame::Instance {
    let mut instance = frame::inst_shell();
    instance.doc = divider_doc(row);
    frame::inst_init(&mut instance);
    frame::inst_set_env(&mut instance, 300.0, 300.0, 0, false, false);
    instance
}

fn instance(row: bool) -> frame::Instance {
    let mut instance = unsolved_instance(row);
    frame::inst_frame(&mut instance, 0.0);
    instance
}

fn attr_value(doc: &slir::Doc, node: usize, attr: u32) -> usize {
    let start = usize::try_from(doc.attr_index[node]).expect("attr start is nonnegative");
    let end = usize::try_from(doc.attr_index[node + 1]).expect("attr end is nonnegative");
    let attribute = (start..end)
        .find(|&index| doc.attr_id[index] == attr)
        .expect("fixture attribute exists");
    usize::try_from(doc.attr_val[attribute]).expect("attribute value fits usize")
}

fn extent(instance: &frame::Instance, node: u32, row: bool) -> f64 {
    let index = usize::try_from(crate::scene::index_of(&instance.sc, node))
        .expect("fixture node is in scene");
    if row {
        instance.sc.w[index]
    } else {
        instance.sc.h[index]
    }
}

fn divider_center(instance: &frame::Instance) -> (f64, f64) {
    let index =
        usize::try_from(crate::scene::index_of(&instance.sc, 2)).expect("divider is in scene");
    (
        instance.sc.x[index] + instance.sc.w[index] / 2.0,
        instance.sc.y[index] + instance.sc.h[index] / 2.0,
    )
}

fn pointer(etype: u32, x: f64, y: f64, clicks: u32) -> dispatch::Event {
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
        mods,
    }
}

/// Verifies the standalone clamp used by host, layout, pointer, and keyboard writes.
pub fn test_divider_clamp_honors_every_bound() {
    assert_eq!(style::divider_clamp(20.0, 80.0, 220.0, 190.0), 80.0);
    assert_eq!(style::divider_clamp(150.0, 80.0, 220.0, 190.0), 150.0);
    assert_eq!(style::divider_clamp(210.0, 80.0, 220.0, 190.0), 190.0);
    assert_eq!(
        style::divider_clamp(210.0, 230.0, 220.0, 190.0),
        230.0,
        "an explicit minimum remains the final escape valve"
    );
}

/// Verifies host restore drives layout and is clamped to both adjacent panes.
pub fn test_divider_host_restore_and_layout_bounds() {
    let authored = instance(true);
    assert_eq!(extent(&authored, 1, true), 147.0, "authored fill fallback");

    let mut instance = unsolved_instance(true);
    assert!(frame::inst_set_divider(
        &mut instance,
        "#root/#split",
        190.0
    ));
    frame::inst_frame(&mut instance, 1.0);
    assert_eq!(extent(&instance, 1, true), 190.0);
    assert_eq!(extent(&instance, 3, true), 104.0);

    assert!(frame::inst_set_divider(
        &mut instance,
        "#root/#split",
        999.0
    ));
    assert_eq!(frame::inst_get_divider(&instance, "#root/#split"), 220.0);
    frame::inst_frame(&mut instance, 2.0);
    assert_eq!(extent(&instance, 1, true), 220.0, "previous max clamps");
    assert_eq!(
        extent(&instance, 3, true),
        74.0,
        "next minimum is preserved"
    );

    assert!(frame::inst_set_divider(
        &mut instance,
        "#root/#split",
        -10.0
    ));
    assert_eq!(frame::inst_get_divider(&instance, "#root/#split"), 80.0);
    frame::inst_frame(&mut instance, 3.0);
    assert_eq!(extent(&instance, 1, true), 80.0, "previous min clamps");
}

/// Verifies live pointer resize, fmt3 payload/meta, keyboard steps, and reset.
pub fn test_divider_pointer_keyboard_cursor_and_reset() {
    let mut instance = instance(true);
    let (x, y) = divider_center(&instance);
    let hover = frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_MOVE, x, y, 0));
    assert_eq!(hover.cursor, dispatch::CUR_COL_RESIZE);

    frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_DOWN, x, y, 1));
    let moved = frame::inst_dispatch(
        &mut instance,
        &pointer(dispatch::E_POINTER_MOVE, x + 20.0, y, 1),
    );
    assert!(moved.repaint, "drag updates the overlay live");
    assert_eq!(
        moved.sig_name.as_slice(),
        &[5],
        "drag delivers the resize signal live"
    );
    assert_eq!(moved.sig_text[0], "167");
    frame::inst_frame(&mut instance, 1.0);
    assert_eq!(extent(&instance, 1, true), 167.0);

    let released = frame::inst_dispatch(
        &mut instance,
        &pointer(dispatch::E_POINTER_UP, x + 20.0, y, 1),
    );
    assert_eq!(released.sig_name.as_slice(), &[5]);
    assert_eq!(released.sig_text.len(), 1);
    assert_eq!(released.sig_text[0], "167");
    assert_eq!(released.sig_meta[0].key, "#root/#split");
    assert_eq!(released.sig_meta[0].x, x + 20.0);
    assert_eq!(released.sig_meta[0].y, y);
    assert_eq!(released.sig_meta[0].button, 0);
    assert_eq!(released.sig_meta[0].clicks, 1);

    let right = frame::inst_dispatch(&mut instance, &key("ArrowRight", 0));
    assert_eq!(right.sig_text.len(), 1);
    assert_eq!(right.sig_text[0], "175");
    assert_eq!(right.sig_meta[0].x, -1.0);
    assert_eq!(right.sig_meta[0].y, -1.0);
    frame::inst_frame(&mut instance, 2.0);
    assert_eq!(extent(&instance, 1, true), 175.0);

    let shifted = frame::inst_dispatch(&mut instance, &key("ArrowLeft", dispatch::M_SHIFT));
    assert_eq!(shifted.sig_text.len(), 1);
    assert_eq!(shifted.sig_text[0], "174");
    assert_eq!(shifted.sig_meta[0].mods, dispatch::M_SHIFT);
    frame::inst_frame(&mut instance, 3.0);

    let (reset_x, reset_y) = divider_center(&instance);
    let reset = frame::inst_dispatch(
        &mut instance,
        &pointer(dispatch::E_POINTER_DOWN, reset_x, reset_y, 2),
    );
    assert_eq!(reset.sig_name.as_slice(), &[6]);
    assert_eq!(frame::inst_get_divider(&instance, "#root/#split"), -1.0);
    let reset_release = frame::inst_dispatch(
        &mut instance,
        &pointer(dispatch::E_POINTER_UP, reset_x, reset_y, 2),
    );
    assert!(
        reset_release.sig_name.is_empty(),
        "double-click reset does not also emit resize or activate"
    );
    frame::inst_frame(&mut instance, 4.0);
    assert_eq!(
        extent(&instance, 1, true),
        147.0,
        "reset restores authored fill"
    );
}

/// Verifies column dividers use the vertical key axis and row-resize cursor.
pub fn test_column_divider_axis_and_cursor() {
    let mut instance = instance(false);
    let (x, y) = divider_center(&instance);
    let hover = frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_MOVE, x, y, 0));
    assert_eq!(hover.cursor, dispatch::CUR_ROW_RESIZE);
    frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_DOWN, x, y, 1));
    frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_UP, x, y, 1));
    let down = frame::inst_dispatch(&mut instance, &key("ArrowDown", 0));
    assert_eq!(down.sig_text.len(), 1);
    assert_eq!(down.sig_text[0], "155");
    let off_axis = frame::inst_dispatch(&mut instance, &key("ArrowRight", 0));
    assert!(
        off_axis.sig_name.is_empty(),
        "off-axis arrow does not resize"
    );
}

/// Verifies structural siblings remain discoverable after sticky paint reordering.
pub fn test_divider_neighbors_ignore_sticky_paint_order() {
    let mut instance = unsolved_instance(true);
    instance.doc.node_flags[0] |= slir::F_SCROLL | slir::F_CLIP;
    instance.doc.node_flags[1] |= slir::F_STICKY;
    frame::inst_frame(&mut instance, 0.0);

    let divider_paint_index = instance
        .sc
        .node
        .iter()
        .position(|&node| node == 2)
        .expect("divider is painted");
    let sticky_paint_index = instance
        .sc
        .node
        .iter()
        .position(|&node| node == 1)
        .expect("sticky pane is painted");
    assert!(
        divider_paint_index < sticky_paint_index,
        "sticky children paint after ordinary siblings"
    );

    let (x, y) = divider_center(&instance);
    let hover = frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_MOVE, x, y, 0));
    assert_eq!(hover.cursor, dispatch::CUR_COL_RESIZE);
    frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_DOWN, x, y, 1));
    let released = frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_UP, x, y, 1));
    assert_eq!(
        released.sig_text.len(),
        1,
        "structural neighbors arm resize"
    );
}

/// Verifies semantic focus order remains authored when sticky painting moves
/// the first child above later siblings, without coalescing duplicate keys.
pub fn test_focus_order_ignores_sticky_promotion_and_duplicate_keys() {
    let mut instance = unsolved_instance(true);
    instance.doc.node_flags[0] |= slir::F_SCROLL | slir::F_CLIP;
    instance.doc.node_flags[1] |= slir::F_STICKY | slir::F_FOCUSABLE;
    instance.doc.node_flags[3] |= slir::F_FOCUSABLE;
    instance.doc.node_key[3] = instance.doc.node_key[1];
    frame::inst_frame(&mut instance, 0.0);

    let later_paint_index = instance
        .sc
        .node
        .iter()
        .position(|&node| node == 3)
        .expect("later normal sibling is painted");
    let sticky_paint_index = instance
        .sc
        .node
        .iter()
        .position(|&node| node == 1)
        .expect("authored-first sticky child is painted");
    assert!(
        later_paint_index < sticky_paint_index,
        "sticky paint promotion is observable in the retained scene"
    );

    let mut focusable = Vec::new();
    scene::focusables(&instance.sc, &mut focusable);
    assert_eq!(
        focusable,
        [1, 2, 3],
        "focus follows authored traversal and retains both duplicate-key nodes"
    );
}

/// Verifies resolved disabled state rejects both host focus and Tab traversal.
pub fn test_disabled_nodes_reject_host_focus_and_tab() {
    let mut instance = unsolved_instance(true);
    instance.doc.node_flags[1] |= slir::F_FOCUSABLE;
    instance.doc.node_flags[3] |= slir::F_FOCUSABLE;
    assert!(frame::inst_set_node_state(
        &mut instance,
        "#root/#before",
        "disabled",
        true
    ));
    frame::inst_frame(&mut instance, 0.0);

    assert!(
        !frame::inst_set_focus(&mut instance, "#root/#before", true),
        "host focus rejects a disabled focusable"
    );
    assert_eq!(instance.ds.fs.focus, slir::NONE);

    frame::inst_dispatch(&mut instance, &key("Tab", 0));
    assert_eq!(
        instance.ds.fs.focus, 2,
        "Tab starts at the next enabled node"
    );
    frame::inst_dispatch(&mut instance, &key("Tab", 0));
    assert_eq!(instance.ds.fs.focus, 3, "Tab advances to the later sibling");
    frame::inst_dispatch(&mut instance, &key("Tab", 0));
    assert_eq!(
        instance.ds.fs.focus, 2,
        "Tab wraps without visiting the disabled node"
    );
}

/// Verifies vanished synthetic divider identities release their retained overlays.
pub fn test_divider_overlay_prunes_with_synthetic_identity() {
    let mut doc = slir::doc_new();
    doc.node_kind.push(slir::K_DIVIDER);
    let mut state = style::st_new();
    assert!(style::divider_set(&mut state, 0, 80.0));
    assert!(style::divider_set(&mut state, 42, 120.0));
    assert!(style::divider_footprint_set(&mut state, 0, 6.0));
    assert!(style::divider_footprint_set(&mut state, 42, 8.0));

    style::prune_node_state(&doc, &mut state);

    assert_eq!(style::divider_get(&state, 0), Some(80.0));
    assert_eq!(style::divider_get(&state, 42), None);
    assert_eq!(style::divider_footprint_get(&state, 0), Some(6.0));
    assert_eq!(style::divider_footprint_get(&state, 42), None);
    assert_eq!(state.divider_node.len(), state.divider_extent.len());
    assert_eq!(
        state.divider_footprint_node.len(),
        state.divider_footprint.len()
    );
}

/// Verifies percentage, hug, and fill handles reserve their solved footprint.
pub fn test_divider_reserves_nonfixed_handle_footprints() {
    let cases = [
        (
            "percent",
            slir::T_SIZE_PCT,
            10.0,
            0.0,
            0.0,
            Some((200.0, 30.0)),
        ),
        ("hug", slir::T_SIZE_HUG, 0.0, 0.0, 15.0, Some((200.0, 30.0))),
        ("fill", slir::T_SIZE_FILL, 2.0, 0.0, 0.0, None),
    ];
    for (name, size_tag, size_value, minimum, pad_side, expected) in cases {
        let mut instance = unsolved_instance(true);
        let size = attr_value(&instance.doc, 2, slir::A_W);
        instance.doc.aval_tag[size] = size_tag;
        instance.doc.aval_num[size] = size_value;
        let min = attr_value(&instance.doc, 2, slir::A_MIN_W);
        instance.doc.aval_num[min] = minimum;
        let pad = attr_value(&instance.doc, 2, slir::A_PAD);
        let pad_offset =
            usize::try_from(instance.doc.aval_lo[pad]).expect("pad tuple offset fits usize");
        instance.doc.f64s[pad_offset + 1] = pad_side;
        instance.doc.f64s[pad_offset + 3] = pad_side;
        assert!(frame::inst_set_divider(
            &mut instance,
            "#root/#split",
            220.0
        ));

        frame::inst_frame(&mut instance, 0.0);
        assert!(!instance.dirty, "{name}: footprint settles within pass cap");
        assert!(
            !instance.st.divider_footprint_changed,
            "{name}: no pending footprint settle"
        );

        let previous = extent(&instance, 1, true);
        let handle = extent(&instance, 2, true);
        let next = extent(&instance, 3, true);
        assert!(next >= 70.0, "{name}: next minimum");
        assert!(
            previous + handle + next <= 300.0 + crate::layout::EPS,
            "{name}: children fit parent"
        );
        let measured =
            style::divider_footprint_get(&instance.st, 2).expect("solved handle footprint");
        assert!(
            (measured - handle).abs() < 1e-9,
            "{name}: measured footprint"
        );
        if let Some((expected_previous, expected_handle)) = expected {
            assert!(
                (previous - expected_previous).abs() < 1e-9,
                "{name}: previous extent"
            );
            assert!(
                (handle - expected_handle).abs() < 1e-9,
                "{name}: handle extent"
            );
        }
    }
}

/// Verifies pointer-up emits the extent clamped by the latest solved geometry.
pub fn test_divider_release_uses_fresh_layout_clamp() {
    let mut instance = instance(true);
    let (x, y) = divider_center(&instance);
    frame::inst_dispatch(&mut instance, &pointer(dispatch::E_POINTER_DOWN, x, y, 1));
    frame::inst_dispatch(
        &mut instance,
        &pointer(dispatch::E_POINTER_MOVE, x + 100.0, y, 1),
    );
    assert_eq!(frame::inst_get_divider(&instance, "#root/#split"), 220.0);

    let value = attr_value(&instance.doc, 2, slir::A_W);
    instance.doc.aval_num[value] = 40.0;
    instance.dirty = true;
    frame::inst_frame(&mut instance, 1.0);
    assert_eq!(extent(&instance, 1, true), 190.0);
    assert_eq!(frame::inst_get_divider(&instance, "#root/#split"), 190.0);

    let released = frame::inst_dispatch(
        &mut instance,
        &pointer(dispatch::E_POINTER_UP, x + 100.0, y, 1),
    );
    assert_eq!(released.sig_text.len(), 1);
    assert_eq!(released.sig_text[0], "190");
    assert_eq!(frame::inst_get_divider(&instance, "#root/#split"), 190.0);
}
