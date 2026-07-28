//! Host-side construction of kernel documents from the SLIR protobuf mirror.

use crate::read;

/// Decodes a SLIR envelope into a kernel document and its embedded image
/// payloads.
pub fn decode_doc(bytes: &[u8]) -> Result<(slab_kernel::slir::Doc, Vec<Vec<u8>>), String> {
	let mut wire = read::decode_pb(bytes)?;
	let imgs = std::mem::take(&mut wire.img_data);
	let mut doc = slab_kernel::slir::doc_new();
	doc.ok = true;
	doc.errs = std::mem::take(&mut wire.errs);
	doc.strs = std::mem::take(&mut wire.strs);
	doc.node_kind = std::mem::take(&mut wire.node_kind);
	doc.node_flags = std::mem::take(&mut wire.node_flags);
	doc.node_parent = read::decode_links(std::mem::take(&mut wire.node_parent));
	doc.node_first = read::decode_links(std::mem::take(&mut wire.node_first));
	doc.node_next = read::decode_links(std::mem::take(&mut wire.node_next));
	doc.node_key = std::mem::take(&mut wire.node_key);
	doc.node_id = std::mem::take(&mut wire.node_id);
	doc.node_line = std::mem::take(&mut wire.node_line);
	doc.aval_tag = std::mem::take(&mut wire.aval_tag);
	doc.aval_lo = std::mem::take(&mut wire.aval_lo);
	doc.aval_hi = std::mem::take(&mut wire.aval_hi);
	doc.aval_num = std::mem::take(&mut wire.aval_num);
	doc.f64s = std::mem::take(&mut wire.f64s);
	doc.tup_dyn_tag = std::mem::take(&mut wire.tup_dyn_tag);
	doc.tup_dyn_num = std::mem::take(&mut wire.tup_dyn_num);
	doc.tup_dyn_param = std::mem::take(&mut wire.tup_dyn_param);
	doc.grad_kind = std::mem::take(&mut wire.grad_kind);
	doc.grad_angle = std::mem::take(&mut wire.grad_angle);
	doc.grad_stop_off = std::mem::take(&mut wire.grad_stop_off);
	doc.grad_stop_len = std::mem::take(&mut wire.grad_stop_len);
	doc.grad_stop_pos = std::mem::take(&mut wire.grad_stop_pos);
	doc.grad_stop_rgba = std::mem::take(&mut wire.grad_stop_rgba);
	doc.shdw_x = std::mem::take(&mut wire.shdw_x);
	doc.shdw_y = std::mem::take(&mut wire.shdw_y);
	doc.shdw_blur = std::mem::take(&mut wire.shdw_blur);
	doc.shdw_spread = std::mem::take(&mut wire.shdw_spread);
	doc.shdw_rgba = std::mem::take(&mut wire.shdw_rgba);
	doc.shdw_inset = std::mem::take(&mut wire.shdw_inset);
	doc.attr_index = std::mem::take(&mut wire.attr_index);
	doc.attr_id = std::mem::take(&mut wire.attr_id);
	doc.attr_val = std::mem::take(&mut wire.attr_val);
	doc.path_verb_off = std::mem::take(&mut wire.path_verb_off);
	doc.path_verb_len = std::mem::take(&mut wire.path_verb_len);
	doc.path_coord_off = std::mem::take(&mut wire.path_coord_off);
	doc.path_coord_len = std::mem::take(&mut wire.path_coord_len);
	doc.path_verbs = std::mem::take(&mut wire.path_verbs);
	doc.path_coords = std::mem::take(&mut wire.path_coords);
	doc.font_family = std::mem::take(&mut wire.font_family);
	doc.font_class = std::mem::take(&mut wire.font_class);
	doc.font_weight = std::mem::take(&mut wire.font_weight);
	doc.font_upem = std::mem::take(&mut wire.font_upem);
	doc.font_ascent = std::mem::take(&mut wire.font_ascent);
	doc.font_descent = std::mem::take(&mut wire.font_descent);
	doc.font_line_gap = std::mem::take(&mut wire.font_line_gap);
	doc.font_default_adv = std::mem::take(&mut wire.font_default_adv);
	doc.font_underline_position = std::mem::take(&mut wire.font_underline_position);
	doc.font_underline_thickness = std::mem::take(&mut wire.font_underline_thickness);
	doc.font_data_off = std::mem::take(&mut wire.font_data_off);
	doc.font_data_len = std::mem::take(&mut wire.font_data_len);
	doc.font_data = std::mem::take(&mut wire.font_data);
	doc.font_cmap_off = std::mem::take(&mut wire.font_cmap_off);
	doc.font_cmap_len = std::mem::take(&mut wire.font_cmap_len);
	doc.font_cmap_cp = std::mem::take(&mut wire.font_cmap_cp);
	doc.font_cmap_gid = std::mem::take(&mut wire.font_cmap_gid);
	doc.font_adv = std::mem::take(&mut wire.font_adv);
	doc.cond_kind = std::mem::take(&mut wire.cond_kind);
	doc.cond_neg = std::mem::take(&mut wire.cond_neg);
	doc.cond_op = std::mem::take(&mut wire.cond_op);
	doc.cond_num = std::mem::take(&mut wire.cond_num);
	doc.cond_sym = std::mem::take(&mut wire.cond_sym);
	doc.patch_node = std::mem::take(&mut wire.patch_node);
	doc.patch_cond = std::mem::take(&mut wire.patch_cond);
	doc.patch_attr_off = std::mem::take(&mut wire.patch_attr_off);
	doc.patch_attr_len = std::mem::take(&mut wire.patch_attr_len);
	doc.patch_child_off = std::mem::take(&mut wire.patch_child_off);
	doc.patch_child_len = std::mem::take(&mut wire.patch_child_len);
	doc.wattr_id = std::mem::take(&mut wire.wattr_id);
	doc.wattr_val = std::mem::take(&mut wire.wattr_val);
	doc.patch_children = std::mem::take(&mut wire.patch_children);
	doc.anim_name = std::mem::take(&mut wire.anim_name);
	doc.anim_stop_off = std::mem::take(&mut wire.anim_stop_off);
	doc.anim_stop_len = std::mem::take(&mut wire.anim_stop_len);
	doc.anim_stop_pos = std::mem::take(&mut wire.anim_stop_pos);
	doc.anim_stop_attr_off = std::mem::take(&mut wire.anim_stop_attr_off);
	doc.anim_stop_attr_len = std::mem::take(&mut wire.anim_stop_attr_len);
	doc.aattr_id = std::mem::take(&mut wire.aattr_id);
	doc.aattr_val = std::mem::take(&mut wire.aattr_val);
	doc.bind_node = std::mem::take(&mut wire.bind_node);
	doc.bind_anim = std::mem::take(&mut wire.bind_anim);
	doc.bind_dur = std::mem::take(&mut wire.bind_dur);
	doc.bind_mode = std::mem::take(&mut wire.bind_mode);
	doc.bind_easing = std::mem::take(&mut wire.bind_easing);
	doc.bind_delay = std::mem::take(&mut wire.bind_delay);
	doc.trans_node = std::mem::take(&mut wire.trans_node);
	doc.trans_easing = std::mem::take(&mut wire.trans_easing);
	doc.trans_dur = std::mem::take(&mut wire.trans_dur);
	doc.trans_delay = std::mem::take(&mut wire.trans_delay);
	doc.parm_name = std::mem::take(&mut wire.parm_name);
	doc.parm_type = std::mem::take(&mut wire.parm_type);
	doc.parm_default = std::mem::take(&mut wire.parm_default);
	doc.parm_enum_off = std::mem::take(&mut wire.parm_enum_off);
	doc.parm_enum_len = std::mem::take(&mut wire.parm_enum_len);
	doc.parm_site_off = std::mem::take(&mut wire.parm_site_off);
	doc.parm_site_len = std::mem::take(&mut wire.parm_site_len);
	doc.parm_enum_syms = std::mem::take(&mut wire.parm_enum_syms);
	doc.parm_site_node = std::mem::take(&mut wire.parm_site_node);
	doc.parm_site_attr = std::mem::take(&mut wire.parm_site_attr);
	doc.list_param = std::mem::take(&mut wire.list_param);
	doc.list_field_off = std::mem::take(&mut wire.list_field_off);
	doc.list_field_len = std::mem::take(&mut wire.list_field_len);
	doc.list_field_name = std::mem::take(&mut wire.list_field_name);
	doc.list_field_type = std::mem::take(&mut wire.list_field_type);
	doc.list_field_default = std::mem::take(&mut wire.list_field_default);
	doc.list_field_sub = std::mem::take(&mut wire.list_field_sub);
	doc.list_field_enum_off = std::mem::take(&mut wire.list_field_enum_off);
	doc.list_field_enum_len = std::mem::take(&mut wire.list_field_enum_len);
	doc.list_enum_syms = std::mem::take(&mut wire.list_enum_syms);
	doc.list_item_field_off = std::mem::take(&mut wire.list_item_field_off);
	doc.list_item_field_len = std::mem::take(&mut wire.list_item_field_len);
	doc.list_item_value_field = std::mem::take(&mut wire.list_item_value_field);
	doc.list_item_value_val = std::mem::take(&mut wire.list_item_value_val);
	doc.theme_name = std::mem::take(&mut wire.theme_name);
	doc.token_name = std::mem::take(&mut wire.token_name);
	doc.token_base = std::mem::take(&mut wire.token_base);
	doc.token_base_repr = std::mem::take(&mut wire.token_base_repr);
	doc.token_theme_off = std::mem::take(&mut wire.token_theme_off);
	doc.token_theme_len = std::mem::take(&mut wire.token_theme_len);
	doc.token_theme_name = std::mem::take(&mut wire.token_theme_name);
	doc.token_theme_val = std::mem::take(&mut wire.token_theme_val);
	doc.token_theme_repr = std::mem::take(&mut wire.token_theme_repr);
	doc.hole_name = std::mem::take(&mut wire.hole_name);
	doc.hole_node = std::mem::take(&mut wire.hole_node);
	doc.sign_name = std::mem::take(&mut wire.sign_name);
	doc.sign_node = std::mem::take(&mut wire.sign_node);
	doc.sign_trigger = std::mem::take(&mut wire.sign_trigger);
	doc.icon_name = std::mem::take(&mut wire.icon_name);
	doc.icon_node = std::mem::take(&mut wire.icon_node);
	doc.icon_viewbox = std::mem::take(&mut wire.icon_viewbox);
	doc.img_src = std::mem::take(&mut wire.img_src);
	doc.img_w = std::mem::take(&mut wire.img_w);
	doc.img_h = std::mem::take(&mut wire.img_h);
	doc.img_format = std::mem::take(&mut wire.img_format);
	doc.img_data.clone_from(&imgs);
	let font_count = doc.font_upem.len();
	if doc.font_underline_position.is_empty() {
		doc.font_underline_position.extend(
			doc.font_upem
				.iter()
				.map(|upem| -(i32::try_from(*upem / 10).expect("font upem fits i32"))),
		);
	}
	if doc.font_underline_thickness.is_empty() {
		doc.font_underline_thickness.extend(
			doc.font_upem
				.iter()
				.map(|upem| i32::try_from((*upem / 20).max(1)).expect("font upem fits i32")),
		);
	}
	if doc.font_data_off.is_empty() && doc.font_data_len.is_empty() {
		doc.font_data_off.resize(font_count, 0);
		doc.font_data_len.resize(font_count, 0);
	}
	if doc.font_data_off.len() != font_count || doc.font_data_len.len() != font_count {
		return Err("font data: parallel arrays have mismatched lengths".into());
	}
	for (&offset, &length) in doc.font_data_off.iter().zip(&doc.font_data_len) {
		let end = offset
			.checked_add(length)
			.ok_or_else(|| "font data: range overflow".to_string())?;
		if offset < 0
			|| length < 0
			|| usize::try_from(end).map_or(true, |end| end > doc.font_data.len())
		{
			return Err("font data: range is out of bounds".into());
		}
	}
	if doc.font_underline_position.len() != font_count
		|| doc.font_underline_thickness.len() != font_count
	{
		return Err("font underline metrics: parallel arrays have mismatched lengths".into());
	}
	let token_count = doc.token_name.len();
	if [
		doc.token_base.len(),
		doc.token_base_repr.len(),
		doc.token_theme_off.len(),
		doc.token_theme_len.len(),
	]
	.iter()
	.any(|&length| length != token_count)
	{
		return Err("token: parallel arrays have mismatched lengths".into());
	}
	if doc.token_theme_name.len() != doc.token_theme_val.len()
		|| doc.token_theme_name.len() != doc.token_theme_repr.len()
	{
		return Err("token theme: parallel arrays have mismatched lengths".into());
	}
	for index in 0..token_count {
		let start = usize::try_from(doc.token_theme_off[index])
			.map_err(|_| "token theme offset must be nonnegative")?;
		let len = usize::try_from(doc.token_theme_len[index])
			.map_err(|_| "token theme length must be nonnegative")?;
		if start
			.checked_add(len)
			.is_none_or(|end| end > doc.token_theme_name.len())
		{
			return Err(format!("token theme range for row {index} is out of bounds"));
		}
	}
	if doc.list_field_sub.is_empty() {
		doc.list_field_sub.resize(doc.list_field_name.len(), 0);
	}
	if doc.list_field_sub.len() != doc.list_field_name.len() {
		return Err("list_field_sub: parallel arrays have mismatched lengths".into());
	}
	if doc.list_field_type.len() != doc.list_field_name.len() {
		return Err("list_field_type: parallel arrays have mismatched lengths".into());
	}
	for (index, &sub) in doc.list_field_sub.iter().enumerate() {
		let is_list = doc.list_field_type[index] == slab_kernel::slir::PARAM_LIST;
		if is_list != (sub != 0) {
			return Err(format!("list_field_sub: field {index} type/sub-schema mismatch"));
		}
		if sub != 0 && usize::try_from(sub - 1).map_or(true, |row| row >= doc.list_param.len()) {
			return Err(format!("list_field_sub: schema row {} is out of range", sub - 1));
		}
	}
	if doc.icon_viewbox.is_empty() {
		doc.icon_viewbox.resize(doc.icon_name.len(), 24.0);
	}
	if doc.icon_name.len() != doc.icon_node.len() || doc.icon_name.len() != doc.icon_viewbox.len() {
		return Err("icon: parallel arrays have mismatched lengths".into());
	}
	for (&name, &node) in doc.icon_name.iter().zip(&doc.icon_node) {
		let name = usize::try_from(name).map_err(|_| "icon_name: string index exceeds usize")?;
		if name >= doc.strs.len() {
			return Err(format!("icon_name: string index {name} is out of range"));
		}
		let node = usize::try_from(node).map_err(|_| "icon_node: node index exceeds usize")?;
		if node >= doc.node_flags.len() {
			return Err(format!("icon_node: node index {node} is out of range"));
		}
		if doc.node_flags[node] & slab_kernel::slir::F_DETACHED == 0 {
			return Err(format!("icon_node: node {node} is not detached"));
		}
	}
	Ok((doc, imgs))
}

