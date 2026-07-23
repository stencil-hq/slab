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
use slir::Doc;
use std::collections::HashMap;
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
    if v.tag != slir::T_PARAM_REF {
        return v.clone();
    }

    let parameter = index(i32::try_from(v.h).expect("parameter index exceeds i32"));
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
        _ => v.clone(),
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
    /// Bindings the driver replays natively (see [`lifts`]); indexed by
    /// binding, empty until a driver lifts. Lifted bindings contribute no
    /// overlay and no activity.
    pub lifted: Vec<bool>,
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
        sp_slot: HashMap::new(),
        sp_by_node: HashMap::new(),
        lifted: Vec::new(),
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
        for patch in 0..d.patch_node.len() {
            if d.patch_node[patch] != base
                || d.cond_kind[index(
                    i32::try_from(d.patch_cond[patch]).expect("condition index exceeds i32"),
                )] != slir::C_STATE
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
    let transition = index(tr);
    let base = list::base(&st.lists, d, node);
    let dur = d.trans_dur[transition];
    if dur <= 0.0 {
        return false;
    }
    let delay = d.trans_delay[transition];
    let any = (0..d.patch_node.len()).any(|patch| {
        if d.patch_node[patch] != base {
            return false;
        }
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

    let mut attrs = Vec::new();
    for patch in 0..d.patch_node.len() {
        if d.patch_node[patch] != base {
            continue;
        }
        let start = d.patch_attr_off[patch];
        let end = start.wrapping_add(d.patch_attr_len[patch]);
        for entry in start..end {
            let attr = d.wattr_id[index(entry)];
            if attr != slir::A_FLAGS && !attrs.contains(&attr) {
                attrs.push(attr);
            }
        }
    }

    for attr in attrs {
        let base_ix = slir::base_attr(d, base, attr);
        let mut has_value = base_ix >= 0;
        let mut current = pv(d, st, &value::decode(d, base_ix));
        let mut tweened = false;
        for patch in 0..d.patch_node.len() {
            if d.patch_node[patch] != base {
                continue;
            }
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

    // Authored transition nodes fold the base and patches in document order.
    for transition in 0..d.trans_node.len() {
        let node = d.trans_node[transition];
        let dur = d.trans_dur[transition];
        if dur <= 0.0 {
            continue;
        }
        let delay = d.trans_delay[transition];
        let any = (0..d.patch_node.len()).any(|patch| {
            let condition =
                index(i32::try_from(d.patch_cond[patch]).expect("condition index exceeds i32"));
            d.patch_node[patch] == node
                && d.cond_kind[condition] == slir::C_STATE
                && ms.p_flip[patch] > NEVER
                && ms.p_prev[patch] != ms.p_last[patch]
                && t - ms.p_flip[patch] - delay < dur
        });
        if !any {
            continue;
        }
        ms.active = true;

        // Preserve first-appearance order for deterministic overlay output.
        let mut attrs = Vec::new();
        for patch in 0..d.patch_node.len() {
            if d.patch_node[patch] != node {
                continue;
            }
            let start = d.patch_attr_off[patch];
            let end = start.wrapping_add(d.patch_attr_len[patch]);
            for entry in start..end {
                let attr = d.wattr_id[index(entry)];
                if attr != slir::A_FLAGS && !attrs.contains(&attr) {
                    attrs.push(attr);
                }
            }
        }

        for attr in attrs {
            let base_ix = slir::base_attr(d, node, attr);
            let mut current = pv(d, st, &value::decode(d, base_ix));
            let mut has_value = base_ix >= 0;
            let mut tweened = false;
            for patch in 0..d.patch_node.len() {
                if d.patch_node[patch] != node {
                    continue;
                }
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
    }

    // Template transitions need distinct clocks and overlays per synthetic instance.
    for transition in 0..d.trans_node.len() {
        let base = d.trans_node[transition];
        let tr = i32::try_from(transition).expect("transition index exceeds i32");
        let synthetic_count = list::materialized(&st.lists).len();
        for synthetic in 0..synthetic_count {
            let node = list::materialized(&st.lists)[synthetic];
            note_synthetic_work();
            if list::base(&st.lists, d, node) == base
                && synthetic_transition(d, st, ms, tr, node, t)
            {
                ms.active = true;
            }
        }
    }

    // Running animations are time-indexed patches applied before layout.
    for binding in 0..d.bind_node.len() {
        if ms.lifted.get(binding).copied().unwrap_or(false) {
            continue;
        }
        let node = d.bind_node[binding];
        let animation = i32::try_from(d.bind_anim[binding]).expect("animation index exceeds i32");
        let dur = d.bind_dur[binding];
        let mode = d.bind_mode[binding];
        let delay = d.bind_delay[binding];
        let progress = ease_code(d.bind_easing[binding], cycle_progress(t, dur, mode, delay));

        // Preserve each attribute's first-appearance order across animation stops.
        let mut attrs = Vec::new();
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

        for attr in attrs {
            let v = keyframe_v(d, st, animation, attr, progress);
            if v.tag == value::V_MISSING {
                continue;
            }
            let mut owner = node;
            while owner != slir::NONE
                && d.node_kind[index(i32::try_from(owner).expect("node index exceeds i32"))]
                    != slir::K_EACH
            {
                owner = d.node_parent[index(i32::try_from(owner).expect("node index exceeds i32"))];
            }
            if owner == slir::NONE {
                style::ov_push(st, node, attr, v.tag, v.num, v.h, v.off, v.ln);
            } else {
                let synthetic_count = list::materialized(&st.lists).len();
                for synthetic_index in 0..synthetic_count {
                    let synthetic = list::materialized(&st.lists)[synthetic_index];
                    note_synthetic_work();
                    if list::base(&st.lists, d, synthetic) == node {
                        style::ov_push(st, synthetic, attr, v.tag, v.num, v.h, v.off, v.ln);
                    }
                }
            }
        }
        if dur > 0.0 && (mode != 1 || t - delay < dur) {
            ms.active = true;
        }
    }
    ms.active
}

/// One normalized keyframe of a liftable binding.
///
/// Stops are sorted, span the full `[0, 1]` cycle, and carry every animated
/// attribute at every position using the kernel's clamp-and-interpolate
/// keyframe semantics, so drivers translate them 1:1 into native keyframes.
#[derive(Clone, Debug, PartialEq)]
pub struct LiftStop {
    /// Cycle position in `[0, 1]`.
    pub pos: f64,
    /// Animated `offset` at this position, when the animation drives it.
    pub offset: Option<(f64, f64)>,
    /// Animated `opacity` at this position, when the animation drives it.
    pub opacity: Option<f64>,
}

/// An animation binding a driver may replay natively (e.g. as a CSS
/// animation) instead of asking the kernel to re-solve every frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Lift {
    /// Index into the document BIND pool.
    pub binding: usize,
    /// The bound node.
    pub node: u32,
    /// Cycle duration in milliseconds.
    pub dur: f64,
    /// Start delay in milliseconds.
    pub delay: f64,
    /// Cycle mode: `0` loops, `1` runs once, `2` alternates.
    pub mode: u32,
    /// Whole-cycle easing code (see [`ease_code`]).
    pub easing: u32,
    /// The node's static base offset. Painted geometry already includes it,
    /// so native translation keyframes are deltas against this value.
    pub base_offset: (f64, f64),
    /// Normalized keyframes.
    pub stops: Vec<LiftStop>,
}

/// Classifies every binding a driver can replay natively.
///
/// A binding lifts only when replaying it outside the kernel is
/// indistinguishable from the per-solve overlay:
/// - it animates nothing but `offset` and `opacity` — pure paint-time
///   translation and blending that never move siblings or resize parents;
/// - every keyframe and the node's base `offset` are static literals, so no
///   parameter, list field, or theme flip can change the track;
/// - the node paints exactly one leaf (`rect`, `text`, `image`, `path`)
///   outside any `each` template, carries no `when` patches, actions, or
///   `rotate`, and emits no signals — its retained hit geometry never
///   diverges from the drawn position in a way an interaction could observe;
/// - a non-linear easing (whole-cycle in Slab, per-segment natively) only
///   lifts when the stops sit at 0% and 100%, where both models agree.
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

/// Builds the lift description for one binding, or `None` when it must stay
/// kernel-driven.
fn lift_of(d: &Doc, binding: usize) -> Option<Lift> {
    let node = d.bind_node[binding];
    let dur = d.bind_dur[binding];
    let easing = d.bind_easing[binding];
    if dur <= 0.0 {
        return None;
    }

    // Leaf paints only: translating a container natively would leave its
    // separately-painted children behind.
    let kind = d.node_kind[index(i32::try_from(node).expect("node index exceeds i32"))];
    if !matches!(
        kind,
        slir::K_TEXT | slir::K_RECT | slir::K_IMG | slir::K_PATH
    ) {
        return None;
    }
    let mut ancestor = d.node_parent[index(i32::try_from(node).expect("node index exceeds i32"))];
    while ancestor != slir::NONE {
        let ai = index(i32::try_from(ancestor).expect("node index exceeds i32"));
        if d.node_kind[ai] == slir::K_EACH {
            return None;
        }
        ancestor = d.node_parent[ai];
    }

    // Interaction and patches keep the kernel authoritative: hover states,
    // actions, and signals hit-test against retained (unlifted) geometry.
    if d.patch_node.contains(&node) || d.sign_node.contains(&node) {
        return None;
    }
    if slir::base_attr(d, node, slir::A_ACT) >= 0 || slir::base_attr(d, node, slir::A_ROTATE) >= 0 {
        return None;
    }

    // Animated attributes, in first-appearance order across stops.
    let mut attrs = Vec::new();
    let animation =
        index(i32::try_from(d.bind_anim[binding]).expect("animation index exceeds i32"));
    let stop_start = d.anim_stop_off[animation];
    let stop_end = stop_start.wrapping_add(d.anim_stop_len[animation]);
    let mut positions = Vec::new();
    for stop in stop_start..stop_end {
        let stop = index(stop);
        let position = d.anim_stop_pos[stop];
        if easing != 0 && position != 0.0 && position != 1.0 {
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
            if attr != slir::A_OFFSET && attr != slir::A_OPACITY {
                return None;
            }
            let v = value::decode(
                d,
                i32::try_from(d.aattr_val[entry]).expect("attribute value index exceeds i32"),
            );
            let static_ok = match attr {
                slir::A_OFFSET => v.tag == slir::T_TUPLE && v.ln >= 2,
                _ => v.tag == slir::T_NUM,
            };
            if !static_ok {
                return None;
            }
            if !attrs.contains(&attr) {
                attrs.push(attr);
            }
        }
    }
    if attrs.is_empty() {
        return None;
    }

    // Base offset must be static too: translation keyframes are deltas.
    let base = value::decode(d, slir::base_attr(d, node, slir::A_OFFSET));
    let base_offset = match base.tag {
        value::V_MISSING => (0.0, 0.0),
        slir::T_TUPLE if base.ln >= 2 => {
            (value::tuple_at(d, &base, 0), value::tuple_at(d, &base, 1))
        }
        _ => return None,
    };

    positions.sort_by(f64::total_cmp);
    if positions.first() != Some(&0.0) {
        positions.insert(0, 0.0);
    }
    if positions.last() != Some(&1.0) {
        positions.push(1.0);
    }
    let animates_offset = attrs.contains(&slir::A_OFFSET);
    let animates_opacity = attrs.contains(&slir::A_OPACITY);
    let stops = positions
        .iter()
        .map(|&pos| LiftStop {
            pos,
            offset: animates_offset.then(|| {
                (
                    track_num(d, animation, slir::A_OFFSET, pos, 0),
                    track_num(d, animation, slir::A_OFFSET, pos, 1),
                )
            }),
            opacity: animates_opacity.then(|| track_num(d, animation, slir::A_OPACITY, pos, -1)),
        })
        .collect();

    Some(Lift {
        binding,
        node,
        dur,
        delay: d.bind_delay[binding],
        mode: d.bind_mode[binding],
        easing,
        base_offset,
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
