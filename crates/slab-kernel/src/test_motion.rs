//! Cycle progress modes, easing codes, and typed interpolation for continuous and discrete values.

use crate::{motion, slir, style, value};

/// Verifies cycle modes, delay handling, and each easing code.
pub fn test_easing_and_cycle_modes() {
    assert_eq!(
        motion::cycle_progress(500.0, 1000.0, 1, 0.0),
        0.5,
        "once mid"
    );
    assert_eq!(
        motion::cycle_progress(1500.0, 1000.0, 1, 0.0),
        1.0,
        "once holds final"
    );
    assert_eq!(
        motion::cycle_progress(1500.0, 1000.0, 0, 0.0),
        0.5,
        "loop wraps"
    );
    assert_eq!(
        motion::cycle_progress(1500.0, 1000.0, 2, 0.0),
        0.5,
        "alternate reverses"
    );
    assert!(
        (motion::cycle_progress(1900.0, 1000.0, 2, 0.0) - 0.1).abs() < 1.0e-9,
        "alternate near end"
    );
    assert_eq!(
        motion::cycle_progress(100.0, 1000.0, 0, 200.0),
        0.0,
        "delay gates start"
    );
    assert_eq!(
        motion::cycle_progress(0.0, 0.0, 0, 0.0),
        1.0,
        "zero dur pins to end"
    );
    assert_eq!(motion::ease_code(0, 0.5), 0.5, "linear");
    assert_eq!(motion::ease_code(1, 0.5), 0.25, "ease-in");
    assert_eq!(motion::ease_code(2, 0.5), 0.75, "ease-out");
    assert_eq!(motion::ease_code(3, 0.25), 0.125, "ease-in-out first half");
}

/// Constructs a numeric value for interpolation tests.
pub fn vnum(x: f64) -> value::V {
    value::V {
        tag: slir::T_NUM,
        num: x,
        h: 0,
        off: 0,
        ln: 0,
    }
}

/// Verifies interpolation of numeric, color, discrete, and mismatched value kinds.
pub fn test_lerp_types() {
    let d = slir::doc_new();
    let mut st = style::st_new();
    let result = motion::lerp_v(&d, &mut st, &vnum(0.0), &vnum(10.0), 0.3);
    assert_eq!((result.tag, result.num), (slir::T_NUM, 3.0), "numbers lerp");

    let a = value::V {
        tag: slir::T_PCT,
        num: 0.0,
        h: 0,
        off: 0,
        ln: 0,
    };
    let b = value::V {
        tag: slir::T_PCT,
        num: 50.0,
        h: 0,
        off: 0,
        ln: 0,
    };
    let percent = motion::lerp_v(&d, &mut st, &a, &b, 0.5);
    assert_eq!(
        (percent.tag, percent.num),
        (slir::T_PCT, 25.0),
        "percents lerp"
    );

    // Color endpoints retain exact SLIR packing, with red in the low byte.
    let red = value::V {
        tag: slir::T_COLOR,
        num: 0.0,
        h: 0xFF0000FF,
        off: 0,
        ln: 0,
    };
    let blue = value::V {
        tag: slir::T_COLOR,
        num: 0.0,
        h: 0xFF00FF00,
        off: 0,
        ln: 0,
    };
    let start = motion::lerp_v(&d, &mut st, &red, &blue, 0.0);
    let end = motion::lerp_v(&d, &mut st, &red, &blue, 1.0);
    assert_eq!(start.h, 0xFF0000FF, "color endpoint 0 exact");
    assert_eq!(end.h, 0xFF00FF00, "color endpoint 1 exact");
    let midpoint = motion::lerp_v(&d, &mut st, &red, &blue, 0.5);
    assert_eq!(midpoint.tag, slir::T_COLOR, "midpoint is a color");
    assert!(
        midpoint.h != red.h && midpoint.h != blue.h,
        "midpoint between"
    );
    assert_eq!((midpoint.h >> 24) & 0xFF, 0xFF, "alpha stays opaque");

    // Discrete values hold until the next stop.
    let early = value::V {
        tag: slir::T_ENUM_SYM,
        num: 0.0,
        h: 1,
        off: 0,
        ln: 0,
    };
    let late = value::V {
        tag: slir::T_ENUM_SYM,
        num: 0.0,
        h: 2,
        off: 0,
        ln: 0,
    };
    let before = motion::lerp_v(&d, &mut st, &early, &late, 0.4);
    let almost = motion::lerp_v(&d, &mut st, &early, &late, 0.999);
    assert_eq!(before.h, 1, "discrete holds early in interval");
    assert_eq!(almost.h, 1, "discrete holds until next stop");
    let at_end = motion::lerp_v(&d, &mut st, &early, &late, 1.0);
    assert_eq!(at_end.h, 2, "discrete switches at the next stop");

    let glyphs = [10, 11, 12, 13];
    let samples = [0.0, 0.5, 0.999999];
    for pair in glyphs.windows(2) {
        let from = value::V {
            tag: slir::T_STR,
            num: 0.0,
            h: pair[0],
            off: 0,
            ln: 0,
        };
        let to = value::V {
            tag: slir::T_STR,
            num: 0.0,
            h: pair[1],
            off: 0,
            ln: 0,
        };
        for sample in samples {
            let glyph = motion::lerp_v(&d, &mut st, &from, &to, sample);
            assert_eq!(
                (glyph.tag, glyph.h),
                (slir::T_STR, pair[0]),
                "spinner glyph holds"
            );
        }
    }

    // Mismatched kinds also hold the earlier value.
    let mismatch = motion::lerp_v(&d, &mut st, &vnum(4.0), &early, 0.9);
    assert_eq!(mismatch.tag, slir::T_NUM, "mismatch holds earlier kind");
}

