//! `slab-slir` — SLIR binary types, writer, reader, and the canonical
//! `slir-dump` text rendering. The byte layout is normative in `spec/SLIR.md`.

pub mod attrs;
pub mod dump;
pub mod kernel;
mod pb;
pub mod read;
pub mod write;

pub use dump::dump;
pub use kernel::{decode_doc, instance};
pub use read::read;
pub use write::write;

/// `0xFFFFFFFF` — "none" for node links.
pub const NONE: u32 = 0xffff_ffff;

pub const MAJOR: u16 = 2;
pub const MINOR: u16 = 0;

/// Node kinds (`NODE.kind`).
pub mod kind {
	pub const ROW: u8 = 0;
	pub const COL: u8 = 1;
	pub const WRAP: u8 = 2;
	pub const GRID: u8 = 3;
	pub const STACK: u8 = 4;
	pub const CANVAS: u8 = 5;
	pub const PARA: u8 = 6;
	pub const GROUP: u8 = 7;
	pub const TEXT: u8 = 8;
	pub const SPAN: u8 = 9;
	pub const RECT: u8 = 10;
	pub const IMG: u8 = 11;
	pub const PATH: u8 = 12;
	pub const SPACER: u8 = 13;
	pub const HOLE: u8 = 14;
	pub const EACH: u8 = 15;
	pub const DIVIDER: u8 = 16;
	pub const ICON: u8 = 17;

	pub const NAMES: [&str; 18] = [
		"Row", "Col", "Wrap", "Grid", "Stack", "Canvas", "Para", "Group", "Text", "Span", "Rect",
		"Img", "Path", "Spacer", "Hole", "Each", "Divider", "Icon",
	];
}

/// Node flag bits (`NODE.flags`, and the `flags` patch attr mask).
pub mod flags {
	pub const CLIP: u16 = 1 << 0;
	pub const BLEED: u16 = 1 << 1;
	pub const SCROLL: u16 = 1 << 2;
	pub const NOWRAP: u16 = 1 << 3;
	pub const ELLIPSIS: u16 = 1 << 4;
	pub const INERT: u16 = 1 << 5;
	pub const FOCUSABLE: u16 = 1 << 6;
	pub const DETACHED: u16 = 1 << 7;
	pub const MULTILINE: u16 = 1 << 8;
	pub const SCROLL_CROSS: u16 = 1 << 9;
	pub const VIRTUAL: u16 = 1 << 10;
	pub const STICKY: u16 = 1 << 11;
	pub const DRAG_GHOST: u16 = 1 << 12;
	pub const ESCAPE_BLUR: u16 = 1 << 13;

	pub const NAMES: [(u16, &str); 14] = [
		(CLIP, "clip"),
		(BLEED, "bleed"),
		(SCROLL, "scroll"),
		(NOWRAP, "nowrap"),
		(ELLIPSIS, "ellipsis"),
		(INERT, "inert"),
		(FOCUSABLE, "focusable"),
		(DETACHED, "detached"),
		(MULTILINE, "multiline"),
		(SCROLL_CROSS, "scroll-cross"),
		(VIRTUAL, "virtual"),
		(STICKY, "sticky"),
		(DRAG_GHOST, "drag-ghost"),
		(ESCAPE_BLUR, "escape-blur"),
	];
}

/// AVAL tags.
pub mod aval {
	pub const NUM: u8 = 0;
	pub const PCT: u8 = 1;
	pub const STR: u8 = 2;
	pub const COLOR: u8 = 3;
	pub const TUPLE: u8 = 4;
	pub const SIZE_FIXED: u8 = 5;
	pub const SIZE_HUG: u8 = 6;
	pub const SIZE_FILL: u8 = 7;
	pub const SIZE_PCT: u8 = 8;
	pub const PAINT_SOLID: u8 = 9;
	pub const PAINT_GRADIENT: u8 = 10;
	pub const PATH_REF: u8 = 11;
	pub const SHADOW_LIST: u8 = 12;
	pub const PARAM_REF: u8 = 13;
	pub const ENUM_SYM: u8 = 14;
	pub const PAINT_NONE: u8 = 15;
	pub const PROP_REF: u8 = 16;
	pub const LIST_DEFAULT: u8 = 17;
	pub const TUPLE_DYN: u8 = 18;
	pub const PAINT_CURRENT: u8 = 19;
	/// Reference to one typed active-theme token row.
	pub const TOKEN_REF: u8 = 20;

