//! Layout fixtures and focused host-facing frame API contracts.

use crate::{frame, layout, slir, style};

/// Appends an attribute value and its reference to the fixture document.
pub fn add_attr(d: &mut slir::Doc, attr: u32, tag: u32, num: f64, h: u32) {
    let ix = u32::try_from(d.aval_tag.len()).expect("fixture attribute count fits in u32");
    d.aval_tag.push(tag);
    d.aval_lo.push(h);
    d.aval_hi.push(0);
    d.aval_num.push(num);
    d.attr_id.push(attr);
    d.attr_val.push(ix);
}

/// Builds a single-hole document with optional row orientation and size constraints.
pub fn hole_doc(row: bool, min_w: f64, max_h: f64) -> slir::Doc {
    let mut d = slir::doc_new();
    d.ok = true;
    d.strs.extend([String::new(), "row".into()]);
    d.node_kind.push(slir::K_HOLE);
    d.node_flags.push(0);
    d.node_parent.push(slir::NONE);
    d.node_first.push(slir::NONE);
    d.node_next.push(slir::NONE);
    d.node_key.push(0);
    d.node_id.push(0);
    d.node_line.push(1);
    d.attr_index.push(0);

    add_attr(&mut d, slir::A_W, slir::T_SIZE_HUG, 0.0, 0);
    add_attr(&mut d, slir::A_H, slir::T_SIZE_HUG, 0.0, 0);
    if min_w > 0.0 {
        add_attr(&mut d, slir::A_MIN_W, slir::T_NUM, min_w, 0);
    }
    if max_h < style::INF {
        add_attr(&mut d, slir::A_MAX_H, slir::T_NUM, max_h, 0);
    }
    if row {
        add_attr(&mut d, slir::A_AXIS, slir::T_ENUM_SYM, 0.0, 1);
    }
    d.attr_index
        .push(i32::try_from(d.attr_id.len()).expect("fixture attribute count fits in i32"));
    d.hole_name.push(0);
    d.hole_node.push(0);
    d
}

/// Builds an initialized frame instance around a single-hole fixture.
pub fn hole_inst(row: bool, min_w: f64, max_h: f64) -> frame::Instance {
    let mut instance = frame::inst_shell();
    instance.ok = true;
    instance.doc = hole_doc(row, min_w, max_h);
    instance.has_env = true;
    instance.dirty = false;
    style::init_params(&instance.doc, &mut instance.st);
    instance
}

/// Solves a fixture instance in a fixed 500-by-500 environment.
pub fn solve(instance: &mut frame::Instance) -> i32 {
    style::begin_solve(&instance.doc, &mut instance.st);
    layout::solve(
        &instance.doc,
        &mut instance.st,
        &mut instance.lay,
        500.0,
        500.0,
        true,
    )
}

/// Verifies that an unreported hug hole stays zero through repeated solves.
pub fn test_hug_hole_unreported_is_zero_across_solves() {
    let mut instance = hole_inst(false, 0.0, style::INF);
    assert_eq!(instance.st.hole_w.len(), 1, "one stored width per hole");
    assert_eq!(instance.st.hole_h.len(), 1, "one stored height per hole");
    assert_eq!(
        (instance.st.hole_w[0], instance.st.hole_h[0]),
        (0.0, 0.0),
        "hole sizes seed to zero"
    );
    let first = usize::try_from(solve(&mut instance)).expect("fixture root index is valid");
    assert_eq!(
        (instance.lay.p_w[first], instance.lay.p_h[first]),
        (0.0, 0.0),
        "unreported hug hole is zero"
    );
    let second = usize::try_from(solve(&mut instance)).expect("fixture root index is valid");
    assert_eq!(
        (instance.lay.p_w[second], instance.lay.p_h[second]),
        (0.0, 0.0),
        "zero persists across solves"
    );
}

/// Verifies that reported dimensions map correctly in both orientations.
pub fn test_hug_hole_reported_dimensions_in_both_orientations() {
    let mut col = hole_inst(false, 0.0, style::INF);
    frame::inst_set_hole_size(&mut col, 0, 37.0, 19.0);
    let first = usize::try_from(solve(&mut col)).expect("fixture root index is valid");
    assert_eq!(
        (col.lay.p_w[first], col.lay.p_h[first]),
        (37.0, 19.0),
        "col hole uses reported axes"
    );
    let second = usize::try_from(solve(&mut col)).expect("fixture root index is valid");
    assert_eq!(
        (col.lay.p_w[second], col.lay.p_h[second]),
        (37.0, 19.0),
        "reported size persists"
    );

    let mut row = hole_inst(true, 0.0, style::INF);
    frame::inst_set_hole_size(&mut row, 0, 37.0, 19.0);
    let root = usize::try_from(solve(&mut row)).expect("fixture root index is valid");
    assert_eq!(
        (row.lay.p_w[root], row.lay.p_h[root]),
        (37.0, 19.0),
        "row hole maps main and cross correctly"
    );
}

