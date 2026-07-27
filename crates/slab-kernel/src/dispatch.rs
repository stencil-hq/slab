//! Event dispatch and host-visible effects.
//!
//! Primary pointer capture lasts from pointer-down through release. Press,
//! context, double-click, and drag/drop gestures resolve the deepest matching
//! signal in the hit path. Keyboard events deliberately bubble from the focused
//! node through scene parents. Pointer-up and matching key-down events synthesize
//! activation, while disabled nodes suppress delivery. Printable input is routed
//! to editable fields.

use std::borrow::Cow;

use crate::{
    edit::{self, EditState},
    focus::{self, FSt},
    layout::Lay,
    list,
    scene::{self, Scene},
    slir::{self, Doc},
    style::{self, St},
    textm::TextLayout,
};

/// Pointer-move event type.
pub const E_POINTER_MOVE: u32 = 0;
/// Pointer-down event type.
pub const E_POINTER_DOWN: u32 = 1;
/// Pointer-up event type.
pub const E_POINTER_UP: u32 = 2;
/// Wheel event type.
pub const E_WHEEL: u32 = 3;
/// Key-down event type.
pub const E_KEY_DOWN: u32 = 4;
/// Text-input event type.
pub const E_TEXT: u32 = 5;
/// Paste event type.
pub const E_PASTE: u32 = 6;
/// Copy event type.
pub const E_COPY: u32 = 7;
/// Cut event type.
pub const E_CUT: u32 = 8;
/// IME composition-start event type.
pub const E_COMPOSITION_START: u32 = 9;
/// IME composition-update event type.
pub const E_COMPOSITION_UPDATE: u32 = 10;
/// IME composition-end event type.
pub const E_COMPOSITION_END: u32 = 11;
/// Blur event type.
pub const E_BLUR: u32 = 12;
/// Viewport-resize event type.
pub const E_RESIZE: u32 = 13;
/// Close event type.
pub const E_CLOSE: u32 = 14;
/// Inspector event type.
pub const E_INSPECT: u32 = 15;
/// Explicit activation event type.
pub const E_ACTIVATE: u32 = 16;
/// Activate signal trigger.
pub const TR_ACTIVATE: u32 = 0;
/// Change signal trigger.
pub const TR_CHANGE: u32 = 1;
/// Submit signal trigger.
pub const TR_SUBMIT: u32 = 2;
/// Primary pointer-down signal trigger.
pub const TR_PRESS: u32 = 3;
/// Secondary pointer-down signal trigger.
pub const TR_CONTEXT: u32 = 4;
/// Double-click signal trigger.
pub const TR_DBLCLICK: u32 = 5;
/// Drag-start signal trigger.
pub const TR_DRAG_START: u32 = 6;
/// Drop signal trigger.
pub const TR_DROP: u32 = 7;
/// Divider resize signal trigger, delivered live while a divider drags (and
/// per keyboard step), then once more with the final clamped extent on release.
pub const TR_RESIZE: u32 = 8;
/// Continuous pointer-move signal trigger.
pub const TR_POINTER_MOVE: u32 = 9;
/// Primary pointer-up signal trigger.
pub const TR_POINTER_UP: u32 = 10;
/// Active-drag movement signal trigger.
pub const TR_DRAG_UPDATE: u32 = 11;
/// Drag termination signal trigger.
pub const TR_DRAG_END: u32 = 12;

/// Shift modifier bit.
pub const M_SHIFT: u32 = 1;
/// Alt modifier bit.
pub const M_ALT: u32 = 2;
/// Control modifier bit.
pub const M_CTRL: u32 = 4;
/// Meta/Command modifier bit.
pub const M_META: u32 = 8;

/// Default cursor effect.
pub const CUR_DEFAULT: u32 = 0;
/// Pointer cursor effect.
pub const CUR_POINTER: u32 = 1;
/// Text cursor effect.
pub const CUR_TEXT: u32 = 2;
/// Column-resize cursor effect.
pub const CUR_COL_RESIZE: u32 = 3;
/// Row-resize cursor effect.
pub const CUR_ROW_RESIZE: u32 = 4;

/// A host input event.
///
/// `key` contains the host's named key, such as `"Tab"`, `"Enter"`,
/// `"ArrowLeft"`, `"Backspace"`, or `"a"`; host key names are not document
/// string-pool references.
#[derive(Clone, Debug)]
pub struct Event {
    /// One of the `E_*` event type codes.
    pub etype: u32,
    /// Pointer x-coordinate.
    pub x: f64,
    /// Pointer y-coordinate.
    pub y: f64,
    /// Horizontal wheel delta or resized viewport width.
    pub dx: f64,
    /// Vertical wheel delta or resized viewport height.
    pub dy: f64,
    /// Pointer button code.
    pub button: u32,
    /// Host-computed click count for pointer-down (`0`/`1` means single).
    pub clicks: u32,
    /// Named keyboard key.
    pub key: String,
    /// Text, paste, or composition payload.
    pub text: String,
    /// Bitset of the `M_*` modifier constants.
    pub mods: u32,
}

/// Metadata attached to every emitted signal.
#[derive(Clone, Debug, PartialEq)]
pub struct SigMeta {
    /// Document-space pointer x, or `-1.0` for keyboard-originated signals.
    pub x: f64,
    /// Document-space pointer y, or `-1.0` for keyboard-originated signals.
    pub y: f64,
    /// Horizontal delta carried by the originating event.
    pub dx: f64,
    /// Vertical delta carried by the originating event.
    pub dy: f64,
    /// Current horizontal drag displacement from the pointer-down origin.
    pub drag_dx: f64,
    /// Current vertical drag displacement from the pointer-down origin.
    pub drag_dy: f64,
    /// Modifier bitset active when the signal was emitted.
    pub mods: u32,
    /// Pointer button code active when the signal was emitted.
    pub button: u32,
    /// Host-computed click count active when the signal was emitted.
    pub clicks: u32,
    /// Full key path of the signal-emitting node.
    pub key: String,
    /// Full drag-source key for a drop signal.
    pub src_key: String,
    /// Innermost drag-source item key for a drop signal.
    pub src_item: String,
    /// Whether a DragEnd represents abnormal termination.
    pub cancelled: bool,
    /// Whether an ordinary DragEnd delivered Drop to an eligible target.
    pub dropped: bool,
}

/// One scroll offset changed by a dispatch.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollChange {
    pub key: String,
    /// `0` is main and `1` is cross.
    pub axis: u32,
    pub off: f64,
}

/// Host-visible consequences of dispatching an [`Event`].
#[derive(Clone, Debug)]
pub struct Effects {
    /// Whether the next frame must re-solve.
    pub repaint: bool,
    /// Document string references, parallel to `sig_text` and `sig_item`.
    pub sig_name: Vec<u32>,
    /// Committed text for Change/Submit, final extent for Resize, or empty.
    pub sig_text: Vec<String>,
    /// Innermost list item key, or empty for a real document node.
    pub sig_item: Vec<String>,
    /// Signal metadata parallel to [`Self::sig_name`].
    pub sig_meta: Vec<SigMeta>,
    /// Scroll offsets changed by this dispatch.
    pub scrolls: Vec<ScrollChange>,
    /// Whether the caret rectangle is available.
    pub has_caret: bool,
    /// Caret rectangle x-coordinate.
    pub caret_x: f64,
    /// Caret rectangle y-coordinate.
    pub caret_y: f64,
    /// Caret rectangle width.
    pub caret_w: f64,
    /// Caret rectangle height.
    pub caret_h: f64,
    /// Whether the IME rectangle is available.
    pub has_ime: bool,
    /// IME rectangle x-coordinate.
    pub ime_x: f64,
    /// IME rectangle y-coordinate.
    pub ime_y: f64,
    /// IME rectangle width.
    pub ime_w: f64,
    /// IME rectangle height.
    pub ime_h: f64,
    /// One of the `CUR_*` cursor codes.
    pub cursor: u32,
    /// Focused node id, or [`slir::NONE`].
    pub focus: u32,
}

/// Creates an empty effect collection.
pub fn effects_new() -> Effects {
    Effects {
        repaint: false,
        sig_name: Vec::new(),
        sig_text: Vec::new(),
        sig_item: Vec::new(),
        sig_meta: Vec::new(),
        scrolls: Vec::new(),
        has_caret: false,
        caret_x: 0.0,
        caret_y: 0.0,
        caret_w: 0.0,
        caret_h: 0.0,
        has_ime: false,
        ime_x: 0.0,
        ime_y: 0.0,
        ime_w: 0.0,
        ime_h: 0.0,
        cursor: CUR_DEFAULT,
        focus: slir::NONE,
    }
}

#[derive(Clone, Debug)]
struct DividerDrag {
    node: u32,
    row: bool,
    start_pos: f64,
    start_extent: f64,
    current_extent: f64,
    min_extent: f64,
    max_extent: f64,
    moved: bool,
}

