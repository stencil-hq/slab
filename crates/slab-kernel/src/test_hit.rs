//! Tests for scene geometry, focus order, scroll clamping, and key dispatch.
//!
//! Scenes and minimal SLIR documents are constructed directly so routing
//! invariants remain independent of document compilation.

use crate::{
    dispatch::{self, Event},
    edit, hit, layout, list,
    scene::{self, Scene},
    slir::{self, Doc},
    style,
};

/// Appends a rectangular node with the supplied geometry to a test scene.
#[allow(clippy::too_many_arguments)]
pub fn add(
    sc: &mut Scene,
    node: u32,
    parent: i32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    rot: f64,
    flags: u32,
) {
    sc.node.push(node);
    sc.parent.push(parent);
    sc.kind.push(slir::K_RECT);
    sc.x.push(x);
    sc.y.push(y);
    sc.w.push(w);
    sc.h.push(h);
    sc.radius.push(radius);
    sc.rot.push(rot);
    sc.cx.push(x + w / 2.0);
    sc.cy.push(y + h / 2.0);
    sc.flags.push(flags);
    sc.content_main.push(0.0);
    sc.scroll_off.push(0.0);
    sc.content_cross.push(0.0);
    sc.scroll_cross.push(0.0);
    sc.is_row.push(false);
}

/// Returns the deepest hit node, or `-1` when the scene is not hit.
pub fn target_of(sc: &Scene, x: f64, y: f64) -> i32 {
    let mut path = Vec::new();
    hit::hit_test(sc, x, y, &mut path);
    path.last().copied().unwrap_or(-1)
}

/// Verifies reverse paint-order targeting and the returned ancestor chain.
pub fn test_topmost_wins_in_overlap() {
    let mut sc = scene::scene_new();
    add(&mut sc, 0, -1, 0.0, 0.0, 100.0, 100.0, 0.0, 0.0, 0); // stack
    add(&mut sc, 1, 0, 0.0, 0.0, 100.0, 100.0, 0.0, 0.0, 0); // under
    add(&mut sc, 2, 0, 0.0, 0.0, 50.0, 50.0, 0.0, 0.0, 0); // over
    assert_eq!(target_of(&sc, 25.0, 25.0), 2, "over wins inside");
    assert_eq!(target_of(&sc, 90.0, 90.0), 1, "under wins outside");

    let mut path = Vec::new();
    hit::hit_test(&sc, 25.0, 25.0, &mut path);
    assert_eq!(path, [0, 2], "chain root->target");
}

/// Verifies that rounded corners reject points outside their arc.
pub fn test_rounded_corner_misses() {
    let mut sc = scene::scene_new();
    add(&mut sc, 0, -1, 0.0, 0.0, 100.0, 100.0, 0.0, 0.0, 0);
    add(&mut sc, 1, 0, 0.0, 0.0, 100.0, 100.0, 40.0, 0.0, 0);
    assert_eq!(target_of(&sc, 2.0, 2.0), 0, "corner outside the arc");
    assert_eq!(target_of(&sc, 50.0, 2.0), 1, "edge midpoint inside");
}

/// Verifies that clipping parents block hits on overflowing children.
pub fn test_clip_parent_blocks_outside_hits() {
    let mut sc = scene::scene_new();
    add(&mut sc, 0, -1, 0.0, 0.0, 200.0, 100.0, 0.0, 0.0, 0); // stack
    add(
        &mut sc,
        1,
        0,
        0.0,
        0.0,
        100.0,
        100.0,
        0.0,
        0.0,
        slir::F_CLIP,
    ); // viewport
    add(&mut sc, 2, 1, 0.0, 0.0, 180.0, 50.0, 0.0, 0.0, 0); // inner
    assert_eq!(target_of(&sc, 50.0, 25.0), 2, "inside the clip hits inner");
    assert_eq!(
        target_of(&sc, 150.0, 25.0),
        0,
        "outside the clip cannot hit inner"
    );
}

/// Verifies hit-testing in a rotated rectangle's local space.
pub fn test_rotation_hit_follows_transform() {
    let mut sc = scene::scene_new();
    add(&mut sc, 0, -1, 0.0, 0.0, 200.0, 200.0, 0.0, 0.0, 0);
    add(&mut sc, 1, 0, 50.0, 90.0, 100.0, 20.0, 0.0, 45.0, 0);
    assert_eq!(
        target_of(&sc, 100.0, 100.0),
        1,
        "center invariant under rotation"
    );
    // An axis-aligned bounding-box corner is not inside the rotated rectangle.
    assert_eq!(target_of(&sc, 51.0, 91.0), 0, "bbox corner misses");
    // This point lies 30 units along the rotated long axis at +45 degrees.
    let dx = 30.0 * hit::cos_deg(45.0);
    assert_eq!(
        target_of(&sc, 100.0 + dx, 100.0 + dx),
        1,
        "rotated long axis hits"
    );
    assert_eq!(
        target_of(&sc, 100.0 + dx, 100.0 - dx),
        0,
        "perpendicular misses"
    );
}

