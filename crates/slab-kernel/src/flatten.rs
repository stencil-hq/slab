//! Lowers a placed tree into frame operations and retained scene nodes in one
//! depth-first traversal. Paint order and scene geometry are therefore
//! identical by construction. Coordinates use absolute document units.
//! This retains the flattening model established by `research/layout.py`.

use crate::{
    dispatch::{self, DState},
    edit, graphemes,
    layout::Lay,
    list,
    motion::MSt,
    slir,
    style::{self, RStyle, St},
    textm,
};

// Frame operation payloads.

/// A painted rectangle, including its fill, stroke, shadows, and opacity.
#[derive(Clone, Debug)]
pub struct OpRect {
    pub node: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub radius: f64,
    /// `0` for none, `1` for a solid color, or `2` for a gradient.
    pub bg_kind: u32,
    pub bg: u32,
    pub stroke_kind: u32,
    pub stroke: u32,
    pub stroke_w: f64,
    /// `0` for centered, `1` for inside, or `2` for outside.
    pub stroke_align: u32,
    /// Side bitmask: top = 1, right = 2, bottom = 4, left = 8.
    pub stroke_sides: u32,
    pub dash_on: f64,
    pub dash_off: f64,
    pub has_dash: bool,
    /// Start of this rectangle's run in the SLIR shadow pool.
    pub shadow_off: i32,
    pub shadow_len: i32,
    pub opacity: f64,
    /// Figma-style corner smoothing 0..1; `0` keeps circular corners.
    pub smooth: f64,
    /// Speckle overlay strength 0..1; `0` disables grain.
    pub grain_amount: f64,
    /// Speckle cell size in layout units.
    pub grain_size: f64,
}

/// A positioned text run referencing the frame-local string pool.
#[derive(Clone, Debug, Default)]
pub struct OpText {
    pub node: u32,
    pub x: f64,
    pub y_baseline: f64,
    /// Index into [`Frame::strings`].
    pub str_ref: i32,
    pub measured_w: f64,
    /// Font table index, or `-1` when the document has no matching font.
    pub font: i32,
    pub size: f64,
    pub weight: u32,
    pub tracking: f64,
    pub color: u32,
    pub opacity: f64,
    /// Whether the renderer paints a line through this text run.
    pub strike: bool,
    /// `1` when `color` is packed RGBA, `2` when it is a gradient handle.
    pub color_kind: u32,
    /// Gradient box (the text node's content box); zero when `color_kind` is 1.
    pub gx: f64,
    pub gy: f64,
    pub gw: f64,
    pub gh: f64,
    /// Start of this run's pairs in [`Frame::uncovered`]; `0` when `uncov_len` is `0`.
    pub uncov_off: i32,
    /// Number of uncovered-glyph codepoint runs in [`Frame::uncovered`].
    pub uncov_len: i32,
}

/// A positioned image operation.
#[derive(Clone, Debug)]
pub struct OpImage {
    pub node: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Image table index, or `-1` for an unresolved source.
    pub img: i32,
    /// `0` for cover, `1` for contain, or `2` for stretch.
    pub fit: u32,
    pub radius: f64,
    pub opacity: f64,
    /// Figma-style corner smoothing 0..1; `0` keeps circular corners.
    pub smooth: f64,
}

/// A positioned vector path operation.
#[derive(Clone, Debug)]
pub struct OpPath {
    pub node: u32,
    pub dx: f64,
    pub dy: f64,
    /// Non-negative document path index, or complemented frame-runtime index.
    pub path: i32,
    pub bg_kind: u32,
    pub bg: u32,
    pub stroke_kind: u32,
    pub stroke: u32,
    pub stroke_w: f64,
    pub dash_on: f64,
    pub dash_off: f64,
    pub has_dash: bool,
    pub opacity: f64,
}

/// A rounded rectangular clipping region.
#[derive(Clone, Debug)]
pub struct OpClip {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub radius: f64,
    /// Figma-style corner smoothing 0..1; `0` keeps circular corners.
    pub smooth: f64,
}

/// Compositing settings applied until the matching group pop.
#[derive(Clone, Debug)]
pub struct OpGroup {
    /// Document node owning this compositing group, or [`slir::NONE`] for a
    /// host-generated group such as the drag ghost envelope.
    pub node: u32,
    pub opacity: f64,
    pub blur: f64,
    /// Fade-mask paint: `0` none, `1` solid, `2` gradient.
    pub mask_kind: u32,
    /// Packed RGBA word or gradient handle selected by `mask_kind`.
    pub mask: u32,
    /// Mask box (the node's border box); ink outside it is fully masked.
    pub mx: f64,
    pub my: f64,
    pub mw: f64,
    pub mh: f64,
}

/// Rotation applied until the matching rotation pop.
#[derive(Clone, Debug)]
pub struct OpRotate {
    pub cx: f64,
    pub cy: f64,
    pub deg: f64,
}

/// Nonuniform scaling applied until the matching scale pop.
#[derive(Clone, Debug)]
pub struct OpScale {
    /// Fixed point left unchanged by the scale.
    pub cx: f64,
    /// Fixed point left unchanged by the scale.
    pub cy: f64,
    /// Horizontal scale factor.
    pub sx: f64,
    /// Vertical scale factor.
    pub sy: f64,
}

/// A backdrop filter over a rounded rectangle.
#[derive(Clone, Debug)]
pub struct OpBackdrop {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub radius: f64,
    pub blur: f64,
    pub saturate: f64,
    /// Backdrop RGB multiplier; `1` is identity.
    pub brightness: f64,
    /// Figma-style corner smoothing 0..1; `0` keeps circular corners.
    pub smooth: f64,
    /// Progressive-blur mask paint: `0` none, `1` solid, `2` gradient.
    pub mask_kind: u32,
    /// Packed RGBA word or gradient handle selected by `mask_kind`.
    pub mask: u32,
}

/// Ink-only 3D perspective applied until the matching tilt pop.
///
/// The subtree renders as one plane warped by
/// `perspective(depth) · rotateX(rx) · rotateY(ry)` about `(cx, cy)`
/// (CSS transform-list order; angles in degrees, depth in layout units).
#[derive(Clone, Debug)]
pub struct OpTilt {
    pub cx: f64,
    pub cy: f64,
    pub rx: f64,
    pub ry: f64,
    pub depth: f64,
}

