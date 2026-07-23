//! SLIR protobuf writer. Documents use a fixed `SLIR` 2.0 envelope around a
//! snappy raw block containing a protobuf `Doc` message.

use crate::{Slir, TupDynE, aval, pb};
use prost::Message;

fn i32_from_usize(value: usize) -> i32 {
    i32::try_from(value).expect("SLIR pool exceeds i32")
}

fn i32_from_u32(value: u32) -> i32 {
    i32::try_from(value).expect("SLIR fencepost exceeds i32")
}

fn push_run(offsets: &mut Vec<i32>, lengths: &mut Vec<i32>, offset: usize, length: usize) {
    offsets.push(i32_from_usize(offset));
    lengths.push(i32_from_usize(length));
}

fn encode_link(link: u32) -> u32 {
    if link == crate::NONE {
        0
    } else {
        link.checked_add(1).expect("node link exceeds u32")
    }
}

fn numeric_aval(tag: u8) -> bool {
    matches!(
        tag,
        aval::NUM | aval::PCT | aval::SIZE_FIXED | aval::SIZE_FILL | aval::SIZE_PCT
    )
}

fn image_data(s: &Slir) -> Vec<Vec<u8>> {
    s.images
        .iter()
        .map(|image| {
            let end = image
                .blob_off
                .checked_add(image.blob_len)
                .expect("image blob range overflows u32");
            let start = usize::try_from(image.blob_off).expect("image blob offset exceeds usize");
            let end = usize::try_from(end).expect("image blob end exceeds usize");
            s.blob
                .get(start..end)
                .expect("image blob range is outside the SLIR staging blob")
                .to_vec()
        })
        .collect()
}

