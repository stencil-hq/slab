//! SLIR document storage and font metric selection.
//!
//! The decoder constructs a flat structure-of-arrays document from the wire
//! format; the helpers in this module query its attributes and font metrics.
/// Node-kind values stored in [`Doc::node_kind`].
pub const K_ROW: u32 = 0u32;

pub const K_COL: u32 = 1u32;

pub const K_WRAP: u32 = 2u32;

pub const K_GRID: u32 = 3u32;

pub const K_STACK: u32 = 4u32;

pub const K_CANVAS: u32 = 5u32;

pub const K_PARA: u32 = 6u32;

pub const K_GROUP: u32 = 7u32;

pub const K_TEXT: u32 = 8u32;

pub const K_SPAN: u32 = 9u32;

pub const K_RECT: u32 = 10u32;

pub const K_IMG: u32 = 11u32;

pub const K_PATH: u32 = 12u32;

pub const K_SPACER: u32 = 13u32;

pub const K_HOLE: u32 = 14u32;

pub const K_EACH: u32 = 15u32;
pub const K_DIVIDER: u32 = 16u32;

pub const K_ICON: u32 = 17u32;

/// Node flag bits stored in [`Doc::node_flags`] and used by `flags` patches.
pub const F_CLIP: u32 = 1u32;

pub const F_BLEED: u32 = 2u32;

pub const F_SCROLL: u32 = 4u32;

pub const F_NOWRAP: u32 = 8u32;

pub const F_ELLIPSIS: u32 = 16u32;

pub const F_INERT: u32 = 32u32;

pub const F_FOCUSABLE: u32 = 64u32;

pub const F_DETACHED: u32 = 128u32;

pub const F_MULTILINE: u32 = 256u32;
pub const F_SCROLL_CROSS: u32 = 512u32;

pub const F_VIRTUAL: u32 = 1024u32;

pub const F_STICKY: u32 = 2048u32;
pub const F_DRAG_GHOST: u32 = 4096u32;
/// Escape clears focus on an editable node instead of bubbling.
pub const F_ESCAPE_BLUR: u32 = 8192u32;
/// Enables kernel-owned pointer selection for static descendant text.
pub const F_SELECT: u32 = 16384u32;
/// Enables kernel-owned proportional split-pane layout and synthetic sashes.
pub const F_SPLITS: u32 = 32768u32;

/// Attribute-value tags stored in [`Doc::aval_tag`].
pub const T_NUM: u32 = 0u32;

pub const T_PCT: u32 = 1u32;

pub const T_STR: u32 = 2u32;

pub const T_COLOR: u32 = 3u32;

pub const T_TUPLE: u32 = 4u32;

pub const T_SIZE_FIXED: u32 = 5u32;

pub const T_SIZE_HUG: u32 = 6u32;

pub const T_SIZE_FILL: u32 = 7u32;

pub const T_SIZE_PCT: u32 = 8u32;

pub const T_PAINT_SOLID: u32 = 9u32;

pub const T_PAINT_GRADIENT: u32 = 10u32;

pub const T_PATH_REF: u32 = 11u32;

pub const T_SHADOW_LIST: u32 = 12u32;

pub const T_PARAM_REF: u32 = 13u32;

pub const T_ENUM_SYM: u32 = 14u32;

pub const T_PAINT_NONE: u32 = 15u32;

pub const T_PROP_REF: u32 = 16u32;

pub const T_LIST_DEFAULT: u32 = 17u32;

pub const T_TUPLE_DYN: u32 = 18u32;
pub const T_PAINT_CURRENT: u32 = 19u32;
pub const T_TOKEN_REF: u32 = 20u32;

/// Condition kinds stored in [`Doc::cond_kind`].
pub const C_STATE: u32 = 0u32;

pub const C_ENV: u32 = 1u32;

pub const C_CLIENT: u32 = 2u32;

pub const C_WCMP: u32 = 3u32;

pub const C_HCMP: u32 = 4u32;

pub const C_PROP: u32 = 5u32;

pub const C_THEME: u32 = 6u32;

/// Attribute identifiers from the normative SLIR attribute table.
pub const A_W: u32 = 0u32;

pub const A_H: u32 = 1u32;

pub const A_MIN_W: u32 = 2u32;

pub const A_MAX_W: u32 = 3u32;

pub const A_MIN_H: u32 = 4u32;

