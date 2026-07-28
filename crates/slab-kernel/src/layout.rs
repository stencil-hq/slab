//! The normative layout solver: constraints flow down, sizes flow up, and
//! parents place their measured children.
//!
//! Sequential measurement preserves containment by construction. When a node
//! is over-constrained, the deflation ladder reports diagnostics instead of
//! allowing overlap.
//!
//! Unbounded constraints use the finite [`INF`] sentinel. Keep the explicit
//! `!= INF` guards: derived near-sentinel values deliberately remain finite
//! and behave as astronomically large dimensions.

use crate::{slir::Doc, style::St, textm::TextLayout};
use rustc_hash::FxHashMap;

fn idx<T>(value: T) -> usize
where
    usize: TryFrom<T>,
    <usize as TryFrom<T>>::Error: std::fmt::Debug,
{
    usize::try_from(value).expect("nonnegative layout index")
}

fn len_i32<T>(values: &[T]) -> i32 {
    i32::try_from(values.len()).expect("layout pool exceeds i32 capacity")
}

fn truncate_i32(value: f64) -> i32 {
    if value.is_nan() {
        return 0;
    }
    if value >= f64::from(i32::MAX) {
        return i32::MAX;
    }
    if value <= f64::from(i32::MIN) {
        return i32::MIN;
    }

    let bits = value.to_bits();
    let exponent = i32::try_from((bits >> 52) & 0x7ff).expect("f64 exponent fits i32") - 1023;
    if exponent < 0 {
        return 0;
    }
    let significand = (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52);
    let magnitude = if exponent < 52 {
        significand >> u32::try_from(52 - exponent).expect("nonnegative right shift")
    } else {
        significand << u32::try_from(exponent - 52).expect("nonnegative left shift")
    };
    let magnitude = i32::try_from(magnitude).expect("bounded f64 magnitude fits i32");
    if value.is_sign_negative() {
        magnitude.wrapping_neg()
    } else {
        magnitude
    }
}

/// Layout comparisons tolerate half a device-independent pixel.
pub const EPS: f64 = 0.5;

/// Finite sentinel used for an unbounded layout constraint.
pub const INF: f64 = 1.0e30;

/// A placed tree stored as structure-of-arrays pools.
///
/// Each node's children occupy
/// `child_pool[p_child_off..p_child_off + p_child_len]`. Coordinates are
/// relative to the parent border-box origin.
#[derive(Clone, Debug, Default)]
pub struct Lay {
    pub p_node: Vec<u32>,
    /// Index of the node's resolved style in [`St::rs`].
    pub p_ri: Vec<i32>,
    pub p_x: Vec<f64>,
    pub p_y: Vec<f64>,
    pub p_w: Vec<f64>,
    pub p_h: Vec<f64>,
    pub p_base: Vec<f64>,
    pub p_has_base: Vec<bool>,
    pub p_clip: Vec<bool>,
    /// Placed index of a quarter-turn payload, or `-1` when absent.
    pub p_rot: Vec<i32>,
    /// Placements omitted from paint, scene export, and hit testing.
    pub p_skip: Vec<bool>,
    pub p_child_off: Vec<i32>,
    pub p_child_len: Vec<i32>,
    /// Index in [`Lay::tls`], or `-1` when this placement has no text layout.
    pub p_tl: Vec<i32>,
    /// Paragraph block index in the paragraph line pools, or `-1`.
    pub p_para: Vec<i32>,
    pub child_pool: Vec<i32>,
    pub tls: Vec<std::rc::Rc<TextLayout>>,
    /// Per-paragraph ranges into the line pools; each line stores a segment range.
    pub para_line_off: Vec<i32>,
    pub para_line_len: Vec<i32>,
    pub pl_h: Vec<f64>,
    pub pl_asc: Vec<f64>,
    pub pl_w: Vec<f64>,
    pub pl_seg_off: Vec<i32>,
    pub pl_seg_len: Vec<i32>,
    pub seg_x: Vec<f64>,
    /// Start of the segment's codepoint slice in [`Lay::para_chars`].
    pub seg_a: Vec<i32>,
    pub seg_b: Vec<i32>,
    pub seg_w: Vec<f64>,
    pub seg_font: Vec<i32>,
    pub seg_size: Vec<f64>,
    pub seg_weight: Vec<f64>,
    pub seg_tracking: Vec<f64>,
    pub seg_strike: Vec<bool>,
    pub seg_color: Vec<u32>,
    /// 1 when the segment color is packed RGBA, 2 when it is a gradient handle.
    pub seg_color_kind: Vec<u32>,
    pub para_chars: Vec<u32>,
    /// Reusable measure-pass scratch buffers, taken on container entry and
    /// returned cleared on exit; recursion depth bounds each pool's size.
    scratch_u32: Vec<Vec<u32>>,
    scratch_i32: Vec<Vec<i32>>,
    scratch_f64: Vec<Vec<f64>>,
    /// Retained [`place_attached`] scratch, cleared and refilled per call.
    attach: AttachScratch,
}

/// Creates an empty set of layout pools.
pub fn lay_new() -> Lay {
    Lay::default()
}

/// Clears every layout pool while retaining its allocated capacity.
pub fn lay_reset(l: &mut Lay) {
    l.p_node.clear();
    l.p_ri.clear();
    l.p_x.clear();
    l.p_y.clear();
    l.p_w.clear();
    l.p_h.clear();
    l.p_base.clear();
    l.p_has_base.clear();
    l.p_clip.clear();
    l.p_rot.clear();
    l.p_skip.clear();
    l.p_child_off.clear();
    l.p_child_len.clear();
    l.p_tl.clear();
    l.p_para.clear();
    l.child_pool.clear();
    l.tls.clear();
    l.para_line_off.clear();
    l.para_line_len.clear();
    l.pl_h.clear();
    l.pl_asc.clear();
    l.pl_w.clear();
    l.pl_seg_off.clear();
    l.pl_seg_len.clear();
    l.seg_x.clear();
    l.seg_a.clear();
    l.seg_b.clear();
    l.seg_w.clear();
    l.seg_font.clear();
    l.seg_size.clear();
    l.seg_tracking.clear();
    l.seg_strike.clear();
    l.seg_weight.clear();
    l.seg_color.clear();
    l.seg_color_kind.clear();
    l.para_chars.clear();
}

fn take_u32(l: &mut Lay) -> Vec<u32> {
    l.scratch_u32.pop().unwrap_or_default()
}

fn give_u32(l: &mut Lay, mut buf: Vec<u32>) {
    buf.clear();
    l.scratch_u32.push(buf);
}

fn take_i32(l: &mut Lay) -> Vec<i32> {
    l.scratch_i32.pop().unwrap_or_default()
}

fn give_i32(l: &mut Lay, mut buf: Vec<i32>) {
    buf.clear();
    l.scratch_i32.push(buf);
}

fn take_f64(l: &mut Lay) -> Vec<f64> {
    l.scratch_f64.pop().unwrap_or_default()
}

fn give_f64(l: &mut Lay, mut buf: Vec<f64>) {
    buf.clear();
    l.scratch_f64.push(buf);
}

/// Returns the text-layout pool index for the last placed occurrence of `node`.
///
/// Re-measurement may leave earlier speculative placements in the pool, so
/// callers must use the last match.
pub fn text_layout_ix(l: &Lay, node: u32) -> i32 {
    let mut found = -1;
    for (placed_node, text_layout) in l.p_node.iter().zip(&l.p_tl) {
        if *placed_node == node && *text_layout >= 0 {
            found = *text_layout;
        }
    }
    found
}

/// Appends a zeroed placement and returns its pool index.
pub fn p_new(l: &mut Lay, node: u32, ri: i32) -> i32 {
    l.p_node.push(node);
    l.p_ri.push(ri);
    l.p_x.push(0.0f64);
    l.p_y.push(0.0f64);
    l.p_w.push(0.0f64);
    l.p_h.push(0.0f64);
    l.p_base.push(0.0f64);
    l.p_has_base.push(false);
    l.p_clip.push(false);
    l.p_rot.push(-1);
    l.p_skip.push(false);
    l.p_child_off.push(0);
    l.p_child_len.push(0);
    l.p_tl.push(-1);
    l.p_para.push(-1);
    len_i32(&l.p_node).wrapping_sub(1)
}
/// Returns a sticky child's painted main coordinate relative to its parent.
///
/// Non-sticky children and children without a main-scroll parent return
/// `None`. A following sticky sibling pushes the current child before it.
pub fn sticky_main_position(
    st: &St,
    l: &Lay,
    parent_pi: i32,
    child_pool_index: i32,
) -> Option<f64> {
    let parent = idx(parent_pi);
    let parent_rule = &st.rs[idx(l.p_ri[parent])];
    if parent_rule.flags & crate::slir::F_SCROLL == 0 {
        return None;
    }
    let child_pi = l.child_pool[idx(child_pool_index)];
    let child = idx(child_pi);
    let child_rule = &st.rs[idx(l.p_ri[child])];
    if l.p_skip[child] || child_rule.flags & crate::slir::F_STICKY == 0 {
        return None;
    }

    let slot = if parent_rule.is_row {
        l.p_x[child]
    } else {
        l.p_y[child]
    };
    let extent = if parent_rule.is_row {
        l.p_w[child]
    } else {
        l.p_h[child]
    };
    let scroll = crate::style::scroll_get(st, l.p_node[parent]);
    let mut painted = (slot - scroll).max(0.0);
    let child_end = l.p_child_off[parent].wrapping_add(l.p_child_len[parent]);
    for following_pool_index in child_pool_index.wrapping_add(1)..child_end {
        let following = idx(l.child_pool[idx(following_pool_index)]);
        let following_rule = &st.rs[idx(l.p_ri[following])];
        if l.p_skip[following] || following_rule.flags & crate::slir::F_STICKY == 0 {
            continue;
        }
        let following_slot = if parent_rule.is_row {
            l.p_x[following]
        } else {
            l.p_y[following]
        };
        painted = painted.min(following_slot - scroll - extent);
        break;
    }
    Some(painted)
}

/// Constraints propagated from a parent while measuring a node.
#[derive(Clone, Debug)]
pub struct Cons {
    pub min_w: f64,
    pub max_w: f64,
    pub min_h: f64,
    pub max_h: f64,
    pub pct_w: f64,
    pub pct_h: f64,
    pub has_pw: bool,
    pub has_ph: bool,
}

/// Inheritable text properties propagated through the placed tree.
#[derive(Clone, Debug)]
pub struct Inh {
    pub color: u32,
    /// 1 when `color` is packed RGBA, 2 when it is a gradient handle.
    pub color_kind: u32,
    pub fam: u32,
    pub size: f64,
    pub weight: f64,
    pub leading: f64,
    pub tracking: f64,
    pub strike: bool,
}

/// Returns the inherited text style used at the document root.
pub fn inh_root() -> Inh {
    Inh {
        color: 0x111111FF,
        color_kind: 1,
        fam: 0,
        size: 14.0,
        weight: 400.0,
        leading: 1.4,
        tracking: 0.0,
        strike: false,
    }
}

/// Extracts the inheritable text properties from a resolved style.
pub fn inh_of(st: &St, ri: i32) -> Inh {
    let style = &st.rs[idx(ri)];
    Inh {
        color: style.color,
        color_kind: style.color_kind,
        fam: style.fam,
        size: style.size,
        weight: style.weight,
        leading: style.leading,
        tracking: style.tracking,
        strike: style.strike,
    }
}

/// A determinate authored length, or no value for content-driven sizing.
#[derive(Clone, Debug)]
pub struct Own {
    pub has: bool,
    pub v: f64,
}

/// Resolves an authored size against the available constraint and percent base.
pub fn resolve_len(
    st: &mut St,
    kind: u32,
    v: f64,
    cmax: f64,
    has_pct: bool,
    pct_base: f64,
    line: u32,
) -> Own {
    match kind {
        crate::style::S_FIXED => Own { has: true, v },
        crate::style::S_PCT if has_pct => Own {
            has: true,
            v: (v / 100.0) * pct_base,
        },
        crate::style::S_PCT => {
            crate::style::warn(
                st,
                "pct-unbounded",
                "% against an unbounded or hug axis behaves as hug (give the parent a determinate size or use fill)",
                line,
            );
            Own { has: false, v: 0.0 }
        }
        crate::style::S_FILL if cmax != INF => Own { has: true, v: cmax },
        _ => Own { has: false, v: 0.0 },
    }
}

/// Clamps a measured size to constraints and authored bounds.
///
/// `amax == INF` means there is no authored maximum. Authored bounds apply
/// last, making them explicit escape valves from parent constraints.
pub fn clamp(v: f64, cmin: f64, cmax: f64, amin: f64, amax: f64) -> f64 {
    let constrained = cmin.max(v.min(cmax));
    let below_author_max = if amax == INF {
        constrained
    } else {
        constrained.min(amax)
    };
    below_author_max.max(amin)
}

/// Emits a diagnostic when a fixed authored size had to be squeezed.
pub fn squeeze_check(
    d: &Doc,
    st: &mut St,
    node: u32,
    spec_kind: u32,
    spec_v: f64,
    fin: f64,
    axis_w: bool,
) {
    if spec_kind == crate::style::S_FIXED && fin < spec_v - EPS {
        let axis = if axis_w { "w" } else { "h" };
        let msg = crate::rt::str_concat(
            &crate::rt::str_concat(
                &crate::rt::str_concat(
                    &crate::rt::str_concat(&crate::style::label(d, st, node), " asked "),
                    axis,
                ),
                &crate::rt::str_concat("=", &crate::value::fmt3(spec_v)),
            ),
            &crate::rt::str_concat(
                &crate::rt::str_concat(" but got ", &crate::value::fmt3(fin)),
                &crate::rt::str_concat(
                    &crate::rt::str_concat(" (short ", &crate::value::fmt3(spec_v - fin)),
                    ")",
                ),
            ),
        );
        let base = crate::list::base(&st.lists, d, node);
        crate::style::warn(st, "squeeze", &msg, d.node_line[idx(base)]);
    }
}

/// Wraps a rotation angle into `[0, 360)` without changing floating-point semantics.
pub fn wrap360(v: f64) -> f64 {
    v - (v / 360.0).floor() * 360.0
}

/// Measures `node` against `cn`.
///
/// `swap` measures a quarter-turn payload by swapping the resolved style axes
/// and skipping rotation handling. `hug_w` and `hug_h` demote the corresponding
/// authored size to content-driven sizing.
#[allow(clippy::too_many_arguments)] // Layout traversal carries the full inherited solve context.
pub fn measure(
    d: &Doc,
    st: &mut St,
    l: &mut Lay,
    node: u32,
    cn: &Cons,
    pk: u32,
    pir: bool,
    inh: &Inh,
    swap: bool,
    hug_w: bool,
    hug_h: bool,
) -> i32 {
    // Width-conditioned patches apply against the incoming constraint.
    crate::style::set_patch_flags(d, st, node, cn.max_w, cn.max_h);
    let ri = crate::style::build_rstyle(
        d,
        st,
        node,
        pk,
        pir,
        inh.color,
        inh.color_kind,
        inh.fam,
        inh.size,
        inh.weight,
        inh.leading,
        inh.tracking,
        inh.strike,
    );
    if swap {
        let tk = st.rs[idx(ri)].w_kind;
        let tv = st.rs[idx(ri)].w_v;
        st.rs[idx(ri)].w_kind = st.rs[idx(ri)].h_kind;
        st.rs[idx(ri)].w_v = st.rs[idx(ri)].h_v;
        st.rs[idx(ri)].h_kind = tk;
        st.rs[idx(ri)].h_v = tv;
        let tmin = st.rs[idx(ri)].min_w;
        let tmax = st.rs[idx(ri)].max_w;
        st.rs[idx(ri)].min_w = st.rs[idx(ri)].min_h;
        st.rs[idx(ri)].max_w = st.rs[idx(ri)].max_h;
        st.rs[idx(ri)].min_h = tmin;
        st.rs[idx(ri)].max_h = tmax;
        // The payload paints through the outer rotation and must not rotate again.
        st.rs[idx(ri)].rotate = 0.0f64;
    }
    if hug_w {
        st.rs[idx(ri)].w_kind = crate::style::S_HUG;
    }
    if hug_h {
        st.rs[idx(ri)].h_kind = crate::style::S_HUG;
    }
    if !swap {
        // Quarter turns measure against swapped constraints and occupy their
        // rotated bounding box.
        let rot = wrap360(st.rs[idx(ri)].rotate);
        if (rot - 90.0).abs() < 0.5 || (rot - 270.0).abs() < 0.5 {
            let swapped = Cons {
                min_w: cn.min_h,
                max_w: cn.max_h,
                min_h: cn.min_w,
                max_h: cn.max_w,
                pct_w: cn.pct_h,
                pct_h: cn.pct_w,
                has_pw: cn.has_ph,
                has_ph: cn.has_pw,
            };
            let ip = measure(d, st, l, node, &swapped, pk, pir, inh, true, hug_h, hug_w);
            let pi = p_new(l, node, ri);
            l.p_w[idx(pi)] = l.p_h[idx(ip)];
            l.p_h[idx(pi)] = l.p_w[idx(ip)];
            l.p_rot[idx(pi)] = ip;
            return pi;
        }
    }
    let kind = st.rs[idx(ri)].kind;
    match kind {
        crate::slir::K_ROW
        | crate::slir::K_COL
        | crate::slir::K_EACH
        | crate::slir::K_GROUP
        | crate::slir::K_RECT
        | crate::slir::K_SPACER
        | crate::slir::K_HOLE
        | crate::slir::K_DIVIDER => box_measure(d, st, l, node, ri, cn),
        crate::slir::K_WRAP => wrap_measure(d, st, l, node, ri, cn),
        crate::slir::K_GRID => grid_measure(d, st, l, node, ri, cn),
        crate::slir::K_STACK => stack_measure(d, st, l, node, ri, cn),
        crate::slir::K_CANVAS => canvas_measure(d, st, l, node, ri, cn),
        crate::slir::K_TEXT | crate::slir::K_SPAN => text_measure(d, st, l, node, ri, cn),
        crate::slir::K_PARA => para_measure(d, st, l, node, ri, cn),
        crate::slir::K_IMG => img_measure(d, st, l, node, ri, cn),
        crate::slir::K_PATH => path_measure(d, st, l, node, ri, cn),
        crate::slir::K_ICON => icon_measure(d, st, l, node, ri, cn),
        _ => {
            crate::style::warn(
                st,
                "attr",
                &crate::rt::str_concat(
                    &crate::rt::str_concat("unhandled node kind '", &crate::style::kind_name(kind)),
                    "'",
                ),
                d.node_line[idx(crate::list::base(&st.lists, d, node))],
            );
            p_new(l, node, ri)
        }
    }
}