/// Verifies elementwise tuple interpolation and discrete length mismatch behavior.
pub fn test_tuple_lerp_elementwise() {
    let mut d = slir::doc_new();
    d.f64s.extend([0.0, 0.0, 10.0, -10.0]);
    let mut st = style::st_new();
    let a = value::V {
        tag: slir::T_TUPLE,
        num: 0.0,
        h: 0,
        off: 0,
        ln: 2,
    };
    let b = value::V {
        tag: slir::T_TUPLE,
        num: 0.0,
        h: 0,
        off: 2,
        ln: 2,
    };
    let result = motion::lerp_v(&d, &mut st, &a, &b, 0.5);
    assert_eq!(
        result.tag,
        style::T_OV_TUPLE,
        "tuple lerp lands in the overlay pool"
    );
    assert_eq!(style::tup_at(&d, &st, &result, 0), 5.0, "elementwise x");
    assert_eq!(style::tup_at(&d, &st, &result, 1), -5.0, "elementwise y");

    // Mismatched lengths step discretely.
    let shorter = value::V {
        tag: slir::T_TUPLE,
        num: 0.0,
        h: 0,
        off: 0,
        ln: 1,
    };
    let mismatch = motion::lerp_v(&d, &mut st, &shorter, &b, 0.4);
    assert_eq!(mismatch.ln, 1, "length mismatch keeps a");
}

/// Verifies byte reversal and its involution property.
pub fn test_rgba_swap_involution() {
    assert_eq!(motion::rgba_swap(0x11223344), 0x44332211, "byte reverse");
    assert_eq!(
        motion::rgba_swap(motion::rgba_swap(0xDEADBEEF)),
        0xDEADBEEF,
        "involution"
    );
}

/// Appends an attribute value to the lift fixture and returns its index.
fn aval(d: &mut slir::Doc, tag: u32, lo: u32, hi: u32, num: f64) -> u32 {
    let ix = u32::try_from(d.aval_tag.len()).expect("fixture attribute count fits in u32");
    d.aval_tag.push(tag);
    d.aval_lo.push(lo);
    d.aval_hi.push(hi);
    d.aval_num.push(num);
    ix
}

