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
    doc: Doc,
    state: St,
    layout: layout::Lay,
    scene: Scene,
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
    test_hit::add(
        &mut scene,
        SOURCE,
        0,
        0.0,
        0.0,
        80.0,
        80.0,
        0.0,
        0.0,
        slir::F_FOCUSABLE,
    );
    test_hit::add(
        &mut scene,
        SOURCE_CHILD,
        1,
        8.0,
        8.0,
        64.0,
        64.0,
        0.0,
        0.0,
        0,
    );
    test_hit::add(&mut scene, TARGET, 0, 120.0, 0.0, 100.0, 80.0, 0.0, 0.0, 0);
    test_hit::add(
        &mut scene,
        TARGET_INNER,
        3,
        132.0,
        8.0,
        76.0,
        64.0,
        0.0,
        0.0,
        0,
    );

    let mut state = style::st_new();
    list::init(&doc, &mut state.lists);
    Fixture {
        doc,
        state,
        layout: layout::lay_new(),
        scene,
        dispatch: dispatch::dstate_new(),
    }
}

fn pointer(etype: u32, x: f64, y: f64, button: u32, clicks: u32, mods: u32) -> Event {
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
        .map(|&name| slir::str_at(&fixture.doc, name))
        .collect()
}

/// Primary Press precedes capture, while secondary Context never presses or focuses.
pub fn test_press_and_context_button_semantics() {
    let mut primary = fixture(&[
        (SOURCE, dispatch::TR_PRESS, "pressed-signal"),
        (SOURCE, dispatch::TR_CONTEXT, "context-signal"),
    ]);
    let down = send(
        &mut primary,
        &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, dispatch::M_CTRL),
    );
    assert_eq!(signal_names(&primary, &down), ["pressed-signal"]);
    assert_eq!(primary.dispatch.pressed, SOURCE);
    assert_eq!(primary.dispatch.fs.focus, SOURCE);
    assert!(style::node_state_on(
        &primary.doc,
        &primary.state,
        SOURCE,
        "pressed"
    ));
    assert_eq!(
        down.sig_meta,
        [SigMeta {
            cancelled: false,
            drag_dx: 0.0,
            drag_dy: 0.0,
            dropped: false,
            dx: 0.0,
            dy: 0.0,
            x: 20.0,
            y: 20.0,
            mods: dispatch::M_CTRL,
            button: 0,
            clicks: 1,
            key: "root/source".into(),
            src_key: String::new(),
            src_item: String::new(),
        }]
    );
    let up = send(
        &mut primary,
        &pointer(dispatch::E_POINTER_UP, 20.0, 20.0, 0, 0, 0),
    );
    assert!(up.sig_name.is_empty());
    assert_eq!(primary.dispatch.pressed, slir::NONE);

    let mut secondary = fixture(&[(SOURCE, dispatch::TR_CONTEXT, "context-signal")]);
    let context = send(
        &mut secondary,
        &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 2, 1, 0),
    );
    assert_eq!(signal_names(&secondary, &context), ["context-signal"]);
    assert_eq!(secondary.dispatch.pressed, slir::NONE);
    assert_eq!(secondary.dispatch.fs.focus, slir::NONE);
    assert!(!style::node_state_on(
        &secondary.doc,
        &secondary.state,
        SOURCE,
        "pressed"
    ));
    assert_eq!(context.sig_meta[0].button, 2);
    assert_eq!(context.sig_meta[0].key, "root/source");
}

/// A handled double-click emits on down and suppresses that gesture's Activate.
pub fn test_double_click_suppresses_activate() {
    let mut fixture = fixture(&[
        (SOURCE, dispatch::TR_ACTIVATE, "activate"),
        (SOURCE, dispatch::TR_DBLCLICK, "double"),
    ]);
    let down = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 2, 0),
    );
    assert_eq!(signal_names(&fixture, &down), ["double"]);
    assert_eq!(down.sig_meta[0].clicks, 2);
    assert_eq!(down.sig_meta[0].key, "root/source");

    let up = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_UP, 20.0, 20.0, 0, 0, 0),
    );
    assert!(up.sig_name.is_empty(), "Activate must be suppressed");
    assert_eq!(fixture.dispatch.pressed, slir::NONE);
    assert!(!fixture.dispatch.suppress_activate);
}