#[derive(Clone, Debug)]
struct PendingSignal {
    name: u32,
    text: String,
    item: String,
    meta: SigMeta,
}

/// Dispatch-owned interaction state, keyed by node id.
#[derive(Clone, Debug)]
pub struct DState {
    /// Keyboard focus state.
    pub fs: FSt,
    /// Node ids currently under the pointer, comprising the whole hit path.
    pub hover: Vec<u32>,
    /// Pointer-captured node, or [`slir::NONE`].
    pub pressed: u32,
    /// Armed or active drag-source node, or [`slir::NONE`].
    pub drag_source: u32,
    /// Current eligible drop-target node, or [`slir::NONE`].
    pub drop_target: u32,
    /// Pointer x-coordinate at drag arm time.
    pub drag_x: f64,
    /// Pointer y-coordinate at drag arm time.
    pub drag_y: f64,
    /// Whether the armed drag crossed the four-unit threshold.
    pub drag_active: bool,
    /// Whether this primary gesture must suppress Activate.
    pub suppress_activate: bool,
    pub(crate) drag_last_x: f64,
    pub(crate) drag_last_y: f64,
    drag_last_dx: f64,
    drag_last_dy: f64,
    drag_last_mods: u32,
    drag_last_button: u32,
    drag_last_clicks: u32,
    pub(crate) drag_grab_x: f64,
    pub(crate) drag_grab_y: f64,
    drag_source_key: String,
    drag_source_item: String,
    drag_update_name: Option<u32>,
    drag_end_name: Option<u32>,
    pending_signals: Vec<PendingSignal>,
    divider: Option<DividerDrag>,
    /// Current `CUR_*` cursor code.
    pub cursor: u32,
    /// Whether the host requested closure.
    pub closed: bool,
    /// Field node ids, parallel to `ed`.
    pub ed_node: Vec<u32>,
    /// Editing states parallel to `ed_node`.
    pub ed: Vec<EditState>,
}

/// Creates empty dispatch state.
pub fn dstate_new() -> DState {
    DState {
        fs: focus::fst_new(),
        hover: Vec::new(),
        pressed: slir::NONE,
        drag_source: slir::NONE,
        drop_target: slir::NONE,
        drag_x: 0.0,
        drag_y: 0.0,
        drag_active: false,
        suppress_activate: false,
        drag_last_x: 0.0,
        drag_last_y: 0.0,
        drag_last_dx: 0.0,
        drag_last_dy: 0.0,
        drag_last_mods: 0,
        drag_last_button: 0,
        drag_last_clicks: 0,
        drag_grab_x: 0.0,
        drag_grab_y: 0.0,
        drag_source_key: String::new(),
        drag_source_item: String::new(),
        drag_update_name: None,
        drag_end_name: None,
        pending_signals: Vec::new(),
        divider: None,
        cursor: CUR_DEFAULT,
        closed: false,
        ed_node: Vec::new(),
        ed: Vec::new(),
    }
}

/// Reports whether a synthetic node was pruned at a solve boundary.
pub fn vanished(d: &Doc, st: &St, node: u32) -> bool {
    node != slir::NONE
        && i32::from_ne_bytes(node.to_ne_bytes())
            >= i32::try_from(d.node_kind.len()).expect("too many document nodes")
        && list::base(&st.lists, d, node) == slir::NONE
}

/// Copies one field's complete editing history into another slot.
pub fn replace_edit(ds: &mut DState, dst: i32, src: i32) {
    let dst = usize::try_from(dst).expect("negative edit destination");
    let src = usize::try_from(src).expect("negative edit source");
    ds.ed[dst] = ds.ed[src].clone();
}

/// Drops dispatch state belonging to synthetic ids pruned at a solve boundary.
///
/// Returns whether clearing a surviving gesture target changed node state.
/// Focus remains until restoration runs against the fresh scene so its
/// nearest-neighbor rule can use the old position.
pub fn prune_vanished(d: &Doc, st: &mut St, ds: &mut DState) -> bool {
    let mut state_changed = false;
    if vanished(d, st, ds.pressed) {
        ds.pressed = slir::NONE;
    }
    if vanished(d, st, ds.drag_source) {
        queue_drag_end(ds);
        let mut effects = effects_new();
        clear_drag(d, st, ds, &mut effects);
        state_changed |= effects.repaint;
    } else if vanished(d, st, ds.drop_target) {
        ds.drop_target = slir::NONE;
    }
    if ds
        .divider
        .as_ref()
        .is_some_and(|divider| vanished(d, st, divider.node))
    {
        ds.divider = None;
    }

    for index in (0..ds.hover.len()).rev() {
        if vanished(d, st, ds.hover[index]) {
            ds.hover.swap_remove(index);
        }
    }
    for index in (0..ds.ed_node.len()).rev() {
        if vanished(d, st, ds.ed_node[index]) {
            let last = ds.ed_node.len() - 1;
            if index != last {
                ds.ed_node[index] = ds.ed_node[last];
                ds.ed[index] = ds.ed[last].clone();
            }
            ds.ed_node.pop();
            ds.ed.pop();
        }
    }
    state_changed
}

/// Finds the signal for `node` and one of the `TR_*` trigger constants.
pub fn sig_of(d: &Doc, st: &St, node: u32, trigger: u32) -> i32 {
    let base = list::base(&st.lists, d, node);
    d.sign_name
        .iter()
        .enumerate()
        .find(|(index, _)| d.sign_node[*index] == base && d.sign_trigger[*index] == trigger)
        .map_or(-1, |(index, _)| {
            i32::try_from(index).expect("too many signals")
        })
}

fn emit_signal(d: &Doc, st: &St, eff: &mut Effects, signal_index: usize, node: u32, text: String) {
    eff.sig_name.push(d.sign_name[signal_index]);
    eff.sig_text.push(text);
    eff.sig_item.push(list::item_key(&st.lists, d, node));
    eff.sig_meta.push(SigMeta {
        x: -1.0,
        y: -1.0,
        dx: 0.0,
        dy: 0.0,
        drag_dx: 0.0,
        drag_dy: 0.0,
        mods: 0,
        button: 0,
        clicks: 0,
        key: scene::key_of(d, &st.lists, node),
        src_key: String::new(),
        src_item: String::new(),
        cancelled: false,
        dropped: false,
    });
}

fn signal_name_of(d: &Doc, st: &St, node: u32, trigger: u32) -> Option<u32> {
    usize::try_from(sig_of(d, st, node, trigger))
        .ok()
        .map(|index| d.sign_name[index])
}

fn remember_drag_event(ds: &mut DState, ev: &Event) {
    ds.drag_last_x = ev.x;
    ds.drag_last_y = ev.y;
    ds.drag_last_dx = ev.dx;
    ds.drag_last_dy = ev.dy;
    ds.drag_last_mods = ev.mods;
    ds.drag_last_button = ev.button;
    ds.drag_last_clicks = ev.clicks;
}

fn drag_meta(ds: &DState, cancelled: bool, dropped: bool) -> SigMeta {
    SigMeta {
        x: ds.drag_last_x,
        y: ds.drag_last_y,
        dx: ds.drag_last_dx,
        dy: ds.drag_last_dy,
        drag_dx: ds.drag_last_x - ds.drag_x,
        drag_dy: ds.drag_last_y - ds.drag_y,
        mods: ds.drag_last_mods,
        button: ds.drag_last_button,
        clicks: ds.drag_last_clicks,
        key: ds.drag_source_key.clone(),
        src_key: String::new(),
        src_item: String::new(),
        cancelled,
        dropped,
    }
}

fn apply_drag_meta(meta: &mut SigMeta, ds: &DState, cancelled: bool, dropped: bool) {
    meta.x = ds.drag_last_x;
    meta.y = ds.drag_last_y;
    meta.dx = ds.drag_last_dx;
    meta.dy = ds.drag_last_dy;
    meta.drag_dx = ds.drag_last_x - ds.drag_x;
    meta.drag_dy = ds.drag_last_y - ds.drag_y;
    meta.mods = ds.drag_last_mods;
    meta.button = ds.drag_last_button;
    meta.clicks = ds.drag_last_clicks;
    meta.cancelled = cancelled;
    meta.dropped = dropped;
}

fn push_cached_drag_signal(
    effects: &mut Effects,
    ds: &DState,
    name: Option<u32>,
    cancelled: bool,
    dropped: bool,
) {
    let Some(name) = name else {
        return;
    };
    effects.sig_name.push(name);
    effects.sig_text.push(String::new());
    effects.sig_item.push(ds.drag_source_item.clone());
    effects.sig_meta.push(drag_meta(ds, cancelled, dropped));
    effects.repaint = true;
}