/// Appends a tuple value backed by the fixture `f64s` pool.
fn atuple(d: &mut slir::Doc, x: f64, y: f64) -> u32 {
    let off = u32::try_from(d.f64s.len()).expect("fixture tuple pool fits in u32");
    d.f64s.extend([x, y]);
    aval(d, slir::T_TUPLE, off, 2, 0.0)
}

/// Builds the lift-classification fixture.
///
/// Animations: `0` drives offset+opacity at 0%/100%, `1` drives solid `bg`
/// (green to red), `2` drives offset at 0%/50%/100% and opacity at 50%/100%
/// (clamped start), `3` drives `w` (layout, unliftable). Bindings: `0` lifts
/// (leaf rect, static values, whole-cycle ease-in-out), `1` lifts as color
/// keyframes, `2` binds a render-only container, `3` binds a patched geometry
/// node, `4`+`5` share a node where only one would lift, `6` lifts with clamped
/// stops, and `7` lifts a non-linear easing over interior stops via time
/// remapping.
fn lift_doc() -> slir::Doc {
    let mut d = slir::doc_new();
    d.ok = true;
    d.strs.extend(["".to_owned(), "hover".to_owned()]);

    let base_off = atuple(&mut d, 40.0, -60.0);
    let t_zero = atuple(&mut d, 0.0, 0.0);
    let t_drift = atuple(&mut d, 90.0, 26.0);
    let t_far = atuple(&mut d, 40.0, 40.0);
    let o_hi = aval(&mut d, slir::T_NUM, 0, 0, 0.85);
    let o_lo = aval(&mut d, slir::T_NUM, 0, 0, 0.55);
    let c_a = aval(&mut d, slir::T_COLOR, 0xFF00FF00, 0, 0.0);
    let c_b = aval(&mut d, slir::T_COLOR, 0xFF0000FF, 0, 0.0);
    let w_a = aval(&mut d, slir::T_NUM, 0, 0, 10.0);
    let w_b = aval(&mut d, slir::T_NUM, 0, 0, 20.0);

    // Nodes 0..=7: stack root, then seven leaves/containers under it.
    d.node_kind.extend([
        slir::K_STACK,
        slir::K_RECT,
        slir::K_RECT,
        slir::K_ROW,
        slir::K_RECT,
        slir::K_RECT,
        slir::K_RECT,
        slir::K_RECT,
    ]);
    d.node_flags.extend([0; 8]);
    d.node_parent.extend([slir::NONE, 0, 0, 0, 0, 0, 0, 0]);
    d.node_first.extend([
        1,
        slir::NONE,
        slir::NONE,
        slir::NONE,
        slir::NONE,
        slir::NONE,
        slir::NONE,
        slir::NONE,
    ]);
    d.node_next
        .extend([slir::NONE, 2, 3, 4, 5, 6, 7, slir::NONE]);
    d.node_key.extend([0; 8]);
    d.node_id.extend([0; 8]);
    d.node_line.extend([0; 8]);
    // Only node 1 carries a base attribute (its static offset).
    d.attr_index.extend([0, 0, 1, 1, 1, 1, 1, 1, 1]);
    d.attr_id.push(slir::A_OFFSET);
    d.attr_val.push(base_off);

    // `when hover` on node 4 keeps that binding kernel-driven.
    d.cond_kind.push(slir::C_STATE);
    d.cond_sym.push(1);
    d.cond_op.push(0);
    d.cond_neg.push(0);
    d.cond_num.push(0.0);
    d.patch_node.push(4);
    d.patch_cond.push(0);
    d.patch_attr_off.push(0);
    d.patch_attr_len.push(1);
    d.patch_child_off.push(0);
    d.patch_child_len.push(0);
    d.wattr_id.push(slir::A_BG);
    d.wattr_val.push(c_a);

    // Animation stop pools.
    d.anim_name.extend([0, 0, 0, 0]);
    d.anim_stop_off.extend([0, 2, 4, 7]);
    d.anim_stop_len.extend([2, 2, 3, 2]);
    d.anim_stop_pos
        .extend([0.0, 1.0, 0.0, 1.0, 0.0, 0.5, 1.0, 0.0, 1.0]);
    d.anim_stop_attr_off.extend([0, 2, 4, 5, 6, 7, 9, 11, 12]);
    d.anim_stop_attr_len.extend([2, 2, 1, 1, 1, 2, 2, 1, 1]);
    d.aattr_id.extend([
        slir::A_OFFSET,
        slir::A_OPACITY,
        slir::A_OFFSET,
        slir::A_OPACITY,
        slir::A_BG,
        slir::A_BG,
        slir::A_OFFSET,
        slir::A_OFFSET,
        slir::A_OPACITY,
        slir::A_OFFSET,
        slir::A_OPACITY,
        slir::A_W,
        slir::A_W,
    ]);
    d.aattr_val.extend([
        t_zero, o_hi, t_drift, o_lo, c_a, c_b, t_zero, t_drift, o_hi, t_far, o_lo, w_a, w_b,
    ]);

    d.bind_node.extend([1, 2, 3, 4, 5, 5, 6, 7]);
    d.bind_anim.extend([0, 1, 0, 0, 0, 3, 2, 2]);
    d.bind_dur.extend([
        3800.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0,
    ]);
    d.bind_mode.extend([2, 0, 0, 0, 0, 0, 0, 0]);
    d.bind_easing.extend([3, 0, 0, 0, 0, 0, 0, 1]);
    d.bind_delay.extend([0.0; 8]);
    d
}