pub const A_MAX_H: u32 = 5u32;

pub const A_PAD: u32 = 6u32;

pub const A_GAP: u32 = 7u32;

pub const A_AXIS: u32 = 8u32;

pub const A_PACK: u32 = 9u32;

pub const A_ALIGN: u32 = 10u32;

pub const A_SELF: u32 = 11u32;

pub const A_OFFSET: u32 = 12u32;

pub const A_AT: u32 = 13u32;

pub const A_ANCHOR: u32 = 14u32;

pub const A_BG: u32 = 15u32;

pub const A_STROKE: u32 = 16u32;

pub const A_STROKE_W: u32 = 17u32;

pub const A_STROKE_ALIGN: u32 = 18u32;

pub const A_STROKE_SIDES: u32 = 19u32;

pub const A_STROKE_DASH: u32 = 20u32;

pub const A_RADIUS: u32 = 21u32;

pub const A_SHADOW: u32 = 22u32;

pub const A_BLUR: u32 = 23u32;

pub const A_BACKDROP: u32 = 24u32;

pub const A_OPACITY: u32 = 25u32;

pub const A_COLOR: u32 = 26u32;

pub const A_FAMILY: u32 = 27u32;

pub const A_SIZE: u32 = 28u32;

pub const A_WEIGHT: u32 = 29u32;

pub const A_LEADING: u32 = 30u32;

pub const A_TRACKING: u32 = 31u32;

pub const A_ROTATE: u32 = 32u32;

pub const A_ALIGN_TEXT: u32 = 33u32;

pub const A_FIT: u32 = 34u32;

pub const A_SRC: u32 = 35u32;

pub const A_D: u32 = 36u32;

pub const A_COLS: u32 = 37u32;

pub const A_SPAN: u32 = 38u32;

pub const A_CONTENT: u32 = 39u32;

pub const A_FLAGS: u32 = 40u32;

pub const A_ACT: u32 = 41u32;

pub const A_FIELD: u32 = 42u32;

pub const A_EACH: u32 = 43u32;

pub const A_KEYS: u32 = 44u32;

pub const A_SCROLLBAR: u32 = 45u32;

pub const A_SCROLLBAR_W: u32 = 46u32;

pub const A_SCROLLBAR_FG: u32 = 47u32;

pub const A_SCROLLBAR_BG: u32 = 48u32;

pub const A_SUBMIT: u32 = 49u32;

pub const A_ITEM_EXTENT: u32 = 50u32;

pub const A_OVERSCAN: u32 = 51u32;

pub const A_ATTACH: u32 = 52u32;

pub const A_GRAVITY: u32 = 53u32;

pub const A_COLLIDE: u32 = 54u32;

pub const A_PRESS: u32 = 55u32;

pub const A_CONTEXT: u32 = 56u32;

pub const A_DBLCLICK: u32 = 57u32;

pub const A_DRAG: u32 = 58u32;

pub const A_DROP: u32 = 59u32;

pub const A_RESIZE: u32 = 60u32;

pub const A_ROLE: u32 = 61u32;

pub const A_LABEL: u32 = 62u32;

pub const A_DESC: u32 = 63u32;

pub const A_SCALE: u32 = 64u32;

pub const A_SMOOTH: u32 = 65u32;

pub const A_GRAIN: u32 = 66u32;

pub const A_MASK: u32 = 67u32;

pub const A_BACKDROP_MASK: u32 = 68u32;

pub const A_TILT: u32 = 69u32;
pub const A_POINTER_MOVE: u32 = 70u32;

pub const A_POINTER_UP: u32 = 71u32;

pub const A_DRAG_UPDATE: u32 = 72u32;

pub const A_DRAG_END: u32 = 73u32;

pub const A_CHECKED: u32 = 74u32;

pub const A_EXPANDED: u32 = 75u32;

pub const A_SELECTED: u32 = 76u32;

pub const A_ACTIVE_DESCENDANT: u32 = 77u32;

pub const A_CONTROLS: u32 = 78u32;

pub const A_VALUE_NOW: u32 = 79u32;

pub const A_VALUE_MIN: u32 = 80u32;

pub const A_VALUE_MAX: u32 = 81u32;

pub const A_VALUE_TEXT: u32 = 82u32;

pub const A_MODAL: u32 = 83u32;

pub const A_LIVE: u32 = 84u32;

