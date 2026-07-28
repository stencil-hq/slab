use slab_compile::{Options, compile};
use slab_kernel::{dispatch, flatten::FrameOp, frame, list, scene};
use slab_slir::{Slir, attrs, aval, flags, kind};

fn compile_ok(source: &str) -> Slir {
    let (slir, diagnostics) = compile(
        source,
        &Options {
            embed_assets: false,
            ..Options::default()
        },
    );
    assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
    slir.expect("valid Lists v2 source")
}

fn list_field<'a>(slir: &'a Slir, row: usize, name: &str) -> &'a slab_slir::ListFieldE {
    let schema = &slir.lists[row];
    (schema.field_off..schema.field_off + schema.field_len)
        .map(|field| &slir.list_fields[field as usize])
        .find(|field| slir.str_at(field.name) == name)
        .expect("schema field")
}

fn each_axis<'a>(slir: &'a Slir, key_segment: &str) -> &'a str {
    let node = slir
        .nodes
        .kind
        .iter()
        .enumerate()
        .find(|(node, candidate)| {
            **candidate == kind::EACH
                && slir
                    .str_at(slir.nodes.key[*node])
                    .rsplit('/')
                    .next()
                    .is_some_and(|segment| segment == key_segment)
        })
        .map(|(node, _)| node)
        .expect("each node");
    let value = slir
        .node_attrs(node as u32)
        .iter()
        .find(|(attr, _)| *attr == attrs::AXIS)
        .map(|(_, value)| slir.avals[*value as usize])
        .expect("each axis");
    assert_eq!(value.tag, aval::ENUM_SYM);
    slir.str_at(value.lo())
}

fn item_value(slir: &Slir, item: u32, field: u32) -> u32 {
    let item = slir.list_items[item as usize];
    (item.field_off..item.field_off + item.field_len)
        .map(|value| slir.list_item_values[value as usize])
        .find(|value| value.field == field)
        .expect("normalized item field")
        .val
}

#[test]
fn recursive_schemas_are_two_pass_and_defaults_are_nested_runs() {
    let source = r#"
def Tree(label="", children=list(Tree)) export {
  col key=branch {
    text label key=label
    each children key=children
  }
}
params {
  trees list(Tree) = [
    Tree(label="root", children=[
      Tree(label="child", children=[
        Tree(label="leaf")
      ])
    ])
  ]
}
col { each param.trees key=trees }
"#;
    let slir = compile_ok(source);
    eprintln!("SLIR LISTS: {:#?}", slir.lists);
    eprintln!("SLIR LIST FIELDS: {:#?}", slir.list_fields);
    eprintln!("SLIR LIST ITEMS: {:#?}", slir.list_items);
    eprintln!("SLIR LIST ITEM VALUES: {:#?}", slir.list_item_values);

    assert_eq!(slir.lists.len(), 2, "canonical row plus host root row");
    for row in 0..2 {
        let children = list_field(&slir, row, "children");
        assert_eq!(children.ty, 6);
        assert_eq!(children.sub, 1, "self recursion targets canonical row zero");
    }

    let mut nested = slir.avals[slir.params[0].default as usize];
    for expected_len in [1_u32, 1, 1] {
        assert_eq!(nested.tag, aval::LIST_DEFAULT);
        assert_eq!(nested.hi(), expected_len);
        nested = slir.avals[item_value(&slir, nested.lo(), 1) as usize];
    }
    assert_eq!(nested.tag, aval::LIST_DEFAULT);
    assert_eq!(
        nested.hi(),
        0,
        "leaf children normalize to an empty list run"
    );

    let eaches: Vec<usize> = slir
        .nodes
        .kind
        .iter()
        .enumerate()
        .filter_map(|(node, node_kind)| (*node_kind == kind::EACH).then_some(node))
        .collect();
    assert_eq!(
        eaches.len(),
        2,
        "one root each and one symbolic recursive edge"
    );
    let nested_each = eaches[1];
    assert_eq!(slir.nodes.first_child[nested_each], slab_slir::NONE);
    let each_value = slir
        .node_attrs(nested_each as u32)
        .iter()
        .find(|(attr, _)| *attr == attrs::EACH)
        .map(|(_, value)| slir.avals[*value as usize])
        .expect("nested each target");
    assert_eq!(each_value.tag, aval::PROP_REF);
}

