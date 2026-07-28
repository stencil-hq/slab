//! SLIR protobuf reader. It validates the 2.0 envelope, decompresses its
//! snappy raw block, then reconstructs the compiler-side representation.

use std::ops::Range;

use prost::Message;

use crate::{
	AnimE, Aval, BindE, CondE, FontE, GradE, IconE, ImgE, ListE, ListFieldE, ListItemE,
	ListItemValueE, Nodes, ParamE, PatchE, PathE, ShadowE, Slir, TokenE, TransE, TupDynE, pb,
};

fn same_len(name: &str, lengths: &[usize]) -> Result<usize, String> {
	let Some((first, rest)) = lengths.split_first() else {
		return Ok(0);
	};
	if rest.iter().any(|length| length != first) {
		return Err(format!("{name}: parallel arrays have mismatched lengths"));
	}
	Ok(*first)
}

fn u8s(name: &str, values: Vec<u32>) -> Result<Vec<u8>, String> {
	values
		.into_iter()
		.map(|value| u8::try_from(value).map_err(|_| format!("{name}: {value} does not fit u8")))
		.collect()
}

fn u16s(name: &str, values: Vec<u32>) -> Result<Vec<u16>, String> {
	values
		.into_iter()
		.map(|value| u16::try_from(value).map_err(|_| format!("{name}: {value} does not fit u16")))
		.collect()
}

fn i16s(name: &str, values: Vec<i32>) -> Result<Vec<i16>, String> {
	values
		.into_iter()
		.map(|value| i16::try_from(value).map_err(|_| format!("{name}: {value} does not fit i16")))
		.collect()
}

fn nonnegative(name: &str, values: Vec<i32>) -> Result<Vec<u32>, String> {
	values
		.into_iter()
		.map(|value| u32::try_from(value).map_err(|_| format!("{name}: {value} is negative")))
		.collect()
}

fn pairs<T, U>(name: &str, left: Vec<T>, right: Vec<U>) -> Result<Vec<(T, U)>, String> {
	if left.len() != right.len() {
		return Err(format!("{name}: parallel arrays have mismatched lengths"));
	}
	Ok(left.into_iter().zip(right).collect())
}

fn index_range(offset: u32, length: u32, total: usize, name: &str) -> Result<Range<usize>, String> {
	let start = usize::try_from(offset).map_err(|_| format!("{name}: offset exceeds usize"))?;
	let length = usize::try_from(length).map_err(|_| format!("{name}: length exceeds usize"))?;
	let end = start
		.checked_add(length)
		.ok_or_else(|| format!("{name}: range overflows usize"))?;
	if end > total {
		return Err(format!("{name}: range {start}..{end} exceeds pool length {total}"));
	}
	Ok(start..end)
}

fn run<'a, T>(values: &'a [T], offset: u32, length: u32, name: &str) -> Result<&'a [T], String> {
	Ok(&values[index_range(offset, length, values.len(), name)?])
}

pub(crate) fn decode_links(values: Vec<u32>) -> Vec<u32> {
	values
		.into_iter()
		.map(|value| if value == 0 { crate::NONE } else { value - 1 })
		.collect()
}

fn header(bytes: &[u8]) -> Result<&[u8], String> {
	if bytes.len() < 4 {
		return Err(format!("truncated: need 4 bytes at offset 0, have {}", bytes.len()));
	}
	if &bytes[..4] != b"SLIR" {
		return Err("not a SLIR file (bad magic)".into());
	}
	if bytes.len() < 6 {
		return Err(format!("truncated: need 2 bytes at offset 4, have {}", bytes.len() - 4));
	}
	let major = u16::from_le_bytes([bytes[4], bytes[5]]);
	if major != crate::MAJOR {
		return Err(format!("unsupported SLIR major version {major} (want {})", crate::MAJOR));
	}
	if bytes.len() < 8 {
		return Err(format!("truncated: need 2 bytes at offset 6, have {}", bytes.len() - 6));
	}
	let minor = u16::from_le_bytes([bytes[6], bytes[7]]);
	if minor > crate::MINOR {
		return Err(format!("unsupported SLIR minor version {minor} (maximum {})", crate::MINOR));
	}
	Ok(&bytes[8..])
}

/// Decode the protobuf document inside a validated SLIR 2.0 envelope.
pub(crate) fn decode_pb(bytes: &[u8]) -> Result<pb::Doc, String> {
	let mut decoder = snap::raw::Decoder::new();
	let uncompressed = decoder
		.decompress_vec(header(bytes)?)
		.map_err(|error| format!("invalid SLIR snappy payload: {error}"))?;
	pb::Doc::decode(uncompressed.as_slice())
		.map_err(|error| format!("invalid SLIR protobuf payload: {error}"))
}