/// A painter command emitted in document paint order.
#[derive(Clone, Debug)]
pub enum FrameOp {
    Rect(OpRect),
    Text(OpText),
    Image(OpImage),
    PathDraw(OpPath),
    ClipPush(OpClip),
    ClipPop,
    GroupPush(OpGroup),
    GroupPop,
    RotatePush(OpRotate),
    RotatePop,
    ScalePush(OpScale),
    ScalePop,
    Backdrop(OpBackdrop),
    TiltPush(OpTilt),
    TiltPop,
}

/// Retained scene entry in painter-order pre-order traversal.
///
/// `flags` contains the node's effective flags: `F_CLIP` means this frame
/// clips, while `F_INERT` is inherited from ancestors as well as the node.
#[derive(Clone, Debug)]
pub struct SceneNode {
    pub node: u32,
    pub parent_ix: i32,
    pub kind: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub radius: f64,
    pub rot_deg: f64,
    pub rot_cx: f64,
    pub rot_cy: f64,
    pub flags: u32,
    /// Child extent including trailing padding, used to clamp scrolling.
    pub content_main: f64,
    /// Current offset along the scrollable main axis.
    pub scroll_off: f64,
    /// Current offset along the scrollable cross axis.
    pub scroll_cross: f64,
    /// Cross-axis child extent including trailing padding.
    pub content_cross: f64,
    /// Whether the main axis is horizontal; this is omitted from frame JSON.
    pub is_row: bool,
    pub src_line: u32,
    /// Pre-order rank in the materialized authored tree, independent of paint promotion.
    pub authored_order: u32,
    /// Reference into [`St::scene_strs`] for the resolved accessibility role.
    pub role: u32,
    /// Reference into [`St::scene_strs`] for the resolved accessible label.
    pub label: u32,
    /// Reference into [`St::scene_strs`] for the resolved accessible description.
    pub desc: u32,
    /// Optional checked state: 0 absent, 1 false, 2 true, 3 mixed.
    pub checked: u32,
    /// Optional expanded state: 0 absent, 1 false, 2 true.
    pub expanded: u32,
    /// Optional selected state: 0 absent, 1 false, 2 true.
    pub selected: u32,
    /// Scene-string reference for the active descendant's full key.
    pub active_descendant: u32,
    /// Scene-string reference for the controlled node's full key.
    pub controls: u32,
    /// Optional current range value.
    pub value_now: Option<f64>,
    /// Optional minimum range value.
    pub value_min: Option<f64>,
    /// Optional maximum range value.
    pub value_max: Option<f64>,
    /// Scene-string reference for the human-readable range value.
    pub value_text: u32,
    /// Optional modal state: 0 absent, 1 false, 2 true.
    pub modal: u32,
    /// Optional live-region mode: 0 absent, 1 off, 2 polite, 3 assertive.
    pub live: u32,
    /// Optional live-region atomicity: 0 absent, 1 false, 2 true.
    pub live_atomic: u32,
    /// Optional hierarchy level.
    pub level: Option<f64>,
    /// Optional one-based position within a set.
    pub pos_in_set: Option<f64>,
    /// Optional set size; -1 means unknown.
    pub set_size: Option<f64>,
    /// Whether the node is currently disabled.
    pub disabled: bool,
    /// Whether the node currently owns kernel focus.
    pub focused: bool,
    /// Whether the node is a text leaf with an active `field=` binder, i.e.
    /// kernel-editable; adapters expose it as textbox semantics.
    pub editable: bool,
}

/// One frame-local runtime path.
#[derive(Clone, Debug)]
pub struct RtPath {
    /// Canonical `M L C Q Z` verb codes.
    pub verbs: Vec<u8>,
    /// Absolute coordinates consumed by `verbs`.
    pub coords: Vec<f64>,
}

/// One host-visible diagnostic produced while solving or inspecting a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDiagnostic {
    pub code: String,
    pub line: u32,
    pub msg: String,
}

/// Flattened output for one frame.
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: f64,
    pub height: f64,
    pub ops: Vec<FrameOp>,
    pub scene: Vec<SceneNode>,
    /// Per-frame text pool addressed by [`OpText::str_ref`].
    pub strings: Vec<String>,
    /// Flat `[start, end)` codepoint-offset pairs of uncovered-glyph runs,
    /// addressed by [`OpText::uncov_off`] and [`OpText::uncov_len`].
    pub uncovered: Vec<u32>,
    /// Runtime paths referenced by negative [`OpPath::path`] values.
    pub paths_rt: Vec<RtPath>,
    /// Diagnostics observed for this frame. Runtime notes may be one-shot.
    pub diagnostics: Vec<FrameDiagnostic>,
    /// Recycled string allocations drained from `strings` on [`Frame::clear`].
    string_pool: Vec<String>,
    /// Recycled runtime-path allocations drained from `paths_rt` on [`Frame::clear`].
    path_pool: Vec<RtPath>,
    /// Retained authored-order scratch reused across flattening traversals.
    order_scratch: Vec<u32>,
}

impl Frame {
    /// Removes all frame output while retaining the backing allocations.
    pub fn clear(&mut self) {
        self.width = 0.0;
        self.height = 0.0;
        self.ops.clear();
        self.scene.clear();
        self.string_pool.append(&mut self.strings);
        self.uncovered.clear();
        self.path_pool.append(&mut self.paths_rt);
        self.diagnostics.clear();
    }
}

/// Creates an empty frame with zero dimensions.
pub fn frame_new() -> Frame {
    Frame {
        width: 0.0,
        height: 0.0,
        ops: Vec::new(),
        scene: Vec::new(),
        strings: Vec::new(),
        uncovered: Vec::new(),
        paths_rt: Vec::new(),
        diagnostics: Vec::new(),
        string_pool: Vec::new(),
        path_pool: Vec::new(),
        order_scratch: Vec::new(),
    }
}

// Flattening traversal.

fn index(value: i32) -> usize {
    usize::try_from(value).expect("negative flattening index")
}