/// Verifies that minimum and maximum constraints clamp reported dimensions.
pub fn test_hug_hole_report_is_clamped_by_min_and_max() {
    let mut instance = hole_inst(false, 50.0, 11.0);
    frame::inst_set_hole_size(&mut instance, 0, 37.0, 19.0);
    let root = usize::try_from(solve(&mut instance)).expect("fixture root index is valid");
    assert_eq!(instance.lay.p_w[root], 50.0, "min-w floors reported width");
    assert_eq!(
        instance.lay.p_h[root], 11.0,
        "max-h ceilings reported height"
    );
}

/// Verifies that authored zero maxima clamp both physical axes to zero.
pub fn test_zero_maximum_is_a_real_clamp() {
    assert_eq!(
        layout::clamp(33.0, 33.0, 100.0, 0.0, 0.0),
        0.0,
        "authored zero maximum overrides a parent minimum"
    );
    let mut height = hole_inst(false, 0.0, 0.0);
    frame::inst_set_hole_size(&mut height, 0, 37.0, 19.0);
    let root = usize::try_from(solve(&mut height)).expect("fixture root index is valid");
    assert_eq!(height.lay.p_h[root], 0.0, "max-h=0 clamps height");

    let mut width = hole_inst(false, 0.0, 0.0);
    let maximum = width
        .doc
        .attr_id
        .iter_mut()
        .find(|attr| **attr == slir::A_MAX_H)
        .expect("fixture has authored maximum");
    *maximum = slir::A_MAX_W;
    style::init_params(&width.doc, &mut width.st);
    frame::inst_set_hole_size(&mut width, 0, 37.0, 19.0);
    let root = usize::try_from(solve(&mut width)).expect("fixture root index is valid");
    assert_eq!(width.lay.p_w[root], 0.0, "max-w=0 clamps width");
}

/// Verifies that reports affect only hug axes belonging to hole nodes.
pub fn test_hole_report_does_not_override_non_hug_or_non_hole() {
    let mut fixed = hole_inst(false, 0.0, style::INF);
    fixed.doc.aval_tag[0] = slir::T_SIZE_FIXED;
    fixed.doc.aval_num[0] = 23.0;
    frame::inst_set_hole_size(&mut fixed, 0, 37.0, 19.0);
    let root = usize::try_from(solve(&mut fixed)).expect("fixture root index is valid");
    assert_eq!(
        fixed.lay.p_w[root], 23.0,
        "fixed axis ignores reported width"
    );
    assert_eq!(
        fixed.lay.p_h[root], 19.0,
        "hug axis still uses reported height"
    );

    let mut rect = hole_inst(false, 0.0, style::INF);
    rect.doc.node_kind[0] = slir::K_RECT;
    frame::inst_set_hole_size(&mut rect, 0, 37.0, 19.0);
    let root = usize::try_from(solve(&mut rect)).expect("fixture root index is valid");
    assert_eq!(
        (rect.lay.p_w[root], rect.lay.p_h[root]),
        (0.0, 0.0),
        "non-hole ignores reports"
    );
}

/// Verifies that invalid and unchanged hole-size reports are no-ops.
pub fn test_hole_size_invalid_and_equal_reports_are_noops() {
    let mut instance = hole_inst(false, 0.0, style::INF);
    frame::inst_set_hole_size(&mut instance, 9, 8.0, 7.0);
    assert!(!instance.dirty, "invalid index does not dirty");
    assert_eq!(
        (instance.st.hole_w[0], instance.st.hole_h[0]),
        (0.0, 0.0),
        "invalid index does not write"
    );

    frame::inst_set_hole_size(&mut instance, 0, 8.0, 7.0);
    assert!(instance.dirty, "changed report dirties");
    instance.dirty = false;
    frame::inst_set_hole_size(&mut instance, 0, 8.0, 7.0);
    assert!(!instance.dirty, "equal re-report does not dirty");
    assert_eq!(
        (instance.st.hole_w[0], instance.st.hole_h[0]),
        (8.0, 7.0),
        "equal report preserves storage"
    );
}

#[cfg(test)]
mod wave0_api {
    use super::*;
    use crate::{flatten, scene};

    fn instance(doc: slir::Doc) -> frame::Instance {
        let mut instance = frame::inst_shell();
        instance.doc = doc;
        frame::inst_init(&mut instance);
        instance.dirty = false;
        instance
    }