pub const A_LIVE_ATOMIC: u32 = 85u32;

pub const A_LEVEL: u32 = 86u32;

pub const A_POS_IN_SET: u32 = 87u32;

pub const A_SET_SIZE: u32 = 88u32;

/// Conditional animation binding channel.
pub const A_ANIMATE: u32 = 89u32;

/// Inherited boolean text strike-through style.
pub const A_STRIKE: u32 = 90u32;

/// Inherited boolean italic text style.
pub const A_ITALIC: u32 = 92u32;

/// Inherited boolean underline text decoration.
pub const A_UNDERLINE: u32 = 93u32;

/// Optional rich inline-code text paint.
pub const A_CODE_COLOR: u32 = 94u32;

/// Optional rich inline-code background paint.
pub const A_CODE_BG: u32 = 95u32;

/// Top padding override (applied after `pad`).
pub const A_PAD_T: u32 = 96u32;
/// Right padding override (applied after `pad`).
pub const A_PAD_R: u32 = 97u32;
/// Bottom padding override (applied after `pad`).
pub const A_PAD_B: u32 = 98u32;
/// Left padding override (applied after `pad`).
pub const A_PAD_L: u32 = 99u32;

/// Selection highlight paint for a `select` root.
pub const A_SELECT_BG: u32 = 100u32;
/// Split sash pointer hit and active-paint thickness.
pub const A_SPLIT_W: u32 = 101u32;
/// Split sash paint while hovered or pressed.
pub const A_SPLIT_FG: u32 = 102u32;
/// Number of spaces inserted by plain Tab in an opted-in multiline field.
pub const A_TAB_SIZE: u32 = 103u32;

/// Field cancel binder channel fired on escape-blur.
pub const A_CANCEL: u32 = 91u32;

/// Total number of normative SLIR attributes (highest attribute ID + 1).
pub const ATTR_COUNT: usize = (A_TAB_SIZE as usize) + 1;

/// Parameter types stored in [`Doc::parm_type`] and host parameter values.
pub const PARAM_TEXT: u32 = 0u32;

pub const PARAM_NUM: u32 = 1u32;

pub const PARAM_PCT: u32 = 2u32;

pub const PARAM_COLOR: u32 = 3u32;

pub const PARAM_BOOL: u32 = 4u32;

pub const PARAM_ENUM: u32 = 5u32;

pub const PARAM_LIST: u32 = 6u32;

/// Sentinel representing the absence of a node link.
pub const NONE: u32 = 0xffffffffu32;