fn count(value: usize) -> i32 {
    i32::try_from(value).expect("flattening pool exceeds i32 capacity")
}

/// Implements Rust's saturating float-to-unsigned-integer cast without `as`.
fn truncate_u32(value: f64) -> u32 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= f64::from(u32::MAX) {
        return u32::MAX;
    }

    let bits = value.to_bits();
    let exponent = i32::try_from((bits >> 52) & 0x7ff).expect("f64 exponent fits i32") - 1023;
    if exponent < 0 {
        return 0;
    }
    let significand = (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52);
    let magnitude = significand >> u32::try_from(52 - exponent).expect("nonnegative right shift");
    u32::try_from(magnitude).expect("bounded f64 magnitude fits u32")
}

fn clip(x: f64, y: f64, w: f64, h: f64, radius: f64, smooth: f64) -> OpClip {
    OpClip {
        x,
        y,
        w,
        h,
        radius,
        smooth,
    }
}

fn unstyled_rect(node: u32, x: f64, y: f64, w: f64, h: f64, radius: f64) -> OpRect {
    OpRect {
        node,
        x,
        y,
        w,
        h,
        radius,
        bg_kind: 0,
        bg: 0,
        stroke_kind: 0,
        stroke: 0,
        stroke_w: 1.0,
        stroke_align: 0,
        stroke_sides: 15,
        dash_on: 0.0,
        dash_off: 0.0,
        has_dash: false,
        shadow_off: 0,
        shadow_len: 0,
        opacity: 1.0,
        smooth: 0.0,
        grain_amount: 0.0,
        grain_size: 1.0,
    }
}

fn styled_rect(rule: &RStyle, node: u32, x: f64, y: f64, w: f64, h: f64, radius: f64) -> OpRect {
    let mut rect = unstyled_rect(node, x, y, w, h, radius);
    rect.bg_kind = rule.bg_kind;
    rect.bg = rule.bg_h;
    rect.stroke_kind = rule.stroke_kind;
    rect.stroke = rule.stroke_h;
    rect.stroke_w = rule.stroke_w;
    rect.stroke_align = rule.stroke_align;
    rect.stroke_sides = rule.stroke_sides;
    rect.dash_on = rule.dash_on;
    rect.dash_off = rule.dash_off;
    rect.has_dash = rule.has_dash;
    rect.shadow_off = rule.shadow_off;
    rect.shadow_len = rule.shadow_len;
    rect.smooth = rule.smooth;
    rect.grain_amount = rule.grain_amount;
    rect.grain_size = rule.grain_size;
    rect
}

/// Returns a representative solid RGBA for a text style: the packed color, or
/// a gradient's first stop when `color` holds a gradient handle.
fn solid_text_rgba(d: &slir::Doc, rule: &RStyle) -> u32 {
    if rule.color_kind != 2 {
        return rule.color;
    }
    let gradient = index(i32::try_from(rule.color).unwrap_or(0));
    if gradient >= d.grad_stop_len.len() || d.grad_stop_len[gradient] <= 0 {
        return 0x111111FF;
    }
    d.grad_stop_rgba[index(d.grad_stop_off[gradient])]
}

fn text_alignment_factor(alignment: u32) -> f64 {
    match alignment {
        1 => 0.5,
        2 => 1.0,
        _ => 0.0,
    }
}

/// Adds a character range to the frame-local string pool and returns its index.
pub fn push_str_slice(fr: &mut Frame, chars: &[u32], a: i32, b: i32) -> i32 {
    let codepoints = if a >= b {
        &[][..]
    } else {
        &chars[index(a)..index(b)]
    };
    let mut pooled = fr.string_pool.pop().unwrap_or_default();
    pooled.clear();
    for &codepoint in codepoints {
        pooled.push(char::from_u32(codepoint).expect("invalid codepoint"));
    }
    fr.strings.push(pooled);
    count(fr.strings.len()).wrapping_sub(1)
}

/// Emits a visible, undecorated solid rectangle.
pub fn push_solid_rect(fr: &mut Frame, node: u32, x: f64, y: f64, w: f64, h: f64, color: u32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    let mut rect = unstyled_rect(node, x, y, w, h, 0.0);
    rect.bg_kind = 1;
    rect.bg = color;
    fr.ops.push(FrameOp::Rect(rect));
}

fn frame_path_ref(d: &slir::Doc, st: &St, fr: &mut Frame, path: i32) -> Option<i32> {
    if path == style::PATH_NONE {
        return None;
    }
    if path >= 0 {
        return Some(path);
    }
    let verbs = style::runtime_path_verbs(st, path)?;
    let coords = style::path_coords(d, st, path)?;
    if let Some(index) = fr
        .paths_rt
        .iter()
        .position(|candidate| candidate.verbs == verbs && candidate.coords == coords)
    {
        return Some(!count(index));
    }
    let index = count(fr.paths_rt.len());
    let mut pooled = fr.path_pool.pop().unwrap_or_else(|| RtPath {
        verbs: Vec::new(),
        coords: Vec::new(),
    });
    pooled.verbs.clear();
    pooled.verbs.extend_from_slice(verbs);
    pooled.coords.clear();
    pooled.coords.extend_from_slice(coords);
    fr.paths_rt.push(pooled);
    Some(!index)
}

fn icon_paint(
    d: &slir::Doc,
    st: &St,
    node: u32,
    attr: u32,
    current_kind: u32,
    current: u32,
) -> (u32, u32) {
    let value = crate::value::decode_active(d, st.theme_index, slir::base_attr(d, node, attr));
    match value.tag {
        slir::T_PAINT_CURRENT => (current_kind, current),
        slir::T_PAINT_SOLID | slir::T_COLOR => (1, value.h),
        slir::T_PAINT_GRADIENT => (2, value.h),
        _ => (0, 0),
    }
}