#[test]
fn mutually_recursive_schemas_allocate_before_field_resolution() {
    let source = r#"
def Branch(leaves=list(Leaf)) export {
  col { each leaves }
}
def Leaf(branches=list(Branch)) export {
  col { each branches }
}
params { roots list(Branch) = [] }
col { each param.roots }
"#;
    let slir = compile_ok(source);
    assert_eq!(slir.lists.len(), 3);
    assert_eq!(list_field(&slir, 0, "leaves").sub, 2);
    assert_eq!(list_field(&slir, 1, "branches").sub, 1);
    assert_eq!(list_field(&slir, 2, "leaves").sub, 2);
}

#[test]
fn nested_each_accepts_forwarded_list_props() {
    let source = r#"
def Chip(label="") export { text label }
def Forward(items) { each items }
def Row(chips=list(Chip)) export { Forward items=chips }
params { rows list(Row) = [Row(chips=[Chip(label="ok")])] }
col { each param.rows }
"#;
    let slir = compile_ok(source);
    assert_eq!(
        slir.nodes
            .kind
            .iter()
            .filter(|node| **node == kind::EACH)
            .count(),
        2
    );
}

#[test]
fn each_encodes_its_enclosing_linear_axis() {
    let source = r#"
def Chip(label="") export { text label }
def Row(chips=list(Chip)) export {
  col key=row {
    row key=chip-line { each chips key=chips }
  }
}
params {
  rows list(Row) = [Row(chips=[Chip(label="ok")])]
}
col { each param.rows key=rows }
"#;
    let slir = compile_ok(source);
    assert_eq!(each_axis(&slir, "rows"), "col");
    assert_eq!(each_axis(&slir, "chips"), "row");
}

#[test]
fn para_each_requires_and_preserves_exactly_one_styled_span() {
    let valid = r#"
def Run(text="", tone=#000000, size=14, weight=400, family="Inter", tracking=0) export {
  span text color=tone size=size weight=weight family=family tracking=tracking
}
params {
  runs list(Run) = [
    Run(text="alpha", tone=#A03030, size=12, weight=500, family="Inter", tracking=0.5),
    Run(text="beta", tone=#3040A0, size=16, weight=700, family="JetBrains Mono", tracking=1)
  ]
}
para w=180 { each param.runs key=runs }
"#;
    let slir = compile_ok(valid);
    let each = slir
        .nodes
        .kind
        .iter()
        .position(|node| *node == kind::EACH)
        .expect("paragraph each");
    assert_eq!(each_axis(&slir, "runs"), "row");
    let span = slir.nodes.first_child[each] as usize;
    assert_eq!(slir.nodes.kind[span], kind::SPAN);
    for attr in [
        attrs::CONTENT,
        attrs::COLOR,
        attrs::SIZE,
        attrs::WEIGHT,
        attrs::FAMILY,
        attrs::TRACKING,
    ] {
        let value = slir
            .node_attrs(span as u32)
            .iter()
            .find(|(candidate, _)| *candidate == attr)
            .map(|(_, value)| slir.avals[*value as usize])
            .unwrap_or_else(|| panic!("missing span attr {attr}"));
        assert_eq!(value.tag, aval::PROP_REF);
    }

    let invalid = r#"
def Two(text="") export {
  span text
  span "again"
}
def NotSpan(text="") export { text text }
params {
  two list(Two) = []
  wrong list(NotSpan) = []
}
col {
  para { each param.two }
  para { each param.wrong }
}
"#;
    let (slir, diagnostics) = compile(invalid, &Options::default());
    assert!(slir.is_none());
    assert_eq!(
        diagnostics
            .0
            .iter()
            .filter(|diagnostic| diagnostic.code == "each-span")
            .count(),
        2
    );
}

#[test]
fn virtual_each_emits_default_window_config_and_rejects_bad_contexts() {
    let valid = r#"
def Row(label="") export { row h=20 { text label } }
params { rows list(Row) = [] }
col #viewport w=120 h=100 scroll {
  each param.rows key=rows virtual item-extent=20
}
"#;
    let slir = compile_ok(valid);
    let each = slir
        .nodes
        .kind
        .iter()
        .position(|node| *node == kind::EACH)
        .expect("virtual each");
    assert_ne!(slir.nodes.flags[each] & flags::VIRTUAL, 0);
    let node_attrs = slir.node_attrs(each as u32);
    let extent = node_attrs
        .iter()
        .find(|(attr, _)| *attr == attrs::ITEM_EXTENT)
        .map(|(_, value)| slir.avals[*value as usize].as_f64());
    let overscan = node_attrs
        .iter()
        .find(|(attr, _)| *attr == attrs::OVERSCAN)
        .map(|(_, value)| slir.avals[*value as usize].as_f64());
    assert_eq!(extent, Some(20.0));
    assert_eq!(overscan, Some(4.0));

    let invalid = r#"
def Row(label="") export { row h=20 { text label } }
params { rows list(Row) = [] }
col { each param.rows virtual item-extent=20 }
col scroll { each param.rows virtual }
"#;
    let (slir, diagnostics) = compile(invalid, &Options::default());
    assert!(slir.is_none());
    let codes: Vec<_> = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect();
    assert!(codes.contains(&"virtual-ctx"));
    assert!(codes.contains(&"virtual-extent"));
}

#[test]
fn virtual_each_accepts_nested_list_properties() {
    let source = r#"
def Row(label="") export { row h=20 { text label } }
def Section(rows=list(Row)) export {
  col #rows-scroll h=60 scroll {
    each rows #rows-each virtual item-extent=20 overscan=2
  }
}
params {
  sections list(Section) = [
    Section(rows=[Row(label="one"), Row(label="two"), Row(label="three")])
  ]
}
col { each param.sections }
"#;
    let slir = compile_ok(source);
    let bytes = slab_slir::write(&slir);
    let (mut instance, _) = slab_slir::instance(&bytes).expect("nested virtual list instance");
    frame::inst_set_env(&mut instance, 120.0, 60.0, 0, false, false);
    assert!(frame::inst_set_list_len(&mut instance, 0, "0.rows", 1_000));

    frame::inst_frame(&mut instance, 0.0);
    let settled = frame::inst_frame(&mut instance, 0.0);
    assert!(
        settled.scene.len() < 100,
        "nested virtual list emitted {} scene nodes",
        settled.scene.len()
    );
    let each = settled
        .scene
        .iter()
        .find_map(|node| {
            list::virtual_config(&instance.doc, &instance.st.lists, node.node)
                .is_some()
                .then_some(node.node)
        })
        .expect("materialized nested virtual each");
    let each_key = scene::key_of(&instance.doc, &instance.st.lists, each);
    let (start, end) = frame::inst_each_window(&instance, &each_key);
    assert_eq!(start, 0);
    assert!(end <= 10);

    let (_, _, scroll_parent) =
        list::virtual_config(&instance.doc, &instance.st.lists, each).expect("virtual config");
    let scroll_key = scene::key_of(&instance.doc, &instance.st.lists, scroll_parent);
    assert!(frame::inst_set_scroll(
        &mut instance,
        &scroll_key,
        0,
        10_000.0
    ));
    frame::inst_frame(&mut instance, 0.0);
    let scrolled = frame::inst_frame(&mut instance, 0.0);
    let (start, end) = frame::inst_each_window(&instance, &each_key);
    assert!(start > 0);
    assert!(end - start <= 10);
    assert!(scrolled.scene.len() < 100);
}