    fn keyed_doc(
        kinds: &[u32],
        flags: &[u32],
        parents: &[u32],
        first: &[u32],
        next: &[u32],
        keys: &[&str],
    ) -> slir::Doc {
        let count = kinds.len();
        assert_eq!(flags.len(), count);
        assert_eq!(parents.len(), count);
        assert_eq!(first.len(), count);
        assert_eq!(next.len(), count);
        assert_eq!(keys.len(), count);

        let mut doc = slir::doc_new();
        doc.ok = true;
        doc.strs.push(String::new());
        for key in keys {
            let key_ref = u32::try_from(doc.strs.len()).expect("fixture string count fits u32");
            doc.strs.push((*key).to_owned());
            doc.node_key.push(key_ref);
        }
        doc.node_kind.extend_from_slice(kinds);
        doc.node_flags.extend_from_slice(flags);
        doc.node_parent.extend_from_slice(parents);
        doc.node_first.extend_from_slice(first);
        doc.node_next.extend_from_slice(next);
        doc.node_id.resize(count, 0);
        doc.node_line.resize(count, 1);
        doc.attr_index.resize(count + 1, 0);
        doc
    }

    fn scene_node(
        node: u32,
        parent_ix: i32,
        kind: u32,
        y: f64,
        h: f64,
        flags: u32,
        content_main: f64,
    ) -> flatten::SceneNode {
        flatten::SceneNode {
            node,
            parent_ix,
            kind,
            x: 0.0,
            y,
            w: 100.0,
            h,
            radius: 0.0,
            rot_deg: 0.0,
            rot_cx: 0.0,
            rot_cy: 0.0,
            flags,
            content_main,
            scroll_off: 0.0,
            scroll_cross: 0.0,
            content_cross: 0.0,
            is_row: false,
            src_line: 1,
            authored_order: node,
            role: 0,
            label: 0,
            desc: 0,
            checked: 0,
            expanded: 0,
            selected: 0,
            active_descendant: 0,
            controls: 0,
            value_now: None,
            value_min: None,
            value_max: None,
            value_text: 0,
            modal: 0,
            live: 0,
            live_atomic: 0,
            level: None,
            pos_in_set: None,
            set_size: None,
            disabled: false,
            focused: false,
            editable: false,
        }
    }

    #[test]
    fn runtime_image_registry_keeps_indices_and_generations_mean_changes() {
        let mut doc = slir::doc_new();
        doc.ok = true;
        doc.strs.push("embedded".into());
        doc.img_src.push(0);
        doc.img_w.push(1);
        doc.img_h.push(1);
        doc.img_format.push(0);
        doc.img_data.push(vec![137, 80, 78, 71]);
        let mut instance = instance(doc);

        assert_eq!(
            frame::inst_img_info(&instance, 0),
            Some((1, 1, 0, 0)),
            "compiled images occupy the front of the unified table"
        );
        let rgba = [0, 1, 2, 3, 4, 5, 6, 7];
        let image = frame::inst_img_register(&mut instance, "avatar", 2, 1, 1, &rgba);
        assert_eq!(image, 1, "runtime indices follow compiled images");
        assert!(instance.dirty, "new registration dirties");
        assert_eq!(frame::inst_img_info(&instance, image), Some((2, 1, 1, 1)));
        assert_eq!(frame::inst_img_bytes(&instance, image), &rgba);

        instance.dirty = false;
        assert_eq!(
            frame::inst_img_register(&mut instance, "avatar", 2, 1, 1, &rgba),
            image
        );
        assert!(!instance.dirty, "equal registration is a no-op");
        assert_eq!(
            frame::inst_img_info(&instance, image),
            Some((2, 1, 1, 1)),
            "equal bytes do not advance generation"
        );

        let changed = [7, 6, 5, 4, 3, 2, 1, 0];
        assert_eq!(
            frame::inst_img_register(&mut instance, "avatar", 2, 1, 1, &changed),
            image
        );
        assert!(instance.dirty, "changed payload dirties");
        assert_eq!(
            frame::inst_img_info(&instance, image),
            Some((2, 1, 1, 2)),
            "real replacement advances generation once"
        );

        instance.dirty = false;
        assert!(frame::inst_img_unregister(&mut instance, "avatar"));
        assert!(instance.dirty, "unregistering an active image dirties");
        assert_eq!(frame::inst_img_info(&instance, image), None);
        assert!(frame::inst_img_bytes(&instance, image).is_empty());
        instance.dirty = false;
        assert!(!frame::inst_img_unregister(&mut instance, "avatar"));
        assert!(!instance.dirty, "repeated unregister is a no-op");

        assert_eq!(
            frame::inst_img_register(&mut instance, "avatar", 2, 1, 1, &changed),
            image,
            "re-registering a reserved name preserves its index"
        );
        assert_eq!(
            frame::inst_img_info(&instance, image),
            Some((2, 1, 1, 4)),
            "unregister and resurrection are both observable changes"
        );
    }

