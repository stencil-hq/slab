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
/// Animations: `0` drives offset+opacity at 0%/100%, `1` drives `bg`
/// (unliftable attribute), `2` drives offset at 0%/50%/100% and opacity at
/// 50%/100% (clamped start). Bindings: `0` lifts (leaf rect, static values),
/// `1` animates paint, `2` binds a container, `3` binds a patched node, `4`+`5`
/// share a node where only one would lift, `6` lifts with clamped stops, and
/// `7` uses a non-linear easing over interior stops.
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
    d.anim_name.extend([0, 0, 0]);
    d.anim_stop_off.extend([0, 2, 4]);
    d.anim_stop_len.extend([2, 2, 3]);
    d.anim_stop_pos.extend([0.0, 1.0, 0.0, 1.0, 0.0, 0.5, 1.0]);
    d.anim_stop_attr_off.extend([0, 2, 4, 5, 6, 7, 9]);
    d.anim_stop_attr_len.extend([2, 2, 1, 1, 1, 2, 2]);
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
    ]);
    d.aattr_val.extend([
        t_zero, o_hi, t_drift, o_lo, c_a, c_b, t_zero, t_drift, o_hi, t_far, o_lo,
    ]);

    d.bind_node.extend([1, 2, 3, 4, 5, 5, 6, 7]);
    d.bind_anim.extend([0, 1, 0, 0, 0, 1, 2, 2]);
    d.bind_dur.extend([
        3800.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0,
    ]);
    d.bind_mode.extend([2, 0, 0, 0, 0, 0, 0, 0]);
    d.bind_easing.extend([3, 0, 0, 0, 0, 0, 0, 1]);
    d.bind_delay.extend([0.0; 8]);
    d
}

/// Verifies which bindings lift and the normalized keyframes they export.
pub fn test_lifts_classification() {
    let d = lift_doc();
    let lifted = motion::lifts(&d);
    let bindings: Vec<usize> = lifted.iter().map(|l| l.binding).collect();
    assert_eq!(
        bindings,
        [0, 6],
        "only static leaf offset/opacity bindings lift"
    );

    let drift = &lifted[0];
    assert_eq!(drift.node, 1, "lift names the bound node");
    assert_eq!(
        (drift.dur, drift.delay, drift.mode, drift.easing),
        (3800.0, 0.0, 2, 3),
        "binding timing carries through"
    );
    assert_eq!(
        drift.base_offset,
        (40.0, -60.0),
        "static base offset resolves"
    );
    assert_eq!(
        drift.stops,
        [
            motion::LiftStop {
                pos: 0.0,
                offset: Some((0.0, 0.0)),
                opacity: Some(0.85),
            },
            motion::LiftStop {
                pos: 1.0,
                offset: Some((90.0, 26.0)),
                opacity: Some(0.55),
            },
        ],
        "two-stop track exports verbatim"
    );

    let clamped = &lifted[1];
    assert_eq!(clamped.node, 6, "clamped lift names its node");
    assert_eq!(
        clamped.base_offset,
        (0.0, 0.0),
        "missing base offset is zero"
    );
    assert_eq!(
        clamped.stops,
        [
            motion::LiftStop {
                pos: 0.0,
                offset: Some((0.0, 0.0)),
                opacity: Some(0.85),
            },
            motion::LiftStop {
                pos: 0.5,
                offset: Some((90.0, 26.0)),
                opacity: Some(0.85),
            },
            motion::LiftStop {
                pos: 1.0,
                offset: Some((40.0, 40.0)),
                opacity: Some(0.55),
            },
        ],
        "opacity clamps to its first stop before 50%"
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