fn emit_icon(
    d: &slir::Doc,
    st: &St,
    fr: &mut Frame,
    rule: &RStyle,
    node: u32,
    origin: (f64, f64),
    side: f64,
) {
    let (x, y) = origin;
    let Ok(icon) = usize::try_from(rule.icon) else {
        return;
    };
    let Some(&root) = d.icon_node.get(icon) else {
        return;
    };
    let viewbox = d.icon_viewbox.get(icon).copied().unwrap_or(24.0);
    if viewbox <= 0.0 || side <= 0.0 {
        return;
    }
    let scale = side / viewbox;
    fr.ops.push(FrameOp::ScalePush(OpScale {
        cx: x,
        cy: y,
        sx: scale,
        sy: scale,
    }));
    let mut child = d.node_first[index(i32::try_from(root).expect("icon root exceeds i32"))];
    while child != slir::NONE {
        let child_index = usize::try_from(child).expect("icon child exceeds usize");
        if d.node_kind[child_index] == slir::K_PATH {
            let path_value = crate::value::decode_active(
                d,
                st.theme_index,
                slir::base_attr(d, child, slir::A_D),
            );
            if path_value.tag == slir::T_PATH_REF {
                let (bg_kind, bg) =
                    icon_paint(d, st, child, slir::A_BG, rule.color_kind, rule.color);
                let (stroke_kind, stroke) =
                    icon_paint(d, st, child, slir::A_STROKE, rule.color_kind, rule.color);
                let stroke_w = crate::value::num_of(
                    &crate::value::decode_active(
                        d,
                        st.theme_index,
                        slir::base_attr(d, child, slir::A_STROKE_W),
                    ),
                    1.0,
                );
                let dash = crate::value::decode_active(
                    d,
                    st.theme_index,
                    slir::base_attr(d, child, slir::A_STROKE_DASH),
                );
                let has_dash = dash.tag == slir::T_TUPLE && dash.ln >= 2;
                let opacity = crate::value::num_of(
                    &crate::value::decode_active(
                        d,
                        st.theme_index,
                        slir::base_attr(d, child, slir::A_OPACITY),
                    ),
                    1.0,
                );
                fr.ops.push(FrameOp::PathDraw(OpPath {
                    node,
                    dx: x,
                    dy: y,
                    path: i32::try_from(path_value.h).expect("path index exceeds i32"),
                    bg_kind,
                    bg,
                    stroke_kind,
                    stroke,
                    stroke_w,
                    dash_on: if has_dash {
                        crate::value::tuple_at(d, &dash, 0)
                    } else {
                        0.0
                    },
                    dash_off: if has_dash {
                        crate::value::tuple_at(d, &dash, 1)
                    } else {
                        0.0
                    },
                    has_dash,
                    opacity,
                }));
            }
        }
        child = d.node_next[child_index];
    }
    fr.ops.push(FrameOp::ScalePop);
}
#[allow(clippy::too_many_arguments)]
fn push_scrollbar_axis(
    fr: &mut Frame,
    rule: &RStyle,
    node: u32,
    axis: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    content: f64,
    offset: f64,
) {
    let horizontal = if axis == 0 { rule.is_row } else { !rule.is_row };
    let viewport = if horizontal { w } else { h };
    let show = rule.scrollbar == 2 || rule.scrollbar == 1 && content > viewport;
    if !show || viewport <= 0.0 || rule.scrollbar_w <= 0.0 {
        return;
    }

    let scrollbar_width = rule.scrollbar_w;
    let (track_x, track_y, track_width, track_height) = if horizontal {
        (x, y + h - scrollbar_width - 2.0, w, scrollbar_width)
    } else {
        (x + w - scrollbar_width - 2.0, y, scrollbar_width, h)
    };
    push_solid_rect(
        fr,
        node,
        track_x,
        track_y,
        track_width,
        track_height,
        rule.scrollbar_bg,
    );

    let thumb_size = if content > 0.0 {
        viewport.min((viewport * viewport / content).max(16.0))
    } else {
        viewport
    };
    let maximum_offset = 0.0_f64.max(content - viewport);
    let thumb_position = if maximum_offset > 0.0 {
        offset / maximum_offset * (viewport - thumb_size)
    } else {
        0.0
    };
    if horizontal {
        push_solid_rect(
            fr,
            node,
            x + thumb_position,
            track_y,
            thumb_size,
            scrollbar_width,
            rule.scrollbar_fg,
        );
    } else {
        push_solid_rect(
            fr,
            node,
            track_x,
            y + thumb_position,
            scrollbar_width,
            thumb_size,
            rule.scrollbar_fg,
        );
    }
}

/// Returns whether `node` is the editable text node of a currently active
/// `field=` binder.
///
/// Activity follows signal resolution: a binder authored inside an inactive
/// `when` patch does not exist for paint, so its text renders as plain text
/// (no editor clip, scroll offset, selection band, or caret).
pub fn is_field(d: &slir::Doc, st: &St, node: u32) -> bool {
    dispatch::sig_of(d, st, node, dispatch::TR_CHANGE) >= 0
}

/// Appends uncovered-glyph codepoint runs for the string at `str_ref`.
///
/// Returns `(uncov_off, uncov_len)` for [`OpText`]: `uncov_len` counts the
/// half-open `[start, end)` codepoint ranges appended to [`Frame::uncovered`]
/// as flat pairs. A grapheme cluster is uncovered when any of its codepoints
/// requires a glyph the font's cmap does not map; adjacent uncovered clusters
/// coalesce into one run. Fallback drivers paint these runs themselves at the
/// kernel-charged replacement advances.
fn push_uncovered_runs(d: &slir::Doc, fr: &mut Frame, font: i32, str_ref: i32) -> (i32, i32) {
    if font < 0 || str_ref < 0 {
        return (0, 0);
    }
    let Frame {
        strings, uncovered, ..
    } = fr;
    let text = strings[index(str_ref)].as_str();
    let covered = |cp: u32| !graphemes::requires_glyph(cp) || slir::font_gid(d, font, cp) != 0;
    if text.chars().map(u32::from).all(covered) {
        return (0, 0);
    }
    let cps: Vec<u32> = text.chars().map(u32::from).collect();
    let mut bounds = Vec::new();
    graphemes::boundaries(text, &mut bounds);
    let off = count(uncovered.len());
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if cps[index(start)..index(end)].iter().all(|&cp| covered(cp)) {
            continue;
        }
        let start = u32::from_ne_bytes(start.to_ne_bytes());
        let end = u32::from_ne_bytes(end.to_ne_bytes());
        if uncovered.len() > index(off) && *uncovered.last().expect("run pool is nonempty") == start
        {
            *uncovered.last_mut().expect("run pool is nonempty") = end;
        } else {
            uncovered.extend([start, end]);
        }
    }
    (off, count(uncovered.len()).wrapping_sub(off) / 2)
}