    #[test]
    fn runtime_image_registry_rejects_bad_rgba_atomically() {
        let mut doc = slir::doc_new();
        doc.ok = true;
        let mut instance = instance(doc);
        let short = [0; 15];

        assert_eq!(
            frame::inst_img_register(&mut instance, "bad", 2, 2, 1, &short),
            -1
        );
        assert!(!instance.dirty, "rejected registration does not dirty");

        let rgba = [1; 16];
        let image = frame::inst_img_register(&mut instance, "good", 2, 2, 1, &rgba);
        assert_eq!(image, 0, "rejection did not reserve an index");
        let before = frame::inst_img_info(&instance, image);
        let before_bytes = frame::inst_img_bytes(&instance, image).to_vec();

        instance.dirty = false;
        assert_eq!(
            frame::inst_img_register(&mut instance, "good", 2, 2, 1, &short),
            -1
        );
        assert_eq!(frame::inst_img_info(&instance, image), before);
        assert_eq!(
            frame::inst_img_bytes(&instance, image),
            before_bytes.as_slice()
        );
        assert!(!instance.dirty, "rejected replacement is atomic");
        assert_eq!(
            frame::inst_img_register(&mut instance, "other", 1, 1, 2, &[0; 4]),
            -1,
            "unknown image formats are rejected"
        );
        assert!(!instance.dirty);
    }

