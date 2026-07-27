//! JSON snapshots for cold-path document metadata, effects, and scene access.

use serde::Serialize;
use slab_kernel::{
    dispatch::Effects,
    frame::{HoleRect, Instance},
    hit, list, motion, scene, slir,
};

#[derive(Serialize)]
struct OwnedParamSnapshot {
    kind: u32,
    num: f64,
    s: String,
    rgba: u32,
    sym: String,
}

#[derive(Serialize)]
struct ParamDef<'a> {
    name: &'a str,
    ty: u32,
    enum_symbols: Vec<&'a str>,
}

#[derive(Serialize)]
struct SignalDef<'a> {
    name: &'a str,
    trigger: u32,
}

#[derive(Serialize)]
struct HoleDef<'a> {
    name: &'a str,
    node: u32,
    scroll: bool,
}

#[derive(Serialize)]
struct ListFieldDef<'a> {
    name: &'a str,
    ty: u32,
    #[serde(rename = "default")]
    default_value: OwnedParamSnapshot,
    enum_symbols: Vec<&'a str>,
}

#[derive(Serialize)]
struct ListDef<'a> {
    param: u32,
    fields: Vec<ListFieldDef<'a>>,
}

#[derive(Serialize)]
struct LiftStopSnapshot {
    pos: f64,
    ctrl: (f64, f64),
    offset: Option<(f64, f64)>,
    opacity: Option<f64>,
    rotate: Option<f64>,
    scale: Option<f64>,
    bg: Option<u32>,
    color: Option<u32>,
}

#[derive(Serialize)]
struct LiftSnapshot {
    binding: usize,
    node: u32,
    kind: u32,
    dur: f64,
    delay: f64,
    mode: u32,
    base_offset: (f64, f64),
    base_rotate: f64,
    base_scale: (f64, f64),
    stops: Vec<LiftStopSnapshot>,
}

#[derive(Serialize)]
struct Statics<'a> {
    strs: &'a [String],
    font_family: &'a [u32],
    font_class: &'a [u32],
    font_weight: &'a [u32],
    font_upem: &'a [u32],
    font_ascent: &'a [i32],
    font_descent: &'a [i32],
    img_src: &'a [u32],
    grad_kind: &'a [u32],
    grad_angle: &'a [f64],
    grad_stop_off: &'a [i32],
    grad_stop_len: &'a [i32],
    grad_stop_pos: &'a [f64],
    grad_stop_rgba: &'a [u32],
    path_verb_off: &'a [i32],
    path_verb_len: &'a [i32],
    path_coord_off: &'a [i32],
    path_coord_len: &'a [i32],
    path_verbs: &'a [u32],
    path_coords: &'a [f64],
    shdw_x: &'a [f64],
    shdw_y: &'a [f64],
    shdw_blur: &'a [f64],
    shdw_spread: &'a [f64],
    shdw_rgba: &'a [u32],
    shdw_inset: &'a [u32],
    params: Vec<ParamDef<'a>>,
    signals: Vec<SignalDef<'a>>,
    holes: Vec<HoleDef<'a>>,
    lists: Vec<ListDef<'a>>,
}

#[derive(Serialize)]
struct SigMetaSnapshot<'a> {
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    drag_dx: f64,
    drag_dy: f64,
    mods: u32,
    button: u32,
    clicks: u32,
    key: &'a str,
    src_key: &'a str,
    src_item: &'a str,
    cancelled: bool,
    dropped: bool,
}

#[derive(Serialize)]
struct ScrollChangeSnapshot<'a> {
    key: &'a str,
    axis: u32,
    off: f64,
}

#[derive(Serialize)]
struct EffectSnapshot<'a> {
    repaint: bool,
    sig_name: &'a [u32],
    sig_text: &'a [String],
    sig_item: &'a [String],
    has_caret: bool,
    caret_x: f64,
    caret_y: f64,
    caret_w: f64,
    caret_h: f64,
    has_ime: bool,
    ime_x: f64,
    ime_y: f64,
    ime_w: f64,
    ime_h: f64,
    cursor: u32,
    focus: u32,
    sig_meta: Vec<SigMetaSnapshot<'a>>,
    scrolls: Vec<ScrollChangeSnapshot<'a>>,
}

