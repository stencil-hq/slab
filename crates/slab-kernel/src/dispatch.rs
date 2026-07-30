//! Event dispatch and host-visible effects.
//!
//! Primary pointer capture lasts from pointer-down through release. Press,
//! context, double-click, and drag/drop gestures resolve the deepest matching
//! signal in the hit path. Keyboard events deliberately bubble from the focused
//! node through scene parents. Pointer-up and matching key-down events
//! synthesize activation, while disabled nodes suppress delivery. Printable
//! input is routed to editable fields.

use std::borrow::Cow;

use serde::Serialize;

use crate::{
	edit::{self, EditState},
	focus::{self, FSt},
	layout::Lay,
	list,
	scene::{self, Scene},
	slir::{self, Doc},
	style::{self, St},
	textm::{self, TextLayout},
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
/// Resize signal trigger. Dividers deliver live plus gesture-end; split
/// containers deliver at gesture-end and per keyboard adjustment.
pub const TR_RESIZE: u32 = 8;
/// Continuous pointer-move signal trigger.
pub const TR_POINTER_MOVE: u32 = 9;
/// Primary pointer-up signal trigger.
pub const TR_POINTER_UP: u32 = 10;
/// Active-drag movement signal trigger.
pub const TR_DRAG_UPDATE: u32 = 11;
/// Drag termination signal trigger.
pub const TR_DRAG_END: u32 = 12;
/// Typed `keys=Key:signal` activation discriminator.
///
/// Host-facing effects remain ordinary Activate signals; the distinct static
/// trigger prevents Enter/Space from selecting an arbitrary mapped signal.
pub const TR_KEY_ACTIVATE: u32 = 13;
/// Field cancel signal trigger, fired on escape-blur with the retained buffer.
pub const TR_CANCEL: u32 = 14;

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
#[derive(Clone, Debug, Serialize)]
pub struct Event {
	/// One of the `E_*` event type codes.
	pub etype:   u32,
	/// Pointer x-coordinate.
	pub x:       f64,
	/// Pointer y-coordinate.
	pub y:       f64,
	/// Horizontal wheel delta or resized viewport width.
	pub dx:      f64,
	/// Vertical wheel delta or resized viewport height.
	pub dy:      f64,
	/// Pointer button code.
	pub button:  u32,
	/// Host-computed click count for pointer-down (`0`/`1` means single).
	pub clicks:  u32,
	/// Named keyboard key.
	pub key:     String,
	/// Text, paste, or composition payload.
	pub text:    String,
	/// Ordered codepoint ranges within a composition-update preedit.
	pub clauses: Vec<(i32, i32)>,
	/// Bitset of the `M_*` modifier constants.
	pub mods:    u32,
}

/// Metadata attached to every emitted signal.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SigMeta {
	/// Document-space pointer x, or `-1.0` for keyboard-originated signals.
	pub x:           f64,
	/// Document-space pointer y, or `-1.0` for keyboard-originated signals.
	pub y:           f64,
	/// Horizontal delta carried by the originating event.
	pub dx:          f64,
	/// Vertical delta carried by the originating event.
	pub dy:          f64,
	/// Current horizontal drag displacement from the pointer-down origin.
	pub drag_dx:     f64,
	/// Current vertical drag displacement from the pointer-down origin.
	pub drag_dy:     f64,
	/// Modifier bitset active when the signal was emitted.
	pub mods:        u32,
	/// Pointer button code active when the signal was emitted.
	pub button:      u32,
	/// Host-computed click count active when the signal was emitted.
	pub clicks:      u32,
	/// Full key path of the signal-emitting node.
	pub key:         String,
	/// Full key of the deepest hit-target node for pointer-derived signals,
	/// or `""` for keyboard- and host-originated signals.
	pub hit_key:     String,
	/// Named key that drove a keyboard activation (a `keys=` match or
	/// Enter/Space), or `""` for pointer- and host-originated signals.
	pub pressed_key: String,
	/// Full drag-source key for a drop signal.
	pub src_key:     String,
	/// Innermost drag-source item key for a drop signal.
	pub src_item:    String,
	/// Whether a `DragEnd` represents abnormal termination.
	pub cancelled:   bool,
	/// Whether an ordinary `DragEnd` delivered Drop to an eligible target.
	pub dropped:     bool,
}

/// One scroll offset changed by a dispatch.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScrollChange {
	pub key:  String,
	/// `0` is main and `1` is cross.
	pub axis: u32,
	pub off:  f64,
}
/// Cross-field range edit initiated by committed text input.
pub const RANGE_EDIT_TEXT: u32 = 0;
/// Cross-field range edit initiated by paste.
pub const RANGE_EDIT_PASTE: u32 = 1;
/// Cross-field range deletion initiated by cut.
pub const RANGE_EDIT_CUT: u32 = 2;
/// Cross-field range deletion initiated by Backspace.
pub const RANGE_EDIT_BACKSPACE: u32 = 3;
/// Cross-field range deletion initiated by Delete.
pub const RANGE_EDIT_DELETE: u32 = 4;
/// Cross-field range edit initiated by IME composition.
pub const RANGE_EDIT_COMPOSITION: u32 = 5;
/// Non-destructive cross-field copy request.
pub const RANGE_EDIT_COPY: u32 = 6;

/// One stable endpoint in a host-composed cross-field edit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RangeEndpoint {
	/// Escaped canonical full field key.
	pub key:    String,
	/// Grapheme-boundary committed-text codepoint offset.
	pub offset: i32,
}

/// A pre-mutation request that the host must apply to its block model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RangeEdit {
	/// One of the `RANGE_EDIT_*` constants.
	pub kind:   u32,
	/// Fixed selection endpoint.
	pub anchor: RangeEndpoint,
	/// Active selection endpoint.
	pub head:   RangeEndpoint,
	/// Replacement text; empty for deletion, cut, and copy.
	pub text:   String,
}

#[allow(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde skip_serializing_if requires a reference predicate"
)]
const fn is_false(value: &bool) -> bool {
	!*value
}

/// Host-visible consequences of dispatching an [`Event`].
#[derive(Clone, Debug, Serialize)]
pub struct Effects {
	/// Whether the next frame must re-solve.
	pub repaint:              bool,
	/// Document string references, parallel to every `sig_*` payload vector.
	pub sig_name:             Vec<u32>,
	/// Committed text for Change/Submit, final extent for Resize, or empty.
	pub sig_text:             Vec<String>,
	/// Rich-field payload JSON parallel to `sig_name`; empty for non-field
	/// signals.
	pub sig_runs:             Vec<String>,
	/// Innermost list item key, or empty for a real document node.
	pub sig_item:             Vec<String>,
	/// Signal metadata parallel to [`Self::sig_name`].
	pub sig_meta:             Vec<SigMeta>,
	/// Scroll offsets changed by this dispatch.
	pub scrolls:              Vec<ScrollChange>,
	/// Host-owned structural edit requested for an active cross-field range.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub range_edit:           Option<RangeEdit>,
	/// Whether the caret rectangle is available.
	pub has_caret:            bool,
	/// Caret rectangle x-coordinate.
	pub caret_x:              f64,
	/// Caret rectangle y-coordinate.
	pub caret_y:              f64,
	/// Caret rectangle width.
	pub caret_w:              f64,
	/// Caret rectangle height.
	pub caret_h:              f64,
	/// Whether the IME rectangle is available.
	pub has_ime:              bool,
	/// IME rectangle x-coordinate.
	pub ime_x:                f64,
	/// IME rectangle y-coordinate.
	pub ime_y:                f64,
	/// IME rectangle width.
	pub ime_w:                f64,
	/// IME rectangle height.
	pub ime_h:                f64,
	/// Selected text requested by `E_COPY`, or `None` when the kernel does not
	/// own the current clipboard selection.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub copy_text:            Option<String>,
	/// Whether a retained kernel-owned static-text selection is active.
	#[serde(skip_serializing_if = "is_false")]
	pub has_static_selection: bool,
	/// One of the `CUR_*` cursor codes.
	pub cursor:               u32,
	/// Focused node id, or [`slir::NONE`].
	pub focus:                u32,
}

/// Creates an empty effect collection.
pub const fn effects_new() -> Effects {
	Effects {
		repaint:              false,
		sig_name:             Vec::new(),
		sig_text:             Vec::new(),
		sig_runs:             Vec::new(),
		sig_item:             Vec::new(),
		sig_meta:             Vec::new(),
		scrolls:              Vec::new(),
		range_edit:           None,
		has_caret:            false,
		caret_x:              0.0,
		caret_y:              0.0,
		caret_w:              0.0,
		caret_h:              0.0,
		has_ime:              false,
		ime_x:                0.0,
		ime_y:                0.0,
		ime_w:                0.0,
		ime_h:                0.0,
		copy_text:            None,
		has_static_selection: false,
		cursor:               CUR_DEFAULT,
		focus:                slir::NONE,
	}
}

#[derive(Clone, Debug)]
struct DividerDrag {
	node:           u32,
	row:            bool,
	start_pos:      f64,
	start_extent:   f64,
	current_extent: f64,
	min_extent:     f64,
	max_extent:     f64,
	moved:          bool,
}
#[derive(Clone, Debug)]
struct SplitDrag {
	sash:      u32,
	container: u32,
	row:       bool,
	start_pos: f64,
	left:      usize,
	keys:      Vec<String>,
	start:     Vec<f64>,
	current:   Vec<f64>,
	min:       Vec<f64>,
	max:       Vec<f64>,
	moved:     bool,
}

#[derive(Clone, Debug)]
struct PendingSignal {
	name: u32,
	text: String,
	runs: String,
	item: String,
	meta: SigMeta,
}

/// One stable endpoint in a kernel-owned static-text selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticEndpoint {
	/// Escaped canonical full key of the text-bearing scene node.
	pub key:    String,
	/// Grapheme-boundary codepoint offset in that node's painted text.
	pub offset: i32,
}

/// Active pointer selection rooted at one authored `select` box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticSelection {
	/// Stable key of the `select` root.
	pub root_key: String,
	/// Fixed endpoint.
	pub anchor:   StaticEndpoint,
	/// Active endpoint.
	pub focus:    StaticEndpoint,
}

/// Dispatch-owned interaction state, keyed by node id.
#[derive(Clone, Debug)]
pub struct DState {
	/// Keyboard focus state.
	pub fs:                   FSt,
	/// Node ids currently under the pointer, comprising the whole hit path.
	pub hover:                Vec<u32>,
	/// Pointer-captured node, or [`slir::NONE`].
	pub pressed:              u32,
	/// Armed or active drag-source node, or [`slir::NONE`].
	pub drag_source:          u32,
	/// Current eligible drop-target node, or [`slir::NONE`].
	pub drop_target:          u32,
	/// Pointer x-coordinate at drag arm time.
	pub drag_x:               f64,
	/// Pointer y-coordinate at drag arm time.
	pub drag_y:               f64,
	/// Whether the armed drag crossed the four-unit threshold.
	pub drag_active:          bool,
	/// Whether this primary gesture must suppress Activate.
	pub suppress_activate:    bool,
	pub(crate) drag_last_x:   f64,
	pub(crate) drag_last_y:   f64,
	drag_last_dx:             f64,
	drag_last_dy:             f64,
	drag_last_mods:           u32,
	drag_last_button:         u32,
	drag_last_clicks:         u32,
	pub(crate) drag_grab_x:   f64,
	pub(crate) drag_grab_y:   f64,
	drag_source_key:          String,
	drag_source_item:         String,
	drag_update_name:         Option<u32>,
	drag_end_name:            Option<u32>,
	pending_signals:          Vec<PendingSignal>,
	divider:                  Option<DividerDrag>,
	split:                    Option<SplitDrag>,
	/// Current `CUR_*` cursor code.
	pub cursor:               u32,
	/// Whether the host requested closure.
	pub closed:               bool,
	/// Active selection spanning two editable fields, if any.
	pub range:                Option<edit::CrossFieldRange>,
	/// Active kernel-owned selection over non-field text.
	pub static_selection:     Option<StaticSelection>,
	/// Whether primary-pointer capture currently extends `static_selection`.
	static_select_capture:    bool,
	static_select_x:          f64,
	static_select_y:          f64,
	static_select_moved:      bool,
	/// Field node ids, parallel to `ed`.
	pub ed_node:              Vec<u32>,
	/// Editing states parallel to `ed_node`.
	pub ed:                   Vec<EditState>,
	/// Whether an edit or caret move requested a reveal on the next solve.
	///
	/// Wheel scrolling never sets it, so users can scroll away from the
	/// caret; the next keystroke or caret move pulls the view back.
	pub follow_caret_pending: bool,
}

