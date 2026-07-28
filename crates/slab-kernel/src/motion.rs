//! Motion overlays interpolate animation inputs before each normal layout solve.
//!
//! Each intermediate frame is solved like any other document, so containment and
//! other layout invariants hold throughout an animation. Transitions use one
//! kernel-managed clock per state patch: a state flip records the solve time, and
//! the patch value is blended with its base while `t - flip - delay < duration`.
//! Entering uses the eased progress as its weight; leaving uses its complement.
//! Attributes without a base step at the midpoint. Flags and conditional children
//! are always discrete.

use crate::{color, ease, list, slir, style, value};
use rustc_hash::FxHashMap as HashMap;
use slir::Doc;
use style::St;
use value::V;

fn index(i: i32) -> usize {
    usize::try_from(i).expect("negative motion pool index")
}

/// Returns the raw cycle position in `[0, 1]` at an absolute time.
///
/// Mode `0` loops, mode `1` runs once, and mode `2` alternates direction.
pub fn cycle_progress(t_ms: f64, dur: f64, mode: u32, delay: f64) -> f64 {
    if dur <= 0.0 {
        return 1.0;
    }
    let t = t_ms - delay;
    if t <= 0.0 {
        return 0.0;
    }
    if mode == 1 {
        return 1.0f64.min(t / dur);
    }

    let cycles = t / dur;
    let frac = cycles - cycles.floor();
    if mode == 2 {
        let even = (cycles / 2.0).floor() * 2.0 == cycles.floor();
        return if even { frac } else { 1.0 - frac };
    }
    frac
}

/// Applies the easing identified by its SLIR code.
///
/// Codes `0` through `3` mean linear, ease-in, ease-out, and ease-in-out.
pub fn ease_code(code: u32, t: f64) -> f64 {
    match code {
        1 => ease::ease_in(t),
        2 => ease::ease_out(t),
        3 => ease::ease_in_out(t),
        _ => ease::linear(t),
    }
}

/// Reverses the bytes between SLIR's low-byte-red RGBA packing and the color
/// module's `0xRRGGBBAA` packing. The conversion is its own inverse.
pub fn rgba_swap(c: u32) -> u32 {
    ((c & 0xff).wrapping_shl(24))
        | ((c.wrapping_shr(8) & 0xff).wrapping_shl(16))
        | ((c.wrapping_shr(16) & 0xff).wrapping_shl(8))
        | (c.wrapping_shr(24) & 0xff)
}

/// Perceptually interpolates two SLIR-packed RGBA words in OKLab.
pub fn lerp_rgba(a: u32, b: u32, f: f64) -> u32 {
    rgba_swap(color::lerp_oklab(rgba_swap(a), rgba_swap(b), f))
}

/// Reports whether a value tag participates in numeric interpolation.
pub fn is_numlike(tag: u32) -> bool {
    tag == slir::T_NUM || tag == slir::T_SIZE_FIXED
}

/// Reports whether a value tag participates in percentage interpolation.
pub fn is_pctlike(tag: u32) -> bool {
    tag == slir::T_PCT || tag == slir::T_SIZE_PCT
}

/// Reports whether a value tag participates in color interpolation.
pub fn is_colorlike(tag: u32) -> bool {
    tag == slir::T_COLOR || tag == slir::T_PAINT_SOLID
}

/// Reports whether a value tag participates in tuple interpolation.
pub fn is_tuplelike(tag: u32) -> bool {
    style::is_tuple_v(tag)
}

/// Resolves a parameter reference used as a keyframe or patch endpoint.
///
/// This mirrors style attribute resolution so interpolation always receives a
/// concrete value.
pub fn pv(d: &Doc, st: &St, v: &V) -> V {
    let resolved = if v.tag == slir::T_TOKEN_REF {
        value::decode_active(d, st.theme_index, value::token_aval(d, st.theme_index, v.h))
    } else {
        v.clone()
    };
    if resolved.tag != slir::T_PARAM_REF {
        return resolved;
    }

    let parameter = index(i32::try_from(resolved.h).expect("parameter index exceeds i32"));
    match d.parm_type[parameter] {
        1 | 4 => V {
            tag: slir::T_NUM,
            num: st.pv_num[parameter],
            h: 0,
            off: 0,
            ln: 0,
        },
        2 => V {
            tag: slir::T_PCT,
            num: st.pv_num[parameter],
            h: 0,
            off: 0,
            ln: 0,
        },
        3 => V {
            tag: slir::T_COLOR,
            num: 0.0,
            h: st.pv_h[parameter],
            off: 0,
            ln: 0,
        },
        _ => resolved,
    }
}

/// Interpolates two resolved values.
///
/// Numbers and percentages interpolate linearly, colors interpolate in OKLab,
/// and equal-length tuples interpolate elementwise into the motion tuple pool.
/// Discrete and mismatched values hold the earlier stop.
pub fn lerp_v(d: &Doc, st: &mut St, a: &V, b: &V, f: f64) -> V {
    if f <= 0.0 {
        return a.clone();
    }
    if f >= 1.0 {
        return b.clone();
    }
    if is_numlike(a.tag) && is_numlike(b.tag) {
        return V {
            tag: slir::T_NUM,
            num: a.num + (b.num - a.num) * f,
            h: 0,
            off: 0,
            ln: 0,
        };
    }
    if is_pctlike(a.tag) && is_pctlike(b.tag) {
        return V {
            tag: slir::T_PCT,
            num: a.num + (b.num - a.num) * f,
            h: 0,
            off: 0,
            ln: 0,
        };
    }
    if is_colorlike(a.tag) && is_colorlike(b.tag) {
        return V {
            tag: slir::T_COLOR,
            num: 0.0,
            h: lerp_rgba(a.h, b.h, f),
            off: 0,
            ln: 0,
        };
    }
    if is_tuplelike(a.tag) && is_tuplelike(b.tag) && a.ln == b.ln {
        let off = i32::try_from(st.mo_f.len()).expect("motion tuple pool exceeds i32");
        for k in 0..a.ln {
            let av = style::tup_at(d, st, a, k);
            let bv = style::tup_at(d, st, b, k);
            st.mo_f.push(av + (bv - av) * f);
        }
        return V {
            tag: style::T_OV_TUPLE,
            num: 0.0,
            h: 0,
            off,
            ln: a.ln,
        };
    }
    a.clone()
}

/// Returns an attribute's value at an eased keyframe cycle position.
///
/// Positions outside the attribute's first and last appearances clamp to the
/// nearest defined stop. A missing value means the animation never mentions the
/// attribute.
pub fn keyframe_v(d: &Doc, st: &mut St, anim: i32, attr: u32, p: f64) -> V {
    let animation = index(anim);
    let stop_start = d.anim_stop_off[animation];
    let stop_end = stop_start.wrapping_add(d.anim_stop_len[animation]);
    let mut previous: Option<(f64, i32)> = None;

    for stop in stop_start..stop_end {
        let stop_ix = index(stop);
        let attr_start = d.anim_stop_attr_off[stop_ix];
        let attr_end = attr_start.wrapping_add(d.anim_stop_attr_len[stop_ix]);
        for entry in attr_start..attr_end {
            let entry_ix = index(entry);
            if d.aattr_id[entry_ix] != attr {
                continue;
            }

            let current = (
                d.anim_stop_pos[stop_ix],
                i32::try_from(d.aattr_val[entry_ix]).expect("attribute value index exceeds i32"),
            );
            let Some((previous_pos, previous_ix)) = previous else {
                if p <= current.0 {
                    return pv(d, st, &value::decode(d, current.1));
                }
                previous = Some(current);
                continue;
            };
            if p <= current.0 {
                let span = current.0 - previous_pos;
                let f = if span > 0.0 {
                    (p - previous_pos) / span
                } else {
                    0.0
                };
                let a = pv(d, st, &value::decode(d, previous_ix));
                let b = pv(d, st, &value::decode(d, current.1));
                return lerp_v(d, st, &a, &b, f);
            }
            previous = Some(current);
        }
    }

    previous
        .map(|(_, value_ix)| pv(d, st, &value::decode(d, value_ix)))
        .unwrap_or_else(value::missing)
}