fn from_pb(mut doc: pb::Doc) -> Result<Slir, String> {
	let mut slir = Slir { strs: std::mem::take(&mut doc.strs), ..Slir::default() };

	let node_kind = u8s("node_kind", std::mem::take(&mut doc.node_kind))?;
	let node_flags = u16s("node_flags", std::mem::take(&mut doc.node_flags))?;
	let node_parent = decode_links(std::mem::take(&mut doc.node_parent));
	let node_first = decode_links(std::mem::take(&mut doc.node_first));
	let node_next = decode_links(std::mem::take(&mut doc.node_next));
	let node_key = std::mem::take(&mut doc.node_key);
	let node_id = std::mem::take(&mut doc.node_id);
	let node_line = std::mem::take(&mut doc.node_line);
	same_len("node", &[
		node_kind.len(),
		node_flags.len(),
		node_parent.len(),
		node_first.len(),
		node_next.len(),
		node_key.len(),
		node_id.len(),
		node_line.len(),
	])?;
	slir.nodes = Nodes {
		kind:        node_kind,
		flags:       node_flags,
		parent:      node_parent,
		first_child: node_first,
		next_sib:    node_next,
		key:         node_key,
		id:          node_id,
		src_line:    node_line,
	};

	let aval_tag = u8s("aval_tag", std::mem::take(&mut doc.aval_tag))?;
	let aval_lo = std::mem::take(&mut doc.aval_lo);
	let aval_hi = std::mem::take(&mut doc.aval_hi);
	let aval_num = std::mem::take(&mut doc.aval_num);
	same_len("aval", &[aval_tag.len(), aval_lo.len(), aval_hi.len(), aval_num.len()])?;
	slir.avals = aval_tag
		.into_iter()
		.zip(aval_lo)
		.zip(aval_hi)
		.zip(aval_num)
		.map(|(((tag, lo), hi), _)| Aval { tag, payload: Aval::pair(lo, hi) })
		.collect();
	slir.f64s = std::mem::take(&mut doc.f64s);

	let tup_dyn_tag = u8s("tup_dyn_tag", std::mem::take(&mut doc.tup_dyn_tag))?;
	let tup_dyn_num = std::mem::take(&mut doc.tup_dyn_num);
	let tup_dyn_param = std::mem::take(&mut doc.tup_dyn_param);
	same_len("tup_dyn", &[tup_dyn_tag.len(), tup_dyn_num.len(), tup_dyn_param.len()])?;
	slir.tup_dyn = tup_dyn_tag
		.into_iter()
		.zip(tup_dyn_num)
		.zip(tup_dyn_param)
		.map(|((tag, num), param)| match tag {
			0 => Ok(TupDynE::Lit(num)),
			1 => Ok(TupDynE::Param(param)),
			other => Err(format!("tup_dyn_tag: invalid member tag {other}")),
		})
		.collect::<Result<Vec<_>, String>>()?;

	let grad_kind = u8s("grad_kind", std::mem::take(&mut doc.grad_kind))?;
	let grad_angle = std::mem::take(&mut doc.grad_angle);
	let grad_stop_off = nonnegative("grad_stop_off", std::mem::take(&mut doc.grad_stop_off))?;
	let grad_stop_len = nonnegative("grad_stop_len", std::mem::take(&mut doc.grad_stop_len))?;
	let grad_stops = pairs(
		"gradient stop",
		std::mem::take(&mut doc.grad_stop_pos),
		std::mem::take(&mut doc.grad_stop_rgba),
	)?;
	let gradient_count = same_len("gradient", &[
		grad_kind.len(),
		grad_angle.len(),
		grad_stop_off.len(),
		grad_stop_len.len(),
	])?;
	let mut gradients = Vec::with_capacity(gradient_count);
	for index in 0..gradient_count {
		gradients.push(GradE {
			kind:  grad_kind[index],
			angle: grad_angle[index],
			stops: run(&grad_stops, grad_stop_off[index], grad_stop_len[index], "gradient stop")?
				.to_vec(),
		});
	}
	slir.grads = gradients;

	let shdw_x = std::mem::take(&mut doc.shdw_x);
	let shdw_y = std::mem::take(&mut doc.shdw_y);
	let shdw_blur = std::mem::take(&mut doc.shdw_blur);
	let shdw_spread = std::mem::take(&mut doc.shdw_spread);
	let shdw_rgba = std::mem::take(&mut doc.shdw_rgba);
	let shdw_inset = u8s("shdw_inset", std::mem::take(&mut doc.shdw_inset))?;
	let shadow_count = same_len("shadow", &[
		shdw_x.len(),
		shdw_y.len(),
		shdw_blur.len(),
		shdw_spread.len(),
		shdw_rgba.len(),
		shdw_inset.len(),
	])?;
	let mut shadows = Vec::with_capacity(shadow_count);
	for index in 0..shadow_count {
		shadows.push(ShadowE {
			x:      shdw_x[index],
			y:      shdw_y[index],
			blur:   shdw_blur[index],
			spread: shdw_spread[index],
			rgba:   shdw_rgba[index],
			inset:  shdw_inset[index],
		});
	}
	slir.shadows = shadows;

	slir.attr_index = nonnegative("attr_index", std::mem::take(&mut doc.attr_index))?;
	slir.attrs = pairs(
		"attr",
		u16s("attr_id", std::mem::take(&mut doc.attr_id))?,
		std::mem::take(&mut doc.attr_val),
	)?;

	let path_verb_off = nonnegative("path_verb_off", std::mem::take(&mut doc.path_verb_off))?;
	let path_verb_len = nonnegative("path_verb_len", std::mem::take(&mut doc.path_verb_len))?;
	let path_coord_off = nonnegative("path_coord_off", std::mem::take(&mut doc.path_coord_off))?;
	let path_coord_len = nonnegative("path_coord_len", std::mem::take(&mut doc.path_coord_len))?;
	let path_verbs = u8s("path_verbs", std::mem::take(&mut doc.path_verbs))?;
	let path_coords = std::mem::take(&mut doc.path_coords);
	let path_count = same_len("path", &[
		path_verb_off.len(),
		path_verb_len.len(),
		path_coord_off.len(),
		path_coord_len.len(),
	])?;
	let mut paths = Vec::with_capacity(path_count);
	for index in 0..path_count {
		paths.push(PathE {
			verbs:  run(&path_verbs, path_verb_off[index], path_verb_len[index], "path verb")?
				.to_vec(),
			coords: run(
				&path_coords,
				path_coord_off[index],
				path_coord_len[index],
				"path coordinate",
			)?
			.to_vec(),
		});
	}
	slir.paths = paths;

	let font_family = std::mem::take(&mut doc.font_family);
	let font_class = u8s("font_class", std::mem::take(&mut doc.font_class))?;
	let font_weight = u16s("font_weight", std::mem::take(&mut doc.font_weight))?;
	let font_upem = u16s("font_upem", std::mem::take(&mut doc.font_upem))?;
	let font_ascent = i16s("font_ascent", std::mem::take(&mut doc.font_ascent))?;
	let font_descent = i16s("font_descent", std::mem::take(&mut doc.font_descent))?;
	let font_line_gap = i16s("font_line_gap", std::mem::take(&mut doc.font_line_gap))?;
	let font_default_adv = u16s("font_default_adv", std::mem::take(&mut doc.font_default_adv))?;
	let mut font_underline_position =
		i16s("font_underline_position", std::mem::take(&mut doc.font_underline_position))?;
	let mut font_underline_thickness =
		i16s("font_underline_thickness", std::mem::take(&mut doc.font_underline_thickness))?;
	if font_underline_position.is_empty() {
		font_underline_position.extend(
			font_upem
				.iter()
				.map(|upem| -(i16::try_from(*upem / 10).expect("font upem fits i16"))),
		);
	}
	if font_underline_thickness.is_empty() {
		font_underline_thickness.extend(
			font_upem
				.iter()
				.map(|upem| i16::try_from((*upem / 20).max(1)).expect("font upem fits i16")),
		);
	}
	let mut font_data_off = nonnegative("font_data_off", std::mem::take(&mut doc.font_data_off))?;
	let mut font_data_len = nonnegative("font_data_len", std::mem::take(&mut doc.font_data_len))?;
	let font_data = std::mem::take(&mut doc.font_data);
	if font_data_off.is_empty() && font_data_len.is_empty() {
		font_data_off.resize(font_upem.len(), 0);
		font_data_len.resize(font_upem.len(), 0);
	}
	let font_cmap_off = nonnegative("font_cmap_off", std::mem::take(&mut doc.font_cmap_off))?;
	let font_cmap_len = nonnegative("font_cmap_len", std::mem::take(&mut doc.font_cmap_len))?;
	let font_cmap_cp = std::mem::take(&mut doc.font_cmap_cp);
	let font_cmap_gid = u16s("font_cmap_gid", std::mem::take(&mut doc.font_cmap_gid))?;
	let font_adv = u16s("font_adv", std::mem::take(&mut doc.font_adv))?;
	if font_cmap_cp.len() != font_cmap_gid.len() {
		return Err("font cmap: parallel arrays have mismatched lengths".into());
	}
	let font_count = same_len("font", &[
		font_family.len(),
		font_class.len(),
		font_weight.len(),
		font_upem.len(),
		font_ascent.len(),
		font_descent.len(),
		font_line_gap.len(),
		font_default_adv.len(),
		font_underline_position.len(),
		font_underline_thickness.len(),
		font_cmap_off.len(),
		font_cmap_len.len(),
		font_data_off.len(),
		font_data_len.len(),
	])?;
	let mut fonts = Vec::with_capacity(font_count);
	for index in 0..font_count {
		let cmap_range =
			index_range(font_cmap_off[index], font_cmap_len[index], font_cmap_cp.len(), "font cmap")?;
		let advance_range =
			index_range(font_cmap_off[index], font_cmap_len[index], font_adv.len(), "font advances")?;
		let data_range =
			index_range(font_data_off[index], font_data_len[index], font_data.len(), "font data")?;
		fonts.push(FontE {
			family:              font_family[index],
			class:               font_class[index],
			weight:              font_weight[index],
			upem:                font_upem[index],
			ascent:              font_ascent[index],
			descent:             font_descent[index],
			line_gap:            font_line_gap[index],
			default_advance:     font_default_adv[index],
			underline_position:  font_underline_position[index],
			underline_thickness: font_underline_thickness[index],
			data:                font_data[data_range].to_vec(),
			cmap:                font_cmap_cp[cmap_range]
				.iter()
				.copied()
				.zip(
					font_cmap_gid[index_range(
						font_cmap_off[index],
						font_cmap_len[index],
						font_cmap_gid.len(),
						"font cmap",
					)?]
					.iter()
					.copied(),
				)
				.collect(),
			advances:            font_adv[advance_range].to_vec(),
		});
	}
	slir.fonts = fonts;

	let cond_kind = u8s("cond_kind", std::mem::take(&mut doc.cond_kind))?;
	let cond_neg = u8s("cond_neg", std::mem::take(&mut doc.cond_neg))?;
	let cond_op = u8s("cond_op", std::mem::take(&mut doc.cond_op))?;
	let cond_num = std::mem::take(&mut doc.cond_num);
	let cond_sym = std::mem::take(&mut doc.cond_sym);
	let condition_count = same_len("condition", &[
		cond_kind.len(),
		cond_neg.len(),
		cond_op.len(),
		cond_num.len(),
		cond_sym.len(),
	])?;
	let mut conditions = Vec::with_capacity(condition_count);
	for index in 0..condition_count {
		conditions.push(CondE {
			kind: cond_kind[index],
			neg:  cond_neg[index],
			op:   cond_op[index],
			num:  cond_num[index],
			sym:  cond_sym[index],
		});
	}
	slir.conds = conditions;

	let patch_node = std::mem::take(&mut doc.patch_node);
	let patch_cond = std::mem::take(&mut doc.patch_cond);
	let patch_attr_off = nonnegative("patch_attr_off", std::mem::take(&mut doc.patch_attr_off))?;
	let patch_attr_len = nonnegative("patch_attr_len", std::mem::take(&mut doc.patch_attr_len))?;
	let patch_child_off = nonnegative("patch_child_off", std::mem::take(&mut doc.patch_child_off))?;
	let patch_child_len = nonnegative("patch_child_len", std::mem::take(&mut doc.patch_child_len))?;
	let patch_count = same_len("patch", &[
		patch_node.len(),
		patch_cond.len(),
		patch_attr_off.len(),
		patch_attr_len.len(),
		patch_child_off.len(),
		patch_child_len.len(),
	])?;
	let mut patches = Vec::with_capacity(patch_count);
	for index in 0..patch_count {
		patches.push(PatchE {
			node:      patch_node[index],
			cond:      patch_cond[index],
			attr_off:  patch_attr_off[index],
			attr_len:  patch_attr_len[index],
			child_off: patch_child_off[index],
			child_len: patch_child_len[index],
		});
	}
	slir.patches = patches;
	slir.patch_attrs = pairs(
		"when attr",
		u16s("wattr_id", std::mem::take(&mut doc.wattr_id))?,
		std::mem::take(&mut doc.wattr_val),
	)?;
	slir.patch_children = std::mem::take(&mut doc.patch_children);

	let anim_name = std::mem::take(&mut doc.anim_name);
	let anim_stop_off = nonnegative("anim_stop_off", std::mem::take(&mut doc.anim_stop_off))?;
	let anim_stop_len = nonnegative("anim_stop_len", std::mem::take(&mut doc.anim_stop_len))?;
	let anim_stop_pos = std::mem::take(&mut doc.anim_stop_pos);
	let anim_stop_attr_off =
		nonnegative("anim_stop_attr_off", std::mem::take(&mut doc.anim_stop_attr_off))?;
	let anim_stop_attr_len =
		nonnegative("anim_stop_attr_len", std::mem::take(&mut doc.anim_stop_attr_len))?;
	let stop_count = same_len("animation stop", &[
		anim_stop_pos.len(),
		anim_stop_attr_off.len(),
		anim_stop_attr_len.len(),
	])?;
	let mut animation_stops = Vec::with_capacity(stop_count);
	for index in 0..stop_count {
		animation_stops.push((
			anim_stop_pos[index],
			anim_stop_attr_off[index],
			anim_stop_attr_len[index],
		));
	}
	let animation_count =
		same_len("animation", &[anim_name.len(), anim_stop_off.len(), anim_stop_len.len()])?;
	let mut animations = Vec::with_capacity(animation_count);
	for index in 0..animation_count {
		animations.push(AnimE {
			name:  anim_name[index],
			stops: run(
				&animation_stops,
				anim_stop_off[index],
				anim_stop_len[index],
				"animation stop",
			)?
			.to_vec(),
		});
	}
	slir.anims = animations;
	slir.anim_attrs = pairs(
		"animation attr",
		u16s("aattr_id", std::mem::take(&mut doc.aattr_id))?,
		std::mem::take(&mut doc.aattr_val),
	)?;

	let bind_node = std::mem::take(&mut doc.bind_node);
	let bind_anim = std::mem::take(&mut doc.bind_anim);
	let bind_dur = std::mem::take(&mut doc.bind_dur);
	let bind_mode = u8s("bind_mode", std::mem::take(&mut doc.bind_mode))?;
	let bind_easing = u8s("bind_easing", std::mem::take(&mut doc.bind_easing))?;
	let bind_delay = std::mem::take(&mut doc.bind_delay);
	let binding_count = same_len("binding", &[
		bind_node.len(),
		bind_anim.len(),
		bind_dur.len(),
		bind_mode.len(),
		bind_easing.len(),
		bind_delay.len(),
	])?;
	let mut bindings = Vec::with_capacity(binding_count);
	for index in 0..binding_count {
		bindings.push(BindE {
			node:   bind_node[index],
			anim:   bind_anim[index],
			dur:    bind_dur[index],
			mode:   bind_mode[index],
			easing: bind_easing[index],
			delay:  bind_delay[index],
		});
	}
	slir.bindings = bindings;

	let trans_node = std::mem::take(&mut doc.trans_node);
	let trans_easing = u8s("trans_easing", std::mem::take(&mut doc.trans_easing))?;
	let trans_dur = std::mem::take(&mut doc.trans_dur);
	let trans_delay = std::mem::take(&mut doc.trans_delay);
	let transition_count = same_len("transition", &[
		trans_node.len(),
		trans_easing.len(),
		trans_dur.len(),
		trans_delay.len(),
	])?;
	let mut transitions = Vec::with_capacity(transition_count);
	for index in 0..transition_count {
		transitions.push(TransE {
			node:   trans_node[index],
			easing: trans_easing[index],
			dur:    trans_dur[index],
			delay:  trans_delay[index],
		});
	}
	slir.transitions = transitions;

	let parm_name = std::mem::take(&mut doc.parm_name);
	let parm_type = u8s("parm_type", std::mem::take(&mut doc.parm_type))?;
	let parm_default = std::mem::take(&mut doc.parm_default);
	let parm_enum_off = nonnegative("parm_enum_off", std::mem::take(&mut doc.parm_enum_off))?;
	let parm_enum_len = nonnegative("parm_enum_len", std::mem::take(&mut doc.parm_enum_len))?;
	let parm_site_off = nonnegative("parm_site_off", std::mem::take(&mut doc.parm_site_off))?;
	let parm_site_len = nonnegative("parm_site_len", std::mem::take(&mut doc.parm_site_len))?;
	let parameter_count = same_len("parameter", &[
		parm_name.len(),
		parm_type.len(),
		parm_default.len(),
		parm_enum_off.len(),
		parm_enum_len.len(),
		parm_site_off.len(),
		parm_site_len.len(),
	])?;
	let mut parameters = Vec::with_capacity(parameter_count);
	for index in 0..parameter_count {
		parameters.push(ParamE {
			name:     parm_name[index],
			ty:       parm_type[index],
			default:  parm_default[index],
			enum_off: parm_enum_off[index],
			enum_len: parm_enum_len[index],
			site_off: parm_site_off[index],
			site_len: parm_site_len[index],
		});
	}
	slir.params = parameters;
	slir.param_enum_syms = std::mem::take(&mut doc.parm_enum_syms);
	slir.param_sites = pairs(
		"parameter site",
		std::mem::take(&mut doc.parm_site_node),
		u16s("parm_site_attr", std::mem::take(&mut doc.parm_site_attr))?,
	)?;

	let list_param = std::mem::take(&mut doc.list_param);
	let list_field_off = nonnegative("list_field_off", std::mem::take(&mut doc.list_field_off))?;
	let list_field_len = nonnegative("list_field_len", std::mem::take(&mut doc.list_field_len))?;
	let list_count =
		same_len("list", &[list_param.len(), list_field_off.len(), list_field_len.len()])?;
	let mut lists = Vec::with_capacity(list_count);
	for index in 0..list_count {
		lists.push(ListE {
			param:     list_param[index],
			field_off: list_field_off[index],
			field_len: list_field_len[index],
		});
	}
	slir.lists = lists;

	let list_field_name = std::mem::take(&mut doc.list_field_name);
	let list_field_type = u8s("list_field_type", std::mem::take(&mut doc.list_field_type))?;
	let list_field_default = std::mem::take(&mut doc.list_field_default);
	let list_field_enum_off =
		nonnegative("list_field_enum_off", std::mem::take(&mut doc.list_field_enum_off))?;
	let list_field_enum_len =
		nonnegative("list_field_enum_len", std::mem::take(&mut doc.list_field_enum_len))?;
	let mut list_field_sub = std::mem::take(&mut doc.list_field_sub);
	if list_field_sub.is_empty() {
		list_field_sub.resize(list_field_name.len(), 0);
	}
	let field_count = same_len("list field", &[
		list_field_name.len(),
		list_field_type.len(),
		list_field_default.len(),
		list_field_enum_off.len(),
		list_field_enum_len.len(),
		list_field_sub.len(),
	])?;
	let mut list_fields = Vec::with_capacity(field_count);
	for index in 0..field_count {
		list_fields.push(ListFieldE {
			name:     list_field_name[index],
			ty:       list_field_type[index],
			default:  list_field_default[index],
			enum_off: list_field_enum_off[index],
			enum_len: list_field_enum_len[index],
			sub:      list_field_sub[index],
		});
	}
	slir.list_fields = list_fields;
	for (index, &sub) in list_field_sub.iter().enumerate() {
		let is_list = list_field_type[index] == 6;
		if is_list != (sub != 0) {
			return Err(format!("list_field_sub: field {index} type/sub-schema mismatch"));
		}
		if sub != 0 && usize::try_from(sub - 1).map_or(true, |row| row >= list_count) {
			return Err(format!("list_field_sub: schema row {} is out of range", sub - 1));
		}
	}
	slir.list_enum_syms = std::mem::take(&mut doc.list_enum_syms);

	let list_item_field_off =
		nonnegative("list_item_field_off", std::mem::take(&mut doc.list_item_field_off))?;
	let list_item_field_len =
		nonnegative("list_item_field_len", std::mem::take(&mut doc.list_item_field_len))?;
	let item_count = same_len("list item", &[list_item_field_off.len(), list_item_field_len.len()])?;
	let mut list_items = Vec::with_capacity(item_count);
	for index in 0..item_count {
		list_items.push(ListItemE {
			field_off: list_item_field_off[index],
			field_len: list_item_field_len[index],
		});
	}
	slir.list_items = list_items;
	slir.list_item_values = pairs(
		"list item value",
		std::mem::take(&mut doc.list_item_value_field),
		std::mem::take(&mut doc.list_item_value_val),
	)?
	.into_iter()
	.map(|(field, val)| ListItemValueE { field, val })
	.collect();

	slir.themes = std::mem::take(&mut doc.theme_name);
	let token_name = std::mem::take(&mut doc.token_name);
	let token_base = std::mem::take(&mut doc.token_base);
	let token_base_repr = std::mem::take(&mut doc.token_base_repr);
	let token_theme_off = nonnegative("token_theme_off", std::mem::take(&mut doc.token_theme_off))?;
	let token_theme_len = nonnegative("token_theme_len", std::mem::take(&mut doc.token_theme_len))?;
	let token_theme_name = std::mem::take(&mut doc.token_theme_name);
	let token_theme_val = std::mem::take(&mut doc.token_theme_val);
	let token_theme_repr = std::mem::take(&mut doc.token_theme_repr);
	let token_count = same_len("token", &[
		token_name.len(),
		token_base.len(),
		token_base_repr.len(),
		token_theme_off.len(),
		token_theme_len.len(),
	])?;
	same_len("token theme", &[
		token_theme_name.len(),
		token_theme_val.len(),
		token_theme_repr.len(),
	])?;
	let mut tokens = Vec::with_capacity(token_count);
	for index in 0..token_count {
		let start =
			usize::try_from(token_theme_off[index]).map_err(|_| "token theme offset exceeds usize")?;
		let len =
			usize::try_from(token_theme_len[index]).map_err(|_| "token theme length exceeds usize")?;
		let end = start
			.checked_add(len)
			.ok_or("token theme range overflows usize")?;
		if end > token_theme_name.len() {
			return Err(format!("token theme range {start}..{end} is out of bounds"));
		}
		for (field, value, bound) in [
			("token_name", token_name[index], slir.strs.len()),
			("token_base", token_base[index], slir.avals.len()),
			("token_base_repr", token_base_repr[index], slir.strs.len()),
		] {
			if usize::try_from(value).map_or(true, |value| value >= bound) {
				return Err(format!("{field}: index {value} is out of range"));
			}
		}
		for theme in start..end {
			for (field, value, bound) in [
				("token_theme_name", token_theme_name[theme], slir.strs.len()),
				("token_theme_val", token_theme_val[theme], slir.avals.len()),
				("token_theme_repr", token_theme_repr[theme], slir.strs.len()),
			] {
				if usize::try_from(value).map_or(true, |value| value >= bound) {
					return Err(format!("{field}: index {value} is out of range"));
				}
			}
		}
		let themes = (start..end)
			.map(|theme| (token_theme_name[theme], token_theme_val[theme], token_theme_repr[theme]))
			.collect();
		tokens.push(TokenE {
			name: token_name[index],
			base: token_base[index],
			base_repr: token_base_repr[index],
			themes,
		});
	}
	slir.tokens = tokens;
	slir.holes =
		pairs("hole", std::mem::take(&mut doc.hole_name), std::mem::take(&mut doc.hole_node))?;

	let sign_name = std::mem::take(&mut doc.sign_name);
	let sign_node = std::mem::take(&mut doc.sign_node);
	let sign_trigger = u8s("sign_trigger", std::mem::take(&mut doc.sign_trigger))?;
	let signal_count = same_len("signal", &[sign_name.len(), sign_node.len(), sign_trigger.len()])?;
	let mut signals = Vec::with_capacity(signal_count);
	for index in 0..signal_count {
		signals.push((sign_name[index], sign_node[index], sign_trigger[index]));
	}
	slir.signals = signals;

	let icon_name = std::mem::take(&mut doc.icon_name);
	let icon_node = std::mem::take(&mut doc.icon_node);
	let mut icon_viewbox = std::mem::take(&mut doc.icon_viewbox);
	if icon_viewbox.is_empty() {
		icon_viewbox.resize(icon_name.len(), 24.0);
	}
	let icon_count = same_len("icon", &[icon_name.len(), icon_node.len(), icon_viewbox.len()])?;
	let mut icons = Vec::with_capacity(icon_count);
	for index in 0..icon_count {
		let name =
			usize::try_from(icon_name[index]).map_err(|_| "icon_name: string index exceeds usize")?;
		if name >= slir.strs.len() {
			return Err(format!("icon_name: string index {name} is out of range"));
		}
		let node =
			usize::try_from(icon_node[index]).map_err(|_| "icon_node: node index exceeds usize")?;
		if node >= slir.nodes.len() {
			return Err(format!("icon_node: node {} is out of range", icon_node[index]));
		}
		if slir.nodes.flags[node] & crate::flags::DETACHED == 0 {
			return Err(format!("icon_node: node {} is not detached", icon_node[index]));
		}
		icons.push(IconE {
			name:    icon_name[index],
			node:    icon_node[index],
			viewbox: icon_viewbox[index],
		});
	}
	slir.icons = icons;

	let img_src = std::mem::take(&mut doc.img_src);
	let img_w = std::mem::take(&mut doc.img_w);
	let img_h = std::mem::take(&mut doc.img_h);
	let img_format = u8s("img_format", std::mem::take(&mut doc.img_format))?;
	let img_data = std::mem::take(&mut doc.img_data);
	let image_count = same_len("image", &[
		img_src.len(),
		img_w.len(),
		img_h.len(),
		img_format.len(),
		img_data.len(),
	])?;
	let mut images = Vec::with_capacity(image_count);
	let mut blob = Vec::new();
	for index in 0..image_count {
		let blob_off = u32::try_from(blob.len()).map_err(|_| "image blob exceeds u32")?;
		let blob_len = u32::try_from(img_data[index].len()).map_err(|_| "image exceeds u32")?;
		blob.extend_from_slice(&img_data[index]);
		images.push(ImgE {
			src: img_src[index],
			w: img_w[index],
			h: img_h[index],
			format: img_format[index],
			blob_off,
			blob_len,
		});
	}
	slir.images = images;
	slir.blob = blob;

	Ok(slir)
}