fn emit_drag_update(effects: &mut Effects, ds: &DState) {
    push_cached_drag_signal(effects, ds, ds.drag_update_name, false, false);
}

fn emit_drag_end(effects: &mut Effects, ds: &DState, cancelled: bool, dropped: bool) {
    if ds.drag_active {
        push_cached_drag_signal(effects, ds, ds.drag_end_name, cancelled, dropped);
    }
}

fn queue_drag_end(ds: &mut DState) {
    if !ds.drag_active {
        return;
    }
    let Some(name) = ds.drag_end_name else {
        return;
    };
    ds.pending_signals.push(PendingSignal {
        name,
        text: String::new(),
        item: ds.drag_source_item.clone(),
        meta: drag_meta(ds, true, false),
    });
}

/// Drains signals produced while settling a frame.
pub fn take_pending_signals(ds: &mut DState) -> Effects {
    let mut effects = effects_new();
    for pending in ds.pending_signals.drain(..) {
        effects.sig_name.push(pending.name);
        effects.sig_text.push(pending.text);
        effects.sig_item.push(pending.item);
        effects.sig_meta.push(pending.meta);
    }
    effects
}

/// Applies one scroll write and records the observable change.
pub fn record_scroll(
    d: &Doc,
    st: &mut St,
    node: u32,
    axis: u32,
    off: f64,
    eff: &mut Effects,
) -> bool {
    if !style::scroll_set_axis(st, node, axis, off) {
        return false;
    }
    eff.repaint = true;
    eff.scrolls.push(ScrollChange {
        key: scene::key_of(d, &st.lists, node),
        axis,
        off,
    });
    true
}

/// Clamps one scene entry's scroll offset to its selected content extent.
#[allow(clippy::manual_clamp)] // `f64::clamp` propagates NaN; min/max preserve kernel semantics.
pub fn clamp_scroll_axis(sc: &Scene, ix: i32, axis: u32, off: f64) -> f64 {
    let ix = usize::try_from(ix).expect("negative scene index");
    let (viewport, content) = if axis == 1 {
        (
            if sc.is_row[ix] { sc.h[ix] } else { sc.w[ix] },
            sc.content_cross[ix],
        )
    } else {
        (
            if sc.is_row[ix] { sc.w[ix] } else { sc.h[ix] },
            sc.content_main[ix],
        )
    };
    0.0_f64.max(off.min((content - viewport).max(0.0)))
}

/// Clamps a scene entry's main-axis scroll offset.
pub fn clamp_scroll(sc: &Scene, ix: i32, off: f64) -> f64 {
    clamp_scroll_axis(sc, ix, 0, off)
}
#[allow(clippy::too_many_arguments)]
fn wheel_axis(
    d: &Doc,
    st: &mut St,
    sc: &Scene,
    path: &[i32],
    axis: u32,
    delta: f64,
    eff: &mut Effects,
) {
    if delta == 0.0 {
        return;
    }
    let required = if axis == 0 {
        slir::F_SCROLL
    } else {
        slir::F_SCROLL_CROSS
    };
    let Some(&scene_index) = path.iter().rev().find(|&&scene_index| {
        sc.flags[usize::try_from(scene_index).expect("negative scene index")] & required != 0
    }) else {
        return;
    };
    let index = usize::try_from(scene_index).expect("negative scene index");
    let node = sc.node[index];
    let current = style::scroll_get_axis(st, node, axis);
    let next = clamp_scroll_axis(sc, scene_index, axis, current + delta);
    record_scroll(d, st, node, axis, next, eff);
}

/// Routes main-axis navigation keys to a focused scroll node.
///
/// Arrows step 40u, or 200u with Shift held (fast scroll). `false` leaves
/// the key available to activation, editing, or focus-ring navigation.
pub fn scroll_key(
    d: &Doc,
    st: &mut St,
    sc: &Scene,
    node: u32,
    key: &str,
    mods: u32,
    eff: &mut Effects,
) -> bool {
    let scene_index = scene::index_of(sc, node);
    if scene_index < 0 {
        return false;
    }
    let index = usize::try_from(scene_index).expect("negative scene index");
    if sc.flags[index] & slir::F_SCROLL == 0 {
        return false;
    }

    let viewport = if sc.is_row[index] {
        sc.w[index]
    } else {
        sc.h[index]
    };
    let current = style::scroll_get(st, node);
    let arrow = if mods & M_SHIFT != 0 { 200.0 } else { 40.0 };
    let next = match key {
        "ArrowLeft" if sc.is_row[index] => current - arrow,
        "ArrowRight" if sc.is_row[index] => current + arrow,
        "ArrowUp" if !sc.is_row[index] => current - arrow,
        "ArrowDown" if !sc.is_row[index] => current + arrow,
        "PageUp" => current - (viewport - 40.0).max(0.0),
        "PageDown" => current + (viewport - 40.0).max(0.0),
        "Home" => 0.0,
        "End" => sc.content_main[index],
        _ => return false,
    };
    record_scroll(d, st, node, 0, clamp_scroll(sc, scene_index, next), eff);
    true
}

/// Finds a field node's editing-state index.
pub fn ed_ix(ds: &DState, node: u32) -> i32 {
    ds.ed_node
        .iter()
        .position(|candidate| *candidate == node)
        .map_or(-1, |index| {
            i32::try_from(index).expect("too many edit states")
        })
}

/// Binds editing state to a field on first focus, seeded from current content.
pub fn ensure_edit(d: &Doc, st: &mut St, ds: &mut DState, node: u32) {
    if ed_ix(ds, node) >= 0 {
        return;
    }
    ds.ed_node.push(node);
    ds.ed
        .push(edit::es_new(node, &style::content_str(d, st, node)));
}

/// Binds edit state after a focus change when the new focus is a `field=`
/// target; shared by keyboard traversal and host-driven focus.
pub fn bind_edit_on_focus(d: &Doc, st: &mut St, ds: &mut DState) {
    let focused = ds.fs.focus;
    if focused != slir::NONE && sig_of(d, st, focused, 1) >= 0 {
        ensure_edit(d, st, ds, focused);
    }
}

/// Writes an edit's display text back into style and emits a change signal.
pub fn sync_field(
    d: &Doc,
    st: &mut St,
    ds: &DState,
    eff: &mut Effects,
    ei: i32,
    text_changed: bool,
) {
    let index = usize::try_from(ei).expect("negative edit index");
    let node = ds.ed_node[index];
    style::field_set(st, node, &edit::display_str(&ds.ed[index]));
    eff.repaint = true;
    if !text_changed {
        return;
    }
    let signal_index = sig_of(d, st, node, 1);
    if signal_index >= 0 {
        let signal_index = usize::try_from(signal_index).expect("negative signal index");
        emit_signal(
            d,
            st,
            eff,
            signal_index,
            node,
            edit::text_str(&ds.ed[index]),
        );
    }
}

/// Finds the last resolved style for a node.
pub fn rstyle_ix(st: &St, node: u32) -> i32 {
    st.rs
        .iter()
        .rposition(|resolved| resolved.node == node)
        .map_or(-1, |index| {
            i32::try_from(index).expect("too many resolved styles")
        })
}

