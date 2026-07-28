//! Instance state and the kernel's host-facing frame API.
//!
//! Hosts decode SLIR bytes, assign the document to an [`inst_shell`] instance,
//! and call [`inst_init`]. Layout diagnostics accumulate in `st.diag_*` for
//! each solve. Event dispatch, retained-scene hit testing, and the motion
//! overlay all meet here: running animation and in-flight transitions re-solve
//! as the clock advances, while an idle instance solves only when an input
//! marks it dirty. [`Event`] and [`Effects`] are defined by the dispatch module.

use crate::{
    dispatch::{self, DState, Effects, Event},
    dumpjson, edit,
    flatten::{self, Frame, FrameOp},
    focus, hit, layout,
    layout::Lay,
    list, motion,
    motion::MSt,
    scene,
    scene::Scene,
    slir::{self, Doc},
    style::{self, St},
    textm,
};

/// Mutable state for one decoded document and its most recent solve.
#[derive(Clone, Debug)]
pub struct Instance {
    /// Whether the assigned document decoded successfully.
    pub ok: bool,
    /// The decoded document and its static pools.
    pub doc: Doc,
    /// Runtime style, parameter, list, environment, and scroll state.
    pub st: St,
    /// Scratch state and results from the latest layout solve.
    pub lay: Lay,
    /// Retained scene from the latest solve.
    pub sc: Scene,
    /// Interaction state, including focus, hover, presses, and edits.
    pub ds: DState,
    /// Per-patch transition clocks and animation liveness.
    pub ms: MSt,
    /// Whether the host has supplied an environment.
    pub has_env: bool,
    /// Whether changed inputs require another solve.
    pub dirty: bool,
    /// Whether the instance has completed at least one solve.
    pub solved: bool,
    /// Motion clock used for the latest solve.
    pub last_t: f64,
    /// Root patch index produced by the latest solve.
    pub root_pi: i32,
}

/// One stable runtime image slot; inactive slots retain their unified index.
#[derive(Clone, Debug)]
pub(crate) struct RuntimeImage {
    pub(crate) name: String,
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) format: u32,
    pub(crate) data: Vec<u8>,
    pub(crate) generation: u32,
    pub(crate) active: bool,
}

/// Host-facing parameter value.
///
/// `kind` must match the declared parameter type: `0` is text, `1` is a
/// number, `2` is a percentage, `3` is RGBA color, `4` is a boolean encoded
/// as numeric zero or one, and `5` is an enum symbol.
#[derive(Clone, Debug)]
pub struct ParamValue {
    /// Parameter type tag.
    pub kind: u32,
    /// Numeric, percentage, or boolean payload.
    pub num: f64,
    /// Text payload.
    pub s: String,
    /// Packed RGBA color payload.
    pub rgba: u32,
    /// Enum symbol payload.
    pub sym: String,
}

/// Absolute geometry for one document hole.
#[derive(Clone, Debug)]
pub struct HoleRect {
    /// Index in the document's hole table.
    pub hole: u32,
    /// Absolute left coordinate.
    pub x: f64,
    /// Absolute top coordinate.
    pub y: f64,
    /// Width of the hole.
    pub w: f64,
    /// Height of the hole.
    pub h: f64,
    /// Whether the hole's node clips its contents.
    pub clip: bool,
}

/// Positioned glyph for a text frame operation.
#[derive(Clone, Debug)]
pub struct GlyphPos {
    /// Font table index.
    pub font: i32,
    /// Font glyph identifier.
    pub gid: u32,
    /// Glyph origin on the horizontal axis.
    pub x: f64,
    /// Glyph baseline coordinate.
    pub y: f64,
    /// Font size used for the glyph.
    pub size: f64,
}

/// Creates an empty instance to which a host can assign a decoded document.
pub fn inst_shell() -> Instance {
    Instance {
        ok: false,
        doc: slir::doc_new(),
        st: style::st_new(),
        lay: layout::lay_new(),
        sc: scene::scene_new(),
        ds: dispatch::dstate_new(),
        ms: motion::mst_new(),
        has_env: false,
        dirty: true,
        solved: false,
        last_t: 0.0,
        root_pi: -1,
    }
}

/// Initializes style state after a host assigns the decoded document.
pub fn inst_init(i: &mut Instance) {
    i.ok = i.doc.ok;
    if i.ok {
        style::init_params(&i.doc, &mut i.st);
    }
}

/// Hands every natively replayable animation binding to the driver.
///
/// Returns [`motion::lifts`] for the document and marks each returned
/// binding driver-owned: it stops contributing motion overlays and activity,
/// so a document whose bindings all lift no longer re-solves every frame.
/// Drivers that call this MUST replay the returned keyframes themselves
/// (e.g. as CSS animations). Repeated calls are idempotent.
pub fn inst_lift_animations(i: &mut Instance) -> Vec<motion::Lift> {
    let lifted = motion::lifts(&i.doc);
    if lifted.is_empty() {
        return lifted;
    }
    i.ms.lifted = vec![false; i.doc.bind_node.len()];
    i.ms.lift_node = vec![false; i.doc.node_kind.len()];
    i.ms.lift_bg = vec![false; i.doc.node_kind.len()];
    for lift in &lifted {
        i.ms.lifted[lift.binding] = true;
        let node = usize::try_from(lift.node).expect("node index exceeds usize");
        i.ms.lift_node[node] = true;
        i.ms.lift_bg[node] |= lift.stops.iter().any(|stop| stop.bg.is_some());
    }
    i.dirty = true;
    lifted
}