	pub const NAMES: [&str; 21] = [
		"Num",
		"Pct",
		"Str",
		"Color",
		"Tuple",
		"SizeFixed",
		"SizeHug",
		"SizeFill",
		"SizePct",
		"PaintSolid",
		"PaintGradient",
		"PathRef",
		"ShadowList",
		"ParamRef",
		"EnumSym",
		"PaintNone",
		"PropRef",
		"ListDefault",
		"TupleDyn",
		"PaintCurrent",
		"TokenRef",
	];
}

/// WHEN condition kinds.
pub mod cond {
	pub const STATE: u8 = 0;
	pub const ENV: u8 = 1;
	pub const CLIENT: u8 = 2;
	pub const WCMP: u8 = 3;
	pub const HCMP: u8 = 4;
	pub const PROP: u8 = 5;
	pub const THEME: u8 = 6;

	pub const NAMES: [&str; 7] = ["State", "Env", "Client", "WCmp", "HCmp", "Prop", "Theme"];
	/// Comparison ops for WCmp/HCmp.
	pub const OPS: [&str; 5] = ["<", "<=", ">", ">=", "=="];
}

/// A typed value-pool entry. `payload` interpretation depends on `tag`:
/// f64 bit pattern, a low-u32 handle, or `(lo: offset, hi: len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aval {
	pub tag:     u8,
	pub payload: u64,
}

impl Aval {
	pub const fn f64_payload(f: f64) -> u64 {
		f.to_bits()
	}

	pub const fn as_f64(&self) -> f64 {
		f64::from_bits(self.payload)
	}

	pub const fn lo(&self) -> u32 {
		self.payload as u32
	}

	pub const fn hi(&self) -> u32 {
		(self.payload >> 32) as u32
	}