/// A resolved scalar transition endpoint: value tag, number, and packed color.
///
/// Numbers and percentages live in `num`; colors and solid paints live in
/// `rgba`. The tag decides which half is meaningful.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VEnd {
    /// Resolved value tag (`T_NUM`, `T_PCT`, `T_COLOR`, …).
    pub tag: u32,
    /// Numeric payload for number- and percent-like tags.
    pub num: f64,
    /// SLIR-packed RGBA payload for color-like tags.
    pub rgba: u32,
}

/// One host-value transition clock.
///
/// A `transition=` node whose resolved base numeric/pct/color attribute
/// changes between solves (host param writes, theme flips) tweens from the
/// previously painted value instead of snapping. The clock is stamped with
/// the observing solve's time, exactly like state-flip clocks.
#[derive(Clone, Debug)]
pub struct VClock {
    /// Document node carrying the `transition=`.
    pub node: u32,
    /// Attribute identifier on that node.
    pub attr: u32,
    /// Resolved value observed at the latest solve (the tween target).
    pub last: VEnd,
    /// Tween origin captured when the value last changed.
    pub from: VEnd,
    /// Solve time of the latest observed change, or [`NEVER`].
    pub flip: f64,
}

/// Motion state retained between solves.
#[derive(Clone, Debug)]
pub struct MSt {
    /// Whether the authored patch clocks have been initialized.
    pub inited: bool,
    /// Activity of each authored patch at the previous solve.
    pub p_last: Vec<bool>,
    /// Activity of each authored patch before its latest flip.
    pub p_prev: Vec<bool>,
    /// Time of each authored patch's latest flip.
    pub p_flip: Vec<f64>,
    /// Synthetic node half of each `(node, patch)` transition-clock key.
    pub sp_node: Vec<u32>,
    /// Patch half of each synthetic transition-clock key.
    pub sp_patch: Vec<i32>,
    /// Latest activity for each synthetic transition clock.
    pub sp_last: Vec<bool>,
    /// Activity before each synthetic transition clock's latest flip.
    pub sp_prev: Vec<bool>,
    /// Time of each synthetic transition clock's latest flip.
    pub sp_flip: Vec<f64>,
    sp_slot: HashMap<(u32, i32), usize>,
    sp_by_node: HashMap<u32, Vec<i32>>,
    /// Host-value transition clocks for `transition=` nodes, keyed by
    /// `(node, attr)`; see [`VClock`].
    pub v_clocks: Vec<VClock>,
    v_slot: HashMap<(u32, u32), usize>,
    /// Reusable attribute-order scratch for the per-solve overlay loops.
    scratch_attrs: Vec<u32>,
    /// Bindings the driver replays natively (see [`lifts`]); indexed by
    /// binding, empty until a driver lifts. Lifted bindings contribute no
    /// overlay and no activity.
    pub lifted: Vec<bool>,
    /// Nodes that require a stable browser-owned compositing wrapper for
    /// lifted animation channels; indexed by document node.
    pub lift_node: Vec<bool>,
    /// Nodes whose lifted `bg` track needs a transparent rect paint target;
    /// indexed by document node.
    pub lift_bg: Vec<bool>,
    /// Whether motion requires another frame after the current solve.
    pub active: bool,
}

/// Sentinel flip time indicating that a patch has never flipped.
pub const NEVER: f64 = -1.0e30;