/// Verifies the payload bounds used for a quarter-turn rotation.
pub fn test_quarter_rotation_bbox() {
    let mut sc = scene::scene_new();
    add(&mut sc, 0, -1, 0.0, 0.0, 200.0, 200.0, 0.0, 0.0, 0);
    // Quarter-turned payload: a 20x120 bounding box for a centered 120x20 payload.
    sc.node.push(1);
    sc.parent.push(0);
    sc.kind.push(slir::K_RECT);
    sc.x.push(-50.0);
    sc.y.push(50.0);
    sc.w.push(120.0);
    sc.h.push(20.0);
    sc.radius.push(0.0);
    sc.rot.push(90.0);
    sc.cx.push(10.0);
    sc.cy.push(60.0);
    sc.flags.push(0);
    sc.content_main.push(0.0);
    sc.is_row.push(false);
    // The center of the rotated bounding box at (10, 60) must hit the payload.
    assert_eq!(target_of(&sc, 10.0, 60.0), 1, "rotated bbox center hits");
    assert_eq!(
        target_of(&sc, 10.0, 130.0),
        0,
        "below the rotated bbox misses"
    );
}

/// Verifies that inert overlays do not intercept a hit.
pub fn test_inert_overlay_passes_through() {
    let mut sc = scene::scene_new();
    add(&mut sc, 0, -1, 0.0, 0.0, 100.0, 100.0, 0.0, 0.0, 0);
    add(&mut sc, 1, 0, 0.0, 0.0, 100.0, 100.0, 0.0, 0.0, 0); // base
    add(
        &mut sc,
        2,
        0,
        0.0,
        0.0,
        100.0,
        100.0,
        0.0,
        0.0,
        slir::F_INERT,
    ); // veil
    assert_eq!(target_of(&sc, 50.0, 50.0), 1, "inert veil passes through");
}

/// Verifies focusable collection order and inert-node filtering.
pub fn test_focusables_document_order() {
    let mut sc = scene::scene_new();
    add(&mut sc, 0, -1, 0.0, 0.0, 100.0, 100.0, 0.0, 0.0, 0);
    add(
        &mut sc,
        1,
        0,
        0.0,
        0.0,
        10.0,
        10.0,
        0.0,
        0.0,
        slir::F_FOCUSABLE,
    );
    add(
        &mut sc,
        2,
        0,
        0.0,
        10.0,
        10.0,
        10.0,
        0.0,
        0.0,
        slir::F_INERT | slir::F_FOCUSABLE,
    );
    add(
        &mut sc,
        3,
        0,
        0.0,
        20.0,
        10.0,
        10.0,
        0.0,
        0.0,
        slir::F_FOCUSABLE,
    );
    let mut focusables = Vec::new();
    scene::focusables(&sc, &mut focusables);
    assert_eq!(focusables.len(), 2, "inert focusable skipped");
    assert_eq!(focusables, [1, 3], "document order");
}

/// Verifies column and row scroll clamping against their viewports.
pub fn test_scroll_clamp_bounds() {
    let mut sc = scene::scene_new();
    add(
        &mut sc,
        0,
        -1,
        0.0,
        0.0,
        300.0,
        100.0,
        0.0,
        0.0,
        slir::F_SCROLL | slir::F_CLIP,
    );
    sc.content_main[0] = 400.0;
    assert_eq!(
        dispatch::clamp_scroll(&sc, 0, 9999.0),
        300.0,
        "clamps to content - viewport"
    );
    assert_eq!(dispatch::clamp_scroll(&sc, 0, -5.0), 0.0, "clamps at 0");
    assert_eq!(
        dispatch::clamp_scroll(&sc, 0, 120.0),
        120.0,
        "in range passes"
    );
    // A row uses its width as the viewport along the main axis.
    sc.is_row[0] = true;
    sc.content_main[0] = 350.0;
    assert_eq!(
        dispatch::clamp_scroll(&sc, 0, 9999.0),
        50.0,
        "row axis viewport = w"
    );
}