/// Creates empty dispatch state.
pub const fn dstate_new() -> DState {
	DState {
		fs:                    focus::fst_new(),
		hover:                 Vec::new(),
		pressed:               slir::NONE,
		drag_source:           slir::NONE,
		drop_target:           slir::NONE,
		drag_x:                0.0,
		drag_y:                0.0,
		drag_active:           false,
		suppress_activate:     false,
		drag_last_x:           0.0,
		drag_last_y:           0.0,
		drag_last_dx:          0.0,
		drag_last_dy:          0.0,
		drag_last_mods:        0,
		drag_last_button:      0,
		drag_last_clicks:      0,
		drag_grab_x:           0.0,
		drag_grab_y:           0.0,
		drag_source_key:       String::new(),
		drag_source_item:      String::new(),
		drag_update_name:      None,
		drag_end_name:         None,
		pending_signals:       Vec::new(),
		divider:               None,
		split:                 None,
		cursor:                CUR_DEFAULT,
		closed:                false,
		range:                 None,
		static_selection:      None,
		static_select_capture: false,
		static_select_x:       0.0,
		static_select_y:       0.0,
		static_select_moved:   false,
		ed_node:               Vec::new(),
		ed:                    Vec::new(),
		follow_caret_pending:  false,
	}
}

/// Reports whether a synthetic node was pruned at a solve boundary.
pub fn vanished(d: &Doc, st: &St, node: u32) -> bool {
	node != slir::NONE
		&& i32::from_ne_bytes(node.to_ne_bytes())
			>= i32::try_from(d.node_kind.len()).expect("too many document nodes")
		&& !list::is_split_sash(&st.lists, node)
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
	state_changed |= validate_range(d, st, ds);

	for index in (0..ds.hover.len()).rev() {
		if vanished(d, st, ds.hover[index]) {
			ds.hover.swap_remove(index);
		}
	}
	for index in (0..ds.ed_node.len()).rev() {
		if vanished(d, st, ds.ed_node[index]) {
			ds.ed_node.swap_remove(index);
			ds.ed.swap_remove(index);
		}
	}
	state_changed
}

const fn signal_attr(trigger: u32) -> Option<u32> {
	match trigger {
		TR_ACTIVATE => Some(slir::A_ACT),
		TR_CHANGE => Some(slir::A_FIELD),
		TR_SUBMIT => Some(slir::A_SUBMIT),
		TR_PRESS => Some(slir::A_PRESS),
		TR_CONTEXT => Some(slir::A_CONTEXT),
		TR_DBLCLICK => Some(slir::A_DBLCLICK),
		TR_DRAG_START => Some(slir::A_DRAG),
		TR_DROP => Some(slir::A_DROP),
		TR_RESIZE => Some(slir::A_RESIZE),
		TR_POINTER_MOVE => Some(slir::A_POINTER_MOVE),
		TR_POINTER_UP => Some(slir::A_POINTER_UP),
		TR_DRAG_UPDATE => Some(slir::A_DRAG_UPDATE),
		TR_DRAG_END => Some(slir::A_DRAG_END),
		TR_CANCEL => Some(slir::A_CANCEL),
		_ => None,
	}
}

fn node_has_signal_channel(d: &Doc, base: u32, attr: u32) -> bool {
	if slir::base_attr(d, base, attr) >= 0 {
		return true;
	}
	d.patch_node.iter().enumerate().any(|(patch, &owner)| {
		if owner != base {
			return false;
		}
		let start = usize::try_from(d.patch_attr_off[patch]).expect("negative patch attr offset");
		let end = usize::try_from(d.patch_attr_off[patch].wrapping_add(d.patch_attr_len[patch]))
			.expect("negative patch attr end");
		d.wattr_id[start..end].contains(&attr)
	})
}

/// Finds the signal for `node` and one of the `TR_*` trigger constants.
pub fn sig_of(d: &Doc, st: &St, node: u32, trigger: u32) -> i32 {
	if !style::attached(d, st, node) {
		return -1;
	}
	let base = list::base(&st.lists, d, node);
	let active_name = signal_attr(trigger).and_then(|attr| {
		let value = crate::value::decode_active(d, st.theme_index, style::attr_ix(d, st, node, attr));
		(value.tag == slir::T_STR).then_some(value.h)
	});
	let uses_channel =
		signal_attr(trigger).is_some_and(|attr| node_has_signal_channel(d, base, attr));
	d.sign_name
		.iter()
		.enumerate()
		.find(|(index, name)| {
			d.sign_node[*index] == base
				&& d.sign_trigger[*index] == trigger
				&& (!uses_channel || active_name == Some(**name))
		})
		.map_or(-1, |(index, _)| i32::try_from(index).expect("too many signals"))
}

fn emit_signal(d: &Doc, st: &St, eff: &mut Effects, signal_index: usize, node: u32, text: &str) {
	eff.sig_name.push(d.sign_name[signal_index]);
	eff.sig_text.push(text.to_owned());
	eff.sig_runs.push(String::new());
	eff.sig_item.push(list::item_key(&st.lists, d, node));
	eff.sig_meta.push(SigMeta {
		x:           -1.0,
		y:           -1.0,
		dx:          0.0,
		dy:          0.0,
		drag_dx:     0.0,
		drag_dy:     0.0,
		mods:        0,
		button:      0,
		clicks:      0,
		key:         scene::key_of(d, &st.lists, node),
		hit_key:     String::new(),
		pressed_key: String::new(),
		src_key:     String::new(),
		src_item:    String::new(),
		cancelled:   false,
		dropped:     false,
	});
}

fn signal_name_of(d: &Doc, st: &St, node: u32, trigger: u32) -> Option<u32> {
	usize::try_from(sig_of(d, st, node, trigger))
		.ok()
		.map(|index| d.sign_name[index])
}

const fn remember_drag_event(ds: &mut DState, ev: &Event, dx: f64, dy: f64) {
	ds.drag_last_x = ev.x;
	ds.drag_last_y = ev.y;
	ds.drag_last_dx = dx;
	ds.drag_last_dy = dy;
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
		hit_key: String::new(),
		pressed_key: String::new(),
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
	effects.sig_runs.push(String::new());
	effects.sig_item.push(ds.drag_source_item.clone());
	effects.sig_meta.push(drag_meta(ds, cancelled, dropped));
	effects.repaint = true;
}

fn emit_drag_update(effects: &mut Effects, ds: &DState) {
	push_cached_drag_signal(effects, ds, ds.drag_update_name, false, false);
}

fn emit_drag_end(effects: &mut Effects, ds: &DState, cancelled: bool, dropped: bool) {
	if ds.drag_source != slir::NONE {
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
		runs: String::new(),
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
		effects.sig_runs.push(pending.runs);
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
	eff.scrolls
		.push(ScrollChange { key: scene::key_of(d, &st.lists, node), axis, off });
	true
}

/// Clamps one scene entry's scroll offset to its selected content extent.
#[allow(
	clippy::manual_clamp,
	reason = "f64::clamp propagates NaN; min/max preserve kernel semantics"
)]
pub fn clamp_scroll_axis(sc: &Scene, ix: i32, axis: u32, off: f64) -> f64 {
	let ix = usize::try_from(ix).expect("negative scene index");
	let entry = &sc.entries[ix];
	let (viewport, content) = if axis == 1 {
		(if entry.is_row { entry.h } else { entry.w }, entry.content_cross)
	} else {
		(if entry.is_row { entry.w } else { entry.h }, entry.content_main)
	};
	0.0_f64.max(off.min((content - viewport).max(0.0)))
}

/// Clamps a scene entry's main-axis scroll offset.
pub fn clamp_scroll(sc: &Scene, ix: i32, off: f64) -> f64 {
	clamp_scroll_axis(sc, ix, 0, off)
}
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
		sc.entries[usize::try_from(scene_index).expect("negative scene index")].flags & required != 0
	}) else {
		return;
	};
	let index = usize::try_from(scene_index).expect("negative scene index");
	let entry = &sc.entries[index];
	let node = entry.node;
	let current = style::scroll_get_axis(st, node, axis);
	let next = clamp_scroll_axis(sc, scene_index, axis, current + delta);
	record_scroll(d, st, node, axis, next, eff);
}

/// Routes main-axis navigation keys to a focused scroll node.
///
/// Arrows step 40u, or 200u with Shift held (fast scroll). `PageUp`,
/// `PageDown`, Home, and End are handled by [`page_scroll_key`] against the
/// nearest scroll ancestor instead. `false` leaves the key available to
/// activation, editing, or focus-ring navigation.
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
	let entry = &sc.entries[index];
	if entry.flags & slir::F_SCROLL == 0 {
		return false;
	}

	let current = style::scroll_get(st, node);
	let arrow = if mods & M_SHIFT != 0 { 200.0 } else { 40.0 };
	let next = match key {
		"ArrowLeft" if entry.is_row => current - arrow,
		"ArrowRight" if entry.is_row => current + arrow,
		"ArrowUp" if !entry.is_row => current - arrow,
		"ArrowDown" if !entry.is_row => current + arrow,
		_ => return false,
	};
	record_scroll(d, st, node, 0, clamp_scroll(sc, scene_index, next), eff);
	true
}

/// Routes page-navigation keys to the nearest scroll container of the focus.
///
/// `PageUp` and `PageDown` move by exactly one viewport extent; Home and End
/// jump to the content edges. The target is the nearest scroll-container
/// ancestor of `node` (including itself), or the primary root scroller — the
/// first scroll container in materialized authored order — when `node` is
/// [`slir::NONE`]. Returns `false` when no scroll container is found, leaving
/// the key available to activation bubbling.
pub fn page_scroll_key(
	d: &Doc,
	st: &mut St,
	sc: &Scene,
	node: u32,
	key: &str,
	eff: &mut Effects,
) -> bool {
	if !matches!(key, "PageUp" | "PageDown" | "Home" | "End") {
		return false;
	}
	let mut target = -1;
	if node == slir::NONE {
		let authored_valid = sc.authored_order.len() == sc.entries.len()
			&& sc
				.authored_order
				.iter()
				.all(|&index| index < sc.entries.len());
		let mut order = (0..sc.entries.len()).collect::<Vec<usize>>();
		if authored_valid {
			order.copy_from_slice(&sc.authored_order);
		}
		for index in order {
			if sc.entries[index].flags & slir::F_SCROLL != 0 {
				target = i32::try_from(index).expect("scene index exceeds i32");
				break;
			}
		}
	} else {
		let mut scene_index = scene::index_of(sc, node);
		while scene_index >= 0 {
			let index = usize::try_from(scene_index).expect("negative scene index");
			let entry = &sc.entries[index];
			if entry.flags & slir::F_SCROLL != 0 {
				target = scene_index;
				break;
			}
			scene_index = entry.parent_ix;
		}
	}
	if target < 0 {
		return false;
	}

	let index = usize::try_from(target).expect("negative scene index");
	let entry = &sc.entries[index];
	let scroller = entry.node;
	let viewport = if entry.is_row { entry.w } else { entry.h };
	let current = style::scroll_get(st, scroller);
	let next = match key {
		"PageUp" => current - viewport,
		"PageDown" => current + viewport,
		"Home" => 0.0,
		_ => entry.content_main,
	};
	record_scroll(d, st, scroller, 0, clamp_scroll(sc, target, next), eff);
	true
}

/// Finds a field node's editing-state index.
pub fn ed_ix(ds: &DState, node: u32) -> i32 {
	ds.ed_node
		.iter()
		.position(|candidate| *candidate == node)
		.map_or(-1, |index| i32::try_from(index).expect("too many edit states"))
}
/// Clears cross-field selection metadata, retaining each field's local state.
pub fn clear_range(ds: &mut DState) -> bool {
	ds.range.take().is_some()
}
/// Drops a range whose stable endpoint identity no longer resolves.
///
/// A de-windowed list item still resolves through retained list identity and
/// therefore does not invalidate the range.
pub fn validate_range(d: &Doc, st: &St, ds: &mut DState) -> bool {
	let invalid = ds.range.as_ref().is_some_and(|range| {
		scene::node_by_key(d, &st.lists, &range.anchor_key) == slir::NONE
			|| scene::node_by_key(d, &st.lists, &range.head_key) == slir::NONE
	});
	if invalid {
		ds.range = None;
	}
	invalid
}

/// Clears the active static-text selection and its pointer capture.
pub fn clear_static_selection(ds: &mut DState) -> bool {
	ds.static_select_capture = false;
	ds.static_select_moved = false;
	ds.static_selection.take().is_some()
}