/// Populates caret and IME rectangles for the focused editable field.
///
/// Geometry comes from the last solve. Hosts refresh it after the next frame.
/// The IME rectangle intentionally equals the caret rectangle.
pub fn caret_effects(d: &Doc, st: &St, lay: &Lay, sc: &Scene, ds: &DState, eff: &mut Effects) {
    let node = ds.fs.focus;
    if node == slir::NONE {
        return;
    }
    let edit_index = ed_ix(ds, node);
    let scene_index = scene::index_of(sc, node);
    if edit_index < 0 || scene_index < 0 {
        return;
    }
    let edit_index = usize::try_from(edit_index).expect("negative edit index");
    let scene_index = usize::try_from(scene_index).expect("negative scene index");

    // Use the field's latest resolved text style from the last solve.
    let (font, size, tracking, pad_top, pad_left, pad_right, align) =
        if let Some(resolved) = st.rs.iter().rev().find(|resolved| resolved.node == node) {
            let align = match resolved.talign {
                1 => 0.5,
                2 => 1.0,
                _ => 0.0,
            };
            (
                resolved.font,
                resolved.size,
                resolved.tracking,
                resolved.pad_t,
                resolved.pad_l,
                resolved.pad_r,
                align,
            )
        } else {
            (-1, 14.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        };

    let text = edit::display_str(&ds.ed[edit_index]);
    let caret = edit::display_caret(&ds.ed[edit_index]);
    let text_layout_index = crate::layout::text_layout_ix(lay, node);
    let (line, line_start, line_height, line_width) = if text_layout_index >= 0 {
        let text_layout =
            &lay.tls[usize::try_from(text_layout_index).expect("negative text layout index")];
        if text_layout.src_ls.is_empty() {
            (0, 0, sc.h[scene_index], 0.0)
        } else {
            let line = line_of(text_layout, caret);
            let index = usize::try_from(line).expect("negative line index");
            (
                line,
                text_layout.src_ls[index],
                text_layout.line_h,
                text_layout.line_w[index],
            )
        }
    } else {
        (0, 0, sc.h[scene_index], 0.0)
    };

    let advance = crate::textm::str_slice_w(d, font, size, tracking, &text, line_start, caret);
    let content_width = sc.w[scene_index] - pad_left - pad_right;
    let origin = sc.x[scene_index] + pad_left + (content_width - line_width) * align;
    eff.focus = node;
    eff.has_caret = true;
    eff.caret_x = origin + advance - ds.ed[edit_index].scroll_x;
    eff.caret_y = sc.y[scene_index] + pad_top + f64::from(line) * line_height;
    eff.caret_w = 1.0;
    eff.caret_h = line_height;
    eff.has_ime = true;
    eff.ime_x = eff.caret_x;
    eff.ime_y = eff.caret_y;
    eff.ime_w = eff.caret_w;
    eff.ime_h = eff.caret_h;
}

/// Delivers one authored trigger for `node`, returning whether it was declared.
pub fn deliver_trigger(
    d: &Doc,
    st: &St,
    eff: &mut Effects,
    node: u32,
    trigger: u32,
    text: String,
) -> bool {
    let signal_index = sig_of(d, st, node, trigger);
    if signal_index < 0 {
        return false;
    }
    let signal_index = usize::try_from(signal_index).expect("negative signal index");
    emit_signal(d, st, eff, signal_index, node, text);
    eff.repaint = true;
    true
}

/// Delivers an activation signal for `node`, when one is declared.
pub fn deliver_activate(d: &Doc, st: &St, eff: &mut Effects, node: u32) {
    deliver_trigger(d, st, eff, node, TR_ACTIVATE, String::new());
}

/// Reports whether `node` is disabled.
pub fn disabled(d: &Doc, st: &St, node: u32) -> bool {
    style::node_state_on(d, st, node, "disabled")
}

#[derive(Clone, Copy)]
struct DividerSnapshot {
    row: bool,
    current: f64,
    min: f64,
    max: f64,
    budget_max: f64,
}

fn divider_scene_siblings(d: &Doc, st: &St, sc: &Scene, node: u32) -> Option<(usize, usize, bool)> {
    let divider = usize::try_from(scene::index_of(sc, node)).ok()?;
    if sc.kind.get(divider) != Some(&slir::K_DIVIDER) {
        return None;
    }
    let parent_i32 = *sc.parent.get(divider)?;
    let parent = usize::try_from(parent_i32).ok()?;
    if !matches!(sc.kind.get(parent), Some(&slir::K_ROW) | Some(&slir::K_COL)) {
        return None;
    }

    let base = list::base(&st.lists, d, node);
    let base_index = usize::try_from(base).ok()?;
    let parent_base = *d.node_parent.get(base_index)?;
    let parent_base_index = usize::try_from(parent_base).ok()?;
    let next_base = *d.node_next.get(base_index)?;
    if next_base == slir::NONE {
        return None;
    }
    let mut sibling = *d.node_first.get(parent_base_index)?;
    let mut previous_base = slir::NONE;
    while sibling != slir::NONE && sibling != base {
        previous_base = sibling;
        sibling = *d.node_next.get(usize::try_from(sibling).ok()?)?;
    }
    if sibling != base || previous_base == slir::NONE {
        return None;
    }

    let mut previous = None;
    let mut next = None;
    for index in 0..sc.node.len() {
        if sc.parent[index] != parent_i32 {
            continue;
        }
        let candidate_base = list::base(&st.lists, d, sc.node[index]);
        if candidate_base == previous_base {
            previous = Some(index);
        } else if candidate_base == next_base {
            next = Some(index);
        }
    }
    Some((previous?, next?, sc.is_row[parent]))
}

fn resolved_axis_bounds(st: &St, node: u32, row: bool) -> (f64, f64) {
    st.rs
        .iter()
        .rev()
        .find(|resolved| resolved.node == node)
        .map_or((0.0, style::INF), |resolved| {
            if row {
                (resolved.min_w, resolved.max_w)
            } else {
                (resolved.min_h, resolved.max_h)
            }
        })
}

fn divider_snapshot(d: &Doc, st: &St, sc: &Scene, node: u32) -> Option<DividerSnapshot> {
    let (previous, next, row) = divider_scene_siblings(d, st, sc, node)?;
    let previous_extent = if row { sc.w[previous] } else { sc.h[previous] };
    let next_extent = if row { sc.w[next] } else { sc.h[next] };
    let (min, authored_max) = resolved_axis_bounds(st, sc.node[previous], row);
    let (next_min, _) = resolved_axis_bounds(st, sc.node[next], row);
    let budget_max = previous_extent + (next_extent - next_min).max(0.0);
    let max = authored_max.min(budget_max);
    let requested = style::divider_get(st, node).unwrap_or(previous_extent);
    Some(DividerSnapshot {
        row,
        current: style::divider_clamp(requested, min, max, budget_max),
        min,
        max,
        budget_max,
    })
}

/// Clamps a divider request against the current adjacent-pane geometry.
pub(crate) fn clamp_divider_for_scene(
    d: &Doc,
    st: &St,
    sc: &Scene,
    node: u32,
    requested: f64,
) -> Option<f64> {
    let bounds = divider_snapshot(d, st, sc, node)?;
    Some(style::divider_clamp(
        requested,
        bounds.min,
        bounds.max,
        bounds.budget_max,
    ))
}

fn path_divider_node(d: &Doc, st: &St, sc: &Scene, path: &[i32]) -> u32 {
    path.iter()
        .rev()
        .find_map(|&scene_index| {
            let index = usize::try_from(scene_index).expect("negative scene index");
            let node = sc.node[index];
            (sc.kind[index] == slir::K_DIVIDER && !disabled(d, st, node)).then_some(node)
        })
        .unwrap_or(slir::NONE)
}

fn arm_divider(d: &Doc, st: &St, sc: &Scene, node: u32, x: f64, y: f64) -> Option<DividerDrag> {
    let bounds = divider_snapshot(d, st, sc, node)?;
    Some(DividerDrag {
        node,
        row: bounds.row,
        start_pos: if bounds.row { x } else { y },
        start_extent: bounds.current,
        current_extent: bounds.current,
        min_extent: bounds.min,
        max_extent: bounds.max,
        moved: false,
    })
}

fn update_divider_request(divider: &mut DividerDrag, x: f64, y: f64) {
    let position = if divider.row { x } else { y };
    if !position.is_finite() {
        return;
    }
    let delta = position - divider.start_pos;
    if delta != 0.0 {
        divider.moved = true;
    }
    divider.current_extent = style::divider_clamp(
        divider.start_extent + delta,
        divider.min_extent,
        divider.max_extent,
        divider.max_extent,
    );
}

fn move_divider(st: &mut St, divider: &mut DividerDrag, x: f64, y: f64) -> bool {
    update_divider_request(divider, x, y);
    divider.moved && style::divider_set(st, divider.node, divider.current_extent)
}

fn release_divider(
    d: &Doc,
    st: &mut St,
    sc: &Scene,
    divider: &mut DividerDrag,
    x: f64,
    y: f64,
) -> bool {
    // Snapshot the latest solved bounds before calculating or storing the final
    // request, preserving a tighter extent written by the preceding layout.
    let fresh = divider_snapshot(d, st, sc, divider.node);
    update_divider_request(divider, x, y);
    if let Some(bounds) = fresh {
        let min = divider.min_extent.max(bounds.min);
        let max = divider.max_extent.min(bounds.max);
        divider.current_extent = style::divider_clamp(divider.current_extent, min, max, max);
    }
    divider.moved && style::divider_set(st, divider.node, divider.current_extent)
}

fn divider_key(
    d: &Doc,
    st: &mut St,
    sc: &Scene,
    node: u32,
    key: &str,
    mods: u32,
    eff: &mut Effects,
) -> bool {
    if disabled(d, st, node) {
        return false;
    }
    let Some(bounds) = divider_snapshot(d, st, sc, node) else {
        return false;
    };
    let direction = match (bounds.row, key) {
        (true, "ArrowLeft") | (false, "ArrowUp") => -1.0,
        (true, "ArrowRight") | (false, "ArrowDown") => 1.0,
        _ => return false,
    };
    let step = if mods & M_SHIFT != 0 { 1.0 } else { 8.0 };
    let next = style::divider_clamp(
        bounds.current + direction * step,
        bounds.min,
        bounds.max,
        bounds.max,
    );
    eff.repaint |= style::divider_set(st, node, next);
    deliver_trigger(d, st, eff, node, TR_RESIZE, crate::value::fmt3(next));
    true
}

fn path_trigger_node(d: &Doc, st: &St, sc: &Scene, path: &[i32], trigger: u32) -> u32 {
    path.iter()
        .rev()
        .find_map(|&scene_index| {
            let node = sc.node[usize::try_from(scene_index).expect("negative scene index")];
            (sig_of(d, st, node, trigger) >= 0 && !disabled(d, st, node)).then_some(node)
        })
        .unwrap_or(slir::NONE)
}

fn routed_pointer_path(sc: &Scene, ds: &DState, hit_path: &[i32], out: &mut Vec<i32>) {
    let captured = scene::index_of(sc, ds.pressed);
    if ds.pressed != slir::NONE && captured >= 0 {
        scene::chain(sc, captured, out);
    } else {
        out.clear();
        out.extend_from_slice(hit_path);
    }
}

fn scene_node_in_subtree(sc: &Scene, mut scene_index: i32, root: u32) -> bool {
    while scene_index >= 0 {
        let index = usize::try_from(scene_index).expect("negative scene index");
        if sc.node[index] == root {
            return true;
        }
        scene_index = sc.parent[index];
    }
    false
}

fn drag_target_of(d: &Doc, st: &St, sc: &Scene, path: &[i32], source: u32) -> u32 {
    path.iter()
        .rev()
        .find_map(|&scene_index| {
            let index = usize::try_from(scene_index).expect("negative scene index");
            let node = sc.node[index];
            (sig_of(d, st, node, TR_DROP) >= 0
                && !disabled(d, st, node)
                && !scene_node_in_subtree(sc, scene_index, source))
            .then_some(node)
        })
        .unwrap_or(slir::NONE)
}

fn set_drop_target(d: &Doc, st: &mut St, ds: &mut DState, target: u32) -> bool {
    if ds.drop_target == target {
        return false;
    }
    let mut changed = false;
    if ds.drop_target != slir::NONE {
        changed |= style::set_node_state(d, st, ds.drop_target, "drop", false);
    }
    ds.drop_target = target;
    if target != slir::NONE {
        changed |= style::set_node_state(d, st, target, "drop", true);
    }
    changed
}

fn clear_drag(d: &Doc, st: &mut St, ds: &mut DState, eff: &mut Effects) {
    // Active drag ink (including `drag-ghost`) changes even when no authored
    // state patch or optional DragEnd binding exists.
    eff.repaint |= ds.drag_active;
    if ds.drop_target != slir::NONE {
        eff.repaint |= style::set_node_state(d, st, ds.drop_target, "drop", false);
    }
    if ds.drag_active && ds.drag_source != slir::NONE {
        eff.repaint |= style::set_node_state(d, st, ds.drag_source, "dragging", false);
    }
    ds.drag_source = slir::NONE;
    ds.drop_target = slir::NONE;
    ds.drag_active = false;
    ds.suppress_activate = false;
    ds.drag_update_name = None;
    ds.drag_end_name = None;
    ds.drag_source_key.clear();
    ds.drag_source_item.clear();
}

fn clear_pressed(d: &Doc, st: &mut St, ds: &mut DState, eff: &mut Effects) {
    if ds.pressed != slir::NONE {
        eff.repaint |= style::set_node_state(d, st, ds.pressed, "pressed", false);
        ds.pressed = slir::NONE;
    }
}

fn cancel_pointer(d: &Doc, st: &mut St, ds: &mut DState, eff: &mut Effects) {
    clear_pressed(d, st, ds, eff);
    clear_drag(d, st, ds, eff);
    ds.divider = None;
}

/// Cancels a pointer gesture whose armed drag source is absent or disabled.
///
/// Called after loading a freshly solved scene so dynamic visibility changes
/// cannot leave capture or Drop styling alive until another host event.
pub(crate) fn cancel_invalid_drag(d: &Doc, st: &mut St, sc: &Scene, ds: &mut DState) -> bool {
    if ds.drag_source == slir::NONE
        || scene::index_of(sc, ds.drag_source) >= 0 && !disabled(d, st, ds.drag_source)
    {
        return false;
    }
    queue_drag_end(ds);
    let mut effects = effects_new();
    cancel_pointer(d, st, ds, &mut effects);
    true
}

/// Reports whether a comma-separated `keys=` value contains `key`.
pub fn key_list_has(keys: &str, key: &str) -> bool {
    !keys.is_empty() && keys.split(',').any(|candidate| candidate == key)
}

/// Delivers to the nearest enabled `keys=` node on the focused scene path.
pub fn activate_key_path(
    d: &Doc,
    st: &St,
    sc: &Scene,
    focused: u32,
    key: &str,
    eff: &mut Effects,
) -> bool {
    let mut scene_index = scene::index_of(sc, focused);
    while scene_index >= 0 {
        let index = usize::try_from(scene_index).expect("negative scene index");
        let node = sc.node[index];
        if key_list_has(&style::attr_str(d, st, node, slir::A_KEYS), key) && !disabled(d, st, node)
        {
            deliver_activate(d, st, eff, node);
            return true;
        }
        scene_index = sc.parent[index];
    }
    false
}

/// Reports whether an editable node accepts multiple lines.
pub fn multiline(d: &Doc, st: &St, node: u32) -> bool {
    let base = list::base(&st.lists, d, node);
    d.node_flags[usize::try_from(base).expect("node id does not fit usize")] & slir::F_MULTILINE
        != 0
}

/// Finds the text-layout line containing a source position.
pub fn line_of(tl: &TextLayout, at: i32) -> i32 {
    if tl.src_ls.is_empty() {
        return 0;
    }
    for (index, &end) in tl.src_le.iter().enumerate() {
        let line = i32::try_from(index).expect("too many text lines");
        if at < end {
            return line;
        }
        if at == end {
            if tl.src_ls.get(index + 1).is_some_and(|start| *start == at) {
                continue;
            }
            return line;
        }
    }
    i32::try_from(tl.src_ls.len() - 1).expect("too many text lines")
}

/// Emits a submit signal containing the field's committed text.
pub fn emit_submit(d: &Doc, st: &St, ds: &DState, eff: &mut Effects, ei: i32) {
    let edit_index = usize::try_from(ei).expect("negative edit index");
    let node = ds.ed_node[edit_index];
    let signal_index = sig_of(d, st, node, 2);
    if signal_index < 0 {
        return;
    }
    let signal_index = usize::try_from(signal_index).expect("negative signal index");
    emit_signal(
        d,
        st,
        eff,
        signal_index,
        node,
        edit::text_str(&ds.ed[edit_index]),
    );
    eff.repaint = true;
}

/// Replaces line breaks with spaces for a single-line field.
pub fn single_line_text(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' | '\r' => ' ',
            _ => character,
        })
        .collect()
}