    #[test]
    fn runtime_image_registry_rejects_zero_corrupt_and_mismatched_png() {
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[10, 20, 30, 255])
                .unwrap();
        }
        let mut doc = slir::doc_new();
        doc.ok = true;
        let mut instance = instance(doc);

        assert_eq!(
            frame::inst_img_register(&mut instance, "zero-png", 0, 1, 0, &png_bytes),
            -1
        );
        assert_eq!(
            frame::inst_img_register(&mut instance, "zero-rgba", 1, 0, 1, &[]),
            -1
        );
        assert_eq!(
            frame::inst_img_register(&mut instance, "corrupt", 1, 1, 0, b"not a png"),
            -1
        );
        assert_eq!(
            frame::inst_img_register(&mut instance, "mismatch", 2, 1, 0, &png_bytes),
            -1
        );
        assert!(!instance.dirty, "invalid inputs are rejected atomically");

        assert_eq!(
            frame::inst_img_register(&mut instance, "valid", 1, 1, 0, &png_bytes),
            0
        );
        assert_eq!(frame::inst_img_bytes(&instance, 0), png_bytes.as_slice());
    }

    #[test]
    fn nested_list_paths_are_unknown_without_nested_schemas() {
        let mut doc = slir::doc_new();
        doc.ok = true;
        doc.strs.extend([String::new(), "items".into()]);
        doc.aval_tag.push(slir::T_LIST_DEFAULT);
        doc.aval_lo.push(0);
        doc.aval_hi.push(0);
        doc.aval_num.push(0.0);
        doc.parm_name.push(1);
        doc.parm_type.push(slir::PARAM_LIST);
        doc.parm_default.push(0);
        doc.parm_enum_off.push(0);
        doc.parm_enum_len.push(0);
        doc.parm_site_off.push(0);
        doc.parm_site_len.push(0);
        doc.list_param.push(0);
        doc.list_field_off.push(0);
        doc.list_field_len.push(0);
        let mut instance = instance(doc);

        assert!(frame::inst_set_list_len(&mut instance, 0, "", 1));
        assert_eq!(frame::inst_list_len(&instance, 0, ""), 1);
        instance.dirty = false;
        let value = frame::ParamValue {
            kind: slir::PARAM_TEXT,
            num: 0.0,
            s: "x".into(),
            rgba: 0,
            sym: String::new(),
        };
        assert_eq!(frame::inst_list_len(&instance, 0, "0.children"), -1);
        assert!(!frame::inst_set_list_len(&mut instance, 0, "0.children", 2));
        assert!(!frame::inst_set_list_field(
            &mut instance,
            0,
            "0.children",
            0,
            "label",
            &value
        ));
        assert!(!frame::inst_set_list_key(
            &mut instance,
            0,
            "0.children",
            0,
            "child"
        ));
        assert!(
            !instance.dirty,
            "unknown paths have no partial side effects"
        );
        assert_eq!(frame::inst_list_len(&instance, 0, ""), 1);
    }

    #[test]
    fn scroll_axes_validate_and_cross_offsets_clamp_after_solve() {
        let mut instance = hole_inst(false, 0.0, style::INF);
        instance.doc.strs[1] = "scroll".into();
        instance.doc.node_key[0] = 1;
        instance.doc.node_flags[0] = slir::F_SCROLL | slir::F_SCROLL_CROSS;
        // Rebuild the key index after patching the fixture doc post-load.
        crate::list::init(&instance.doc, &mut instance.st.lists);

        assert!(!frame::inst_set_scroll(&mut instance, "scroll", 2, 10.0));
        assert_eq!(frame::inst_get_scroll(&instance, "scroll", 2), 0.0);
        assert!(!instance.dirty, "unknown axis has no side effects");
        assert!(frame::inst_set_scroll(&mut instance, "scroll", 0, 150.0));
        assert!(frame::inst_set_scroll(&mut instance, "scroll", 1, 80.0));
        assert_eq!(frame::inst_get_scroll(&instance, "scroll", 0), 150.0);
        assert_eq!(frame::inst_get_scroll(&instance, "scroll", 1), 80.0);

        frame::inst_set_env(&mut instance, 100.0, 50.0, 0, false, false);
        let _ = frame::inst_frame(&mut instance, 0.0);
        assert_eq!(
            frame::inst_get_scroll(&instance, "scroll", 0),
            0.0,
            "childless main content clamps to zero"
        );
        assert_eq!(
            frame::inst_get_scroll(&instance, "scroll", 1),
            0.0,
            "childless cross content clamps to zero"
        );
        assert!(
            instance.dirty,
            "post-solve clamping schedules a settled frame"
        );
    }

    #[test]
    fn static_frame_clamps_both_axes_and_returns_settled_geometry() {
        let mut instance = hole_inst(false, 0.0, style::INF);
        instance.doc.strs[1] = "scroll".into();
        instance.doc.node_key[0] = 1;
        instance.doc.node_flags[0] = slir::F_SCROLL | slir::F_SCROLL_CROSS;
        // Rebuild the key index after patching the fixture doc post-load.
        crate::list::init(&instance.doc, &mut instance.st.lists);
        assert!(frame::inst_set_scroll(&mut instance, "scroll", 0, 150.0));
        assert!(frame::inst_set_scroll(&mut instance, "scroll", 1, 80.0));
        frame::inst_set_env(&mut instance, 100.0, 50.0, 0, false, false);

        let solved = frame::inst_frame_static(&mut instance);
        assert_eq!(frame::inst_get_scroll(&instance, "scroll", 0), 0.0);
        assert_eq!(frame::inst_get_scroll(&instance, "scroll", 1), 0.0);
        assert_eq!(solved.scene.len(), 1);
        assert_eq!(solved.scene[0].scroll_off, 0.0);
        assert_eq!(solved.scene[0].scroll_cross, 0.0);
    }

    #[test]
    fn reveal_minimally_scrolls_every_main_axis_ancestor() {
        let doc = keyed_doc(
            &[slir::K_COL, slir::K_COL, slir::K_RECT],
            &[slir::F_SCROLL, slir::F_SCROLL, 0],
            &[slir::NONE, 0, 1],
            &[1, 2, slir::NONE],
            &[slir::NONE, slir::NONE, slir::NONE],
            &["outer", "inner", "target"],
        );
        let mut instance = instance(doc);
        let mut solved = flatten::frame_new();
        solved.scene.push(scene_node(
            0,
            -1,
            slir::K_COL,
            0.0,
            100.0,
            slir::F_SCROLL,
            400.0,
        ));
        solved.scene.push(scene_node(
            1,
            0,
            slir::K_COL,
            150.0,
            100.0,
            slir::F_SCROLL,
            300.0,
        ));
        solved
            .scene
            .push(scene_node(2, 1, slir::K_RECT, 330.0, 20.0, 0, 0.0));
        scene::load(&mut instance.sc, &solved);
        instance.solved = true;

        assert!(frame::inst_reveal(&mut instance, "target", 10.0));
        assert_eq!(
            frame::inst_get_scroll(&instance, "inner", 0),
            110.0,
            "inner viewport moves only until the margin is visible"
        );
        assert_eq!(
            frame::inst_get_scroll(&instance, "outer", 0),
            150.0,
            "outer calculation observes the inner scroll displacement"
        );
        instance.dirty = false;
        assert!(!frame::inst_reveal(&mut instance, "missing", 10.0));
        assert!(!instance.dirty, "unknown reveal target has no side effects");
    }
    #[test]
    fn reveal_minimally_scrolls_both_axes_of_every_ancestor() {
        let both = slir::F_SCROLL | slir::F_SCROLL_CROSS;
        let doc = keyed_doc(
            &[slir::K_COL, slir::K_ROW, slir::K_RECT],
            &[both, both, 0],
            &[slir::NONE, 0, 1],
            &[1, 2, slir::NONE],
            &[slir::NONE, slir::NONE, slir::NONE],
            &["outer", "inner", "target"],
        );
        let mut instance = instance(doc);
        let mut solved = flatten::frame_new();

        let mut outer = scene_node(0, -1, slir::K_COL, 0.0, 100.0, both, 500.0);
        outer.content_cross = 500.0;
        solved.scene.push(outer);

        let mut inner = scene_node(1, 0, slir::K_ROW, 150.0, 100.0, both, 400.0);
        inner.x = 150.0;
        inner.is_row = true;
        inner.content_cross = 400.0;
        solved.scene.push(inner);

        let mut target = scene_node(2, 1, slir::K_RECT, 330.0, 20.0, 0, 0.0);
        target.x = 330.0;
        target.w = 20.0;
        solved.scene.push(target);
        scene::load(&mut instance.sc, &solved);
        instance.solved = true;

        assert!(frame::inst_reveal(&mut instance, "target", 0.0));
        assert_eq!(
            (
                frame::inst_get_scroll(&instance, "inner", 0),
                frame::inst_get_scroll(&instance, "inner", 1),
            ),
            (100.0, 100.0),
            "inner row moves its physical x and y axes minimally"
        );
        assert_eq!(
            (
                frame::inst_get_scroll(&instance, "outer", 0),
                frame::inst_get_scroll(&instance, "outer", 1),
            ),
            (150.0, 150.0),
            "outer col observes both inner-axis displacements"
        );
    }

    #[test]
    fn reveal_composes_rotations_and_inner_scroll_displacements() {
        let both = slir::F_SCROLL | slir::F_SCROLL_CROSS;
        let doc = keyed_doc(
            &[slir::K_COL, slir::K_STACK, slir::K_ROW, slir::K_RECT],
            &[both, 0, slir::F_SCROLL, 0],
            &[slir::NONE, 0, 1, 2],
            &[1, 2, 3, slir::NONE],
            &[slir::NONE, slir::NONE, slir::NONE, slir::NONE],
            &["outer", "rotator", "inner", "target"],
        );
        let mut instance = instance(doc);
        let mut solved = flatten::frame_new();

        let mut outer = scene_node(0, -1, slir::K_COL, 0.0, 100.0, both, 500.0);
        outer.content_cross = 500.0;
        solved.scene.push(outer);

        let mut rotator = scene_node(1, 0, slir::K_STACK, 0.0, 100.0, 0, 0.0);
        rotator.rot_deg = 90.0;
        rotator.rot_cx = 50.0;
        rotator.rot_cy = 50.0;
        solved.scene.push(rotator);

        let mut inner = scene_node(2, 1, slir::K_ROW, 0.0, 100.0, slir::F_SCROLL, 400.0);
        inner.x = 100.0;
        inner.is_row = true;
        solved.scene.push(inner);

        let mut target = scene_node(3, 2, slir::K_RECT, 10.0, 20.0, 0, 0.0);
        target.x = 250.0;
        target.w = 20.0;
        solved.scene.push(target);
        scene::load(&mut instance.sc, &solved);
        instance.solved = true;

        assert!(frame::inst_reveal(&mut instance, "target", 0.0));
        assert!(
            (frame::inst_get_scroll(&instance, "inner", 0) - 70.0).abs() < 1.0e-9,
            "inner row scrolls its raw x axis"
        );
        assert!(
            (frame::inst_get_scroll(&instance, "outer", 0) - 100.0).abs() < 1.0e-9,
            "the rotated inner displacement reaches the outer main y axis"
        );
        assert!(
            frame::inst_get_scroll(&instance, "outer", 1).abs() < 1.0e-9,
            "rotation must not misroute the displacement to outer cross x"
        );
    }

    #[test]
    fn reveal_parks_targets_below_pinned_sticky_headers() {
        let doc = keyed_doc(
            &[slir::K_COL, slir::K_RECT, slir::K_RECT],
            &[slir::F_SCROLL, slir::F_STICKY, 0],
            &[slir::NONE, 0, 0],
            &[1, slir::NONE, slir::NONE],
            &[slir::NONE, 2, slir::NONE],
            &["outer", "head", "target"],
        );
        let mut instance = instance(doc);
        assert!(frame::inst_set_scroll(&mut instance, "outer", 0, 50.0));
        let mut solved = flatten::frame_new();
        solved.scene.push(scene_node(
            0,
            -1,
            slir::K_COL,
            0.0,
            100.0,
            slir::F_SCROLL,
            400.0,
        ));
        solved
            .scene
            .push(scene_node(1, 0, slir::K_RECT, 0.0, 20.0, slir::F_STICKY, 0.0));
        solved
            .scene
            .push(scene_node(2, 0, slir::K_RECT, 10.0, 20.0, 0, 0.0));
        scene::load(&mut instance.sc, &solved);
        instance.solved = true;

        // The target's band starts under the pinned 20u header; a minimal
        // reveal scrolls it below the header, not to the raw viewport edge.
        assert!(frame::inst_reveal(&mut instance, "target", 0.0));
        assert_eq!(
            frame::inst_get_scroll(&instance, "outer", 0),
            40.0,
            "target parks below the pinned sticky header"
        );

        // Revealing the sticky header itself never scrolls against its own
        // pinned position.
        instance.dirty = false;
        assert!(frame::inst_reveal(&mut instance, "head", 0.0));
        assert_eq!(
            frame::inst_get_scroll(&instance, "outer", 0),
            40.0,
            "a pinned sticky target needs no scroll"
        );
    }

    #[test]
    fn controls_without_labels_take_scene_names_from_content() {
        let mut doc = keyed_doc(
            &[slir::K_COL, slir::K_ROW, slir::K_TEXT],
            &[0, slir::F_FOCUSABLE, 0],
            &[slir::NONE, 0, 1],
            &[1, 2, slir::NONE],
            &[slir::NONE, slir::NONE, slir::NONE],
            &["root", "save", "caption"],
        );
        let save_str = u32::try_from(doc.strs.len()).expect("fixture strings fit u32");
        doc.strs.push("Save".into());
        let edit_str = u32::try_from(doc.strs.len()).expect("fixture strings fit u32");
        doc.strs.push("edit".into());
        doc.aval_tag.push(slir::T_STR);
        doc.aval_lo.push(save_str);
        doc.aval_hi.push(0);
        doc.aval_num.push(0.0);
        doc.attr_id.push(slir::A_CONTENT);
        doc.attr_val.push(0);
        doc.attr_index[3] = 1;
        // The caption also carries an always-active field binder, so its
        // scene entry reports kernel editability.
        doc.sign_name.push(edit_str);
        doc.sign_node.push(2);
        doc.sign_trigger.push(1);
        let mut instance = instance(doc);
        frame::inst_set_env(&mut instance, 200.0, 100.0, 0, false, false);
        let frame_out = frame::inst_frame(&mut instance, 0.0);

        let row = frame_out
            .scene
            .iter()
            .find(|entry| entry.node == 1)
            .expect("focusable row is in the scene");
        assert_eq!(
            instance.st.scene_strs[usize::try_from(row.label).expect("label ref fits usize")],
            "Save",
            "an unlabeled control takes its name from descendant text"
        );
        let caption = frame_out
            .scene
            .iter()
            .find(|entry| entry.node == 2)
            .expect("caption is in the scene");
        assert_eq!(caption.label, 0, "non-control text derives no name");
        assert!(
            caption.editable,
            "an active field binder marks the scene entry editable"
        );
        assert!(!row.editable, "containers are not editable");
    }

    #[test]
    fn virtual_and_divider_unavailable_cases_are_total() {
        let each_doc = keyed_doc(
            &[slir::K_EACH],
            &[0],
            &[slir::NONE],
            &[slir::NONE],
            &[slir::NONE],
            &["items"],
        );
        let mut each = instance(each_doc);
        assert_eq!(frame::inst_each_window(&each, "items"), (-1, -1));
        assert!(!frame::inst_reveal_item(&mut each, "items", 0, 3));
        assert!(!each.dirty, "non-virtual fallbacks do not dirty");

        let divider_doc = keyed_doc(
            &[slir::K_DIVIDER],
            &[0],
            &[slir::NONE],
            &[slir::NONE],
            &[slir::NONE],
            &["orphan"],
        );
        let mut divider = instance(divider_doc);
        assert!(!frame::inst_set_divider(&mut divider, "orphan", 40.0));
        assert_eq!(frame::inst_get_divider(&divider, "orphan"), -1.0);
        assert!(!frame::inst_set_divider(&mut divider, "missing", 40.0));
        assert_eq!(frame::inst_get_divider(&divider, "missing"), -1.0);
        assert!(
            !divider.dirty,
            "invalid divider writes have no side effects"
        );
    }

    #[test]
    fn divider_overlay_requires_adjacent_panes_and_real_changes() {
        let doc = keyed_doc(
            &[slir::K_ROW, slir::K_RECT, slir::K_DIVIDER, slir::K_RECT],
            &[0, 0, 0, 0],
            &[slir::NONE, 0, 0, 0],
            &[1, slir::NONE, slir::NONE, slir::NONE],
            &[slir::NONE, 2, 3, slir::NONE],
            &["root", "before", "divider", "after"],
        );
        let mut instance = instance(doc);

        assert_eq!(frame::inst_get_divider(&instance, "divider"), -1.0);
        assert!(frame::inst_set_divider(&mut instance, "divider", 120.0));
        assert_eq!(frame::inst_get_divider(&instance, "divider"), 120.0);
        assert!(instance.dirty);
        instance.dirty = false;
        assert!(frame::inst_set_divider(&mut instance, "divider", 120.0));
        assert!(!instance.dirty, "equal overlay write is a no-op");
        assert!(!frame::inst_set_divider(&mut instance, "divider", f64::NAN));
        assert_eq!(frame::inst_get_divider(&instance, "divider"), 120.0);
        assert!(!instance.dirty, "non-finite extent is rejected atomically");
    }
}