/// Drops static selection state when its root or either endpoint is no longer
/// present in the active scene.
pub fn validate_static_selection(d: &Doc, st: &St, sc: &Scene, ds: &mut DState) -> bool {
	let invalid = ds.static_selection.as_ref().is_some_and(|selection| {
		let root = scene::node_by_key(d, &st.lists, &selection.root_key);
		let anchor = scene::node_by_key(d, &st.lists, &selection.anchor.key);
		let focus = scene::node_by_key(d, &st.lists, &selection.focus.key);
		let root_index = scene::index_of(sc, root);
		let anchor_index = scene::index_of(sc, anchor);
		let focus_index = scene::index_of(sc, focus);
		if root_index < 0 || anchor_index < 0 || focus_index < 0 {
			return true;
		}
		let root_index = usize::try_from(root_index).expect("negative root scene index");
		let anchor_index = usize::try_from(anchor_index).expect("negative anchor scene index");
		let focus_index = usize::try_from(focus_index).expect("negative focus scene index");
		(sc.entries[root_index].flags
			| sc.entries[anchor_index].flags
			| sc.entries[focus_index].flags)
			& slir::F_INERT
			!= 0 || sc.entries[root_index].flags & slir::F_SELECT == 0
			|| !static_text_entry(&sc.entries[anchor_index])
			|| !static_text_entry(&sc.entries[focus_index])
			|| !scene_descends_from(sc, anchor_index, root_index)
			|| !scene_descends_from(sc, focus_index, root_index)
	});
	invalid && clear_static_selection(ds)
}
fn request_range_edit(ds: &DState, effects: &mut Effects, kind: u32, text: &str) -> bool {
	let Some(range) = ds.range.as_ref() else {
		return false;
	};
	effects.range_edit = Some(RangeEdit {
		kind,
		anchor: RangeEndpoint { key: range.anchor_key.clone(), offset: range.anchor_offset },
		head: RangeEndpoint { key: range.head_key.clone(), offset: range.head_offset },
		text: text.to_owned(),
	});
	true
}

fn scene_order(sc: &Scene, node: u32) -> Option<u32> {
	sc.entries
		.iter()
		.filter(|entry| entry.node == node)
		.map(|entry| entry.authored_order)
		.min()
}