/// Decode SLIR bytes. Fails on unsupported versions, bad magic, decompression,
/// protobuf, or representation errors.
pub fn read(bytes: &[u8]) -> Result<Slir, String> {
	from_pb(decode_pb(bytes)?)
}

#[cfg(test)]
mod tests {
	use super::{from_pb, read};
	use crate::{
		AnimE, Aval, BindE, CondE, FontE, GradE, ImgE, ListE, ListFieldE, ListItemE, ListItemValueE,
		Nodes, ParamE, PatchE, PathE, ShadowE, Slir, TokenE, TransE, TupDynE, aval, pb, write,
	};

	fn list_field_doc(ty: u32, sub: Option<u32>) -> pb::Doc {
		let mut doc = pb::Doc {
			strs: vec![String::new()],
			list_field_name: vec![0],
			list_field_type: vec![ty],
			list_field_default: vec![0],
			list_field_enum_off: vec![0],
			list_field_enum_len: vec![0],
			..pb::Doc::default()
		};
		if let Some(sub) = sub {
			doc.list_field_sub.push(sub);
		}
		doc
	}

	#[test]
	fn list_field_sub_matches_the_field_type() {
		assert!(from_pb(list_field_doc(0, None)).is_ok());
		assert!(
			from_pb(list_field_doc(6, Some(0)))
				.expect_err("list field without a sub-schema must fail")
				.contains("type/sub-schema mismatch")
		);

		let mut scalar_with_sub = list_field_doc(0, Some(1));
		scalar_with_sub.list_param.push(crate::NONE);
		scalar_with_sub.list_field_off.push(0);
		scalar_with_sub.list_field_len.push(0);
		assert!(
			from_pb(scalar_with_sub)
				.expect_err("scalar field with a sub-schema must fail")
				.contains("type/sub-schema mismatch")
		);
	}