/// Appends a runtime font table, which wins equal compiled matches by index.
#[allow(clippy::too_many_arguments)] // A font table is intrinsically defined by these parallel metrics and slices.
pub fn inst_font_register(
    i: &mut Instance,
    family: &str,
    weight: u32,
    upem: u32,
    ascent: i32,
    descent: i32,
    line_gap: i32,
    default_adv: u32,
    cmap_cp: &[u32],
    cmap_gid: &[u32],
    adv: &[u32],
) -> i32 {
    let family_ref = u32::try_from(i.doc.strs.len()).expect("too many document strings");
    let fallback_class = slir::family_class(family);
    let cmap_off = i32::try_from(i.doc.font_cmap_cp.len()).expect("font cmap is too large");

    i.doc.strs.push(family.to_owned());
    i.doc.font_family.push(family_ref);
    i.doc.font_class.push(fallback_class);
    i.doc.font_weight.push(weight);
    i.doc.font_upem.push(upem);
    i.doc.font_ascent.push(ascent);
    i.doc.font_descent.push(descent);
    i.doc.font_line_gap.push(line_gap);
    i.doc.font_default_adv.push(default_adv);
    i.doc.font_cmap_off.push(cmap_off);
    i.doc
        .font_cmap_len
        .push(i32::try_from(cmap_cp.len()).expect("font cmap is too large"));
    i.doc.font_cmap_cp.extend_from_slice(cmap_cp);
    i.doc.font_cmap_gid.extend_from_slice(cmap_gid);
    i.doc.font_adv.extend_from_slice(adv);
    style::invalidate_font_selection(&mut i.st);
    i.dirty = true;

    i32::try_from(i.doc.font_family.len())
        .expect("too many document fonts")
        .wrapping_sub(1)
}

fn valid_runtime_png(data: &[u8], w: u32, h: u32) -> bool {
    let Ok(mut reader) = png::Decoder::new(std::io::Cursor::new(data)).read_info() else {
        return false;
    };
    if reader.info().width != w || reader.info().height != h {
        return false;
    }
    let Some(output_len) = reader.output_buffer_size() else {
        return false;
    };
    let mut decoded = Vec::new();
    if decoded.try_reserve_exact(output_len).is_err() {
        return false;
    }
    decoded.resize(output_len, 0);
    reader.next_frame(&mut decoded).is_ok()
}

