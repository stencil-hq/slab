//! Deterministic JSON serialization for frames and interaction traces.
//!
//! Frame JSON is emitted on one line with a fixed key order. Model numbers go
//! through [`crate::value::fmt3`], which rounds to three decimals using integer
//! arithmetic and normalizes negative zero. Strings are escaped codepoint by
//! codepoint. The serializer never delegates model-number formatting or object
//! ordering to the host, so its output is byte-identical on every target.

use crate::{dispatch, edit, flatten, frame, list, scene, slir, style, value};

const QUOTE: u32 = 34;
const BACKSLASH: u32 = 92;
const COMMA: u32 = 44;
const ARRAY_OPEN: u32 = 91;
const ARRAY_CLOSE: u32 = 93;
const OBJECT_CLOSE: u32 = 125;

/// Appends a string's Unicode scalar values to the output buffer.
pub fn emit(out: &mut Vec<u32>, s: &str) {
    out.extend(s.chars().map(u32::from));
}

/// Appends a JSON string, including quotes and deterministic escaping.
pub fn emit_jstr(out: &mut Vec<u32>, s: &str) {
    out.push(QUOTE);
    for c in s.chars().map(u32::from) {
        match c {
            34 => out.extend([BACKSLASH, QUOTE]),
            92 => out.extend([BACKSLASH, BACKSLASH]),
            10 => out.extend([BACKSLASH, u32::from(b'n')]),
            9 => out.extend([BACKSLASH, u32::from(b't')]),
            13 => out.extend([BACKSLASH, u32::from(b'r')]),
            0..=31 => {
                // JSON represents remaining control characters as \u00XX.
                out.extend([
                    BACKSLASH,
                    u32::from(b'u'),
                    u32::from(b'0'),
                    u32::from(b'0'),
                    hex_digit(c.wrapping_shr(4)),
                    hex_digit(c & 15),
                ]);
            }
            _ => out.push(c),
        }
    }
    out.push(QUOTE);
}

/// Returns the lowercase ASCII hexadecimal digit for a nibble.
pub fn hex_digit(v: u32) -> u32 {
    if v < 10 {
        u32::from(b'0').wrapping_add(v)
    } else {
        u32::from(b'a').wrapping_sub(10).wrapping_add(v)
    }
}

/// Appends a model number in the canonical three-decimal representation.
pub fn emit_num(out: &mut Vec<u32>, v: f64) {
    emit(out, &value::fmt3(v));
}

/// Appends an unsigned integer without allocating an intermediate string.
pub fn emit_u32(out: &mut Vec<u32>, mut value: u32) {
    let mut digits = [0_u32; 10];
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = u32::from(b'0').wrapping_add(value % 10);
        value /= 10;
        if value == 0 {
            break;
        }
    }
    out.extend_from_slice(&digits[start..]);
}

/// Appends a signed integer while preserving wrapping behavior at `i32::MIN`.
pub fn emit_i32(out: &mut Vec<u32>, value: i32) {
    if value < 0 {
        out.push(u32::from(b'-'));
        emit_u32(out, value.wrapping_neg().cast_unsigned());
    } else {
        emit_u32(out, value.cast_unsigned());
    }
}

/// Appends a JSON boolean.
pub fn emit_bool(out: &mut Vec<u32>, v: bool) {
    emit(out, if v { "true" } else { "false" });
}

/// Appends a JSON number or `null` for an absent semantic value.
pub fn emit_optional_num(out: &mut Vec<u32>, value: Option<f64>) {
    if let Some(value) = value {
        emit_num(out, value);
    } else {
        emit(out, "null");
    }
}

/// Appends `"#rrggbbaa"` from an SLIR-packed RGBA8 word.
pub fn emit_color(out: &mut Vec<u32>, v: u32) {
    out.extend([
        QUOTE,
        u32::from(b'#'),
        hex_digit(v.wrapping_shr(4) & 15),
        hex_digit(v & 15),
        hex_digit(v.wrapping_shr(12) & 15),
        hex_digit(v.wrapping_shr(8) & 15),
        hex_digit(v.wrapping_shr(20) & 15),
        hex_digit(v.wrapping_shr(16) & 15),
        hex_digit(v.wrapping_shr(28) & 15),
        hex_digit(v.wrapping_shr(24) & 15),
        QUOTE,
    ]);
}