	#[test]
	fn icon_names_must_reference_the_string_pool() {
		let doc = pb::Doc {
			strs: vec![String::new()],
			node_kind: vec![10],
			node_flags: vec![u32::from(crate::flags::DETACHED)],
			node_parent: vec![0],
			node_first: vec![0],
			node_next: vec![0],
			node_key: vec![0],
			node_id: vec![0],
			node_line: vec![1],
			icon_name: vec![1],
			icon_node: vec![0],
			icon_viewbox: vec![24.0],
			..pb::Doc::default()
		};
		assert!(
			from_pb(doc)
				.expect_err("out-of-range icon name must fail")
				.contains("icon_name: string index 1 is out of range")
		);
	}

	#[test]
	fn protobuf_snappy_round_trip_preserves_every_pool() {
		let slir = Slir {
			strs:             vec![String::new(), "name".into()],
			nodes:            Nodes {
				kind:        vec![8],
				flags:       vec![3],
				parent:      vec![crate::NONE],
				first_child: vec![crate::NONE],
				next_sib:    vec![crate::NONE],
				key:         vec![1],
				id:          vec![1],
				src_line:    vec![7],
			},
			avals:            vec![Aval { tag: aval::NUM, payload: Aval::f64_payload(42.5) }, Aval {
				tag:     aval::STR,
				payload: Aval::pair(1, 0),
			}],
			f64s:             vec![1.5],
			tup_dyn:          vec![TupDynE::Lit(2.5), TupDynE::Param(0)],
			grads:            vec![GradE { kind: 0, angle: 90.0, stops: vec![(0.0, 0xff00_00ff)] }],
			shadows:          vec![ShadowE {
				x:      1.0,
				y:      2.0,
				blur:   3.0,
				spread: 4.0,
				rgba:   0x0102_03ff,
				inset:  1,
			}],
			attr_index:       vec![0, 1],
			attrs:            vec![(27, 1)],
			paths:            vec![PathE { verbs: vec![0, 4], coords: vec![1.0, 2.0] }],
			fonts:            vec![FontE {
				family:              0,
				class:               0,
				weight:              400,
				upem:                1000,
				ascent:              800,
				descent:             -200,
				line_gap:            20,
				default_advance:     600,
				underline_position:  -100,
				underline_thickness: 50,
				data:                vec![0, 1, 2, 3],
				cmap:                vec![(65, 3)],
				advances:            vec![600],
			}],
			conds:            vec![CondE { kind: 3, neg: 1, op: 2, num: 20.0, sym: 1 }],
			patches:          vec![PatchE {
				node:      0,
				cond:      0,
				attr_off:  0,
				attr_len:  1,
				child_off: 0,
				child_len: 1,
			}],
			patch_attrs:      vec![(1, 0)],
			patch_children:   vec![0],
			anims:            vec![AnimE { name: 1, stops: vec![(0.5, 0, 1)] }],
			anim_attrs:       vec![(2, 1)],
			bindings:         vec![BindE {
				node:   0,
				anim:   0,
				dur:    250.0,
				mode:   1,
				easing: 2,
				delay:  10.0,
			}],
			transitions:      vec![TransE { node: 0, easing: 3, dur: 125.0, delay: 5.0 }],
			params:           vec![ParamE {
				name:     1,
				ty:       5,
				default:  1,
				enum_off: 0,
				enum_len: 1,
				site_off: 0,
				site_len: 1,
			}],
			param_enum_syms:  vec![1],
			param_sites:      vec![(0, 27)],
			lists:            vec![ListE { param: 0, field_off: 0, field_len: 1 }],
			list_fields:      vec![ListFieldE {
				name:     1,
				ty:       5,
				default:  1,
				enum_off: 0,
				enum_len: 1,
				sub:      0,
			}],
			list_enum_syms:   vec![1],
			list_items:       vec![ListItemE { field_off: 0, field_len: 1 }],
			list_item_values: vec![ListItemValueE { field: 0, val: 1 }],
			themes:           vec![1],
			tokens:           vec![TokenE {
				name:      1,
				base:      0,
				base_repr: 1,
				themes:    vec![(1, 1, 1)],
			}],
			holes:            vec![(1, 0)],
			signals:          vec![(1, 0, 2)],
			images:           vec![ImgE {
				src:      1,
				w:        2,
				h:        3,
				format:   0,
				blob_off: 0,
				blob_len: 3,
			}],
			icons:            vec![],
			blob:             vec![1, 2, 3],
		};

		let bytes = write(&slir);
		assert_eq!(&bytes[..4], b"SLIR");
		assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), crate::MAJOR);
		assert_eq!(read(&bytes).expect("round trip must decode"), slir);
	}
}