#[derive(Clone, Copy)]
struct RotationFrame {
    deg: f64,
    cx: f64,
    cy: f64,
}

#[derive(Clone, Copy)]
struct WalkContext {
    parent_ix: i32,
    parent_inert: bool,
    rotation: Option<RotationFrame>,
}

fn mark_authored_order(l: &Lay, pi: i32, next: &mut u32, order: &mut [u32]) {
    let placement = index(pi);
    if l.p_skip[placement] {
        return;
    }
    if l.p_rot[placement] >= 0 {
        mark_authored_order(l, l.p_rot[placement], next, order);
        return;
    }

    order[placement] = *next;
    *next = (*next)
        .checked_add(1)
        .expect("materialized scene exceeds u32::MAX entries");
    let first_child = l.p_child_off[placement];
    let child_end = first_child.wrapping_add(l.p_child_len[placement]);
    for child_pool_index in first_child..child_end {
        mark_authored_order(l, l.child_pool[index(child_pool_index)], next, order);
    }
}

/// Flattens one placed node and its descendants into `fr`.
///
/// The positional context and inherited state are kept as individual
/// parameters for API compatibility; recursive traversal uses a structured
/// context internally.
#[allow(clippy::too_many_arguments)] // Public traversal state is kept explicit for cross-crate API compatibility.
pub fn walk(
    d: &slir::Doc,
    st: &St,
    l: &Lay,
    ds: &DState,
    ms: &MSt,
    fr: &mut Frame,
    pi: i32,
    ox: f64,
    oy: f64,
    parent_ix: i32,
    parent_inert: bool,
    has_rot: bool,
    in_deg: f64,
    in_cx: f64,
    in_cy: f64,
) {
    let rotation = has_rot.then_some(RotationFrame {
        deg: in_deg,
        cx: in_cx,
        cy: in_cy,
    });
    let mut authored_order = core::mem::take(&mut fr.order_scratch);
    authored_order.clear();
    authored_order.resize(l.p_node.len(), u32::MAX);
    let mut next = 0;
    mark_authored_order(l, pi, &mut next, &mut authored_order);
    walk_node(
        d,
        st,
        l,
        ds,
        ms,
        fr,
        pi,
        ox,
        oy,
        &authored_order,
        WalkContext {
            parent_ix,
            parent_inert,
            rotation,
        },
    );
    fr.order_scratch = authored_order;
}