#[derive(Serialize)]
struct HoleSnapshot {
    hole: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    clip: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
enum CheckedSnapshot {
    Boolean(bool),
    Mixed(&'static str),
}

#[derive(Serialize)]
struct SceneSnapshot<'a> {
    key: String,
    node: u32,
    parent: i32,
    kind: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    rotation: f64,
    cx: f64,
    cy: f64,
    flags: u32,
    content_main: f64,
    scroll_off: f64,
    is_row: bool,
    scroll: bool,
    src_line: u32,
    scroll_cross: f64,
    content_cross: f64,
    role: &'a str,
    label: &'a str,
    desc: &'a str,
    checked: Option<CheckedSnapshot>,
    expanded: Option<bool>,
    selected: Option<bool>,
    active_descendant: &'a str,
    controls: &'a str,
    value_now: Option<f64>,
    value_min: Option<f64>,
    value_max: Option<f64>,
    value_text: &'a str,
    modal: Option<bool>,
    live: Option<&'static str>,
    live_atomic: Option<bool>,
    level: Option<f64>,
    pos_in_set: Option<f64>,
    set_size: Option<f64>,
    disabled: bool,
    focused: bool,
}

pub(crate) fn statics_json(instance: &Instance) -> String {
    let document = &instance.doc;
    let params = document
        .parm_name
        .iter()
        .zip(&document.parm_type)
        .enumerate()
        .map(|(param, (&name, &ty))| ParamDef {
            name: string_at(document, name),
            ty,
            enum_symbols: enum_symbols(
                document,
                document.parm_enum_off[param],
                document.parm_enum_len[param],
                &document.parm_enum_syms,
            ),
        })
        .collect();
    let signals = document
        .sign_name
        .iter()
        .zip(&document.sign_trigger)
        .map(|(&name, &trigger)| SignalDef {
            name: string_at(document, name),
            trigger,
        })
        .collect();
    let holes = document
        .hole_name
        .iter()
        .zip(&document.hole_node)
        .map(|(&name, &node)| HoleDef {
            name: string_at(document, name),
            node,
            scroll: document.node_flags[index_u32(node)] & slir::F_SCROLL != 0,
        })
        .collect();
    let lists = document
        .list_param
        .iter()
        .enumerate()
        .map(|(schema, &param)| {
            let offset = document.list_field_off[schema];
            let length = document.list_field_len[schema];
            let fields = (offset..offset.wrapping_add(length))
                .map(|field| {
                    let field_index = index_i32(field);
                    let ty = document.list_field_type[field_index];
                    let value = list::val_from_aval(
                        document,
                        ty,
                        signed(document.list_field_default[field_index]),
                    );
                    ListFieldDef {
                        name: string_at(document, document.list_field_name[field_index]),
                        ty,
                        default_value: OwnedParamSnapshot {
                            kind: value.kind,
                            num: value.num,
                            s: value.s,
                            rgba: value.rgba,
                            sym: value.sym,
                        },
                        enum_symbols: enum_symbols(
                            document,
                            document.list_field_enum_off[field_index],
                            document.list_field_enum_len[field_index],
                            &document.list_enum_syms,
                        ),
                    }
                })
                .collect();
            ListDef { param, fields }
        })
        .collect();

    let statics = Statics {
        strs: &document.strs,
        font_family: &document.font_family,
        font_class: &document.font_class,
        font_weight: &document.font_weight,
        font_upem: &document.font_upem,
        font_ascent: &document.font_ascent,
        font_descent: &document.font_descent,
        img_src: &document.img_src,
        grad_kind: &document.grad_kind,
        grad_angle: &document.grad_angle,
        grad_stop_off: &document.grad_stop_off,
        grad_stop_len: &document.grad_stop_len,
        grad_stop_pos: &document.grad_stop_pos,
        grad_stop_rgba: &document.grad_stop_rgba,
        path_verb_off: &document.path_verb_off,
        path_verb_len: &document.path_verb_len,
        path_coord_off: &document.path_coord_off,
        path_coord_len: &document.path_coord_len,
        path_verbs: &document.path_verbs,
        path_coords: &document.path_coords,
        shdw_x: &document.shdw_x,
        shdw_y: &document.shdw_y,
        shdw_blur: &document.shdw_blur,
        shdw_spread: &document.shdw_spread,
        shdw_rgba: &document.shdw_rgba,
        shdw_inset: &document.shdw_inset,
        params,
        signals,
        holes,
        lists,
    };
    to_json(&statics)
}

pub(crate) fn effects_json(effects: &Effects) -> String {
    let sig_meta = effects
        .sig_meta
        .iter()
        .map(|meta| SigMetaSnapshot {
            x: meta.x,
            y: meta.y,
            dx: meta.dx,
            dy: meta.dy,
            drag_dx: meta.drag_dx,
            drag_dy: meta.drag_dy,
            mods: meta.mods,
            button: meta.button,
            clicks: meta.clicks,
            key: &meta.key,
            src_key: &meta.src_key,
            src_item: &meta.src_item,
            cancelled: meta.cancelled,
            dropped: meta.dropped,
        })
        .collect();
    let scrolls = effects
        .scrolls
        .iter()
        .map(|scroll| ScrollChangeSnapshot {
            key: &scroll.key,
            axis: scroll.axis,
            off: scroll.off,
        })
        .collect();
    to_json(&EffectSnapshot {
        repaint: effects.repaint,
        sig_name: &effects.sig_name,
        sig_text: &effects.sig_text,
        sig_item: &effects.sig_item,
        has_caret: effects.has_caret,
        caret_x: effects.caret_x,
        caret_y: effects.caret_y,
        caret_w: effects.caret_w,
        caret_h: effects.caret_h,
        has_ime: effects.has_ime,
        ime_x: effects.ime_x,
        ime_y: effects.ime_y,
        ime_w: effects.ime_w,
        ime_h: effects.ime_h,
        cursor: effects.cursor,
        focus: effects.focus,
        sig_meta,
        scrolls,
    })
}

pub(crate) fn holes_json(holes: &[HoleRect]) -> String {
    let holes: Vec<_> = holes
        .iter()
        .map(|hole| HoleSnapshot {
            hole: hole.hole,
            x: hole.x,
            y: hole.y,
            w: hole.w,
            h: hole.h,
            clip: hole.clip,
        })
        .collect();
    to_json(&holes)
}

pub(crate) fn lifts_json(lifts: &[motion::Lift]) -> String {
    let lifts: Vec<_> = lifts
        .iter()
        .map(|lift| LiftSnapshot {
            binding: lift.binding,
            node: lift.node,
            kind: lift.kind,
            dur: lift.dur,
            delay: lift.delay,
            mode: lift.mode,
            base_offset: lift.base_offset,
            base_rotate: lift.base_rotate,
            base_scale: lift.base_scale,
            stops: lift
                .stops
                .iter()
                .map(|stop| LiftStopSnapshot {
                    pos: stop.pos,
                    ctrl: stop.ctrl,
                    offset: stop.offset,
                    opacity: stop.opacity,
                    rotate: stop.rotate,
                    scale: stop.scale,
                    bg: stop.bg,
                    color: stop.color,
                })
                .collect(),
        })
        .collect();
    to_json(&lifts)
}

pub(crate) fn scene_json(instance: &Instance) -> String {
    let scene = &instance.sc;
    let nodes: Vec<_> = scene
        .node
        .iter()
        .enumerate()
        .map(|(index, &node)| {
            let base = list::base(&instance.st.lists, &instance.doc, node);
            let src_line = if base == slir::NONE {
                0
            } else {
                instance.doc.node_line[index_u32(base)]
            };
            SceneSnapshot {
                key: scene::key_of(&instance.doc, &instance.st.lists, node),
                node,
                parent: scene.parent[index],
                kind: scene.kind[index],
                x: scene.x[index],
                y: scene.y[index],
                w: scene.w[index],
                h: scene.h[index],
                radius: scene.radius[index],
                rotation: scene.rot[index],
                cx: scene.cx[index],
                cy: scene.cy[index],
                flags: scene.flags[index],
                content_main: scene.content_main[index],
                scroll_off: scene.scroll_off[index],
                is_row: scene.is_row[index],
                scroll: scene.flags[index] & slir::F_SCROLL != 0,
                src_line,
                scroll_cross: scene.scroll_cross[index],
                content_cross: scene.content_cross[index],
                role: scene_string_at(&instance.st.scene_strs, scene.role[index]),
                label: scene_string_at(&instance.st.scene_strs, scene.label[index]),
                desc: scene_string_at(&instance.st.scene_strs, scene.desc[index]),
                checked: checked_snapshot(scene.checked[index]),
                expanded: optional_bool(scene.expanded[index]),
                selected: optional_bool(scene.selected[index]),
                active_descendant: scene_string_at(
                    &instance.st.scene_strs,
                    scene.active_descendant[index],
                ),
                controls: scene_string_at(&instance.st.scene_strs, scene.controls[index]),
                value_now: scene.value_now[index],
                value_min: scene.value_min[index],
                value_max: scene.value_max[index],
                value_text: scene_string_at(&instance.st.scene_strs, scene.value_text[index]),
                modal: optional_bool(scene.modal[index]),
                live: live_snapshot(scene.live[index]),
                live_atomic: optional_bool(scene.live_atomic[index]),
                level: scene.level[index],
                pos_in_set: scene.pos_in_set[index],
                set_size: scene.set_size[index],
                disabled: scene.disabled[index],
                focused: scene.focused[index],
            }
        })
        .collect();
    to_json(&nodes)
}

pub(crate) fn chain_json(instance: &Instance, scene_index: i32) -> String {
    let mut chain = Vec::new();
    scene::chain(&instance.sc, scene_index, &mut chain);
    to_json(&chain)
}

pub(crate) fn hit_contains(instance: &Instance, scene_index: i32, x: f64, y: f64) -> bool {
    hit::contains(&instance.sc, scene_index, x, y)
}

fn enum_symbols<'a>(
    document: &'a slir::Doc,
    offset: i32,
    length: i32,
    pool: &[u32],
) -> Vec<&'a str> {
    let start = index_i32(offset);
    let length = index_i32(length);
    pool[start..start + length]
        .iter()
        .map(|&symbol| string_at(document, symbol))
        .collect()
}

fn string_at(document: &slir::Doc, index: u32) -> &str {
    document.strs[index_u32(index)].as_str()
}

fn optional_bool(code: u32) -> Option<bool> {
    match code {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

fn checked_snapshot(code: u32) -> Option<CheckedSnapshot> {
    match code {
        1 => Some(CheckedSnapshot::Boolean(false)),
        2 => Some(CheckedSnapshot::Boolean(true)),
        3 => Some(CheckedSnapshot::Mixed("mixed")),
        _ => None,
    }
}

fn live_snapshot(code: u32) -> Option<&'static str> {
    match code {
        1 => Some("off"),
        2 => Some("polite"),
        3 => Some("assertive"),
        _ => None,
    }
}

fn scene_string_at(strings: &[String], reference: u32) -> &str {
    strings[index_u32(reference)].as_str()
}

fn index_i32(value: i32) -> usize {
    usize::try_from(value).expect("kernel index must be nonnegative")
}

fn index_u32(value: u32) -> usize {
    usize::try_from(value).expect("kernel index exceeds usize")
}

fn signed(value: u32) -> i32 {
    i32::from_ne_bytes(value.to_ne_bytes())
}

fn to_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("kernel snapshot contains finite serializable values")
}