/// Appends a paint as `null`, an RGBA color, or a `"grad:N"` reference.
pub fn emit_paint(out: &mut Vec<u32>, kind: u32, handle: u32) {
    match kind {
        1 => emit_color(out, handle),
        2 => {
            emit(out, "\"grad:");
            emit_u32(out, handle);
            out.push(QUOTE);
        }
        _ => emit(out, "null"),
    }
}

/// Appends a dash pair, or `null` when dashing is disabled.
pub fn emit_dash(out: &mut Vec<u32>, has: bool, on: f64, off: f64) {
    if !has {
        emit(out, "null");
        return;
    }
    out.push(ARRAY_OPEN);
    emit_num(out, on);
    out.push(COMMA);
    emit_num(out, off);
    out.push(ARRAY_CLOSE);
}

/// Appends an inline shadow run in the canonical wire shape.
///
/// Each shadow contains `x`, `y`, `blur`, `spread`, `color`, and `inset`.
pub fn emit_shadows(out: &mut Vec<u32>, d: &slir::Doc, off: i32, len: i32) {
    out.push(ARRAY_OPEN);
    for k in off..off.wrapping_add(len) {
        if k > off {
            out.push(COMMA);
        }
        let k = usize::try_from(k).expect("shadow index is non-negative");
        emit(out, "{\"x\":");
        emit_num(out, d.shdw_x[k]);
        emit(out, ",\"y\":");
        emit_num(out, d.shdw_y[k]);
        emit(out, ",\"blur\":");
        emit_num(out, d.shdw_blur[k]);
        emit(out, ",\"spread\":");
        emit_num(out, d.shdw_spread[k]);
        emit(out, ",\"color\":");
        emit_color(out, d.shdw_rgba[k]);
        emit(out, ",\"inset\":");
        emit_bool(out, d.shdw_inset[k] != 0);
        out.push(OBJECT_CLOSE);
    }
    out.push(ARRAY_CLOSE);
}

/// Appends a rectangle draw operation.
pub fn emit_rect(out: &mut Vec<u32>, d: &slir::Doc, r: &flatten::OpRect) {
    emit(out, "{\"op\":\"Rect\",\"node\":");
    emit_u32(out, r.node);
    emit(out, ",\"x\":");
    emit_num(out, r.x);
    emit(out, ",\"y\":");
    emit_num(out, r.y);
    emit(out, ",\"w\":");
    emit_num(out, r.w);
    emit(out, ",\"h\":");
    emit_num(out, r.h);
    emit(out, ",\"radius\":");
    emit_num(out, r.radius);
    emit(out, ",\"bg\":");
    emit_paint(out, r.bg_kind, r.bg);
    emit(out, ",\"stroke\":");
    emit_paint(out, r.stroke_kind, r.stroke);
    emit(out, ",\"stroke_w\":");
    emit_num(out, r.stroke_w);
    emit(out, ",\"stroke_align\":");
    emit_u32(out, r.stroke_align);
    emit(out, ",\"stroke_sides\":");
    emit_u32(out, r.stroke_sides);
    emit(out, ",\"dash\":");
    emit_dash(out, r.has_dash, r.dash_on, r.dash_off);
    emit(out, ",\"shadows\":");
    emit_shadows(out, d, r.shadow_off, r.shadow_len);
    emit(out, ",\"opacity\":");
    emit_num(out, r.opacity);
    if r.smooth > 0.0 {
        emit(out, ",\"smooth\":");
        emit_num(out, r.smooth);
    }
    if r.grain_amount > 0.0 {
        emit(out, ",\"grain\":[");
        emit_num(out, r.grain_amount);
        out.push(COMMA);
        emit_num(out, r.grain_size);
        out.push(ARRAY_CLOSE);
    }
    out.push(125u32);
}