/// Convert the compiler-side document into its protobuf mirror.
pub(crate) fn to_pb(s: &Slir) -> pb::Doc {
    let mut doc = pb::Doc {
        ok: true,
        strs: s.strs.clone(),
        node_kind: s.nodes.kind.iter().copied().map(u32::from).collect(),
        node_flags: s.nodes.flags.iter().copied().map(u32::from).collect(),
        node_parent: s.nodes.parent.iter().copied().map(encode_link).collect(),
        node_first: s
            .nodes
            .first_child
            .iter()
            .copied()
            .map(encode_link)
            .collect(),
        node_next: s.nodes.next_sib.iter().copied().map(encode_link).collect(),
        node_key: s.nodes.key.clone(),
        node_id: s.nodes.id.clone(),
        node_line: s.nodes.src_line.clone(),
        aval_tag: s.avals.iter().map(|value| u32::from(value.tag)).collect(),
        aval_lo: s.avals.iter().map(crate::Aval::lo).collect(),
        aval_hi: s.avals.iter().map(crate::Aval::hi).collect(),
        aval_num: s
            .avals
            .iter()
            .map(|value| {
                if numeric_aval(value.tag) {
                    value.as_f64()
                } else {
                    0.0
                }
            })
            .collect(),
        f64s: s.f64s.clone(),
        tup_dyn_tag: s
            .tup_dyn
            .iter()
            .map(|member| match member {
                TupDynE::Lit(_) => 0,
                TupDynE::Param(_) => 1,
            })
            .collect(),
        tup_dyn_num: s
            .tup_dyn
            .iter()
            .map(|member| match member {
                TupDynE::Lit(value) => *value,
                TupDynE::Param(_) => 0.0,
            })
            .collect(),
        tup_dyn_param: s
            .tup_dyn
            .iter()
            .map(|member| match member {
                TupDynE::Lit(_) => 0,
                TupDynE::Param(param) => *param,
            })
            .collect(),
        attr_index: s.attr_index.iter().copied().map(i32_from_u32).collect(),
        ..Default::default()
    };

    for gradient in &s.grads {
        doc.grad_kind.push(u32::from(gradient.kind));
        doc.grad_angle.push(gradient.angle);
        push_run(
            &mut doc.grad_stop_off,
            &mut doc.grad_stop_len,
            doc.grad_stop_pos.len(),
            gradient.stops.len(),
        );
        for &(position, rgba) in &gradient.stops {
            doc.grad_stop_pos.push(position);
            doc.grad_stop_rgba.push(rgba);
        }
    }

    for shadow in &s.shadows {
        doc.shdw_x.push(shadow.x);
        doc.shdw_y.push(shadow.y);
        doc.shdw_blur.push(shadow.blur);
        doc.shdw_spread.push(shadow.spread);
        doc.shdw_rgba.push(shadow.rgba);
        doc.shdw_inset.push(u32::from(shadow.inset));
    }

    for &(id, value) in &s.attrs {
        doc.attr_id.push(u32::from(id));
        doc.attr_val.push(value);
    }

    for path in &s.paths {
        push_run(
            &mut doc.path_verb_off,
            &mut doc.path_verb_len,
            doc.path_verbs.len(),
            path.verbs.len(),
        );
        doc.path_verbs
            .extend(path.verbs.iter().copied().map(u32::from));
        push_run(
            &mut doc.path_coord_off,
            &mut doc.path_coord_len,
            doc.path_coords.len(),
            path.coords.len(),
        );
        doc.path_coords.extend_from_slice(&path.coords);
    }

    for font in &s.fonts {
        doc.font_family.push(font.family);
        doc.font_class.push(u32::from(font.class));
        doc.font_weight.push(u32::from(font.weight));
        doc.font_upem.push(u32::from(font.upem));
        doc.font_ascent.push(i32::from(font.ascent));
        doc.font_descent.push(i32::from(font.descent));
        doc.font_line_gap.push(i32::from(font.line_gap));
        doc.font_default_adv.push(u32::from(font.default_advance));
        push_run(
            &mut doc.font_cmap_off,
            &mut doc.font_cmap_len,
            doc.font_cmap_cp.len(),
            font.cmap.len(),
        );
        for &(codepoint, glyph) in &font.cmap {
            doc.font_cmap_cp.push(codepoint);
            doc.font_cmap_gid.push(u32::from(glyph));
        }
        doc.font_adv
            .extend(font.advances.iter().copied().map(u32::from));
    }

    for condition in &s.conds {
        doc.cond_kind.push(u32::from(condition.kind));
        doc.cond_neg.push(u32::from(condition.neg));
        doc.cond_op.push(u32::from(condition.op));
        doc.cond_num.push(condition.num);
        doc.cond_sym.push(condition.sym);
    }

    for patch in &s.patches {
        doc.patch_node.push(patch.node);
        doc.patch_cond.push(patch.cond);
        doc.patch_attr_off.push(i32_from_u32(patch.attr_off));
        doc.patch_attr_len.push(i32_from_u32(patch.attr_len));
        doc.patch_child_off.push(i32_from_u32(patch.child_off));
        doc.patch_child_len.push(i32_from_u32(patch.child_len));
    }
    for &(id, value) in &s.patch_attrs {
        doc.wattr_id.push(u32::from(id));
        doc.wattr_val.push(value);
    }
    doc.patch_children.clone_from(&s.patch_children);

    for animation in &s.anims {
        doc.anim_name.push(animation.name);
        push_run(
            &mut doc.anim_stop_off,
            &mut doc.anim_stop_len,
            doc.anim_stop_pos.len(),
            animation.stops.len(),
        );
        for &(position, attr_off, attr_len) in &animation.stops {
            doc.anim_stop_pos.push(position);
            doc.anim_stop_attr_off.push(i32_from_u32(attr_off));
            doc.anim_stop_attr_len.push(i32_from_u32(attr_len));
        }
    }
    for &(id, value) in &s.anim_attrs {
        doc.aattr_id.push(u32::from(id));
        doc.aattr_val.push(value);
    }
    for binding in &s.bindings {
        doc.bind_node.push(binding.node);
        doc.bind_anim.push(binding.anim);
        doc.bind_dur.push(binding.dur);
        doc.bind_mode.push(u32::from(binding.mode));
        doc.bind_easing.push(u32::from(binding.easing));
        doc.bind_delay.push(binding.delay);
    }
    for transition in &s.transitions {
        doc.trans_node.push(transition.node);
        doc.trans_easing.push(u32::from(transition.easing));
        doc.trans_dur.push(transition.dur);
        doc.trans_delay.push(transition.delay);
    }

    for parameter in &s.params {
        doc.parm_name.push(parameter.name);
        doc.parm_type.push(u32::from(parameter.ty));
        doc.parm_default.push(parameter.default);
        doc.parm_enum_off.push(i32_from_u32(parameter.enum_off));
        doc.parm_enum_len.push(i32_from_u32(parameter.enum_len));
        doc.parm_site_off.push(i32_from_u32(parameter.site_off));
        doc.parm_site_len.push(i32_from_u32(parameter.site_len));
    }
    doc.parm_enum_syms.clone_from(&s.param_enum_syms);
    for &(node, attr) in &s.param_sites {
        doc.parm_site_node.push(node);
        doc.parm_site_attr.push(u32::from(attr));
    }

    for list in &s.lists {
        doc.list_param.push(list.param);
        doc.list_field_off.push(i32_from_u32(list.field_off));
        doc.list_field_len.push(i32_from_u32(list.field_len));
    }
    for field in &s.list_fields {
        doc.list_field_name.push(field.name);
        doc.list_field_type.push(u32::from(field.ty));
        doc.list_field_default.push(field.default);
        doc.list_field_enum_off.push(i32_from_u32(field.enum_off));
        doc.list_field_enum_len.push(i32_from_u32(field.enum_len));
        doc.list_field_sub.push(field.sub);
    }
    doc.list_enum_syms.clone_from(&s.list_enum_syms);
    for item in &s.list_items {
        doc.list_item_field_off.push(i32_from_u32(item.field_off));
        doc.list_item_field_len.push(i32_from_u32(item.field_len));
    }
    for value in &s.list_item_values {
        doc.list_item_value_field.push(value.field);
        doc.list_item_value_val.push(value.val);
    }

    doc.theme_name.clone_from(&s.themes);
    for &(name, node) in &s.holes {
        doc.hole_name.push(name);
        doc.hole_node.push(node);
    }
    for &(name, node, trigger) in &s.signals {
        doc.sign_name.push(name);
        doc.sign_node.push(node);
        doc.sign_trigger.push(u32::from(trigger));
    }
    for icon in &s.icons {
        doc.icon_name.push(icon.name);
        doc.icon_node.push(icon.node);
        doc.icon_viewbox.push(icon.viewbox);
    }
    for image in &s.images {
        doc.img_src.push(image.src);
        doc.img_w.push(image.w);
        doc.img_h.push(image.h);
        doc.img_format.push(u32::from(image.format));
    }
    doc.img_data = image_data(s);

    doc
}

/// Serialize a `Slir` document to canonical SLIR 2.0 bytes.
pub fn write(s: &Slir) -> Vec<u8> {
    let mut encoded = Vec::new();
    to_pb(s)
        .encode(&mut encoded)
        .expect("encoding a protobuf document into Vec cannot fail");

    let mut encoder = snap::raw::Encoder::new();
    let compressed = encoder
        .compress_vec(&encoded)
        .expect("compressing a protobuf document cannot fail");
    let mut output = Vec::with_capacity(8 + compressed.len());
    output.extend_from_slice(b"SLIR");
    output.extend_from_slice(&crate::MAJOR.to_le_bytes());
    output.extend_from_slice(&crate::MINOR.to_le_bytes());
    output.extend_from_slice(&compressed);
    output
}
