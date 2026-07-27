use crate::{frame, list, motion, scene, slir, style};

/// Appends an attribute value to the fixture document and returns its index.
pub fn aval(d: &mut slir::Doc, tag: u32, lo: u32, hi: u32, num: f64) -> u32 {
    let ix = u32::try_from(d.aval_tag.len()).expect("fixture attribute count fits in u32");
    d.aval_tag.push(tag);
    d.aval_lo.push(lo);
    d.aval_hi.push(hi);
    d.aval_num.push(num);
    ix
}

/// Builds the list fixture shared by the list behavior tests.
pub fn list_doc() -> slir::Doc {
    let mut d = slir::doc_new();
    d.ok = true;
    d.strs.extend([
        "".into(),         // 0
        "items".into(),    // 1
        "label".into(),    // 2
        "shown".into(),    // 3
        "A".into(),        // 4
        "B".into(),        // 5
        "list".into(),     // 6
        "row".into(),      // 7 (template-relative)
        "text".into(),     // 8 (template-relative)
        "selected".into(), // 9
    ]);

    let list_default = aval(&mut d, slir::T_LIST_DEFAULT, 0, 2, 0.0);
    let empty = aval(&mut d, slir::T_STR, 0, 0, 0.0);
    let no = aval(&mut d, slir::T_NUM, 0, 0, 0.0);
    let a = aval(&mut d, slir::T_STR, 4, 0, 0.0);
    let yes = aval(&mut d, slir::T_NUM, 0, 0, 1.0);
    let b = aval(&mut d, slir::T_STR, 5, 0, 0.0);
    let each = aval(&mut d, slir::T_NUM, 0, 0, 0.0);
    let prop_label = aval(&mut d, slir::T_PROP_REF, 0, 0, 0.0);
    let inert = aval(&mut d, slir::T_NUM, 0, 0, f64::from(slir::F_INERT));
    let scroll = aval(&mut d, slir::T_NUM, 0, 0, f64::from(slir::F_SCROLL));
    let clip = aval(&mut d, slir::T_NUM, 0, 0, f64::from(slir::F_CLIP));
    let base_opacity = aval(&mut d, slir::T_NUM, 0, 0, 1.0);
    let selected_opacity = aval(&mut d, slir::T_NUM, 0, 0, 0.0);

    d.parm_name.push(1);
    d.parm_type.push(6);
    d.parm_default.push(list_default);
    d.parm_enum_off.push(0);
    d.parm_enum_len.push(0);
    d.parm_site_off.push(0);
    d.parm_site_len.push(0);

    d.list_param.push(0);
    d.list_field_off.push(0);
    d.list_field_len.push(2);
    d.list_field_name.push(2);
    d.list_field_type.push(0);
    d.list_field_default.push(empty);
    d.list_field_enum_off.push(0);
    d.list_field_enum_len.push(0);
    d.list_field_name.push(3);
    d.list_field_type.push(4);
    d.list_field_default.push(no);
    d.list_field_enum_off.push(0);
    d.list_field_enum_len.push(0);
    d.list_item_field_off.extend([0, 2]);
    d.list_item_field_len.extend([2, 2]);
    d.list_item_value_field.extend([0, 1, 0, 1]);
    d.list_item_value_val.extend([a, yes, b, no]);

    // Each item expands to a detached row containing text.
    d.node_kind
        .extend([slir::K_EACH, slir::K_ROW, slir::K_TEXT]);
    d.node_flags
        .extend([0, slir::F_DETACHED | slir::F_FOCUSABLE, 0]);
    d.node_parent.extend([slir::NONE, 0, 1]);
    d.node_first.extend([1, 2, slir::NONE]);
    d.node_next.extend([slir::NONE, slir::NONE, slir::NONE]);
    d.node_key.extend([6, 7, 8]);
    d.node_id.extend([0, 0, 0]);
    d.node_line.extend([1, 2, 3]);
    d.attr_index.push(0);
    d.attr_id.push(slir::A_EACH);
    d.attr_val.push(each);
    d.attr_index.push(1);
    d.attr_id.push(slir::A_OPACITY);
    d.attr_val.push(base_opacity);
    d.attr_index.push(2);
    d.attr_id.push(slir::A_CONTENT);
    d.attr_val.push(prop_label);
    d.attr_index.push(3);

    // `when shown` makes item 0 inert; `when selected` applies scrolling per synthetic id.
    d.cond_kind
        .extend([slir::C_PROP, slir::C_STATE, slir::C_WCMP]);
    d.cond_sym.extend([1, 9, 0]);
    d.cond_op.extend([0, 0, 2]);
    d.cond_neg.extend([0, 0, 0]);
    d.cond_num.extend([0.0, 0.0, 50.0]);
    d.patch_node.extend([1, 1, 1]);
    d.patch_cond.extend([0, 1, 2]);
    d.patch_attr_off.extend([0, 1, 3]);
    d.patch_attr_len.extend([1, 2, 1]);
    d.patch_child_off.extend([0, 0, 0]);
    d.patch_child_len.extend([0, 0, 0]);
    d.wattr_id
        .extend([slir::A_FLAGS, slir::A_FLAGS, slir::A_OPACITY, slir::A_FLAGS]);
    d.wattr_val.extend([inert, scroll, selected_opacity, clip]);
    d.trans_node.push(1);
    d.trans_easing.push(0);
    d.trans_dur.push(100.0);
    d.trans_delay.push(0.0);
    d
}