/// Appends a text draw operation.
pub fn emit_text(out: &mut Vec<u32>, t: &flatten::OpText) {
    emit(out, "{\"op\":\"Text\",\"node\":");
    emit_u32(out, t.node);
    emit(out, ",\"x\":");
    emit_num(out, t.x);
    emit(out, ",\"y_baseline\":");
    emit_num(out, t.y_baseline);
    emit(out, ",\"str_ref\":");
    emit_i32(out, t.str_ref);
    emit(out, ",\"measured_w\":");
    emit_num(out, t.measured_w);
    emit(out, ",\"font\":");
    emit_i32(out, t.font);
    emit(out, ",\"size\":");
    emit_num(out, t.size);
    emit(out, ",\"weight\":");
    emit_u32(out, t.weight);
    emit(out, ",\"tracking\":");
    emit_num(out, t.tracking);
    emit(out, ",\"color\":");
    emit_paint(out, t.color_kind, t.color);
    emit(out, ",\"opacity\":");
    emit_num(out, t.opacity);
    if t.color_kind == 2 {
        emit(out, ",\"grad_box\":[");
        emit_num(out, t.gx);
        out.push(COMMA);
        emit_num(out, t.gy);
        out.push(COMMA);
        emit_num(out, t.gw);
        out.push(COMMA);
        emit_num(out, t.gh);
        out.push(ARRAY_CLOSE);
    }
    out.push(125u32);
}

/// Appends an image draw operation.
pub fn emit_image(out: &mut Vec<u32>, im: &flatten::OpImage) {
    emit(out, "{\"op\":\"Image\",\"node\":");
    emit_u32(out, im.node);
    emit(out, ",\"x\":");
    emit_num(out, im.x);
    emit(out, ",\"y\":");
    emit_num(out, im.y);
    emit(out, ",\"w\":");
    emit_num(out, im.w);
    emit(out, ",\"h\":");
    emit_num(out, im.h);
    emit(out, ",\"img\":");
    emit_i32(out, im.img);
    emit(out, ",\"fit\":");
    emit_u32(out, im.fit);
    emit(out, ",\"radius\":");
    emit_num(out, im.radius);
    emit(out, ",\"opacity\":");
    emit_num(out, im.opacity);
    if im.smooth > 0.0 {
        emit(out, ",\"smooth\":");
        emit_num(out, im.smooth);
    }
    out.push(125u32);
}

/// Appends a path draw operation.
pub fn emit_path(out: &mut Vec<u32>, p: &flatten::OpPath) {
    emit(out, "{\"op\":\"PathDraw\",\"node\":");
    emit_u32(out, p.node);
    emit(out, ",\"dx\":");
    emit_num(out, p.dx);
    emit(out, ",\"dy\":");
    emit_num(out, p.dy);
    emit(out, ",\"path\":");
    if p.path < 0 {
        out.push(QUOTE);
        emit(out, "rt:");
        emit_i32(out, !p.path);
        out.push(QUOTE);
    } else {
        emit_i32(out, p.path);
    }
    emit(out, ",\"bg\":");
    emit_paint(out, p.bg_kind, p.bg);
    emit(out, ",\"stroke\":");
    emit_paint(out, p.stroke_kind, p.stroke);
    emit(out, ",\"stroke_w\":");
    emit_num(out, p.stroke_w);
    emit(out, ",\"dash\":");
    emit_dash(out, p.has_dash, p.dash_on, p.dash_off);
    emit(out, ",\"opacity\":");
    emit_num(out, p.opacity);
    out.push(125u32);
}