/// A [`motion::LiftStop`] with only the linear-segment defaults filled in.
fn stop(pos: f64, offset: Option<(f64, f64)>, opacity: Option<f64>) -> motion::LiftStop {
    motion::LiftStop {
        pos,
        ctrl: (1.0 / 3.0, 2.0 / 3.0),
        offset,
        opacity,
        rotate: None,
        scale: None,
        bg: None,
        color: None,
    }
}

/// Verifies which bindings lift and the normalized keyframes they export.
pub fn test_lifts_classification() {
    let d = lift_doc();
    let lifted = motion::lifts(&d);
    let bindings: Vec<usize> = lifted.iter().map(|l| l.binding).collect();
    assert_eq!(
        bindings,
        [0, 1, 2, 6, 7],
        "static ink leaves and render-only containers lift; patched geometry, layout, and shared nodes stay"
    );

    let drift = &lifted[0];
    assert_eq!(drift.node, 1, "lift names the bound node");
    assert_eq!(drift.kind, slir::K_RECT, "lift names the bound node's kind");
    assert_eq!(
        (drift.dur, drift.delay, drift.mode),
        (3800.0, 0.0, 2),
        "binding timing carries through"
    );
    assert_eq!(
        drift.base_offset,
        (40.0, -60.0),
        "static base offset resolves"
    );
    assert_eq!(
        (drift.base_rotate, drift.base_scale),
        (0.0, (1.0, 1.0)),
        "absent transform bases default to identity"
    );
    // Whole-cycle ease-in-out splits at 50% into its exact quadratic halves.
    assert_eq!(
        drift.stops,
        [
            motion::LiftStop {
                ctrl: (0.0, 1.0 / 3.0),
                ..stop(0.0, Some((0.0, 0.0)), Some(0.85))
            },
            motion::LiftStop {
                ctrl: (2.0 / 3.0, 1.0),
                ..stop(0.5, Some((45.0, 13.0)), Some(0.7))
            },
            stop(1.0, Some((90.0, 26.0)), Some(0.55)),
        ],
        "ease-in-out emits its quad-in/quad-out segment curves"
    );

    let clamped = &lifted[3];
    assert_eq!(clamped.node, 6, "clamped lift names its node");
    assert_eq!(
        clamped.base_offset,
        (0.0, 0.0),
        "missing base offset is zero"
    );
    assert_eq!(
        clamped.stops,
        [
            stop(0.0, Some((0.0, 0.0)), Some(0.85)),
            stop(0.5, Some((90.0, 26.0)), Some(0.85)),
            stop(1.0, Some((40.0, 40.0)), Some(0.55)),
        ],
        "opacity clamps to its first stop before 50%"
    );
}