#[cfg(test)]
thread_local! {
    static SYNTHETIC_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[inline]
fn note_synthetic_work() {
    #[cfg(test)]
    SYNTHETIC_WORK.with(|work| work.set(work.get().saturating_add(1)));
}

#[cfg(test)]
pub fn reset_synthetic_work() {
    SYNTHETIC_WORK.with(|work| work.set(0));
}

#[cfg(test)]
pub fn synthetic_work() -> usize {
    SYNTHETIC_WORK.with(std::cell::Cell::get)
}

/// Creates empty motion state for a new instance.
pub fn mst_new() -> MSt {
    MSt {
        inited: false,
        p_last: Vec::new(),
        p_prev: Vec::new(),
        p_flip: Vec::new(),
        sp_node: Vec::new(),
        sp_patch: Vec::new(),
        sp_last: Vec::new(),
        sp_prev: Vec::new(),
        sp_flip: Vec::new(),
        sp_slot: HashMap::default(),
        sp_by_node: HashMap::default(),
        v_clocks: Vec::new(),
        v_slot: HashMap::default(),
        scratch_attrs: Vec::new(),
        lifted: Vec::new(),
        lift_node: Vec::new(),
        lift_bg: Vec::new(),
        active: false,
    }
}

/// Returns the last value index for an attribute in a patch, or `-1` when absent.
pub fn patch_attr_ix(d: &Doc, pi: i32, attr: u32) -> i32 {
    let patch = index(pi);
    let start = d.patch_attr_off[patch];
    let end = start.wrapping_add(d.patch_attr_len[patch]);
    let mut found = -1;
    for entry in start..end {
        let entry = index(entry);
        if d.wattr_id[entry] == attr {
            found = i32::try_from(d.wattr_val[entry]).expect("patch value index exceeds i32");
        }
    }
    found
}

/// Finds a synthetic transition clock by its indexed `(node, patch)` key.
pub fn sp_find(ms: &MSt, node: u32, pi: i32) -> i32 {
    note_synthetic_work();
    ms.sp_slot.get(&(node, pi)).map_or(-1, |&clock| {
        i32::try_from(clock).expect("synthetic clock index exceeds i32")
    })
}

fn sp_push(ms: &mut MSt, node: u32, pi: i32, current: bool) {
    let clock = ms.sp_node.len();
    let previous = ms.sp_slot.insert((node, pi), clock);
    debug_assert!(previous.is_none(), "duplicate synthetic transition clock");
    ms.sp_by_node.entry(node).or_default().push(pi);
    ms.sp_node.push(node);
    ms.sp_patch.push(pi);
    ms.sp_last.push(current);
    ms.sp_prev.push(current);
    ms.sp_flip.push(NEVER);
}

fn sp_remove_node(ms: &mut MSt, node: u32) {
    let Some(patches) = ms.sp_by_node.remove(&node) else {
        return;
    };
    for patch in patches {
        let Some(clock) = ms.sp_slot.remove(&(node, patch)) else {
            continue;
        };
        ms.sp_node.swap_remove(clock);
        ms.sp_patch.swap_remove(clock);
        ms.sp_last.swap_remove(clock);
        ms.sp_prev.swap_remove(clock);
        ms.sp_flip.swap_remove(clock);
        if clock < ms.sp_node.len() {
            ms.sp_slot
                .insert((ms.sp_node[clock], ms.sp_patch[clock]), clock);
        }
    }
}

fn prune_deleted_synthetic_clocks(st: &mut St, ms: &mut MSt) {
    for node in list::take_deleted_synthetic(&mut st.lists) {
        sp_remove_node(ms, node);
    }
}

/// Samples state flips only for currently materialized synthetic identities.
pub fn sync_synthetic_clocks(d: &Doc, st: &St, ms: &mut MSt, t: f64) {
    for &node in list::materialized(&st.lists) {
        note_synthetic_work();
        let base = list::base(&st.lists, d, node);
        let base_index = usize::try_from(base).expect("node index exceeds usize");
        for &patch in &st.lists.patches_by_node[base_index] {
            if d.cond_kind
                [index(i32::try_from(d.patch_cond[patch]).expect("condition index exceeds i32"))]
                != slir::C_STATE
            {
                continue;
            }
            let pi = i32::try_from(patch).expect("patch index exceeds i32");
            let current = style::patch_on_for(d, st, pi, node);
            let clock = sp_find(ms, node, pi);
            if clock < 0 {
                sp_push(ms, node, pi, current);
            } else {
                let clock = index(clock);
                if current != ms.sp_last[clock] {
                    ms.sp_prev[clock] = ms.sp_last[clock];
                    ms.sp_last[clock] = current;
                    ms.sp_flip[clock] = t;
                }
            }
        }
    }
}

/// Builds a transition overlay for one synthetic list instance.
///
/// Returns whether any patch on the instance remains in flight.
pub fn synthetic_transition(d: &Doc, st: &mut St, ms: &MSt, tr: i32, node: u32, t: f64) -> bool {
    synthetic_transition_scratch(d, st, ms, tr, node, t, &mut Vec::new())
}

fn synthetic_transition_scratch(
    d: &Doc,
    st: &mut St,
    ms: &MSt,
    tr: i32,
    node: u32,
    t: f64,
    attrs: &mut Vec<u32>,
) -> bool {
    let transition = index(tr);
    let base = list::base(&st.lists, d, node);
    let base_index = usize::try_from(base).expect("node index exceeds usize");
    let patch_count = st.lists.patches_by_node[base_index].len();
    let dur = d.trans_dur[transition];
    if dur <= 0.0 {
        return false;
    }
    let delay = d.trans_delay[transition];
    let any = (0..patch_count).any(|position| {
        let patch = st.lists.patches_by_node[base_index][position];
        let pi = i32::try_from(patch).expect("patch index exceeds i32");
        let clock = sp_find(ms, node, pi);
        clock >= 0 && {
            let clock = index(clock);
            ms.sp_flip[clock] > NEVER
                && ms.sp_prev[clock] != ms.sp_last[clock]
                && t - ms.sp_flip[clock] - delay < dur
        }
    });
    if !any {
        return false;
    }

    attrs.clear();
    for position in 0..patch_count {
        let patch = st.lists.patches_by_node[base_index][position];
        let start = d.patch_attr_off[patch];
        let end = start.wrapping_add(d.patch_attr_len[patch]);
        for entry in start..end {
            let attr = d.wattr_id[index(entry)];
            if attr != slir::A_FLAGS && !attrs.contains(&attr) {
                attrs.push(attr);
            }
        }
    }

    for &attr in attrs.iter() {
        let base_ix = slir::base_attr(d, base, attr);
        let mut has_value = base_ix >= 0;
        let mut current = pv(d, st, &value::decode(d, base_ix));
        let mut tweened = false;
        for position in 0..patch_count {
            let patch = st.lists.patches_by_node[base_index][position];
            let pi = i32::try_from(patch).expect("patch index exceeds i32");
            let target_ix = patch_attr_ix(d, pi, attr);
            if target_ix < 0 {
                continue;
            }
            let clock = sp_find(ms, node, pi);
            let in_flight = if clock >= 0 {
                let clock = index(clock);
                ms.sp_flip[clock] > NEVER && ms.sp_prev[clock] != ms.sp_last[clock]
            } else {
                false
            };
            let mut weight = 0.0;
            let flight = if in_flight {
                let clock = index(clock);
                let age = t - ms.sp_flip[clock] - delay;
                if age < dur {
                    tweened = true;
                    let progress = ease_code(d.trans_easing[transition], 0.0f64.max(age) / dur);
                    weight = if ms.sp_last[clock] {
                        progress
                    } else {
                        1.0 - progress
                    };
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if flight {
                let target = pv(d, st, &value::decode(d, target_ix));
                if has_value {
                    current = lerp_v(d, st, &current, &target, weight);
                } else if weight >= 0.5 {
                    current = target;
                    has_value = true;
                }
            } else if style::patch_on_for(d, st, pi, node) {
                current = pv(d, st, &value::decode(d, target_ix));
                has_value = true;
            }
        }
        if tweened {
            if has_value {
                style::ov_push(
                    st,
                    node,
                    attr,
                    current.tag,
                    current.num,
                    current.h,
                    current.off,
                    current.ln,
                );
            } else {
                style::ov_push(st, node, attr, value::V_MISSING, 0.0, 0, 0, 0);
            }
        }
    }
    true
}

fn node_has_patch_attr(d: &Doc, node: u32, attr: u32) -> bool {
    d.patch_node.iter().enumerate().any(|(patch, &owner)| {
        if owner != node {
            return false;
        }
        let start = index(d.patch_attr_off[patch]);
        let end = index(d.patch_attr_off[patch].wrapping_add(d.patch_attr_len[patch]));
        d.wattr_id[start..end].contains(&attr)
    })
}

fn animation_binding_active(d: &Doc, st: &St, node: u32, animation: i32) -> bool {
    if !style::attached(d, st, node) {
        return false;
    }
    let base = list::base(&st.lists, d, node);
    let uses_channel = slir::base_attr(d, base, slir::A_ANIMATE) >= 0
        || node_has_patch_attr(d, base, slir::A_ANIMATE);
    if !uses_channel {
        return true;
    }
    let value = value::decode(d, style::attr_ix(d, st, node, slir::A_ANIMATE));
    value.tag == slir::T_STR && value.h == d.anim_name[index(animation)]
}

/// Classifies a resolved value as a host-value transition endpoint.
///
/// Only scalar numbers, percentages, and colors participate; everything else
/// (strings, enums, tuples, gradients) snaps as before.
fn vend_of(resolved: &V) -> Option<VEnd> {
    if is_numlike(resolved.tag) || is_pctlike(resolved.tag) {
        return Some(VEnd {
            tag: resolved.tag,
            num: resolved.num,
            rgba: 0,
        });
    }
    if is_colorlike(resolved.tag) {
        return Some(VEnd {
            tag: resolved.tag,
            num: 0.0,
            rgba: resolved.h,
        });
    }
    None
}

/// Interpolates two host-value endpoints with the transition weight.
fn vend_lerp(from: VEnd, to: VEnd, weight: f64) -> VEnd {
    if weight <= 0.0 {
        return from;
    }
    if weight >= 1.0 {
        return to;
    }
    if is_colorlike(from.tag) && is_colorlike(to.tag) {
        return VEnd {
            tag: to.tag,
            num: 0.0,
            rgba: lerp_rgba(from.rgba, to.rgba, weight),
        };
    }
    VEnd {
        tag: to.tag,
        num: from.num + (to.num - from.num) * weight,
        rgba: 0,
    }
}

/// Reports whether an active patch on `node` currently writes `attr`, in
/// which case the patch value owns the attribute and the base must not tween
/// underneath it.
fn active_patch_writes(d: &Doc, st: &St, node: u32, attr: u32) -> bool {
    let node_index = usize::try_from(node).expect("node index exceeds usize");
    st.lists.patches_by_node[node_index].iter().any(|&patch| {
        st.patch_on[patch]
            && patch_attr_ix(
                d,
                i32::try_from(patch).expect("patch index exceeds i32"),
                attr,
            ) >= 0
    })
}

/// Detects host-driven base-value changes on `transition=` nodes and pushes
/// the in-flight tween overlays. Returns whether any tween remains in flight.
fn value_transitions(d: &Doc, st: &mut St, ms: &mut MSt, t: f64) -> bool {
    let mut active = false;
    for transition in 0..d.trans_node.len() {
        let dur = d.trans_dur[transition];
        if dur <= 0.0 {
            continue;
        }
        let node = d.trans_node[transition];
        let node_index = usize::try_from(node).expect("node index exceeds usize");
        let delay = d.trans_delay[transition];
        let easing = d.trans_easing[transition];
        let lo = d.attr_index[node_index];
        let hi = d.attr_index[node_index.wrapping_add(1)];
        for entry in lo..hi {
            let entry = index(i32::try_from(entry).expect("attribute index exceeds i32"));
            let attr = d.attr_id[entry];
            if attr == slir::A_FLAGS {
                continue;
            }
            let value_ix = i32::from_ne_bytes(d.attr_val[entry].to_ne_bytes());
            let resolved = pv(d, st, &value::decode(d, value_ix));
            let Some(now) = vend_of(&resolved) else {
                continue;
            };
            let slot = if let Some(&slot) = ms.v_slot.get(&(node, attr)) {
                slot
            } else {
                let slot = ms.v_clocks.len();
                ms.v_slot.insert((node, attr), slot);
                ms.v_clocks.push(VClock {
                    node,
                    attr,
                    last: now,
                    from: now,
                    flip: NEVER,
                });
                slot
            };
            let clock = &mut ms.v_clocks[slot];
            if now != clock.last {
                // Chain from the currently painted value so a write landing
                // mid-tween continues smoothly instead of jumping.
                let age = t - clock.flip - delay;
                clock.from = if clock.flip > NEVER && age < dur {
                    vend_lerp(
                        clock.from,
                        clock.last,
                        ease_code(easing, 0.0f64.max(age) / dur),
                    )
                } else {
                    clock.last
                };
                clock.last = now;
                clock.flip = t;
            }
            let age = t - clock.flip - delay;
            if clock.flip <= NEVER || age >= dur {
                continue;
            }
            let from = clock.from;
            let to = clock.last;
            active = true;
            if active_patch_writes(d, st, node, attr) {
                continue;
            }
            let weight = ease_code(easing, 0.0f64.max(age) / dur);
            let sample = vend_lerp(from, to, weight);
            style::ov_push(
                st,
                node,
                attr,
                sample.tag,
                sample.num,
                if is_colorlike(sample.tag) {
                    sample.rgba
                } else {
                    0
                },
                0,
                0,
            );
        }
    }
    active
}

/// Builds the motion overlay for one solve.
///
/// Call this after [`style::begin_solve`] has seeded patch activity and before
/// layout. The return value is true when a running animation or in-flight
/// transition requires another solve on the next frame.
pub fn apply(d: &Doc, st: &mut St, ms: &mut MSt, t: f64) -> bool {
    ms.active = false;

    // State transitions use per-patch clocks stamped at the instant of a flip.
    if !ms.inited || ms.p_last.len() != d.patch_node.len() {
        ms.p_last.clear();
        ms.p_prev.clear();
        ms.p_flip.clear();
        ms.p_last
            .extend_from_slice(&st.patch_on[..d.patch_node.len()]);
        ms.p_prev
            .extend_from_slice(&st.patch_on[..d.patch_node.len()]);
        ms.p_flip.resize(d.patch_node.len(), NEVER);
        ms.v_clocks.clear();
        ms.v_slot.clear();
        ms.inited = true;
    } else {
        for patch in 0..d.patch_node.len() {
            let condition =
                index(i32::try_from(d.patch_cond[patch]).expect("condition index exceeds i32"));
            if d.cond_kind[condition] == slir::C_STATE && st.patch_on[patch] != ms.p_last[patch] {
                ms.p_prev[patch] = ms.p_last[patch];
                ms.p_flip[patch] = t;
                ms.p_last[patch] = st.patch_on[patch];
            }
        }
    }
    prune_deleted_synthetic_clocks(st, ms);
    sync_synthetic_clocks(d, st, ms, t);

    // Host-value transitions run before the patch folds below so a
    // simultaneous state flip still owns its attributes (last write wins).
    if value_transitions(d, st, ms, t) {
        ms.active = true;
    }

    // Authored transition nodes fold the base and patches in document order.
    for transition in 0..d.trans_node.len() {
        let node = d.trans_node[transition];
        let node_index = usize::try_from(node).expect("node index exceeds usize");
        let patch_count = st.lists.patches_by_node[node_index].len();
        let dur = d.trans_dur[transition];
        if dur <= 0.0 {
            continue;
        }
        let delay = d.trans_delay[transition];
        let any = (0..patch_count).any(|position| {
            let patch = st.lists.patches_by_node[node_index][position];
            let condition =
                index(i32::try_from(d.patch_cond[patch]).expect("condition index exceeds i32"));
            d.cond_kind[condition] == slir::C_STATE
                && ms.p_flip[patch] > NEVER
                && ms.p_prev[patch] != ms.p_last[patch]
                && t - ms.p_flip[patch] - delay < dur
        });
        if !any {
            continue;
        }
        ms.active = true;

        // Preserve first-appearance order for deterministic overlay output.
        let mut attrs = std::mem::take(&mut ms.scratch_attrs);
        attrs.clear();
        for position in 0..patch_count {
            let patch = st.lists.patches_by_node[node_index][position];
            let start = d.patch_attr_off[patch];
            let end = start.wrapping_add(d.patch_attr_len[patch]);
            for entry in start..end {
                let attr = d.wattr_id[index(entry)];
                if attr != slir::A_FLAGS && !attrs.contains(&attr) {
                    attrs.push(attr);
                }
            }
        }

        for &attr in &attrs {
            let base_ix = slir::base_attr(d, node, attr);
            let mut current = pv(d, st, &value::decode(d, base_ix));
            let mut has_value = base_ix >= 0;
            let mut tweened = false;
            for position in 0..patch_count {
                let patch = st.lists.patches_by_node[node_index][position];
                let target_ix = patch_attr_ix(
                    d,
                    i32::try_from(patch).expect("patch index exceeds i32"),
                    attr,
                );
                if target_ix < 0 {
                    continue;
                }
                let target = pv(d, st, &value::decode(d, target_ix));
                let condition =
                    index(i32::try_from(d.patch_cond[patch]).expect("condition index exceeds i32"));
                let is_state = d.cond_kind[condition] == slir::C_STATE;
                let age = t - ms.p_flip[patch] - delay;
                let in_flight = is_state
                    && ms.p_flip[patch] > NEVER
                    && ms.p_prev[patch] != ms.p_last[patch]
                    && age < dur;
                if in_flight {
                    tweened = true;
                    let progress = ease_code(d.trans_easing[transition], 0.0f64.max(age) / dur);
                    let weight = if ms.p_last[patch] {
                        progress
                    } else {
                        1.0 - progress
                    };
                    if has_value {
                        current = lerp_v(d, st, &current, &target, weight);
                    } else if is_colorlike(target.tag) {
                        // A color without a base fades through the target hue at zero alpha,
                        // matching CSS `transparent`, rather than stepping into existence.
                        let transparent = V {
                            h: target.h & 0x00ff_ffff,
                            ..target.clone()
                        };
                        current = lerp_v(d, st, &transparent, &target, weight);
                        has_value = true;
                    } else if weight >= 0.5 {
                        current = target;
                        has_value = true;
                    }
                    // Other values without a base have nothing to blend from and step at 50%.
                } else if st.patch_on[patch] {
                    current = target;
                    has_value = true;
                }
            }
            if tweened {
                if has_value {
                    style::ov_push(
                        st,
                        node,
                        attr,
                        current.tag,
                        current.num,
                        current.h,
                        current.off,
                        current.ln,
                    );
                } else {
                    style::ov_push(st, node, attr, value::V_MISSING, 0.0, 0, 0, 0);
                }
            }
        }
        ms.scratch_attrs = attrs;
    }

    // Template transitions need distinct clocks and overlays per synthetic instance.
    let mut attrs = std::mem::take(&mut ms.scratch_attrs);
    for transition in 0..d.trans_node.len() {
        let base = d.trans_node[transition];
        let tr = i32::try_from(transition).expect("transition index exceeds i32");
        let synthetic_count = list::materialized(&st.lists).len();
        for synthetic in 0..synthetic_count {
            let node = list::materialized(&st.lists)[synthetic];
            note_synthetic_work();
            if list::base(&st.lists, d, node) == base
                && synthetic_transition_scratch(d, st, ms, tr, node, t, &mut attrs)
            {
                ms.active = true;
            }
        }
    }
    ms.scratch_attrs = attrs;

    // Running animations are time-indexed patches applied before layout.
    for binding in 0..d.bind_node.len() {
        if ms.lifted.get(binding).copied().unwrap_or(false) {
            continue;
        }
        let node = d.bind_node[binding];
        let animation = i32::try_from(d.bind_anim[binding]).expect("animation index exceeds i32");
        let mut owner = node;
        while owner != slir::NONE
            && d.node_kind[index(i32::try_from(owner).expect("node index exceeds i32"))]
                != slir::K_EACH
        {
            owner = d.node_parent[index(i32::try_from(owner).expect("node index exceeds i32"))];
        }
        let has_active_target = if owner == slir::NONE {
            animation_binding_active(d, st, node, animation)
        } else {
            list::materialized(&st.lists)
                .iter()
                .copied()
                .any(|synthetic| {
                    list::base(&st.lists, d, synthetic) == node
                        && animation_binding_active(d, st, synthetic, animation)
                })
        };
        if !has_active_target {
            continue;
        }
        let dur = d.bind_dur[binding];
        let mode = d.bind_mode[binding];
        let delay = d.bind_delay[binding];
        let progress = ease_code(d.bind_easing[binding], cycle_progress(t, dur, mode, delay));

        // Preserve each attribute's first-appearance order across animation stops.
        let mut attrs = std::mem::take(&mut ms.scratch_attrs);
        attrs.clear();
        let stop_start = d.anim_stop_off[index(animation)];
        let stop_end = stop_start.wrapping_add(d.anim_stop_len[index(animation)]);
        for stop in stop_start..stop_end {
            let stop = index(stop);
            let attr_start = d.anim_stop_attr_off[stop];
            let attr_end = attr_start.wrapping_add(d.anim_stop_attr_len[stop]);
            for entry in attr_start..attr_end {
                let attr = d.aattr_id[index(entry)];
                if !attrs.contains(&attr) {
                    attrs.push(attr);
                }
            }
        }

        for &attr in &attrs {
            let v = keyframe_v(d, st, animation, attr, progress);
            if v.tag == value::V_MISSING {
                continue;
            }
            if owner == slir::NONE {
                style::ov_push(st, node, attr, v.tag, v.num, v.h, v.off, v.ln);
            } else {
                let synthetic_count = list::materialized(&st.lists).len();
                for synthetic_index in 0..synthetic_count {
                    let synthetic = list::materialized(&st.lists)[synthetic_index];
                    note_synthetic_work();
                    if list::base(&st.lists, d, synthetic) == node
                        && animation_binding_active(d, st, synthetic, animation)
                    {
                        style::ov_push(st, synthetic, attr, v.tag, v.num, v.h, v.off, v.ln);
                    }
                }
            }
        }
        ms.scratch_attrs = attrs;
        if dur > 0.0 && (mode != 1 || t - delay < dur) {
            ms.active = true;
        }
    }
    ms.active
}

/// One normalized keyframe of a liftable binding.
///
/// Stops are sorted, span the full `[0, 1]` cycle in the *time* domain
/// (whole-cycle easing is already folded into the positions and per-segment
/// curves), and carry every animated attribute at every position using the
/// kernel's clamp-and-interpolate keyframe semantics, so drivers translate
/// them 1:1 into native keyframes.
#[derive(Clone, Debug, PartialEq)]
pub struct LiftStop {
    /// Cycle position in `[0, 1]`, measured in time (not eased progress).
    pub pos: f64,
    /// Y control points of the exact `cubic-bezier(1/3, y1, 2/3, y2)` timing
    /// curve for the segment leaving this stop. `(1/3, 2/3)` is linear. The
    /// last stop carries the linear curve. Slab's quadratic easings restricted
    /// to any sub-interval are quadratics, and every monotone quadratic through
    /// `(0,0)`/`(1,1)` is exactly a cubic Bézier with x controls at 1/3 and 2/3.
    pub ctrl: (f64, f64),
    /// Animated `offset` at this position, when the animation drives it.
    pub offset: Option<(f64, f64)>,
    /// Animated `opacity` at this position, when the animation drives it.
    pub opacity: Option<f64>,
    /// Animated `rotate` in degrees (absolute, not a delta), when driven.
    pub rotate: Option<f64>,
    /// Animated scale factors (absolute, not deltas), when driven.
    pub scale: Option<(f64, f64)>,
    /// Animated solid `bg` as a SLIR-packed RGBA word, when driven.
    pub bg: Option<u32>,
    /// Animated text `color` as a SLIR-packed RGBA word, when driven.
    pub color: Option<u32>,
}

/// An animation binding a driver may replay natively (e.g. as a CSS
/// animation) instead of asking the kernel to re-solve every frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Lift {
    /// Index into the document BIND pool.
    pub binding: usize,
    /// The bound node.
    pub node: u32,
    /// The bound node's kind; drivers map color keyframes onto the paint
    /// channel that kind uses natively (rect background, path fill, text ink).
    pub kind: u32,
    /// Cycle duration in milliseconds.
    pub dur: f64,
    /// Start delay in milliseconds.
    pub delay: f64,
    /// Cycle mode: `0` loops, `1` runs once, `2` alternates.
    pub mode: u32,
    /// The node's static base offset. Painted geometry already includes it,
    /// so native translation keyframes are deltas against this value.
    pub base_offset: (f64, f64),
    /// The node's static base rotation in degrees; rotation keyframes are
    /// deltas against it (same-center rotations compose additively).
    pub base_rotate: f64,
    /// The node's static base scale; scale keyframes are per-axis ratios
    /// against it (same-center scales compose multiplicatively).
    pub base_scale: (f64, f64),
    /// Normalized keyframes.
    pub stops: Vec<LiftStop>,
}

/// Classifies every binding a driver can replay natively.
///
/// A binding lifts only when replaying it outside the kernel is
/// indistinguishable from the per-solve overlay:
/// - it animates nothing but ink: `offset`/`opacity` on paint leaves or
///   render-only containers, `rotate`/`scale` on `rect`/`image`/`path`,
///   solid `bg` on plain rects and paths, and text `color`;
/// - every keyframe (and each base used for a delta) is a static literal, so
///   no parameter, list field, or theme flip can change the track;
/// - geometry tracks stay outside `each` and require an interaction-free
///   subtree without patches, signals, actions, scroll, detached paint, holes,
///   or conditional materialization. Paint-only tracks may coexist with those
///   behaviors when patches cannot replace their native paint channel;
/// - the browser replays `offset`/`opacity` on a stable, node-sized group, so
///   container children and multi-op leaves move/fade together and authored
///   opacity is replaced rather than multiplied;
/// - leaf-local transform bases must stay static. A base quarter-turn is
///   unliftable, while an animated rotation may cross the quarter-turn window
///   only for a statically square leaf whose axis swap changes no geometry;
/// - a `bg` lift requires the retained base paint to stay a solid (or absent)
///   fill on an un-smoothed rect, so the native color channel it animates is
///   the one the driver painted.
///
/// Slab easings apply to the whole cycle while native models ease per
/// keyframe segment; [`lifts`] therefore remaps stop positions into the time
/// domain and emits each segment's exact quadratic-restriction Bézier.
/// Color keyframes interpolate in OKLab (§14.2) while native replay lerps
/// sRGB; segments where the two visibly diverge are subdivided until the
/// difference stays within one 8-bit quantization step.
///
/// Bindings sharing a node lift all-or-nothing so per-attribute overlay
/// precedence between them is preserved.
pub fn lifts(d: &Doc) -> Vec<Lift> {
    let mut lifted: Vec<Lift> = (0..d.bind_node.len())
        .filter_map(|binding| lift_of(d, binding))
        .collect();
    let unliftable_nodes: Vec<u32> = (0..d.bind_node.len())
        .filter(|&binding| !lifted.iter().any(|l| l.binding == binding))
        .map(|binding| d.bind_node[binding])
        .collect();
    lifted.retain(|l| !unliftable_nodes.contains(&l.node));
    lifted
}

/// Maximum bisections per keyframe segment when reconciling OKLab color
/// interpolation with a driver's native sRGB lerp (up to 16 sub-segments).
const COLOR_SPLIT_DEPTH: u32 = 4;

/// Inverts [`ease_code`] on `[0, 1]`: the time at which the whole-cycle
/// easing reaches progress `p`. Every Slab easing is strictly monotone.
fn ease_inv(code: u32, p: f64) -> f64 {
    match code {
        1 => p.sqrt(),
        2 => 1.0 - (1.0 - p).sqrt(),
        3 => {
            if p < 0.5 {
                (p / 2.0).sqrt()
            } else {
                1.0 - ((1.0 - p) / 2.0).sqrt()
            }
        }
        _ => p,
    }
}

/// Quadratic coefficients `(α, β)` of the easing piece `e(t) = αt² + βt + γ`
/// covering times at and after `t0` (γ cancels out of segment restrictions).
/// `ease-in-out` callers must never span a segment across `t = 0.5`.
fn ease_quad(code: u32, t0: f64) -> (f64, f64) {
    match code {
        1 => (1.0, 0.0),
        2 => (-1.0, 2.0),
        3 => {
            if t0 < 0.5 {
                (2.0, 0.0)
            } else {
                (-2.0, 4.0)
            }
        }
        _ => (0.0, 1.0),
    }
}

/// Exact cubic-Bézier y controls for the easing restricted to the time
/// segment `[t0, t1]` spanning progress `[p0, p1]`.
///
/// The normalized restriction is the quadratic `f(u) = au² + bu` with
/// `f(1) = 1`; matching polynomial coefficients against the Bézier with x
/// controls at 1/3 and 2/3 gives `y1 = b/3` and `y2 = (a + 2b)/3`.
fn segment_ctrl(code: u32, t0: f64, t1: f64, p0: f64, p1: f64) -> (f64, f64) {
    let (alpha, beta) = ease_quad(code, t0);
    let span = t1 - t0;
    let progress = p1 - p0;
    let a = alpha * span * span / progress;
    let b = (2.0 * alpha * t0 + beta) * span / progress;
    (b / 3.0, (a + 2.0 * b) / 3.0)
}

/// The linear segment curve ([`segment_ctrl`] for `f(u) = u`).
const CTRL_LINEAR: (f64, f64) = (1.0 / 3.0, 2.0 / 3.0);

/// Whether any point of the linear rotation segment `[a, b]` (degrees,
/// unbounded) falls within the kernel's quarter-turn window — ±0.5° of 90° or
/// 270° modulo 360 — where layout re-solves in the rotated bounding box and
/// ink-only native replay would diverge.
fn hits_quarter_turn(a: f64, b: f64) -> bool {
    let lo = a.min(b) - 0.5;
    let hi = a.max(b) + 0.5;
    [90.0, 270.0].iter().any(|target| {
        let k = ((lo - target) / 360.0).ceil();
        target + 360.0 * k <= hi
    })
}

/// Interpolates two SLIR-packed RGBA words the way native drivers do:
/// premultiplied-alpha lerp in gamma-encoded sRGB (the CSS legacy-color
/// rule). Used only to decide where OKLab segments need subdividing.
fn css_lerp_rgba(a: u32, b: u32, f: f64) -> u32 {
    let ch = |c: u32, shift: u32| f64::from(c.wrapping_shr(shift) & 0xff);
    let a1 = ch(a, 24);
    let a2 = ch(b, 24);
    let am = a1 + (a2 - a1) * f;
    let mix = |shift: u32| -> u32 {
        let v = if am > 0.0 {
            (ch(a, shift) * a1 * (1.0 - f) + ch(b, shift) * a2 * f) / am
        } else {
            ch(a, shift) + (ch(b, shift) - ch(a, shift)) * f
        };
        truncate_u32(v.round())
    };
    mix(0)
        | mix(8).wrapping_shl(8)
        | mix(16).wrapping_shl(16)
        | truncate_u32(am.round()).wrapping_shl(24)
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

/// Largest per-channel difference between two packed RGBA words.
fn max_channel_delta(a: u32, b: u32) -> u32 {
    (0..4)
        .map(|i| (a.wrapping_shr(8 * i) & 0xff).abs_diff(b.wrapping_shr(8 * i) & 0xff))
        .max()
        .unwrap_or(0)
}

/// Splits progress segment `[p0, p1]` until native sRGB replay of every color
/// track stays within one quantization step of the kernel's OKLab path, then
/// appends every knot after `p0` to `out`.
fn refine_colors(
    d: &Doc,
    animation: usize,
    color_attrs: &[u32],
    p0: f64,
    p1: f64,
    depth: u32,
    out: &mut Vec<f64>,
) {
    let mid = (p0 + p1) / 2.0;
    let diverges = depth > 0
        && color_attrs.iter().any(|&attr| {
            let start = track_rgba(d, animation, attr, p0);
            let end = track_rgba(d, animation, attr, p1);
            let native = css_lerp_rgba(start, end, 0.5);
            max_channel_delta(native, track_rgba(d, animation, attr, mid)) > 1
        });
    if diverges {
        refine_colors(d, animation, color_attrs, p0, mid, depth - 1, out);
        refine_colors(d, animation, color_attrs, mid, p1, depth - 1, out);
    } else {
        out.push(p1);
    }
}

fn lift_leaf(kind: u32) -> bool {
    matches!(
        kind,
        slir::K_TEXT
            | slir::K_RECT
            | slir::K_IMG
            | slir::K_PATH
            | slir::K_ICON
            | slir::K_PARA
            | slir::K_DIVIDER
            | slir::K_SPACER
    )
}

fn lift_container(kind: u32) -> bool {
    matches!(
        kind,
        slir::K_ROW
            | slir::K_COL
            | slir::K_WRAP
            | slir::K_GRID
            | slir::K_STACK
            | slir::K_CANVAS
            | slir::K_GROUP
    )
}

fn in_subtree(d: &Doc, mut node: u32, root: u32) -> bool {
    loop {
        if node == root {
            return true;
        }
        let parent = d.node_parent[index(i32::try_from(node).expect("node index exceeds i32"))];
        if parent == slir::NONE {
            return false;
        }
        node = parent;
    }
}

/// Geometry replay is observable through retained hit regions, scroll routing,
/// detached paint, holes, or nodes materialized after the CSS clock starts.
fn geometry_subtree_safe(d: &Doc, root: u32) -> bool {
    for candidate in 0..d.node_kind.len() {
        let node = u32::try_from(candidate).expect("node index exceeds u32");
        if !in_subtree(d, node, root) {
            continue;
        }
        let flags = d.node_flags[candidate];
        if matches!(d.node_kind[candidate], slir::K_EACH | slir::K_HOLE)
            || flags
                & (slir::F_SCROLL
                    | slir::F_SCROLL_CROSS
                    | slir::F_FOCUSABLE
                    | slir::F_DETACHED
                    | slir::F_STICKY
                    | slir::F_DRAG_GHOST)
                != 0
            || d.patch_node.contains(&node)
            || d.sign_node.contains(&node)
            || slir::base_attr(d, node, slir::A_ACT) >= 0
            || slir::base_attr(d, node, slir::A_ATTACH) >= 0
        {
            return false;
        }
    }
    true
}

/// Paint-only replay may coexist with unrelated patches. Flags can remove and
/// recreate the CSS clock; paint-channel patches can switch the DOM/SVG
/// channel underneath a lifted color track.
fn paint_patches_safe(d: &Doc, node: u32, attrs: &[u32]) -> bool {
    for (patch, &owner) in d.patch_node.iter().enumerate() {
        if owner != node {
            continue;
        }
        let start = d.patch_attr_off[patch];
        let end = start.wrapping_add(d.patch_attr_len[patch]);
        for entry in start..end {
            let attr = d.wattr_id[index(entry)];
            if attr == slir::A_FLAGS
                || attrs.contains(&slir::A_BG) && matches!(attr, slir::A_BG | slir::A_SMOOTH)
                || attrs.contains(&slir::A_COLOR) && attr == slir::A_COLOR
            {
                return false;
            }
        }
    }
    true
}

fn static_square(d: &Doc, node: u32) -> bool {
    if [slir::A_MIN_W, slir::A_MAX_W, slir::A_MIN_H, slir::A_MAX_H]
        .iter()
        .any(|&attr| slir::base_attr(d, node, attr) >= 0)
    {
        return false;
    }
    let fixed = |attr: u32| {
        let value = value::decode(d, slir::base_attr(d, node, attr));
        matches!(value.tag, slir::T_NUM | slir::T_SIZE_FIXED).then_some(value.num)
    };
    matches!((fixed(slir::A_W), fixed(slir::A_H)), (Some(w), Some(h)) if w == h)
}

/// Builds the lift description for one binding, or `None` when it must stay
/// kernel-driven.
fn lift_of(d: &Doc, binding: usize) -> Option<Lift> {
    let node = d.bind_node[binding];
    if node_has_patch_attr(d, node, slir::A_ANIMATE) {
        return None;
    }
    let dur = d.bind_dur[binding];
    let easing = d.bind_easing[binding];
    if dur <= 0.0 {
        return None;
    }

    let node_index = index(i32::try_from(node).expect("node index exceeds i32"));
    let kind = d.node_kind[node_index];
    let leaf = lift_leaf(kind);
    let container = lift_container(kind);
    if !leaf && !container {
        return None;
    }
    let mut ancestor = d.node_parent[node_index];
    while ancestor != slir::NONE {
        let ai = index(i32::try_from(ancestor).expect("node index exceeds i32"));
        if d.node_kind[ai] == slir::K_EACH {
            return None;
        }
        ancestor = d.node_parent[ai];
    }

    // Animated attributes, in first-appearance order across stops. Every
    // keyframe value must be a static literal of the attribute's shape.
    let mut attrs = Vec::new();
    let animation =
        index(i32::try_from(d.bind_anim[binding]).expect("animation index exceeds i32"));
    let stop_start = d.anim_stop_off[animation];
    let stop_end = stop_start.wrapping_add(d.anim_stop_len[animation]);
    let mut positions = Vec::new();
    let mut rotate_track = Vec::new();
    let mut scale_tuple = None;
    for stop in stop_start..stop_end {
        let stop = index(stop);
        let position = d.anim_stop_pos[stop];
        if positions.contains(&position) && d.anim_stop_attr_len[stop] > 0 {
            // Duplicate positions are step discontinuities native keyframes
            // sample differently at the shared instant.
            return None;
        }
        if !positions.contains(&position) {
            positions.push(position);
        }
        let attr_start = d.anim_stop_attr_off[stop];
        let attr_end = attr_start.wrapping_add(d.anim_stop_attr_len[stop]);
        for entry in attr_start..attr_end {
            let entry = index(entry);
            let attr = d.aattr_id[entry];
            let v = value::decode(
                d,
                i32::try_from(d.aattr_val[entry]).expect("attribute value index exceeds i32"),
            );
            let static_ok = match attr {
                slir::A_OFFSET => v.tag == slir::T_TUPLE && v.ln >= 2,
                slir::A_OPACITY => v.tag == slir::T_NUM,
                slir::A_ROTATE if leaf => v.tag == slir::T_NUM,
                slir::A_SCALE if leaf => {
                    let tuple = v.tag == slir::T_TUPLE && v.ln >= 2;
                    if v.tag != slir::T_NUM && !tuple
                        || scale_tuple.is_some_and(|previous| previous != tuple)
                    {
                        false
                    } else {
                        scale_tuple = Some(tuple);
                        true
                    }
                }
                slir::A_BG | slir::A_COLOR if leaf => is_colorlike(v.tag),
                _ => return None,
            };
            if !static_ok {
                return None;
            }
            if attr == slir::A_ROTATE {
                rotate_track.push(v.num);
            }
            if !attrs.contains(&attr) {
                attrs.push(attr);
            }
        }
    }
    if attrs.is_empty() {
        return None;
    }

    let geometry = attrs
        .iter()
        .any(|attr| matches!(*attr, slir::A_OFFSET | slir::A_ROTATE | slir::A_SCALE));
    if geometry {
        if !geometry_subtree_safe(d, node) {
            return None;
        }
    } else if !paint_patches_safe(d, node, &attrs) {
        return None;
    }

    let base_of = |attr: u32| value::decode(d, slir::base_attr(d, node, attr));
    let transforms_ok = matches!(kind, slir::K_RECT | slir::K_IMG | slir::K_PATH);
    let base_rot = base_of(slir::A_ROTATE);
    let base_sc = base_of(slir::A_SCALE);
    let has_tilt = slir::base_attr(d, node, slir::A_TILT) >= 0;

    if geometry {
        match base_rot.tag {
            value::V_MISSING => {}
            slir::T_NUM if !hits_quarter_turn(base_rot.num, base_rot.num) => {}
            _ => return None,
        }
    }

    let mut base_rotate = 0.0;
    if attrs.contains(&slir::A_ROTATE) {
        if !transforms_ok || has_tilt {
            return None;
        }
        base_rotate = match base_rot.tag {
            value::V_MISSING => 0.0,
            slir::T_NUM => base_rot.num,
            _ => return None,
        };
        // The leaf-local delta sits inside retained scale groups. Only a
        // uniform base scale commutes with that rotation.
        if !matches!(base_sc.tag, value::V_MISSING | slir::T_NUM) {
            return None;
        }
        let quarter = match rotate_track.as_slice() {
            [] => false,
            [only] => hits_quarter_turn(*only, *only),
            track => track.windows(2).any(|w| hits_quarter_turn(w[0], w[1])),
        };
        if quarter && !static_square(d, node) {
            return None;
        }
    }

    let mut base_scale = (1.0, 1.0);
    if attrs.contains(&slir::A_SCALE) {
        if !transforms_ok || has_tilt {
            return None;
        }
        base_scale = match base_sc.tag {
            value::V_MISSING => (1.0, 1.0),
            slir::T_NUM => (base_sc.num, base_sc.num),
            slir::T_TUPLE if base_sc.ln >= 2 => (
                value::tuple_at(d, &base_sc, 0),
                value::tuple_at(d, &base_sc, 1),
            ),
            _ => return None,
        };
        if base_scale.0 == 0.0 || base_scale.1 == 0.0 {
            return None;
        }
    }

    if attrs.contains(&slir::A_BG) {
        match kind {
            // A path's fill is a native paint channel regardless of its base;
            // a rect's is only when the retained fill stays a plain solid
            // (gradients paint as images, smoothing paints as inline vectors).
            slir::K_PATH => {}
            slir::K_RECT => {
                let base_bg = base_of(slir::A_BG);
                if !(base_bg.tag == value::V_MISSING || is_colorlike(base_bg.tag))
                    || slir::base_attr(d, node, slir::A_SMOOTH) >= 0
                {
                    return None;
                }
            }
            _ => return None,
        }
    }

    if attrs.contains(&slir::A_COLOR) {
        let base_color = base_of(slir::A_COLOR);
        if kind != slir::K_TEXT
            || !(base_color.tag == value::V_MISSING || is_colorlike(base_color.tag))
        {
            return None;
        }
    }

    // Base offset must be static too: translation keyframes are deltas.
    let base_offset = if attrs.contains(&slir::A_OFFSET) {
        let base = base_of(slir::A_OFFSET);
        match base.tag {
            value::V_MISSING => (0.0, 0.0),
            slir::T_TUPLE if base.ln >= 2 => {
                (value::tuple_at(d, &base, 0), value::tuple_at(d, &base, 1))
            }
            _ => return None,
        }
    } else {
        (0.0, 0.0)
    };

    positions.sort_by(f64::total_cmp);
    if positions.first() != Some(&0.0) {
        positions.insert(0, 0.0);
    }
    if positions.last() != Some(&1.0) {
        positions.push(1.0);
    }
    if easing == 3 && !positions.contains(&0.5) {
        // Keep every segment inside one quadratic piece of ease-in-out.
        let at = positions.partition_point(|&p| p < 0.5);
        positions.insert(at, 0.5);
    }

    // Where OKLab and native sRGB interpolation visibly disagree, add knots.
    let color_attrs: Vec<u32> = attrs
        .iter()
        .copied()
        .filter(|&attr| attr == slir::A_BG || attr == slir::A_COLOR)
        .collect();
    if !color_attrs.is_empty() {
        let mut refined = vec![positions[0]];
        for pair in positions.windows(2) {
            refine_colors(
                d,
                animation,
                &color_attrs,
                pair[0],
                pair[1],
                COLOR_SPLIT_DEPTH,
                &mut refined,
            );
        }
        positions = refined;
    }

    let times: Vec<f64> = positions.iter().map(|&p| ease_inv(easing, p)).collect();
    let has = |attr: u32| attrs.contains(&attr);
    let stops = positions
        .iter()
        .zip(&times)
        .enumerate()
        .map(|(i, (&pos, &t))| LiftStop {
            pos: t,
            ctrl: if i + 1 < times.len() {
                segment_ctrl(easing, t, times[i + 1], pos, positions[i + 1])
            } else {
                CTRL_LINEAR
            },
            offset: has(slir::A_OFFSET).then(|| {
                (
                    track_num(d, animation, slir::A_OFFSET, pos, 0),
                    track_num(d, animation, slir::A_OFFSET, pos, 1),
                )
            }),
            opacity: has(slir::A_OPACITY)
                .then(|| track_num(d, animation, slir::A_OPACITY, pos, -1)),
            rotate: has(slir::A_ROTATE).then(|| track_num(d, animation, slir::A_ROTATE, pos, -1)),
            scale: has(slir::A_SCALE).then(|| {
                if scale_tuple == Some(true) {
                    (
                        track_num(d, animation, slir::A_SCALE, pos, 0),
                        track_num(d, animation, slir::A_SCALE, pos, 1),
                    )
                } else {
                    let scale = track_num(d, animation, slir::A_SCALE, pos, -1);
                    (scale, scale)
                }
            }),
            bg: has(slir::A_BG).then(|| track_rgba(d, animation, slir::A_BG, pos)),
            color: has(slir::A_COLOR).then(|| track_rgba(d, animation, slir::A_COLOR, pos)),
        })
        .collect();

    Some(Lift {
        binding,
        node,
        kind,
        dur,
        delay: d.bind_delay[binding],
        mode: d.bind_mode[binding],
        base_offset,
        base_rotate,
        base_scale,
        stops,
    })
}

/// Evaluates one numeric component of a static keyframe track at `pos`.
///
/// `component` indexes a tuple element, or `-1` for a scalar. Mirrors
/// [`keyframe_v`]'s clamp-and-interpolate rule without touching solve state;
/// [`lift_of`] has already proven every entry static and numeric.
fn track_num(d: &Doc, animation: usize, attr: u32, pos: f64, component: i32) -> f64 {
    let read = |aval: u32| -> f64 {
        let v = value::decode(
            d,
            i32::try_from(aval).expect("attribute value index exceeds i32"),
        );
        if component < 0 {
            value::num_of(&v, 0.0)
        } else {
            value::tuple_at(d, &v, component)
        }
    };

    let stop_start = d.anim_stop_off[animation];
    let stop_end = stop_start.wrapping_add(d.anim_stop_len[animation]);
    let mut previous: Option<(f64, f64)> = None;
    for stop in stop_start..stop_end {
        let stop = index(stop);
        let attr_start = d.anim_stop_attr_off[stop];
        let attr_end = attr_start.wrapping_add(d.anim_stop_attr_len[stop]);
        for entry in attr_start..attr_end {
            let entry = index(entry);
            if d.aattr_id[entry] != attr {
                continue;
            }
            let current = (d.anim_stop_pos[stop], read(d.aattr_val[entry]));
            let Some((previous_pos, previous_num)) = previous else {
                if pos <= current.0 {
                    return current.1;
                }
                previous = Some(current);
                continue;
            };
            if pos <= current.0 {
                let span = current.0 - previous_pos;
                let f = if span > 0.0 {
                    (pos - previous_pos) / span
                } else {
                    0.0
                };
                return previous_num + (current.1 - previous_num) * f;
            }
            previous = Some(current);
        }
    }
    previous.map(|(_, num)| num).unwrap_or(0.0)
}

/// Evaluates a static color keyframe track at `pos` with the kernel's OKLab
/// clamp-and-interpolate rule, returning the SLIR-packed RGBA word.
fn track_rgba(d: &Doc, animation: usize, attr: u32, pos: f64) -> u32 {
    let read = |aval: u32| -> u32 {
        value::decode(
            d,
            i32::try_from(aval).expect("attribute value index exceeds i32"),
        )
        .h
    };

    let stop_start = d.anim_stop_off[animation];
    let stop_end = stop_start.wrapping_add(d.anim_stop_len[animation]);
    let mut previous: Option<(f64, u32)> = None;
    for stop in stop_start..stop_end {
        let stop = index(stop);
        let attr_start = d.anim_stop_attr_off[stop];
        let attr_end = attr_start.wrapping_add(d.anim_stop_attr_len[stop]);
        for entry in attr_start..attr_end {
            let entry = index(entry);
            if d.aattr_id[entry] != attr {
                continue;
            }
            let current = (d.anim_stop_pos[stop], read(d.aattr_val[entry]));
            let Some((previous_pos, previous_rgba)) = previous else {
                if pos <= current.0 {
                    return current.1;
                }
                previous = Some(current);
                continue;
            };
            if pos <= current.0 {
                let span = current.0 - previous_pos;
                let f = if span > 0.0 {
                    (pos - previous_pos) / span
                } else {
                    0.0
                };
                return lerp_rgba(previous_rgba, current.1, f);
            }
            previous = Some(current);
        }
    }
    previous.map(|(_, rgba)| rgba).unwrap_or(0)
}