/// Appends the frame operation at `index`.
pub fn emit_op(out: &mut Vec<u32>, d: &slir::Doc, fr: &flatten::Frame, index: i32) {
    let index = usize::try_from(index).expect("frame operation index is non-negative");
    match &fr.ops[index] {
        flatten::FrameOp::Rect(rect) => emit_rect(out, d, rect),
        flatten::FrameOp::Text(text) => emit_text(out, text),
        flatten::FrameOp::Image(image) => emit_image(out, image),
        flatten::FrameOp::PathDraw(path) => emit_path(out, path),
        flatten::FrameOp::ClipPush(c) => {
            emit(out, "{\"op\":\"ClipPush\",\"x\":");
            emit_num(out, c.x);
            emit(out, ",\"y\":");
            emit_num(out, c.y);
            emit(out, ",\"w\":");
            emit_num(out, c.w);
            emit(out, ",\"h\":");
            emit_num(out, c.h);
            emit(out, ",\"radius\":");
            emit_num(out, c.radius);
            if c.smooth > 0.0 {
                emit(out, ",\"smooth\":");
                emit_num(out, c.smooth);
            }
            out.push(125u32);
        }
        flatten::FrameOp::ClipPop => {
            emit(out, "{\"op\":\"ClipPop\"}");
        }
        flatten::FrameOp::GroupPush(g) => {
            emit(out, "{\"op\":\"GroupPush\",\"opacity\":");
            emit_num(out, g.opacity);
            emit(out, ",\"blur\":");
            emit_num(out, g.blur);
            if g.mask_kind != 0 {
                emit(out, ",\"mask\":");
                emit_paint(out, g.mask_kind, g.mask);
                emit(out, ",\"mask_box\":[");
                emit_num(out, g.mx);
                out.push(COMMA);
                emit_num(out, g.my);
                out.push(COMMA);
                emit_num(out, g.mw);
                out.push(COMMA);
                emit_num(out, g.mh);
                out.push(ARRAY_CLOSE);
            }
            out.push(125u32);
        }
        flatten::FrameOp::GroupPop => {
            emit(out, "{\"op\":\"GroupPop\"}");
        }
        flatten::FrameOp::RotatePush(rt) => {
            emit(out, "{\"op\":\"RotatePush\",\"cx\":");
            emit_num(out, rt.cx);
            emit(out, ",\"cy\":");
            emit_num(out, rt.cy);
            emit(out, ",\"deg\":");
            emit_num(out, rt.deg);
            out.push(125u32);
        }
        flatten::FrameOp::RotatePop => {
            emit(out, "{\"op\":\"RotatePop\"}");
        }
        flatten::FrameOp::ScalePush(scale) => {
            emit(out, "{\"op\":\"ScalePush\",\"cx\":");
            emit_num(out, scale.cx);
            emit(out, ",\"cy\":");
            emit_num(out, scale.cy);
            emit(out, ",\"sx\":");
            emit_num(out, scale.sx);
            emit(out, ",\"sy\":");
            emit_num(out, scale.sy);
            out.push(125u32);
        }
        flatten::FrameOp::ScalePop => {
            emit(out, "{\"op\":\"ScalePop\"}");
        }
        flatten::FrameOp::Backdrop(b) => {
            emit(out, "{\"op\":\"Backdrop\",\"x\":");
            emit_num(out, b.x);
            emit(out, ",\"y\":");
            emit_num(out, b.y);
            emit(out, ",\"w\":");
            emit_num(out, b.w);
            emit(out, ",\"h\":");
            emit_num(out, b.h);
            emit(out, ",\"radius\":");
            emit_num(out, b.radius);
            emit(out, ",\"blur\":");
            emit_num(out, b.blur);
            emit(out, ",\"saturate\":");
            emit_num(out, b.saturate);
            emit(out, ",\"brightness\":");
            emit_num(out, b.brightness);
            if b.smooth > 0.0 {
                emit(out, ",\"smooth\":");
                emit_num(out, b.smooth);
            }
            if b.mask_kind != 0 {
                emit(out, ",\"mask\":");
                emit_paint(out, b.mask_kind, b.mask);
            }
            out.push(125u32);
        }
        flatten::FrameOp::TiltPush(tilt) => {
            emit(out, "{\"op\":\"TiltPush\",\"cx\":");
            emit_num(out, tilt.cx);
            emit(out, ",\"cy\":");
            emit_num(out, tilt.cy);
            emit(out, ",\"rx\":");
            emit_num(out, tilt.rx);
            emit(out, ",\"ry\":");
            emit_num(out, tilt.ry);
            emit(out, ",\"depth\":");
            emit_num(out, tilt.depth);
            out.push(125u32);
        }
        flatten::FrameOp::TiltPop => {
            emit(out, "{\"op\":\"TiltPop\"}");
        }
    }
}