/// Initializes list fixture style state for a solve.
pub fn fresh(d: &slir::Doc) -> style::St {
    let mut st = style::st_new();
    style::init_params(d, &mut st);
    style::begin_solve(d, &mut st);
    st
}

/// Collects the detached roots produced by the fixture's each node.
pub fn roots(d: &slir::Doc, st: &mut style::St, out: &mut Vec<u32>) {
    style::children(d, st, 0, out);
}

/// Verifies defaults, resizing, and atomic rejection of invalid field values.
pub fn test_list_defaults_extend_truncate_and_atomic_rejection() {
    let d = list_doc();
    let mut st = fresh(&d);
    assert_eq!(list::length(&d, &st.lists, 0), 2, "default list length");
    assert_eq!(
        list::get(&d, &st.lists, 0, 0, 0).s,
        "A",
        "normalized first default"
    );
    assert_eq!(
        list::get(&d, &st.lists, 0, 0, 1).num,
        1.0,
        "typed bool default"
    );

    let before = list::get(&d, &st.lists, 0, 0, 0).s.clone();
    let bad = list::Val {
        kind: 1,
        num: 9.0,
        s: String::new(),
        rgba: 0,
        sym: String::new(),
    };
    assert_eq!(
        list::set_field(&d, &mut st.lists, 0, 0, "label", &bad),
        -1,
        "type mismatch rejected"
    );
    assert_eq!(
        list::get(&d, &st.lists, 0, 0, 0).s,
        before,
        "rejection has no partial write"
    );
    assert_eq!(
        list::set_len(&d, &mut st.lists, 0, 1),
        1,
        "truncate changes"
    );
    assert_eq!(list::set_len(&d, &mut st.lists, 0, 2), 1, "extend changes");
    assert_eq!(
        list::get(&d, &st.lists, 0, 1, 0).s,
        "",
        "extend seeds schema default"
    );
}

/// Verifies that patches, state, focus, and content remain isolated per item.
pub fn test_list_prop_patch_state_isolation_focus_and_content() {
    let d = list_doc();
    let mut st = fresh(&d);
    let mut roots = Vec::new();
    self::roots(&d, &mut st, &mut roots);
    assert_eq!(roots.len(), 2, "one detached root per item");
    let (first, second) = (roots[0], roots[1]);
    assert_ne!(
        style::eff_flags(&d, &st, first) & slir::F_INERT,
        0,
        "truthy PROP patch on first item"
    );
    assert_eq!(
        style::eff_flags(&d, &st, second) & slir::F_INERT,
        0,
        "false PROP isolated on second item"
    );
    style::set_node_state(&d, &mut st, second, "selected", true);
    assert_eq!(
        style::eff_flags(&d, &st, first) & slir::F_SCROLL,
        0,
        "state absent on sibling item"
    );
    assert_ne!(
        style::eff_flags(&d, &st, second) & slir::F_SCROLL,
        0,
        "state follows synthetic id"
    );
    style::set_patch_flags(&d, &mut st, first, 100.0, 100.0);
    style::set_patch_flags(&d, &mut st, second, 20.0, 100.0);
    assert_ne!(
        style::eff_flags(&d, &st, first) & slir::F_CLIP,
        0,
        "width patch on first item"
    );
    assert_eq!(
        style::eff_flags(&d, &st, second) & slir::F_CLIP,
        0,
        "different item constraint cannot inherit width patch"
    );

    let mut kids = Vec::new();
    style::children(&d, &mut st, first, &mut kids);
    assert_eq!(
        style::content_str(&d, &st, kids[0]),
        "A",
        "PROP_REF content resolves item value"
    );
    let mut sc = scene::scene_new();
    sc.node.extend([first, second]);
    sc.flags.extend([
        style::eff_flags(&d, &st, first),
        style::eff_flags(&d, &st, second),
    ]);
    let mut focus = Vec::new();
    scene::focusables(&sc, &mut focus);
    assert_eq!(
        focus,
        [second],
        "focus order follows item order and inertness"
    );
}

/// Verifies that transition clocks and overlays are independent per list item.
pub fn test_list_transition_clock_and_overlay_are_per_item() {
    let d = list_doc();
    let mut st = fresh(&d);
    let mut roots = Vec::new();
    self::roots(&d, &mut st, &mut roots);
    let mut ms = motion::mst_new();
    motion::apply(&d, &mut st, &mut ms, 0.0);
    style::set_node_state(&d, &mut st, roots[1], "selected", true);
    style::begin_solve(&d, &mut st);
    motion::apply(&d, &mut st, &mut ms, 0.0);
    style::begin_solve(&d, &mut st);
    motion::apply(&d, &mut st, &mut ms, 50.0);
    assert_eq!(
        style::attr_num(&d, &st, roots[0], slir::A_OPACITY, -1.0),
        1.0,
        "unselected item has no transition overlay"
    );
    assert_eq!(
        style::attr_num(&d, &st, roots[1], slir::A_OPACITY, -1.0),
        0.5,
        "selected item owns midpoint transition overlay"
    );
}