/// Verifies whole-cycle non-linear easing lifts over interior stops by
/// remapping positions into the time domain with exact segment curves.
pub fn test_lift_easing_remap() {
    let d = lift_doc();
    let lifted = motion::lifts(&d);
    let eased = lifted
        .iter()
        .find(|l| l.binding == 7)
        .expect("ease-in binding lifts");

    // ease-in reaches progress 0.5 at t = sqrt(0.5).
    let mid = 0.5f64.sqrt();
    let positions: Vec<f64> = eased.stops.iter().map(|s| s.pos).collect();
    assert_eq!(positions, [0.0, mid, 1.0], "stops remap into time domain");

    // Segment restrictions of t² are quadratics with exact Bézier forms
    // (expectations use the same float operations the kernel performs).
    let head = mid * mid / 0.5;
    assert_eq!(
        eased.stops[0].ctrl,
        (0.0, head / 3.0),
        "first segment is the quad-in restriction over [0, sqrt(0.5)]"
    );
    let span = 1.0 - mid;
    let a = span * span / 0.5;
    let b = 2.0 * mid * span / 0.5;
    assert_eq!(
        eased.stops[1].ctrl,
        (b / 3.0, (a + 2.0 * b) / 3.0),
        "second segment is the exact t² restriction"
    );
    assert_eq!(
        eased.stops[2].ctrl,
        (1.0 / 3.0, 2.0 / 3.0),
        "the last stop carries the linear curve"
    );

    // Keyframe values are sampled at eased progress, not at the time knot.
    assert_eq!(
        eased.stops[1].offset,
        Some((90.0, 26.0)),
        "values follow the progress-space track"
    );
}

/// Verifies solid color keyframes lift with OKLab-faithful subdivision.
pub fn test_lift_color_subdivision() {
    let d = lift_doc();
    let lifted = motion::lifts(&d);
    let paint = lifted
        .iter()
        .find(|l| l.binding == 1)
        .expect("solid bg binding lifts");
    assert_eq!(paint.kind, slir::K_RECT, "bg lift carries the rect kind");

    let stops = &paint.stops;
    assert!(
        stops.len() > 2,
        "green-to-red OKLab track subdivides for sRGB replay, got {} stops",
        stops.len()
    );
    assert_eq!(stops[0].bg, Some(0xFF00FF00), "first stop is the green end");
    assert_eq!(
        stops[stops.len() - 1].bg,
        Some(0xFF0000FF),
        "last stop is the red end"
    );
    for s in stops {
        assert_eq!(
            s.bg,
            Some(motion::lerp_rgba(0xFF00FF00, 0xFF0000FF, s.pos)),
            "every knot sits exactly on the kernel's OKLab path"
        );
        assert_eq!(s.offset, None, "color-only binding carries no offset");
    }
    let positions: Vec<f64> = stops.iter().map(|s| s.pos).collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "knots stay strictly increasing"
    );
}