/// Builds an initialized kernel instance from a host-decoded SLIR document.
pub fn instance(bytes: &[u8]) -> Result<(slab_kernel::frame::Instance, Vec<Vec<u8>>), String> {
	let (doc, imgs) = decode_doc(bytes)?;
	let instance = slab_kernel::frame::inst_from_doc(doc);
	Ok((instance, imgs))
}

#[cfg(test)]
mod tests {
	use super::{decode_doc, instance};
	use crate::{ImgE, Nodes, Slir, write};

	#[test]
	fn host_decoder_constructs_kernel_document_and_instance() {
		let slir = Slir {
			strs: vec![String::new()],
			nodes: Nodes {
				kind:        vec![0],
				flags:       vec![0],
				parent:      vec![crate::NONE],
				first_child: vec![crate::NONE],
				next_sib:    vec![crate::NONE],
				key:         vec![0],
				id:          vec![0],
				src_line:    vec![1],
			},
			attr_index: vec![0, 0],
			images: vec![ImgE {
				src:      0,
				w:        1,
				h:        1,
				format:   0,
				blob_off: 0,
				blob_len: 3,
			}],
			blob: vec![1, 2, 3],
			..Default::default()
		};
		let bytes = write(&slir);

		let (doc, images) = decode_doc(&bytes).expect("host decoder must decode");
		assert!(doc.ok);
		assert_eq!(doc.node_parent, vec![slab_kernel::slir::NONE]);
		assert_eq!(images, vec![vec![1, 2, 3]]);

		let (instance, images) = instance(&bytes).expect("instance must initialize");
		assert!(instance.ok);
		assert_eq!(images, vec![vec![1, 2, 3]]);
	}
}