#[derive(Clone, Debug, Default)]
/// A fully decoded SLIR document.
///
/// Pool slices use `(offset, length)` fenceposts into parallel arrays.
/// `attr_index` instead contains `node_count + 1` fenceposts.
pub struct Doc {
	pub ok: bool,
	pub errs: Vec<String>,
	// STRS pool.
	pub strs: Vec<String>,
	// NODE pool (indices are node IDs).
	pub node_kind: Vec<u32>,
	pub node_flags: Vec<u32>,
	pub node_parent: Vec<u32>,
	pub node_first: Vec<u32>,
	pub node_next: Vec<u32>,
	pub node_key: Vec<u32>,
	pub node_id: Vec<u32>,
	pub node_line: Vec<u32>,
	// AVAL pool.
	pub aval_tag: Vec<u32>,
	pub aval_lo: Vec<u32>,
	pub aval_hi: Vec<u32>,
	pub aval_num: Vec<f64>,
	pub f64s: Vec<f64>,
	// TUPLE_DYN member pool: tag 0 = literal (`tup_dyn_num`), 1 = num/pct
	// param reference (`tup_dyn_param`).
	pub tup_dyn_tag: Vec<u32>,
	pub tup_dyn_num: Vec<f64>,
	pub tup_dyn_param: Vec<u32>,
	// GRAD pool.
	pub grad_kind: Vec<u32>,
	pub grad_angle: Vec<f64>,
	pub grad_stop_off: Vec<i32>,
	pub grad_stop_len: Vec<i32>,
	pub grad_stop_pos: Vec<f64>,
	pub grad_stop_rgba: Vec<u32>,
	// SHDW pool.
	pub shdw_x: Vec<f64>,
	pub shdw_y: Vec<f64>,
	pub shdw_blur: Vec<f64>,
	pub shdw_spread: Vec<f64>,
	pub shdw_rgba: Vec<u32>,
	pub shdw_inset: Vec<u32>,
	// ATTR pool.
	pub attr_index: Vec<i32>,
	pub attr_id: Vec<u32>,
	pub attr_val: Vec<u32>,
	// PATH pool.
	pub path_verb_off: Vec<i32>,
	pub path_verb_len: Vec<i32>,
	pub path_coord_off: Vec<i32>,
	pub path_coord_len: Vec<i32>,
	pub path_verbs: Vec<u32>,
	pub path_coords: Vec<f64>,
	// FONT pool.
	pub font_family: Vec<u32>,
	pub font_class: Vec<u32>,
	pub font_underline_position: Vec<i32>,
	pub font_underline_thickness: Vec<i32>,
	pub font_weight: Vec<u32>,
	pub font_upem: Vec<u32>,
	pub font_ascent: Vec<i32>,
	pub font_descent: Vec<i32>,
	pub font_line_gap: Vec<i32>,
	pub font_default_adv: Vec<u32>,
	pub font_cmap_off: Vec<i32>,
	pub font_cmap_len: Vec<i32>,
	pub font_data_off: Vec<i32>,
	pub font_data_len: Vec<i32>,
	pub font_data: Vec<u8>,
	pub font_cmap_cp: Vec<u32>,
	pub font_cmap_gid: Vec<u32>,
	pub font_adv: Vec<u32>,
	/// FONT tables present at decode. Later tables are runtime-registered:
	/// they never resolve vendored fallback data (see [`face_data`]).
	pub compiled_fonts: usize,
	// WHEN pools.
	pub cond_kind: Vec<u32>,
	pub cond_neg: Vec<u32>,
	pub cond_op: Vec<u32>,
	pub cond_num: Vec<f64>,
	pub cond_sym: Vec<u32>,
	pub patch_node: Vec<u32>,
	pub patch_cond: Vec<u32>,
	pub patch_attr_off: Vec<i32>,
	pub patch_attr_len: Vec<i32>,
	pub patch_child_off: Vec<i32>,
	pub patch_child_len: Vec<i32>,
	pub wattr_id: Vec<u32>,
	pub wattr_val: Vec<u32>,
	pub patch_children: Vec<u32>,
	// ANIM pools.
	pub anim_name: Vec<u32>,
	pub anim_stop_off: Vec<i32>,
	pub anim_stop_len: Vec<i32>,
	pub anim_stop_pos: Vec<f64>,
	pub anim_stop_attr_off: Vec<i32>,
	pub anim_stop_attr_len: Vec<i32>,
	pub aattr_id: Vec<u32>,
	pub aattr_val: Vec<u32>,
	pub bind_node: Vec<u32>,
	pub bind_anim: Vec<u32>,
	pub bind_dur: Vec<f64>,
	pub bind_mode: Vec<u32>,
	pub bind_easing: Vec<u32>,
	pub bind_delay: Vec<f64>,
	pub trans_node: Vec<u32>,
	pub trans_easing: Vec<u32>,
	pub trans_dur: Vec<f64>,
	pub trans_delay: Vec<f64>,
	// PARM pool.
	pub parm_name: Vec<u32>,
	pub parm_type: Vec<u32>,
	pub parm_default: Vec<u32>,
	pub parm_enum_off: Vec<i32>,
	pub parm_enum_len: Vec<i32>,
	pub parm_site_off: Vec<i32>,
	pub parm_site_len: Vec<i32>,
	pub parm_enum_syms: Vec<u32>,
	pub parm_site_node: Vec<u32>,
	pub parm_site_attr: Vec<u32>,
	// LIST pools.
	pub list_param: Vec<u32>,
	pub list_field_off: Vec<i32>,
	pub list_field_len: Vec<i32>,
	pub list_field_name: Vec<u32>,
	pub list_field_type: Vec<u32>,
	pub list_field_default: Vec<u32>,
	/// Zero for scalar fields, otherwise one plus a nested list-schema row.
	pub list_field_sub: Vec<u32>,
	pub list_field_enum_off: Vec<i32>,
	pub list_field_enum_len: Vec<i32>,
	pub list_enum_syms: Vec<u32>,
	pub list_item_field_off: Vec<i32>,
	pub list_item_field_len: Vec<i32>,
	pub list_item_value_field: Vec<u32>,
	pub list_item_value_val: Vec<u32>,
	// THEM pool.
	pub theme_name: Vec<u32>,
	// TOKN pool. Public rows precede typed use-site rows.
	pub token_name: Vec<u32>,
	pub token_base: Vec<u32>,
	pub token_base_repr: Vec<u32>,
	pub token_theme_off: Vec<i32>,
	pub token_theme_len: Vec<i32>,
	pub token_theme_name: Vec<u32>,
	pub token_theme_val: Vec<u32>,
	pub token_theme_repr: Vec<u32>,
	// HOLE pool.
	pub hole_name: Vec<u32>,
	pub hole_node: Vec<u32>,
	// SIGN pool.
	pub sign_name: Vec<u32>,
	pub sign_node: Vec<u32>,
	pub sign_trigger: Vec<u32>,
	// ICON pool.
	pub icon_name: Vec<u32>,
	pub icon_node: Vec<u32>,
	pub icon_viewbox: Vec<f64>,
	// IMGS pool.
	pub img_src: Vec<u32>,
	pub img_w: Vec<u32>,
	pub img_h: Vec<u32>,
	/// Embedded image payloads parallel to `img_src`.
	pub img_data: Vec<Vec<u8>>,
	pub img_format: Vec<u32>,
}