/// Registers or replaces a named runtime image in the unified image table.
///
/// Compiled images occupy the leading indices. A runtime name keeps its
/// appended index through replacement, unregister, and re-registration.
/// Format `0` is PNG and format `1` is straight-alpha sRGB RGBA8. Dimensions
/// must be nonzero; PNG dimensions must match after a full decode, and RGBA8
/// payloads must contain exactly `w*h*4` bytes. Invalid input returns `-1`
/// atomically. Equal active registrations preserve the generation and do not
/// dirty.
pub fn inst_img_register(
    i: &mut Instance,
    name: &str,
    w: u32,
    h: u32,
    format: u32,
    data: &[u8],
) -> i32 {
    if w == 0 || h == 0 {
        return -1;
    }
    match format {
        0 if !valid_runtime_png(data, w, h) => return -1,
        1 => {
            let Some(expected) = usize::try_from(w)
                .ok()
                .and_then(|width| {
                    usize::try_from(h)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .and_then(|pixels| pixels.checked_mul(4))
            else {
                return -1;
            };
            if data.len() != expected {
                return -1;
            }
        }
        0 => {}
        _ => return -1,
    }

    let position =
        i.st.runtime_images
            .iter()
            .position(|image| image.name == name);
    let runtime_index = position.unwrap_or(i.st.runtime_images.len());
    let Some(unified_index) = i.doc.img_src.len().checked_add(runtime_index) else {
        return -1;
    };
    let Ok(unified_index) = i32::try_from(unified_index) else {
        return -1;
    };
    if let Some(position) = position {
        let image = &mut i.st.runtime_images[position];
        let changed = !image.active
            || image.w != w
            || image.h != h
            || image.format != format
            || image.data != data;
        if changed {
            image.w = w;
            image.h = h;
            image.format = format;
            image.data.clear();
            image.data.extend_from_slice(data);
            image.generation = image.generation.wrapping_add(1);
            image.active = true;
            i.dirty = true;
        }
    } else {
        i.st.runtime_images.push(RuntimeImage {
            name: name.to_owned(),
            w,
            h,
            format,
            data: data.to_vec(),
            generation: 1,
            active: true,
        });
        i.dirty = true;
    };
    unified_index
}

/// Unregisters a runtime image while reserving its unified table index.
pub fn inst_img_unregister(i: &mut Instance, name: &str) -> bool {
    let Some(image) =
        i.st.runtime_images
            .iter_mut()
            .find(|image| image.name == name && image.active)
    else {
        return false;
    };
    image.active = false;
    image.generation = image.generation.wrapping_add(1);
    i.dirty = true;
    true
}

/// Returns image dimensions, format, and generation for a unified image index.
///
/// Compiled images have generation zero; inactive and unknown indices return
/// `None`.
pub fn inst_img_info(i: &Instance, img: i32) -> Option<(u32, u32, u32, u32)> {
    let index = usize::try_from(img).ok()?;
    if index < i.doc.img_src.len() {
        return Some((
            *i.doc.img_w.get(index)?,
            *i.doc.img_h.get(index)?,
            *i.doc.img_format.get(index)?,
            0,
        ));
    }
    let image = i.st.runtime_images.get(index - i.doc.img_src.len())?;
    image
        .active
        .then_some((image.w, image.h, image.format, image.generation))
}

/// Returns the immutable payload for a unified image index, or an empty slice.
pub fn inst_img_bytes(i: &Instance, img: i32) -> &[u8] {
    let Ok(index) = usize::try_from(img) else {
        return &[];
    };
    if index < i.doc.img_src.len() {
        return i.doc.img_data.get(index).map(Vec::as_slice).unwrap_or(&[]);
    }
    i.st.runtime_images
        .get(index - i.doc.img_src.len())
        .filter(|image| image.active)
        .map(|image| image.data.as_slice())
        .unwrap_or(&[])
}

/// Sets viewport, client class, and media flags.
///
/// Portrait and landscape derive from `vw < vh`. A non-positive height means
/// unbounded height for a static render invocation.
pub fn inst_set_env(i: &mut Instance, vw: f64, vh: f64, client: u32, dark: bool, coarse: bool) {
    if i.has_env
        && i.st.env.vw == vw
        && i.st.env.vh == vh
        && i.st.env.client == client
        && i.st.env.dark == dark
        && i.st.env.coarse == coarse
    {
        return;
    }
    i.st.env.vw = vw;
    i.st.env.vh = vh;
    i.st.env.client = client;
    i.st.env.dark = dark;
    i.st.env.coarse = coarse;
    i.has_env = true;
    i.dirty = true;
}

/// Selects a compiler-declared theme.
///
/// The empty name restores the authored base. An unknown name returns `false`
/// and leaves the current theme unchanged.
pub fn inst_set_theme(i: &mut Instance, name: &str) -> bool {
    let known = name.is_empty()
        || i.doc.theme_name.iter().any(|&name_ref| {
            let index = usize::try_from(name_ref).expect("theme string index is too large");
            i.doc.strs[index] == name
        });
    if !known {
        return false;
    }
    if i.st.env.theme != name {
        i.st.env.theme = name.to_owned();
        i.dirty = true;
    }
    true
}

/// Returns the current theme name; empty means the authored base.
pub fn inst_theme(i: &Instance) -> String {
    i.st.env.theme.clone()
}

/// Toggles a global state by name.
///
/// Names are interned against the document string pool. A name the document
/// never mentions cannot affect a condition and is therefore a no-op.
pub fn inst_set_state(i: &mut Instance, name: &str, on: bool) {
    let Some(sym) = i
        .doc
        .strs
        .iter()
        .rposition(|candidate| candidate == name)
        .and_then(|index| u32::try_from(index).ok())
    else {
        return;
    };
    let index = i.st.states.iter().rposition(|&state| state == sym);
    match (on, index) {
        (true, None) => {
            i.st.states.push(sym);
            i.dirty = true;
        }
        (false, Some(index)) => {
            i.st.states.swap_remove(index);
            i.dirty = true;
        }
        _ => {}
    }
}

/// Toggles a named state on one node addressed by its full key path.
///
/// Dispatch owns hover, pressed, and focus states; hosts use this API for app
/// states such as `disabled` and `selected`. Returns `false` for an unknown key.
pub fn inst_set_node_state(i: &mut Instance, key: &str, name: &str, on: bool) -> bool {
    let node = scene::node_by_key(&i.doc, &i.st.lists, key);
    if node == slir::NONE {
        return false;
    }
    if style::set_node_state(&i.doc, &mut i.st, node, name, on) {
        i.dirty = true;
    }
    true
}

/// Moves focus to a keyed node, or clears focus when `key` is empty.
///
/// Hosts move focus for dialogs and wizards. The node must be focusable in
/// the CURRENT scene (present, not inert, `focusable`); unknown, absent, or
/// non-focusable keys return `false` without side effects. `visible` selects
/// the keyboard-grade focus ring exactly as Tab traversal does, and a
/// `field=` target binds its edit state on focus.
pub fn inst_set_focus(i: &mut Instance, key: &str, visible: bool) -> bool {
    if key.is_empty() {
        if focus::set_focus(&i.doc, &mut i.st, &mut i.ds.fs, slir::NONE, false) {
            i.dirty = true;
        }
        return true;
    }
    let node = scene::node_by_key(&i.doc, &i.st.lists, key);
    if node == slir::NONE {
        return false;
    }
    let mut focusables = Vec::new();
    scene::focusables(&i.sc, &mut focusables);
    if !focusables.contains(&node) {
        return false;
    }
    if focus::set_focus(&i.doc, &mut i.st, &mut i.ds.fs, node, visible) {
        i.dirty = true;
    }
    dispatch::bind_edit_on_focus(&i.doc, &mut i.st, &mut i.ds);
    true
}
/// Replaces a keyed field's edit buffer and synchronizes a same-named text parameter.
///
/// The replacement clears composition, selection, and undo/redo history, then
/// places the caret at the end. A changed value queues one Change signal for
/// [`inst_take_signals`] and marks the instance dirty. Unknown and non-field
/// keys return `false` without side effects.
pub fn inst_set_field_text(i: &mut Instance, key: &str, text: &str) -> bool {
    let node = scene::node_by_key(&i.doc, &i.st.lists, key);
    if node == slir::NONE {
        return false;
    }
    let signal_index = dispatch::sig_of(&i.doc, &i.st, node, dispatch::TR_CHANGE);
    if signal_index < 0 {
        return false;
    }

    let edit_index = dispatch::ed_ix(&i.ds, node);
    let (previous, display_changed) = if edit_index >= 0 {
        let index = usize::try_from(edit_index).expect("negative edit index");
        let state = &i.ds.ed[index];
        (
            edit::text_str(state),
            edit::display_str(state) != text
                || state.caret != crate::rt::str_len(text)
                || state.anchor != state.caret,
        )
    } else {
        let content = style::content_str(&i.doc, &i.st, node);
        let changed = content != text;
        (content, changed)
    };
    let text_changed = previous != text;

    let replacement = edit::es_new(node, text);
    if edit_index >= 0 {
        let index = usize::try_from(edit_index).expect("negative edit index");
        i.ds.ed[index] = replacement;
    } else {
        i.ds.ed_node.push(node);
        i.ds.ed.push(replacement);
    }
    let composing_changed = style::set_node_state(&i.doc, &mut i.st, node, "composing", false);
    let scroll_changed = style::field_scroll_x(&i.st, node) != 0.0;
    if scroll_changed {
        style::field_scroll_set(&mut i.st, node, 0.0);
    }
    if display_changed {
        style::field_set(&mut i.st, node, text);
    }
    let param_changed = dispatch::sync_bound_text_param(&i.doc, &mut i.st, node, text);
    if text_changed || param_changed {
        dispatch::queue_field_change(&i.doc, &i.st, &mut i.ds, node, text);
    }
    if display_changed || param_changed || composing_changed || scroll_changed {
        i.dirty = true;
    }
    true
}

/// Returns a keyed field's committed edit text, or its content before first bind.
pub fn inst_field_text(i: &Instance, key: &str) -> Option<String> {
    let node = scene::node_by_key(&i.doc, &i.st.lists, key);
    if node == slir::NONE || dispatch::sig_of(&i.doc, &i.st, node, dispatch::TR_CHANGE) < 0 {
        return None;
    }
    let edit_index = dispatch::ed_ix(&i.ds, node);
    if edit_index < 0 {
        Some(style::content_str(&i.doc, &i.st, node))
    } else {
        let index = usize::try_from(edit_index).expect("negative edit index");
        Some(edit::text_str(&i.ds.ed[index]))
    }
}

/// Returns the focused node, or [`slir::NONE`] when focus is clear.
pub fn inst_focus(i: &Instance) -> u32 {
    i.ds.fs.focus
}

/// Returns a named parameter's current value as deterministic JSON.
pub fn inst_param_json(i: &Instance, name: &str) -> Option<String> {
    let param = i
        .doc
        .parm_name
        .iter()
        .position(|param_name| slir::str_at(&i.doc, *param_name) == name)?;
    dumpjson::param_json(
        &i.doc,
        &i.st,
        u32::try_from(param).expect("parameter index exceeds u32"),
    )
}

/// Sets one axis of a keyed scroll node, clamped to retained geometry.
///
/// Axis `0` is the node's main axis and axis `1` is its cross axis. A write
/// before the first solve is retained and clamped after geometry is available.
/// Unknown axes, keys, and inactive axes return `false`.
pub fn inst_set_scroll(i: &mut Instance, key: &str, axis: u32, off: f64) -> bool {
    if axis > 1 {
        return false;
    }
    let node = scene::node_by_key(&i.doc, &i.st.lists, key);
    if node == slir::NONE {
        return false;
    }
    let flags = style::eff_flags(&i.doc, &i.st, node);
    let required = if axis == 0 {
        slir::F_SCROLL
    } else {
        slir::F_SCROLL_CROSS
    };
    if flags & required == 0 {
        return false;
    }

    let mut next = off;
    if i.solved {
        let scene_index = scene::index_of(&i.sc, node);
        if scene_index >= 0 {
            next = dispatch::clamp_scroll_axis(&i.sc, scene_index, axis, next);
        }
    }
    if style::scroll_set_axis(&mut i.st, node, axis, next) {
        i.dirty = true;
    }
    true
}

/// Returns one axis of a keyed scroll node; unknown keys and axes read as zero.
pub fn inst_get_scroll(i: &Instance, key: &str, axis: u32) -> f64 {
    if axis > 1 {
        return 0.0;
    }
    let node = scene::node_by_key(&i.doc, &i.st.lists, key);
    if node == slir::NONE {
        0.0
    } else {
        style::scroll_get_axis(&i.st, node, axis)
    }
}

type RevealCorners = [(f64, f64); 4];

fn rotate_reveal_corners(sc: &Scene, corners: &mut RevealCorners, scene_index: usize) {
    let degrees = sc.rot[scene_index];
    if degrees == 0.0 {
        return;
    }
    let cosine = hit::cos_deg(degrees);
    let sine = hit::sin_deg(degrees);
    let cx = sc.cx[scene_index];
    let cy = sc.cy[scene_index];
    for (x, y) in corners {
        let dx = *x - cx;
        let dy = *y - cy;
        *x = cx + dx * cosine - dy * sine;
        *y = cy + dx * sine + dy * cosine;
    }
}

fn reveal_bounds(corners: &RevealCorners, physical_x: bool) -> (f64, f64) {
    let mut start = f64::INFINITY;
    let mut end = f64::NEG_INFINITY;
    for &(x, y) in corners {
        let value = if physical_x { x } else { y };
        start = start.min(value);
        end = end.max(value);
    }
    (start, end)
}

fn translate_reveal_corners(corners: &mut RevealCorners, physical_x: bool, delta: f64) {
    for (x, y) in corners {
        if physical_x {
            *x -= delta;
        } else {
            *y -= delta;
        }
    }
}

/// Scrolls every active-axis ancestor minimally to reveal a current scene node.
///
/// The nonnegative finite margin is applied on every active scroll axis.
/// Descendant rotations and inner scroll movements are composed into each
/// ancestor's local scroll frame. Returns `false` only when the key does not
/// resolve to a retained scene entry.
pub fn inst_reveal(i: &mut Instance, key: &str, margin: f64) -> bool {
    let node = scene::node_by_key(&i.doc, &i.st.lists, key);
    let target = scene::index_of(&i.sc, node);
    if node == slir::NONE || target < 0 {
        return false;
    }
    let target = usize::try_from(target).expect("negative scene index");
    let x = i.sc.x[target];
    let y = i.sc.y[target];
    let w = i.sc.w[target];
    let h = i.sc.h[target];
    let mut corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
    let margin = if margin.is_finite() {
        margin.max(0.0)
    } else {
        0.0
    };

    let mut child = target;
    let mut current = i.sc.parent[target];
    while current >= 0 {
        rotate_reveal_corners(&i.sc, &mut corners, child);
        let index = usize::try_from(current).expect("negative scene index");
        for axis in 0_u32..=1 {
            let required = if axis == 0 {
                slir::F_SCROLL
            } else {
                slir::F_SCROLL_CROSS
            };
            if i.sc.flags[index] & required == 0 {
                continue;
            }

            let owner = i.sc.node[index];
            let old = style::scroll_get_axis(&i.st, owner, axis);
            let physical_x = i.sc.is_row[index] == (axis == 0);
            let (mut start, mut end) = reveal_bounds(&corners, physical_x);
            start -= margin;
            end += margin;
            let (viewport_start, viewport_end) = if physical_x {
                (i.sc.x[index], i.sc.x[index] + i.sc.w[index])
            } else {
                (i.sc.y[index], i.sc.y[index] + i.sc.h[index])
            };
            let desired = if start < viewport_start {
                old + start - viewport_start
            } else if end > viewport_end {
                old + end - viewport_end
            } else {
                old
            };
            let next = dispatch::clamp_scroll_axis(&i.sc, current, axis, desired);
            if style::scroll_set_axis(&mut i.st, owner, axis, next) {
                i.dirty = true;
                translate_reveal_corners(&mut corners, physical_x, next - old);
            }
        }
        child = index;
        current = i.sc.parent[index];
    }
    true
}

fn virtual_scene_geometry(i: &Instance, parent: u32, each: u32) -> Option<(f64, f64, f64)> {
    let parent_index = scene::index_of(&i.sc, parent);
    let each_index = scene::index_of(&i.sc, each);
    if parent_index < 0 || each_index < 0 {
        return None;
    }
    let parent_index = usize::try_from(parent_index).expect("negative scene index");
    let each_index = usize::try_from(each_index).expect("negative scene index");
    let row = i.sc.is_row[parent_index];
    let viewport = if row {
        i.sc.w[parent_index]
    } else {
        i.sc.h[parent_index]
    };
    let painted_origin = if row {
        i.sc.x[each_index] - i.sc.x[parent_index]
    } else {
        i.sc.y[each_index] - i.sc.y[parent_index]
    };
    let origin = painted_origin + style::scroll_get(&i.st, parent);
    Some((viewport, i.sc.content_main[parent_index], origin))
}

/// Reveals an item in a virtual `each`; non-virtual and unknown lists return `false`.
pub fn inst_reveal_item(i: &mut Instance, each_key: &str, item_index: i32, align: u32) -> bool {
    if align > 3 {
        return false;
    }
    let each = scene::node_by_key(&i.doc, &i.st.lists, each_key);
    let Some((extent, _, parent)) = list::virtual_config(&i.doc, &i.st.lists, each) else {
        return false;
    };
    let list_id = list::each_list(&i.doc, &i.st.lists, each);
    if list_id < 0
        || item_index < 0
        || item_index >= list::length(&i.doc, &i.st.lists, list_id as u32)
    {
        return false;
    }
    let Some((viewport, content, origin)) = virtual_scene_geometry(i, parent, each) else {
        return false;
    };
    if viewport <= 0.0 {
        return false;
    }
    let old = style::scroll_get(&i.st, parent);
    let start = origin + f64::from(item_index) * extent;
    let end = start + extent;
    let target = match align {
        0 => start,
        1 => start - (viewport - extent) / 2.0,
        2 => end - viewport,
        3 if start < old => start,
        3 if end > old + viewport => end - viewport,
        3 => old,
        _ => unreachable!("align was validated"),
    };
    let target = target.clamp(0.0, (content - viewport).max(0.0));
    if style::scroll_set(&mut i.st, parent, target) {
        i.dirty = true;
    }
    true
}

/// Returns the materialized range for a virtual `each`, or `(-1, -1)`.
pub fn inst_each_window(i: &Instance, each_key: &str) -> (i32, i32) {
    let each = scene::node_by_key(&i.doc, &i.st.lists, each_key);
    if list::virtual_config(&i.doc, &i.st.lists, each).is_none() {
        (-1, -1)
    } else {
        list::current_window(&i.st.lists, each)
    }
}

/// Sets a parameter's current value.
///
/// Returns `false` for an unknown parameter, a type mismatch, a list
/// parameter, or an enum value that is not a declared member.
///
/// Equal values are a no-op and do not mark the instance dirty.
pub fn inst_set_param(i: &mut Instance, param: u32, v: &ParamValue) -> bool {
    let Ok(param_index) = usize::try_from(param) else {
        return false;
    };
    if param_index >= i.doc.parm_name.len()
        || i.doc.parm_type[param_index] != v.kind
        || v.kind == slir::PARAM_LIST
    {
        return false;
    }

    match v.kind {
        0 => {
            if i.st.pv_str[param_index] == v.s {
                return true;
            }
            i.st.pv_str[param_index] = v.s.clone();
        }
        1 | 2 => {
            if i.st.pv_num[param_index] == v.num {
                return true;
            }
            i.st.pv_num[param_index] = v.num;
        }
        3 => {
            if i.st.pv_h[param_index] == v.rgba {
                return true;
            }
            i.st.pv_h[param_index] = v.rgba;
        }
        4 => {
            let next = if v.num == 0.0 { 0.0 } else { 1.0 };
            if i.st.pv_num[param_index] == next {
                return true;
            }
            i.st.pv_num[param_index] = next;
        }
        5 => {
            let enum_offset = i.doc.parm_enum_off[param_index];
            let enum_len = i.doc.parm_enum_len[param_index];
            let declared = (enum_offset..enum_offset.wrapping_add(enum_len)).any(|index| {
                let index = usize::try_from(index).expect("negative parameter enum index");
                let symbol = i.doc.parm_enum_syms[index];
                let symbol = usize::try_from(symbol).expect("enum string index is too large");
                i.doc.strs[symbol] == v.sym
            });
            if !declared {
                return false;
            }
            if i.st.pv_sym[param_index] == v.sym {
                return true;
            }
            i.st.pv_sym[param_index] = v.sym.clone();
        }
        _ => {}
    }
    i.dirty = true;
    true
}

/// Returns a root or nested list's current length, or `-1` when unresolved.
///
/// An empty path selects the root list. A nested path alternates a decimal item
/// index and a list-typed field name, for example `3.segments` or
/// `3.segments.0.points`. Malformed paths, scalar fields, absent schemas, and
/// out-of-range items are unknown.
pub fn inst_list_len(i: &Instance, param: u32, path: &str) -> i32 {
    let list_id = list::resolve_path(&i.doc, &i.st.lists, param, path);
    if list_id == u32::MAX {
        -1
    } else {
        list::length(&i.doc, &i.st.lists, list_id)
    }
}

/// Resizes the list selected by `param` and `path` atomically.
pub fn inst_set_list_len(i: &mut Instance, param: u32, path: &str, n: i32) -> bool {
    let changed = list::set_len_path(&i.doc, &mut i.st.lists, param, path, n);
    if changed < 0 {
        return false;
    }
    i.dirty |= changed > 0;
    true
}

/// Sets one typed scalar field in an item of the selected list atomically.
pub fn inst_set_list_field(
    i: &mut Instance,
    param: u32,
    path: &str,
    index: i32,
    field: &str,
    v: &ParamValue,
) -> bool {
    let value = list::Val {
        kind: v.kind,
        num: v.num,
        s: v.s.clone(),
        rgba: v.rgba,
        sym: v.sym.clone(),
    };
    let changed = list::set_field_path(&i.doc, &mut i.st.lists, param, path, index, field, &value);
    if changed < 0 {
        return false;
    }
    i.dirty |= changed > 0;
    true
}

/// Sets one stable item key in the list selected by `param` and `path`.
pub fn inst_set_list_key(i: &mut Instance, param: u32, path: &str, index: i32, key: &str) -> bool {
    let changed = list::set_key_path(&i.doc, &mut i.st.lists, param, path, index, key);
    if changed < 0 {
        return false;
    }
    i.dirty |= changed > 0;
    true
}

fn divider_node(i: &Instance, key: &str) -> Option<u32> {
    let node = scene::node_by_key(&i.doc, &i.st.lists, key);
    let base = list::base(&i.st.lists, &i.doc, node);
    let base_index = usize::try_from(base).ok()?;
    if i.doc.node_kind.get(base_index) != Some(&slir::K_DIVIDER) {
        return None;
    }

    let parent = *i.doc.node_parent.get(base_index)?;
    let parent_index = usize::try_from(parent).ok()?;
    if !matches!(
        i.doc.node_kind.get(parent_index),
        Some(&slir::K_ROW) | Some(&slir::K_COL)
    ) {
        return None;
    }

    let next = *i.doc.node_next.get(base_index)?;
    if next == slir::NONE {
        return None;
    }
    let next_index = usize::try_from(next).ok()?;
    if i.doc.node_parent.get(next_index) != Some(&parent) {
        return None;
    }

    let mut sibling = *i.doc.node_first.get(parent_index)?;
    for _ in 0..i.doc.node_kind.len() {
        if sibling == base {
            return (sibling != *i.doc.node_first.get(parent_index)?).then_some(node);
        }
        let sibling_index = usize::try_from(sibling).ok()?;
        sibling = *i.doc.node_next.get(sibling_index)?;
    }
    None
}

fn clamp_divider_authored(i: &Instance, node: u32, extent: f64) -> f64 {
    let base = list::base(&i.st.lists, &i.doc, node);
    let Ok(base_index) = usize::try_from(base) else {
        return extent;
    };
    let Some(&parent) = i.doc.node_parent.get(base_index) else {
        return extent;
    };
    let Ok(parent_index) = usize::try_from(parent) else {
        return extent;
    };
    let row = i.doc.node_kind.get(parent_index) == Some(&slir::K_ROW);
    let mut sibling = i.doc.node_first[parent_index];
    let mut previous = slir::NONE;
    while sibling != slir::NONE && sibling != base {
        previous = sibling;
        let Ok(index) = usize::try_from(sibling) else {
            return extent;
        };
        sibling = i.doc.node_next[index];
    }
    if previous == slir::NONE || sibling != base {
        return extent;
    }
    let min = style::attr_num(
        &i.doc,
        &i.st,
        previous,
        if row { slir::A_MIN_W } else { slir::A_MIN_H },
        0.0,
    );
    let max = style::attr_num(
        &i.doc,
        &i.st,
        previous,
        if row { slir::A_MAX_W } else { slir::A_MAX_H },
        style::INF,
    );
    style::divider_clamp(extent, min, max, max)
}

/// Sets the size overlay controlled by a keyed, structurally valid divider.
///
/// The divider must be between two siblings in a row or column. The previous
/// pane's min/max and, after a solve, the next pane's minimum clamp the value.
/// Non-finite extents and invalid keys return `false` without changing state.
pub fn inst_set_divider(i: &mut Instance, key: &str, extent: f64) -> bool {
    let Some(node) = divider_node(i, key) else {
        return false;
    };
    if !extent.is_finite() {
        return false;
    }
    let extent = if i.solved {
        dispatch::clamp_divider_for_scene(&i.doc, &i.st, &i.sc, node, extent)
            .unwrap_or_else(|| clamp_divider_authored(i, node, extent))
    } else {
        clamp_divider_authored(i, node, extent)
    };
    i.dirty |= style::divider_set(&mut i.st, node, extent);
    true
}

/// Returns a keyed divider overlay, or `-1` when unknown or unset.
pub fn inst_get_divider(i: &Instance, key: &str) -> f64 {
    let Some(node) = divider_node(i, key) else {
        return -1.0;
    };
    style::divider_get(&i.st, node).unwrap_or(-1.0)
}

/// Records hole content size.
///
/// Equal reports are a no-op so demand-driven hosts converge after the
/// re-solve triggered by a changed measurement.
pub fn inst_set_hole_size(i: &mut Instance, hole: u32, w: f64, h: f64) {
    let Ok(hole) = usize::try_from(hole) else {
        return;
    };
    let Some(current_width) = i.st.hole_w.get_mut(hole) else {
        return;
    };
    if *current_width == w && i.st.hole_h[hole] == h {
        return;
    }
    *current_width = w;
    i.st.hole_h[hole] = h;
    i.dirty = true;
}

/// Solves and lowers one frame, optionally applying motion overlays.
pub fn solve_frame(i: &mut Instance, t_ms: f64, with_motion: bool) -> Frame {
    let mut frame = flatten::frame_new();
    solve_frame_into(i, t_ms, with_motion, &mut frame);
    frame
}

fn solve_frame_into(i: &mut Instance, t_ms: f64, with_motion: bool, frame: &mut Frame) {
    solve_layout(i, t_ms, with_motion);
    flatten::flatten_into(&i.doc, &i.st, &i.lay, &i.ds, &i.ms, i.root_pi, frame);
}

fn solve_layout(i: &mut Instance, t_ms: f64, with_motion: bool) {
    style::begin_solve(&i.doc, &mut i.st);
    if dispatch::prune_vanished(&i.doc, &mut i.st, &mut i.ds) {
        // A surviving Drop target changed state after list identity pruning;
        // rebuild state conditions before layout consumes the fresh patches.
        style::begin_solve(&i.doc, &mut i.st);
    }
    if with_motion {
        motion::apply(&i.doc, &mut i.st, &mut i.ms, t_ms);
    }
    let viewport_width = i.st.env.vw;
    let viewport_height = i.st.env.vh;
    i.root_pi = layout::solve(
        &i.doc,
        &mut i.st,
        &mut i.lay,
        viewport_width,
        viewport_height,
        viewport_height > 0.0,
    );
    layout::place_attached(
        &i.doc,
        &i.st,
        &mut i.lay,
        i.root_pi,
        viewport_width,
        viewport_height,
    );
}

// A non-fixed divider handle can change after its pane overlay is clamped.
// Iterate only to the layout solver's EPS tolerance and keep each frame call bounded.
// Settling iterations only re-measure; the converged layout flattens once.
fn solve_frame_settled(i: &mut Instance, t_ms: f64, with_motion: bool, frame: &mut Frame) -> bool {
    const LIMIT: usize = 16;
    for _ in 0..LIMIT {
        solve_layout(i, t_ms, with_motion);
        if !i.st.divider_footprint_changed {
            break;
        }
    }
    flatten::flatten_into(&i.doc, &i.st, &i.lay, &i.ds, &i.ms, i.root_pi, frame);
    i.st.divider_footprint_changed
}

/// Solves if needed and lowers the result to a frame.
///
/// `t_ms` is the motion clock. A document with animation bindings or
/// transitions re-solves whenever it changes so interpolated inputs can be
/// laid out. A static document solves only when environment, parameter, state,
/// scroll, or edit changes mark it dirty.
///
/// After a solve, scroll offsets are clamped against the fresh scene and
/// vanished focus is restored. Either may mark the instance dirty for the next
/// frame.
fn refresh_virtual_window(i: &mut Instance, each: u32) -> bool {
    let Some((_, _, parent)) = list::virtual_config(&i.doc, &i.st.lists, each) else {
        return false;
    };
    let Some((viewport, _, origin)) = virtual_scene_geometry(i, parent, each) else {
        return false;
    };
    let off = style::scroll_get(&i.st, parent);
    list::set_virtual_viewport(&i.doc, &mut i.st.lists, each, viewport, off, origin)
}

fn refresh_virtual_windows(i: &mut Instance) -> bool {
    let mut changed = false;
    for each_index in 0..i.doc.node_kind.len() {
        if i.doc.node_kind[each_index] != slir::K_EACH
            || i.doc.node_flags[each_index] & slir::F_VIRTUAL == 0
        {
            continue;
        }
        let each = u32::try_from(each_index).expect("node index exceeds u32");
        changed |= refresh_virtual_window(i, each);
    }
    let materialized_len = list::materialized(&i.st.lists).len();
    for index in 0..materialized_len {
        let each = list::materialized(&i.st.lists)[index];
        let base = list::base(&i.st.lists, &i.doc, each);
        let Ok(base_index) = usize::try_from(base) else {
            continue;
        };
        if i.doc.node_kind.get(base_index) != Some(&slir::K_EACH)
            || i.doc.node_flags.get(base_index).copied().unwrap_or(0) & slir::F_VIRTUAL == 0
        {
            continue;
        }
        changed |= refresh_virtual_window(i, each);
    }
    changed
}

fn clamp_retained_scrolls(i: &mut Instance) -> bool {
    let mut changed = false;
    for index in 0..i.st.scroll_node.len() {
        let scene_index = scene::index_of(&i.sc, i.st.scroll_node[index]);
        if scene_index < 0 {
            continue;
        }
        let clamped = dispatch::clamp_scroll(&i.sc, scene_index, i.st.scroll_off[index]);
        if clamped != i.st.scroll_off[index] {
            i.st.scroll_off[index] = clamped;
            changed = true;
        }
    }

    for index in 0..i.st.scroll_cross_node.len() {
        let scene_index = scene::index_of(&i.sc, i.st.scroll_cross_node[index]);
        if scene_index < 0 {
            continue;
        }
        let clamped =
            dispatch::clamp_scroll_axis(&i.sc, scene_index, 1, i.st.scroll_cross_off[index]);
        if clamped != i.st.scroll_cross_off[index] {
            i.st.scroll_cross_off[index] = clamped;
            changed = true;
        }
    }
    changed
}

/// Updates a caller-retained frame when solving or animation changes its output.
///
/// `frame` must remain paired with this instance from its first call. A clean
/// instance leaves it untouched and returns `false`, preserving every backing
/// allocation and avoiding a redundant flatten pass.
pub fn inst_frame_update(i: &mut Instance, t_ms: f64, frame: &mut Frame) -> bool {
    write_frame(i, t_ms, frame, true)
}

/// Solves if needed and lowers the result to a frame at the supplied motion clock.
///
/// Fresh virtual-list viewport geometry marks the instance dirty for one
/// settling frame, without pruning identities outside the materialized window.
pub fn inst_frame(i: &mut Instance, t_ms: f64) -> Frame {
    let mut frame = flatten::frame_new();
    write_frame(i, t_ms, &mut frame, false);
    frame
}

fn write_frame(i: &mut Instance, t_ms: f64, frame: &mut Frame, retain_clean: bool) -> bool {
    if !i.ok {
        frame.clear();
        return true;
    }

    let has_motion = !i.doc.bind_node.is_empty() || !i.doc.trans_node.is_empty();
    let needs_solve = i.dirty || !i.solved || i.ms.active || has_motion && t_ms != i.last_t;
    if !needs_solve {
        if retain_clean {
            return false;
        }
        flatten::flatten_into(&i.doc, &i.st, &i.lay, &i.ds, &i.ms, i.root_pi, frame);
        return true;
    }

    let divider_unsettled = solve_frame_settled(i, t_ms, true, frame);
    i.dirty = divider_unsettled;
    i.solved = true;
    i.last_t = t_ms;
    scene::load(&mut i.sc, frame);
    if dispatch::cancel_invalid_drag(&i.doc, &mut i.st, &i.sc, &mut i.ds) {
        i.dirty |= solve_frame_settled(i, t_ms, true, frame);
        scene::load(&mut i.sc, frame);
    }
    if refresh_virtual_windows(i) {
        i.dirty = true;
    }

    if clamp_retained_scrolls(i) {
        i.dirty = true;
    }

    // Editing dispatch used the previous layout. Follow once more against the
    // freshly wrapped lines, then settle any changed scroll inputs immediately.
    if dispatch::follow_caret_fresh(&i.doc, &mut i.st, &i.lay, &i.sc, &mut i.ds) {
        i.dirty = solve_frame_settled(i, t_ms, true, frame);
        scene::load(&mut i.sc, frame);
        if refresh_virtual_windows(i) {
            i.dirty = true;
        }
        if dispatch::follow_caret_fresh(&i.doc, &mut i.st, &i.lay, &i.sc, &mut i.ds) {
            i.dirty = true;
        }
    }

    // Restore vanished focus to its nearest neighbour.
    if i.ds.fs.focus != slir::NONE
        && scene::index_of(&i.sc, i.ds.fs.focus) < 0
        && focus::restore(&i.doc, &mut i.st, &i.sc, &mut i.ds.fs)
    {
        i.dirty = true;
    }
    focus::refresh(&i.sc, &mut i.ds.fs);
    true
}

/// Solves without animation or transition overlays for static exporters.
///
/// Current parameters, conditions, fields, and scroll offsets still apply.
pub fn inst_frame_static(i: &mut Instance) -> Frame {
    let mut frame = flatten::frame_new();
    if !i.ok {
        return frame;
    }
    solve_frame_settled(i, 0.0, false, &mut frame);
    scene::load(&mut i.sc, &frame);
    if refresh_virtual_windows(i) {
        solve_frame_settled(i, 0.0, false, &mut frame);
        scene::load(&mut i.sc, &frame);
    }
    if clamp_retained_scrolls(i) {
        refresh_virtual_windows(i);
        solve_frame_settled(i, 0.0, false, &mut frame);
        scene::load(&mut i.sc, &frame);
    }
    frame
}

/// Solves pending changes and returns the resulting hole rectangles.
pub fn inst_holes(i: &mut Instance) -> Vec<HoleRect> {
    if !i.ok {
        return Vec::new();
    }
    if i.dirty || !i.solved || i.ms.active {
        let _ = inst_frame(i, i.last_t);
    }
    inst_holes_retained(i)
}

/// Returns hole rectangles from the most recently solved scene without re-solving.
pub fn inst_holes_retained(i: &Instance) -> Vec<HoleRect> {
    if !i.ok {
        return Vec::new();
    }
    let mut holes = Vec::new();
    for hole in 0..i.doc.hole_name.len() {
        let node = i.doc.hole_node[hole];
        for (scene_index, scene_node) in i.sc.node.iter().copied().enumerate() {
            if scene_node == node {
                holes.push(HoleRect {
                    hole: u32::try_from(hole).expect("too many document holes"),
                    x: i.sc.x[scene_index],
                    y: i.sc.y[scene_index],
                    w: i.sc.w[scene_index],
                    h: i.sc.h[scene_index],
                    clip: i.sc.flags[scene_index] & (slir::F_CLIP | slir::F_SCROLL) != 0,
                });
            }
        }
    }
    holes
}

/// Returns the node path from root to target for a point in the retained scene.
///
/// Hit testing observes reverse paint order, rotation, and clipping.
pub fn inst_hit(i: &Instance, x: f64, y: f64) -> Vec<u32> {
    let mut path = Vec::new();
    hit::hit_test(&i.sc, x, y, &mut path);
    path.into_iter()
        .map(|index| {
            let index = usize::try_from(index).expect("negative scene path index");
            i.sc.node[index]
        })
        .collect()
}

/// Dispatches an event against the retained scene.
///
/// [`Effects::repaint`] also marks the instance dirty so the next frame call
/// re-solves.
pub fn inst_dispatch(i: &mut Instance, ev: &Event) -> Effects {
    if !i.ok {
        return dispatch::effects_new();
    }
    let effects = dispatch::dispatch(&i.doc, &mut i.st, &i.lay, &i.sc, &mut i.ds, ev);
    if effects.repaint {
        i.dirty = true;
    }
    effects
}

/// Drains signals queued by frame-settle gesture cancellation.
///
/// Live hosts call this immediately after each settled frame. Dispatch-time
/// signals remain in the [`Effects`] returned by [`inst_dispatch`].
pub fn inst_take_signals(i: &mut Instance) -> Effects {
    dispatch::take_pending_signals(&mut i.ds)
}

/// Returns glyph positions for a text frame operation.
///
/// GPU drivers receive a per-codepoint advance walk using the same font table
/// as the solver. `op` indexes `fr.ops` and must name a text operation; any
/// other index or operation yields an empty vector.
pub fn text_glyphs(i: &Instance, fr: &Frame, op: i32) -> Vec<GlyphPos> {
    let Ok(op) = usize::try_from(op) else {
        return Vec::new();
    };
    let Some(FrameOp::Text(text)) = fr.ops.get(op) else {
        return Vec::new();
    };
    if text.font < 0 {
        return Vec::new();
    }

    let string_index = usize::try_from(text.str_ref).expect("negative frame string index");
    let mut x = text.x;
    fr.strings[string_index]
        .chars()
        .map(|character| {
            let codepoint = u32::from(character);
            let glyph = GlyphPos {
                font: text.font,
                gid: slir::font_gid(&i.doc, text.font, codepoint),
                x,
                y: text.y_baseline,
                size: text.size,
            };
            x += textm::char_w(&i.doc, text.font, text.size, text.tracking, codepoint);
            glyph
        })
        .collect()
}