/// Builds the transform-lift fixture.
///
/// Nodes: `1` rect with static base `rotate=10`, `2` rect, `3` text, `4` rect
/// with base `scale=(2,1)`, `5` rect with base `rotate=30`, `6` rect with a
/// gradient base `bg`, `7` text. Bindings: `0` animates rotate 0→45 on `1`
/// (lifts against the base delta), `1` spins `2` through a quarter turn
/// (refused), `2` rotates the text `3` (refused — per-line runs), `3` scales
/// `4` (lifts with the tuple base), `4` translates `5` outside its static base
/// rotation group, `5` animates `bg` over a gradient base (refused), and `6`
/// animates text `color` on `7` (lifts).
fn transform_doc() -> slir::Doc {
    let mut d = slir::doc_new();
    d.ok = true;
    d.strs.push(String::new());

    let rot10 = aval(&mut d, slir::T_NUM, 0, 0, 10.0);
    let rot30 = aval(&mut d, slir::T_NUM, 0, 0, 30.0);
    let r0 = aval(&mut d, slir::T_NUM, 0, 0, 0.0);
    let r45 = aval(&mut d, slir::T_NUM, 0, 0, 45.0);
    let r360 = aval(&mut d, slir::T_NUM, 0, 0, 360.0);
    let s1 = aval(&mut d, slir::T_NUM, 0, 0, 1.0);
    let s14 = aval(&mut d, slir::T_NUM, 0, 0, 1.4);
    let sc_base = atuple(&mut d, 2.0, 1.0);
    let off0 = atuple(&mut d, 0.0, 0.0);
    let off10 = atuple(&mut d, 10.0, 10.0);
    let c_a = aval(&mut d, slir::T_COLOR, 0xFF00FF00, 0, 0.0);
    let c_b = aval(&mut d, slir::T_COLOR, 0xFF0000FF, 0, 0.0);
    let grad = aval(&mut d, slir::T_PAINT_GRADIENT, 0, 0, 0.0);

    d.node_kind.extend([
        slir::K_STACK,
        slir::K_RECT,
        slir::K_RECT,
        slir::K_TEXT,
        slir::K_RECT,
        slir::K_RECT,
        slir::K_RECT,
        slir::K_TEXT,
    ]);
    d.node_flags.extend([0; 8]);
    d.node_parent.extend([slir::NONE, 0, 0, 0, 0, 0, 0, 0]);
    d.node_first.extend([
        1,
        slir::NONE,
        slir::NONE,
        slir::NONE,
        slir::NONE,
        slir::NONE,
        slir::NONE,
        slir::NONE,
    ]);
    d.node_next
        .extend([slir::NONE, 2, 3, 4, 5, 6, 7, slir::NONE]);
    d.node_key.extend([0; 8]);
    d.node_id.extend([0; 8]);
    d.node_line.extend([0; 8]);
    d.attr_index.extend([0, 0, 1, 1, 1, 2, 3, 4, 4]);
    d.attr_id
        .extend([slir::A_ROTATE, slir::A_SCALE, slir::A_ROTATE, slir::A_BG]);
    d.attr_val.extend([rot10, sc_base, rot30, grad]);

    d.anim_name.extend([0; 6]);
    d.anim_stop_off.extend([0, 2, 4, 6, 8, 10]);
    d.anim_stop_len.extend([2; 6]);
    d.anim_stop_pos
        .extend([0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0]);
    d.anim_stop_attr_off
        .extend([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    d.anim_stop_attr_len.extend([1; 12]);
    d.aattr_id.extend([
        slir::A_ROTATE,
        slir::A_ROTATE,
        slir::A_ROTATE,
        slir::A_ROTATE,
        slir::A_SCALE,
        slir::A_SCALE,
        slir::A_OFFSET,
        slir::A_OFFSET,
        slir::A_BG,
        slir::A_BG,
        slir::A_COLOR,
        slir::A_COLOR,
    ]);
    d.aattr_val
        .extend([r0, r45, r0, r360, s1, s14, off0, off10, c_a, c_b, c_a, c_b]);

    d.bind_node.extend([1, 2, 3, 4, 5, 6, 7]);
    d.bind_anim.extend([0, 1, 0, 2, 3, 4, 5]);
    d.bind_dur.extend([500.0; 7]);
    d.bind_mode.extend([0; 7]);
    d.bind_easing.extend([0; 7]);
    d.bind_delay.extend([0.0; 7]);
    d
}

/// Verifies rotate/scale/color lift gating: transform deltas against static
/// bases, quarter-turn and per-line-text refusals, and paint-channel bases.
pub fn test_lift_transform_tracks() {
    let d = transform_doc();
    let lifted = motion::lifts(&d);
    let bindings: Vec<usize> = lifted.iter().map(|l| l.binding).collect();
    assert_eq!(
        bindings,
        [0, 3, 4, 6],
        "rotate/scale/text-color and wrapper translation lift; spins, text transforms, and gradient bases stay"
    );

    let rotated = &lifted[0];
    assert_eq!(
        (rotated.kind, rotated.base_rotate),
        (slir::K_RECT, 10.0),
        "rotation lifts carry the static base for delta replay"
    );
    let track: Vec<Option<f64>> = rotated.stops.iter().map(|s| s.rotate).collect();
    assert_eq!(track, [Some(0.0), Some(45.0)], "rotate stops are absolute");

    let scaled = &lifted[1];
    assert_eq!(
        scaled.base_scale,
        (2.0, 1.0),
        "scale lifts carry the static per-axis base"
    );
    let track: Vec<Option<(f64, f64)>> = scaled.stops.iter().map(|s| s.scale).collect();
    assert_eq!(
        track,
        [Some((1.0, 1.0)), Some((1.4, 1.4))],
        "uniform scale stops expand to per-axis factors"
    );

    let inked = &lifted[3];
    assert_eq!(inked.kind, slir::K_TEXT, "text color lift names its kind");
    assert_eq!(
        (
            inked.stops[0].color,
            inked.stops[inked.stops.len() - 1].color
        ),
        (Some(0xFF00FF00), Some(0xFF0000FF)),
        "color stops carry SLIR-packed endpoints"
    );
}

/// Verifies that lifted bindings stop overlaying and driving motion activity.
pub fn test_apply_skips_lifted_bindings() {
    let d = lift_doc();

    // Kernel-driven: halfway through the alternate cycle the overlay blends.
    let mut st = style::st_new();
    style::init_params(&d, &mut st);
    style::begin_solve(&d, &mut st);
    let mut ms = motion::mst_new();
    assert!(
        motion::apply(&d, &mut st, &mut ms, 1900.0),
        "animations stay live"
    );
    assert!(
        (style::attr_num(&d, &st, 1, slir::A_OPACITY, 1.0) - 0.7).abs() < 1e-9,
        "unlifted binding overlays interpolated opacity"
    );

    // Lifting binding 0 removes its overlay while others keep motion active.
    let mut st = style::st_new();
    style::init_params(&d, &mut st);
    style::begin_solve(&d, &mut st);
    let mut ms = motion::mst_new();
    ms.lifted = vec![false; d.bind_node.len()];
    for lift in motion::lifts(&d) {
        ms.lifted[lift.binding] = true;
    }
    assert!(
        motion::apply(&d, &mut st, &mut ms, 1900.0),
        "remaining kernel-driven bindings keep motion active"
    );
    assert_eq!(
        style::attr_num(&d, &st, 1, slir::A_OPACITY, 1.0),
        1.0,
        "lifted binding leaves the base value untouched"
    );

    // A fully lifted document goes idle: no re-solve per frame.
    let mut st = style::st_new();
    style::init_params(&d, &mut st);
    style::begin_solve(&d, &mut st);
    let mut ms = motion::mst_new();
    ms.lifted = vec![true; d.bind_node.len()];
    assert!(
        !motion::apply(&d, &mut st, &mut ms, 1900.0),
        "fully lifted document reports no motion"
    );
}