/// Creates an empty, invalid document ready to be populated by a decoder.
pub fn doc_new() -> Doc {
	Doc::default()
}

fn u32_index(value: u32) -> usize {
	usize::try_from(value).expect("SLIR index exceeds usize")
}

fn i32_index(value: i32) -> usize {
	usize::try_from(value).expect("negative SLIR index")
}

const fn signed(value: u32) -> i32 {
	i32::from_ne_bytes(value.to_ne_bytes())
}

fn count(value: usize) -> i32 {
	i32::try_from(value).expect("SLIR table exceeds i32 capacity")
}

// Accessors and font metric selection.
/// Borrows string-pool entry `i`.
pub fn str_ref(d: &Doc, i: u32) -> &str {
	d.strs[u32_index(i)].as_str()
}

/// Borrows string-pool entry `i`.
pub fn str_at(d: &Doc, i: u32) -> &str {
	&d.strs[u32_index(i)]
}

/// Finds an attribute-value index in a node's base attribute run.
///
/// Returns `-1` when `attr` is absent.
pub fn base_attr(d: &Doc, node: u32, attr: u32) -> i32 {
	let lo = d.attr_index[u32_index(node)];
	let hi = d.attr_index[u32_index(node.wrapping_add(1))];
	for index in lo..hi {
		let index = i32_index(index);
		if d.attr_id[index] == attr {
			return signed(d.attr_val[index]);
		}
	}
	-1
}

/// Binary-searches a font's cmap slice.
///
/// Returns the index in the shared cmap arrays, or `-1` when missing.
pub fn font_cmap_ix(d: &Doc, font: i32, codepoint: u32) -> i32 {
	// Fontless documents (host-measured shells, tests) degrade to the
	// deterministic fallback advance instead of panicking — same contract
	// as `textm::char_w`'s upem guard.
	if font < 0 || usize::try_from(font).is_ok_and(|f| f >= d.font_cmap_off.len()) {
		return -1;
	}
	let font = i32_index(font);
	let offset = d.font_cmap_off[font];
	if codepoint <= 0x7f && d.font_cmap_len[font] > 0 {
		let first = d.font_cmap_cp[i32_index(offset)];
		if codepoint >= first {
			let candidate = i32::try_from(codepoint - first).expect("ASCII cmap offset exceeds i32");
			if candidate < d.font_cmap_len[font] {
				let candidate = offset.wrapping_add(candidate);
				if d.font_cmap_cp[i32_index(candidate)] == codepoint {
					return candidate;
				}
			}
		}
	}
	let mut lo = 0;
	let mut hi = d.font_cmap_len[font];
	while lo < hi {
		let mid = lo.wrapping_add(hi.wrapping_sub(lo).wrapping_div(2));
		let value = d.font_cmap_cp[i32_index(offset.wrapping_add(mid))];
		if value == codepoint {
			return offset.wrapping_add(mid);
		}
		if value < codepoint {
			lo = mid.wrapping_add(1);
		} else {
			hi = mid;
		}
	}
	-1
}