/// Measures a child with main/cross constraints mapped onto physical axes.
#[allow(clippy::too_many_arguments)] // Axis mapping requires both physical constraint pairs and inheritance.
pub fn measure_child(
    d: &Doc,
    st: &mut St,
    l: &mut Lay,
    kid: u32,
    row: bool,
    main_min: f64,
    main_max: f64,
    cross_min: f64,
    cross_max: f64,
    pct_w: f64,
    pct_h: f64,
    has_pw: bool,
    has_ph: bool,
    pk: u32,
    pir: bool,
    inh: &Inh,
    hug_w: bool,
    hug_h: bool,
) -> i32 {
    let c = if row {
        Cons {
            min_w: main_min,
            max_w: main_max,
            min_h: cross_min,
            max_h: cross_max,
            pct_w,
            pct_h,
            has_pw,
            has_ph,
        }
    } else {
        Cons {
            min_w: cross_min,
            max_w: cross_max,
            min_h: main_min,
            max_h: main_max,
            pct_w,
            pct_h,
            has_pw,
            has_ph,
        }
    };
    measure(d, st, l, kid, &c, pk, pir, inh, false, hug_w, hug_h)
}

/// Marks overflowing placements as clipped unless bleeding was explicitly allowed.
///
/// An offset applied by a child affects only its ink and does not count as
/// layout overflow. Attached overlays are positioned after layout and therefore
/// do not participate. Explicit clipping and scrolling suppress the warning.
pub fn boundary(d: &Doc, st: &mut St, l: &mut Lay, pi: i32) {
    let ri = l.p_ri[idx(pi)];
    let flags = st.rs[idx(ri)].flags;
    if flags & crate::slir::F_BLEED != 0 {
        return;
    }
    let mut exceeded = false;
    let off = l.p_child_off[idx(pi)];
    for k in off..off.wrapping_add(l.p_child_len[idx(pi)]) {
        let ci = l.child_pool[idx(k)];
        let cri = l.p_ri[idx(ci)];
        if st.rs[idx(cri)].has_attach {
            continue;
        }
        let x = l.p_x[idx(ci)] - st.rs[idx(cri)].offset_x;
        let y = l.p_y[idx(ci)] - st.rs[idx(cri)].offset_y;
        if x < -EPS
            || y < -EPS
            || x + l.p_w[idx(ci)] > l.p_w[idx(pi)] + EPS
            || y + l.p_h[idx(ci)] > l.p_h[idx(pi)] + EPS
        {
            exceeded = true;
        }
    }
    if flags & (crate::slir::F_CLIP | crate::slir::F_SCROLL | crate::slir::F_SCROLL_CROSS) != 0 {
        l.p_clip[idx(pi)] = true;
        return;
    }
    if exceeded {
        l.p_clip[idx(pi)] = true;
        crate::style::warn(
            st,
            "clipped",
            &crate::rt::str_concat(
                &crate::rt::str_concat(
                    "content exceeds ",
                    &crate::style::label(d, st, l.p_node[idx(pi)]),
                ),
                "; clipped (use `bleed` or `clip` to opt in)",
            ),
            st.rs[idx(ri)].line,
        );
    }
}

fn effective_min(d: &Doc, st: &St, node: u32, row: bool) -> f64 {
    crate::style::attr_num(
        d,
        st,
        node,
        if row {
            crate::slir::A_MIN_W
        } else {
            crate::slir::A_MIN_H
        },
        0.0,
    )
}

fn effective_max(d: &Doc, st: &St, node: u32, row: bool) -> f64 {
    crate::style::attr_num(
        d,
        st,
        node,
        if row {
            crate::slir::A_MAX_W
        } else {
            crate::slir::A_MAX_H
        },
        crate::style::INF,
    )
}

/// Resolves the overlay for `kids[index]` when the following child is a divider.
///
/// The next pane's authored minimum and the divider's own footprint are reserved
/// from the current box budget. Pointer drags impose the stronger gesture-start
/// snapshot bound before they write the same overlay.
fn divider_extent_for_child(
    d: &Doc,
    st: &mut St,
    kids: &[u32],
    index: i32,
    row: bool,
    parent_kind: u32,
    remaining: f64,
) -> Option<f64> {
    let divider_index = index.wrapping_add(1);
    let next_index = index.wrapping_add(2);
    if next_index >= len_i32(kids) {
        return None;
    }
    let divider = kids[idx(divider_index)];
    let divider_base = crate::list::base(&st.lists, d, divider);
    if divider_base == crate::slir::NONE || d.node_kind[idx(divider_base)] != crate::slir::K_DIVIDER
    {
        return None;
    }
    let requested = crate::style::divider_get(st, divider)?;
    let divider_size = crate::style::peek_size(d, st, divider, row, parent_kind, row);
    let previous = kids[idx(index)];
    let next = kids[idx(next_index)];
    let min = effective_min(d, st, previous, row);
    let max = effective_max(d, st, previous, row);
    let next_min = effective_min(d, st, next, row);
    let mut budget_max = crate::style::INF;
    if remaining != crate::style::INF {
        let divider_min = effective_min(d, st, divider, row);
        let measured = crate::style::divider_footprint_get(st, divider).unwrap_or({
            if divider_size.kind == crate::style::S_FIXED {
                divider_size.v
            } else {
                divider_min
            }
        });
        let divider_reserve = divider_min.max(measured);
        budget_max = (0.0f64).max(remaining - divider_reserve - next_min);
    }
    let extent = crate::style::divider_clamp(requested, min, max, budget_max);
    crate::style::divider_set(st, divider, extent);
    Some(extent)
}

/// Measures a row or column container and places its children in document order.
pub fn box_measure(d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let row = st.rs[idx(ri)].is_row;
    let kind = st.rs[idx(ri)].kind;
    let pt = st.rs[idx(ri)].pad_t;
    let pr = st.rs[idx(ri)].pad_r;
    let pb = st.rs[idx(ri)].pad_b;
    let pl = st.rs[idx(ri)].pad_l;
    let mut pad_main = pt + pb;
    let mut pad_cross = pl + pr;
    if row {
        pad_main = pl + pr;
        pad_cross = pt + pb;
    }
    let mut spec_main_k = st.rs[idx(ri)].h_kind;
    let mut spec_main_v = st.rs[idx(ri)].h_v;
    let mut spec_cross_k = st.rs[idx(ri)].w_kind;
    let mut spec_cross_v = st.rs[idx(ri)].w_v;
    let mut cmin_main = cn.min_h;
    let mut cmax_main = cn.max_h;
    let mut cmin_cross = cn.min_w;
    let mut cmax_cross = cn.max_w;
    let mut amin_main = st.rs[idx(ri)].min_h;
    let mut amax_main = st.rs[idx(ri)].max_h;
    let mut amin_cross = st.rs[idx(ri)].min_w;
    let mut amax_cross = st.rs[idx(ri)].max_w;
    let mut pct_main = cn.pct_h;
    let mut has_pct_main = cn.has_ph;
    let mut pct_cross = cn.pct_w;
    let mut has_pct_cross = cn.has_pw;
    if row {
        spec_main_k = st.rs[idx(ri)].w_kind;
        spec_main_v = st.rs[idx(ri)].w_v;
        spec_cross_k = st.rs[idx(ri)].h_kind;
        spec_cross_v = st.rs[idx(ri)].h_v;
        cmin_main = cn.min_w;
        cmax_main = cn.max_w;
        cmin_cross = cn.min_h;
        cmax_cross = cn.max_h;
        amin_main = st.rs[idx(ri)].min_w;
        amax_main = st.rs[idx(ri)].max_w;
        amin_cross = st.rs[idx(ri)].min_h;
        amax_cross = st.rs[idx(ri)].max_h;
        pct_main = cn.pct_w;
        has_pct_main = cn.has_pw;
        pct_cross = cn.pct_h;
        has_pct_cross = cn.has_ph;
    }
    let line = st.rs[idx(ri)].line;
    let om = resolve_len(
        st,
        spec_main_k,
        spec_main_v,
        cmax_main,
        has_pct_main,
        pct_main,
        line,
    );
    let oc = resolve_len(
        st,
        spec_cross_k,
        spec_cross_v,
        cmax_cross,
        has_pct_cross,
        pct_cross,
        line,
    );
    let mut own_main = om.v;
    let mut has_main = om.has;
    let mut own_cross = oc.v;
    let mut has_cross = oc.has;
    // A hole reports its host's intrinsic size only while the authored axis
    // remains hug. Fixed, fill, and percent sizing ignore the report.
    if kind == crate::slir::K_HOLE {
        let mut hole_w = 0.0f64;
        let mut hole_h = 0.0f64;
        for h in 0i32..(len_i32(&d.hole_node)) {
            if (d.hole_node[idx(h)] == node) && (h < (len_i32(&st.hole_w))) {
                hole_w = st.hole_w[idx(h)];
                hole_h = st.hole_h[idx(h)];
            }
        }
        if spec_main_k == crate::style::S_HUG {
            if row {
                own_main = hole_w;
            } else {
                own_main = hole_h;
            }
            has_main = true;
        }
        if spec_cross_k == crate::style::S_HUG {
            if row {
                own_cross = hole_h;
            } else {
                own_cross = hole_w;
            }
            has_cross = true;
        }
    }
    if has_main {
        own_main = clamp(own_main, cmin_main, cmax_main, amin_main, amax_main);
    }
    if has_cross {
        own_cross = clamp(own_cross, cmin_cross, cmax_cross, amin_cross, amax_cross);
    }
    let mut budget_main = cmax_main.min(amax_main);
    if has_main {
        budget_main = own_main;
    }
    let mut budget_cross = cmax_cross.min(amax_cross);
    if has_cross {
        budget_cross = own_cross;
    }
    let mut content_main = INF;
    if budget_main != INF {
        content_main = (0.0f64).max(budget_main - pad_main);
    }
    let mut content_cross = INF;
    if budget_cross != INF {
        content_cross = (0.0f64).max(budget_cross - pad_cross);
    }
    if (st.rs[idx(ri)].flags & crate::slir::F_SCROLL) != 0u32 {
        content_main = INF;
    }
    if (st.rs[idx(ri)].flags & crate::slir::F_SCROLL_CROSS) != 0u32 {
        content_cross = INF;
    }
    let mut kids: Vec<u32> = take_u32(l);
    crate::style::children(d, st, node, &mut kids);
    let nk = len_i32(&kids);
    let virtual_metrics = crate::list::virtual_metrics(d, &st.lists, node);
    let gap = st.rs[idx(ri)].gap;
    let gaps = gap * (0.0f64).max(f64::from(nk.wrapping_sub(1i32)));
    let mut remaining = INF;
    if content_main != INF {
        remaining = content_main - gaps;
    }
    let mut kp: Vec<i32> = take_i32(l);
    let mut fills: Vec<i32> = take_i32(l);
    for _i in 0i32..(nk) {
        kp.push(-1i32);
    }
    let hug_cross_container = !has_cross;
    // Percent lengths require a determinate parent axis.
    // `(pw, ph)` are main/cross percentage bases; map them back to width/height.
    let mut pw = 0.0f64;
    let mut has_pw = false;
    if has_main && (content_main != INF) {
        pw = content_main;
        has_pw = true;
    }
    let mut ph = 0.0f64;
    let mut has_ph = false;
    if has_cross && (content_cross != INF) {
        ph = content_cross;
        has_ph = true;
    }
    let mut cpw = ph;
    let mut chas_pw = has_ph;
    let mut cph = pw;
    let mut chas_ph = has_pw;
    if row {
        cpw = pw;
        chas_pw = has_pw;
        cph = ph;
        chas_ph = has_ph;
    }
    // First measure non-fill children in document order.
    let inh = inh_of(st, ri);
    for i in 0i32..(nk) {
        crate::style::reset_wh_patches(d, st, kids[idx(i)]);
        let forced = divider_extent_for_child(d, st, &kids, i, row, kind, (0.0f64).max(remaining));
        let sm = forced.as_ref().map_or_else(
            || crate::style::peek_size(d, st, kids[idx(i)], row, kind, row),
            |extent| crate::style::Size {
                kind: crate::style::S_FIXED,
                v: *extent,
            },
        );
        if sm.kind == crate::style::S_FILL {
            fills.push(i);
        } else {
            kp[idx(i)] = measure_in_box(
                d,
                st,
                l,
                kids[idx(i)],
                row,
                (0.0f64).max(remaining),
                content_cross,
                cpw,
                cph,
                chas_pw,
                chas_ph,
                hug_cross_container,
                forced.is_some(),
                sm.v,
                kind,
                &inh,
                false,
            );
            let mut sz = l.p_h[idx(kp[idx(i)])];
            if row {
                sz = l.p_w[idx(kp[idx(i)])];
            }
            if remaining != INF {
                remaining = (0.0f64).max(remaining - sz);
            }
        }
    }
    // Fill children share leftover space by weight in a second pass.
    if !fills.is_empty() {
        let leftover = remaining;
        if leftover == INF {
            if !st.warned_fill_unbounded {
                crate::style::warn(
                    st,
                    "fill-unbounded",
                    &crate::rt::str_concat(
                        &crate::rt::str_concat(
                            "fill inside unbounded ",
                            &crate::style::label(d, st, node),
                        ),
                        " behaves as hug",
                    ),
                    line,
                );
                st.warned_fill_unbounded = true;
            }
            for j in 0i32..(len_i32(&fills)) {
                let i = fills[idx(j)];
                kp[idx(i)] = measure_in_box(
                    d,
                    st,
                    l,
                    kids[idx(i)],
                    row,
                    INF,
                    content_cross,
                    cpw,
                    cph,
                    chas_pw,
                    chas_ph,
                    hug_cross_container,
                    false,
                    0.0f64,
                    kind,
                    &inh,
                    true,
                );
            }
        } else {
            let mut total_wt = 0.0f64;
            for j in 0i32..(len_i32(&fills)) {
                let s = crate::style::peek_size(d, st, kids[idx(fills[idx(j)])], row, kind, row);
                total_wt += s.v;
            }
            if total_wt == 0.0f64 {
                total_wt = 1.0f64;
            }
            let mut given = 0.0f64;
            for j in 0i32..(len_i32(&fills)) {
                let i = fills[idx(j)];
                let s = crate::style::peek_size(d, st, kids[idx(i)], row, kind, row);
                let mut share = (leftover * s.v) / total_wt;
                if j == len_i32(&fills).wrapping_sub(1i32) {
                    share = leftover - given;
                }
                share = (0.0f64).max(share);
                given += share;
                kp[idx(i)] = measure_in_box(
                    d,
                    st,
                    l,
                    kids[idx(i)],
                    row,
                    0.0f64,
                    content_cross,
                    cpw,
                    cph,
                    chas_pw,
                    chas_ph,
                    hug_cross_container,
                    true,
                    share,
                    kind,
                    &inh,
                    false,
                );
            }
        }
    }
    let mut used = gaps;
    for i in 0i32..(nk) {
        if kp[idx(i)] < 0i32 {
            continue;
        }
        let child_extent = if row {
            l.p_w[idx(kp[idx(i)])]
        } else {
            l.p_h[idx(kp[idx(i)])]
        };
        used += child_extent;
        let child = kids[idx(i)];
        let base = crate::list::base(&st.lists, d, child);
        if base != crate::slir::NONE && d.node_kind[idx(base)] == crate::slir::K_DIVIDER {
            crate::style::divider_footprint_set(st, child, child_extent);
        }
    }
    if let Some((extent, len, _, _)) = virtual_metrics {
        // A virtual EACH represents its entire logical list. Only the retained
        // window has child placements, but its main extent remains exact.
        own_main = f64::from(len) * extent;
        has_main = true;
    }
    if !has_main {
        own_main = clamp(used + pad_main, cmin_main, cmax_main, amin_main, amax_main);
    }
    squeeze_check(d, st, node, spec_main_k, spec_main_v, own_main, row);
    // Rows may align children by their exported text baseline.
    let want_baseline = row && (st.rs[idx(ri)].align == 3i32);
    let mut row_base = 0.0f64;
    if want_baseline {
        for i in 0i32..(nk) {
            if (kp[idx(i)] >= 0i32) && l.p_has_base[idx(kp[idx(i)])] {
                row_base = row_base.max(l.p_base[idx(kp[idx(i)])]);
            }
        }
    }
    if !has_cross {
        let mut needed = 0.0f64;
        for i in 0i32..(nk) {
            if kp[idx(i)] >= 0i32 {
                let mut c = l.p_w[idx(kp[idx(i)])];
                if row {
                    c = l.p_h[idx(kp[idx(i)])];
                }
                if want_baseline && l.p_has_base[idx(kp[idx(i)])] {
                    c += row_base - l.p_base[idx(kp[idx(i)])];
                }
                needed = needed.max(c);
            }
        }
        own_cross = clamp(
            needed + pad_cross,
            cmin_cross,
            cmax_cross,
            amin_cross,
            amax_cross,
        );
    }
    squeeze_check(d, st, node, spec_cross_k, spec_cross_v, own_cross, !row);
    // Cross-fill children are remeasured at the final cross-axis size.
    let final_content_cross = (0.0f64).max(own_cross - pad_cross);
    for i in 0i32..(nk) {
        if kp[idx(i)] < 0i32 {
            continue;
        }
        if want_baseline && l.p_has_base[idx(kp[idx(i)])] {
            continue;
        }
        let sc = crate::style::peek_size(d, st, kids[idx(i)], !row, kind, row);
        let mut cur_cross = l.p_w[idx(kp[idx(i)])];
        let mut main_sz = l.p_h[idx(kp[idx(i)])];
        if row {
            cur_cross = l.p_h[idx(kp[idx(i)])];
            main_sz = l.p_w[idx(kp[idx(i)])];
        }
        if (sc.kind == crate::style::S_FILL) && ((cur_cross - final_content_cross).abs() > EPS) {
            kp[idx(i)] = measure_child(
                d,
                st,
                l,
                kids[idx(i)],
                row,
                main_sz,
                main_sz,
                final_content_cross,
                final_content_cross,
                cpw,
                cph,
                chas_pw,
                chas_ph,
                kind,
                row,
                &inh,
                false,
                false,
            );
        }
    }
    // Place children only after every final measurement is known.
    let content_main_final = (0.0f64).max(own_main - pad_main);
    used = gaps;
    for i in 0i32..(nk) {
        if kp[idx(i)] >= 0i32 {
            if row {
                used += l.p_w[idx(kp[idx(i)])];
            } else {
                used += l.p_h[idx(kp[idx(i)])];
            }
        }
    }
    let free = (0.0f64).max(content_main_final - used);
    let mut lead = 0.0f64;
    let mut step = gap;
    if st.rs[idx(ri)].pack == 1u32 {
        lead = free / 2.0f64;
    } else if st.rs[idx(ri)].pack == 2u32 {
        lead = free;
    } else if (st.rs[idx(ri)].pack == 3u32) && (nk > 1i32) {
        step = gap + (free / (f64::from(nk.wrapping_sub(1i32))));
    }
    let mut cur = pt + lead;
    if row {
        cur = pl + lead;
    }
    let mut virtual_item = -1;
    let mut virtual_within = 0.0;
    let pi = p_new(l, node, ri);
    // Placement follows measurement so child pool ranges remain contiguous.
    l.p_child_off[idx(pi)] = len_i32(&l.child_pool);
    for i in 0i32..(nk) {
        if kp[idx(i)] < 0i32 {
            continue;
        }
        let ci = kp[idx(i)];
        let cri = l.p_ri[idx(ci)];
        let mut child_cross = l.p_w[idx(ci)];
        if row {
            child_cross = l.p_h[idx(ci)];
        }
        let mut a = st.rs[idx(cri)].self_align;
        if a < 0i32 {
            a = st.rs[idx(ri)].align;
        }
        let off = if want_baseline && l.p_has_base[idx(ci)] {
            row_base - l.p_base[idx(ci)]
        } else {
            (final_content_cross - child_cross) * crate::style::cross_f(a)
        };
        if let Some((extent, _, _, _)) = virtual_metrics {
            let item = crate::list::item_ix(&st.lists, d, kids[idx(i)]);
            if item != virtual_item {
                virtual_item = item;
                virtual_within = 0.0;
            }
            cur = if row { pl } else { pt } + f64::from(item) * extent + virtual_within;
        }
        if row {
            l.p_x[idx(ci)] = cur;
            l.p_y[idx(ci)] = pt + off;
            cur = (cur + l.p_w[idx(ci)]) + step;
        } else {
            l.p_x[idx(ci)] = pl + off;
            l.p_y[idx(ci)] = cur;
            cur = (cur + l.p_h[idx(ci)]) + step;
        }
        if virtual_metrics.is_some() {
            let child_main = if row { l.p_w[idx(ci)] } else { l.p_h[idx(ci)] };
            virtual_within += child_main + gap;
        }
        l.p_x[idx(ci)] += st.rs[idx(cri)].offset_x;
        l.p_y[idx(ci)] += st.rs[idx(cri)].offset_y;
        l.child_pool.push(ci);
    }
    l.p_child_len[idx(pi)] = len_i32(&l.child_pool).wrapping_sub(l.p_child_off[idx(pi)]);
    if row {
        l.p_w[idx(pi)] = own_main;
        l.p_h[idx(pi)] = own_cross;
    } else {
        l.p_w[idx(pi)] = own_cross;
        l.p_h[idx(pi)] = own_main;
    }
    boundary(d, st, l, pi);
    // Export the first child baseline to the parent.
    let off0 = l.p_child_off[idx(pi)];
    for k in off0..(off0.wrapping_add(l.p_child_len[idx(pi)])) {
        let ci = l.child_pool[idx(k)];
        if l.p_has_base[idx(ci)] && (!l.p_has_base[idx(pi)]) {
            l.p_base[idx(pi)] = l.p_y[idx(ci)] + l.p_base[idx(ci)];
            l.p_has_base[idx(pi)] = true;
        }
    }
    give_u32(l, kids);
    give_i32(l, kp);
    give_i32(l, fills);
    pi
}