/// Verifies arrow scroll steps: 40u plain, 200u with Shift (fast scroll).
pub fn test_shift_arrow_fast_scrolls() {
    let mut sc = scene::scene_new();
    add(
        &mut sc,
        0,
        -1,
        0.0,
        0.0,
        100.0,
        100.0,
        0.0,
        0.0,
        slir::F_SCROLL | slir::F_CLIP,
    );
    sc.content_main[0] = 1000.0;
    let mut st = style::st_new();
    let mut eff = dispatch::effects_new();
    let d = slir::doc_new();
    assert!(
        dispatch::scroll_key(&d, &mut st, &sc, 0, "ArrowDown", 0, &mut eff),
        "plain arrow consumed"
    );
    assert_eq!(style::scroll_get(&st, 0), 40.0, "plain arrow steps 40u");
    assert!(
        dispatch::scroll_key(
            &d,
            &mut st,
            &sc,
            0,
            "ArrowDown",
            dispatch::M_SHIFT,
            &mut eff,
        ),
        "shift arrow consumed"
    );
    assert_eq!(
        style::scroll_get(&st, 0),
        240.0,
        "shift arrow steps 200u (fast scroll)"
    );
    assert!(
        dispatch::scroll_key(&d, &mut st, &sc, 0, "ArrowUp", dispatch::M_SHIFT, &mut eff,),
        "shift arrow up consumed"
    );
    assert_eq!(
        style::scroll_get(&st, 0),
        40.0,
        "shift arrow up steps back 200u"
    );
}
/// Verifies independent deepest-owner wheel routing and Shift axis swapping.
pub fn test_wheel_routes_main_and_cross_axes_independently() {
    let mut doc = slir::doc_new();
    doc.strs
        .extend([String::new(), "outer".into(), "inner".into()]);
    doc.node_kind.extend([slir::K_COL, slir::K_COL]);
    doc.node_key.extend([1, 2]);

    let mut sc = scene::scene_new();
    add(
        &mut sc,
        0,
        -1,
        0.0,
        0.0,
        100.0,
        100.0,
        0.0,
        0.0,
        slir::F_SCROLL | slir::F_CLIP,
    );
    add(
        &mut sc,
        1,
        0,
        0.0,
        0.0,
        100.0,
        100.0,
        0.0,
        0.0,
        slir::F_SCROLL_CROSS | slir::F_CLIP,
    );
    sc.content_main[0] = 400.0;
    sc.content_cross[1] = 400.0;

    let mut st = style::st_new();
    let mut ds = dispatch::dstate_new();
    let lay = layout::lay_new();
    let wheel = Event {
        etype: dispatch::E_WHEEL,
        x: 50.0,
        y: 50.0,
        dx: 15.0,
        dy: 20.0,
        button: 0,
        clicks: 0,
        key: String::new(),
        text: String::new(),
        mods: 0,
    };
    let effects = dispatch::dispatch(&doc, &mut st, &lay, &sc, &mut ds, &wheel);
    assert_eq!(
        style::scroll_get(&st, 0),
        20.0,
        "dy reaches deepest main owner"
    );
    assert_eq!(
        style::scroll_cross_get(&st, 1),
        15.0,
        "dx reaches deepest cross owner"
    );
    assert_eq!(
        effects.scrolls,
        [
            dispatch::ScrollChange {
                key: "outer".into(),
                axis: 0,
                off: 20.0,
            },
            dispatch::ScrollChange {
                key: "inner".into(),
                axis: 1,
                off: 15.0,
            },
        ],
        "each actual axis change is notified once"
    );

    let shifted = Event {
        dx: 7.0,
        dy: 9.0,
        mods: dispatch::M_SHIFT,
        ..wheel
    };
    let effects = dispatch::dispatch(&doc, &mut st, &lay, &sc, &mut ds, &shifted);
    assert_eq!(style::scroll_get(&st, 0), 27.0, "Shift routes dx to main");
    assert_eq!(
        style::scroll_cross_get(&st, 1),
        24.0,
        "Shift routes dy to cross"
    );
    assert_eq!(
        effects
            .scrolls
            .iter()
            .map(|change| (change.axis, change.off))
            .collect::<Vec<_>>(),
        [(0, 27.0), (1, 24.0)]
    );
    let saturating = Event {
        dx: 1000.0,
        dy: 1000.0,
        mods: 0,
        ..shifted
    };
    let changed = dispatch::dispatch(&doc, &mut st, &lay, &sc, &mut ds, &saturating);
    assert_eq!(
        changed.scrolls.len(),
        2,
        "both axes notify on reaching their clamps"
    );
    let unchanged = dispatch::dispatch(&doc, &mut st, &lay, &sc, &mut ds, &saturating);
    assert!(
        unchanged.scrolls.is_empty(),
        "clamped no-op emits no ScrollChange"
    );
    assert!(!unchanged.repaint, "clamped no-op does not request a frame");
}