/// Drag starts strictly beyond four units, targets the deepest external Drop, and cleans up.
pub fn test_drag_threshold_deepest_drop_and_source_metadata() {
    let mut fixture = fixture(&[
        (SOURCE, dispatch::TR_ACTIVATE, "activate"),
        (SOURCE, dispatch::TR_DRAG_START, "drag-start"),
        (SOURCE_CHILD, dispatch::TR_DROP, "inside-drop"),
        (TARGET, dispatch::TR_DROP, "outer-drop"),
        (TARGET_INNER, dispatch::TR_DROP, "inner-drop"),
    ]);
    let down = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0),
    );
    assert!(down.sig_name.is_empty());

    let threshold_boundary = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_MOVE, 24.0, 20.0, 0, 0, 0),
    );
    assert!(threshold_boundary.sig_name.is_empty());
    assert!(!fixture.dispatch.drag_active);

    let start = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_MOVE, 28.0, 20.0, 0, 0, 0),
    );
    assert_eq!(signal_names(&fixture, &start), ["drag-start"]);
    assert!(fixture.dispatch.drag_active);
    assert_eq!(fixture.dispatch.drop_target, slir::NONE);
    assert!(style::node_state_on(
        &fixture.doc,
        &fixture.state,
        SOURCE,
        "dragging"
    ));
    assert_eq!(start.sig_meta[0].key, "root/source");
    assert_eq!((start.sig_meta[0].x, start.sig_meta[0].y), (28.0, 20.0));

    let over_target = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, 0),
    );
    assert!(over_target.sig_name.is_empty());
    assert_eq!(fixture.dispatch.drop_target, TARGET_INNER);
    assert!(style::node_state_on(
        &fixture.doc,
        &fixture.state,
        TARGET_INNER,
        "drop"
    ));
    assert!(!style::node_state_on(
        &fixture.doc,
        &fixture.state,
        SOURCE_CHILD,
        "drop"
    ));

    let dropped = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_UP, 150.0, 20.0, 0, 0, dispatch::M_SHIFT),
    );
    assert_eq!(signal_names(&fixture, &dropped), ["inner-drop"]);
    assert_eq!(dropped.sig_item, [""]);
    assert_eq!(dropped.sig_meta[0].key, "root/target/inner");
    assert_eq!(dropped.sig_meta[0].src_key, "root/source");
    assert_eq!(dropped.sig_meta[0].src_item, "");
    assert_eq!(dropped.sig_meta[0].mods, dispatch::M_SHIFT);
    assert_eq!(fixture.dispatch.drag_source, slir::NONE);
    assert_eq!(fixture.dispatch.drop_target, slir::NONE);
    assert!(!fixture.dispatch.drag_active);
    assert!(!style::node_state_on(
        &fixture.doc,
        &fixture.state,
        SOURCE,
        "dragging"
    ));
    assert!(!style::node_state_on(
        &fixture.doc,
        &fixture.state,
        TARGET_INNER,
        "drop"
    ));
}

/// Releasing away from a target and Blur cancellation never synthesize Drop or Activate.
pub fn test_drag_cancel_and_blur_clear_all_gesture_state() {
    let mut fixture = fixture(&[
        (SOURCE, dispatch::TR_ACTIVATE, "activate"),
        (SOURCE, dispatch::TR_DRAG_START, "drag-start"),
        (TARGET_INNER, dispatch::TR_DROP, "drop-finished"),
    ]);
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0),
    );
    let started = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_MOVE, 28.0, 20.0, 0, 0, 0),
    );
    assert_eq!(signal_names(&fixture, &started), ["drag-start"]);
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, 0),
    );
    assert_eq!(fixture.dispatch.drop_target, TARGET_INNER);
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_MOVE, 270.0, 20.0, 0, 0, 0),
    );
    assert_eq!(fixture.dispatch.drop_target, slir::NONE);
    let released = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_UP, 270.0, 20.0, 0, 0, 0),
    );
    assert!(released.sig_name.is_empty());
    assert_eq!(fixture.dispatch.pressed, slir::NONE);
    assert_eq!(fixture.dispatch.drag_source, slir::NONE);
    assert!(!fixture.dispatch.drag_active);

    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0),
    );
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_MOVE, 28.0, 20.0, 0, 0, 0),
    );
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, 0),
    );
    let blurred = send(&mut fixture, &pointer(dispatch::E_BLUR, 0.0, 0.0, 0, 0, 0));
    assert!(blurred.sig_name.is_empty());
    assert_eq!(fixture.dispatch.pressed, slir::NONE);
    assert_eq!(fixture.dispatch.drag_source, slir::NONE);
    assert_eq!(fixture.dispatch.drop_target, slir::NONE);
    assert!(!style::node_state_on(
        &fixture.doc,
        &fixture.state,
        SOURCE,
        "dragging"
    ));
    assert!(!style::node_state_on(
        &fixture.doc,
        &fixture.state,
        TARGET_INNER,
        "drop"
    ));
}