/// Verifies keyed identity across reorder and pruning after truncation.
pub fn test_list_keyed_reorder_identity_prune_and_key_addressing() {
    let d = list_doc();
    let mut st = fresh(&d);
    assert_eq!(list::set_key(&d, &mut st.lists, 0, 0, "a"), 1, "first key");
    assert_eq!(list::set_key(&d, &mut st.lists, 0, 1, "b"), 1, "second key");
    let mut first = Vec::new();
    roots(&d, &mut st, &mut first);
    let (a, b) = (first[0], first[1]);
    assert_eq!(
        scene::node_by_key(&d, &st.lists, "list~a/row"),
        a,
        "synthetic key addressing"
    );
    style::set_node_state(&d, &mut st, a, "selected", true);
    style::scroll_set(&mut st, a, 12.0);
    style::field_set(&mut st, a, "draft");
    style::field_scroll_set(&mut st, a, 4.0);
    assert_eq!(
        list::set_key(&d, &mut st.lists, 0, 0, "b"),
        1,
        "transient duplicate accepted"
    );
    assert_eq!(
        list::set_key(&d, &mut st.lists, 0, 1, "a"),
        1,
        "complete direct swap"
    );
    style::begin_solve(&d, &mut st);
    let mut reordered = Vec::new();
    roots(&d, &mut st, &mut reordered);
    assert_eq!(reordered, [b, a], "ids stable across direct keyed reorder");
    assert_ne!(
        style::eff_flags(&d, &st, reordered[1]) & slir::F_SCROLL,
        0,
        "per-item state follows key after reorder"
    );
    assert_eq!(
        list::set_len(&d, &mut st.lists, 0, 1),
        1,
        "truncate keyed list"
    );
    style::begin_solve(&d, &mut st);
    assert!(
        !style::node_state_on(&d, &st, a, "selected"),
        "truncate drops node state"
    );
    assert_eq!(
        style::scroll_get(&st, a),
        0.0,
        "truncate drops scroll state"
    );
    assert_eq!(
        style::field_scroll_x(&st, a),
        0.0,
        "truncate drops field scroll state"
    );
    assert_eq!(
        list::base(&st.lists, &d, a),
        slir::NONE,
        "truncated synthetic id pruned"
    );
    assert_eq!(
        list::base(&st.lists, &d, b),
        1,
        "remaining keyed id retained"
    );
    assert_eq!(
        scene::node_by_key(&d, &st.lists, "list~a/row"),
        slir::NONE,
        "vanished key not addressable"
    );
}

/// Builds a self-recursive list schema with a nested each template.
pub fn recursive_list_doc() -> slir::Doc {
    let mut d = slir::doc_new();
    d.ok = true;
    d.strs.extend([
        "".into(),
        "trees".into(),
        "label".into(),
        "children".into(),
        "tree".into(),
        "row".into(),
        "row/children".into(),
        "text".into(),
    ]);
    let empty = aval(&mut d, slir::T_STR, 0, 0, 0.0);
    let empty_list = aval(&mut d, slir::T_LIST_DEFAULT, 0, 0, 0.0);
    let root_each = aval(&mut d, slir::T_NUM, 0, 0, 0.0);
    let child_each = aval(&mut d, slir::T_PROP_REF, 1, 0, 0.0);
    let label = aval(&mut d, slir::T_PROP_REF, 0, 0, 0.0);
    d.parm_name.push(1);
    d.parm_type.push(6);
    d.parm_default.push(empty_list);
    d.parm_enum_off.push(0);
    d.parm_enum_len.push(0);
    d.parm_site_off.push(0);
    d.parm_site_len.push(0);
    d.list_param.extend([slir::NONE, 0]);
    d.list_field_off.extend([0, 2]);
    d.list_field_len.extend([2, 2]);
    for _ in 0..2 {
        d.list_field_name.extend([2, 3]);
        d.list_field_type.extend([0, 6]);
        d.list_field_default.extend([empty, empty_list]);
        d.list_field_enum_off.extend([0, 0]);
        d.list_field_enum_len.extend([0, 0]);
        d.list_field_sub.extend([0, 1]);
    }
    d.node_kind
        .extend([slir::K_EACH, slir::K_ROW, slir::K_EACH, slir::K_TEXT]);
    d.node_flags.extend([
        0,
        slir::F_DETACHED | slir::F_FOCUSABLE,
        0,
        slir::F_FOCUSABLE,
    ]);
    d.node_parent.extend([slir::NONE, 0, 1, 2]);
    d.node_first.extend([1, 2, 3, slir::NONE]);
    d.node_next
        .extend([slir::NONE, slir::NONE, slir::NONE, slir::NONE]);
    d.node_key.extend([4, 5, 6, 7]);
    d.node_id.extend([0, 0, 0, 0]);
    d.node_line.extend([1, 2, 3, 4]);
    d.attr_index.extend([0, 1, 1, 2, 3]);
    d.attr_id
        .extend([slir::A_EACH, slir::A_EACH, slir::A_CONTENT]);
    d.attr_val.extend([root_each, child_each, label]);
    d
}