#[cfg(test)]
mod runtime_image_resolution {
    use super::*;
    use crate::flatten::FrameOp;

    fn image_doc() -> slir::Doc {
        let mut doc = slir::doc_new();
        doc.ok = true;
        doc.strs = vec![String::new(), "source".to_string(), "avatar".to_string()];
        doc.node_kind = vec![slir::K_COL, slir::K_IMG, slir::K_IMG];
        doc.node_flags = vec![0; 3];
        doc.node_parent = vec![slir::NONE, 0, 0];
        doc.node_first = vec![1, slir::NONE, slir::NONE];
        doc.node_next = vec![slir::NONE, 2, slir::NONE];
        doc.node_key = vec![0; 3];
        doc.node_id = vec![0; 3];
        doc.node_line = vec![1, 2, 3];

        doc.aval_tag.push(slir::T_STR);
        doc.aval_lo.push(2);
        doc.aval_hi.push(0);
        doc.aval_num.push(0.0);
        doc.parm_name.push(1);
        doc.parm_type.push(slir::PARAM_TEXT);
        doc.parm_default.push(0);
        doc.parm_enum_off.push(0);
        doc.parm_enum_len.push(0);
        doc.parm_site_off.push(0);
        doc.parm_site_len.push(2);
        doc.parm_site_node.extend([1, 2]);
        doc.parm_site_attr.extend([slir::A_SRC, slir::A_SRC]);
        doc.attr_index.extend([0, 0]);
        add_attr(&mut doc, slir::A_SRC, slir::T_PARAM_REF, 0.0, 0);
        add_attr(&mut doc, slir::A_W, slir::T_SIZE_HUG, 0.0, 0);
        add_attr(&mut doc, slir::A_H, slir::T_SIZE_HUG, 0.0, 0);
        doc.attr_index.push(3);
        add_attr(&mut doc, slir::A_SRC, slir::T_PARAM_REF, 0.0, 0);
        add_attr(&mut doc, slir::A_W, slir::T_SIZE_HUG, 0.0, 0);
        add_attr(&mut doc, slir::A_H, slir::T_SIZE_HUG, 0.0, 0);
        doc.attr_index.push(6);
        doc.img_src.push(2);
        doc.img_w.push(32);
        doc.img_h.push(16);
        doc.img_format.push(0);
        doc.img_data.push(vec![137, 80, 78, 71]);
        doc
    }