/// Release cancels an active drag whose source became disabled after the final move.
pub fn test_drag_release_revalidates_source() {
    let mut fixture = fixture(&[
        (SOURCE, dispatch::TR_DRAG_START, "drag-start"),
        (TARGET_INNER, dispatch::TR_DROP, "drop-finished"),
    ]);
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0),
    );
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_MOVE, 150.0, 20.0, 0, 0, 0),
    );
    assert_eq!(fixture.dispatch.drop_target, TARGET_INNER);
    assert!(style::set_node_state(
        &fixture.doc,
        &mut fixture.state,
        SOURCE,
        "disabled",
        true
    ));

    let released = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_UP, 150.0, 20.0, 0, 0, 0),
    );
    assert!(released.sig_name.is_empty());
    assert_eq!(fixture.dispatch.drag_source, slir::NONE);
    assert_eq!(fixture.dispatch.drop_target, slir::NONE);
    assert!(!style::node_state_on(
        &fixture.doc,
        &fixture.state,
        TARGET_INNER,
        "drop"
    ));
}

/// Pruning a synthetic source clears Drop state on a surviving real target.
pub fn test_pruned_drag_source_clears_surviving_drop_state() {
    let mut fixture = fixture(&[]);
    let source = list::synthetic(&fixture.doc, &mut fixture.state.lists, ROOT, SOURCE, "gone");
    assert!(style::set_node_state(
        &fixture.doc,
        &mut fixture.state,
        source,
        "dragging",
        true
    ));
    assert!(style::set_node_state(
        &fixture.doc,
        &mut fixture.state,
        TARGET_INNER,
        "drop",
        true
    ));
    fixture.dispatch.drag_source = source;
    fixture.dispatch.drop_target = TARGET_INNER;
    fixture.dispatch.drag_active = true;
    fixture.state.lists.sy_id.clear();
    fixture.state.lists.sy_each.clear();
    fixture.state.lists.sy_tpl.clear();
    fixture.state.lists.sy_key.clear();

    assert!(dispatch::prune_vanished(
        &fixture.doc,
        &mut fixture.state,
        &mut fixture.dispatch
    ));
    assert_eq!(fixture.dispatch.drag_source, slir::NONE);
    assert_eq!(fixture.dispatch.drop_target, slir::NONE);
    assert!(!style::node_state_on(
        &fixture.doc,
        &fixture.state,
        TARGET_INNER,
        "drop"
    ));
}

/// A fresh scene that omits an active real source cancels capture and Drop styling.
pub fn test_fresh_scene_cancels_missing_drag_source() {
    let mut fixture = fixture(&[]);
    assert!(style::set_node_state(
        &fixture.doc,
        &mut fixture.state,
        SOURCE,
        "dragging",
        true
    ));
    assert!(style::set_node_state(
        &fixture.doc,
        &mut fixture.state,
        TARGET_INNER,
        "drop",
        true
    ));
    fixture.dispatch.pressed = SOURCE;
    fixture.dispatch.drag_source = SOURCE;
    fixture.dispatch.drop_target = TARGET_INNER;
    fixture.dispatch.drag_active = true;

    let mut fresh_scene = scene::scene_new();
    test_hit::add(
        &mut fresh_scene,
        ROOT,
        -1,
        0.0,
        0.0,
        300.0,
        80.0,
        0.0,
        0.0,
        0,
    );
    test_hit::add(
        &mut fresh_scene,
        TARGET,
        0,
        120.0,
        0.0,
        100.0,
        80.0,
        0.0,
        0.0,
        0,
    );
    test_hit::add(
        &mut fresh_scene,
        TARGET_INNER,
        1,
        132.0,
        8.0,
        76.0,
        64.0,
        0.0,
        0.0,
        0,
    );

    assert!(dispatch::cancel_invalid_drag(
        &fixture.doc,
        &mut fixture.state,
        &fresh_scene,
        &mut fixture.dispatch
    ));
    assert_eq!(fixture.dispatch.pressed, slir::NONE);
    assert_eq!(fixture.dispatch.drag_source, slir::NONE);
    assert_eq!(fixture.dispatch.drop_target, slir::NONE);
    assert!(!style::node_state_on(
        &fixture.doc,
        &fixture.state,
        SOURCE,
        "dragging"
    ));
    assert!(!style::node_state_on(
        &fixture.doc,
        &fixture.state,
        TARGET_INNER,
        "drop"
    ));
}