/// Verifies recursive path writes, nested materialization, innermost item
/// identity, full key composition, and recursive truncation pruning.
pub fn test_recursive_list_paths_materialization_and_pruning() {
    let d = recursive_list_doc();
    let mut st = fresh(&d);
    assert_eq!(list::set_len_path(&d, &mut st.lists, 0, "", 1), 1);
    assert_eq!(list::set_key_path(&d, &mut st.lists, 0, "", 0, "root"), 1);
    assert_eq!(list::set_len_path(&d, &mut st.lists, 0, "0.children", 2), 1);
    let child = list::Val {
        kind: 0,
        num: 0.0,
        s: "child".into(),
        rgba: 0,
        sym: String::new(),
    };
    assert_eq!(
        list::set_field_path(&d, &mut st.lists, 0, "0.children", 0, "label", &child),
        1
    );
    assert_eq!(
        list::set_key_path(&d, &mut st.lists, 0, "0.children", 0, "child"),
        1
    );
    assert_eq!(
        list::set_len_path(&d, &mut st.lists, 0, "0.children.0.children", 1),
        1
    );
    assert_eq!(
        list::length(
            &d,
            &st.lists,
            list::resolve_path(&d, &st.lists, 0, "0.children.0.children")
        ),
        1,
        "self-recursive schema reaches depth three"
    );

    style::begin_solve(&d, &mut st);
    let mut outer = Vec::new();
    style::children(&d, &mut st, 0, &mut outer);
    let mut row_children = Vec::new();
    style::children(&d, &mut st, outer[0], &mut row_children);
    let inner_each = row_children[0];
    let mut inner = Vec::new();
    style::children(&d, &mut st, inner_each, &mut inner);
    assert_eq!(inner.len(), 2);
    assert_eq!(list::item_key(&st.lists, &d, inner[0]), "child");
    assert_eq!(
        scene::key_of(&d, &st.lists, inner[0]),
        "tree~root/row/children~child/text"
    );
    let removed = inner[0];
    assert_eq!(list::set_len_path(&d, &mut st.lists, 0, "0.children", 0), 1);
    style::begin_solve(&d, &mut st);
    assert_eq!(list::base(&st.lists, &d, removed), slir::NONE);
    assert_ne!(list::base(&st.lists, &d, inner_each), slir::NONE);

    // A generated recursive setter rewrites a parent item completely before
    // moving to the next one. The first key write therefore creates a
    // transient duplicate, and its child-length shrink must not prune the
    // temporarily absent parent's descendant identities.
    let mut reordered = fresh(&d);
    assert_eq!(list::set_len_path(&d, &mut reordered.lists, 0, "", 2), 1);
    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "", 0, "a"),
        1
    );
    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "", 1, "b"),
        1
    );
    assert_eq!(
        list::set_len_path(&d, &mut reordered.lists, 0, "0.children", 2),
        1
    );
    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "0.children", 0, "a0"),
        1
    );
    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "0.children", 1, "a1"),
        1
    );
    assert_eq!(
        list::set_len_path(&d, &mut reordered.lists, 0, "1.children", 1),
        1
    );
    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "1.children", 0, "b0"),
        1
    );
    style::begin_solve(&d, &mut reordered);
    let mut before = Vec::new();
    style::children(&d, &mut reordered, 0, &mut before);
    let a_root = before[0];
    let mut a_row = Vec::new();
    style::children(&d, &mut reordered, a_root, &mut a_row);
    let a_each = a_row[0];
    let mut a_children = Vec::new();
    style::children(&d, &mut reordered, a_each, &mut a_children);
    let retained = a_children[0];
    style::scroll_set(&mut reordered, retained, 17.0);

    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "", 0, "b"),
        1
    );
    assert_eq!(
        list::set_len_path(&d, &mut reordered.lists, 0, "0.children", 1),
        1,
        "child shrink occurs while parent key a is transiently absent"
    );
    assert_ne!(
        list::base(&reordered.lists, &d, retained),
        slir::NONE,
        "an individual recursive write cannot prune global identity"
    );
    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "0.children", 0, "b0"),
        1
    );
    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "", 1, "a"),
        1
    );
    assert_eq!(
        list::set_len_path(&d, &mut reordered.lists, 0, "1.children", 2),
        1
    );
    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "1.children", 0, "a0"),
        1
    );
    assert_eq!(
        list::set_key_path(&d, &mut reordered.lists, 0, "1.children", 1, "a1"),
        1
    );
    style::begin_solve(&d, &mut reordered);
    let mut after = Vec::new();
    style::children(&d, &mut reordered, 0, &mut after);
    assert_eq!(
        after[1], a_root,
        "parent identity follows its reordered key"
    );
    let mut a_row_after = Vec::new();
    style::children(&d, &mut reordered, after[1], &mut a_row_after);
    assert_eq!(
        a_row_after[0], a_each,
        "nested each identity survives reorder"
    );
    let mut a_children_after = Vec::new();
    style::children(&d, &mut reordered, a_row_after[0], &mut a_children_after);
    assert_eq!(
        a_children_after[0], retained,
        "descendant identity survives reorder plus child-length changes"
    );
    assert_eq!(
        style::scroll_get(&reordered, retained),
        17.0,
        "descendant persistent state follows the retained identity"
    );
    assert_eq!(list::set_len(&d, &mut reordered.lists, 0, 1), 1);
    style::begin_solve(&d, &mut reordered);
    assert_eq!(
        list::base(&reordered.lists, &d, retained),
        slir::NONE,
        "the next complete solve boundary prunes a deleted parent subtree"
    );
    assert_eq!(
        style::scroll_get(&reordered, retained),
        0.0,
        "deleted descendant state is pruned with its identity"
    );
}