/// Appends the scene record at `index`.
pub fn emit_scene(out: &mut Vec<u32>, fr: &flatten::Frame, index: i32) {
    let index = usize::try_from(index).expect("scene index is non-negative");
    let node = &fr.scene[index];
    emit(out, "{\"node\":");
    emit_u32(out, node.node);
    emit(out, ",\"parent\":");
    emit_i32(out, node.parent_ix);
    emit(out, ",\"kind\":");
    emit_u32(out, node.kind);
    emit(out, ",\"x\":");
    emit_num(out, node.x);
    emit(out, ",\"y\":");
    emit_num(out, node.y);
    emit(out, ",\"w\":");
    emit_num(out, node.w);
    emit(out, ",\"h\":");
    emit_num(out, node.h);
    emit(out, ",\"radius\":");
    emit_num(out, node.radius);
    emit(out, ",\"rot\":");
    emit_num(out, node.rot_deg);
    emit(out, ",\"cx\":");
    emit_num(out, node.rot_cx);
    emit(out, ",\"cy\":");
    emit_num(out, node.rot_cy);
    emit(out, ",\"flags\":");
    emit_u32(out, node.flags);
    emit(out, ",\"content_main\":");
    emit_num(out, node.content_main);
    emit(out, ",\"scroll_off\":");
    emit_num(out, node.scroll_off);
    emit(out, ",\"line\":");
    emit_u32(out, node.src_line);
    emit(out, ",\"scroll_cross\":");
    emit_num(out, node.scroll_cross);
    emit(out, ",\"content_cross\":");
    emit_num(out, node.content_cross);
    emit(out, ",\"role\":");
    emit_u32(out, node.role);
    emit(out, ",\"label\":");
    emit_u32(out, node.label);
    emit(out, ",\"desc\":");
    emit_u32(out, node.desc);
    emit(out, ",\"checked\":");
    emit_u32(out, node.checked);
    emit(out, ",\"expanded\":");
    emit_u32(out, node.expanded);
    emit(out, ",\"selected\":");
    emit_u32(out, node.selected);
    emit(out, ",\"active_descendant\":");
    emit_u32(out, node.active_descendant);
    emit(out, ",\"controls\":");
    emit_u32(out, node.controls);
    emit(out, ",\"value_now\":");
    emit_optional_num(out, node.value_now);
    emit(out, ",\"value_min\":");
    emit_optional_num(out, node.value_min);
    emit(out, ",\"value_max\":");
    emit_optional_num(out, node.value_max);
    emit(out, ",\"value_text\":");
    emit_u32(out, node.value_text);
    emit(out, ",\"modal\":");
    emit_u32(out, node.modal);
    emit(out, ",\"live\":");
    emit_u32(out, node.live);
    emit(out, ",\"live_atomic\":");
    emit_u32(out, node.live_atomic);
    emit(out, ",\"level\":");
    emit_optional_num(out, node.level);
    emit(out, ",\"pos_in_set\":");
    emit_optional_num(out, node.pos_in_set);
    emit(out, ",\"set_size\":");
    emit_optional_num(out, node.set_size);
    emit(out, ",\"disabled\":");
    emit_bool(out, node.disabled);
    emit(out, ",\"focused\":");
    emit_bool(out, node.focused);
    out.push(OBJECT_CLOSE);
}

/// Emits the complete conformance payload: frame data and solved diagnostics.
pub fn dump(d: &slir::Doc, st: &style::St, fr: &flatten::Frame) -> String {
    let mut out: Vec<u32> = vec![];
    emit(&mut out, "{\"width\":");
    emit_num(&mut out, fr.width);
    emit(&mut out, ",\"height\":");
    emit_num(&mut out, fr.height);
    emit(&mut out, ",\"ops\":[");
    for (index, _) in fr.ops.iter().enumerate() {
        if index > 0 {
            out.push(COMMA);
        }
        emit_op(
            &mut out,
            d,
            fr,
            i32::try_from(index).expect("operation count fits i32"),
        );
    }
    emit(&mut out, "],\"scene\":[");
    for (index, _) in fr.scene.iter().enumerate() {
        if index > 0 {
            out.push(COMMA);
        }
        emit_scene(
            &mut out,
            fr,
            i32::try_from(index).expect("scene count fits i32"),
        );
    }
    emit(&mut out, "],\"strings\":[");
    for (index, string) in fr.strings.iter().enumerate() {
        if index > 0 {
            out.push(COMMA);
        }
        emit_jstr(&mut out, string);
    }
    emit(&mut out, "],\"paths_rt\":[");
    for (path_index, path) in fr.paths_rt.iter().enumerate() {
        if path_index > 0 {
            out.push(COMMA);
        }
        emit(&mut out, "{\"verbs\":[");
        for (verb_index, &verb) in path.verbs.iter().enumerate() {
            if verb_index > 0 {
                out.push(COMMA);
            }
            emit_u32(&mut out, u32::from(verb));
        }
        emit(&mut out, "],\"coords\":[");
        for (coord_index, &coord) in path.coords.iter().enumerate() {
            if coord_index > 0 {
                out.push(COMMA);
            }
            emit_num(&mut out, coord);
        }
        emit(&mut out, "]}");
    }
    emit(&mut out, "],\"diags\":[");
    for (index, code) in st.diag_code.iter().enumerate() {
        if index > 0 {
            out.push(COMMA);
        }
        emit(&mut out, "{\"code\":");
        emit_jstr(&mut out, code);
        emit(&mut out, ",\"line\":");
        emit_u32(&mut out, st.diag_line[index]);
        emit(&mut out, ",\"msg\":");
        emit_jstr(&mut out, &st.diag_msg[index]);
        out.push(OBJECT_CLOSE);
    }
    emit(&mut out, "]}");
    crate::rt::str_from_chars(&out)
}