#[test]
fn nested_signal_reports_innermost_item_and_full_key() {
    let source = r#"
def Chip(label="") export {
  row key=chip w=160 h=24 focusable act=choose { text label }
}
def Row(label="", chips=list(Chip)) export {
  col key=row w=160 {
    text label key=label w=160 h=20
    each chips key=chips
  }
}
params {
  rows list(Row) = [Row(label="row", chips=[Chip(label="chip")])]
}
col #surface w=160 { each param.rows key=rows }
"#;
    let slir = compile_ok(source);
    let bytes = slab_slir::write(&slir);
    let (mut instance, _) = slab_slir::instance(&bytes).expect("nested list instance");
    frame::inst_set_env(&mut instance, 160.0, 68.0, 0, false, false);
    assert!(frame::inst_set_list_key(&mut instance, 0, "", 0, "outer"));
    assert!(frame::inst_set_list_key(
        &mut instance,
        0,
        "0.chips",
        0,
        "inner"
    ));
    let _ = frame::inst_frame(&mut instance, 0.0);
    let event = |etype| dispatch::Event {
        etype,
        x: 40.0,
        y: 30.0,
        dx: 0.0,
        dy: 0.0,
        button: 0,
        clicks: 1,
        key: String::new(),
        text: String::new(),
        mods: 0,
    };
    let down = frame::inst_dispatch(&mut instance, &event(dispatch::E_POINTER_DOWN));
    assert!(down.sig_name.is_empty());
    let focused = instance.ds.fs.focus;
    assert!(slab_kernel::style::attached(
        &instance.doc,
        &instance.st,
        focused,
    ));
    assert_eq!(dispatch::sig_of(&instance.doc, &instance.st, focused, 0), 0);
    let up = frame::inst_dispatch(&mut instance, &event(dispatch::E_POINTER_UP));
    assert_eq!(up.sig_name.len(), 1);
    assert_eq!(
        slab_kernel::slir::str_at(&instance.doc, up.sig_name[0]),
        "choose"
    );
    assert_eq!(up.sig_item, ["inner"]);
    assert_eq!(
        up.sig_meta[0].key,
        "#surface/rows~outer/row/chips~inner/chip"
    );
}