/// Builds a 10k-capable top-level virtual each fixture.
pub fn virtual_list_doc() -> slir::Doc {
    let mut d = slir::doc_new();
    d.ok = true;
    d.strs.extend([
        "".into(),
        "items".into(),
        "label".into(),
        "scroll".into(),
        "virtual".into(),
        "row".into(),
        "text".into(),
    ]);
    let empty = aval(&mut d, slir::T_STR, 0, 0, 0.0);
    let empty_list = aval(&mut d, slir::T_LIST_DEFAULT, 0, 0, 0.0);
    let each = aval(&mut d, slir::T_NUM, 0, 0, 0.0);
    let extent = aval(&mut d, slir::T_NUM, 0, 0, 20.0);
    let overscan = aval(&mut d, slir::T_NUM, 0, 0, 4.0);
    let row_h = aval(&mut d, slir::T_SIZE_FIXED, 0, 0, 20.0);
    let prop_label = aval(&mut d, slir::T_PROP_REF, 0, 0, 0.0);
    d.parm_name.push(1);
    d.parm_type.push(6);
    d.parm_default.push(empty_list);
    d.parm_enum_off.push(0);
    d.parm_enum_len.push(0);
    d.parm_site_off.push(0);
    d.parm_site_len.push(0);
    d.list_param.extend([slir::NONE, 0]);
    d.list_field_off.extend([0, 1]);
    d.list_field_len.extend([1, 1]);
    d.list_field_name.extend([2, 2]);
    d.list_field_type.extend([0, 0]);
    d.list_field_default.extend([empty, empty]);
    d.list_field_enum_off.extend([0, 0]);
    d.list_field_enum_len.extend([0, 0]);
    d.list_field_sub.extend([0, 0]);
    d.node_kind
        .extend([slir::K_COL, slir::K_EACH, slir::K_ROW, slir::K_TEXT]);
    d.node_flags.extend([
        slir::F_SCROLL,
        slir::F_VIRTUAL,
        slir::F_DETACHED | slir::F_FOCUSABLE,
        slir::F_DETACHED,
    ]);
    d.node_parent.extend([slir::NONE, 0, 1, 2]);
    d.node_first.extend([1, 2, 3, slir::NONE]);
    d.node_next
        .extend([slir::NONE, slir::NONE, slir::NONE, slir::NONE]);
    d.node_key.extend([3, 4, 5, 6]);
    d.node_id.extend([0, 0, 0, 0]);
    d.node_line.extend([1, 2, 3, 4]);
    d.attr_index.extend([0, 0, 3, 4, 5]);
    d.attr_id.extend([
        slir::A_EACH,
        slir::A_ITEM_EXTENT,
        slir::A_OVERSCAN,
        slir::A_H,
        slir::A_CONTENT,
    ]);
    d.attr_val
        .extend([each, extent, overscan, row_h, prop_label]);
    d
}

#[cfg(test)]
fn virtual_motion_doc() -> slir::Doc {
    let mut d = virtual_list_doc();
    let selected = u32::try_from(d.strs.len()).expect("fixture strings fit in u32");
    d.strs.push("selected".into());
    let selected_opacity = aval(&mut d, slir::T_NUM, 0, 0, 0.0);
    d.cond_kind.push(slir::C_STATE);
    d.cond_sym.push(selected);
    d.cond_op.push(0);
    d.cond_neg.push(0);
    d.cond_num.push(0.0);
    d.patch_node.push(2);
    d.patch_cond.push(0);
    d.patch_attr_off.push(0);
    d.patch_attr_len.push(1);
    d.patch_child_off.push(0);
    d.patch_child_len.push(0);
    d.wattr_id.push(slir::A_OPACITY);
    d.wattr_val.push(selected_opacity);
    d.trans_node.push(2);
    d.trans_easing.push(0);
    d.trans_dur.push(100.0);
    d.trans_delay.push(0.0);
    let anim_low = aval(&mut d, slir::T_NUM, 0, 0, 0.25);
    let anim_high = aval(&mut d, slir::T_NUM, 0, 0, 0.75);
    d.anim_name.push(0);
    d.anim_stop_off.push(0);
    d.anim_stop_len.push(2);
    d.anim_stop_pos.extend([0.0, 1.0]);
    d.anim_stop_attr_off.extend([0, 1]);
    d.anim_stop_attr_len.extend([1, 1]);
    d.aattr_id.extend([slir::A_OPACITY, slir::A_OPACITY]);
    d.aattr_val.extend([anim_low, anim_high]);
    d.bind_node.push(3);
    d.bind_anim.push(0);
    d.bind_dur.push(1_000.0);
    d.bind_mode.push(0);
    d.bind_easing.push(0);
    d.bind_delay.push(0.0);
    d
}