/// Verifies the degree-based sine and cosine helpers at canonical angles.
pub fn test_trig_values() {
    assert!((hit::sin_deg(90.0) - 1.0).abs() < 1.0e-12, "sin 90");
    assert!((hit::cos_deg(0.0) - 1.0).abs() < 1.0e-12, "cos 0");
    assert!((hit::sin_deg(30.0) - 0.5).abs() < 1.0e-12, "sin 30");
    assert!((hit::cos_deg(60.0) - 0.5).abs() < 1.0e-12, "cos 60");
    assert!((hit::sin_deg(-90.0) + 1.0).abs() < 1.0e-12, "sin -90");
    let c = hit::cos_deg(45.0);
    assert!((2.0 * c * c - 1.0).abs() < 1.0e-12, "cos45 identity");
    assert!(hit::sin_deg(360.0).abs() < 1.0e-12, "sin 360");
}

/// Verifies clipping and child targeting beneath a rotated ancestor.
pub fn test_rotated_clipping_ancestor() {
    // The point is transformed into the rotated clipping parent's local space
    // for both the clip and child tests. The child overflows to the right, so
    // points beyond the parent's clip edge must not hit it.
    let mut sc = scene::scene_new();
    add(&mut sc, 0, -1, 0.0, 0.0, 300.0, 200.0, 0.0, 0.0, 0);
    add(
        &mut sc,
        1,
        0,
        40.0,
        40.0,
        100.0,
        20.0,
        0.0,
        45.0,
        slir::F_CLIP,
    ); // rotated clip
    add(&mut sc, 2, 1, 40.0, 40.0, 180.0, 20.0, 0.0, 0.0, 0); // overflowing child
    let c = hit::cos_deg(45.0);
    // Local (100, 50) is inside both the clip and child.
    let ix = 10.0 * c;
    assert_eq!(
        target_of(&sc, 90.0 + ix, 50.0 + ix),
        2,
        "rotated clip passes inside"
    );
    // Local (150, 50) is inside the child but outside the clip.
    let ox = 60.0 * c;
    assert_eq!(
        target_of(&sc, 90.0 + ox, 50.0 + ox),
        0,
        "rotated clip rejects outside"
    );
}

/// Builds the document used to test bubbling keyboard activation.
pub fn activation_doc() -> Doc {
    let mut doc = slir::doc_new();
    doc.strs.extend([
        String::new(),
        "Escape,F2".into(),
        "cancel".into(),
        "disabled".into(),
    ]);
    for node in 0..2 {
        doc.node_kind.push(slir::K_RECT);
        doc.node_flags.push(slir::F_FOCUSABLE);
        doc.node_parent.push(if node == 0 { slir::NONE } else { 0 });
        doc.node_first.push(slir::NONE);
        doc.node_next.push(slir::NONE);
        doc.node_key.push(0);
        doc.node_id.push(0);
        doc.node_line.push(1);
    }
    doc.aval_tag.push(slir::T_STR);
    doc.aval_lo.push(1);
    doc.aval_hi.push(0);
    doc.aval_num.push(0.0);
    doc.attr_index.extend([0, 1, 1]);
    doc.attr_id.push(slir::A_KEYS);
    doc.attr_val.push(0);
    doc.sign_name.push(2);
    doc.sign_node.push(0);
    doc.sign_trigger.push(0);
    doc
}

/// Builds the parent and focused child scene used by activation tests.
pub fn activation_scene() -> Scene {
    let mut sc = scene::scene_new();
    add(
        &mut sc,
        0,
        -1,
        0.0,
        0.0,
        100.0,
        100.0,
        0.0,
        0.0,
        slir::F_FOCUSABLE,
    );
    add(
        &mut sc,
        1,
        0,
        0.0,
        0.0,
        50.0,
        50.0,
        0.0,
        0.0,
        slir::F_FOCUSABLE,
    );
    sc
}

/// Constructs a key-down event with neutral pointer and modifier fields.
pub fn key_event(key: &str) -> Event {
    Event {
        etype: dispatch::E_KEY_DOWN,
        x: 0.0,
        y: 0.0,
        dx: 0.0,
        dy: 0.0,
        button: 0,
        clicks: 0,
        key: key.into(),
        text: String::new(),
        mods: 0,
    }
}