#[allow(clippy::too_many_arguments)] // The recursive kernel already groups all inherited traversal state.
fn walk_node(
    d: &slir::Doc,
    st: &St,
    l: &Lay,
    ds: &DState,
    ms: &MSt,
    fr: &mut Frame,
    pi: i32,
    ox: f64,
    oy: f64,
    authored_order: &[u32],
    context: WalkContext,
) {
    let pi = index(pi);
    if l.p_skip[pi] {
        return;
    }
    let ri = index(l.p_ri[pi]);
    let rule = &st.rs[ri];
    let node = l.p_node[pi];
    let x = ox + l.p_x[pi];
    let y = oy + l.p_y[pi];
    let w = l.p_w[pi];
    let h = l.p_h[pi];
    let radius = rule.radius;

    // A quarter-turn payload is centered in the rotated bounding box. The
    // outer node has no scene entry because its payload owns the rotation.
    if l.p_rot[pi] >= 0 {
        let inner = index(l.p_rot[pi]);
        let rotation = RotationFrame {
            deg: rule.rotate,
            cx: x + w / 2.0,
            cy: y + h / 2.0,
        };
        fr.ops.push(FrameOp::RotatePush(OpRotate {
            cx: rotation.cx,
            cy: rotation.cy,
            deg: rotation.deg,
        }));
        walk_node(
            d,
            st,
            l,
            ds,
            ms,
            fr,
            inner
                .try_into()
                .expect("placed node index exceeds i32 capacity"),
            x + (w - l.p_w[inner]) / 2.0,
            y + (h - l.p_h[inner]) / 2.0,
            authored_order,
            WalkContext {
                rotation: Some(rotation),
                ..context
            },
        );
        fr.ops.push(FrameOp::RotatePop);
        return;
    }

    let rotation = context.rotation.or_else(|| {
        (rule.rotate != 0.0).then_some(RotationFrame {
            deg: rule.rotate,
            cx: x + w / 2.0,
            cy: y + h / 2.0,
        })
    });
    let (rot_deg, rot_cx, rot_cy) = rotation
        .map(|rotation| (rotation.deg, rotation.cx, rotation.cy))
        .unwrap_or((0.0, 0.0, 0.0));

    let kind = rule.kind;
    let child_count = l.p_child_len[pi];
    let child_clip = l.p_clip[pi] && child_count > 0;
    let field_clip = kind == slir::K_TEXT && is_field(d, st, node);
    let field_scroll_x = if field_clip {
        style::field_scroll_x(st, node)
    } else {
        0.0
    };
    let clips = child_clip || field_clip;
    let mut flags = if clips {
        rule.flags | slir::F_CLIP
    } else {
        rule.flags & !slir::F_CLIP
    };
    let inert = context.parent_inert || rule.flags & slir::F_INERT != 0;
    if inert {
        flags |= slir::F_INERT;
    }

    fr.scene.push(SceneNode {
        node,
        parent_ix: context.parent_ix,
        kind,
        x,
        y,
        w,
        h,
        radius,
        rot_deg,
        rot_cx,
        rot_cy,
        flags,
        content_main: 0.0,
        scroll_off: style::scroll_get(st, node),
        scroll_cross: style::scroll_cross_get(st, node),
        content_cross: 0.0,
        is_row: rule.is_row,
        src_line: rule.line,
        authored_order: authored_order[pi],
        role: rule.role,
        label: rule.label,
        desc: rule.desc,
        checked: rule.checked,
        expanded: rule.expanded,
        selected: rule.selected,
        active_descendant: rule.active_descendant,
        controls: rule.controls,
        value_now: rule.value_now,
        value_min: rule.value_min,
        value_max: rule.value_max,
        value_text: rule.value_text,
        modal: rule.modal,
        live: rule.live,
        live_atomic: rule.live_atomic,
        level: rule.level,
        pos_in_set: rule.pos_in_set,
        set_size: rule.set_size,
        disabled: style::node_disabled(st, node),
        focused: ds.fs.focus == node,
        editable: kind == slir::K_TEXT && is_field(d, st, node),
    });
    let scene_index = fr.scene.len() - 1;
    let scene_index_i32 = count(scene_index);

    let native_group = ms
        .lift_node
        .get(usize::try_from(node).expect("node index exceeds usize"))
        .copied()
        .unwrap_or(false);
    let grouped = native_group || rule.opacity < 1.0 || rule.blur > 0.0 || rule.mask_kind != 0;
    if grouped {
        fr.ops.push(FrameOp::GroupPush(OpGroup {
            node,
            opacity: rule.opacity,
            blur: rule.blur,
            mask_kind: rule.mask_kind,
            mask: rule.mask_h,
            mx: x,
            my: y,
            mw: w,
            mh: h,
        }));
    }

    let arbitrarily_rotated = rule.rotate != 0.0;
    if arbitrarily_rotated {
        fr.ops.push(FrameOp::RotatePush(OpRotate {
            cx: x + w / 2.0,
            cy: y + h / 2.0,
            deg: rule.rotate,
        }));
    }

    // Ink-only transforms wrap the node about its center: rotation outermost,
    // then scale, then tilt (SPEC §7).
    let scaled = rule.scale_x != 1.0 || rule.scale_y != 1.0;
    if scaled {
        fr.ops.push(FrameOp::ScalePush(OpScale {
            cx: x + w / 2.0,
            cy: y + h / 2.0,
            sx: rule.scale_x,
            sy: rule.scale_y,
        }));
    }
    let tilted = rule.has_tilt;
    if tilted {
        fr.ops.push(FrameOp::TiltPush(OpTilt {
            cx: x + w / 2.0,
            cy: y + h / 2.0,
            rx: rule.tilt_rx,
            ry: rule.tilt_ry,
            depth: rule.tilt_depth,
        }));
    }

    if rule.has_backdrop {
        fr.ops.push(FrameOp::Backdrop(OpBackdrop {
            x,
            y,
            w,
            h,
            radius,
            blur: rule.backdrop_blur,
            saturate: rule.backdrop_sat,
            brightness: rule.backdrop_bright,
            smooth: rule.smooth,
            mask_kind: rule.bmask_kind,
            mask: rule.bmask_h,
        }));
    }

    if kind == slir::K_IMG {
        if rule.bg_kind != 0 {
            let mut background = unstyled_rect(node, x, y, w, h, radius);
            background.bg_kind = rule.bg_kind;
            background.bg = rule.bg_h;
            fr.ops.push(FrameOp::Rect(background));
        }
        if rule.img >= 0 {
            fr.ops.push(FrameOp::Image(OpImage {
                node,
                x,
                y,
                w,
                h,
                img: rule.img,
                fit: rule.fit,
                radius,
                opacity: 1.0,
                smooth: rule.smooth,
            }));
        }
    } else if kind == slir::K_PATH {
        if let Some(path) = frame_path_ref(d, st, fr, rule.path) {
            fr.ops.push(FrameOp::PathDraw(OpPath {
                node,
                dx: x,
                dy: y,
                path,
                bg_kind: rule.bg_kind,
                bg: rule.bg_h,
                stroke_kind: rule.stroke_kind,
                stroke: rule.stroke_h,
                stroke_w: rule.stroke_w,
                dash_on: rule.dash_on,
                dash_off: rule.dash_off,
                has_dash: rule.has_dash,
                opacity: 1.0,
            }));
        }
    } else if kind == slir::K_ICON {
        emit_icon(d, st, fr, rule, node, (x, y), w.min(h));
    } else if rule.bg_kind != 0
        || rule.stroke_kind != 0
        || rule.shadow_len > 0
        || rule.grain_amount > 0.0
        || ms
            .lift_bg
            .get(usize::try_from(node).expect("node index exceeds usize"))
            .copied()
            .unwrap_or(false)
    {
        fr.ops
            .push(FrameOp::Rect(styled_rect(rule, node, x, y, w, h, radius)));
    }

    let padding_top = rule.pad_t;
    let padding_right = rule.pad_r;
    let padding_bottom = rule.pad_b;
    let padding_left = rule.pad_l;

    if field_clip {
        fr.ops
            .push(FrameOp::ClipPush(clip(x, y, w, h, radius, rule.smooth)));
    }

    if (kind == slir::K_TEXT || kind == slir::K_SPAN) && l.p_tl[pi] >= 0 {
        let text_layout = &l.tls[index(l.p_tl[pi])];
        let content_width = w - padding_left - padding_right;
        let alignment = text_alignment_factor(rule.talign);
        let mut font_weight = truncate_u32(rule.weight);
        if rule.font >= 0 {
            font_weight = d.font_weight[index(rule.font)];
        }

        // The focused field's selection paints as one half-alpha band of
        // the text color per visual line, before the glyphs (SPEC §15.6).
        if field_clip && ds.fs.focus == node {
            let edit_index = dispatch::ed_ix(ds, node);
            if edit_index >= 0 {
                let es = &ds.ed[index(edit_index)];
                let (sel_lo, sel_hi) = (edit::sel_lo(es), edit::sel_hi(es));
                if sel_hi > sel_lo {
                    let text = edit::display_str(es);
                    // RGBA words are little-endian [r, g, b, a]: keep the
                    // text rgb, set alpha to exactly half (0x80).
                    let band = (solid_text_rgba(d, rule) & 0x00FF_FFFF) | 0x8000_0000;
                    for line in 0..text_layout.src_ls.len() {
                        let line_start = text_layout.src_ls[line];
                        let overlap_lo = sel_lo.max(line_start);
                        let overlap_hi = sel_hi.min(text_layout.src_le[line]);
                        if overlap_hi <= overlap_lo {
                            continue;
                        }
                        let measure = |to: i32| {
                            textm::str_slice_w(
                                d,
                                rule.font,
                                rule.size,
                                rule.tracking,
                                &text,
                                line_start,
                                to,
                            )
                        };
                        let origin = x
                            + padding_left
                            + (content_width - text_layout.line_w[line]) * alignment
                            - field_scroll_x;
                        let band_x = origin + measure(overlap_lo);
                        push_solid_rect(
                            fr,
                            node,
                            band_x,
                            y + padding_top + f64::from(count(line)) * text_layout.line_h,
                            origin + measure(overlap_hi) - band_x,
                            text_layout.line_h,
                            band,
                        );
                    }
                }
            }
        }

        for line in 0..count(text_layout.ls.len()) {
            let line_index = index(line);
            let start = text_layout.ls[line_index];
            let end = text_layout.le[line_index];
            if end <= start {
                continue;
            }
            let string_ref = push_str_slice(fr, &text_layout.chars, start, end);
            let measured_width = text_layout.line_w[line_index];
            let (uncov_off, uncov_len) = push_uncovered_runs(d, fr, rule.font, string_ref);
            let mut text_op = OpText {
                node,
                x: x + padding_left + (content_width - measured_width) * alignment - field_scroll_x,
                y_baseline: y
                    + padding_top
                    + text_layout.ascent
                    + f64::from(line) * text_layout.line_h,
                str_ref: string_ref,
                measured_w: measured_width,
                font: rule.font,
                size: rule.size,
                weight: font_weight,
                tracking: rule.tracking,
                color: rule.color,
                opacity: 1.0,
                strike: rule.strike,
                color_kind: rule.color_kind,
                gx: 0.0,
                gy: 0.0,
                gw: 0.0,
                gh: 0.0,
                uncov_off,
                uncov_len,
            };
            if rule.color_kind == 2 {
                // The gradient spans the node's content box so every line of
                // the run samples one continuous ramp.
                text_op.gx = x + padding_left;
                text_op.gy = y + padding_top;
                text_op.gw = content_width;
                text_op.gh = h - padding_top - padding_bottom;
            }
            fr.ops.push(FrameOp::Text(text_op));
        }
    }

    if field_clip {
        fr.ops.push(FrameOp::ClipPop);
    }

    if kind == slir::K_PARA && l.p_para[pi] >= 0 {
        let paragraph = index(l.p_para[pi]);
        let content_width = w - padding_left - padding_right;
        let alignment = text_alignment_factor(rule.talign);
        let mut baseline_y = y + padding_top;
        let first_line = l.para_line_off[paragraph];
        let line_end = first_line.wrapping_add(l.para_line_len[paragraph]);

        for line in first_line..line_end {
            let line_index = index(line);
            let leading = (content_width - l.pl_w[line_index]) * alignment;
            let first_segment = l.pl_seg_off[line_index];
            let segment_end = first_segment.wrapping_add(l.pl_seg_len[line_index]);
            for segment in first_segment..segment_end {
                let segment_index = index(segment);
                let string_ref = push_str_slice(
                    fr,
                    &l.para_chars,
                    l.seg_a[segment_index],
                    l.seg_b[segment_index],
                );
                let font = l.seg_font[segment_index];
                let mut font_weight = truncate_u32(l.seg_weight[segment_index]);
                if font >= 0 {
                    font_weight = d.font_weight[index(font)];
                }
                let seg_kind = l.seg_color_kind[segment_index];
                let (uncov_off, uncov_len) = push_uncovered_runs(d, fr, font, string_ref);
                let mut text_op = OpText {
                    node,
                    x: x + padding_left + leading + l.seg_x[segment_index],
                    y_baseline: baseline_y + l.pl_asc[line_index],
                    str_ref: string_ref,
                    measured_w: l.seg_w[segment_index],
                    font,
                    size: l.seg_size[segment_index],
                    weight: font_weight,
                    tracking: l.seg_tracking[segment_index],
                    color: l.seg_color[segment_index],
                    opacity: 1.0,
                    strike: l.seg_strike[segment_index],
                    color_kind: seg_kind,
                    gx: 0.0,
                    gy: 0.0,
                    gw: 0.0,
                    gh: 0.0,
                    uncov_off,
                    uncov_len,
                };
                if seg_kind == 2 {
                    // Paragraph segments share the paragraph's content box.
                    text_op.gx = x + padding_left;
                    text_op.gy = y + padding_top;
                    text_op.gw = content_width;
                    text_op.gh = h - padding_top - padding_bottom;
                }
                fr.ops.push(FrameOp::Text(text_op));
            }
            baseline_y += l.pl_h[line_index];
        }
    }

    // Child extents include trailing padding so both scroll axes clamp exactly.
    if child_count > 0 {
        let first_child = l.p_child_off[pi];
        let child_end = first_child.wrapping_add(child_count);
        let mut main_extent: f64 = 0.0;
        let mut cross_extent: f64 = 0.0;
        for child_pool_index in first_child..child_end {
            let child = index(l.child_pool[index(child_pool_index)]);
            if l.p_skip[child] || st.rs[index(l.p_ri[child])].has_attach {
                continue;
            }
            if rule.is_row {
                main_extent = main_extent.max(l.p_x[child] + l.p_w[child]);
                cross_extent = cross_extent.max(l.p_y[child] + l.p_h[child]);
            } else {
                main_extent = main_extent.max(l.p_y[child] + l.p_h[child]);
                cross_extent = cross_extent.max(l.p_x[child] + l.p_w[child]);
            }
        }
        fr.scene[scene_index].content_main = main_extent
            + if rule.is_row {
                padding_right
            } else {
                padding_bottom
            };
        fr.scene[scene_index].content_cross = cross_extent
            + if rule.is_row {
                padding_bottom
            } else {
                padding_right
            };
    }
    if let Some((extent, len, _, _)) = list::virtual_metrics(d, &st.lists, node) {
        fr.scene[scene_index].content_main = f64::from(len) * extent;
    }

    if child_clip {
        fr.ops
            .push(FrameOp::ClipPush(clip(x, y, w, h, radius, rule.smooth)));
    }

    // Each active scroll axis independently shifts children inside the clip.
    let main_offset = if rule.flags & slir::F_SCROLL != 0 {
        style::scroll_get(st, node)
    } else {
        0.0
    };
    let cross_offset = if rule.flags & slir::F_SCROLL_CROSS != 0 {
        style::scroll_cross_get(st, node)
    } else {
        0.0
    };
    let (children_x, children_y) = if rule.is_row {
        (x - main_offset, y - cross_offset)
    } else {
        (x - cross_offset, y - main_offset)
    };

    let first_child = l.p_child_off[pi];
    let child_end = first_child.wrapping_add(child_count);
    // Normal children paint first. Sticky children are promoted above siblings.
    for child_pool_index in first_child..child_end {
        if crate::layout::sticky_main_position(
            st,
            l,
            i32::try_from(pi).expect("placed node index exceeds i32"),
            child_pool_index,
        )
        .is_some()
        {
            continue;
        }
        walk_node(
            d,
            st,
            l,
            ds,
            ms,
            fr,
            l.child_pool[index(child_pool_index)],
            children_x,
            children_y,
            authored_order,
            WalkContext {
                parent_ix: scene_index_i32,
                parent_inert: inert,
                rotation: None,
            },
        );
    }

    for child_pool_index in first_child..child_end {
        let Some(painted) = crate::layout::sticky_main_position(
            st,
            l,
            i32::try_from(pi).expect("placed node index exceeds i32"),
            child_pool_index,
        ) else {
            continue;
        };
        let child_pi = l.child_pool[index(child_pool_index)];
        let child = index(child_pi);
        let slot = if rule.is_row {
            l.p_x[child]
        } else {
            l.p_y[child]
        };
        let (sticky_x, sticky_y) = if rule.is_row {
            (x + painted - slot, children_y)
        } else {
            (children_x, y + painted - slot)
        };
        walk_node(
            d,
            st,
            l,
            ds,
            ms,
            fr,
            child_pi,
            sticky_x,
            sticky_y,
            authored_order,
            WalkContext {
                parent_ix: scene_index_i32,
                parent_inert: inert,
                rotation: None,
            },
        );
    }

    let content_main = fr.scene[scene_index].content_main;
    let content_cross = fr.scene[scene_index].content_cross;
    if rule.flags & slir::F_SCROLL != 0 {
        push_scrollbar_axis(fr, rule, node, 0, x, y, w, h, content_main, main_offset);
    }
    if rule.flags & slir::F_SCROLL_CROSS != 0 {
        push_scrollbar_axis(fr, rule, node, 1, x, y, w, h, content_cross, cross_offset);
    }

    if child_clip {
        fr.ops.push(FrameOp::ClipPop);
    }
    if tilted {
        fr.ops.push(FrameOp::TiltPop);
    }
    if scaled {
        fr.ops.push(FrameOp::ScalePop);
    }
    if arbitrarily_rotated {
        fr.ops.push(FrameOp::RotatePop);
    }
    if grouped {
        fr.ops.push(FrameOp::GroupPop);
    }
}