#[test]
fn list_item_conditions_only_materialize_matching_children() {
    let source = r#"
def Row(first=false, second=false, third=false) export {
  col w=100 {
    when first { text "A" }
    when second { text "B" }
    when third { text "C" }
  }
}
params {
  rows list(Row) = [
    Row(first=true),
    Row(second=true)
  ]
}
col w=100 { each param.rows key=rows }
"#;
    let slir = compile_ok(source);
    let bytes = slab_slir::write(&slir);
    let (mut instance, _) = slab_slir::instance(&bytes).expect("conditional list instance");
    frame::inst_set_env(&mut instance, 100.0, 100.0, 0, false, false);

    let frame = frame::inst_frame(&mut instance, 0.0);
    let text: Vec<_> = frame
        .ops
        .iter()
        .filter_map(|op| {
            let FrameOp::Text(text) = op else {
                return None;
            };
            Some(frame.strings[usize::try_from(text.str_ref).expect("text string ref")].as_str())
        })
        .collect();

    assert_eq!(text, ["A", "B"]);
}

#[test]
fn paragraph_runs_resolve_all_item_text_style_properties() {
    let source = r##"
def Run(text="", tone=#112233, size=12, weight=500, family="Inter", tracking=0) export {
  span text color=tone size=size weight=weight family=family tracking=tracking
}
params {
  runs list(Run) = [
    Run(text="one", tone=#112233, size=12, weight=400, family="", tracking=0),
    Run(text="two", tone=#AABBCC, size=18, weight=700, family="Custom Mono", tracking=1.5)
  ]
}
para w=200 h=40 { each param.runs key=runs }
"##;
    let slir = compile_ok(source);
    for (family, weight) in [("Inter", 500), ("Custom Mono", 700)] {
        assert!(
            slir.fonts
                .iter()
                .any(|font| { slir.str_at(font.family) == family && font.weight == weight }),
            "missing dynamic-run fallback {family}/{weight}"
        );
    }
    let custom_700 = slir
        .fonts
        .iter()
        .position(|font| slir.str_at(font.family) == "Custom Mono" && font.weight == 700)
        .map(|font| i32::try_from(font).expect("font index exceeds i32"))
        .expect("compiled Custom Mono/700 fallback");
    assert_ne!(custom_700, 0);
    let bytes = slab_slir::write(&slir);
    let (mut instance, _) = slab_slir::instance(&bytes).expect("paragraph run instance");
    frame::inst_set_env(&mut instance, 200.0, 40.0, 0, false, false);
    let fallback_frame = frame::inst_frame(&mut instance, 0.0);
    let fallback_font = fallback_frame
        .ops
        .iter()
        .find_map(|op| {
            let FrameOp::Text(text) = op else {
                return None;
            };
            let content =
                &fallback_frame.strings[usize::try_from(text.str_ref).expect("text string ref")];
            (content.as_str() == "two").then_some(text.font)
        })
        .expect("second paragraph run fallback");
    assert_eq!(fallback_font, custom_700);
    let custom_font = frame::inst_font_register(
        &mut instance,
        "Custom Mono",
        700,
        1000,
        800,
        -200,
        0,
        500,
        &[],
        &[],
        &[],
    );
    frame::inst_set_env(&mut instance, 200.0, 40.0, 0, false, false);
    let frame = frame::inst_frame(&mut instance, 0.0);
    let mut runs = frame.ops.iter().filter_map(|op| {
        let FrameOp::Text(text) = op else {
            return None;
        };
        let content = &frame.strings[usize::try_from(text.str_ref).expect("text string ref")];
        Some((content.as_str(), text))
    });
    let (one_content, one) = runs.next().expect("first paragraph run");
    let (two_content, two) = runs.next().expect("second paragraph run");
    assert_eq!((one_content, two_content), ("one", "two"));
    assert_eq!((one.size, one.weight, one.tracking), (12.0, 400, 0.0));
    assert_eq!((two.size, two.weight, two.tracking), (18.0, 700, 1.5));
    assert_ne!(one.color, two.color);
    assert_eq!(two.font, custom_font);
    assert!(runs.next().is_none());
}