    fn image_indices(frame: &crate::flatten::Frame) -> Vec<i32> {
        frame
            .ops
            .iter()
            .filter_map(|op| match op {
                FrameOp::Image(image) => Some(image.img),
                _ => None,
            })
            .collect()
    }
    fn image_sizes(frame: &crate::flatten::Frame) -> Vec<(f64, f64)> {
        frame
            .ops
            .iter()
            .filter_map(|op| match op {
                FrameOp::Image(image) => Some((image.w, image.h)),
                _ => None,
            })
            .collect()
    }

    fn set_source(instance: &mut frame::Instance, source: &str) {
        assert!(frame::inst_set_param(
            instance,
            0,
            &frame::ParamValue {
                kind: slir::PARAM_TEXT,
                num: 0.0,
                s: source.to_string(),
                rgba: 0,
                sym: String::new(),
            },
        ));
    }

    #[test]
    fn runtime_image_names_override_compiled_and_missing_sources_keep_layout() {
        let mut instance = frame::inst_shell();
        instance.doc = image_doc();
        frame::inst_init(&mut instance);
        frame::inst_set_env(&mut instance, 200.0, 200.0, 1, false, false);
        let compiled = frame::inst_frame(&mut instance, 0.0);
        assert_eq!(image_indices(&compiled), [0, 0]);

        let rgba = [255, 0, 0, 255, 0, 255, 0, 128];
        let runtime = frame::inst_img_register(&mut instance, "avatar", 2, 1, 1, &rgba);
        assert_eq!(runtime, 1);
        let overridden = frame::inst_frame(&mut instance, 0.0);
        assert_eq!(image_indices(&overridden), [runtime, runtime]);
        assert_eq!(image_sizes(&overridden), [(2.0, 1.0), (2.0, 1.0)]);

        set_source(&mut instance, "missing");
        let missing = frame::inst_frame(&mut instance, 0.0);
        assert!(image_indices(&missing).is_empty());
        assert_eq!(
            missing.scene.len(),
            3,
            "unresolved images keep layout and scene"
        );
        assert_eq!((missing.scene[1].w, missing.scene[1].h), (64.0, 64.0));
        assert_eq!((missing.scene[2].w, missing.scene[2].h), (64.0, 64.0));
        assert_eq!(
            instance
                .st
                .diag_code
                .iter()
                .filter(|code| code.as_str() == "img-missing")
                .count(),
            1,
            "equal missing names diagnose once per instance",
        );

        set_source(&mut instance, "avatar");
        assert_eq!(
            image_indices(&frame::inst_frame(&mut instance, 0.0)),
            [runtime, runtime],
        );
        assert!(frame::inst_img_unregister(&mut instance, "avatar"));
        let fallback = frame::inst_frame(&mut instance, 0.0);
        assert_eq!(
            image_indices(&fallback),
            [0, 0],
            "unregistration reveals the compiled source with the same name",
        );
        assert_eq!(image_sizes(&fallback), [(32.0, 16.0), (32.0, 16.0)]);
    }
}