const DRAG_GHOST_OPACITY: f64 = 0.72;

fn append_drag_ghost(d: &slir::Doc, st: &St, l: &Lay, ds: &DState, ms: &MSt, frame: &mut Frame) {
    if !ds.drag_active || ds.drag_source == slir::NONE {
        return;
    }
    let Some(source_scene) = frame
        .scene
        .iter()
        .find(|node| node.node == ds.drag_source && node.flags & slir::F_DRAG_GHOST != 0)
    else {
        return;
    };
    let Some(placement) = l
        .p_node
        .iter()
        .enumerate()
        .rfind(|(index, node)| **node == ds.drag_source && !l.p_skip[*index])
        .map(|(index, _)| index)
    else {
        return;
    };
    let source_w = source_scene.w;
    let source_h = source_scene.h;
    let desired_x = ds.drag_last_x - ds.drag_grab_x;
    let desired_y = ds.drag_last_y - ds.drag_grab_y;
    // Preserve the selected placement's offset from its painted AABB. Quarter
    // turns of non-square nodes have different placement and scene origins.
    let move_x = desired_x - source_scene.x;
    let move_y = desired_y - source_scene.y;
    let scene_len = frame.scene.len();
    frame.ops.push(FrameOp::GroupPush(OpGroup {
        node: slir::NONE,
        opacity: DRAG_GHOST_OPACITY,
        blur: 0.0,
        mask_kind: 0,
        mask: 0,
        mx: desired_x,
        my: desired_y,
        mw: source_w,
        mh: source_h,
    }));
    walk(
        d,
        st,
        l,
        ds,
        ms,
        frame,
        i32::try_from(placement).expect("placed node index exceeds i32"),
        move_x,
        move_y,
        -1,
        false,
        false,
        0.0,
        0.0,
        0.0,
    );
    frame.scene.truncate(scene_len);
    frame.ops.push(FrameOp::GroupPop);
}

/// Lowers the placed tree rooted at `root_pi` into a reusable frame.
pub fn flatten_into(
    d: &slir::Doc,
    st: &St,
    l: &Lay,
    ds: &DState,
    ms: &MSt,
    root_pi: i32,
    frame: &mut Frame,
) {
    let root = index(root_pi);
    frame.clear();
    frame.width = l.p_w[root];
    frame.height = l.p_h[root];
    walk(
        d, st, l, ds, ms, frame, root_pi, 0.0, 0.0, -1, false, false, 0.0, 0.0, 0.0,
    );
    append_drag_ghost(d, st, l, ds, ms, frame);
}

/// Lowers the placed tree rooted at `root_pi` into a newly allocated frame.
pub fn flatten(d: &slir::Doc, st: &St, l: &Lay, ds: &DState, ms: &MSt, root_pi: i32) -> Frame {
    let mut frame = frame_new();
    flatten_into(d, st, l, ds, ms, root_pi, &mut frame);
    frame
}