/// Measures one child inside a box container's available content area.
#[allow(clippy::too_many_arguments)] // Box child measurement preserves distinct authored and resolved constraints.
pub fn measure_in_box(
    d: &Doc,
    st: &mut St,
    l: &mut Lay,
    kid: u32,
    row: bool,
    avail_main: f64,
    avail_cross: f64,
    pct_w: f64,
    pct_h: f64,
    has_pw: bool,
    has_ph: bool,
    hug_cross_container: bool,
    has_fixed_main: bool,
    fixed_main: f64,
    pk: u32,
    inh: &Inh,
    hug_main: bool,
) -> i32 {
    let mut hug_w = false;
    let mut hug_h = false;
    if hug_cross_container {
        let sc = crate::style::peek_size(d, st, kid, !row, pk, row);
        if sc.kind == crate::style::S_FILL {
            if row {
                hug_h = true;
            } else {
                hug_w = true;
            }
        }
    }
    if hug_main {
        if row {
            hug_w = true;
        } else {
            hug_h = true;
        }
    }
    if has_fixed_main {
        return measure_child(
            d,
            st,
            l,
            kid,
            row,
            fixed_main,
            fixed_main,
            0.0f64,
            avail_cross,
            pct_w,
            pct_h,
            has_pw,
            has_ph,
            pk,
            row,
            inh,
            hug_w,
            hug_h,
        );
    }
    measure_child(
        d,
        st,
        l,
        kid,
        row,
        0.0f64,
        avail_main,
        0.0f64,
        avail_cross,
        pct_w,
        pct_h,
        has_pw,
        has_ph,
        pk,
        row,
        inh,
        hug_w,
        hug_h,
    )
}