/// Continuous gestures emit pointer, drag-update, Drop, and DragEnd in order.
pub fn test_continuous_drag_signals_and_release_metadata() {
    let mut fixture = fixture(&[
        (SOURCE, dispatch::TR_POINTER_MOVE, "pointer-move"),
        (SOURCE, dispatch::TR_POINTER_UP, "pointer-up"),
        (SOURCE, dispatch::TR_DRAG_START, "drag-start"),
        (SOURCE, dispatch::TR_DRAG_UPDATE, "drag-update"),
        (SOURCE, dispatch::TR_DRAG_END, "drag-end"),
        (TARGET_INNER, dispatch::TR_DROP, "drop"),
    ]);
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0),
    );
    let mut move_event = pointer(
        dispatch::E_POINTER_MOVE,
        150.0,
        20.0,
        0,
        0,
        dispatch::M_CTRL,
    );
    move_event.dx = 130.0;
    let moved = send(&mut fixture, &move_event);
    assert_eq!(
        signal_names(&fixture, &moved),
        ["pointer-move", "drag-start", "drag-update"]
    );
    for meta in &moved.sig_meta {
        assert_eq!(meta.key, "root/source");
        assert_eq!(meta.dx, 130.0);
        assert_eq!(meta.dy, 0.0);
        assert_eq!(meta.drag_dx, 130.0);
        assert_eq!(meta.drag_dy, 0.0);
        assert!(!meta.cancelled);
        assert!(!meta.dropped);
    }

    let released = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_UP, 150.0, 20.0, 0, 0, dispatch::M_SHIFT),
    );
    assert_eq!(
        signal_names(&fixture, &released),
        ["pointer-up", "drop", "drag-end"]
    );
    assert_eq!(released.sig_meta[0].drag_dx, 130.0);
    assert!(!released.sig_meta[0].dropped);
    assert_eq!(released.sig_meta[1].key, "root/target/inner");
    assert_eq!(released.sig_meta[1].src_key, "root/source");
    assert!(released.sig_meta[1].dropped);
    assert_eq!(released.sig_meta[2].key, "root/source");
    assert!(!released.sig_meta[2].cancelled);
    assert!(released.sig_meta[2].dropped);
}

/// PointerUp routes for every button while primary gesture cleanup remains primary-only.
pub fn test_secondary_pointer_up_routes_without_releasing_primary_capture() {
    let mut fixture = fixture(&[(SOURCE, dispatch::TR_POINTER_UP, "released")]);
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0),
    );
    let released = send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_UP, 270.0, 20.0, 2, 1, dispatch::M_ALT),
    );
    assert_eq!(signal_names(&fixture, &released), ["released"]);
    assert_eq!(released.sig_meta[0].button, 2);
    assert_eq!(released.sig_meta[0].mods, dispatch::M_ALT);
    assert_eq!(released.sig_meta[0].key, "root/source");
    assert_eq!(fixture.dispatch.pressed, SOURCE);
    send(
        &mut fixture,
        &pointer(dispatch::E_POINTER_UP, 270.0, 20.0, 0, 0, 0),
    );
    assert_eq!(fixture.dispatch.pressed, slir::NONE);
}

/// Blur and Close each terminate an active drag exactly once from cached pointer data.
pub fn test_blur_and_close_emit_cancelled_drag_end_once() {
    for terminal in [dispatch::E_BLUR, dispatch::E_CLOSE] {
        let mut fixture = fixture(&[
            (SOURCE, dispatch::TR_DRAG_START, "drag-start"),
            (SOURCE, dispatch::TR_DRAG_END, "drag-end"),
            (TARGET_INNER, dispatch::TR_DROP, "drop"),
        ]);
        send(
            &mut fixture,
            &pointer(dispatch::E_POINTER_DOWN, 20.0, 20.0, 0, 1, 0),
        );
        let mut moved = pointer(
            dispatch::E_POINTER_MOVE,
            150.0,
            20.0,
            0,
            0,
            dispatch::M_META,
        );
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