/// Adds parent padding and a fixed preceding sibling before the virtual each.
pub fn virtual_list_with_origin_doc() -> slir::Doc {
    let mut d = virtual_list_doc();
    let header_key = u32::try_from(d.strs.len()).expect("fixture strings fit in u32");
    d.strs.push("header".into());
    let pad_off = u32::try_from(d.f64s.len()).expect("fixture tuple pool fits in u32");
    d.f64s.extend([10.0, 0.0, 15.0, 0.0]);
    let pad = aval(&mut d, slir::T_TUPLE, pad_off, 4, 0.0);
    let header_h = aval(&mut d, slir::T_SIZE_FIXED, 0, 0, 30.0);

    d.attr_id.insert(0, slir::A_PAD);
    d.attr_val.insert(0, pad);
    for boundary in d.attr_index.iter_mut().skip(1) {
        *boundary = boundary.wrapping_add(1);
    }

    let header = u32::try_from(d.node_kind.len()).expect("fixture nodes fit in u32");
    d.node_kind.push(slir::K_RECT);
    d.node_flags.push(0);
    d.node_parent.push(0);
    d.node_first.push(slir::NONE);
    d.node_next.push(1);
    d.node_key.push(header_key);
    d.node_id.push(0);
    d.node_line.push(5);
    d.node_first[0] = header;
    d.attr_id.push(slir::A_H);
    d.attr_val.push(header_h);
    d.attr_index
        .push(i32::try_from(d.attr_id.len()).expect("fixture attributes fit in i32"));
    d
}

/// Verifies bounded windows, exact logical extent, retained de-windowed
/// identity, and focus traversal over materialized nodes only.
pub fn test_virtual_list_window_extent_identity_and_focus() {
    let d = virtual_list_doc();
    let mut st = fresh(&d);
    assert_eq!(list::set_len(&d, &mut st.lists, 0, 10_000), 1);
    let mut first = Vec::new();
    style::children(&d, &mut st, 1, &mut first);
    assert_eq!(list::current_window(&st.lists, 1), (0, 8));
    assert_eq!(first.len(), 8);
    let retained = first[0];
    assert!(list::set_virtual_viewport(
        &d,
        &mut st.lists,
        1,
        100.0,
        1000.0,
        0.0,
    ));
    style::scroll_set(&mut st, 0, 1000.0);
    let mut moved = Vec::new();
    style::children(&d, &mut st, 1, &mut moved);
    assert_eq!(list::base(&st.lists, &d, retained), 2);
    let (extent, len, start, end) =
        list::virtual_metrics(&d, &st.lists, 1).expect("virtual metrics");
    assert_eq!((extent * f64::from(len), start, end), (200_000.0, 46, 59));
    let mut sc = scene::scene_new();
    sc.node.extend(moved.iter().copied());
    sc.flags.resize(moved.len(), slir::F_FOCUSABLE);
    let mut focusable = Vec::new();
    scene::focusables(&sc, &mut focusable);
    assert_eq!(
        focusable, moved,
        "unmaterialized identities are not tabbable"
    );
}

#[cfg(test)]
#[test]
fn virtual_lookup_work_is_independent_of_logical_length() {
    fn solve_work(len: i32) -> usize {
        let d = virtual_list_doc();
        let mut st = fresh(&d);
        assert_eq!(list::set_len(&d, &mut st.lists, 0, len), 1);
        let mut label = list::Val {
            kind: 0,
            num: 0.0,
            s: String::new(),
            rgba: 0,
            sym: String::new(),
        };
        for item in 0..len {
            let key = format!("key-{item}");
            label.s.clone_from(&key);
            assert_eq!(list::set_key(&d, &mut st.lists, 0, item, &key), 1);
            assert_eq!(
                list::set_field(&d, &mut st.lists, 0, item, "label", &label),
                1
            );
        }
        assert!(list::set_virtual_viewport(
            &d,
            &mut st.lists,
            1,
            100.0,
            1_000.0,
            0.0,
        ));
        style::scroll_set(&mut st, 0, 1_000.0);
        list::reset_lookup_work();
        style::begin_solve(&d, &mut st);
        let mut lay = crate::layout::lay_new();
        crate::layout::solve(&d, &mut st, &mut lay, 120.0, 100.0, true);
        list::lookup_work()
    }

    let short = solve_work(100);
    let long = solve_work(10_000);
    assert!(
        long <= short.saturating_add(8),
        "indexed materialization work grew with logical length: {short} -> {long}"
    );
}

/// Builds a depth-three recursive default to exercise nested `ListDefault`
/// seeding independently from host path writes.
pub fn recursive_default_doc() -> slir::Doc {
    let mut d = recursive_list_doc();
    d.strs
        .extend(["root".into(), "child".into(), "leaf".into()]);
    let root_label = aval(&mut d, slir::T_STR, 8, 0, 0.0);
    let child_label = aval(&mut d, slir::T_STR, 9, 0, 0.0);
    let leaf_label = aval(&mut d, slir::T_STR, 10, 0, 0.0);
    let no_children = aval(&mut d, slir::T_LIST_DEFAULT, 0, 0, 0.0);
    let leaf_run = aval(&mut d, slir::T_LIST_DEFAULT, 2, 1, 0.0);
    let child_run = aval(&mut d, slir::T_LIST_DEFAULT, 1, 1, 0.0);
    let root_run = aval(&mut d, slir::T_LIST_DEFAULT, 0, 1, 0.0);
    d.list_item_field_off.extend([0, 2, 4]);
    d.list_item_field_len.extend([2, 2, 2]);
    d.list_item_value_field.extend([0, 1, 0, 1, 0, 1]);
    d.list_item_value_val.extend([
        root_label,
        child_run,
        child_label,
        leaf_run,
        leaf_label,
        no_children,
    ]);
    d.parm_default[0] = root_run;
    d
}