/// Verifies that activation keys bubble from the focused child to an ancestor.
pub fn test_activation_key_bubbles_to_ancestor() {
    let doc = activation_doc();
    let mut st = style::st_new();
    let sc = activation_scene();
    let mut ds = dispatch::dstate_new();
    ds.fs.focus = 1;
    let lay = layout::lay_new();
    let effects = dispatch::dispatch(&doc, &mut st, &lay, &sc, &mut ds, &key_event("Escape"));
    assert!(effects.repaint, "matching key requests repaint");
    assert!(
        effects.sig_name.len() == 1 && effects.sig_name[0] == 2,
        "ancestor act signal"
    );
    assert!(
        effects.sig_item.len() == 1 && effects.sig_item[0].is_empty(),
        "real signal item empty"
    );
    assert_eq!(
        effects.sig_meta[0].key, "Escape",
        "metadata names fired key"
    );
}

/// Verifies that disabled ancestors suppress matching activation keys.
pub fn test_disabled_activation_key_is_suppressed() {
    let doc = activation_doc();
    let mut st = style::st_new();
    assert!(
        style::set_node_state(&doc, &mut st, 0, "disabled", true),
        "disabled state set"
    );
    let sc = activation_scene();
    let mut ds = dispatch::dstate_new();
    ds.fs.focus = 1;
    let lay = layout::lay_new();
    let effects = dispatch::dispatch(&doc, &mut st, &lay, &sc, &mut ds, &key_event("Escape"));
    assert!(
        effects.sig_name.is_empty(),
        "disabled keys node does not activate"
    );
}

/// Verifies item-key propagation for synthetic activation, change, and submit.
pub fn test_synthetic_activation_carries_item_key() {
    let mut doc = slir::doc_new();
    doc.strs.extend([
        String::new(),
        "pick".into(),
        "change".into(),
        "submit".into(),
    ]);
    for template_node in 0..2 {
        let (kind, flags, parent) = if template_node == 0 {
            (slir::K_EACH, 0, slir::NONE)
        } else {
            (slir::K_RECT, slir::F_FOCUSABLE, 0)
        };
        doc.node_kind.push(kind);
        doc.node_flags.push(flags);
        doc.node_parent.push(parent);
        doc.node_first.push(slir::NONE);
        doc.node_next.push(slir::NONE);
        doc.node_key.push(0);
        doc.node_id.push(0);
        doc.node_line.push(1);
    }
    doc.attr_index.extend([0, 0, 0]);
    doc.sign_name.push(1);
    doc.sign_node.push(1);
    doc.sign_trigger.push(0);

    let mut st = style::st_new();
    st.lists.sy_next = u32::try_from(doc.node_kind.len()).expect("test document fits in u32");
    let node = list::synthetic(&doc, &mut st.lists, 0, 1, "item-7");
    doc.sign_name.push(2);
    doc.sign_node.push(1);
    doc.sign_trigger.push(1);
    doc.sign_name.push(3);
    doc.sign_node.push(1);
    doc.sign_trigger.push(2);

    let mut sc = scene::scene_new();
    add(
        &mut sc,
        node,
        -1,
        0.0,
        0.0,
        20.0,
        20.0,
        0.0,
        0.0,
        slir::F_FOCUSABLE,
    );
    let mut ds = dispatch::dstate_new();
    ds.fs.focus = node;
    let lay = layout::lay_new();
    let effects = dispatch::dispatch(&doc, &mut st, &lay, &sc, &mut ds, &key_event("Enter"));
    assert!(
        effects.sig_name.len() == 1 && effects.sig_name[0] == 1,
        "template signal found"
    );
    assert!(
        effects.sig_item.len() == 1 && effects.sig_item[0] == "item-7",
        "synthetic signal carries innermost item key"
    );

    let mut eds = dispatch::dstate_new();
    eds.fs.focus = node;
    eds.ed_node.push(node);
    eds.ed.push(edit::es_new(node, ""));
    let text_event = Event {
        etype: dispatch::E_TEXT,
        x: 0.0,
        y: 0.0,
        dx: 0.0,
        dy: 0.0,
        button: 0,
        clicks: 0,
        key: String::new(),
        text: "x".into(),
        mods: 0,
    };
    let change = dispatch::dispatch(&doc, &mut st, &lay, &sc, &mut eds, &text_event);
    assert!(
        change.sig_item.len() == 1 && change.sig_item[0] == "item-7",
        "synthetic Change carries item key"
    );
    let submit = dispatch::dispatch(&doc, &mut st, &lay, &sc, &mut eds, &key_event("Enter"));
    assert!(
        submit.sig_name.len() == 1
            && submit.sig_name[0] == 3
            && submit.sig_text[0] == "x"
            && submit.sig_item[0] == "item-7",
        "synthetic Submit carries text and item key"
    );
}