/// Records a cross-field range from one stable anchor key to the currently
/// materialized head field and projects ordinary local endpoint selections.
pub fn set_range(
	d: &Doc,
	st: &St,
	sc: &Scene,
	ds: &mut DState,
	anchor_key: &str,
	anchor_offset: i32,
	head_node: u32,
	head_offset: i32,
) -> bool {
	let anchor_node = scene::node_by_key(d, &st.lists, anchor_key);
	if anchor_node == slir::NONE || anchor_node == head_node {
		return false;
	}
	let anchor_key = scene::key_of(d, &st.lists, anchor_node);
	let head_key = scene::key_of(d, &st.lists, head_node);
	if anchor_key.is_empty() || head_key.is_empty() {
		return false;
	}
	let (Some(anchor_order), Some(head_order)) =
		(scene_order(sc, anchor_node), scene_order(sc, head_node))
	else {
		return false;
	};
	let anchor_index = ed_ix(ds, anchor_node);
	let head_index = ed_ix(ds, head_node);
	if anchor_index < 0 || head_index < 0 {
		return false;
	}
	let anchor_index = usize::try_from(anchor_index).expect("negative edit index");
	let head_index = usize::try_from(head_index).expect("negative edit index");
	let anchor_end = crate::rt::str_len(&ds.ed[anchor_index].text);
	let head_end = crate::rt::str_len(&ds.ed[head_index].text);
	let anchor_offset = anchor_offset.clamp(0, anchor_end);
	let head_offset = head_offset.clamp(0, head_end);
	let anchor_caret = if anchor_order < head_order {
		anchor_end
	} else {
		0
	};
	let head_anchor = if anchor_order < head_order {
		0
	} else {
		head_end
	};
	edit::set_selection(&mut ds.ed[anchor_index], anchor_caret, anchor_offset);
	edit::set_selection(&mut ds.ed[head_index], head_offset, head_anchor);
	ds.range = Some(edit::CrossFieldRange { anchor_key, anchor_offset, head_key, head_offset });
	true
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

/// Clears focus and cancels any uncommitted IME composition without dropping
/// the field's committed buffer, selection, or undo history.
pub fn clear_focus(d: &Doc, st: &mut St, ds: &mut DState) -> bool {
	let focused = ds.fs.focus;
	if focused != slir::NONE {
		let edit_index = ed_ix(ds, focused);
		if edit_index >= 0 {
			let index = usize::try_from(edit_index).expect("negative edit index");
			edit::composition_end(&mut ds.ed[index], "");
			style::set_node_state(d, st, focused, "composing", false);
		}
	}
	let range_changed = clear_range(ds);
	let focus_changed = focus::set_focus(d, st, &mut ds.fs, slir::NONE, false);
	range_changed || focus_changed
}
pub(crate) fn sync_bound_text_param(d: &Doc, st: &mut St, node: u32, text: &str) -> bool {
	let signal_index = sig_of(d, st, node, TR_CHANGE);
	if signal_index < 0 {
		return false;
	}
	let signal_index = usize::try_from(signal_index).expect("negative signal index");
	let signal_name = slir::str_at(d, d.sign_name[signal_index]);
	let Some(param) = d.parm_name.iter().enumerate().find_map(|(index, name)| {
		(d.parm_type[index] == slir::PARAM_TEXT && slir::str_at(d, *name) == signal_name)
			.then_some(index)
	}) else {
		return false;
	};
	if st.pv_str[param] == text {
		return false;
	}
	text.clone_into(&mut st.pv_str[param]);
	true
}

/// Resets the edit buffers of fields synced to text parameter `param` after a
/// host parameter write.
///
/// A field whose editor is mid-composition keeps kernel priority and is left
/// untouched. Reset buffers keep their undo history: the replaced text becomes
/// one undo step and the caret collapses at the end of the new value. Returns
/// whether any buffer changed.
pub(crate) fn reset_synced_edits(
	d: &Doc,
	st: &mut St,
	ds: &mut DState,
	param: usize,
	text: &str,
) -> bool {
	let param_name = slir::str_at(d, d.parm_name[param]);
	let mut changed = false;
	let range_nodes = ds.range.as_ref().map(|range| {
		(
			scene::node_by_key(d, &st.lists, &range.anchor_key),
			scene::node_by_key(d, &st.lists, &range.head_key),
		)
	});
	let mut endpoint_changed = false;
	for index in 0..ds.ed_node.len() {
		let node = ds.ed_node[index];
		if ds.ed[index].composing {
			continue;
		}
		let signal_index = sig_of(d, st, node, TR_CHANGE);
		if signal_index < 0 {
			continue;
		}
		let signal_index = usize::try_from(signal_index).expect("negative signal index");
		if slir::str_at(d, d.sign_name[signal_index]) != param_name {
			continue;
		}
		if ds.ed[index].text == text {
			continue;
		}
		edit::history_barrier(&mut ds.ed[index]);
		edit::begin_mutation(&mut ds.ed[index], edit::MUT_NONE);
		let revision = ds.ed[index].revision;
		let old_end = crate::rt::str_len(&ds.ed[index].text);
		edit::splice(&mut ds.ed[index], 0, old_end, text);
		ds.ed[index].revision = revision;
		let end = crate::rt::str_len(text);
		ds.ed[index].caret = end;
		ds.ed[index].anchor = end;
		edit::history_barrier(&mut ds.ed[index]);
		style::field_set(st, node, text);
		// The full content is now published; drop the whole-text splice so
		// the next local edit cannot merge against a stale lineage.
		ds.ed[index].reset_measure_delta();
		endpoint_changed |= range_nodes.is_some_and(|(anchor, head)| node == anchor || node == head);
		changed = true;
	}
	if endpoint_changed {
		clear_range(ds);
	}
	changed
}

/// Queues one host-driven field Change for the next [`take_pending_signals`]
/// call.
pub(crate) fn queue_field_change(d: &Doc, st: &St, ds: &mut DState, node: u32, text: &str) {
	let signal_index = sig_of(d, st, node, TR_CHANGE);
	if signal_index < 0 {
		return;
	}
	let signal_index = usize::try_from(signal_index).expect("negative signal index");
	let runs = {
		let edit_index = ed_ix(ds, node);
		if edit_index < 0 {
			String::new()
		} else {
			let edit = &ds.ed[usize::try_from(edit_index).expect("negative edit index")];
			edit::spans_json(edit.revision, &edit.spans)
		}
	};
	let mut effects = effects_new();
	emit_signal(d, st, &mut effects, signal_index, node, text);
	effects.sig_runs[0] = runs;
	ds.pending_signals.push(PendingSignal {
		name: effects.sig_name[0],
		text: effects.sig_text.swap_remove(0),
		runs: effects.sig_runs.swap_remove(0),
		item: effects.sig_item.swap_remove(0),
		meta: effects.sig_meta.swap_remove(0),
	});
}

/// Writes an edit's display text back into style and emits a change signal.
///
/// The pending measure transition travels with the content: contiguous
/// keystrokes publish a splice lineage so layout re-measures only the hard
/// lines they touched; composition activity and non-contiguous groups
/// publish a full transition.
pub fn sync_field(
	d: &Doc,
	st: &mut St,
	ds: &mut DState,
	eff: &mut Effects,
	ei: i32,
	text_changed: bool,
) {
	let index = usize::try_from(ei).expect("negative edit index");
	let node = ds.ed_node[index];
	let display = edit::display_str(&ds.ed[index]);
	// Every synced edit or caret transition reveals the caret next solve.
	ds.follow_caret_pending = true;
	match ds.ed[index].take_measure_delta() {
		// Pure caret motion: published content is already current, so the
		// lineage must not advance — a following keystroke splices against
		// the same measured revision.
		edit::MeasureDelta::Unchanged => {},
		edit::MeasureDelta::Splice(delta) => style::field_set_spliced(st, node, &display, delta),
		edit::MeasureDelta::Full => style::field_set(st, node, &display),
	}
	eff.repaint = true;
	if !text_changed {
		return;
	}
	let text = edit::text_str(&ds.ed[index]);
	sync_bound_text_param(d, st, node, &text);
	let signal_index = sig_of(d, st, node, 1);
	if signal_index >= 0 {
		let signal_index = usize::try_from(signal_index).expect("negative signal index");
		emit_signal(d, st, eff, signal_index, node, &text);
		*eff.sig_runs.last_mut().expect("signal payload exists") =
			edit::spans_json(ds.ed[index].revision, &ds.ed[index].spans);
	}
}

/// Finds the last resolved style for a node.
pub fn rstyle_ix(st: &St, node: u32) -> i32 {
	st.rs
		.iter()
		.rposition(|resolved| resolved.node == node)
		.map_or(-1, |index| i32::try_from(index).expect("too many resolved styles"))
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
	// A retained edit whose `field=` binder went inactive (committed
	// conditional editor) keeps its state but must not paint a caret or
	// anchor an IME rectangle.
	if sig_of(d, st, node, TR_CHANGE) < 0 {
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

	let caret = edit::display_caret(&ds.ed[edit_index]);
	let text_layout_index = crate::layout::text_layout_ix(lay, node);
	let (line, line_height, line_width, advance) = if text_layout_index >= 0 {
		let text_layout =
			&lay.tls[usize::try_from(text_layout_index).expect("negative text layout index")];
		if text_layout.src_ls.is_empty() {
			(0, sc.entries[scene_index].h, 0.0, 0.0)
		} else {
			// Caret geometry comes from the retained layout: line-local work
			// instead of re-measuring the whole display string.
			let line = line_of(text_layout, caret);
			let index = usize::try_from(line).expect("negative line index");
			let shaper = crate::textm::Shaper { d, cache: &lay.shape_cache };
			(
				line,
				text_layout.line_h,
				text_layout.line_w[index],
				crate::textm::caret_x(shaper, text_layout, index, caret),
			)
		}
	} else {
		let text = edit::display_str(&ds.ed[edit_index]);
		(
			0,
			sc.entries[scene_index].h,
			0.0,
			crate::textm::str_slice_w(d, font, size, tracking, &text, 0, caret),
		)
	};
	let entry = &sc.entries[scene_index];
	let content_width = entry.w - pad_left - pad_right;
	let origin = (content_width - line_width).mul_add(align, entry.x + pad_left);
	eff.focus = node;
	eff.has_caret = true;
	eff.caret_x = origin + advance - ds.ed[edit_index].scroll_x;
	eff.caret_y = f64::from(line).mul_add(line_height, entry.y + pad_top);
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
	text: &str,
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
fn split_snapshot(d: &Doc, st: &St, sc: &Scene, sash: u32) -> Option<SplitDrag> {
	let sash_index = usize::try_from(scene::index_of(sc, sash)).ok()?;
	let sash_entry = sc.entries.get(sash_index)?;
	let parent_index = usize::try_from(sash_entry.parent_ix).ok()?;
	let parent = sc.entries.get(parent_index)?;
	if parent.flags & slir::F_SPLITS == 0 {
		return None;
	}
	let left_key = list::split_sash_left(&st.lists, sash)?;
	let mut panes: Vec<&crate::flatten::SceneNode> = sc
		.entries
		.iter()
		.filter(|entry| {
			entry.parent_ix == sash_entry.parent_ix && !list::is_split_sash(&st.lists, entry.node)
		})
		.collect();
	panes.sort_by(|left, right| {
		let left_pos = if parent.is_row { left.x } else { left.y };
		let right_pos = if parent.is_row { right.x } else { right.y };
		left_pos.total_cmp(&right_pos)
	});
	let keys: Vec<String> = panes
		.iter()
		.map(|entry| scene::key_of(d, &st.lists, entry.node))
		.collect();
	let left = keys.iter().position(|key| key == left_key)?;
	if left + 1 >= panes.len() {
		return None;
	}
	let current: Vec<f64> = panes
		.iter()
		.zip(&keys)
		.map(|(entry, key)| {
			style::split_get(st, key).unwrap_or(if parent.is_row { entry.w } else { entry.h })
		})
		.collect();
	let mut min = Vec::with_capacity(panes.len());
	let mut max = Vec::with_capacity(panes.len());
	for entry in &panes {
		let (pane_min, pane_max) = resolved_axis_bounds(st, entry.node, parent.is_row);
		min.push(pane_min);
		max.push(pane_max);
	}
	Some(SplitDrag {
		sash,
		container: parent.node,
		row: parent.is_row,
		start_pos: 0.0,
		left,
		keys,
		start: current.clone(),
		current,
		min,
		max,
		moved: false,
	})
}

fn split_apply_delta(split: &mut SplitDrag, delta: f64) {
	split.current.clone_from(&split.start);
	if !delta.is_finite() || delta == 0.0 {
		split.moved = false;
		return;
	}
	let left = split.left;
	let count = split.current.len();
	let (grow_left, requested) = (delta > 0.0, delta.abs());
	let capacity_left: f64 = if grow_left {
		(0..=left)
			.map(|index| (split.max[index] - split.current[index]).max(0.0))
			.sum()
	} else {
		(0..=left)
			.map(|index| (split.current[index] - split.min[index]).max(0.0))
			.sum()
	};
	let capacity_right: f64 = if grow_left {
		(left + 1..count)
			.map(|index| (split.current[index] - split.min[index]).max(0.0))
			.sum()
	} else {
		(left + 1..count)
			.map(|index| (split.max[index] - split.current[index]).max(0.0))
			.sum()
	};
	let amount = requested.min(capacity_left).min(capacity_right);
	let mut remaining = amount;
	for index in (0..=left).rev() {
		let capacity = if grow_left {
			split.max[index] - split.current[index]
		} else {
			split.current[index] - split.min[index]
		}
		.max(0.0);
		let applied = remaining.min(capacity);
		split.current[index] += if grow_left { applied } else { -applied };
		remaining -= applied;
	}
	remaining = amount;
	for index in left + 1..count {
		let capacity = if grow_left {
			split.current[index] - split.min[index]
		} else {
			split.max[index] - split.current[index]
		}
		.max(0.0);
		let applied = remaining.min(capacity);
		split.current[index] += if grow_left { -applied } else { applied };
		remaining -= applied;
	}
	split.moved = amount > 0.0;
}

fn split_store(st: &mut St, split: &SplitDrag) -> bool {
	let mut changed = false;
	for (key, extent) in split.keys.iter().zip(&split.current) {
		changed |= style::split_set(st, key, *extent);
	}
	changed
}

fn arm_split(d: &Doc, st: &St, sc: &Scene, sash: u32, x: f64, y: f64) -> Option<SplitDrag> {
	let mut split = split_snapshot(d, st, sc, sash)?;
	split.start_pos = if split.row { x } else { y };
	Some(split)
}

fn move_split(st: &mut St, split: &mut SplitDrag, x: f64, y: f64) -> bool {
	let position = if split.row { x } else { y };
	split_apply_delta(split, position - split.start_pos);
	split_store(st, split)
}

fn split_signal(d: &Doc, st: &St, eff: &mut Effects, split: &SplitDrag) {
	if deliver_trigger(
		d,
		st,
		eff,
		split.container,
		TR_RESIZE,
		&crate::value::fmt3(split.current[split.left]),
	) && let Some(meta) = eff.sig_meta.last_mut()
	{
		meta.key = scene::key_of(d, &st.lists, split.sash);
	}
}

fn path_split_sash(st: &St, sc: &Scene, path: &[i32]) -> u32 {
	path
		.iter()
		.rev()
		.find_map(|&scene_index| {
			let node = sc.entries[usize::try_from(scene_index).expect("negative scene index")].node;
			list::is_split_sash(&st.lists, node).then_some(node)
		})
		.unwrap_or(slir::NONE)
}

fn split_even(d: &Doc, st: &mut St, sc: &Scene, sash: u32) -> Option<SplitDrag> {
	let mut split = split_snapshot(d, st, sc, sash)?;
	let left = split.left;
	let total = split.current[left] + split.current[left + 1];
	let lower = split.min[left].max(total - split.max[left + 1]);
	let upper = split.max[left].min(total - split.min[left + 1]);
	let next = (total / 2.0).clamp(lower, upper);
	split.current[left] = next;
	split.current[left + 1] = total - next;
	split.start.clone_from(&split.current);
	split.moved = split_store(st, &split);
	Some(split)
}

fn split_key(
	d: &Doc,
	st: &mut St,
	sc: &Scene,
	sash: u32,
	key: &str,
	mods: u32,
	eff: &mut Effects,
) -> bool {
	let Some(mut split) = split_snapshot(d, st, sc, sash) else {
		return false;
	};
	let direction = match (split.row, key) {
		(true, "ArrowLeft") | (false, "ArrowUp") => -1.0,
		(true, "ArrowRight") | (false, "ArrowDown") => 1.0,
		_ => return false,
	};
	let step = if mods & M_SHIFT != 0 { 1.0 } else { 8.0 };
	split_apply_delta(&mut split, direction * step);
	eff.repaint |= split_store(st, &split);
	split_signal(d, st, eff, &split);
	true
}

/// Delivers an activation signal for `node`, reporting whether one is declared.
pub fn deliver_activate(d: &Doc, st: &St, eff: &mut Effects, node: u32) -> bool {
	deliver_trigger(d, st, eff, node, TR_ACTIVATE, "")
}

/// Reports whether `node` is disabled.
pub fn disabled(_d: &Doc, st: &St, node: u32) -> bool {
	style::node_disabled(st, node)
}

#[derive(Clone, Copy)]
struct DividerSnapshot {
	row:        bool,
	current:    f64,
	min:        f64,
	max:        f64,
	budget_max: f64,
}

fn divider_scene_siblings(d: &Doc, st: &St, sc: &Scene, node: u32) -> Option<(usize, usize, bool)> {
	let divider = usize::try_from(scene::index_of(sc, node)).ok()?;
	let entry = sc.entries.get(divider)?;
	if entry.kind != slir::K_DIVIDER {
		return None;
	}
	let parent_i32 = entry.parent_ix;
	let parent = usize::try_from(parent_i32).ok()?;
	let parent_entry = sc.entries.get(parent)?;
	if !matches!(parent_entry.kind, slir::K_ROW | slir::K_COL) {
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
	for (index, candidate) in sc.entries.iter().enumerate() {
		if candidate.parent_ix != parent_i32 {
			continue;
		}
		let candidate_base = list::base(&st.lists, d, candidate.node);
		if candidate_base == previous_base {
			previous = Some(index);
		} else if candidate_base == next_base {
			next = Some(index);
		}
	}
	Some((previous?, next?, parent_entry.is_row))
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
	let previous_entry = &sc.entries[previous];
	let next_entry = &sc.entries[next];
	let previous_extent = if row {
		previous_entry.w
	} else {
		previous_entry.h
	};
	let next_extent = if row { next_entry.w } else { next_entry.h };
	let (min, authored_max) = resolved_axis_bounds(st, previous_entry.node, row);
	let (next_min, _) = resolved_axis_bounds(st, next_entry.node, row);
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
	Some(style::divider_clamp(requested, bounds.min, bounds.max, bounds.budget_max))
}

fn path_divider_node(d: &Doc, st: &St, sc: &Scene, path: &[i32]) -> u32 {
	path
		.iter()
		.rev()
		.find_map(|&scene_index| {
			let index = usize::try_from(scene_index).expect("negative scene index");
			let entry = &sc.entries[index];
			let node = entry.node;
			(entry.kind == slir::K_DIVIDER
				&& !list::is_split_sash(&st.lists, node)
				&& !disabled(d, st, node))
			.then_some(node)
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
	let next =
		style::divider_clamp(bounds.current + direction * step, bounds.min, bounds.max, bounds.max);
	eff.repaint |= style::divider_set(st, node, next);
	deliver_trigger(d, st, eff, node, TR_RESIZE, &crate::value::fmt3(next));
	true
}

fn path_trigger_node(d: &Doc, st: &St, sc: &Scene, path: &[i32], trigger: u32) -> u32 {
	path
		.iter()
		.rev()
		.find_map(|&scene_index| {
			let node = sc.entries[usize::try_from(scene_index).expect("negative scene index")].node;
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
		if sc.entries[index].node == root {
			return true;
		}
		scene_index = sc.entries[index].parent_ix;
	}
	false
}

fn drag_target_of(d: &Doc, st: &St, sc: &Scene, path: &[i32], source: u32) -> u32 {
	path
		.iter()
		.rev()
		.find_map(|&scene_index| {
			let index = usize::try_from(scene_index).expect("negative scene index");
			let node = sc.entries[index].node;
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
		eff.repaint |= list::is_split_sash(&st.lists, ds.pressed);
		eff.repaint |= style::set_node_state(d, st, ds.pressed, "pressed", false);
		ds.pressed = slir::NONE;
	}
}

fn cancel_pointer(d: &Doc, st: &mut St, ds: &mut DState, eff: &mut Effects) {
	clear_pressed(d, st, ds, eff);
	clear_drag(d, st, ds, eff);
	ds.divider = None;
	ds.split = None;
}

/// Cancels a pointer gesture whose armed drag source is absent or disabled.
///
/// Called after loading a freshly solved scene so dynamic visibility changes
/// cannot leave capture or Drop styling alive until another host event.
pub(crate) fn cancel_invalid_drag(d: &Doc, st: &mut St, sc: &Scene, ds: &mut DState) -> bool {
	let invalid_drag = ds.drag_source != slir::NONE
		&& (scene::index_of(sc, ds.drag_source) < 0 || disabled(d, st, ds.drag_source));
	let invalid_split = ds
		.split
		.as_ref()
		.is_some_and(|split| scene::index_of(sc, split.sash) < 0);
	if !invalid_drag && !invalid_split {
		return false;
	}
	if invalid_drag {
		queue_drag_end(ds);
	}
	let mut effects = effects_new();
	cancel_pointer(d, st, ds, &mut effects);
	true
}

/// Reports whether a concise comma-separated `keys=` value contains `key`.
pub fn key_list_has(keys: &str, key: &str) -> bool {
	!keys.is_empty()
		&& keys.split(',').any(|candidate| {
			candidate
				.rsplit_once(':')
				.is_none_or(|(_, signal)| signal.is_empty())
				&& candidate == key
		})
}

/// Resolves `key` in a typed `keys=Key:signal,...` map.
pub fn key_map_signal<'a>(keys: &'a str, key: &str) -> Option<&'a str> {
	keys.split(',').find_map(|entry| {
		let (candidate, signal) = entry.rsplit_once(':')?;
		(candidate == key && !signal.is_empty()).then_some(signal)
	})
}

/// Delivers `key` through one node's own `keys=` map: the typed
/// `Key:signal` form wins over a concise `Key` entry activating the node.
/// The emitted signal records the key and the event's modifier bitset.
pub fn deliver_key_map(
	d: &Doc,
	st: &St,
	node: u32,
	key: &str,
	mods: u32,
	eff: &mut Effects,
) -> bool {
	if disabled(d, st, node) {
		return false;
	}
	let keys = style::attr_str_ref(d, st, node, slir::A_KEYS);
	if let Some(signal) = key_map_signal(&keys, key) {
		let base = list::base(&st.lists, d, node);
		if let Some(signal_index) = d.sign_name.iter().enumerate().find_map(|(index, name)| {
			(d.sign_node[index] == base
				&& d.sign_trigger[index] == TR_KEY_ACTIVATE
				&& slir::str_at(d, *name) == signal)
				.then_some(index)
		}) {
			emit_signal(d, st, eff, signal_index, node, "");
			let meta = eff.sig_meta.last_mut().expect("activation has metadata");
			key.clone_into(&mut meta.pressed_key);
			meta.mods = mods;
			return true;
		}
	} else if key_list_has(&keys, key) && deliver_activate(d, st, eff, node) {
		let meta = eff.sig_meta.last_mut().expect("activation has metadata");
		key.clone_into(&mut meta.pressed_key);
		meta.mods = mods;
		return true;
	}
	false
}

/// Delivers to the nearest enabled `keys=` node on the focused scene path.
pub fn activate_key_path(
	d: &Doc,
	st: &St,
	sc: &Scene,
	focused: u32,
	key: &str,
	mods: u32,
	eff: &mut Effects,
) -> bool {
	let mut scene_index = scene::index_of(sc, focused);
	while scene_index >= 0 {
		let index = usize::try_from(scene_index).expect("negative scene index");
		if deliver_key_map(d, st, sc.entries[index].node, key, mods, eff) {
			return true;
		}
		scene_index = sc.entries[index].parent_ix;
	}
	false
}

/// Delivers to the focused node's own `keys=` map, skipping ancestors: a
/// field-authored binding preempts kernel editing for the bound key.
pub fn activate_key_own(
	d: &Doc,
	st: &St,
	sc: &Scene,
	focused: u32,
	key: &str,
	mods: u32,
	eff: &mut Effects,
) -> bool {
	let scene_index = scene::index_of(sc, focused);
	if scene_index < 0 {
		return false;
	}
	let index = usize::try_from(scene_index).expect("negative scene index");
	deliver_key_map(d, st, sc.entries[index].node, key, mods, eff)
}

/// Reports whether an editable node accepts multiple lines.
pub fn multiline(d: &Doc, st: &St, node: u32) -> bool {
	let base = list::base(&st.lists, d, node);
	d.node_flags[usize::try_from(base).expect("node id does not fit usize")] & slir::F_MULTILINE != 0
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

/// Geometry needed to map a document-space hit into one editable field.
pub(crate) struct FieldHit<'a> {
	pub(crate) d:    &'a Doc,
	pub(crate) st:   &'a St,
	pub(crate) lay:  &'a Lay,
	pub(crate) sc:   &'a Scene,
	pub(crate) node: u32,
}

/// Maps a document-space field hit to the nearest source caret.
fn field_caret_at(hit: &FieldHit<'_>, scroll_x: f64, x: f64, y: f64) -> i32 {
	let st = hit.st;
	let lay = hit.lay;
	let sc = hit.sc;
	let node = hit.node;
	let scene_index = scene::index_of(sc, node);
	let text_layout_index = crate::layout::text_layout_ix(lay, node);
	if scene_index < 0 || text_layout_index < 0 {
		return 0;
	}
	let scene_index = usize::try_from(scene_index).expect("negative scene index");
	let text_layout =
		&lay.tls[usize::try_from(text_layout_index).expect("negative text layout index")];
	if text_layout.src_ls.is_empty() {
		return 0;
	}
	let (pad_top, pad_left, pad_right, align) =
		if let Some(resolved) = st.rs.iter().rev().find(|resolved| resolved.node == node) {
			let align = match resolved.talign {
				1 => 0.5,
				2 => 1.0,
				_ => 0.0,
			};
			(resolved.pad_t, resolved.pad_l, resolved.pad_r, align)
		} else {
			(0.0, 0.0, 0.0, 0.0)
		};
	let line = (((y - sc.entries[scene_index].y - pad_top) / text_layout.line_h).floor() as i32)
		.clamp(0, i32::try_from(text_layout.src_ls.len() - 1).expect("too many text lines"));
	let line_index = usize::try_from(line).expect("negative line index");
	let content_width = sc.entries[scene_index].w - pad_left - pad_right;
	let origin = (content_width - text_layout.line_w[line_index])
		.mul_add(align, sc.entries[scene_index].x + pad_left)
		- scroll_x;
	textm::caret_for_visual_x(
		textm::Shaper { d: hit.d, cache: &hit.lay.shape_cache },
		text_layout,
		line_index,
		x - origin,
	)
}

/// Applies secondary-click selection semantics to one editable field.
pub(crate) fn place_context_caret(
	hit: &FieldHit<'_>,
	edit_state: &mut edit::EditState,
	x: f64,
	y: f64,
) -> bool {
	let hit = field_caret_at(hit, edit_state.scroll_x, x, y);
	if hit >= edit::sel_lo(edit_state) && hit <= edit::sel_hi(edit_state) {
		return false;
	}
	edit::history_barrier(edit_state);
	edit_state.caret = hit;
	edit_state.anchor = hit;
	edit_state.goal_x = -1.0;
	true
}
fn scene_descends_from(sc: &Scene, mut index: usize, root: usize) -> bool {
	loop {
		if index == root {
			return true;
		}
		let parent = sc.entries[index].parent_ix;
		if parent < 0 {
			return false;
		}
		index = usize::try_from(parent).expect("negative scene parent");
	}
}

const fn static_text_entry(entry: &crate::flatten::SceneNode) -> bool {
	matches!(entry.kind, slir::K_TEXT | slir::K_SPAN | slir::K_PARA) && !entry.editable
}

fn paragraph_placement(lay: &Lay, node: u32) -> Option<(usize, usize)> {
	lay.p_node
		.iter()
		.zip(&lay.p_para)
		.enumerate()
		.rev()
		.find_map(|(placement, (&candidate, &paragraph))| {
			(candidate == node && paragraph >= 0)
				.then(|| (placement, usize::try_from(paragraph).expect("negative paragraph index")))
		})
}

fn paragraph_source_bounds(lay: &Lay, paragraph: usize) -> Option<(i32, i32)> {
	let lo = *lay.para_src_off.get(paragraph)?;
	let len = *lay.para_src_len.get(paragraph)?;
	Some((lo, lo.wrapping_add(len)))
}

fn shaped_caret_at(shaped: &textm::ShapedLine, base: i32, end: i32, goal: f64) -> i32 {
	for cluster in &shaped.clusters {
		let midpoint = f64::midpoint(cluster.x0, cluster.x1);
		if goal < midpoint {
			return base
				.wrapping_add(if cluster.rtl {
					cluster.end
				} else {
					cluster.start
				})
				.min(end);
		}
		if goal <= cluster.x1 {
			return base
				.wrapping_add(if cluster.rtl {
					cluster.start
				} else {
					cluster.end
				})
				.min(end);
		}
	}
	shaped.clusters.last().map_or(base, |cluster| {
		base
			.wrapping_add(if cluster.rtl {
				cluster.start
			} else {
				cluster.end
			})
			.min(end)
	})
}

fn paragraph_caret_at(st: &St, lay: &Lay, sc: &Scene, node: u32, x: f64, y: f64) -> i32 {
	let scene_index = scene::index_of(sc, node);
	let Some((_, paragraph)) = paragraph_placement(lay, node) else {
		return 0;
	};
	if scene_index < 0 {
		return 0;
	}
	let scene_index = usize::try_from(scene_index).expect("negative scene index");
	let entry = &sc.entries[scene_index];
	let Some((source_base, _)) = paragraph_source_bounds(lay, paragraph) else {
		return 0;
	};
	let (pad_top, pad_left, pad_right, align) =
		if let Some(resolved) = st.rs.iter().rev().find(|resolved| resolved.node == node) {
			let align = match resolved.talign {
				1 => 0.5,
				2 => 1.0,
				_ => 0.0,
			};
			(resolved.pad_t, resolved.pad_l, resolved.pad_r, align)
		} else {
			(0.0, 0.0, 0.0, 0.0)
		};
	let first_line = lay.para_line_off[paragraph];
	let line_end = first_line.wrapping_add(lay.para_line_len[paragraph]);
	let mut line_top = entry.y + pad_top;
	let mut chosen_line = first_line;
	for line in first_line..line_end {
		chosen_line = line;
		let line_index = usize::try_from(line).expect("negative paragraph line");
		if y < line_top + lay.pl_h[line_index] || line + 1 == line_end {
			break;
		}
		line_top += lay.pl_h[line_index];
	}
	let line_index = usize::try_from(chosen_line).expect("negative paragraph line");
	let content_width = entry.w - pad_left - pad_right;
	let line_origin = (content_width - lay.pl_w[line_index]).mul_add(align, entry.x + pad_left);
	let first_segment = lay.pl_seg_off[line_index];
	let segment_end = first_segment.wrapping_add(lay.pl_seg_len[line_index]);
	let line_source_start = lay.pl_src_a[line_index];
	let mut chosen = None;
	let mut best = f64::INFINITY;
	for segment in first_segment..segment_end {
		let segment_index = usize::try_from(segment).expect("negative paragraph segment");
		let left = line_origin + lay.seg_x[segment_index];
		let right = left + lay.seg_w[segment_index];
		let distance = if x < left {
			left - x
		} else if x > right {
			x - right
		} else {
			0.0
		};
		if distance < best {
			best = distance;
			chosen = Some((segment_index, left));
		}
	}
	let Some((segment, left)) = chosen else {
		return 0;
	};
	if segment == usize::try_from(first_segment).expect("negative paragraph segment") && x <= left {
		return line_source_start.wrapping_sub(source_base);
	}
	if segment == usize::try_from(segment_end.wrapping_sub(1)).expect("negative paragraph segment")
		&& x >= left + lay.seg_w[segment]
	{
		return lay.pl_src_b[line_index].wrapping_sub(source_base);
	}
	let caret =
		shaped_caret_at(&lay.seg_shaped[segment], lay.seg_a[segment], lay.seg_b[segment], x - left);
	lay.seg_src_a[segment]
		.wrapping_add(caret.wrapping_sub(lay.seg_a[segment]))
		.min(lay.seg_src_b[segment])
		.wrapping_sub(source_base)
}

fn static_caret_at(d: &Doc, st: &St, lay: &Lay, sc: &Scene, node: u32, x: f64, y: f64) -> i32 {
	let scene_index = scene::index_of(sc, node);
	if scene_index < 0 {
		return 0;
	}
	let entry = &sc.entries[usize::try_from(scene_index).expect("negative scene index")];
	if entry.kind == slir::K_PARA {
		paragraph_caret_at(st, lay, sc, node, x, y)
	} else {
		field_caret_at(&FieldHit { d, st, lay, sc, node }, 0.0, x, y)
	}
}

fn static_position_nearest(
	d: &Doc,
	st: &St,
	lay: &Lay,
	sc: &Scene,
	root: u32,
	x: f64,
	y: f64,
) -> Option<(u32, i32)> {
	let root_index = usize::try_from(scene::index_of(sc, root)).ok()?;
	let mut chosen = None;
	let mut best = f64::INFINITY;
	for (index, entry) in sc.entries.iter().enumerate() {
		if !static_text_entry(entry)
			|| entry.flags & slir::F_INERT != 0
			|| !scene_descends_from(sc, index, root_index)
		{
			continue;
		}
		let dx = if x < entry.x {
			entry.x - x
		} else if x > entry.x + entry.w {
			x - entry.x - entry.w
		} else {
			0.0
		};
		let dy = if y < entry.y {
			entry.y - y
		} else if y > entry.y + entry.h {
			y - entry.y - entry.h
		} else {
			0.0
		};
		let distance = dy.mul_add(dy, dx * dx);
		if distance < best {
			best = distance;
			chosen = Some(entry.node);
		}
	}
	let node = chosen?;
	let offset = static_caret_at(d, st, lay, sc, node, x, y);
	Some((node, offset))
}

fn static_position_on_path(
	d: &Doc,
	st: &St,
	lay: &Lay,
	sc: &Scene,
	path: &[i32],
	x: f64,
	y: f64,
) -> Option<(u32, u32, i32)> {
	let root_path = path.iter().rposition(|&scene_index| {
		sc.entries[usize::try_from(scene_index).expect("negative scene index")].flags & slir::F_SELECT
			!= 0
	})?;
	if path[root_path..].iter().any(|&scene_index| {
		let entry = &sc.entries[usize::try_from(scene_index).expect("negative scene index")];
		entry.flags & slir::F_FOCUSABLE != 0
			|| entry.editable
			|| sig_of(d, st, entry.node, TR_DRAG_START) >= 0
	}) {
		return None;
	}
	let text = path[root_path..].iter().rev().find_map(|&scene_index| {
		let entry = &sc.entries[usize::try_from(scene_index).expect("negative scene index")];
		(static_text_entry(entry) && entry.flags & slir::F_INERT == 0).then_some(entry.node)
	})?;
	let root = sc.entries[usize::try_from(path[root_path]).expect("negative scene index")].node;
	Some((root, text, static_caret_at(d, st, lay, sc, text, x, y)))
}
fn update_static_focus(d: &Doc, st: &St, ds: &mut DState, node: u32, offset: i32) -> bool {
	let Some(selection) = ds.static_selection.as_mut() else {
		return false;
	};
	if scene::node_by_key(d, &st.lists, &selection.focus.key) == node {
		if selection.focus.offset == offset {
			return false;
		}
		selection.focus.offset = offset;
	} else {
		selection.focus = StaticEndpoint { key: scene::key_of(d, &st.lists, node), offset };
	}
	true
}

fn static_selection_limits(
	d: &Doc,
	st: &St,
	sc: &Scene,
	selection: &StaticSelection,
	node: u32,
	text_end: i32,
) -> Option<(i32, i32)> {
	let anchor_node = scene::node_by_key(d, &st.lists, &selection.anchor.key);
	let focus_node = scene::node_by_key(d, &st.lists, &selection.focus.key);
	let anchor_index = usize::try_from(scene::index_of(sc, anchor_node)).ok()?;
	let focus_index = usize::try_from(scene::index_of(sc, focus_node)).ok()?;
	let node_index = usize::try_from(scene::index_of(sc, node)).ok()?;
	let anchor_order = sc.entries[anchor_index].authored_order;
	let focus_order = sc.entries[focus_index].authored_order;
	let node_order = sc.entries[node_index].authored_order;
	if anchor_node == focus_node {
		return (node == anchor_node).then(|| {
			(
				selection
					.anchor
					.offset
					.min(selection.focus.offset)
					.clamp(0, text_end),
				selection
					.anchor
					.offset
					.max(selection.focus.offset)
					.clamp(0, text_end),
			)
		});
	}
	let anchor_before = anchor_order < focus_order;
	let (first_node, first_offset, first_order, last_node, last_offset, last_order) =
		if anchor_before {
			(
				anchor_node,
				selection.anchor.offset,
				anchor_order,
				focus_node,
				selection.focus.offset,
				focus_order,
			)
		} else {
			(
				focus_node,
				selection.focus.offset,
				focus_order,
				anchor_node,
				selection.anchor.offset,
				anchor_order,
			)
		};
	if node == first_node {
		Some((first_offset.clamp(0, text_end), text_end))
	} else if node == last_node {
		Some((0, last_offset.clamp(0, text_end)))
	} else if node_order > first_order && node_order < last_order {
		Some((0, text_end))
	} else {
		None
	}
}

fn append_visual_piece(
	out: &mut String,
	last_baseline: &mut Option<f64>,
	baseline: f64,
	text: &str,
) {
	if text.is_empty() {
		return;
	}
	if last_baseline.is_some_and(|previous| (previous - baseline).abs() > 0.5) && !out.is_empty() {
		out.push('\n');
	}
	out.push_str(text);
	*last_baseline = Some(baseline);
}

fn codepoint_slice(text: &str, start: i32, end: i32) -> String {
	let start = usize::try_from(start.max(0)).expect("nonnegative codepoint offset");
	let len = usize::try_from(end.max(0))
		.expect("nonnegative codepoint offset")
		.saturating_sub(start);
	text.chars().skip(start).take(len).collect()
}

fn static_selection_text(
	d: &Doc,
	st: &St,
	lay: &Lay,
	sc: &Scene,
	selection: &StaticSelection,
) -> String {
	let root = scene::node_by_key(d, &st.lists, &selection.root_key);
	let Ok(root_index) = usize::try_from(scene::index_of(sc, root)) else {
		return String::new();
	};
	let mut out = String::new();
	let mut last_baseline = None;
	for &scene_index in &sc.authored_order {
		let entry = &sc.entries[scene_index];
		if !static_text_entry(entry)
			|| entry.flags & slir::F_INERT != 0
			|| !scene_descends_from(sc, scene_index, root_index)
		{
			continue;
		}
		if entry.kind == slir::K_PARA {
			let Some((_, paragraph)) = paragraph_placement(lay, entry.node) else {
				continue;
			};
			let Some((source_base, source_end)) = paragraph_source_bounds(lay, paragraph) else {
				continue;
			};
			let Some((lo, hi)) = static_selection_limits(
				d,
				st,
				sc,
				selection,
				entry.node,
				source_end.wrapping_sub(source_base),
			) else {
				continue;
			};
			let lo = source_base.wrapping_add(lo);
			let hi = source_base.wrapping_add(hi);
			let first_line = lay.para_line_off[paragraph];
			let line_end = first_line.wrapping_add(lay.para_line_len[paragraph]);
			let mut line_top = entry.y;
			for line in first_line..line_end {
				let line_index = usize::try_from(line).expect("negative paragraph line");
				let baseline = line_top + lay.pl_asc[line_index];
				let start = lo.max(lay.pl_src_a[line_index]);
				let end = hi.min(lay.pl_src_b[line_index]);
				if end > start {
					let piece = crate::rt::str_from_chars(
						&lay.para_chars[usize::try_from(start).expect("negative paragraph offset")
							..usize::try_from(end).expect("negative paragraph offset")],
					);
					append_visual_piece(&mut out, &mut last_baseline, baseline, &piece);
				}
				line_top += lay.pl_h[line_index];
			}
			continue;
		}
		let text_layout_index = crate::layout::text_layout_ix(lay, entry.node);
		if text_layout_index < 0 {
			continue;
		}
		let text_layout =
			&lay.tls[usize::try_from(text_layout_index).expect("negative text layout index")];
		let text_end = text_layout.src_le.iter().copied().max().unwrap_or(0);
		let Some((lo, hi)) = static_selection_limits(d, st, sc, selection, entry.node, text_end)
		else {
			continue;
		};
		let content = style::content_str(d, st, entry.node);
		let (pad_top, ascent, line_h) = if let Some(resolved) = st
			.rs
			.iter()
			.rev()
			.find(|resolved| resolved.node == entry.node)
		{
			(resolved.pad_t, text_layout.ascent, text_layout.line_h)
		} else {
			(0.0, text_layout.ascent, text_layout.line_h)
		};
		for line in 0..text_layout.src_ls.len() {
			let start = lo.max(text_layout.src_ls[line]);
			let end = hi.min(text_layout.src_le[line]);
			if end <= start {
				continue;
			}
			let piece = codepoint_slice(&content, start, end);
			let baseline = f64::from(i32::try_from(line).expect("too many text lines"))
				.mul_add(line_h, entry.y + pad_top + ascent);
			append_visual_piece(&mut out, &mut last_baseline, baseline, &piece);
		}
	}
	out
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
	emit_signal(d, st, eff, signal_index, node, &edit::text_str(&ds.ed[edit_index]));
	eff.repaint = true;
}

/// Replaces line breaks with spaces for a single-line field.
pub fn single_line_text(text: &str) -> String {
	text
		.chars()
		.map(|character| match character {
			'\n' | '\r' => ' ',
			_ => character,
		})
		.collect()
}

/// Scrolls the focused field and its nearest scroll ancestor to reveal the
/// caret.
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
	let (line, line_height, line_width, advance) = if text_layout_index >= 0 {
		let text_layout =
			&lay.tls[usize::try_from(text_layout_index).expect("negative text layout index")];
		if text_layout.src_ls.is_empty() {
			(0, sc.entries[scene_index].h, 0.0, 0.0)
		} else {
			// Line-local caret geometry from the retained layout; never
			// re-measures the whole display string.
			let line = line_of(text_layout, caret);
			let line_index = usize::try_from(line).expect("negative line index");
			let shaper = crate::textm::Shaper { d, cache: &lay.shape_cache };
			(
				line,
				text_layout.line_h,
				text_layout.line_w[line_index],
				crate::textm::caret_x(shaper, text_layout, line_index, caret),
			)
		}
	} else {
		let text = edit::display_str(&ds.ed[edit_index]);
		(
			0,
			sc.entries[scene_index].h,
			0.0,
			crate::textm::str_slice_w(d, font, size, tracking, &text, 0, caret),
		)
	};

	if !multiline(d, st, node) {
		let old_scroll = ds.ed[edit_index].scroll_x;
		let content_width = sc.entries[scene_index].w - pad_left - pad_right;
		let origin = (content_width - line_width).mul_add(align, pad_left);
		let left = pad_left + 8.0;
		let right = left.max(sc.entries[scene_index].w - pad_right - 8.0);
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

	let top = f64::from(line).mul_add(line_height, sc.entries[scene_index].y + pad_top);
	let bottom = top + line_height;
	let mut parent = sc.entries[scene_index].parent_ix;
	let mut found_scroll_parent = false;
	while parent >= 0 {
		let parent_index = usize::try_from(parent).expect("negative parent index");
		if !found_scroll_parent && sc.entries[parent_index].flags & slir::F_SCROLL != 0 {
			let parent_node = sc.entries[parent_index].node;
			let mut next = style::scroll_get(st, parent_node);
			if top < sc.entries[parent_index].y {
				next -= sc.entries[parent_index].y - top;
			} else if bottom > sc.entries[parent_index].y + sc.entries[parent_index].h {
				next += bottom - (sc.entries[parent_index].y + sc.entries[parent_index].h);
			}
			record_scroll(d, st, parent_node, 0, clamp_scroll(sc, parent, next), eff);
			found_scroll_parent = true;
		}
		parent = sc.entries[parent_index].parent_ix;
	}
}

/// Re-runs caret following against a freshly solved text layout and scene.
///
/// Only acts while an edit or caret move is pending: wheel scrolling never
/// requests a follow, so users can scroll away from the caret.
/// Returns `true` when one more settle solve is required.
pub fn follow_caret_fresh(d: &Doc, st: &mut St, lay: &Lay, sc: &Scene, ds: &mut DState) -> bool {
	if !ds.follow_caret_pending || ds.fs.focus == slir::NONE {
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

/// Routes the focused field's editing keys, running before activation-key
/// bubbling.
///
/// Returns `false` for unrecognized keys and for boundary commands that
/// changed nothing (Backspace at the start, an arrow clamped at an edge) so
/// the key still reaches `keys=` maps; commands that mutate text, move the
/// caret, or emit an effect (submit) stay consumed.
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
	let before = (ds.ed[index].caret, ds.ed[index].anchor);
	let selecting = mods & M_SHIFT != 0;
	let alt = mods & M_ALT != 0;
	let control = mods & M_CTRL != 0;
	let command = mods & (M_META | M_CTRL) != 0;
	let is_multiline = multiline(d, st, node);
	let text_layout_index = crate::layout::text_layout_ix(lay, node);
	let text_layout = (text_layout_index >= 0)
		.then(|| &lay.tls[usize::try_from(text_layout_index).expect("negative text layout index")]);
	let mut text_changed = false;
	let mut effect_emitted = false;
	let mut refresh = true;

	match key {
		"Enter" => {
			let submits = sig_of(d, st, node, 2) >= 0;
			if is_multiline && (!submits || selecting || alt) {
				text_changed = edit::insert(&mut ds.ed[index], "\n");
			} else if submits && (!is_multiline || mods == 0) {
				emit_submit(d, st, ds, eff, edit_index);
				effect_emitted = true;
			} else {
				refresh = false;
			}
		},
		"Backspace" => {
			text_changed = if control || alt {
				edit::word_back(&mut ds.ed[index])
			} else {
				edit::backspace(&mut ds.ed[index])
			};
		},
		"Delete" => {
			text_changed = if control || alt {
				edit::word_forward(&mut ds.ed[index])
			} else {
				edit::del(&mut ds.ed[index])
			};
		},
		"w" | "W" if command => {
			edit::history_barrier(&mut ds.ed[index]);
			text_changed = edit::word_back(&mut ds.ed[index]);
		},
		"z" | "Z" if command => {
			text_changed = if selecting {
				edit::redo(&mut ds.ed[index])
			} else {
				edit::undo(&mut ds.ed[index])
			};
		},
		"k" | "K" if control => {
			if let Some(text_layout) = text_layout {
				text_changed = edit::kill_end(&mut ds.ed[index], text_layout);
			}
		},
		"u" | "U" if control => {
			if let Some(text_layout) = text_layout {
				text_changed = edit::kill_start(&mut ds.ed[index], text_layout);
			}
		},
		"ArrowLeft" => {
			if command {
				edit::home(&mut ds.ed[index], selecting);
			} else if alt {
				edit::move_caret(&mut ds.ed[index], -1, selecting, true);
			} else if let Some(text_layout) = text_layout {
				edit::visual_step(
					textm::Shaper { d, cache: &lay.shape_cache },
					&mut ds.ed[index],
					text_layout,
					-1,
					selecting,
				);
			} else {
				edit::move_caret(&mut ds.ed[index], -1, selecting, false);
			}
		},
		"ArrowRight" => {
			if command {
				edit::end(&mut ds.ed[index], selecting);
			} else if alt {
				edit::move_caret(&mut ds.ed[index], 1, selecting, true);
			} else if let Some(text_layout) = text_layout {
				edit::visual_step(
					textm::Shaper { d, cache: &lay.shape_cache },
					&mut ds.ed[index],
					text_layout,
					1,
					selecting,
				);
			} else {
				edit::move_caret(&mut ds.ed[index], 1, selecting, false);
			}
		},
		"ArrowUp" | "ArrowDown" if is_multiline => {
			if let Some(text_layout) = text_layout {
				let delta = if key == "ArrowUp" { -1 } else { 1 };
				edit::visual_move(
					textm::Shaper { d, cache: &lay.shape_cache },
					&mut ds.ed[index],
					text_layout,
					delta,
					selecting,
				);
			}
		},
		"Home" => {
			if is_multiline && !command {
				if let Some(text_layout) = text_layout {
					edit::visual_home(
						textm::Shaper { d, cache: &lay.shape_cache },
						&mut ds.ed[index],
						text_layout,
						selecting,
					);
				}
			} else {
				edit::home(&mut ds.ed[index], selecting);
			}
		},
		"End" => {
			if is_multiline && !command {
				if let Some(text_layout) = text_layout {
					edit::visual_end(
						textm::Shaper { d, cache: &lay.shape_cache },
						&mut ds.ed[index],
						text_layout,
						selecting,
					);
				}
			} else {
				edit::end(&mut ds.ed[index], selecting);
			}
		},
		"a" | "A" if command => edit::select_all(&mut ds.ed[index]),
		_ => return false,
	}

	if !refresh {
		return false;
	}
	let after = (ds.ed[index].caret, ds.ed[index].anchor);
	if selecting {
		if let Some(range) = ds.range.as_mut()
			&& scene::node_by_key(d, &st.lists, &range.head_key) == node
		{
			range.head_offset = ds.ed[index].caret;
		}
	} else if (text_changed || before != after) && clear_range(ds) {
		eff.repaint = true;
	}
	sync_field(d, st, ds, eff, edit_index, text_changed);
	follow_caret(d, st, lay, sc, ds, edit_index, eff);
	// A boundary command that changed nothing — Backspace at the start,
	// Delete at the end, an arrow clamped at an edge — bubbles through
	// `keys=` so hosts can bind block-level behaviors (merge, split,
	// cross-field navigation). Submit already emitted its effect and must
	// not double-dispatch.
	if !effect_emitted && !text_changed && before == (ds.ed[index].caret, ds.ed[index].anchor) {
		return false;
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
	if validate_range(d, st, ds) {
		effects.repaint = true;
	}
	if validate_static_selection(d, st, sc, ds) {
		effects.repaint = true;
	}
	let (pointer_dx, pointer_dy) = if ev.etype == E_POINTER_MOVE
		&& ds.drag_source != slir::NONE
		&& ev.dx == 0.0
		&& ev.dy == 0.0
		&& (ev.x != ds.drag_last_x || ev.y != ds.drag_last_y)
	{
		(ev.x - ds.drag_last_x, ev.y - ds.drag_last_y)
	} else {
		(ev.dx, ev.dy)
	};
	let mut path = Vec::new();
	if matches!(ev.etype, E_POINTER_MOVE | E_POINTER_DOWN | E_POINTER_UP | E_WHEEL) {
		crate::hit::hit_test(sc, ev.x, ev.y, &mut path);
	}
	let hit_key = path.last().map_or_else(String::new, |&scene_index| {
		scene::key_of(
			d,
			&st.lists,
			sc.entries[usize::try_from(scene_index).expect("negative scene index")].node,
		)
	});

	match ev.etype {
		E_POINTER_MOVE => {
			if ds.drag_source != slir::NONE {
				remember_drag_event(ds, ev, pointer_dx, pointer_dy);
			}
			if ds.static_select_capture {
				let dx = ev.x - ds.static_select_x;
				let dy = ev.y - ds.static_select_y;
				ds.static_select_moved |= dy.mul_add(dy, dx * dx) > 16.0;
				let root = ds
					.static_selection
					.as_ref()
					.map_or(slir::NONE, |selection| {
						scene::node_by_key(d, &st.lists, &selection.root_key)
					});
				if root == slir::NONE {
					effects.repaint |= clear_static_selection(ds);
				} else if let Some((node, offset)) =
					static_position_nearest(d, st, lay, sc, root, ev.x, ev.y)
					&& update_static_focus(d, st, ds, node, offset)
				{
					effects.repaint = true;
				}
			}
			let mut signal_path = Vec::new();
			routed_pointer_path(sc, ds, &path, &mut signal_path);
			let pointer_target = path_trigger_node(d, st, sc, &signal_path, TR_POINTER_MOVE);
			if pointer_target != slir::NONE {
				let emitted = deliver_trigger(d, st, &mut effects, pointer_target, TR_POINTER_MOVE, "");
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
					sc.entries[usize::try_from(scene_index).expect("negative scene index")].node
						== hovered
				});
				if !still_hovered && style::set_node_state(d, st, hovered, "hover", false) {
					changed = true;
				}
			}

			let mut next_hover = Vec::with_capacity(path.len());
			for &scene_index in &path {
				let node = sc.entries[usize::try_from(scene_index).expect("negative scene index")].node;
				if !ds.hover.contains(&node) && style::set_node_state(d, st, node, "hover", true) {
					changed = true;
				}
				next_hover.push(node);
			}
			if ds.hover != next_hover
				&& ds
					.hover
					.iter()
					.chain(&next_hover)
					.any(|node| list::is_split_sash(&st.lists, *node))
			{
				changed = true;
			}
			ds.hover = next_hover;
			effects.repaint |= changed;

			let cancel_split = ds
				.split
				.as_ref()
				.is_some_and(|split| ds.pressed != split.sash || scene::index_of(sc, split.sash) < 0);
			if cancel_split {
				ds.split = None;
			} else if let Some(split) = ds.split.as_mut()
				&& move_split(st, split, ev.x, ev.y)
			{
				effects.repaint = true;
			}

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
					&crate::value::fmt3(divider.current_extent),
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
				if dy.mul_add(dy, dx * dx) > 16.0 {
					ds.drag_active = true;
					ds.suppress_activate = true;
					effects.repaint |= style::set_node_state(d, st, ds.drag_source, "dragging", true);
					if deliver_trigger(d, st, &mut effects, ds.drag_source, TR_DRAG_START, "")
						&& let Some(meta) = effects.sig_meta.last_mut()
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
				let node = sc.entries[index].node;
				if list::is_split_sash(&st.lists, node) {
					ds.cursor = if sc.entries[index].is_row {
						CUR_COL_RESIZE
					} else {
						CUR_ROW_RESIZE
					};
					break;
				}
				if sc.entries[index].kind == slir::K_DIVIDER
					&& !disabled(d, st, node)
					&& let Some((_, _, row)) = divider_scene_siblings(d, st, sc, node)
				{
					ds.cursor = if row { CUR_COL_RESIZE } else { CUR_ROW_RESIZE };
					break;
				}
				if static_position_on_path(d, st, lay, sc, &path, ev.x, ev.y).is_some() {
					ds.cursor = CUR_TEXT;
					break;
				}
				if sig_of(d, st, node, TR_CHANGE) >= 0 && !disabled(d, st, node) {
					ds.cursor = CUR_TEXT;
					break;
				}
				if sc.entries[index].flags & slir::F_FOCUSABLE != 0 && !disabled(d, st, node) {
					ds.cursor = CUR_POINTER;
					break;
				}
			}
		},
		E_POINTER_DOWN => {
			let previous_focus = ds.fs.focus;
			if ev.mods & M_SHIFT == 0 && clear_range(ds) {
				effects.repaint = true;
			}
			if clear_static_selection(ds) {
				effects.repaint = true;
			}
			// A fresh down cancels stale capture without disturbing keyboard
			// focus, except that a secondary field hit applies pointer focus.
			if ds.drag_active {
				remember_drag_event(ds, ev, ev.dx, ev.dy);
				emit_drag_end(&mut effects, ds, true, false);
			}
			cancel_pointer(d, st, ds, &mut effects);
			let divider_target = if ev.button == 0 {
				path_divider_node(d, st, sc, &path)
			} else {
				slir::NONE
			};
			let split_target = if ev.button == 0 {
				path_split_sash(st, sc, &path)
			} else {
				slir::NONE
			};
			if ev.button == 2 {
				let field_target = path.iter().rev().find_map(|&scene_index| {
					let index = usize::try_from(scene_index).expect("negative scene index");
					let node = sc.entries[index].node;
					(sc.entries[index].flags & slir::F_FOCUSABLE != 0
						&& sig_of(d, st, node, TR_CHANGE) >= 0
						&& !disabled(d, st, node))
					.then_some(node)
				});
				if let Some(field) = field_target {
					ensure_edit(d, st, ds, field);
					let edit_index =
						usize::try_from(ed_ix(ds, field)).expect("field edit state is missing");
					effects.repaint |= place_context_caret(
						&FieldHit { d, st, lay, sc, node: field },
						&mut ds.ed[edit_index],
						ev.x,
						ev.y,
					);
					if field != previous_focus && clear_range(ds) {
						effects.repaint = true;
					}
					effects.repaint |= focus::set_focus(d, st, &mut ds.fs, field, false);
				}
				let target = path_trigger_node(d, st, sc, &path, TR_CONTEXT);
				if target != slir::NONE {
					deliver_trigger(d, st, &mut effects, target, TR_CONTEXT, "");
				}
			} else if ev.button == 0 {
				// Press is observable before capture/focus side effects.
				let press_target = path_trigger_node(d, st, sc, &path, TR_PRESS);
				if press_target != slir::NONE {
					deliver_trigger(d, st, &mut effects, press_target, TR_PRESS, "");
				}
				if ev.clicks >= 2 {
					if split_target != slir::NONE
						&& let Some(split) = split_even(d, st, sc, split_target)
					{
						ds.suppress_activate = true;
						effects.repaint = true;
						split_signal(d, st, &mut effects, &split);
					}
					if divider_target != slir::NONE {
						ds.suppress_activate = true;
						effects.repaint |= style::divider_clear(st, divider_target);
						deliver_trigger(d, st, &mut effects, divider_target, TR_DBLCLICK, "");
					}
					let target = path_trigger_node(d, st, sc, &path, TR_DBLCLICK);
					if target != slir::NONE && target != divider_target {
						ds.suppress_activate = true;
						deliver_trigger(d, st, &mut effects, target, TR_DBLCLICK, "");
					}
				}
				ds.drag_source = if divider_target == slir::NONE && split_target == slir::NONE {
					path_trigger_node(d, st, sc, &path, TR_DRAG_START)
				} else {
					slir::NONE
				};
				ds.drag_x = ev.x;
				ds.drag_y = ev.y;
				if ds.drag_source != slir::NONE {
					remember_drag_event(ds, ev, ev.dx, ev.dy);
					ds.drag_source_key = scene::key_of(d, &st.lists, ds.drag_source);
					ds.drag_source_item = list::item_key(&st.lists, d, ds.drag_source);
					ds.drag_update_name = signal_name_of(d, st, ds.drag_source, TR_DRAG_UPDATE);
					ds.drag_end_name = signal_name_of(d, st, ds.drag_source, TR_DRAG_END);
					let source_index = scene::index_of(sc, ds.drag_source);
					if source_index >= 0 {
						let source_index = usize::try_from(source_index).expect("negative scene index");
						ds.drag_grab_x = ev.x - sc.entries[source_index].x;
						ds.drag_grab_y = ev.y - sc.entries[source_index].y;
					}
				}
				if let Some((root, text, offset)) =
					static_position_on_path(d, st, lay, sc, &path, ev.x, ev.y)
				{
					let endpoint = StaticEndpoint { key: scene::key_of(d, &st.lists, text), offset };
					ds.static_selection = Some(StaticSelection {
						root_key: scene::key_of(d, &st.lists, root),
						anchor:   endpoint.clone(),
						focus:    endpoint,
					});
					ds.static_select_capture = true;
					ds.static_select_x = ev.x;
					ds.static_select_y = ev.y;
					ds.static_select_moved = false;
				}

				// Capture the nearest focusable node, or the raw target. Pointer
				// focus deliberately carries no keyboard focus ring.
				let focus_target = path
					.iter()
					.rev()
					.copied()
					.find(|&scene_index| {
						sc.entries[usize::try_from(scene_index).expect("negative scene index")].flags
							& slir::F_FOCUSABLE
							!= 0
					})
					.map_or(slir::NONE, |scene_index| {
						sc.entries[usize::try_from(scene_index).expect("negative scene index")].node
					});
				let pressed = if focus_target == slir::NONE {
					path.last().map_or(slir::NONE, |&scene_index| {
						sc.entries[usize::try_from(scene_index).expect("negative scene index")].node
					})
				} else {
					focus_target
				};
				if pressed != slir::NONE {
					ds.pressed = pressed;
					effects.repaint |= style::set_node_state(d, st, pressed, "pressed", true);
					effects.repaint |= list::is_split_sash(&st.lists, pressed);
				}
				if focus_target != slir::NONE && sig_of(d, st, focus_target, TR_CHANGE) >= 0 {
					ensure_edit(d, st, ds, focus_target);
					let edit_index =
						usize::try_from(ed_ix(ds, focus_target)).expect("field edit state is missing");
					let hit = field_caret_at(
						&FieldHit { d, st, lay, sc, node: focus_target },
						ds.ed[edit_index].scroll_x,
						ev.x,
						ev.y,
					);
					let selecting = ev.mods & M_SHIFT != 0;
					let prior_range = ds.range.clone();
					let source = if selecting && previous_focus != focus_target {
						prior_range
							.as_ref()
							.filter(|range| {
								scene::node_by_key(d, &st.lists, &range.head_key) == previous_focus
							})
							.map(|range| (range.anchor_key.clone(), range.anchor_offset))
							.or_else(|| {
								let source_index = ed_ix(ds, previous_focus);
								(source_index >= 0).then(|| {
									let source_index =
										usize::try_from(source_index).expect("negative edit index");
									(scene::key_of(d, &st.lists, previous_focus), ds.ed[source_index].anchor)
								})
							})
					} else if selecting {
						prior_range
							.as_ref()
							.filter(|range| {
								scene::node_by_key(d, &st.lists, &range.head_key) == focus_target
							})
							.map(|range| (range.anchor_key.clone(), range.anchor_offset))
					} else {
						None
					};
					let ranged = source.is_some_and(|(anchor_key, anchor_offset)| {
						set_range(d, st, sc, ds, &anchor_key, anchor_offset, focus_target, hit)
					});
					if ranged {
						effects.repaint = true;
					} else {
						let anchor = if selecting && previous_focus == focus_target {
							ds.ed[edit_index].anchor
						} else {
							hit
						};
						effects.repaint |= edit::set_selection(&mut ds.ed[edit_index], hit, anchor);
					}
				}
				if focus_target != previous_focus
					&& (focus_target == slir::NONE || sig_of(d, st, focus_target, TR_CHANGE) < 0)
					&& clear_range(ds)
				{
					effects.repaint = true;
				}
				effects.repaint |= focus::set_focus(d, st, &mut ds.fs, focus_target, false);
				if split_target != slir::NONE
					&& ev.clicks < 2
					&& let Some(split) = arm_split(d, st, sc, split_target, ev.x, ev.y)
				{
					ds.split = Some(split);
					ds.suppress_activate = true;
				}
				if divider_target != slir::NONE
					&& ev.clicks < 2
					&& let Some(divider) = arm_divider(d, st, sc, divider_target, ev.x, ev.y)
				{
					ds.divider = Some(divider);
					ds.suppress_activate = true;
				}
			}
		},
		E_POINTER_UP => {
			if ds.drag_source != slir::NONE {
				remember_drag_event(ds, ev, ev.dx, ev.dy);
			}
			let mut signal_path = Vec::new();
			routed_pointer_path(sc, ds, &path, &mut signal_path);
			let pointer_target = path_trigger_node(d, st, sc, &signal_path, TR_POINTER_UP);
			if pointer_target != slir::NONE {
				let emitted = deliver_trigger(d, st, &mut effects, pointer_target, TR_POINTER_UP, "");
				if emitted
					&& ds.drag_active
					&& let Some(meta) = effects.sig_meta.last_mut()
				{
					apply_drag_meta(meta, ds, false, false);
				}
			}
			if ev.button == 0 && ds.static_select_capture {
				let dx = ev.x - ds.static_select_x;
				let dy = ev.y - ds.static_select_y;
				ds.static_select_moved |= dy.mul_add(dy, dx * dx) > 16.0;
				if ds.static_select_moved {
					let root = ds
						.static_selection
						.as_ref()
						.map_or(slir::NONE, |selection| {
							scene::node_by_key(d, &st.lists, &selection.root_key)
						});
					if let Some((node, offset)) =
						static_position_nearest(d, st, lay, sc, root, ev.x, ev.y)
					{
						effects.repaint |= update_static_focus(d, st, ds, node, offset);
					}
					ds.static_select_capture = false;
				} else {
					effects.repaint |= clear_static_selection(ds);
				}
			}
			if ev.button == 0 {
				if let Some(mut split) = ds.split.take()
					&& scene::index_of(sc, split.sash) >= 0
				{
					effects.repaint |= move_split(st, &mut split, ev.x, ev.y);
					split_signal(d, st, &mut effects, &split);
					ds.suppress_activate = true;
				}
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
						&crate::value::fmt3(divider.current_extent),
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
					if target != slir::NONE && deliver_trigger(d, st, &mut effects, target, TR_DROP, "")
					{
						dropped = true;
						let meta = effects
							.sig_meta
							.last_mut()
							.expect("delivered drop has metadata");
						meta.src_key.clone_from(&ds.drag_source_key);
						meta.src_item.clone_from(&ds.drag_source_item);
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
						sc.entries[usize::try_from(scene_index).expect("negative scene index")].node
							== pressed
					});
					let scene_index = scene::index_of(sc, pressed);
					if pointer_over && scene_index >= 0 && !disabled(d, st, pressed) {
						let index = usize::try_from(scene_index).expect("negative scene index");
						if sc.entries[index].flags & slir::F_FOCUSABLE != 0 {
							deliver_activate(d, st, &mut effects, pressed);
						}
					}
				}
			}
		},
		E_WHEEL => {
			let (main_delta, cross_delta) = if ev.mods & M_SHIFT != 0 {
				(ev.dx, ev.dy)
			} else {
				(ev.dy, ev.dx)
			};
			wheel_axis(d, st, sc, &path, 0, main_delta, &mut effects);
			wheel_axis(d, st, sc, &path, 1, cross_delta, &mut effects);
		},
		E_KEY_DOWN => {
			let selecting = ev.mods & M_SHIFT != 0;
			let focused = ds.fs.focus;

			// Structural range edits preempt field-local mutation and authored
			// key bindings; the host applies them to its block model.
			let mut handled =
				if ds.range.is_some() && matches!(ev.key.as_str(), "Backspace" | "Delete") {
					let kind = if ev.key == "Backspace" {
						RANGE_EDIT_BACKSPACE
					} else {
						RANGE_EDIT_DELETE
					};
					request_range_edit(ds, &mut effects, kind, "")
				} else if ev.key == "Escape" && ds.drag_source != slir::NONE {
					emit_drag_end(&mut effects, ds, true, false);
					cancel_pointer(d, st, ds, &mut effects);
					true
				} else {
					false
				};
			if !handled && ev.key == "Escape" && ds.static_selection.is_some() {
				handled = true;
				effects.repaint |= clear_static_selection(ds);
			}
			if !handled
				&& ev.key == "Escape"
				&& focused != slir::NONE
				&& ed_ix(ds, focused) >= 0
				&& style::eff_flags(d, st, focused) & slir::F_ESCAPE_BLUR != 0
			{
				handled = true;
				// A `cancel=` binder observes the escape-blur with the field's
				// retained committed buffer before focus clears.
				let edit_index =
					usize::try_from(ed_ix(ds, focused)).expect("focused field has edit state");
				let retained = edit::text_str(&ds.ed[edit_index]);
				deliver_trigger(d, st, &mut effects, focused, TR_CANCEL, &retained);
				effects.repaint |= clear_focus(d, st, ds);
			}
			// A plain non-printable `keys=` binding authored on the focused field
			// itself preempts kernel editing (plain Enter splits; Shift+Enter and
			// ordinary typing still reach the editor). Boundary no-op edit
			// commands fall through to `keys=` bubbling, followed by divider
			// adjustment, scrolling, and focus-ring navigation.
			if !handled
				&& ev.mods == 0
				&& ev.key.chars().count() != 1
				&& focused != slir::NONE
				&& ed_ix(ds, focused) >= 0
			{
				handled = activate_key_own(d, st, sc, focused, &ev.key, ev.mods, &mut effects);
			}
			if !handled && focused != slir::NONE {
				handled = route_edit_key(d, st, lay, sc, ds, focused, &ev.key, ev.mods, &mut effects);
			}
			if !handled && focused != slir::NONE && list::is_split_sash(&st.lists, focused) {
				handled = split_key(d, st, sc, focused, &ev.key, ev.mods, &mut effects);
			}
			if !handled && focused != slir::NONE && ed_ix(ds, focused) < 0 {
				handled = divider_key(d, st, sc, focused, &ev.key, ev.mods, &mut effects);
			}
			if !handled && focused != slir::NONE && ed_ix(ds, focused) < 0 {
				handled = scroll_key(d, st, sc, focused, &ev.key, ev.mods, &mut effects);
			}
			if !handled && (focused == slir::NONE || ed_ix(ds, focused) < 0) {
				handled = page_scroll_key(d, st, sc, focused, &ev.key, &mut effects);
			}
			// Unmodified printable keys stay with a focused editor; everything
			// else — including modified shortcuts such as Cmd+B — may bubble
			// through `keys=` maps.
			let editor_printable = focused != slir::NONE
				&& ed_ix(ds, focused) >= 0
				&& ev.mods == 0
				&& ev.key.chars().count() == 1;
			if !handled && focused != slir::NONE && !editor_printable {
				handled = activate_key_path(d, st, sc, focused, &ev.key, ev.mods, &mut effects);
			}
			// With no focus, or when the focused walk leaves the key
			// unhandled, dispatch falls back to the document root `keys=` map.
			if !handled && !editor_printable {
				let root = sc.entries.first().map_or(slir::NONE, |e| e.node);
				if root != slir::NONE && root != focused {
					handled = activate_key_path(d, st, sc, root, &ev.key, ev.mods, &mut effects);
				}
			}

			if !handled && ev.key == "Tab" {
				if focus::focus_next(d, st, sc, &mut ds.fs, selecting) {
					effects.repaint = true;
					bind_edit_on_focus(d, st, ds);
					if clear_range(ds) {
						effects.repaint = true;
					}
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
					&& sc.entries[usize::try_from(scene_index).expect("negative scene index")].flags
						& slir::F_FOCUSABLE
						!= 0 && deliver_activate(d, st, &mut effects, focused)
				{
					ev.key.clone_into(
						&mut effects
							.sig_meta
							.last_mut()
							.expect("activation has metadata")
							.pressed_key,
					);
				}
			// Without an active editor, arrows walk the focus ring.
			} else if !handled
				&& matches!(ev.key.as_str(), "ArrowRight" | "ArrowDown" | "ArrowLeft" | "ArrowUp")
				&& (focused == slir::NONE || ed_ix(ds, focused) < 0)
			{
				let backwards = matches!(ev.key.as_str(), "ArrowLeft" | "ArrowUp");
				if focus::focus_next(d, st, sc, &mut ds.fs, backwards) {
					effects.repaint = true;
					bind_edit_on_focus(d, st, ds);
					if clear_range(ds) {
						effects.repaint = true;
					}
				}
			}
		},
		E_TEXT | E_PASTE => {
			let kind = if ev.etype == E_PASTE {
				RANGE_EDIT_PASTE
			} else {
				RANGE_EDIT_TEXT
			};
			if request_range_edit(ds, &mut effects, kind, &ev.text) {
				// The host owns replacement across block boundaries.
			} else {
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
		},
		E_COPY => {
			if !request_range_edit(ds, &mut effects, RANGE_EDIT_COPY, "") {
				let edit_index = ed_ix(ds, ds.fs.focus);
				if ds.fs.focus != slir::NONE && edit_index >= 0 {
					let state = &ds.ed[usize::try_from(edit_index).expect("negative edit state index")];
					effects.copy_text =
						Some(codepoint_slice(&state.text, edit::sel_lo(state), edit::sel_hi(state)));
				} else if let Some(selection) = ds.static_selection.as_ref() {
					effects.copy_text = Some(static_selection_text(d, st, lay, sc, selection));
				}
			}
		},
		E_CUT => {
			if !request_range_edit(ds, &mut effects, RANGE_EDIT_CUT, "") {
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
		},
		E_COMPOSITION_START => {
			if !request_range_edit(ds, &mut effects, RANGE_EDIT_COMPOSITION, "") {
				let focused = ds.fs.focus;
				let edit_index = ed_ix(ds, focused);
				if focused != slir::NONE && edit_index >= 0 {
					let index = usize::try_from(edit_index).expect("negative edit index");
					let changed = edit::composition_update(&mut ds.ed[index], "");
					style::set_node_state(d, st, focused, "composing", true);
					sync_field(d, st, ds, &mut effects, edit_index, changed);
				}
			}
		},
		E_COMPOSITION_UPDATE | E_COMPOSITION_END => {
			if !request_range_edit(ds, &mut effects, RANGE_EDIT_COMPOSITION, &ev.text) {
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
						edit::composition_update_clauses(&mut ds.ed[index], text.as_ref(), &ev.clauses)
					} else {
						let changed = edit::composition_end(&mut ds.ed[index], text.as_ref());
						style::set_node_state(d, st, focused, "composing", false);
						changed
					};
					sync_field(d, st, ds, &mut effects, edit_index, changed);
					follow_caret(d, st, lay, sc, ds, edit_index, &mut effects);
				}
			}
		},
		E_BLUR => {
			for &hovered in &ds.hover {
				effects.repaint |= style::set_node_state(d, st, hovered, "hover", false);
			}
			ds.hover.clear();
			emit_drag_end(&mut effects, ds, true, false);
			cancel_pointer(d, st, ds, &mut effects);
			if clear_range(ds) {
				effects.repaint = true;
			}
		},
		E_RESIZE => {
			if ev.dx > 0.0 {
				st.env.vw = ev.dx;
				effects.repaint = true;
			}
			if ev.dy > 0.0 {
				st.env.vh = ev.dy;
				effects.repaint = true;
			}
		},
		E_CLOSE => {
			emit_drag_end(&mut effects, ds, true, false);
			cancel_pointer(d, st, ds, &mut effects);
			ds.closed = true;
			if clear_range(ds) {
				effects.repaint = true;
			}
		},
		// These host-originated events have no kernel-side semantics.
		E_INSPECT | E_ACTIVATE => {},
		_ => {},
	}

	let pointer_origin =
		matches!(ev.etype, E_POINTER_MOVE | E_POINTER_DOWN | E_POINTER_UP | E_WHEEL);
	for meta in &mut effects.sig_meta {
		if pointer_origin {
			meta.x = ev.x;
			meta.y = ev.y;
			meta.dx = pointer_dx;
			meta.dy = pointer_dy;
			meta.hit_key.clone_from(&hit_key);
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

	effects.has_static_selection = ds
		.static_selection
		.as_ref()
		.is_some_and(|selection| selection.anchor != selection.focus);
	effects.cursor = ds.cursor;
	effects.focus = ds.fs.focus;
	caret_effects(d, st, lay, sc, ds, &mut effects);
	effects
}