/// Measures and places a wrapping row.
pub fn wrap_measure(d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let pt = st.rs[idx(ri)].pad_t;
    let pr = st.rs[idx(ri)].pad_r;
    let pb = st.rs[idx(ri)].pad_b;
    let pl = st.rs[idx(ri)].pad_l;
    let line = st.rs[idx(ri)].line;
    let gap = st.rs[idx(ri)].gap;
    let om = resolve_len(
        st,
        st.rs[idx(ri)].w_kind,
        st.rs[idx(ri)].w_v,
        cn.max_w,
        cn.has_pw,
        cn.pct_w,
        line,
    );
    let mut own_w = om.v;
    let has_w = om.has;
    if has_w {
        own_w = clamp(
            own_w,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    let mut budget_w = (cn.max_w).min(st.rs[idx(ri)].max_w);
    if has_w {
        budget_w = own_w;
    }
    let mut content_w = INF;
    if budget_w != INF {
        content_w = (0.0f64).max((budget_w - pl) - pr);
    }
    let mut pct_w = 0.0f64;
    let mut has_pct = false;
    if has_w && (content_w != INF) {
        pct_w = content_w;
        has_pct = true;
    }
    let mut kids: Vec<u32> = take_u32(l);
    crate::style::children(d, st, node, &mut kids);
    let inh = inh_of(st, ri);
    let is_row = st.rs[idx(ri)].is_row;
    let mut kp: Vec<i32> = take_i32(l);
    let mut kline: Vec<i32> = take_i32(l);
    let mut cur_line = 0i32;
    let mut rem = content_w;
    for i in 0i32..(len_i32(&kids)) {
        crate::style::reset_wh_patches(d, st, kids[idx(i)]);
        let sw = crate::style::peek_size(d, st, kids[idx(i)], true, crate::slir::K_WRAP, is_row);
        let mut hug_main = false;
        if sw.kind == crate::style::S_FILL {
            crate::style::warn(
                st,
                "attr",
                "fill width inside wrap is not supported; treating as hug",
                d.node_line[idx(crate::list::base(&st.lists, d, kids[idx(i)]))],
            );
            hug_main = true;
        }
        let mut avail = INF;
        if rem != INF {
            avail = (0.0f64).max(rem);
        }
        let mut c = Cons {
            min_w: 0.0f64,
            max_w: avail,
            min_h: 0.0f64,
            max_h: INF,
            pct_w,
            pct_h: 0.0f64,
            has_pw: has_pct,
            has_ph: false,
        };
        let mut p = measure(
            d,
            st,
            l,
            kids[idx(i)],
            &c,
            crate::slir::K_WRAP,
            is_row,
            &inh,
            false,
            hug_main,
            false,
        );
        let mut line_nonempty = false;
        for j in 0i32..(i) {
            if kline[idx(j)] == cur_line {
                line_nonempty = true;
            }
        }
        if (line_nonempty && (rem != INF)) && (l.p_w[idx(p)] > (rem + EPS)) {
            cur_line = cur_line.wrapping_add(1i32);
            rem = content_w;
            c = Cons {
                min_w: 0.0f64,
                max_w: (0.0f64).max(rem),
                min_h: 0.0f64,
                max_h: INF,
                pct_w,
                pct_h: 0.0f64,
                has_pw: has_pct,
                has_ph: false,
            };
            p = measure(
                d,
                st,
                l,
                kids[idx(i)],
                &c,
                crate::slir::K_WRAP,
                is_row,
                &inh,
                false,
                hug_main,
                false,
            );
        }
        kp.push(p);
        kline.push(cur_line);
        if rem != INF {
            rem = (rem - l.p_w[idx(p)]) - gap;
        }
    }
    let mut gap_cross = gap;
    if st.rs[idx(ri)].has_gap_cross {
        gap_cross = st.rs[idx(ri)].gap_cross;
    }
    let mut y = pt;
    let mut max_line_w = 0.0f64;
    let pi = p_new(l, node, ri);
    l.p_child_off[idx(pi)] = len_i32(&l.child_pool);
    let nlines = cur_line.wrapping_add(1i32);
    let mut any = false;
    for ln in 0i32..(nlines) {
        let mut line_h = 0.0f64;
        let mut has_any = false;
        for i in 0i32..(len_i32(&kp)) {
            if kline[idx(i)] == ln {
                line_h = line_h.max(l.p_h[idx(kp[idx(i)])]);
                has_any = true;
            }
        }
        if !has_any {
            continue;
        }
        any = true;
        let af = crate::style::cross_f(st.rs[idx(ri)].align);
        let mut x = pl;
        for i in 0i32..(len_i32(&kp)) {
            if kline[idx(i)] == ln {
                let ci = kp[idx(i)];
                l.p_x[idx(ci)] = x;
                l.p_y[idx(ci)] = y + (af * (line_h - l.p_h[idx(ci)]));
                x = (x + l.p_w[idx(ci)]) + gap;
                l.child_pool.push(ci);
            }
        }
        max_line_w = max_line_w.max((x - gap) - pl);
        y = (y + line_h) + gap_cross;
    }
    l.p_child_len[idx(pi)] = len_i32(&l.child_pool).wrapping_sub(l.p_child_off[idx(pi)]);
    let mut total_h = pt + pb;
    if any {
        total_h = (y - gap_cross) + pb;
    }
    if !has_w {
        own_w = clamp(
            (max_line_w + pl) + pr,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    let oh = resolve_len(
        st,
        st.rs[idx(ri)].h_kind,
        st.rs[idx(ri)].h_v,
        cn.max_h,
        cn.has_ph,
        cn.pct_h,
        line,
    );
    let mut own_h = total_h;
    if oh.has {
        own_h = oh.v;
    }
    own_h = clamp(
        own_h,
        cn.min_h,
        cn.max_h,
        st.rs[idx(ri)].min_h,
        st.rs[idx(ri)].max_h,
    );
    l.p_w[idx(pi)] = own_w;
    l.p_h[idx(pi)] = own_h;
    boundary(d, st, l, pi);
    give_u32(l, kids);
    give_i32(l, kp);
    give_i32(l, kline);
    pi
}

/// Measures and places a grid with fixed, percent, fill, and hug tracks.
pub fn grid_measure(d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let pt = st.rs[idx(ri)].pad_t;
    let pr = st.rs[idx(ri)].pad_r;
    let pb = st.rs[idx(ri)].pad_b;
    let pl = st.rs[idx(ri)].pad_l;
    let line = st.rs[idx(ri)].line;
    let gap = st.rs[idx(ri)].gap;
    let inh = inh_of(st, ri);
    let is_row = st.rs[idx(ri)].is_row;
    // A grid with no authored tracks has one fill track.
    let mut tk: Vec<u32> = take_u32(l);
    let mut tv: Vec<f64> = take_f64(l);
    if st.rs[idx(ri)].track_len == 0i32 {
        tk.push(2u32);
        tv.push(1.0f64);
    } else {
        let toff = st.rs[idx(ri)].track_off;
        for t in 0i32..(st.rs[idx(ri)].track_len) {
            tk.push(st.track_kind[idx(toff.wrapping_add(t))]);
            tv.push(st.track_v[idx(toff.wrapping_add(t))]);
        }
    }
    let ntr = len_i32(&tk);
    let om = resolve_len(
        st,
        st.rs[idx(ri)].w_kind,
        st.rs[idx(ri)].w_v,
        cn.max_w,
        cn.has_pw,
        cn.pct_w,
        line,
    );
    let mut own_w = om.v;
    let has_w = om.has;
    if has_w {
        own_w = clamp(
            own_w,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    let mut budget_w = (cn.max_w).min(st.rs[idx(ri)].max_w);
    if has_w {
        budget_w = own_w;
    }
    let mut content_w = INF;
    if budget_w != INF {
        content_w = (0.0f64).max((budget_w - pl) - pr);
    }
    // Assign cells row-major, honoring each cell's column span.
    let mut kids: Vec<u32> = take_u32(l);
    crate::style::children(d, st, node, &mut kids);
    let mut ccol: Vec<i32> = take_i32(l);
    let mut cspan: Vec<i32> = take_i32(l);
    let mut col = 0i32;
    for i in 0i32..(len_i32(&kids)) {
        crate::style::reset_wh_patches(d, st, kids[idx(i)]);
        let span = truncate_i32((1.0f64).max(crate::style::attr_num(
            d,
            st,
            kids[idx(i)],
            crate::slir::A_SPAN,
            1.0f64,
        )))
        .min(ntr);
        if col.wrapping_add(span) > ntr {
            col = 0i32;
        }
        ccol.push(col);
        cspan.push(span);
        col = (col.wrapping_add(span)).wrapping_rem(ntr);
    }
    let gaps_total = gap * (f64::from(ntr.wrapping_sub(1i32)));
    let mut remaining = INF;
    if content_w != INF {
        remaining = content_w - gaps_total;
    }
    let mut widths: Vec<f64> = take_f64(l);
    for _t in 0i32..(ntr) {
        widths.push(0.0f64);
    }
    let mut fill_idx: Vec<i32> = take_i32(l);
    let mut pct_base = 0.0f64;
    let mut has_pct = false;
    if content_w != INF {
        pct_base = content_w;
        has_pct = true;
    }
    for t in 0i32..(ntr) {
        let mut w = 0.0f64;
        // Fixed tracks consume their authored width.
        if tk[idx(t)] == 0u32 {
            w = tv[idx(t)];
            if remaining != INF {
                w = (tv[idx(t)]).min(remaining);
            }
            if w < (tv[idx(t)] - EPS) {
                let msg = crate::rt::str_concat(
                    &crate::rt::str_concat(
                        &crate::rt::str_concat(
                            "grid col ",
                            &u32::try_from(t.wrapping_add(1i32))
                                .expect("grid column is nonnegative")
                                .to_string(),
                        ),
                        &crate::rt::str_concat(" asked ", &crate::value::fmt3(tv[idx(t)])),
                    ),
                    &crate::rt::str_concat(
                        &crate::rt::str_concat(", got ", &crate::value::fmt3(w)),
                        "",
                    ),
                );
                crate::style::warn(st, "squeeze", &msg, line);
            }
        // Percent tracks resolve only against a determinate content width.
        } else if tk[idx(t)] == 3u32 {
            if content_w != INF {
                w = (tv[idx(t)] / 100.0f64) * content_w;
            }
            if remaining != INF {
                w = w.min(remaining);
            }
        // Fill tracks share the remaining width after other tracks resolve.
        } else if tk[idx(t)] == 2u32 {
            fill_idx.push(t);
            continue;
        } else {
            // Hug tracks use the widest natural, non-spanning cell in the column.
            for i in 0i32..(len_i32(&kids)) {
                if (ccol[idx(i)] == t) && (cspan[idx(i)] == 1i32) {
                    let mut avail = INF;
                    if remaining != INF {
                        avail = remaining;
                    }
                    let c = Cons {
                        min_w: 0.0f64,
                        max_w: avail,
                        min_h: 0.0f64,
                        max_h: INF,
                        pct_w: pct_base,
                        pct_h: 0.0f64,
                        has_pw: has_pct,
                        has_ph: false,
                    };
                    let p = measure(
                        d,
                        st,
                        l,
                        kids[idx(i)],
                        &c,
                        crate::slir::K_GRID,
                        is_row,
                        &inh,
                        false,
                        false,
                        false,
                    );
                    w = w.max(l.p_w[idx(p)]);
                }
            }
            if remaining != INF {
                w = w.min(remaining);
            }
        }
        widths[idx(t)] = w;
        if remaining != INF {
            remaining = (0.0f64).max(remaining - w);
        }
    }
    if !fill_idx.is_empty() {
        if remaining == INF {
            for j in 0i32..(len_i32(&fill_idx)) {
                widths[idx(fill_idx[idx(j)])] = 0.0f64;
            }
        } else {
            let mut total_wt = 0.0f64;
            for j in 0i32..(len_i32(&fill_idx)) {
                total_wt += tv[idx(fill_idx[idx(j)])];
            }
            if total_wt == 0.0f64 {
                total_wt = 1.0f64;
            }
            for j in 0i32..(len_i32(&fill_idx)) {
                widths[idx(fill_idx[idx(j)])] = (remaining * tv[idx(fill_idx[idx(j)])]) / total_wt;
            }
        }
    }
    let mut xs: Vec<f64> = take_f64(l);
    xs.push(pl);
    for t in 1i32..(ntr) {
        xs.push((xs[idx(t.wrapping_sub(1i32))] + widths[idx(t.wrapping_sub(1i32))]) + gap);
    }
    // Measure cells row by row, then stretch fill-height cells.
    let mut gap_cross = gap;
    if st.rs[idx(ri)].has_gap_cross {
        gap_cross = st.rs[idx(ri)].gap_cross;
    }
    let mut pct_w_val = 0.0f64;
    let mut has_pct_w_val = false;
    if has_w && (content_w != INF) {
        pct_w_val = content_w;
        has_pct_w_val = true;
    }
    let mut kp: Vec<i32> = take_i32(l);
    let mut y = pt;
    let mut row_start = 0i32;
    let mut cur_col_expected = 0i32;
    let pi = p_new(l, node, ri);
    for i in 0i32..(len_i32(&kids).wrapping_add(1i32)) {
        let mut flushrow = i == (len_i32(&kids));
        if (i < (len_i32(&kids))) && (ccol[idx(i)] < cur_col_expected) {
            flushrow = true;
        }
        if flushrow && (i > row_start) {
            let mut row_h = 0.0f64;
            for k in row_start..(i) {
                row_h = row_h.max(l.p_h[idx(kp[idx(k)])]);
            }
            for k in row_start..(i) {
                let mut ci = kp[idx(k)];
                let justify = l.p_x[idx(ci)];
                let sh = crate::style::peek_size(
                    d,
                    st,
                    kids[idx(k)],
                    false,
                    crate::slir::K_GRID,
                    is_row,
                );
                // Stretch fill-height cells to the completed row height.
                if (sh.kind == crate::style::S_FILL) && ((l.p_h[idx(ci)] - row_h).abs() > EPS) {
                    let c = Cons {
                        min_w: l.p_w[idx(ci)],
                        max_w: l.p_w[idx(ci)],
                        min_h: row_h,
                        max_h: row_h,
                        pct_w: pct_w_val,
                        pct_h: row_h,
                        has_pw: has_pct_w_val,
                        has_ph: true,
                    };
                    ci = measure(
                        d,
                        st,
                        l,
                        kids[idx(k)],
                        &c,
                        crate::slir::K_GRID,
                        is_row,
                        &inh,
                        false,
                        false,
                        false,
                    );
                    kp[idx(k)] = ci;
                }
                l.p_x[idx(ci)] = xs[idx(ccol[idx(k)])] + justify;
                l.p_y[idx(ci)] = y;
            }
            y = (y + row_h) + gap_cross;
            row_start = i;
        }
        if i == (len_i32(&kids)) {
            break;
        }
        cur_col_expected = (ccol[idx(i)]).wrapping_add(cspan[idx(i)]);
        let mut cw = gap * (f64::from((cspan[idx(i)]).wrapping_sub(1i32)));
        for t in (ccol[idx(i)])..((ccol[idx(i)]).wrapping_add(cspan[idx(i)])) {
            cw += widths[idx(t)];
        }
        let sa = crate::style::align_code(&crate::style::attr_enum_ref(
            d,
            st,
            kids[idx(i)],
            crate::slir::A_SELF,
        ));
        // Justified cells retain their natural size within the track.
        if ((sa == 0i32) || (sa == 1i32)) || (sa == 2i32) {
            let c = Cons {
                min_w: 0.0f64,
                max_w: cw,
                min_h: 0.0f64,
                max_h: INF,
                pct_w: pct_w_val,
                pct_h: 0.0f64,
                has_pw: has_pct_w_val,
                has_ph: false,
            };
            let p = measure(
                d,
                st,
                l,
                kids[idx(i)],
                &c,
                crate::slir::K_GRID,
                is_row,
                &inh,
                false,
                false,
                false,
            );
            l.p_x[idx(p)] = (cw - l.p_w[idx(p)]) * crate::style::cross_f(sa);
            kp.push(p);
        } else {
            let c = Cons {
                min_w: cw,
                max_w: cw,
                min_h: 0.0f64,
                max_h: INF,
                pct_w: pct_w_val,
                pct_h: 0.0f64,
                has_pw: has_pct_w_val,
                has_ph: false,
            };
            let p = measure(
                d,
                st,
                l,
                kids[idx(i)],
                &c,
                crate::slir::K_GRID,
                is_row,
                &inh,
                false,
                false,
                false,
            );
            l.p_x[idx(p)] = 0.0f64;
            kp.push(p);
        }
    }
    // Claim the child slice only after measuring cells: cell measurement may
    // append nested placements to `child_pool` first.
    l.p_child_off[idx(pi)] = len_i32(&l.child_pool);
    for k in 0i32..(len_i32(&kp)) {
        l.child_pool.push(kp[idx(k)]);
    }
    l.p_child_len[idx(pi)] = len_i32(&l.child_pool).wrapping_sub(l.p_child_off[idx(pi)]);
    let mut total_h = pt + pb;
    if !kp.is_empty() {
        total_h = (y - gap_cross) + pb;
    }
    if !has_w {
        let mut sumw = gaps_total;
        for t in 0i32..(ntr) {
            sumw += widths[idx(t)];
        }
        own_w = clamp(
            (sumw + pl) + pr,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    let oh = resolve_len(
        st,
        st.rs[idx(ri)].h_kind,
        st.rs[idx(ri)].h_v,
        cn.max_h,
        cn.has_ph,
        cn.pct_h,
        line,
    );
    let mut own_h = total_h;
    if oh.has {
        own_h = oh.v;
    }
    own_h = clamp(
        own_h,
        cn.min_h,
        cn.max_h,
        st.rs[idx(ri)].min_h,
        st.rs[idx(ri)].max_h,
    );
    l.p_w[idx(pi)] = own_w;
    l.p_h[idx(pi)] = own_h;
    boundary(d, st, l, pi);
    give_u32(l, tk);
    give_f64(l, tv);
    give_u32(l, kids);
    give_i32(l, ccol);
    give_i32(l, cspan);
    give_f64(l, widths);
    give_i32(l, fill_idx);
    give_f64(l, xs);
    give_i32(l, kp);
    pi
}

fn overlay_hug_extent(
    st: &St,
    l: &Lay,
    placements: &[i32],
    horizontal: bool,
    positioned: bool,
) -> f64 {
    let mut extent = 0.0_f64;
    for &pi in placements {
        let placement = idx(pi);
        if st.rs[idx(l.p_ri[placement])].has_attach {
            continue;
        }
        let position = if !positioned {
            0.0
        } else if horizontal {
            l.p_x[placement]
        } else {
            l.p_y[placement]
        };
        let size = if horizontal {
            l.p_w[placement]
        } else {
            l.p_h[placement]
        };
        extent = extent.max(position + size);
    }
    extent
}

/// Measures a stack and overlays its children according to alignment.
pub fn stack_measure(d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let pt = st.rs[idx(ri)].pad_t;
    let pr = st.rs[idx(ri)].pad_r;
    let pb = st.rs[idx(ri)].pad_b;
    let pl = st.rs[idx(ri)].pad_l;
    let line = st.rs[idx(ri)].line;
    let inh = inh_of(st, ri);
    let is_row = st.rs[idx(ri)].is_row;
    let ow = resolve_len(
        st,
        st.rs[idx(ri)].w_kind,
        st.rs[idx(ri)].w_v,
        cn.max_w,
        cn.has_pw,
        cn.pct_w,
        line,
    );
    let oh = resolve_len(
        st,
        st.rs[idx(ri)].h_kind,
        st.rs[idx(ri)].h_v,
        cn.max_h,
        cn.has_ph,
        cn.pct_h,
        line,
    );
    let mut own_w = ow.v;
    let has_w = ow.has;
    let mut own_h = oh.v;
    let has_h = oh.has;
    if has_w {
        own_w = clamp(
            own_w,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    if has_h {
        own_h = clamp(
            own_h,
            cn.min_h,
            cn.max_h,
            st.rs[idx(ri)].min_h,
            st.rs[idx(ri)].max_h,
        );
    }
    let mut bw = ((cn.max_w).min(st.rs[idx(ri)].max_w) - pl) - pr;
    if has_w {
        bw = (own_w - pl) - pr;
    }
    let mut bh = ((cn.max_h).min(st.rs[idx(ri)].max_h) - pt) - pb;
    if has_h {
        bh = (own_h - pt) - pb;
    }
    if bw != INF {
        bw = (0.0f64).max(bw);
    }
    if bh != INF {
        bh = (0.0f64).max(bh);
    }
    let mut pcw = 0.0f64;
    let mut has_pcw = false;
    if has_w && (bw != INF) {
        pcw = bw;
        has_pcw = true;
    }
    let mut pch = 0.0f64;
    let mut has_pch = false;
    if has_h && (bh != INF) {
        pch = bh;
        has_pch = true;
    }
    let mut kids: Vec<u32> = take_u32(l);
    crate::style::children(d, st, node, &mut kids);
    let mut kp: Vec<i32> = take_i32(l);
    for i in 0i32..(len_i32(&kids)) {
        let c = Cons {
            min_w: 0.0f64,
            max_w: bw,
            min_h: 0.0f64,
            max_h: bh,
            pct_w: pcw,
            pct_h: pch,
            has_pw: has_pcw,
            has_ph: has_pch,
        };
        kp.push(measure(
            d,
            st,
            l,
            kids[idx(i)],
            &c,
            crate::slir::K_STACK,
            is_row,
            &inh,
            false,
            false,
            false,
        ));
    }
    if !has_w {
        let mw = overlay_hug_extent(st, l, &kp, true, false);
        own_w = clamp(
            (mw + pl) + pr,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    if !has_h {
        let mh = overlay_hug_extent(st, l, &kp, false, false);
        own_h = clamp(
            (mh + pt) + pb,
            cn.min_h,
            cn.max_h,
            st.rs[idx(ri)].min_h,
            st.rs[idx(ri)].max_h,
        );
    }
    let cw = (own_w - pl) - pr;
    let ch = (own_h - pt) - pb;
    let pi = p_new(l, node, ri);
    l.p_child_off[idx(pi)] = len_i32(&l.child_pool);
    for k in 0i32..(len_i32(&kp)) {
        let ci = kp[idx(k)];
        let cri = l.p_ri[idx(ci)];
        let mut a = st.rs[idx(cri)].self_align;
        if a < 0i32 {
            a = st.rs[idx(ri)].align;
            if !crate::style::is_nine(a) {
                a = 5i32;
            }
        }
        if !crate::style::is_nine(a) {
            a = 5i32;
        }
        l.p_x[idx(ci)] =
            (pl + ((cw - l.p_w[idx(ci)]) * crate::style::nine_fx(a))) + st.rs[idx(cri)].offset_x;
        l.p_y[idx(ci)] =
            (pt + ((ch - l.p_h[idx(ci)]) * crate::style::nine_fy(a))) + st.rs[idx(cri)].offset_y;
        l.child_pool.push(ci);
    }
    l.p_child_len[idx(pi)] = len_i32(&l.child_pool).wrapping_sub(l.p_child_off[idx(pi)]);
    l.p_w[idx(pi)] = own_w;
    l.p_h[idx(pi)] = own_h;
    boundary(d, st, l, pi);
    let off0 = l.p_child_off[idx(pi)];
    for k in off0..(off0.wrapping_add(l.p_child_len[idx(pi)])) {
        let ci = l.child_pool[idx(k)];
        if l.p_has_base[idx(ci)] && (!l.p_has_base[idx(pi)]) {
            l.p_base[idx(pi)] = l.p_y[idx(ci)] + l.p_base[idx(ci)];
            l.p_has_base[idx(pi)] = true;
        }
    }
    give_u32(l, kids);
    give_i32(l, kp);
    pi
}

/// Measures a canvas and positions children from their anchors.
pub fn canvas_measure(d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let pt = st.rs[idx(ri)].pad_t;
    let pr = st.rs[idx(ri)].pad_r;
    let pb = st.rs[idx(ri)].pad_b;
    let pl = st.rs[idx(ri)].pad_l;
    let line = st.rs[idx(ri)].line;
    let inh = inh_of(st, ri);
    let is_row = st.rs[idx(ri)].is_row;
    let ow = resolve_len(
        st,
        st.rs[idx(ri)].w_kind,
        st.rs[idx(ri)].w_v,
        cn.max_w,
        cn.has_pw,
        cn.pct_w,
        line,
    );
    let oh = resolve_len(
        st,
        st.rs[idx(ri)].h_kind,
        st.rs[idx(ri)].h_v,
        cn.max_h,
        cn.has_ph,
        cn.pct_h,
        line,
    );
    let mut own_w = ow.v;
    let has_w = ow.has;
    let mut own_h = oh.v;
    let has_h = oh.has;
    if has_w {
        own_w = clamp(
            own_w,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    if has_h {
        own_h = clamp(
            own_h,
            cn.min_h,
            cn.max_h,
            st.rs[idx(ri)].min_h,
            st.rs[idx(ri)].max_h,
        );
    }
    let mut cw = INF;
    if has_w {
        cw = (own_w - pl) - pr;
    }
    let mut ch = INF;
    if has_h {
        ch = (own_h - pt) - pb;
    }
    let mut kids: Vec<u32> = take_u32(l);
    crate::style::children(d, st, node, &mut kids);
    let mut kp: Vec<i32> = take_i32(l);
    for i in 0i32..(len_i32(&kids)) {
        crate::style::reset_wh_patches(d, st, kids[idx(i)]);
        // Read each child's position and anchor before measuring because they
        // determine its constraints.
        let at = crate::style::attr_val(d, st, kids[idx(i)], crate::slir::A_AT);
        let mut ax = 0.0f64;
        let mut ay = 0.0f64;
        if crate::style::is_tuple_v(at.tag) {
            ax = crate::style::tup_at(d, st, &at, 0i32);
            ay = crate::style::tup_at(d, st, &at, 1i32);
        }
        let anchor = crate::style::align_code(&crate::style::attr_enum_ref(
            d,
            st,
            kids[idx(i)],
            crate::slir::A_ANCHOR,
        ));
        let mut mw = INF;
        if cw != INF {
            mw = (0.0f64).max(cw - ax);
        }
        let mut mh = INF;
        if ch != INF {
            mh = (0.0f64).max(ch - ay);
        }
        // Anchored children measure against the full canvas.
        if anchor >= 0i32 {
            mw = cw;
            mh = ch;
        }
        let mut pcw = 0.0f64;
        let mut has_pcw = false;
        if cw != INF {
            pcw = cw;
            has_pcw = true;
        }
        let mut pch = 0.0f64;
        let mut has_pch = false;
        if ch != INF {
            pch = ch;
            has_pch = true;
        }
        let c = Cons {
            min_w: 0.0f64,
            max_w: mw,
            min_h: 0.0f64,
            max_h: mh,
            pct_w: pcw,
            pct_h: pch,
            has_pw: has_pcw,
            has_ph: has_pch,
        };
        let p = measure(
            d,
            st,
            l,
            kids[idx(i)],
            &c,
            crate::slir::K_CANVAS,
            is_row,
            &inh,
            false,
            false,
            false,
        );
        let cri = l.p_ri[idx(p)];
        let mut fx = 0.0f64;
        let mut fy = 0.0f64;
        if (anchor >= 0i32) && crate::style::is_nine(anchor) {
            fx = crate::style::nine_fx(anchor);
            fy = crate::style::nine_fy(anchor);
        }
        l.p_x[idx(p)] = ((pl + ax) - (fx * l.p_w[idx(p)])) + st.rs[idx(cri)].offset_x;
        l.p_y[idx(p)] = ((pt + ay) - (fy * l.p_h[idx(p)])) + st.rs[idx(cri)].offset_y;
        kp.push(p);
    }
    if !has_w {
        let mx = overlay_hug_extent(st, l, &kp, true, true);
        own_w = clamp(
            mx + pr,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    if !has_h {
        let my = overlay_hug_extent(st, l, &kp, false, true);
        own_h = clamp(
            my + pb,
            cn.min_h,
            cn.max_h,
            st.rs[idx(ri)].min_h,
            st.rs[idx(ri)].max_h,
        );
    }
    let pi = p_new(l, node, ri);
    l.p_child_off[idx(pi)] = len_i32(&l.child_pool);
    for k in 0i32..(len_i32(&kp)) {
        l.child_pool.push(kp[idx(k)]);
    }
    l.p_child_len[idx(pi)] = len_i32(&l.child_pool).wrapping_sub(l.p_child_off[idx(pi)]);
    l.p_w[idx(pi)] = own_w;
    l.p_h[idx(pi)] = own_h;
    boundary(d, st, l, pi);
    give_u32(l, kids);
    give_i32(l, kp);
    pi
}

/// Measures a text node and stores its shaped text layout.
pub fn text_measure(d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let pt = st.rs[idx(ri)].pad_t;
    let pr = st.rs[idx(ri)].pad_r;
    let pb = st.rs[idx(ri)].pad_b;
    let pl = st.rs[idx(ri)].pad_l;
    let line = st.rs[idx(ri)].line;
    let ow = resolve_len(
        st,
        st.rs[idx(ri)].w_kind,
        st.rs[idx(ri)].w_v,
        cn.max_w,
        cn.has_pw,
        cn.pct_w,
        line,
    );
    let oh = resolve_len(
        st,
        st.rs[idx(ri)].h_kind,
        st.rs[idx(ri)].h_v,
        cn.max_h,
        cn.has_ph,
        cn.pct_h,
        line,
    );
    let mut own_w = ow.v;
    let has_w = ow.has;
    let mut own_h = oh.v;
    let has_h = oh.has;
    if has_w {
        own_w = clamp(
            own_w,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    if has_h {
        own_h = clamp(
            own_h,
            cn.min_h,
            cn.max_h,
            st.rs[idx(ri)].min_h,
            st.rs[idx(ri)].max_h,
        );
    }
    let mut avail_w = ((cn.max_w).min(st.rs[idx(ri)].max_w) - pl) - pr;
    if has_w {
        avail_w = (own_w - pl) - pr;
    }
    let mut max_lines = -1i32;
    let lh = crate::textm::line_h(st.rs[idx(ri)].size, st.rs[idx(ri)].leading);
    let mut budget_h = (cn.max_h).min(st.rs[idx(ri)].max_h);
    if has_h {
        budget_h = own_h;
    }
    if budget_h != INF {
        max_lines = truncate_i32((1.0f64).max(((((budget_h - pt) - pb) + EPS) / lh).floor()));
    }
    let flags = st.rs[idx(ri)].flags;
    let font = st.rs[idx(ri)].font;
    let size = st.rs[idx(ri)].size;
    let leading = st.rs[idx(ri)].leading;
    let tracking = st.rs[idx(ri)].tracking;
    let wrap = (flags & crate::slir::F_NOWRAP) == 0u32;
    let ellipsis = (flags & crate::slir::F_ELLIPSIS) != 0u32;
    let entry_matches = |entry: &crate::textm::TextCacheEntry, content: &str| {
        entry.font == font
            && entry.size == size.to_bits()
            && entry.leading == leading.to_bits()
            && entry.tracking == tracking.to_bits()
            && entry.max_w == avail_w.to_bits()
            && entry.wrap == wrap
            && entry.ellipsis == ellipsis
            && entry.max_lines == max_lines
            && entry.content == content
    };
    let cached = match st.text_layout_cache.get(&node) {
        Some(entry) if entry_matches(entry, &st.rs[idx(ri)].content) => Some(entry.layout.clone()),
        Some(_) => None,
        None => match st.text_layout_cache_cold.remove(&node) {
            Some(entry) if entry_matches(&entry, &st.rs[idx(ri)].content) => {
                let layout = entry.layout.clone();
                st.text_layout_cache.insert(node, entry);
                Some(layout)
            }
            _ => None,
        },
    };
    if let Some(layout) = cached {
        l.tls.push(layout);
    } else {
        let layout = std::rc::Rc::new(crate::textm::measure_text(
            d,
            font,
            size,
            leading,
            tracking,
            &st.rs[idx(ri)].content,
            avail_w,
            wrap,
            ellipsis,
            max_lines,
        ));
        // Bound the hot generation; the demoted generation still serves probes
        // until the next swap, so eviction never re-measures a whole frame.
        if st.text_layout_cache.len() >= 4096 {
            std::mem::swap(&mut st.text_layout_cache, &mut st.text_layout_cache_cold);
            st.text_layout_cache.clear();
        }
        st.text_layout_cache.insert(
            node,
            crate::textm::TextCacheEntry {
                font,
                size: size.to_bits(),
                leading: leading.to_bits(),
                tracking: tracking.to_bits(),
                max_w: avail_w.to_bits(),
                wrap,
                ellipsis,
                max_lines,
                content: st.rs[idx(ri)].content.clone(),
                layout: layout.clone(),
            },
        );
        l.tls.push(layout);
    }
    let ti = len_i32(&l.tls).wrapping_sub(1i32);
    if l.tls[idx(ti)].truncated && ((flags & crate::slir::F_ELLIPSIS) == 0u32) {
        let mut head: Vec<u32> = vec![];
        let mut taken = 0i32;
        for cp in st.rs[idx(ri)].content.chars().map(u32::from) {
            if taken >= 40i32 {
                break;
            }
            head.push(cp);
            taken = taken.wrapping_add(1i32);
        }
        let msg = crate::rt::str_concat(
            &crate::rt::str_concat(
                &crate::rt::str_concat(
                    &crate::style::label(d, st, node),
                    " does not fit and truncates: '",
                ),
                &crate::rt::str_from_chars(&head),
            ),
            "' (wrap, resize, or flag `ellipsis`)",
        );
        crate::style::warn(
            st,
            "clipped",
            &msg,
            d.node_line[idx(crate::list::base(&st.lists, d, node))],
        );
    }
    let pi = p_new(l, node, ri);
    let mut w = own_w;
    if !has_w {
        w = clamp(
            (l.tls[idx(ti)].w + pl) + pr,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    let mut h = own_h;
    if !has_h {
        h = clamp(
            (l.tls[idx(ti)].h + pt) + pb,
            cn.min_h,
            cn.max_h,
            st.rs[idx(ri)].min_h,
            st.rs[idx(ri)].max_h,
        );
    }
    l.p_w[idx(pi)] = w;
    l.p_h[idx(pi)] = h;
    l.p_base[idx(pi)] = pt + l.tls[idx(ti)].ascent;
    l.p_has_base[idx(pi)] = true;
    l.p_tl[idx(pi)] = ti;
    pi
}

fn truncate_para_segments(l: &mut Lay, end: usize) {
    l.seg_x.truncate(end);
    l.seg_a.truncate(end);
    l.seg_b.truncate(end);
    l.seg_w.truncate(end);
    l.seg_font.truncate(end);
    l.seg_size.truncate(end);
    l.seg_weight.truncate(end);
    l.seg_tracking.truncate(end);
    l.seg_strike.truncate(end);
    l.seg_color.truncate(end);
    l.seg_color_kind.truncate(end);
}

/// Rewrites the current tail line to a styled prefix plus one ellipsis.
///
/// The ellipsis joins the last retained segment, so it inherits that run's
/// font, size, weight, tracking, color, and decoration without a new paint op.
fn ellipsize_para_line(d: &Doc, l: &mut Lay, segment_start: i32, max_width: f64) {
    let start = idx(segment_start);
    let end = l.seg_x.len();
    if start >= end {
        return;
    }
    let line_width = l.seg_x[end - 1] + l.seg_w[end - 1];
    if line_width <= max_width + EPS {
        return;
    }

    let mut retained: Vec<Vec<u32>> = Vec::new();
    'segments: for segment in start..end {
        let mut run = Vec::new();
        let mut prefix_width = 0.0;
        let ellipsis_width = crate::textm::char_w(
            d,
            l.seg_font[segment],
            l.seg_size[segment],
            l.seg_tracking[segment],
            crate::textm::ELLIPSIS,
        );
        for character in l.seg_a[segment]..l.seg_b[segment] {
            let codepoint = l.para_chars[idx(character)];
            let character_width = crate::textm::char_w(
                d,
                l.seg_font[segment],
                l.seg_size[segment],
                l.seg_tracking[segment],
                codepoint,
            );
            if l.seg_x[segment] + prefix_width + character_width + ellipsis_width > max_width + EPS
            {
                if !run.is_empty() {
                    retained.push(run);
                }
                break 'segments;
            }
            run.push(codepoint);
            prefix_width += character_width;
        }
        if !run.is_empty() {
            retained.push(run);
        }
    }
    while retained.len() > 1
        && retained.last().is_some_and(|run| {
            run.iter()
                .all(|codepoint| crate::textm::is_strippable(*codepoint))
        })
    {
        retained.pop();
    }
    if let Some(last) = retained.last_mut() {
        while last
            .last()
            .is_some_and(|codepoint| crate::textm::is_strippable(*codepoint))
        {
            last.pop();
        }
    }
    if retained.is_empty() {
        retained.push(Vec::new());
    }
    retained
        .last_mut()
        .expect("retained paragraph run exists")
        .push(crate::textm::ELLIPSIS);

    let kept_end = start + retained.len();
    for (offset, codepoints) in retained.into_iter().enumerate() {
        let segment = start + offset;
        let output_start = len_i32(&l.para_chars);
        l.para_chars.extend(codepoints);
        let output_end = len_i32(&l.para_chars);
        l.seg_a[segment] = output_start;
        l.seg_b[segment] = output_end;
        l.seg_w[segment] = crate::textm::slice_w(
            d,
            l.seg_font[segment],
            l.seg_size[segment],
            l.seg_tracking[segment],
            &l.para_chars,
            output_start,
            output_end,
        );
    }
    truncate_para_segments(l, kept_end);
}

/// Measures a paragraph using greedy word wrapping and merged text segments.
pub fn para_measure(d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let pt = st.rs[idx(ri)].pad_t;
    let pr = st.rs[idx(ri)].pad_r;
    let pb = st.rs[idx(ri)].pad_b;
    let pl = st.rs[idx(ri)].pad_l;
    let line = st.rs[idx(ri)].line;
    let ow = resolve_len(
        st,
        st.rs[idx(ri)].w_kind,
        st.rs[idx(ri)].w_v,
        cn.max_w,
        cn.has_pw,
        cn.pct_w,
        line,
    );
    let mut own_w = ow.v;
    let has_w = ow.has;
    if has_w {
        own_w = clamp(
            own_w,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    let mut avail = ((cn.max_w).min(st.rs[idx(ri)].max_w) - pl) - pr;
    if has_w {
        avail = (own_w - pl) - pr;
    }
    let flags = st.rs[idx(ri)].flags;
    let nowrap = flags & crate::slir::F_NOWRAP != 0;
    let ellipsis = flags & crate::slir::F_ELLIPSIS != 0;
    // Flatten spans into words while retaining each word's resolved style.
    let mut direct: Vec<u32> = take_u32(l);
    crate::style::children(d, st, node, &mut direct);
    let mut kids: Vec<u32> = take_u32(l);
    for &child in &direct {
        let base = crate::list::base(&st.lists, d, child);
        if base != crate::slir::NONE && d.node_kind[idx(base)] == crate::slir::K_EACH {
            let mut runs = take_u32(l);
            crate::style::children(d, st, child, &mut runs);
            kids.extend_from_slice(&runs);
            give_u32(l, runs);
        } else {
            kids.push(child);
        }
    }
    let mut w_a: Vec<i32> = take_i32(l);
    let mut w_b: Vec<i32> = take_i32(l);
    let mut w_ri: Vec<i32> = take_i32(l);
    // Source spaces preceding each word, counted across span boundaries so
    // adjacent spans join with exactly the whitespace the content contains.
    let mut w_gap: Vec<i32> = take_i32(l);
    let mut pending_gap = 0i32;
    let mut cs: Vec<u32> = take_u32(l);
    for i in 0i32..(len_i32(&kids)) {
        crate::style::set_patch_flags(d, st, kids[idx(i)], avail, INF);
        let sri = crate::style::build_rstyle(
            d,
            st,
            kids[idx(i)],
            crate::slir::K_PARA,
            st.rs[idx(ri)].is_row,
            st.rs[idx(ri)].color,
            st.rs[idx(ri)].color_kind,
            st.rs[idx(ri)].fam,
            st.rs[idx(ri)].size,
            st.rs[idx(ri)].weight,
            st.rs[idx(ri)].leading,
            st.rs[idx(ri)].tracking,
            st.rs[idx(ri)].strike,
        );
        cs.clear();
        for cp in st.rs[idx(sri)].content.chars().map(u32::from) {
            cs.push(cp);
        }
        let n = len_i32(&cs);
        let base = len_i32(&l.para_chars);
        for k in 0i32..(n) {
            l.para_chars.push(cs[idx(k)]);
        }
        // Split on spaces; each word remembers how many source spaces led it.
        let mut a = 0i32;
        loop {
            let mut b = a;
            while (b < n) && (cs[idx(b)] != 32u32) {
                b = b.wrapping_add(1i32);
            }
            if b > a {
                w_a.push(base.wrapping_add(a));
                w_b.push(base.wrapping_add(b));
                w_ri.push(sri);
                w_gap.push(pending_gap);
                pending_gap = 0i32;
            }
            if b >= n {
                break;
            }
            pending_gap = pending_gap.wrapping_add(1i32);
            a = b.wrapping_add(1i32);
        }
    }
    // Greedily wrap words with the same EPS tolerance used by the solver.
    // Each word advances by its source gap; a gap is dropped when the word
    // opens a wrapped line, and `w_eff` records the gap actually applied.
    let mut wline: Vec<i32> = take_i32(l);
    let mut w_eff: Vec<i32> = take_i32(l);
    let mut cur_line = 0i32;
    let mut cur_w = 0.0f64;
    let mut line_len = 0i32;
    for i in 0i32..(len_i32(&w_a)) {
        let sri = w_ri[idx(i)];
        let ww = crate::textm::slice_w(
            d,
            st.rs[idx(sri)].font,
            st.rs[idx(sri)].size,
            st.rs[idx(sri)].tracking,
            &l.para_chars,
            w_a[idx(i)],
            w_b[idx(i)],
        );
        let sp = crate::textm::char_w(
            d,
            st.rs[idx(sri)].font,
            st.rs[idx(sri)].size,
            st.rs[idx(sri)].tracking,
            32u32,
        );
        let mut eff = w_gap[idx(i)];
        let mut add = ww + (f64::from(eff)) * sp;
        if !nowrap && ((line_len > 0i32) && (avail != INF)) && ((cur_w + add) > (avail + EPS)) {
            cur_line = cur_line.wrapping_add(1i32);
            cur_w = 0.0f64;
            line_len = 0i32;
            eff = 0i32;
            add = ww;
        }
        wline.push(cur_line);
        w_eff.push(eff);
        cur_w += add;
        line_len = line_len.wrapping_add(1i32);
    }
    let pi = p_new(l, node, ri);
    l.para_line_off.push(len_i32(&l.pl_h));
    let nlines = cur_line.wrapping_add(1i32);
    let mut max_w = 0.0f64;
    let mut total_h = 0.0f64;
    let mut nl_used = 0i32;
    for ln in 0i32..(nlines) {
        let mut lh = 0.0f64;
        let mut asc = 0.0f64;
        let mut has_any = false;
        for i in 0i32..(len_i32(&w_a)) {
            if wline[idx(i)] == ln {
                has_any = true;
                let sri = w_ri[idx(i)];
                lh = lh.max(crate::textm::line_h(
                    st.rs[idx(sri)].size,
                    st.rs[idx(sri)].leading,
                ));
                asc = asc.max(crate::textm::ascent(
                    d,
                    st.rs[idx(sri)].font,
                    st.rs[idx(sri)].size,
                    st.rs[idx(sri)].leading,
                ));
            }
        }
        if !has_any {
            continue;
        }
        nl_used = nl_used.wrapping_add(1i32);
        l.pl_h.push(lh);
        l.pl_asc.push(asc);
        // Merge adjacent words only when every shaped and painted run input matches.
        l.pl_seg_off.push(len_i32(&l.seg_x));
        let mut x = 0.0f64;
        let mut seg_open = false;
        let mut seg_start = 0i32;
        let mut seg_sri = 0i32;
        for i in 0i32..(len_i32(&w_a)) {
            if wline[idx(i)] != ln {
                continue;
            }
            let sri = w_ri[idx(i)];
            let same = if seg_open {
                let style = &st.rs[idx(sri)];
                let segment_style = &st.rs[idx(seg_sri)];
                sri == seg_sri
                    || (style.size == segment_style.size
                        && style.fam == segment_style.fam
                        && style.weight == segment_style.weight
                        && style.tracking == segment_style.tracking
                        && style.strike == segment_style.strike
                        && style.color_kind == segment_style.color_kind
                        && style.color == segment_style.color)
            } else {
                false
            };
            // Joined words contribute their applied source gap to the pool;
            // a zero gap butts adjacent spans together with no space.
            let eff = w_eff[idx(i)];
            if same {
                for _s in 0i32..(eff) {
                    l.para_chars.push(32u32);
                }
                for k in (w_a[idx(i)])..(w_b[idx(i)]) {
                    l.para_chars.push(l.para_chars[idx(k)]);
                }
            } else {
                let gw = (f64::from(eff))
                    * crate::textm::char_w(
                        d,
                        st.rs[idx(sri)].font,
                        st.rs[idx(sri)].size,
                        st.rs[idx(sri)].tracking,
                        32u32,
                    );
                if seg_open {
                    close_seg(d, st, l, seg_start, seg_sri, x);
                    let last = len_i32(&l.seg_x).wrapping_sub(1i32);
                    x = (l.seg_x[idx(last)] + l.seg_w[idx(last)]) + gw;
                } else {
                    x = gw;
                }
                seg_open = true;
                seg_sri = sri;
                seg_start = len_i32(&l.para_chars);
                for k in (w_a[idx(i)])..(w_b[idx(i)]) {
                    l.para_chars.push(l.para_chars[idx(k)]);
                }
            }
        }
        if seg_open {
            close_seg(d, st, l, seg_start, seg_sri, x);
        }
        let segment_off = l.pl_seg_off[idx(len_i32(&l.pl_seg_off).wrapping_sub(1i32))];
        if nowrap && ellipsis && avail != INF {
            ellipsize_para_line(d, l, segment_off, avail);
        }
        l.pl_seg_len.push(
            len_i32(&l.seg_x)
                .wrapping_sub(l.pl_seg_off[idx(len_i32(&l.pl_seg_off).wrapping_sub(1i32))]),
        );
        let so = l.pl_seg_off[idx(len_i32(&l.pl_seg_off).wrapping_sub(1i32))];
        let sn = l.pl_seg_len[idx(len_i32(&l.pl_seg_len).wrapping_sub(1i32))];
        let mut line_w = 0.0f64;
        if sn > 0i32 {
            line_w = l.seg_x[idx(so.wrapping_add(sn).wrapping_sub(1i32))]
                + l.seg_w[idx(so.wrapping_add(sn).wrapping_sub(1i32))];
        }
        l.pl_w.push(line_w);
        max_w = max_w.max(line_w);
        total_h += lh;
    }
    l.para_line_len.push(nl_used);
    l.p_para[idx(pi)] = len_i32(&l.para_line_off).wrapping_sub(1i32);
    let mut w = own_w;
    if !has_w {
        w = clamp(
            (max_w + pl) + pr,
            cn.min_w,
            cn.max_w,
            st.rs[idx(ri)].min_w,
            st.rs[idx(ri)].max_w,
        );
    }
    let oh = resolve_len(
        st,
        st.rs[idx(ri)].h_kind,
        st.rs[idx(ri)].h_v,
        cn.max_h,
        cn.has_ph,
        cn.pct_h,
        line,
    );
    let mut h = (total_h + pt) + pb;
    if oh.has {
        h = oh.v;
    }
    h = clamp(
        h,
        cn.min_h,
        cn.max_h,
        st.rs[idx(ri)].min_h,
        st.rs[idx(ri)].max_h,
    );
    l.p_w[idx(pi)] = w;
    l.p_h[idx(pi)] = h;
    if nl_used > 0i32 {
        let lo = l.para_line_off[idx(l.p_para[idx(pi)])];
        l.p_base[idx(pi)] = pt + l.pl_asc[idx(lo)];
        l.p_has_base[idx(pi)] = true;
    }
    give_u32(l, direct);
    give_u32(l, kids);
    give_u32(l, cs);
    give_i32(l, w_a);
    give_i32(l, w_b);
    give_i32(l, w_ri);
    give_i32(l, w_gap);
    give_i32(l, wline);
    give_i32(l, w_eff);
    pi
}

/// Closes a paragraph segment and records its measured glyph slice.
pub fn close_seg(d: &Doc, st: &St, l: &mut Lay, seg_start: i32, sri: i32, x: f64) {
    let seg_end = len_i32(&l.para_chars);
    let wseg = crate::textm::slice_w(
        d,
        st.rs[idx(sri)].font,
        st.rs[idx(sri)].size,
        st.rs[idx(sri)].tracking,
        &l.para_chars,
        seg_start,
        seg_end,
    );
    l.seg_x.push(x);
    l.seg_a.push(seg_start);
    l.seg_b.push(seg_end);
    l.seg_w.push(wseg);
    l.seg_font.push(st.rs[idx(sri)].font);
    l.seg_size.push(st.rs[idx(sri)].size);
    l.seg_weight.push(st.rs[idx(sri)].weight);
    l.seg_tracking.push(st.rs[idx(sri)].tracking);
    l.seg_color.push(st.rs[idx(sri)].color);
    l.seg_color_kind.push(st.rs[idx(sri)].color_kind);
    l.seg_strike.push(st.rs[idx(sri)].strike);
}

/// Measures an image while preserving its intrinsic aspect ratio.
pub fn img_measure(d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let line = st.rs[idx(ri)].line;
    let ow = resolve_len(
        st,
        st.rs[idx(ri)].w_kind,
        st.rs[idx(ri)].w_v,
        cn.max_w,
        cn.has_pw,
        cn.pct_w,
        line,
    );
    let oh = resolve_len(
        st,
        st.rs[idx(ri)].h_kind,
        st.rs[idx(ri)].h_v,
        cn.max_h,
        cn.has_ph,
        cn.pct_h,
        line,
    );
    let image = usize::try_from(st.rs[idx(ri)].img).ok();
    let dimensions = image.and_then(|image| {
        if image < d.img_src.len() {
            Some((*d.img_w.get(image)?, *d.img_h.get(image)?))
        } else {
            st.runtime_images
                .get(image - d.img_src.len())
                .filter(|image| image.active)
                .map(|image| (image.w, image.h))
        }
    });
    let (natural_w, natural_h) = dimensions
        .filter(|&(w, h)| w > 0 && h > 0)
        .map(|(w, h)| (f64::from(w), f64::from(h)))
        .unwrap_or((64.0, 64.0));
    let mut own_w = ow.v;
    let mut own_h = oh.v;
    if !ow.has && !oh.has {
        own_w = natural_w;
        own_h = natural_h;
    } else if !ow.has {
        own_w = own_h * natural_w / natural_h;
    } else if !oh.has {
        own_h = own_w * natural_h / natural_w;
    }
    own_w = clamp(
        own_w,
        cn.min_w,
        cn.max_w,
        st.rs[idx(ri)].min_w,
        st.rs[idx(ri)].max_w,
    );
    own_h = clamp(
        own_h,
        cn.min_h,
        cn.max_h,
        st.rs[idx(ri)].min_h,
        st.rs[idx(ri)].max_h,
    );
    let pi = p_new(l, node, ri);
    l.p_w[idx(pi)] = own_w;
    l.p_h[idx(pi)] = own_h;
    pi
}

/// Measures a path from the bounds of its normalized absolute coordinates.
pub fn path_measure(d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let path = st.rs[idx(ri)].path;
    let (natural_w, natural_h) = crate::style::path_coords(d, st, path)
        .and_then(crate::pathdata::bounds)
        .map(|(min_x, min_y, max_x, max_y)| (max_x - min_x, max_y - min_y))
        .unwrap_or((0.0, 0.0));
    let line = st.rs[idx(ri)].line;
    let width = resolve_len(
        st,
        st.rs[idx(ri)].w_kind,
        st.rs[idx(ri)].w_v,
        cn.max_w,
        cn.has_pw,
        cn.pct_w,
        line,
    );
    let height = resolve_len(
        st,
        st.rs[idx(ri)].h_kind,
        st.rs[idx(ri)].h_v,
        cn.max_h,
        cn.has_ph,
        cn.pct_h,
        line,
    );
    let own_w = clamp(
        if width.has { width.v } else { natural_w },
        cn.min_w,
        cn.max_w,
        st.rs[idx(ri)].min_w,
        st.rs[idx(ri)].max_w,
    );
    let own_h = clamp(
        if height.has { height.v } else { natural_h },
        cn.min_h,
        cn.max_h,
        st.rs[idx(ri)].min_h,
        st.rs[idx(ri)].max_h,
    );
    let pi = p_new(l, node, ri);
    l.p_w[idx(pi)] = own_w;
    l.p_h[idx(pi)] = own_h;
    pi
}

/// Measures an icon as a square whose intrinsic side is its resolved text size.
pub fn icon_measure(_d: &Doc, st: &mut St, l: &mut Lay, node: u32, ri: i32, cn: &Cons) -> i32 {
    let line = st.rs[idx(ri)].line;
    let width = resolve_len(
        st,
        st.rs[idx(ri)].w_kind,
        st.rs[idx(ri)].w_v,
        cn.max_w,
        cn.has_pw,
        cn.pct_w,
        line,
    );
    let height = resolve_len(
        st,
        st.rs[idx(ri)].h_kind,
        st.rs[idx(ri)].h_v,
        cn.max_h,
        cn.has_ph,
        cn.pct_h,
        line,
    );
    let side = st.rs[idx(ri)].size.max(0.0);
    let own_w = clamp(
        if width.has { width.v } else { side },
        cn.min_w,
        cn.max_w,
        st.rs[idx(ri)].min_w,
        st.rs[idx(ri)].max_w,
    );
    let own_h = clamp(
        if height.has { height.v } else { side },
        cn.min_h,
        cn.max_h,
        st.rs[idx(ri)].min_h,
        st.rs[idx(ri)].max_h,
    );
    let pi = p_new(l, node, ri);
    l.p_w[idx(pi)] = own_w;
    l.p_h[idx(pi)] = own_h;
    pi
}

#[derive(Clone, Copy)]
struct AttachRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn gravity_position(
    anchor: AttachRect,
    popup_w: f64,
    popup_h: f64,
    gravity: crate::style::Gravity,
) -> (f64, f64) {
    use crate::style::Gravity;
    match gravity {
        Gravity::BelowStart => (anchor.x, anchor.y + anchor.h),
        Gravity::BelowCenter => (anchor.x + (anchor.w - popup_w) / 2.0, anchor.y + anchor.h),
        Gravity::BelowEnd => (anchor.x + anchor.w - popup_w, anchor.y + anchor.h),
        Gravity::AboveStart => (anchor.x, anchor.y - popup_h),
        Gravity::AboveCenter => (anchor.x + (anchor.w - popup_w) / 2.0, anchor.y - popup_h),
        Gravity::AboveEnd => (anchor.x + anchor.w - popup_w, anchor.y - popup_h),
        Gravity::LeftStart => (anchor.x - popup_w, anchor.y),
        Gravity::LeftCenter => (anchor.x - popup_w, anchor.y + (anchor.h - popup_h) / 2.0),
        Gravity::LeftEnd => (anchor.x - popup_w, anchor.y + anchor.h - popup_h),
        Gravity::RightStart => (anchor.x + anchor.w, anchor.y),
        Gravity::RightCenter => (anchor.x + anchor.w, anchor.y + (anchor.h - popup_h) / 2.0),
        Gravity::RightEnd => (anchor.x + anchor.w, anchor.y + anchor.h - popup_h),
    }
}

fn opposite_gravity(gravity: crate::style::Gravity) -> crate::style::Gravity {
    use crate::style::Gravity;
    match gravity {
        Gravity::BelowStart => Gravity::AboveStart,
        Gravity::BelowCenter => Gravity::AboveCenter,
        Gravity::BelowEnd => Gravity::AboveEnd,
        Gravity::AboveStart => Gravity::BelowStart,
        Gravity::AboveCenter => Gravity::BelowCenter,
        Gravity::AboveEnd => Gravity::BelowEnd,
        Gravity::LeftStart => Gravity::RightStart,
        Gravity::LeftCenter => Gravity::RightCenter,
        Gravity::LeftEnd => Gravity::RightEnd,
        Gravity::RightStart => Gravity::LeftStart,
        Gravity::RightCenter => Gravity::LeftCenter,
        Gravity::RightEnd => Gravity::LeftEnd,
    }
}

fn vertical_gravity(gravity: crate::style::Gravity) -> bool {
    matches!(
        gravity,
        crate::style::Gravity::BelowStart
            | crate::style::Gravity::BelowCenter
            | crate::style::Gravity::BelowEnd
            | crate::style::Gravity::AboveStart
            | crate::style::Gravity::AboveCenter
            | crate::style::Gravity::AboveEnd
    )
}

fn positive_gravity(gravity: crate::style::Gravity) -> bool {
    matches!(
        gravity,
        crate::style::Gravity::BelowStart
            | crate::style::Gravity::BelowCenter
            | crate::style::Gravity::BelowEnd
            | crate::style::Gravity::RightStart
            | crate::style::Gravity::RightCenter
            | crate::style::Gravity::RightEnd
    )
}

/// Flips a gravity's alignment between start and end; centers are unchanged.
fn flipped_alignment(gravity: crate::style::Gravity) -> crate::style::Gravity {
    use crate::style::Gravity;
    match gravity {
        Gravity::BelowStart => Gravity::BelowEnd,
        Gravity::BelowEnd => Gravity::BelowStart,
        Gravity::AboveStart => Gravity::AboveEnd,
        Gravity::AboveEnd => Gravity::AboveStart,
        Gravity::LeftStart => Gravity::LeftEnd,
        Gravity::LeftEnd => Gravity::LeftStart,
        Gravity::RightStart => Gravity::RightEnd,
        Gravity::RightEnd => Gravity::RightStart,
        center => center,
    }
}

#[allow(clippy::too_many_arguments)] // Placement keeps the anchor, popup, viewport, and authored offset explicit.
fn attachment_position(
    anchor: AttachRect,
    popup_w: f64,
    popup_h: f64,
    mut gravity: crate::style::Gravity,
    collide_auto: bool,
    viewport_w: f64,
    viewport_h: f64,
    offset_x: f64,
    offset_y: f64,
) -> (f64, f64) {
    let mut position = gravity_position(anchor, popup_w, popup_h, gravity);
    if collide_auto {
        let vertical = vertical_gravity(gravity);
        let overflows_main = if vertical {
            if positive_gravity(gravity) {
                position.1 + popup_h > viewport_h
            } else {
                position.1 < 0.0
            }
        } else if positive_gravity(gravity) {
            position.0 + popup_w > viewport_w
        } else {
            position.0 < 0.0
        };
        if overflows_main {
            gravity = opposite_gravity(gravity);
            position = gravity_position(anchor, popup_w, popup_h, gravity);
        }
        // Alignment-axis resolution order: keep the authored alignment when it
        // fits, then prefer flipping start<->end (which keeps the overlay
        // attached to its anchor), and only slide as the last resort.
        let (alignment_position, alignment_extent, viewport_extent) = if vertical_gravity(gravity) {
            (position.0, popup_w, viewport_w)
        } else {
            (position.1, popup_h, viewport_h)
        };
        if alignment_position < 0.0 || alignment_position + alignment_extent > viewport_extent {
            let flipped = flipped_alignment(gravity);
            if flipped != gravity {
                let candidate = gravity_position(anchor, popup_w, popup_h, flipped);
                let flipped_position = if vertical_gravity(gravity) {
                    candidate.0
                } else {
                    candidate.1
                };
                if flipped_position >= 0.0 && flipped_position + alignment_extent <= viewport_extent
                {
                    gravity = flipped;
                    position = candidate;
                }
            }
        }
        if vertical_gravity(gravity) {
            position.0 = position.0.clamp(0.0, (viewport_w - popup_w).max(0.0));
        } else {
            position.1 = position.1.clamp(0.0, (viewport_h - popup_h).max(0.0));
        }
    }
    (position.0 + offset_x, position.1 + offset_y)
}

#[allow(clippy::too_many_arguments)] // The pre-flatten traversal records parent, rotation, and sibling-slot links.
fn collect_placements(
    l: &Lay,
    pi: i32,
    parent: i32,
    rotated_payload: bool,
    child_slot: i32,
    parents: &mut [i32],
    rotated: &mut [bool],
    slots: &mut [i32],
    actual: &mut Vec<i32>,
) {
    let placement = idx(pi);
    if parents[placement] != -2 {
        return;
    }
    parents[placement] = parent;
    rotated[placement] = rotated_payload;
    slots[placement] = child_slot;
    actual.push(pi);
    if l.p_rot[placement] >= 0 {
        collect_placements(
            l,
            l.p_rot[placement],
            pi,
            true,
            -1,
            parents,
            rotated,
            slots,
            actual,
        );
        return;
    }
    let first = l.p_child_off[placement];
    let end = first.wrapping_add(l.p_child_len[placement]);
    for child_index in first..end {
        collect_placements(
            l,
            l.child_pool[idx(child_index)],
            pi,
            false,
            child_index,
            parents,
            rotated,
            slots,
            actual,
        );
    }
}

#[derive(Clone, Copy, Debug)]
struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

impl Affine {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    fn rotation(cx: f64, cy: f64, deg: f64) -> Self {
        let sin = crate::hit::sin_deg(deg);
        let cos = crate::hit::cos_deg(deg);
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            tx: cx - cos * cx + sin * cy,
            ty: cy - sin * cx - cos * cy,
        }
    }

    /// Returns `self ∘ inner`.
    fn compose(self, inner: Self) -> Self {
        Self {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            tx: self.a * inner.tx + self.c * inner.ty + self.tx,
            ty: self.b * inner.tx + self.d * inner.ty + self.ty,
        }
    }

    fn map(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    fn inverse_map(self, x: f64, y: f64) -> (f64, f64) {
        let det = self.a * self.d - self.b * self.c;
        debug_assert!(det.abs() > f64::EPSILON);
        let x = x - self.tx;
        let y = y - self.ty;
        (
            (self.d * x - self.c * y) / det,
            (-self.b * x + self.a * y) / det,
        )
    }
}

fn painted_rect(transform: Affine, x: f64, y: f64, w: f64, h: f64) -> AttachRect {
    let corners = [
        transform.map(x, y),
        transform.map(x + w, y),
        transform.map(x, y + h),
        transform.map(x + w, y + h),
    ];
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (corner_x, corner_y) in corners {
        min_x = min_x.min(corner_x);
        min_y = min_y.min(corner_y);
        max_x = max_x.max(corner_x);
        max_y = max_y.max(corner_y);
    }
    AttachRect {
        x: min_x,
        y: min_y,
        w: max_x - min_x,
        h: max_y - min_y,
    }
}

fn raw_origin_for_painted_rect(
    transform: Affine,
    painted_x: f64,
    painted_y: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    let transformed_origin = transform.map(0.0, 0.0);
    let extent = painted_rect(transform, 0.0, 0.0, w, h);
    transform.inverse_map(
        painted_x - (extent.x - transformed_origin.0),
        painted_y - (extent.y - transformed_origin.1),
    )
}

/// Retained scratch for [`place_attached`], cleared and reused every call.
#[derive(Clone, Debug, Default)]
struct AttachScratch {
    parents: Vec<i32>,
    rotated: Vec<bool>,
    slots: Vec<i32>,
    actual: Vec<i32>,
    state: Vec<u8>,
    abs_x: Vec<f64>,
    abs_y: Vec<f64>,
    transform: Vec<Affine>,
    keys: FxHashMap<String, i32>,
}

struct AttachmentVisibility<'a> {
    st: &'a St,
    l: &'a mut Lay,
    parents: &'a [i32],
    rotated: &'a [bool],
    keys: &'a FxHashMap<String, i32>,
    state: Vec<u8>,
}

impl AttachmentVisibility<'_> {
    fn reject(&mut self, placement: usize) -> bool {
        self.l.p_skip[placement] = true;
        self.state[placement] = 3;
        false
    }

    fn resolve(&mut self, pi: i32) -> bool {
        let placement = idx(pi);
        match self.state[placement] {
            1 => return self.reject(placement),
            2 => return true,
            3 => return false,
            _ => {}
        }
        self.state[placement] = 1;

        let parent = self.parents[placement];
        if parent >= 0 && !self.resolve(parent) {
            return self.reject(placement);
        }
        if !self.rotated[placement] {
            let rule = &self.st.rs[idx(self.l.p_ri[placement])];
            if rule.has_attach {
                let Some(target) = self.keys.get(&rule.attach).copied() else {
                    return self.reject(placement);
                };
                if !self.resolve(target) {
                    return self.reject(placement);
                }
            }
        }

        self.state[placement] = 2;
        true
    }
}

struct AttachmentPass<'a> {
    st: &'a St,
    l: &'a mut Lay,
    parents: &'a [i32],
    rotated: &'a [bool],
    slots: &'a [i32],
    keys: &'a FxHashMap<String, i32>,
    state: Vec<u8>,
    abs_x: Vec<f64>,
    abs_y: Vec<f64>,
    transform: Vec<Affine>,
    viewport_w: f64,
    viewport_h: f64,
}

impl AttachmentPass<'_> {
    fn child_origin(&self, placement: usize, parent: usize) -> (f64, f64) {
        if self.rotated[placement] {
            return (
                self.abs_x[parent] + (self.l.p_w[parent] - self.l.p_w[placement]) / 2.0,
                self.abs_y[parent] + (self.l.p_h[parent] - self.l.p_h[placement]) / 2.0,
            );
        }
        let parent_rule = &self.st.rs[idx(self.l.p_ri[parent])];
        let mut x = self.abs_x[parent];
        let mut y = self.abs_y[parent];
        if parent_rule.flags & crate::slir::F_SCROLL != 0 {
            let offset = crate::style::scroll_get(self.st, self.l.p_node[parent]);
            if parent_rule.is_row {
                x -= offset;
            } else {
                y -= offset;
            }
        }
        if parent_rule.flags & crate::slir::F_SCROLL_CROSS != 0 {
            let offset = crate::style::scroll_cross_get(self.st, self.l.p_node[parent]);
            if parent_rule.is_row {
                y -= offset;
            } else {
                x -= offset;
            }
        }
        if self.slots[placement] >= 0
            && let Some(painted) = sticky_main_position(
                self.st,
                self.l,
                i32::try_from(parent).expect("placed node index exceeds i32"),
                self.slots[placement],
            )
        {
            if parent_rule.is_row {
                x = self.abs_x[parent] + painted - self.l.p_x[placement];
            } else {
                y = self.abs_y[parent] + painted - self.l.p_y[placement];
            }
        }
        (x, y)
    }

    fn resolve(&mut self, pi: i32) -> bool {
        let placement = idx(pi);
        if self.l.p_skip[placement] {
            self.state[placement] = 3;
            return false;
        }
        match self.state[placement] {
            1 | 3 => return false,
            2 => return true,
            _ => {}
        }
        self.state[placement] = 1;

        let parent = self.parents[placement];
        let (origin_x, origin_y, parent_transform) = if parent >= 0 {
            if !self.resolve(parent) {
                self.state[placement] = 3;
                return false;
            }
            let parent = idx(parent);
            let (origin_x, origin_y) = self.child_origin(placement, parent);
            (origin_x, origin_y, self.transform[parent])
        } else {
            (0.0, 0.0, Affine::IDENTITY)
        };
        let mut x = origin_x + self.l.p_x[placement];
        let mut y = origin_y + self.l.p_y[placement];

        if !self.rotated[placement] {
            let rule = &self.st.rs[idx(self.l.p_ri[placement])];
            if rule.has_attach {
                let target = self.keys.get(&rule.attach).copied();
                let gravity = rule.gravity;
                let collide_auto = rule.collide_auto;
                let offset_x = rule.offset_x;
                let offset_y = rule.offset_y;
                let Some(target) = target else {
                    self.state[placement] = 3;
                    return false;
                };
                if !self.resolve(target) {
                    self.state[placement] = 3;
                    return false;
                }
                let target_index = idx(target);
                let anchor = painted_rect(
                    self.transform[target_index],
                    self.abs_x[target_index],
                    self.abs_y[target_index],
                    self.l.p_w[target_index],
                    self.l.p_h[target_index],
                );
                let popup_extent = painted_rect(
                    parent_transform,
                    0.0,
                    0.0,
                    self.l.p_w[placement],
                    self.l.p_h[placement],
                );
                let (painted_x, painted_y) = attachment_position(
                    anchor,
                    popup_extent.w,
                    popup_extent.h,
                    gravity,
                    collide_auto,
                    self.viewport_w,
                    self.viewport_h,
                    offset_x,
                    offset_y,
                );
                // TUI output quantizes every op to the 8x16 cell grid; snap
                // the anchored overlay's painted origin to whole cells so its
                // borders land on cell boundaries instead of melting into
                // neighbouring content rows.
                let (painted_x, painted_y) = if self.st.env.client == crate::when::CLIENT_TUI {
                    (
                        f64::from(crate::cells::rhe(painted_x / crate::cells::CW))
                            * crate::cells::CW,
                        f64::from(crate::cells::rhe(painted_y / crate::cells::CH))
                            * crate::cells::CH,
                    )
                } else {
                    (painted_x, painted_y)
                };
                (x, y) = raw_origin_for_painted_rect(
                    parent_transform,
                    painted_x,
                    painted_y,
                    self.l.p_w[placement],
                    self.l.p_h[placement],
                );
                self.l.p_x[placement] = x - origin_x;
                self.l.p_y[placement] = y - origin_y;
            }
        }

        self.abs_x[placement] = x;
        self.abs_y[placement] = y;
        let rotate = self.st.rs[idx(self.l.p_ri[placement])].rotate;
        self.transform[placement] = if rotate == 0.0 {
            parent_transform
        } else {
            parent_transform.compose(Affine::rotation(
                x + self.l.p_w[placement] / 2.0,
                y + self.l.p_h[placement] / 2.0,
                rotate,
            ))
        };
        self.state[placement] = 2;
        true
    }
}

/// Places attached overlays against the current solved, scroll-adjusted scene.
///
/// Missing anchors and attachment cycles mark the attached subtree as skipped,
/// so neither frame lowering nor hit testing can observe stale geometry.
pub fn place_attached(
    d: &Doc,
    st: &St,
    l: &mut Lay,
    root_pi: i32,
    viewport_w: f64,
    viewport_h: f64,
) {
    l.p_skip.fill(false);
    if root_pi < 0 || l.p_node.is_empty() {
        return;
    }
    if !l.p_ri.iter().any(|&ri| st.rs[idx(ri)].has_attach) {
        return;
    }

    let count = l.p_node.len();
    let mut s = std::mem::take(&mut l.attach);
    s.parents.clear();
    s.parents.resize(count, -2);
    s.rotated.clear();
    s.rotated.resize(count, false);
    s.slots.clear();
    s.slots.resize(count, -1);
    s.actual.clear();
    collect_placements(
        l,
        root_pi,
        -1,
        false,
        -1,
        &mut s.parents,
        &mut s.rotated,
        &mut s.slots,
        &mut s.actual,
    );
    if !s.actual.iter().any(|&pi| {
        let placement = idx(pi);
        !s.rotated[placement] && st.rs[idx(l.p_ri[placement])].has_attach
    }) {
        l.attach = s;
        return;
    }

    s.keys.clear();
    for &pi in &s.actual {
        let placement = idx(pi);
        // Quarter-turn outers have no scene entry; their centered payload
        // carries the same key and is the painted rect attachment targets see.
        if l.p_rot[placement] >= 0 {
            continue;
        }
        let key = crate::scene::key_of(d, &st.lists, l.p_node[placement]);
        if !key.is_empty() {
            s.keys.entry(key).or_insert(pi);
        }
    }
    {
        // Finalize suppression for the whole attachment dependency graph before
        // sticky geometry or any other painted position is evaluated.
        s.state.clear();
        s.state.resize(count, 0);
        let mut visibility = AttachmentVisibility {
            st,
            l,
            parents: &s.parents,
            rotated: &s.rotated,
            keys: &s.keys,
            state: std::mem::take(&mut s.state),
        };
        for &pi in &s.actual {
            visibility.resolve(pi);
        }
        s.state = visibility.state;
    }
    let root = idx(root_pi);
    let viewport_w = if viewport_w > 0.0 {
        viewport_w
    } else {
        l.p_w[root]
    };
    let viewport_h = if viewport_h > 0.0 {
        viewport_h
    } else {
        l.p_h[root]
    };
    s.state.clear();
    s.state.resize(count, 0);
    s.abs_x.clear();
    s.abs_x.resize(count, 0.0);
    s.abs_y.clear();
    s.abs_y.resize(count, 0.0);
    s.transform.clear();
    s.transform.resize(count, Affine::IDENTITY);
    let mut pass = AttachmentPass {
        st,
        l,
        parents: &s.parents,
        rotated: &s.rotated,
        slots: &s.slots,
        keys: &s.keys,
        state: std::mem::take(&mut s.state),
        abs_x: std::mem::take(&mut s.abs_x),
        abs_y: std::mem::take(&mut s.abs_y),
        transform: std::mem::take(&mut s.transform),
        viewport_w,
        viewport_h,
    };
    for &pi in &s.actual {
        pass.resolve(pi);
    }
    s.state = pass.state;
    s.abs_x = pass.abs_x;
    s.abs_y = pass.abs_y;
    s.transform = pass.transform;
    l.attach = s;
}

/// Solves the document under a bounded width and optionally bounded height.
///
/// The invocation viewport is also the percentage sizing base.
pub fn solve(d: &Doc, st: &mut St, l: &mut Lay, width: f64, height: f64, has_height: bool) -> i32 {
    lay_reset(l);
    let max_height = if has_height { height } else { INF };
    let constraints = Cons {
        min_w: 0.0,
        max_w: width,
        min_h: 0.0,
        max_h: max_height,
        pct_w: width,
        pct_h: height,
        has_pw: true,
        has_ph: has_height,
    };
    let root_inh = inh_root();
    measure(
        d,
        st,
        l,
        0,
        &constraints,
        255,
        false,
        &root_inh,
        false,
        false,
        false,
    )
}

#[cfg(test)]
mod attachment_tests {
    use super::*;
    use crate::{slir, style};

    #[test]
    fn twelve_gravities_place_on_the_expected_edge() {
        use style::Gravity::*;
        let anchor = AttachRect {
            x: 100.0,
            y: 80.0,
            w: 40.0,
            h: 20.0,
        };
        let cases = [
            (BelowStart, (100.0, 100.0)),
            (BelowCenter, (105.0, 100.0)),
            (BelowEnd, (110.0, 100.0)),
            (AboveStart, (100.0, 70.0)),
            (AboveCenter, (105.0, 70.0)),
            (AboveEnd, (110.0, 70.0)),
            (LeftStart, (70.0, 80.0)),
            (LeftCenter, (70.0, 85.0)),
            (LeftEnd, (70.0, 90.0)),
            (RightStart, (140.0, 80.0)),
            (RightCenter, (140.0, 85.0)),
            (RightEnd, (140.0, 90.0)),
        ];
        for (gravity, expected) in cases {
            assert_eq!(
                gravity_position(anchor, 30.0, 10.0, gravity),
                expected,
                "{gravity:?}"
            );
        }
    }

    #[test]
    fn collision_flips_main_then_alignment_before_offset() {
        let anchor = AttachRect {
            x: 140.0,
            y: 105.0,
            w: 10.0,
            h: 10.0,
        };
        // Main axis flips below->above, then the start alignment (which would
        // overflow the right viewport edge) flips to end, keeping the popup
        // attached to its anchor instead of sliding away from it.
        assert_eq!(
            attachment_position(
                anchor,
                50.0,
                30.0,
                style::Gravity::BelowStart,
                true,
                160.0,
                120.0,
                7.0,
                -3.0,
            ),
            (107.0, 72.0)
        );
        assert_eq!(
            attachment_position(
                anchor,
                50.0,
                30.0,
                style::Gravity::BelowStart,
                false,
                160.0,
                120.0,
                0.0,
                0.0,
            ),
            (140.0, 115.0)
        );
    }

    #[test]
    fn alignment_flip_precedes_slide_and_falls_back_when_both_overflow() {
        // W14: a 150u menu end-aligned to a 62u chip near the left edge must
        // flip to start alignment (staying attached) rather than slide.
        let chip = AttachRect {
            x: 48.0,
            y: 40.0,
            w: 62.0,
            h: 24.0,
        };
        assert_eq!(
            attachment_position(
                chip,
                150.0,
                90.0,
                style::Gravity::BelowEnd,
                true,
                640.0,
                480.0,
                0.0,
                0.0,
            ),
            (48.0, 64.0)
        );
        // Center alignment has no flip; it slides.
        assert_eq!(
            attachment_position(
                chip,
                150.0,
                90.0,
                style::Gravity::BelowCenter,
                true,
                150.0,
                480.0,
                0.0,
                0.0,
            ),
            (0.0, 64.0)
        );
        // When both alignments overflow, the authored alignment slides.
        let wide = AttachRect {
            x: 10.0,
            y: 10.0,
            w: 30.0,
            h: 10.0,
        };
        assert_eq!(
            attachment_position(
                wide,
                80.0,
                20.0,
                style::Gravity::BelowEnd,
                true,
                60.0,
                200.0,
                0.0,
                0.0,
            ),
            (0.0, 20.0)
        );
    }

    #[test]
    fn rotated_bounds_and_inverse_parent_mapping_share_paint_geometry() {
        let transform = Affine::rotation(40.0, 70.0, 90.0);
        let anchor = painted_rect(transform, 10.0, 55.0, 20.0, 10.0);
        for (actual, expected) in [
            (anchor.x, 45.0),
            (anchor.y, 40.0),
            (anchor.w, 10.0),
            (anchor.h, 20.0),
        ] {
            assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
        }

        let (raw_x, raw_y) = raw_origin_for_painted_rect(transform, 30.0, 10.0, 30.0, 10.0);
        let popup = painted_rect(transform, raw_x, raw_y, 30.0, 10.0);
        for (actual, expected) in [
            (popup.x, 30.0),
            (popup.y, 10.0),
            (popup.w, 10.0),
            (popup.h, 30.0),
        ] {
            assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
        }
    }

    #[test]
    fn attached_overlay_uses_quarter_turn_targets_painted_bounds() {
        let mut d = slir::doc_new();
        d.strs.extend([
            String::new(),
            "#root".into(),
            "#root/#rotated".into(),
            "#root/#rotated/#anchor".into(),
            "#root/#popup".into(),
        ]);
        d.node_kind
            .extend([slir::K_STACK, slir::K_STACK, slir::K_RECT, slir::K_RECT]);
        d.node_key.extend([1, 2, 3, 4]);

        let mut st = style::st_new();
        st.rs.push(style::rstyle_default(0, slir::K_STACK, 1));
        st.rs.push(style::rstyle_default(1, slir::K_STACK, 2));
        st.rs.push(style::rstyle_default(1, slir::K_STACK, 2));
        st.rs.push(style::rstyle_default(2, slir::K_RECT, 3));
        st.rs.push(style::rstyle_default(3, slir::K_RECT, 4));
        st.rs[1].rotate = 90.0;
        st.rs[4].has_attach = true;
        st.rs[4].attach = "#root/#rotated/#anchor".into();
        st.rs[4].gravity = style::Gravity::RightStart;
        st.rs[4].collide_auto = false;

        let mut l = lay_new();
        let root = p_new(&mut l, 0, 0);
        let outer = p_new(&mut l, 1, 1);
        let inner = p_new(&mut l, 1, 2);
        let anchor = p_new(&mut l, 2, 3);
        let popup = p_new(&mut l, 3, 4);
        l.p_w[idx(root)] = 200.0;
        l.p_h[idx(root)] = 120.0;
        l.p_x[idx(outer)] = 20.0;
        l.p_y[idx(outer)] = 30.0;
        l.p_w[idx(outer)] = 40.0;
        l.p_h[idx(outer)] = 80.0;
        l.p_w[idx(inner)] = 80.0;
        l.p_h[idx(inner)] = 40.0;
        l.p_x[idx(anchor)] = 10.0;
        l.p_y[idx(anchor)] = 5.0;
        l.p_w[idx(anchor)] = 20.0;
        l.p_h[idx(anchor)] = 10.0;
        l.p_w[idx(popup)] = 30.0;
        l.p_h[idx(popup)] = 10.0;
        l.p_rot[idx(outer)] = inner;
        l.child_pool.extend([outer, popup, anchor]);
        l.p_child_off[idx(root)] = 0;
        l.p_child_len[idx(root)] = 2;
        l.p_child_off[idx(inner)] = 2;
        l.p_child_len[idx(inner)] = 1;

        place_attached(&d, &st, &mut l, root, 200.0, 120.0);

        assert!((l.p_x[idx(popup)] - 55.0).abs() < 1e-9);
        assert!((l.p_y[idx(popup)] - 40.0).abs() < 1e-9);
    }

    #[test]
    fn attached_children_do_not_expand_overlay_hug_extents() {
        let mut st = style::st_new();
        st.rs.push(style::rstyle_default(0, slir::K_RECT, 1));
        st.rs.push(style::rstyle_default(1, slir::K_RECT, 2));
        st.rs[1].has_attach = true;

        let mut l = lay_new();
        let ordinary = p_new(&mut l, 0, 0);
        let attached = p_new(&mut l, 1, 1);
        l.p_x[idx(ordinary)] = 5.0;
        l.p_y[idx(ordinary)] = 7.0;
        l.p_w[idx(ordinary)] = 40.0;
        l.p_h[idx(ordinary)] = 20.0;
        l.p_x[idx(attached)] = 80.0;
        l.p_y[idx(attached)] = 90.0;
        l.p_w[idx(attached)] = 200.0;
        l.p_h[idx(attached)] = 300.0;
        let placements = [ordinary, attached];

        assert_eq!(overlay_hug_extent(&st, &l, &placements, true, false), 40.0);
        assert_eq!(overlay_hug_extent(&st, &l, &placements, false, false), 20.0);
        assert_eq!(overlay_hug_extent(&st, &l, &placements, true, true), 45.0);
        assert_eq!(overlay_hug_extent(&st, &l, &placements, false, true), 27.0);
    }

    #[test]
    fn prop_ref_attachment_retains_presence_after_text_resolution() {
        let mut d = crate::test_list::list_doc();
        let prop_label = crate::test_list::aval(&mut d, slir::T_PROP_REF, 0, 0, 0.0);
        d.attr_id.insert(2, slir::A_ATTACH);
        d.attr_val.insert(2, prop_label);
        d.attr_index[2] = 3;
        d.attr_index[3] = 4;

        let mut st = crate::test_list::fresh(&d);
        let mut roots = Vec::new();
        crate::test_list::roots(&d, &mut st, &mut roots);
        let row = roots[0];
        let ri = style::build_rstyle(
            &d, &mut st, row, 255, false, 0, 1, 0, 14.0, 400.0, 1.2, 0.0, false,
        );
        let rule = &st.rs[idx(ri)];

        assert!(rule.has_attach);
        assert_eq!(rule.attach, "A");
    }

    #[test]
    fn attached_children_do_not_trigger_implicit_boundary_clipping() {
        let d = slir::doc_new();
        let mut st = style::st_new();
        st.rs.push(style::rstyle_default(0, slir::K_STACK, 1));
        st.rs.push(style::rstyle_default(1, slir::K_RECT, 2));
        st.rs[1].has_attach = true;

        let mut l = lay_new();
        let parent = p_new(&mut l, 0, 0);
        let attached = p_new(&mut l, 1, 1);
        l.p_w[idx(parent)] = 20.0;
        l.p_h[idx(parent)] = 20.0;
        l.p_w[idx(attached)] = 100.0;
        l.p_h[idx(attached)] = 50.0;
        l.p_child_off[idx(parent)] = 0;
        l.p_child_len[idx(parent)] = 1;
        l.child_pool.push(attached);

        boundary(&d, &mut st, &mut l, parent);
        assert!(!l.p_clip[idx(parent)]);

        st.rs[0].flags = slir::F_CLIP;
        boundary(&d, &mut st, &mut l, parent);
        assert!(l.p_clip[idx(parent)]);
    }

    fn attachment_fixture(scroll: f64) -> (Doc, St, Lay, usize) {
        let mut d = slir::doc_new();
        d.strs.extend([
            String::new(),
            "#surface".into(),
            "#surface/#scroller".into(),
            "#surface/#scroller/#first".into(),
            "#surface/#scroller/#second".into(),
            "#surface/#popup".into(),
        ]);
        d.node_kind.extend([
            slir::K_STACK,
            slir::K_COL,
            slir::K_RECT,
            slir::K_RECT,
            slir::K_RECT,
        ]);
        d.node_key.extend([1, 2, 3, 4, 5]);

        let mut st = style::st_new();
        st.rs.push(style::rstyle_default(0, slir::K_STACK, 1));
        st.rs.push(style::rstyle_default(1, slir::K_COL, 2));
        st.rs.push(style::rstyle_default(2, slir::K_RECT, 3));
        st.rs.push(style::rstyle_default(3, slir::K_RECT, 4));
        st.rs.push(style::rstyle_default(4, slir::K_RECT, 5));
        st.rs[1].flags = slir::F_SCROLL;
        st.rs[4].has_attach = true;
        st.rs[4].attach = "#surface/#scroller/#first".into();
        st.rs[4].collide_auto = false;
        st.scroll_node.push(1);
        st.scroll_off.push(scroll);

        let mut l = lay_new();
        let root = p_new(&mut l, 0, 0);
        let scroller = p_new(&mut l, 1, 1);
        let first = p_new(&mut l, 2, 2);
        let second = p_new(&mut l, 3, 3);
        let popup = p_new(&mut l, 4, 4);
        l.p_w[idx(root)] = 200.0;
        l.p_h[idx(root)] = 120.0;
        l.p_w[idx(scroller)] = 100.0;
        l.p_h[idx(scroller)] = 80.0;
        l.p_x[idx(first)] = 10.0;
        l.p_y[idx(first)] = 60.0;
        l.p_w[idx(first)] = 20.0;
        l.p_h[idx(first)] = 10.0;
        l.p_x[idx(second)] = 50.0;
        l.p_y[idx(second)] = 100.0;
        l.p_w[idx(second)] = 20.0;
        l.p_h[idx(second)] = 10.0;
        l.p_w[idx(popup)] = 30.0;
        l.p_h[idx(popup)] = 15.0;
        l.child_pool.extend([scroller, popup, first, second]);
        l.p_child_off[idx(root)] = 0;
        l.p_child_len[idx(root)] = 2;
        l.p_child_off[idx(scroller)] = 2;
        l.p_child_len[idx(scroller)] = 2;
        (d, st, l, idx(popup))
    }

    #[test]
    fn dynamic_targets_follow_scroll_and_missing_targets_disappear() {
        let (d, mut st, base, popup) = attachment_fixture(25.0);

        let mut first = base.clone();
        place_attached(&d, &st, &mut first, 0, 200.0, 120.0);
        assert_eq!((first.p_x[popup], first.p_y[popup]), (10.0, 45.0));
        assert!(!first.p_skip[popup]);

        st.rs[4].attach = "#surface/#scroller/#second".into();
        let mut second = base.clone();
        place_attached(&d, &st, &mut second, 0, 200.0, 120.0);
        assert_eq!((second.p_x[popup], second.p_y[popup]), (50.0, 85.0));

        st.rs[4].attach = "#surface/#scroller/#first".into();
        st.scroll_off[0] = 40.0;
        let mut followed = base.clone();
        place_attached(&d, &st, &mut followed, 0, 200.0, 120.0);
        assert_eq!((followed.p_x[popup], followed.p_y[popup]), (10.0, 30.0));

        st.rs[2].flags = slir::F_STICKY;
        st.scroll_off[0] = 80.0;
        let mut sticky = base.clone();
        place_attached(&d, &st, &mut sticky, 0, 200.0, 120.0);
        assert_eq!((sticky.p_x[popup], sticky.p_y[popup]), (10.0, 10.0));

        st.rs[4].attach = "#surface/#popup".into();
        let mut cyclic = base.clone();
        place_attached(&d, &st, &mut cyclic, 0, 200.0, 120.0);
        assert!(cyclic.p_skip[popup]);

        st.rs[4].attach = "#surface/#missing".into();
        let mut missing = base.clone();
        place_attached(&d, &st, &mut missing, 0, 200.0, 120.0);
        assert!(missing.p_skip[popup]);

        st.rs[4].attach.clear();
        let mut empty = base;
        place_attached(&d, &st, &mut empty, 0, 200.0, 120.0);
        assert!(empty.p_skip[popup]);
    }

    #[test]
    fn tui_client_snaps_anchored_overlay_origins_to_the_cell_grid() {
        // A fractional scroll offset drops the anchor off the 8x16 cell grid;
        // on the tui client the overlay origin snaps back to whole cells so
        // its borders align with content rows (T9/C-13).
        let (d, mut st, base, popup) = attachment_fixture(22.5);
        let mut web = base.clone();
        place_attached(&d, &st, &mut web, 0, 200.0, 120.0);
        assert_eq!(
            (web.p_x[popup], web.p_y[popup]),
            (10.0, 47.5),
            "non-tui clients keep exact fractional placement"
        );

        st.env.client = crate::when::CLIENT_TUI;
        let mut tui = base;
        place_attached(&d, &st, &mut tui, 0, 200.0, 120.0);
        assert_eq!(
            (tui.p_x[popup], tui.p_y[popup]),
            (8.0, 48.0),
            "tui placement snaps the overlay origin to 8x16 cells"
        );
    }

    #[test]
    fn suppression_is_final_before_sticky_anchor_geometry() {
        let (mut d, mut st, mut l, popup) = attachment_fixture(80.0);
        d.node_kind[1] = slir::K_STACK;
        st.rs[1].kind = slir::K_STACK;
        st.rs[2].flags = slir::F_STICKY;
        st.rs[3].flags = slir::F_STICKY;
        st.rs[3].has_attach = true;
        st.rs[3].attach = "#surface/#missing".into();
        l.p_y[3] = 65.0;

        place_attached(&d, &st, &mut l, 0, 200.0, 120.0);

        assert!(l.p_skip[3]);
        assert_eq!((l.p_x[popup], l.p_y[popup]), (10.0, 10.0));
    }
}

#[cfg(test)]
mod paragraph_tests {
    use super::*;
    use crate::{slir, style, test_list};

    #[test]
    fn adjacent_runs_with_different_tracking_stay_separate() {
        let mut d = slir::doc_new();
        d.ok = true;
        d.strs
            .extend([String::new(), "alpha".into(), "beta".into()]);
        let alpha = test_list::aval(&mut d, slir::T_STR, 1, 0, 0.0);
        let beta = test_list::aval(&mut d, slir::T_STR, 2, 0, 0.0);
        let tracked = test_list::aval(&mut d, slir::T_NUM, 0, 0, 2.0);
        d.node_kind
            .extend([slir::K_PARA, slir::K_SPAN, slir::K_SPAN]);
        d.node_flags.extend([0, 0, 0]);
        d.node_parent.extend([slir::NONE, 0, 0]);
        d.node_first.extend([1, slir::NONE, slir::NONE]);
        d.node_next.extend([slir::NONE, 2, slir::NONE]);
        d.node_key.extend([0, 0, 0]);
        d.node_id.extend([0, 0, 0]);
        d.node_line.extend([1, 2, 3]);
        d.attr_index.extend([0, 0, 1, 3]);
        d.attr_id
            .extend([slir::A_CONTENT, slir::A_CONTENT, slir::A_TRACKING]);
        d.attr_val.extend([alpha, beta, tracked]);

        let mut st = style::st_new();
        style::init_params(&d, &mut st);
        style::begin_solve(&d, &mut st);
        let mut lay = lay_new();
        let root = solve(&d, &mut st, &mut lay, 500.0, 100.0, true);
        let paragraph = lay.p_para[idx(root)];
        let line = lay.para_line_off[idx(paragraph)];

        assert_eq!(lay.pl_seg_len[idx(line)], 2);
        assert_eq!(lay.seg_tracking, [0.0, 2.0]);
    }
}