/// Verifies recursive document defaults seed every descendant list, and that
/// truncation removes descendant values before a later extension gets a clean
/// schema-default child list.
pub fn test_recursive_list_defaults_and_reextension_are_clean() {
    let d = recursive_default_doc();
    let mut st = fresh(&d);
    let root = list::root_id(&d, &st.lists, 0);
    assert_eq!(list::length(&d, &st.lists, root), 1);
    assert_eq!(list::get(&d, &st.lists, root, 0, 0).s, "root");
    let children = list::resolve_path(&d, &st.lists, 0, "0.children");
    assert_ne!(children, slir::NONE);
    assert_eq!(list::length(&d, &st.lists, children), 1);
    assert_eq!(list::get(&d, &st.lists, children, 0, 0).s, "child");
    let grandchildren = list::resolve_path(&d, &st.lists, 0, "0.children.0.children");
    assert_ne!(grandchildren, slir::NONE);
    assert_eq!(list::length(&d, &st.lists, grandchildren), 1);
    assert_eq!(list::get(&d, &st.lists, grandchildren, 0, 0).s, "leaf");
    assert_eq!(
        list::length(
            &d,
            &st.lists,
            list::resolve_path(&d, &st.lists, 0, "0.children.0.children.0.children")
        ),
        0
    );

    assert_eq!(list::set_len(&d, &mut st.lists, 0, 0), 1);
    assert_eq!(
        list::resolve_path(&d, &st.lists, 0, "0.children"),
        slir::NONE
    );
    assert_eq!(list::set_len(&d, &mut st.lists, 0, 1), 1);
    let fresh_children = list::resolve_path(&d, &st.lists, 0, "0.children");
    assert_ne!(fresh_children, slir::NONE);
    assert_eq!(list::length(&d, &st.lists, fresh_children), 0);
    assert_eq!(list::get(&d, &st.lists, root, 0, 0).s, "");
}

/// Exercises the full first-frame settle and scroll/reveal path for 10k
/// virtual items, including exact logical extent and bounded output size.
pub fn test_virtual_list_frame_settle_reveal_and_op_bound() {
    let mut inst = frame::inst_shell();
    inst.doc = virtual_list_with_origin_doc();
    frame::inst_init(&mut inst);
    frame::inst_set_env(&mut inst, 120.0, 100.0, 0, false, false);
    assert!(frame::inst_set_list_len(&mut inst, 0, "", 10_000));

    let first = frame::inst_frame(&mut inst, 0.0);
    let settled = frame::inst_frame(&mut inst, 0.0);
    assert!(
        settled.scene.len() < first.scene.len(),
        "the retained origin trims items hidden behind padding and the preceding sibling"
    );
    let scroll = settled
        .scene
        .iter()
        .find(|node| node.node == 0)
        .expect("scroll scene node");
    assert_eq!(scroll.content_main, 200_055.0);
    assert!(settled.scene.len() < 200);
    assert!(settled.ops.len() < 200);

    assert_eq!(list::virtual_origin(&inst.st.lists, 1), 40.0);
    assert!(frame::inst_set_scroll(&mut inst, "scroll", 0, 20.0));
    frame::inst_frame(&mut inst, 8.0);
    assert_eq!(
        frame::inst_each_window(&inst, "virtual").0,
        0,
        "the window stays at the first item until the each enters the viewport"
    );
    assert!(frame::inst_reveal_item(&mut inst, "virtual", 0, 0));
    frame::inst_frame(&mut inst, 12.0);
    assert_eq!(
        frame::inst_get_scroll(&inst, "scroll", 0),
        40.0,
        "start alignment includes padding and the preceding sibling"
    );

    assert!(frame::inst_set_scroll(&mut inst, "scroll", 0, 1_000.0));
    let moved = frame::inst_frame(&mut inst, 16.0);
    let (start, end) = frame::inst_each_window(&inst, "virtual");
    assert!(start > 0);
    assert!(end - start < 32);
    assert!(moved.scene.len() < 200);
    assert!(moved.ops.len() < 200);

    assert!(frame::inst_reveal_item(&mut inst, "virtual", 9_999, 2));
    let revealed = frame::inst_frame(&mut inst, 32.0);
    let (start, end) = frame::inst_each_window(&inst, "virtual");
    assert!(start < 10_000);
    assert_eq!(end, 10_000);
    assert_eq!(frame::inst_get_scroll(&inst, "scroll", 0), 199_940.0);
    assert!(revealed.scene.len() < 200);
    assert!(revealed.ops.len() < 200);
}

/// Verifies retained frame updates skip clean instances without clearing or
/// reallocating the caller-owned frame.
pub fn test_retained_frame_update_reuses_output_and_reports_clean_frames() {
    let mut inst = frame::inst_shell();
    inst.doc = virtual_list_with_origin_doc();
    frame::inst_init(&mut inst);
    frame::inst_set_env(&mut inst, 120.0, 100.0, 0, false, false);
    assert!(frame::inst_set_list_len(&mut inst, 0, "", 10_000));

    let mut output = crate::flatten::frame_new();
    assert!(frame::inst_frame_update(&mut inst, 0.0, &mut output));
    assert!(frame::inst_frame_update(&mut inst, 0.0, &mut output));
    let scene_len = output.scene.len();
    let ops_len = output.ops.len();
    let strings_len = output.strings.len();
    let scene_ptr = output.scene.as_ptr();
    let ops_ptr = output.ops.as_ptr();

    assert!(!frame::inst_frame_update(&mut inst, 0.0, &mut output));
    assert_eq!(output.scene.len(), scene_len);
    assert_eq!(output.ops.len(), ops_len);
    assert_eq!(output.strings.len(), strings_len);
    assert_eq!(output.scene.as_ptr(), scene_ptr);
    assert_eq!(output.ops.as_ptr(), ops_ptr);

    assert!(frame::inst_set_scroll(&mut inst, "scroll", 0, 1_000.0));
    assert!(frame::inst_frame_update(&mut inst, 0.0, &mut output));
    assert!(frame::inst_each_window(&inst, "virtual").0 > 0);
}

