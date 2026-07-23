//! Negative-path fixtures: each 1.0 diagnostic code fires with the right
//! code, level, and line.

use slab_compile::{Options, compile};
use slab_syntax::diag::Level;

fn diags_of(src: &str) -> Vec<(String, Level, u32)> {
    let (_, diags) = compile(src, &Options::default());
    diags
        .0
        .iter()
        .map(|d| (d.code.to_string(), d.level, d.line))
        .collect()
}

fn assert_has(src: &str, code: &str, level: Level, line: u32) {
    let ds = diags_of(src);
    assert!(
        ds.iter()
            .any(|(c, l, ln)| c == code && *l == level && *ln == line),
        "expected {level:?}[{code}] at line {line}, got {ds:?}"
    );
}

#[test]
fn param_type_bad_default() {
    assert_has(
        "params {\n  title text = 42\n}\ncol w=fill { }\n",
        "param-type",
        Level::Error,
        2,
    );
}

#[test]
fn param_type_non_bool_when_cond() {
    assert_has(
        "params {\n  title text = \"x\"\n}\ncol w=fill {\n  when title { bg=#fff }\n}\n",
        "param-type",
        Level::Error,
        5,
    );
}

#[test]
fn ref_unknown_param() {
    assert_has(
        "col w=fill {\n  text param.nope\n}\n",
        "ref",
        Level::Error,
        2,
    );
}

#[test]
fn hug_hole_accepts_both_axes_and_clamps() {
    let ds = diags_of("col w=fill {\n  hole content w=hug h=hug min-w=120 max-h=240\n}\n");
    assert!(
        ds.is_empty(),
        "hug-sized hole with min/max clamps should compile cleanly: {ds:?}"
    );
}

#[test]
fn dup_param_warns() {
    assert_has(
        "params {\n  t text = \"a\"\n  t text = \"b\"\n}\ncol w=fill { }\n",
        "dup-param",
        Level::Warning,
        3,
    );
}

#[test]
fn dup_hole_errors() {
    assert_has(
        "col w=fill {\n  hole a w=fill h=40\n  hole a w=fill h=40\n}\n",
        "dup-hole",
        Level::Error,
        3,
    );
}

#[test]
fn shadow_warns_on_attr_named_def_param() {
    assert_has(
        "def Row2(d) {\n  text d\n}\ncol w=fill {\n  Row2 d=\"x\"\n}\n",
        "shadow",
        Level::Warning,
        1,
    );
}

#[test]
fn dup_signal_warns_on_mixed_payload_types() {
    assert_has(
        "col w=fill {\n  row focusable act=save { text \"s\" }\n  text \"n\" field=save\n}\n",
        "dup-signal",
        Level::Warning,
        3,
    );
}

#[test]
fn dup_signal_treats_resize_as_text_bearing() {
    assert_has(
        "row {\n  box press=changed\n  divider w=6 resize=changed\n  box\n}\n",
        "dup-signal",
        Level::Warning,
        3,
    );
}