/// Appends a node's stable scene key, or `null` for no node.
pub fn emit_key_or_null(out: &mut Vec<u32>, d: &slir::Doc, st: &style::St, node: u32) {
    if node == slir::NONE {
        emit(out, "null");
    } else {
        emit_jstr(out, &scene::key_of(d, &st.lists, node));
    }
}

/// Appends a four-number rectangle, or `null` when absent.
pub fn emit_rectf(out: &mut Vec<u32>, has: bool, x: f64, y: f64, w: f64, h: f64) {
    if !has {
        emit(out, "null");
        return;
    }
    out.push(ARRAY_OPEN);
    emit_num(out, x);
    out.push(COMMA);
    emit_num(out, y);
    out.push(COMMA);
    emit_num(out, w);
    out.push(COMMA);
    emit_num(out, h);
    out.push(ARRAY_CLOSE);
}

/// Emits one dispatched event's effects in canonical key order.
pub fn dump_effects(d: &slir::Doc, st: &style::St, effects: &dispatch::Effects) -> String {
    let mut out = Vec::new();
    emit(&mut out, "{\"repaint\":");
    emit_bool(&mut out, effects.repaint);
    emit(&mut out, ",\"signals\":[");
    for (index, name) in effects.sig_name.iter().enumerate() {
        if index > 0 {
            out.push(COMMA);
        }
        let meta = &effects.sig_meta[index];
        emit(&mut out, "{\"name\":");
        emit_jstr(&mut out, &slir::str_at(d, *name));
        emit(&mut out, ",\"text\":");
        emit_jstr(&mut out, &effects.sig_text[index]);
        emit(&mut out, ",\"item\":");
        emit_jstr(&mut out, &effects.sig_item[index]);
        emit(&mut out, ",\"meta\":{\"x\":");
        emit_num(&mut out, meta.x);
        emit(&mut out, ",\"y\":");
        emit_num(&mut out, meta.y);
        emit(&mut out, ",\"dx\":");
        emit_num(&mut out, meta.dx);
        emit(&mut out, ",\"dy\":");
        emit_num(&mut out, meta.dy);
        emit(&mut out, ",\"drag_dx\":");
        emit_num(&mut out, meta.drag_dx);
        emit(&mut out, ",\"drag_dy\":");
        emit_num(&mut out, meta.drag_dy);
        emit(&mut out, ",\"mods\":");
        emit_u32(&mut out, meta.mods);
        emit(&mut out, ",\"button\":");
        emit_u32(&mut out, meta.button);
        emit(&mut out, ",\"clicks\":");
        emit_u32(&mut out, meta.clicks);
        emit(&mut out, ",\"key\":");
        emit_jstr(&mut out, &meta.key);
        emit(&mut out, ",\"src_key\":");
        emit_jstr(&mut out, &meta.src_key);
        emit(&mut out, ",\"src_item\":");
        emit_jstr(&mut out, &meta.src_item);
        emit(&mut out, ",\"cancelled\":");
        emit_bool(&mut out, meta.cancelled);
        emit(&mut out, ",\"dropped\":");
        emit_bool(&mut out, meta.dropped);
        out.push(OBJECT_CLOSE);
        out.push(OBJECT_CLOSE);
    }
    emit(&mut out, "],\"caret\":");
    emit_rectf(
        &mut out,
        effects.has_caret,
        effects.caret_x,
        effects.caret_y,
        effects.caret_w,
        effects.caret_h,
    );
    emit(&mut out, ",\"ime\":");
    emit_rectf(
        &mut out,
        effects.has_ime,
        effects.ime_x,
        effects.ime_y,
        effects.ime_w,
        effects.ime_h,
    );
    emit(&mut out, ",\"cursor\":");
    emit_u32(&mut out, effects.cursor);
    emit(&mut out, ",\"focus\":");
    emit_key_or_null(&mut out, d, st, effects.focus);
    emit(&mut out, ",\"scrolls\":[");
    for (index, scroll) in effects.scrolls.iter().enumerate() {
        if index > 0 {
            out.push(COMMA);
        }
        emit(&mut out, "{\"key\":");
        emit_jstr(&mut out, &scroll.key);
        emit(&mut out, ",\"axis\":");
        emit_u32(&mut out, scroll.axis);
        emit(&mut out, ",\"off\":");
        emit_num(&mut out, scroll.off);
        out.push(OBJECT_CLOSE);
    }
    out.push(ARRAY_CLOSE);
    out.push(OBJECT_CLOSE);
    crate::rt::str_from_chars(&out)
}