/// Scrolls the focused field and its nearest scroll ancestor to reveal the caret.
pub fn follow_caret(
    d: &Doc,
    st: &mut St,
    lay: &Lay,
    sc: &Scene,
    ds: &mut DState,
    ei: i32,
    eff: &mut Effects,
) {
    let edit_index = usize::try_from(ei).expect("negative edit index");
    let node = ds.ed_node[edit_index];
    let scene_index = scene::index_of(sc, node);
    if scene_index < 0 {
        return;
    }
    let scene_index = usize::try_from(scene_index).expect("negative scene index");

    let (font, size, tracking, pad_top, pad_left, pad_right, align) =
        if let Some(resolved) = st.rs.iter().rev().find(|resolved| resolved.node == node) {
            let align = match resolved.talign {
                1 => 0.5,
                2 => 1.0,
                _ => 0.0,
            };
            (
                resolved.font,
                resolved.size,
                resolved.tracking,
                resolved.pad_t,
                resolved.pad_l,
                resolved.pad_r,
                align,
            )
        } else {
            (-1, 14.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        };

    let text_layout_index = crate::layout::text_layout_ix(lay, node);
    let caret = edit::display_caret(&ds.ed[edit_index]);
    let (line, line_start, line_height, line_width) = if text_layout_index >= 0 {
        let text_layout =
            &lay.tls[usize::try_from(text_layout_index).expect("negative text layout index")];
        if text_layout.src_ls.is_empty() {
            (0, 0, sc.h[scene_index], 0.0)
        } else {
            let line = line_of(text_layout, caret);
            let line_index = usize::try_from(line).expect("negative line index");
            (
                line,
                text_layout.src_ls[line_index],
                text_layout.line_h,
                text_layout.line_w[line_index],
            )
        }
    } else {
        (0, 0, sc.h[scene_index], 0.0)
    };
    let advance = crate::textm::str_slice_w(
        d,
        font,
        size,
        tracking,
        &edit::display_str(&ds.ed[edit_index]),
        line_start,
        caret,
    );

    if !multiline(d, st, node) {
        let old_scroll = ds.ed[edit_index].scroll_x;
        let content_width = sc.w[scene_index] - pad_left - pad_right;
        let origin = pad_left + (content_width - line_width) * align;
        let left = pad_left + 8.0;
        let right = left.max(sc.w[scene_index] - pad_right - 8.0);
        let shown = origin + advance - old_scroll;
        if shown < left {
            ds.ed[edit_index].scroll_x = (origin + advance - left).max(0.0);
        } else if shown > right {
            ds.ed[edit_index].scroll_x = (origin + advance - right).max(0.0);
        }
        style::field_scroll_set(st, node, ds.ed[edit_index].scroll_x);
        eff.repaint |= ds.ed[edit_index].scroll_x != old_scroll;
        return;
    }

    let top = sc.y[scene_index] + pad_top + f64::from(line) * line_height;
    let bottom = top + line_height;
    let mut parent = sc.parent[scene_index];
    let mut found_scroll_parent = false;
    while parent >= 0 {
        let parent_index = usize::try_from(parent).expect("negative parent index");
        if !found_scroll_parent && sc.flags[parent_index] & slir::F_SCROLL != 0 {
            let parent_node = sc.node[parent_index];
            let mut next = style::scroll_get(st, parent_node);
            if top < sc.y[parent_index] {
                next -= sc.y[parent_index] - top;
            } else if bottom > sc.y[parent_index] + sc.h[parent_index] {
                next += bottom - (sc.y[parent_index] + sc.h[parent_index]);
            }
            record_scroll(d, st, parent_node, 0, clamp_scroll(sc, parent, next), eff);
            found_scroll_parent = true;
        }
        parent = sc.parent[parent_index];
    }
}

/// Re-runs caret following against a freshly solved text layout and scene.
///
/// Returns `true` when one more settle solve is required.
pub fn follow_caret_fresh(d: &Doc, st: &mut St, lay: &Lay, sc: &Scene, ds: &mut DState) -> bool {
    if ds.fs.focus == slir::NONE {
        return false;
    }
    let edit_index = ed_ix(ds, ds.fs.focus);
    if edit_index < 0 {
        return false;
    }
    let mut effects = effects_new();
    follow_caret(d, st, lay, sc, ds, edit_index, &mut effects);
    effects.repaint
}

/// Consumes the focused field's editing keys before activation-key bubbling.
#[allow(clippy::too_many_arguments)] // Routing needs the retained model, scene, state, and event parts together.
pub fn route_edit_key(
    d: &Doc,
    st: &mut St,
    lay: &Lay,
    sc: &Scene,
    ds: &mut DState,
    node: u32,
    key: &str,
    mods: u32,
    eff: &mut Effects,
) -> bool {
    let edit_index = ed_ix(ds, node);
    if edit_index < 0 {
        return false;
    }
    let index = usize::try_from(edit_index).expect("negative edit index");
    let selecting = mods & M_SHIFT != 0;
    let alt = mods & M_ALT != 0;
    let control = mods & M_CTRL != 0;
    let command = mods & (M_META | M_CTRL) != 0;
    let is_multiline = multiline(d, st, node);
    let text_layout_index = crate::layout::text_layout_ix(lay, node);
    let text_layout = (text_layout_index >= 0)
        .then(|| &lay.tls[usize::try_from(text_layout_index).expect("negative text layout index")]);
    let mut text_changed = false;
    let mut refresh = true;

    match key {
        "Enter" => {
            let submits = sig_of(d, st, node, 2) >= 0;
            if is_multiline && (!submits || selecting || alt) {
                text_changed = edit::insert(&mut ds.ed[index], "\n");
            } else if submits && (!is_multiline || mods == 0) {
                emit_submit(d, st, ds, eff, edit_index);
            } else {
                refresh = false;
            }
        }
        "Backspace" => {
            text_changed = if control || alt {
                edit::word_back(&mut ds.ed[index])
            } else {
                edit::backspace(&mut ds.ed[index])
            };
        }
        "Delete" => {
            text_changed = if control || alt {
                edit::word_forward(&mut ds.ed[index])
            } else {
                edit::del(&mut ds.ed[index])
            };
        }
        "w" | "W" if command => {
            edit::history_barrier(&mut ds.ed[index]);
            text_changed = edit::word_back(&mut ds.ed[index]);
        }
        "z" | "Z" if command => {
            text_changed = if selecting {
                edit::redo(&mut ds.ed[index])
            } else {
                edit::undo(&mut ds.ed[index])
            };
        }
        "k" | "K" if control => {
            if let Some(text_layout) = text_layout {
                text_changed = edit::kill_end(&mut ds.ed[index], text_layout);
            }
        }
        "u" | "U" if control => {
            if let Some(text_layout) = text_layout {
                text_changed = edit::kill_start(&mut ds.ed[index], text_layout);
            }
        }
        "ArrowLeft" => {
            if command {
                edit::home(&mut ds.ed[index], selecting);
            } else {
                edit::move_caret(&mut ds.ed[index], -1, selecting, alt);
            }
        }
        "ArrowRight" => {
            if command {
                edit::end(&mut ds.ed[index], selecting);
            } else {
                edit::move_caret(&mut ds.ed[index], 1, selecting, alt);
            }
        }
        "ArrowUp" | "ArrowDown" if is_multiline => {
            if let Some(text_layout) = text_layout {
                let (font, size, tracking) = st
                    .rs
                    .iter()
                    .rev()
                    .find(|resolved| resolved.node == node)
                    .map_or((-1, 14.0, 0.0), |resolved| {
                        (resolved.font, resolved.size, resolved.tracking)
                    });
                let delta = if key == "ArrowUp" { -1 } else { 1 };
                edit::visual_move(
                    d,
                    &mut ds.ed[index],
                    text_layout,
                    font,
                    size,
                    tracking,
                    delta,
                    selecting,
                );
            }
        }
        "Home" => {
            if is_multiline && !command {
                if let Some(text_layout) = text_layout {
                    edit::visual_home(&mut ds.ed[index], text_layout, selecting);
                }
            } else {
                edit::home(&mut ds.ed[index], selecting);
            }
        }
        "End" => {
            if is_multiline && !command {
                if let Some(text_layout) = text_layout {
                    edit::visual_end(&mut ds.ed[index], text_layout, selecting);
                }
            } else {
                edit::end(&mut ds.ed[index], selecting);
            }
        }
        "a" | "A" if command => edit::select_all(&mut ds.ed[index]),
        _ => return false,
    }

    if refresh {
        sync_field(d, st, ds, eff, edit_index, text_changed);
        follow_caret(d, st, lay, sc, ds, edit_index, eff);
    }
    true
}

/// Routes one event through the retained scene and aggregates its host effects.
///
/// `repaint` means document state changed and the next frame must re-solve.
/// Pointer routing uses the retained scene from the last solve.
pub fn dispatch(
    d: &Doc,
    st: &mut St,
    lay: &Lay,
    sc: &Scene,
    ds: &mut DState,
    ev: &Event,
) -> Effects {
    let mut effects = effects_new();
    let mut path = Vec::new();
    if matches!(
        ev.etype,
        E_POINTER_MOVE | E_POINTER_DOWN | E_POINTER_UP | E_WHEEL
    ) {
        crate::hit::hit_test(sc, ev.x, ev.y, &mut path);
    }

    match ev.etype {
        E_POINTER_MOVE => {
            if ds.drag_source != slir::NONE {
                remember_drag_event(ds, ev);
            }
            let mut signal_path = Vec::new();
            routed_pointer_path(sc, ds, &path, &mut signal_path);
            let pointer_target = path_trigger_node(d, st, sc, &signal_path, TR_POINTER_MOVE);
            if pointer_target != slir::NONE {
                let emitted = deliver_trigger(
                    d,
                    st,
                    &mut effects,
                    pointer_target,
                    TR_POINTER_MOVE,
                    String::new(),
                );
                if emitted
                    && ds.drag_active
                    && let Some(meta) = effects.sig_meta.last_mut()
                {
                    apply_drag_meta(meta, ds, false, false);
                }
            }
            // Hover enter/leave applies to the entire hit path.
            let mut changed = false;
            for &hovered in &ds.hover {
                let still_hovered = path.iter().any(|&scene_index| {
                    sc.node[usize::try_from(scene_index).expect("negative scene index")] == hovered
                });
                if !still_hovered && style::set_node_state(d, st, hovered, "hover", false) {
                    changed = true;
                }
            }

            let mut next_hover = Vec::with_capacity(path.len());
            for &scene_index in &path {
                let node = sc.node[usize::try_from(scene_index).expect("negative scene index")];
                if !ds.hover.contains(&node) && style::set_node_state(d, st, node, "hover", true) {
                    changed = true;
                }
                next_hover.push(node);
            }
            ds.hover = next_hover;
            effects.repaint |= changed;

            let cancel_divider = ds.divider.as_ref().is_some_and(|divider| {
                ds.pressed != divider.node
                    || scene::index_of(sc, divider.node) < 0
                    || disabled(d, st, divider.node)
            });
            if cancel_divider {
                ds.divider = None;
            } else if let Some(divider) = ds.divider.as_mut()
                && move_divider(st, divider, ev.x, ev.y)
            {
                effects.repaint = true;
                deliver_trigger(
                    d,
                    st,
                    &mut effects,
                    divider.node,
                    TR_RESIZE,
                    crate::value::fmt3(divider.current_extent),
                );
            }

            if ds.drag_source != slir::NONE
                && (ds.pressed == slir::NONE
                    || scene::index_of(sc, ds.drag_source) < 0
                    || disabled(d, st, ds.drag_source))
            {
                emit_drag_end(&mut effects, ds, true, false);
                clear_drag(d, st, ds, &mut effects);
            }
            if ds.drag_source != slir::NONE && !ds.drag_active {
                let dx = ev.x - ds.drag_x;
                let dy = ev.y - ds.drag_y;
                if dx * dx + dy * dy > 16.0 {
                    ds.drag_active = true;
                    ds.suppress_activate = true;
                    effects.repaint |=
                        style::set_node_state(d, st, ds.drag_source, "dragging", true);
                    if deliver_trigger(
                        d,
                        st,
                        &mut effects,
                        ds.drag_source,
                        TR_DRAG_START,
                        String::new(),
                    ) && let Some(meta) = effects.sig_meta.last_mut()
                    {
                        apply_drag_meta(meta, ds, false, false);
                    }
                }
            }
            if ds.drag_active {
                // The opt-in ghost follows every active move independently of
                // authored state patches or an optional DragUpdate binding.
                effects.repaint = true;
                emit_drag_update(&mut effects, ds);
                let target = drag_target_of(d, st, sc, &path, ds.drag_source);
                effects.repaint |= set_drop_target(d, st, ds, target);
            }

            // The uncaptured target decides the cursor, with the target first.
            ds.cursor = CUR_DEFAULT;
            for &scene_index in path.iter().rev() {
                let index = usize::try_from(scene_index).expect("negative scene index");
                let node = sc.node[index];
                if sc.kind[index] == slir::K_DIVIDER
                    && !disabled(d, st, node)
                    && let Some((_, _, row)) = divider_scene_siblings(d, st, sc, node)
                {
                    ds.cursor = if row { CUR_COL_RESIZE } else { CUR_ROW_RESIZE };
                    break;
                }
                if sig_of(d, st, node, TR_CHANGE) >= 0 && !disabled(d, st, node) {
                    ds.cursor = CUR_TEXT;
                    break;
                }
                if sc.flags[index] & slir::F_FOCUSABLE != 0 && !disabled(d, st, node) {
                    ds.cursor = CUR_POINTER;
                    break;
                }
            }
        }
        E_POINTER_DOWN => {
            // A fresh down cancels stale capture without disturbing keyboard
            // focus. Secondary and auxiliary buttons never press or focus.
            if ds.drag_active {
                remember_drag_event(ds, ev);
                emit_drag_end(&mut effects, ds, true, false);
            }
            cancel_pointer(d, st, ds, &mut effects);
            let divider_target = if ev.button == 0 {
                path_divider_node(d, st, sc, &path)
            } else {
                slir::NONE
            };
            if ev.button == 2 {
                let target = path_trigger_node(d, st, sc, &path, TR_CONTEXT);
                if target != slir::NONE {
                    deliver_trigger(d, st, &mut effects, target, TR_CONTEXT, String::new());
                }
            } else if ev.button == 0 {
                // Press is observable before capture/focus side effects.
                let press_target = path_trigger_node(d, st, sc, &path, TR_PRESS);
                if press_target != slir::NONE {
                    deliver_trigger(d, st, &mut effects, press_target, TR_PRESS, String::new());
                }
                if ev.clicks == 2 {
                    if divider_target != slir::NONE {
                        ds.suppress_activate = true;
                        effects.repaint |= style::divider_clear(st, divider_target);
                        deliver_trigger(
                            d,
                            st,
                            &mut effects,
                            divider_target,
                            TR_DBLCLICK,
                            String::new(),
                        );
                    }
                    let target = path_trigger_node(d, st, sc, &path, TR_DBLCLICK);
                    if target != slir::NONE && target != divider_target {
                        ds.suppress_activate = true;
                        deliver_trigger(d, st, &mut effects, target, TR_DBLCLICK, String::new());
                    }
                }
                ds.drag_source = if divider_target == slir::NONE {
                    path_trigger_node(d, st, sc, &path, TR_DRAG_START)
                } else {
                    slir::NONE
                };
                ds.drag_x = ev.x;
                ds.drag_y = ev.y;
                if ds.drag_source != slir::NONE {
                    remember_drag_event(ds, ev);
                    ds.drag_source_key = scene::key_of(d, &st.lists, ds.drag_source);
                    ds.drag_source_item = list::item_key(&st.lists, d, ds.drag_source);
                    ds.drag_update_name = signal_name_of(d, st, ds.drag_source, TR_DRAG_UPDATE);
                    ds.drag_end_name = signal_name_of(d, st, ds.drag_source, TR_DRAG_END);
                    let source_index = scene::index_of(sc, ds.drag_source);
                    if source_index >= 0 {
                        let source_index =
                            usize::try_from(source_index).expect("negative scene index");
                        ds.drag_grab_x = ev.x - sc.x[source_index];
                        ds.drag_grab_y = ev.y - sc.y[source_index];
                    }
                }

                // Capture the nearest focusable node, or the raw target. Pointer
                // focus deliberately carries no keyboard focus ring.
                let focus_target = path
                    .iter()
                    .rev()
                    .copied()
                    .find(|&scene_index| {
                        sc.flags[usize::try_from(scene_index).expect("negative scene index")]
                            & slir::F_FOCUSABLE
                            != 0
                    })
                    .map_or(slir::NONE, |scene_index| {
                        sc.node[usize::try_from(scene_index).expect("negative scene index")]
                    });
                let pressed = if focus_target != slir::NONE {
                    focus_target
                } else {
                    path.last().map_or(slir::NONE, |&scene_index| {
                        sc.node[usize::try_from(scene_index).expect("negative scene index")]
                    })
                };
                if pressed != slir::NONE {
                    ds.pressed = pressed;
                    effects.repaint |= style::set_node_state(d, st, pressed, "pressed", true);
                }
                if focus_target != slir::NONE && sig_of(d, st, focus_target, TR_CHANGE) >= 0 {
                    ensure_edit(d, st, ds, focus_target);
                }
                effects.repaint |= focus::set_focus(d, st, &mut ds.fs, focus_target, false);
                if divider_target != slir::NONE
                    && ev.clicks != 2
                    && let Some(divider) = arm_divider(d, st, sc, divider_target, ev.x, ev.y)
                {
                    ds.divider = Some(divider);
                    ds.suppress_activate = true;
                }
            }
        }
        E_POINTER_UP => {
            if ds.drag_source != slir::NONE {
                remember_drag_event(ds, ev);
            }
            let mut signal_path = Vec::new();
            routed_pointer_path(sc, ds, &path, &mut signal_path);
            let pointer_target = path_trigger_node(d, st, sc, &signal_path, TR_POINTER_UP);
            if pointer_target != slir::NONE {
                let emitted = deliver_trigger(
                    d,
                    st,
                    &mut effects,
                    pointer_target,
                    TR_POINTER_UP,
                    String::new(),
                );
                if emitted
                    && ds.drag_active
                    && let Some(meta) = effects.sig_meta.last_mut()
                {
                    apply_drag_meta(meta, ds, false, false);
                }
            }
            if ev.button == 0 {
                if let Some(mut divider) = ds.divider.take()
                    && !disabled(d, st, divider.node)
                    && scene::index_of(sc, divider.node) >= 0
                {
                    effects.repaint |= release_divider(d, st, sc, &mut divider, ev.x, ev.y);
                    deliver_trigger(
                        d,
                        st,
                        &mut effects,
                        divider.node,
                        TR_RESIZE,
                        crate::value::fmt3(divider.current_extent),
                    );
                    ds.suppress_activate = true;
                }
                let canceled_drag = ds.drag_active
                    && (ds.drag_source == slir::NONE
                        || scene::index_of(sc, ds.drag_source) < 0
                        || disabled(d, st, ds.drag_source));
                if canceled_drag {
                    emit_drag_end(&mut effects, ds, true, false);
                    clear_drag(d, st, ds, &mut effects);
                }
                let drag_active = ds.drag_active;
                let source = ds.drag_source;
                let mut dropped = false;
                if drag_active {
                    let target = drag_target_of(d, st, sc, &path, source);
                    effects.repaint |= set_drop_target(d, st, ds, target);
                    if target != slir::NONE
                        && deliver_trigger(d, st, &mut effects, target, TR_DROP, String::new())
                    {
                        dropped = true;
                        let meta = effects
                            .sig_meta
                            .last_mut()
                            .expect("delivered drop has metadata");
                        meta.src_key = ds.drag_source_key.clone();
                        meta.src_item = ds.drag_source_item.clone();
                        apply_drag_meta(meta, ds, false, true);
                    }
                }
                if drag_active {
                    emit_drag_end(&mut effects, ds, false, dropped);
                }

                let suppress_activate = ds.suppress_activate || drag_active || canceled_drag;
                let pressed = ds.pressed;
                clear_pressed(d, st, ds, &mut effects);
                clear_drag(d, st, ds, &mut effects);
                if !suppress_activate && pressed != slir::NONE {
                    let pointer_over = path.iter().any(|&scene_index| {
                        sc.node[usize::try_from(scene_index).expect("negative scene index")]
                            == pressed
                    });
                    let scene_index = scene::index_of(sc, pressed);
                    if pointer_over && scene_index >= 0 && !disabled(d, st, pressed) {
                        let index = usize::try_from(scene_index).expect("negative scene index");
                        if sc.flags[index] & slir::F_FOCUSABLE != 0 {
                            deliver_activate(d, st, &mut effects, pressed);
                        }
                    }
                }
            }
        }
        E_WHEEL => {
            let (main_delta, cross_delta) = if ev.mods & M_SHIFT != 0 {
                (ev.dx, ev.dy)
            } else {
                (ev.dy, ev.dx)
            };
            wheel_axis(d, st, sc, &path, 0, main_delta, &mut effects);
            wheel_axis(d, st, sc, &path, 1, cross_delta, &mut effects);
        }
        E_KEY_DOWN => {
            let selecting = ev.mods & M_SHIFT != 0;
            let focused = ds.fs.focus;

            // Editing has precedence, followed by divider adjustment, scrolling,
            // activation-key bubbling, and focus-ring navigation.
            let mut handled = focused != slir::NONE
                && route_edit_key(d, st, lay, sc, ds, focused, &ev.key, ev.mods, &mut effects);
            if !handled && focused != slir::NONE && ed_ix(ds, focused) < 0 {
                handled = divider_key(d, st, sc, focused, &ev.key, ev.mods, &mut effects);
            }
            if !handled && focused != slir::NONE && ed_ix(ds, focused) < 0 {
                handled = scroll_key(d, st, sc, focused, &ev.key, ev.mods, &mut effects);
            }
            if !handled
                && focused != slir::NONE
                && (ed_ix(ds, focused) < 0 || ev.key.chars().count() != 1)
            {
                handled = activate_key_path(d, st, sc, focused, &ev.key, &mut effects);
            }

            if !handled && ev.key == "Tab" {
                if focus::focus_next(d, st, sc, &mut ds.fs, selecting) {
                    effects.repaint = true;
                    bind_edit_on_focus(d, st, ds);
                }
            // Enter and Space activate only focused, non-editable controls.
            } else if !handled
                && matches!(ev.key.as_str(), "Enter" | " " | "Space")
                && focused != slir::NONE
                && !disabled(d, st, focused)
                && ed_ix(ds, focused) < 0
            {
                let scene_index = scene::index_of(sc, focused);
                if scene_index >= 0
                    && sc.flags[usize::try_from(scene_index).expect("negative scene index")]
                        & slir::F_FOCUSABLE
                        != 0
                {
                    deliver_activate(d, st, &mut effects, focused);
                }
            // Without an active editor, arrows walk the focus ring.
            } else if !handled
                && matches!(
                    ev.key.as_str(),
                    "ArrowRight" | "ArrowDown" | "ArrowLeft" | "ArrowUp"
                )
                && (focused == slir::NONE || ed_ix(ds, focused) < 0)
            {
                let backwards = matches!(ev.key.as_str(), "ArrowLeft" | "ArrowUp");
                if focus::focus_next(d, st, sc, &mut ds.fs, backwards) {
                    effects.repaint = true;
                    bind_edit_on_focus(d, st, ds);
                }
            }
        }
        E_TEXT | E_PASTE => {
            let focused = ds.fs.focus;
            let edit_index = ed_ix(ds, focused);
            if focused != slir::NONE && edit_index >= 0 && !ev.text.is_empty() {
                let text = if multiline(d, st, focused) {
                    Cow::Borrowed(ev.text.as_str())
                } else {
                    Cow::Owned(single_line_text(&ev.text))
                };
                let index = usize::try_from(edit_index).expect("negative edit index");
                if ev.etype == E_PASTE {
                    edit::history_barrier(&mut ds.ed[index]);
                }
                let changed = edit::insert(&mut ds.ed[index], text.as_ref());
                sync_field(d, st, ds, &mut effects, edit_index, changed);
                follow_caret(d, st, lay, sc, ds, edit_index, &mut effects);
            }
        }
        E_CUT => {
            let edit_index = ed_ix(ds, ds.fs.focus);
            if ds.fs.focus != slir::NONE && edit_index >= 0 {
                let index = usize::try_from(edit_index).expect("negative edit index");
                edit::history_barrier(&mut ds.ed[index]);
                if edit::delete_selection(&mut ds.ed[index]) {
                    sync_field(d, st, ds, &mut effects, edit_index, true);
                    follow_caret(d, st, lay, sc, ds, edit_index, &mut effects);
                }
            }
        }
        E_COMPOSITION_START => {
            let focused = ds.fs.focus;
            let edit_index = ed_ix(ds, focused);
            if focused != slir::NONE && edit_index >= 0 {
                let index = usize::try_from(edit_index).expect("negative edit index");
                let changed = edit::composition_update(&mut ds.ed[index], "");
                style::set_node_state(d, st, focused, "composing", true);
                sync_field(d, st, ds, &mut effects, edit_index, changed);
            }
        }
        E_COMPOSITION_UPDATE | E_COMPOSITION_END => {
            let focused = ds.fs.focus;
            let edit_index = ed_ix(ds, focused);
            if focused != slir::NONE && edit_index >= 0 {
                let text = if multiline(d, st, focused) {
                    Cow::Borrowed(ev.text.as_str())
                } else {
                    Cow::Owned(single_line_text(&ev.text))
                };
                let index = usize::try_from(edit_index).expect("negative edit index");
                let changed = if ev.etype == E_COMPOSITION_UPDATE {
                    edit::composition_update(&mut ds.ed[index], text.as_ref())
                } else {
                    let changed = edit::composition_end(&mut ds.ed[index], text.as_ref());
                    style::set_node_state(d, st, focused, "composing", false);
                    changed
                };
                sync_field(d, st, ds, &mut effects, edit_index, changed);
                follow_caret(d, st, lay, sc, ds, edit_index, &mut effects);
            }
        }
        E_BLUR => {
            for &hovered in &ds.hover {
                effects.repaint |= style::set_node_state(d, st, hovered, "hover", false);
            }
            ds.hover.clear();
            emit_drag_end(&mut effects, ds, true, false);
            cancel_pointer(d, st, ds, &mut effects);
        }
        E_RESIZE => {
            if ev.dx > 0.0 {
                st.env.vw = ev.dx;
                effects.repaint = true;
            }
            if ev.dy > 0.0 {
                st.env.vh = ev.dy;
                effects.repaint = true;
            }
        }
        E_CLOSE => {
            emit_drag_end(&mut effects, ds, true, false);
            cancel_pointer(d, st, ds, &mut effects);
            ds.closed = true;
        }
        // These host-originated events have no kernel-side semantics.
        E_COPY | E_INSPECT | E_ACTIVATE => {}
        _ => {}
    }

    let pointer_origin = matches!(
        ev.etype,
        E_POINTER_MOVE | E_POINTER_DOWN | E_POINTER_UP | E_WHEEL
    );
    for meta in &mut effects.sig_meta {
        if pointer_origin {
            meta.x = ev.x;
            meta.y = ev.y;
            meta.dx = ev.dx;
            meta.dy = ev.dy;
            if ds.drag_active {
                meta.drag_dx = ev.x - ds.drag_x;
                meta.drag_dy = ev.y - ds.drag_y;
            }
        }
        if !meta.cancelled {
            meta.mods = ev.mods;
            meta.button = ev.button;
            meta.clicks = ev.clicks;
        }
    }

    effects.cursor = ds.cursor;
    effects.focus = ds.fs.focus;
    caret_effects(d, st, lay, sc, ds, &mut effects);
    effects
}