#[test]
fn bool_param_when_defers_to_state_cond() {
    let (slir, diags) = compile(
        "params {\n  hot bool = false\n}\ncol w=fill {\n  when hot { bg=#fff }\n}\n",
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    assert!(!diags.has_errors(), "{:?}", diags.0);
    let slir = slir.unwrap();
    assert_eq!(slir.conds.len(), 1);
    assert_eq!(slir.conds[0].kind, slab_slir::cond::STATE);
    assert_eq!(slir.str_at(slir.conds[0].sym), "hot");
}

#[test]
fn field_implies_focusable_and_change_signal() {
    let (slir, _) = compile(
        "col w=fill {\n  text \"n\" field=username\n}\n",
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    let slir = slir.unwrap();
    assert_eq!(slir.signals.len(), 1);
    let (name, node, trigger) = slir.signals[0];
    assert_eq!(slir.str_at(name), "username");
    assert_eq!(trigger, 1);
    assert_ne!(
        slir.nodes.flags[node as usize] & slab_slir::flags::FOCUSABLE,
        0,
        "field implies focusable"
    );
}

#[test]
fn gesture_triggers_emit_and_only_press_and_drag_imply_focusable() {
    let source = r#"
col {
  box press=pressed
  box drag=started drag-update=moving drag-end=ended drag-ghost
  box context=menu dblclick=twice drop=dropped
  box pointer-move=hovered pointer-up=released
  row {
    rect w=40
    divider w=6 resize=resized
    rect w=40
  }
}
"#;
    let (slir, diagnostics) = compile(
        source,
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
    let slir = slir.expect("gesture document");
    for (name, trigger) in [
        ("pressed", 3),
        ("menu", 4),
        ("twice", 5),
        ("started", 6),
        ("dropped", 7),
        ("resized", 8),
        ("hovered", 9),
        ("released", 10),
        ("moving", 11),
        ("ended", 12),
    ] {
        assert!(
            slir.signals
                .iter()
                .any(|&(name_ref, _, candidate)| slir.str_at(name_ref) == name
                    && candidate == trigger)
        );
    }
    for name in ["pressed", "started"] {
        let node = slir
            .signals
            .iter()
            .find_map(|&(name_ref, node, _)| (slir.str_at(name_ref) == name).then_some(node))
            .expect("focus-implying signal");
        assert_ne!(
            slir.nodes.flags[node as usize] & slab_slir::flags::FOCUSABLE,
            0,
            "{name}"
        );
    }
    for name in ["menu", "twice", "dropped", "hovered", "released"] {
        let node = slir
            .signals
            .iter()
            .find_map(|&(name_ref, node, _)| (slir.str_at(name_ref) == name).then_some(node))
            .expect("non-focus-implying signal");
        assert_eq!(
            slir.nodes.flags[node as usize] & slab_slir::flags::FOCUSABLE,
            0,
            "{name}"
        );
    }
    let drag_node = slir
        .signals
        .iter()
        .find_map(|&(name_ref, node, _)| (slir.str_at(name_ref) == "started").then_some(node))
        .expect("drag node");
    assert_ne!(
        slir.nodes.flags[drag_node as usize] & slab_slir::flags::DRAG_GHOST,
        0
    );
}

#[test]
fn gesture_bindings_are_rejected_inside_when_patches() {
    let source = r#"
box {
  when hot {
    press=pressed context=menu dblclick=twice drag=started drop=dropped resize=resized pointer-move=moved pointer-up=released drag-update=updated drag-end=ended
  }
}
"#;
    let (_, diagnostics) = compile(
        source,
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    for attribute in [
        "press",
        "context",
        "dblclick",
        "drag",
        "drop",
        "resize",
        "pointer-move",
        "pointer-up",
        "drag-update",
        "drag-end",
    ] {
        assert!(
            diagnostics.0.iter().any(|diagnostic| {
                diagnostic.code == "attr"
                    && diagnostic.level == Level::Warning
                    && (diagnostic.line == 3 || diagnostic.line == 4)
                    && diagnostic
                        .msg
                        .contains(&format!("{attribute} inside a `when` patch"))
            }),
            "{attribute}: {:?}",
            diagnostics.0
        );
    }
}

#[test]
fn drag_companions_require_drag_binding() {
    let (slir, diagnostics) = compile(
        "box drag-update=updated drag-end=ended drag-ghost\n",
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    let slir = slir.expect("warnings do not reject the document");
    assert_eq!(slir.signals.len(), 0);
    assert_eq!(slir.nodes.flags[0] & slab_slir::flags::DRAG_GHOST, 0);
    for message in [
        "`drag-update=` requires",
        "`drag-end=` requires",
        "`drag-ghost` requires",
    ] {
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == "attr" && diagnostic.msg.contains(message)),
            "{message}: {:?}",
            diagnostics.0
        );
    }
}

#[test]
fn divider_rejects_generic_drag_surface() {
    let source = r#"
row {
  rect w=40
  divider w=6 drag=started drag-update=updated drag-end=ended drag-ghost
  rect w=40
}
"#;
    let (slir, diagnostics) = compile(
        source,
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    let slir = slir.expect("divider drag diagnostics are warnings");
    for attribute in ["`drag=`", "`drag-update=`", "`drag-end=`", "`drag-ghost`"] {
        assert!(
            diagnostics.0.iter().any(|diagnostic| {
                diagnostic.code == "attr"
                    && diagnostic.level == Level::Warning
                    && diagnostic.msg.contains(attribute)
                    && diagnostic.msg.contains("divider owns its resize gesture")
            }),
            "{attribute}: {:?}",
            diagnostics.0
        );
    }
    assert!(
        slir.signals
            .iter()
            .all(|&(_, _, trigger)| !matches!(trigger, 6 | 11 | 12))
    );
    let divider = slir
        .nodes
        .kind
        .iter()
        .position(|&kind| kind == slab_slir::kind::DIVIDER)
        .expect("divider node");
    assert_eq!(slir.nodes.flags[divider] & slab_slir::flags::DRAG_GHOST, 0);
}

#[test]
fn scrollbar_rejects_unknown_mode() {
    assert_has(
        "col scroll scrollbar=\"sometimes\" { }\n",
        "ref",
        Level::Error,
        1,
    );
}

#[test]
fn scrollbar_warns_without_scroll_flag() {
    assert_has("col scrollbar=always { }\n", "attr", Level::Warning, 1);
}

#[test]
fn unknown_activation_key_warns() {
    assert_has("col keys=LaunchMail { }\n", "attr", Level::Warning, 1);
}

#[test]
fn activation_keys_imply_focusable() {
    let (slir, diags) = compile(
        "col keys=Escape,F2 act=cancel { }\n",
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    assert!(!diags.has_errors(), "{:?}", diags.0);
    let slir = slir.unwrap();
    assert_ne!(
        slir.nodes.flags[0] & slab_slir::flags::FOCUSABLE,
        0,
        "keys implies focusable"
    );
    let value = slir
        .node_attrs(0)
        .iter()
        .find(|(id, _)| *id == slab_slir::attrs::KEYS)
        .map(|(_, value)| slir.avals[*value as usize])
        .expect("keys attr");
    assert_eq!(value.tag, slab_slir::aval::STR);
    assert_eq!(slir.str_at(value.lo()), "Escape,F2");
}

#[test]
fn content_keyframes_warn_on_non_text_bind() {
    assert_has(
        "anim spin {\n  0% { content=\"a\" }\n  100% { content=\"b\" }\n}\nrect animate=spin,1000\n",
        "attr",
        Level::Warning,
        5,
    );
}

#[test]
fn divider_requires_a_middle_row_or_col_position() {
    assert_has(
        "row {\n  divider w=6\n  rect w=fill\n}\n",
        "divider-ctx",
        Level::Error,
        2,
    );
    assert_has(
        "row {\n  rect w=fill\n  divider w=6\n}\n",
        "divider-ctx",
        Level::Error,
        3,
    );
    assert_has(
        "stack {\n  rect w=40\n  divider w=6\n  rect w=40\n}\n",
        "divider-ctx",
        Level::Error,
        3,
    );
}

#[test]
fn divider_accepts_fixed_percentage_hug_and_fill_footprints() {
    for extent in ["6", "10%", "hug", "fill"] {
        let source = format!(
            "row w=300 h=80 {{\n  rect w=100\n  divider w={extent}\n  rect w=fill min-w=70\n}}\n"
        );
        let (_, diags) = compile(
            &source,
            &Options {
                embed_assets: false,
                ..Default::default()
            },
        );
        assert!(!diags.has_errors(), "{extent}: {:?}", diags.0);
    }
}

#[test]
fn divider_is_focusable_and_registers_resize() {
    let (slir, diags) = compile(
        "row w=240 h=80 {\n  rect w=fill\n  divider #split w=6 resize=split_changed\n  rect w=fill\n}\n",
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    assert!(!diags.has_errors(), "{:?}", diags.0);
    let slir = slir.unwrap();
    let divider = slir
        .nodes
        .kind
        .iter()
        .position(|kind| *kind == slab_slir::kind::DIVIDER)
        .expect("divider node");
    assert_ne!(
        slir.nodes.flags[divider] & slab_slir::flags::FOCUSABLE,
        0,
        "divider is focusable by default"
    );
    assert!(slir.signals.iter().any(|(name, node, trigger)| {
        *node as usize == divider && *trigger == 8 && slir.str_at(*name) == "split_changed"
    }));
}

#[test]
fn icon_declarations_reject_duplicate_names_and_non_path_bodies() {
    assert_has(
        "icon mark {\n  rect w=4 h=4\n}\n",
        "icon-body",
        Level::Error,
        1,
    );
    assert_has(
        "icon mark { path \"M0 0 L1 1\" }\nicon mark { path \"M0 0 L2 2\" }\n",
        "icon-dup",
        Level::Error,
        2,
    );
    assert_has(
        "icon mark { path \"M0 0 L1 1\" act=save keys=Enter transition=100 }\n",
        "icon-body",
        Level::Error,
        1,
    );
}

#[test]
fn accessibility_semantics_accept_typed_dynamic_values() {
    let source = r##"params {
  open bool = false
  title text = "Current"
  now num = 5
}
col #root role=listbox label=param.title expanded=param.open checked=mixed \
  active-descendant="#root/#choice" value-now=param.now value-min=0 value-max=10 \
  value-text=param.title live=polite live-atomic=true level=1 {
  row #choice role=option selected=true pos-in-set=1 set-size=1
}
rect #panel role=region modal=false
col #controller controls="#panel"
"##;
    let (slir, diags) = compile(
        source,
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    assert!(!diags.has_errors(), "{:?}", diags.0);
    let slir = slir.expect("semantic document");
    let root = slir
        .nodes
        .key
        .iter()
        .position(|&key| slir.str_at(key) == "#root")
        .expect("root node");
    let root_attrs = slir.node_attrs(u32::try_from(root).expect("node index exceeds u32"));
    for expected in [
        slab_slir::attrs::ROLE,
        slab_slir::attrs::LABEL,
        slab_slir::attrs::CHECKED,
        slab_slir::attrs::EXPANDED,
        slab_slir::attrs::ACTIVE_DESCENDANT,
        slab_slir::attrs::VALUE_NOW,
        slab_slir::attrs::LIVE,
        slab_slir::attrs::LIVE_ATOMIC,
        slab_slir::attrs::LEVEL,
    ] {
        assert!(
            root_attrs
                .iter()
                .any(|(attribute, _)| *attribute == expected),
            "missing semantic attr {expected}"
        );
    }
}

#[test]
fn accessibility_semantics_reject_invalid_domains_ranges_and_keys() {
    assert_has("col checked=maybe\n", "ref", Level::Error, 1);
    assert_has("col checked=\"maybe\"\n", "ref", Level::Error, 1);
    assert_has("col live=urgent\n", "ref", Level::Error, 1);
    assert_has("col role=42\n", "ref", Level::Error, 1);
    assert_has("col label=42\n", "ref", Level::Error, 1);
    assert_has("col expanded=1\n", "ref", Level::Error, 1);
    assert_has("col value-now=\"many\"\n", "ref", Level::Error, 1);
    assert_has("col level=0\n", "a11y-range", Level::Error, 1);
    assert_has(
        "col value-min=10 value-now=5 value-max=20\n",
        "a11y-range",
        Level::Error,
        1,
    );
    assert_has("col controls=\"#missing\"\n", "a11y-key", Level::Error, 1);
    assert_has(
        "col pos-in-set=3 set-size=2\n",
        "a11y-range",
        Level::Error,
        1,
    );
}

#[test]
fn drag_ghost_appends_paint_without_scene_identity() {
    let source = r#"
row w=300 h=64 gap=40 {
  box key=source w=80 h=64 drag=started drag-ghost bg=#314158 {
    text "source" color=#ffffff
  }
  box key=target w=100 h=64 drop=dropped bg=#234738
}
"#;
    let (slir, diagnostics) = compile(
        source,
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
    let bytes = slab_slir::write(&slir.expect("drag ghost document"));
    let (mut instance, _) = slab_slir::instance(&bytes).expect("drag ghost instance");
    slab_kernel::frame::inst_set_env(&mut instance, 300.0, 96.0, 0, false, false);
    let base = slab_kernel::frame::inst_frame(&mut instance, 0.0);
    let base_scene_len = base.scene.len();
    let base_rects = base
        .ops
        .iter()
        .filter(|op| matches!(op, slab_kernel::flatten::FrameOp::Rect(_)))
        .count();
    let event = |etype, x, y, dx| slab_kernel::dispatch::Event {
        etype,
        x,
        y,
        dx,
        dy: 0.0,
        button: 0,
        clicks: 1,
        key: String::new(),
        text: String::new(),
        mods: 0,
    };
    slab_kernel::frame::inst_dispatch(
        &mut instance,
        &event(slab_kernel::dispatch::E_POINTER_DOWN, 20.0, 20.0, 0.0),
    );
    let moved = slab_kernel::frame::inst_dispatch(
        &mut instance,
        &event(slab_kernel::dispatch::E_POINTER_MOVE, 28.0, 20.0, 8.0),
    );
    assert!(moved.repaint, "ghost movement must request a frame");
    let ghost = slab_kernel::frame::inst_frame(&mut instance, 1.0);
    assert_eq!(ghost.scene.len(), base_scene_len);
    assert!(ghost.ops.iter().any(|op| matches!(
        op,
        slab_kernel::flatten::FrameOp::GroupPush(group)
            if group.opacity == 0.72
    )));
    assert!(matches!(
        ghost.ops.last(),
        Some(slab_kernel::flatten::FrameOp::GroupPop)
    ));
    assert!(
        ghost
            .ops
            .iter()
            .filter(|op| matches!(op, slab_kernel::flatten::FrameOp::Rect(_)))
            .count()
            > base_rects
    );
    let released = slab_kernel::frame::inst_dispatch(
        &mut instance,
        &event(slab_kernel::dispatch::E_POINTER_UP, 28.0, 20.0, 0.0),
    );
    assert!(released.repaint, "ghost removal must request a frame");
    let after_release = slab_kernel::frame::inst_frame(&mut instance, 2.0);
    assert!(!after_release.ops.iter().any(|op| matches!(
        op,
        slab_kernel::flatten::FrameOp::GroupPush(group)
            if group.opacity == 0.72
    )));
}

#[test]
fn quarter_turn_drag_ghost_preserves_placement_to_painted_offset() {
    let source = r#"
canvas w=240 h=160 {
  box key=source at=80,40 w=80 h=40 rotate=90 drag=started drag-update=moving drag-end=ended drag-ghost bg=#314158
}
"#;
    let (slir, diagnostics) = compile(
        source,
        &Options {
            embed_assets: false,
            ..Default::default()
        },
    );
    assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
    let slir = slir.expect("rotated drag ghost document");
    let source_node = slir
        .nodes
        .key
        .iter()
        .position(|&key| {
            let key = slir.str_at(key);
            key == "source" || key.ends_with("/source")
        })
        .expect("source node") as u32;
    let bytes = slab_slir::write(&slir);
    let (mut instance, _) = slab_slir::instance(&bytes).expect("drag ghost instance");
    slab_kernel::frame::inst_set_env(&mut instance, 240.0, 160.0, 0, false, false);
    let base = slab_kernel::frame::inst_frame(&mut instance, 0.0);
    let painted = base
        .scene
        .iter()
        .find(|node| node.node == source_node)
        .expect("source scene");
    let down_x = painted.x + painted.w / 2.0;
    let down_y = painted.y + painted.h / 2.0;
    let base_rect = base
        .ops
        .iter()
        .find_map(|op| match op {
            slab_kernel::flatten::FrameOp::Rect(rect) if rect.node == source_node => Some(rect),
            _ => None,
        })
        .expect("source rectangle");
    let event = |etype, x, y, dx| slab_kernel::dispatch::Event {
        etype,
        x,
        y,
        dx,
        dy: 0.0,
        button: 0,
        clicks: 1,
        key: String::new(),
        text: String::new(),
        mods: 0,
    };
    slab_kernel::frame::inst_dispatch(
        &mut instance,
        &event(slab_kernel::dispatch::E_POINTER_DOWN, down_x, down_y, 0.0),
    );
    slab_kernel::frame::inst_dispatch(
        &mut instance,
        &event(
            slab_kernel::dispatch::E_POINTER_MOVE,
            down_x + 20.0,
            down_y,
            20.0,
        ),
    );
    let ghost = slab_kernel::frame::inst_frame(&mut instance, 1.0);
    let ghost_rect = ghost
        .ops
        .iter()
        .rev()
        .find_map(|op| match op {
            slab_kernel::flatten::FrameOp::Rect(rect) if rect.node == source_node => Some(rect),
            _ => None,
        })
        .expect("ghost source rectangle");
    assert!((ghost_rect.x - base_rect.x - 20.0).abs() < 1e-9);
    assert!((ghost_rect.y - base_rect.y).abs() < 1e-9);
}