/// Returns the glyph ID for a codepoint, or `0` when it is missing.
pub fn font_gid(d: &Doc, font: i32, codepoint: u32) -> u32 {
	let index = font_cmap_ix(d, font, codepoint);
	if index < 0 {
		0
	} else {
		d.font_cmap_gid[i32_index(index)]
	}
}

/// Returns the sfnt bytes for one font table, or an empty slice when absent.
pub fn font_data(d: &Doc, font: i32) -> &[u8] {
	let Ok(font) = usize::try_from(font) else {
		return &[];
	};
	let Some((&offset, &length)) = d.font_data_off.get(font).zip(d.font_data_len.get(font)) else {
		return &[];
	};
	let Ok(start) = usize::try_from(offset) else {
		return &[];
	};
	let Ok(length) = usize::try_from(length) else {
		return &[];
	};
	d.font_data
		.get(start..start.saturating_add(length))
		.unwrap_or(&[])
}
/// Returns the sfnt bytes shaping and paint must use for one font table.
///
/// Embedded data wins: the kernel shaped that table's glyph ids against
/// exactly those bytes. Compiled tables without data resolve the vendored
/// class asset — the compiler stopped embedding bundled faces, so every host
/// resolves the identical vendored bytes instead. Runtime-registered tables
/// without data return empty: the host owns that face and the table's
/// cmap/advances drive layout.
pub fn face_data(d: &Doc, font: i32) -> &[u8] {
	let embedded = font_data(d, font);
	if !embedded.is_empty() {
		return embedded;
	}
	let Ok(index) = usize::try_from(font) else {
		return &[];
	};
	if index >= d.compiled_fonts {
		return &[];
	}
	let (Some(&class), Some(&weight)) = (d.font_class.get(index), d.font_weight.get(index)) else {
		return &[];
	};
	let class = if class == 1 {
		slab_fonts::CLASS_MONO
	} else {
		slab_fonts::CLASS_SANS
	};
	let weight = u16::try_from(weight).unwrap_or(400);
	slab_fonts::asset(class, weight).bytes
}

/// Folds an ASCII uppercase codepoint to lowercase, leaving all others intact.
pub fn ascii_fold(codepoint: u32) -> u32 {
	if (65..=90).contains(&codepoint) {
		codepoint.wrapping_add(32)
	} else {
		codepoint
	}
}

/// Compares authored family names using ASCII-only case folding.
pub fn family_eq(a: &str, b: &str) -> bool {
	a.chars()
		.map(u32::from)
		.map(ascii_fold)
		.eq(b.chars().map(u32::from).map(ascii_fold))
}

/// Classifies a registered family for fallback metrics when no name matches.
pub fn family_class(name: &str) -> u32 {
	let mut previous = [0; 3];
	for (index, codepoint) in name.chars().map(u32::from).map(ascii_fold).enumerate() {
		if index >= 3 && previous == [109, 111, 110] && codepoint == 111 {
			return 1;
		}
		previous.rotate_left(1);
		previous[2] = codepoint;
	}
	0
}

fn nearest_family_font(d: &Doc, family: &str, weight: u32) -> i32 {
	let mut best = -1;
	let mut best_distance = 100_000;
	for (index, (&candidate, &candidate_weight)) in
		d.font_family.iter().zip(&d.font_weight).enumerate()
	{
		if family_eq(&d.strs[u32_index(candidate)], family) {
			let distance = signed(candidate_weight)
				.wrapping_sub(signed(weight))
				.wrapping_abs();
			let index = count(index);
			if distance < best_distance || (distance == best_distance && index > best) {
				best = index;
				best_distance = distance;
			}
		}
	}
	best
}

/// Selects the nearest weight for a runtime-resolved family name, preferring
/// later equal matches and then falling back to the document's default family.
pub fn font_select_name(d: &Doc, family: &str, weight: u32) -> i32 {
	if d.font_family.is_empty() {
		return -1;
	}
	let best = nearest_family_font(d, family, weight);
	if best >= 0 {
		return best;
	}
	let fallback = d.strs.first().map_or("", String::as_str);
	if !family_eq(family, fallback) {
		let best = nearest_family_font(d, fallback, weight);
		if best >= 0 {
			return best;
		}
	}
	0
}

/// Selects the nearest weight for an interned family, preferring later equal
/// matches.
pub fn font_select(d: &Doc, family: u32, weight: u32) -> i32 {
	font_select_name(d, &d.strs[u32_index(family)], weight)
}