/// Emits a hit-query result whose node keys run from root to target.
pub fn dump_hit(d: &slir::Doc, st: &style::St, nodes: &[u32]) -> String {
    let mut out = Vec::new();
    emit(&mut out, "{\"hit\":[");
    for (index, node) in nodes.iter().enumerate() {
        if index > 0 {
            out.push(COMMA);
        }
        emit_jstr(&mut out, &scene::key_of(d, &st.lists, *node));
    }
    emit(&mut out, "]}");
    crate::rt::str_from_chars(&out)
}

/// Emits the final interaction state.
///
/// Committed edit text is keyed by each field's Change-signal name in edit
/// creation order. Scroll offsets follow scene/document order, with the main
/// axis before cross for each node.
pub fn dump_trace_summary(d: &slir::Doc, st: &style::St, instance: &frame::Instance) -> String {
    let mut out = Vec::new();
    emit(&mut out, "{\"focus\":");
    emit_key_or_null(&mut out, d, st, instance.ds.fs.focus);
    emit(&mut out, ",\"edits\":[");
    let mut first = true;
    for (edit_index, node) in instance.ds.ed_node.iter().copied().enumerate() {
        let base_node = list::base(&st.lists, d, node);
        let signal = d
            .sign_node
            .iter()
            .zip(&d.sign_trigger)
            .enumerate()
            .filter(|(_, (signal_node, trigger))| **signal_node == base_node && **trigger == 1)
            .map(|(index, _)| index)
            .next_back();

        if let Some(signal) = signal {
            if !first {
                out.push(COMMA);
            }
            first = false;
            emit(&mut out, "{\"name\":");
            emit_jstr(&mut out, &slir::str_at(d, d.sign_name[signal]));
            emit(&mut out, ",\"text\":");
            emit_jstr(&mut out, &edit::text_str(&instance.ds.ed[edit_index]));
            emit(&mut out, ",\"item\":");
            emit_jstr(&mut out, &list::item_key(&st.lists, d, node));
            out.push(OBJECT_CLOSE);
        }
    }
    emit(&mut out, "],\"scroll\":[");
    let mut scrolls = Vec::with_capacity(
        st.scroll_node
            .len()
            .saturating_add(st.scroll_cross_node.len()),
    );
    scrolls.extend(
        st.scroll_node
            .iter()
            .copied()
            .zip(st.scroll_off.iter().copied())
            .map(|(node, off)| (node, 0u32, off)),
    );
    scrolls.extend(
        st.scroll_cross_node
            .iter()
            .copied()
            .zip(st.scroll_cross_off.iter().copied())
            .map(|(node, off)| (node, 1u32, off)),
    );
    scrolls.sort_by_key(|&(node, axis, _)| {
        let scene_index = instance
            .sc
            .node
            .iter()
            .position(|&candidate| candidate == node);
        match scene_index {
            Some(index) => (false, index, axis),
            None => (
                true,
                usize::try_from(node).expect("node id exceeds usize"),
                axis,
            ),
        }
    });
    for (index, (node, axis, off)) in scrolls.into_iter().enumerate() {
        if index > 0 {
            out.push(COMMA);
        }
        emit(&mut out, "{\"key\":");
        emit_jstr(&mut out, &scene::key_of(d, &st.lists, node));
        emit(&mut out, ",\"off\":");
        emit_num(&mut out, off);
        emit(&mut out, ",\"axis\":");
        emit_u32(&mut out, axis);
        out.push(OBJECT_CLOSE);
    }
    emit(&mut out, "]}");
    crate::rt::str_from_chars(&out)
}