	pub const fn pair(lo: u32, hi: u32) -> u64 {
		lo as u64 | ((hi as u64) << 32)
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct GradE {
	/// 0 linear | 1 radial
	pub kind:  u8,
	pub angle: f64,
	/// (pos 0..=1, rgba8)
	pub stops: Vec<(f64, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShadowE {
	pub x:      f64,
	pub y:      f64,
	pub blur:   f64,
	pub spread: f64,
	pub rgba:   u32,
	pub inset:  u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PathE {
	/// 0 M | 1 L | 2 C | 3 Q | 4 Z (absolute coords)
	pub verbs:  Vec<u8>,
	pub coords: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontE {
	pub family:          u32,
	/// 0 sans | 1 mono
	pub class:           u8,
	pub weight:          u16,
	pub upem:            u16,
	pub ascent:          i16,
	pub descent:         i16,
	pub line_gap:        i16,
	pub default_advance: u16,
	/// (codepoint, glyph id), sorted by codepoint.
	pub cmap:            Vec<(u32, u16)>,
	/// Advance widths in font units, parallel to `cmap`.
	pub advances:        Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CondE {
	pub kind: u8,
	pub neg:  u8,
	pub op:   u8,
	pub num:  f64,
	pub sym:  u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchE {
	pub node:      u32,
	pub cond:      u32,
	pub attr_off:  u32,
	pub attr_len:  u32,
	pub child_off: u32,
	pub child_len: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimE {
	pub name:  u32,
	/// (pos 0..=1, `attr_off`, `attr_len`) into `anim_attrs`.
	pub stops: Vec<(f64, u32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BindE {
	pub node:   u32,
	pub anim:   u32,
	pub dur:    f64,
	/// 0 loop | 1 once | 2 alternate
	pub mode:   u8,
	/// 0 linear | 1 `ease_in` | 2 `ease_out` | 3 `ease_in_out`
	pub easing: u8,
	pub delay:  f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransE {
	pub node:   u32,
	pub easing: u8,
	pub dur:    f64,
	pub delay:  f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamE {
	pub name:     u32,
	/// 0 Text | 1 Num | 2 Pct | 3 Color | 4 Bool | 5 Enum | 6 List
	pub ty:       u8,
	pub default:  u32,
	pub enum_off: u32,
	pub enum_len: u32,
	pub site_off: u32,
	pub site_len: u32,
}

pub const PARAM_TYPE_NAMES: [&str; 7] = ["text", "num", "pct", "color", "bool", "enum", "list"];

/// One member of a `TupleDyn` run (`aval::TUPLE_DYN`): a literal number or
/// a num/pct parameter reference resolved by the kernel per solve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TupDynE {
	Lit(f64),
	Param(u32),
}

/// A list-param schema slice. `param` indexes `Slir::params`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListE {
	pub param:     u32,
	pub field_off: u32,
	pub field_len: u32,
}

/// One field in a list element schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListFieldE {
	pub name:     u32,
	/// PARAM type code, including `List=6`.
	pub ty:       u8,
	pub default:  u32,
	pub enum_off: u32,
	pub enum_len: u32,
	/// Zero for a scalar, otherwise one plus the nested list-schema row.
	pub sub:      u32,
}

/// One normalized default item, slicing `Slir::list_item_values`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListItemE {
	pub field_off: u32,
	pub field_len: u32,
}

/// One field assignment in a normalized default item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListItemValueE {
	pub field: u32,
	pub val:   u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImgE {
	pub src:      u32,
	pub w:        u32,
	pub h:        u32,
	/// 0 png
	pub format:   u8,
	pub blob_off: u32,
	pub blob_len: u32,
}

/// One named, detached icon subtree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconE {
	pub name:    u32,
	pub node:    u32,
	pub viewbox: f64,
}

/// One typed token row with a canonical host representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenE {
	pub name:      u32,
	pub base:      u32,
	pub base_repr: u32,
	/// `(theme-name STRS ref, AVAL ref, canonical-repr STRS ref)`.
	pub themes:    Vec<(u32, u32, u32)>,
}

/// `SoA` node arrays; index = node id, node 0 = root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Nodes {
	pub kind:        Vec<u8>,
	pub flags:       Vec<u16>,
	pub parent:      Vec<u32>,
	pub first_child: Vec<u32>,
	pub next_sib:    Vec<u32>,
	pub key:         Vec<u32>,
	pub id:          Vec<u32>,
	pub src_line:    Vec<u32>,
}

impl Nodes {
	pub const fn len(&self) -> usize {
		self.kind.len()
	}

	pub const fn is_empty(&self) -> bool {
		self.kind.is_empty()
	}
}

/// A fully decoded SLIR document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slir {
	pub strs:             Vec<String>,
	pub nodes:            Nodes,
	pub avals:            Vec<Aval>,
	/// f64 pool tail addressed by `Tuple` values.
	pub f64s:             Vec<f64>,
	/// Member pool addressed by `TupleDyn` values.
	pub tup_dyn:          Vec<TupDynE>,
	pub grads:            Vec<GradE>,
	pub shadows:          Vec<ShadowE>,
	/// `node_count + 1` fenceposts into `attrs`.
	pub attr_index:       Vec<u32>,
	/// `(attr id, AVAL index)`, per node ascending by attr id.
	pub attrs:            Vec<(u16, u32)>,
	pub paths:            Vec<PathE>,
	pub fonts:            Vec<FontE>,
	pub conds:            Vec<CondE>,
	pub patches:          Vec<PatchE>,
	pub patch_attrs:      Vec<(u16, u32)>,
	pub patch_children:   Vec<u32>,
	pub anims:            Vec<AnimE>,
	pub anim_attrs:       Vec<(u16, u32)>,
	pub bindings:         Vec<BindE>,
	pub transitions:      Vec<TransE>,
	pub params:           Vec<ParamE>,
	/// STRS refs pool addressed by scalar-enum `ParamE.enum_off/len`.
	pub param_enum_syms:  Vec<u32>,
	/// `(node, attr)` pool addressed by `ParamE.site_off/len`.
	pub param_sites:      Vec<(u32, u16)>,
	/// One entry per list param, slicing `list_fields`.
	pub lists:            Vec<ListE>,
	pub list_fields:      Vec<ListFieldE>,
	/// STRS refs addressed by `ListFieldE.enum_off/len`.
	pub list_enum_syms:   Vec<u32>,
	/// Default item runs addressed by `LIST_DEFAULT` AVAL payloads.
	pub list_items:       Vec<ListItemE>,
	pub list_item_values: Vec<ListItemValueE>,
	/// STRS refs for every compiler-declared theme, in declaration order.
	pub themes:           Vec<u32>,
	/// Scalar public-token rows first, followed by typed token-use rows.
	pub tokens:           Vec<TokenE>,
	/// `(name strref, node)`.
	pub holes:            Vec<(u32, u32)>,
	/// `(name strref, node, trigger: 0 Activate | 1 Change)`.
	pub signals:          Vec<(u32, u32, u8)>,
	pub images:           Vec<ImgE>,
	pub icons:            Vec<IconE>,
	pub blob:             Vec<u8>,
}

impl Slir {
	pub fn str_at(&self, i: u32) -> &str {
		self.strs.get(i as usize).map_or("", String::as_str)
	}

	/// Attr run of one node.
	pub fn node_attrs(&self, node: u32) -> &[(u16, u32)] {
		let a = self.attr_index[node as usize] as usize;
		let b = self.attr_index[node as usize + 1] as usize;
		&self.attrs[a..b]
	}

	/// List schema metadata for a PARAM index.
	pub fn list_for_param(&self, param: u32) -> Option<&ListE> {
		self.lists.iter().find(|list| list.param == param)
	}
}