#[cfg(test)]
#[test]
fn virtual_motion_work_tracks_current_window_and_retains_clock_state() {
    let mut inst = frame::inst_shell();
    inst.doc = virtual_motion_doc();
    frame::inst_init(&mut inst);
    frame::inst_set_env(&mut inst, 120.0, 100.0, 0, false, false);
    assert!(frame::inst_set_list_len(&mut inst, 0, "", 20_000));
    assert!(frame::inst_set_list_key(&mut inst, 0, "", 0, "retained"));

    frame::inst_frame(&mut inst, 0.0);
    frame::inst_frame(&mut inst, 0.0);
    let retained = scene::node_by_key(&inst.doc, &inst.st.lists, "virtual~retained/row");
    assert_ne!(retained, slir::NONE);
    let current_count = list::materialized(&inst.st.lists).len();
    assert!(current_count > 0);
    assert_eq!(
        inst.st.lists.sy_id.len(),
        current_count,
        "a template animation must not materialize the logical list"
    );

    motion::reset_synthetic_work();
    frame::inst_frame(&mut inst, 1.0);
    let initial_work = motion::synthetic_work();
    assert!(initial_work > 0);

    let initial_clock =
        usize::try_from(motion::sp_find(&inst.ms, retained, 0)).expect("initial synthetic clock");
    assert_eq!(inst.ms.sp_flip[initial_clock], motion::NEVER);
    assert!(style::set_node_state(
        &inst.doc,
        &mut inst.st,
        retained,
        "selected",
        true
    ));
    inst.dirty = true;
    frame::inst_frame(&mut inst, 10.0);
    let flipped_clock =
        usize::try_from(motion::sp_find(&inst.ms, retained, 0)).expect("flipped synthetic clock");
    let retained_flip = inst.ms.sp_flip[flipped_clock];
    assert_eq!(retained_flip, 10.0);
    assert!(inst.ms.sp_last[flipped_clock]);

    for visit in 1..=64 {
        let off = f64::from(visit) * 4_000.0;
        assert!(frame::inst_set_scroll(&mut inst, "scroll", 0, off));
        let moved = frame::inst_frame(&mut inst, 20.0 + f64::from(visit));
        if visit == 1 {
            let (start, _) = frame::inst_each_window(&inst, "virtual");
            assert!(start > 0);
            let newly_materialized = moved
                .scene
                .iter()
                .map(|entry| entry.node)
                .find(|&node| {
                    list::base(&inst.st.lists, &inst.doc, node) == 2
                        && list::item_ix(&inst.st.lists, &inst.doc, node) >= start
                })
                .expect("new window row");
            let new_clock = usize::try_from(motion::sp_find(&inst.ms, newly_materialized, 0))
                .expect("the first scrolled frame must initialize its materialized clock");
            assert_eq!(inst.ms.sp_flip[new_clock], motion::NEVER);
        }
    }

    assert!(
        inst.st.lists.sy_id.len() > current_count.saturating_mul(32),
        "fixture did not accumulate enough retained identities"
    );
    assert_eq!(list::base(&inst.st.lists, &inst.doc, retained), 2);
    assert!(style::node_state_on(
        &inst.doc, &inst.st, retained, "selected"
    ));
    let dewindowed_clock = usize::try_from(motion::sp_find(&inst.ms, retained, 0))
        .expect("de-windowed synthetic clock");
    assert_eq!(inst.ms.sp_flip[dewindowed_clock], retained_flip);

    assert!(frame::inst_set_scroll(&mut inst, "scroll", 0, 0.0));
    let rematerialized = frame::inst_frame(&mut inst, 1_000.0);
    assert!(
        rematerialized
            .scene
            .iter()
            .any(|entry| entry.node == retained)
    );
    assert_eq!(list::materialized(&inst.st.lists).len(), current_count);
    assert!(style::node_state_on(
        &inst.doc, &inst.st, retained, "selected"
    ));
    let rematerialized_clock = usize::try_from(motion::sp_find(&inst.ms, retained, 0))
        .expect("rematerialized synthetic clock");
    assert_eq!(inst.ms.sp_flip[rematerialized_clock], retained_flip);
    assert!(inst.ms.sp_last[rematerialized_clock]);

    motion::reset_synthetic_work();
    frame::inst_frame(&mut inst, 1_001.0);
    let retained_work = motion::synthetic_work();
    assert!(
        retained_work <= initial_work,
        "synthetic motion work grew with retained history: {initial_work} -> {retained_work}"
    );

    assert!(frame::inst_set_list_len(&mut inst, 0, "", 0));
    frame::inst_frame(&mut inst, 1_002.0);
    assert_eq!(motion::sp_find(&inst.ms, retained, 0), -1);
    assert_eq!(list::base(&inst.st.lists, &inst.doc, retained), slir::NONE);
}
