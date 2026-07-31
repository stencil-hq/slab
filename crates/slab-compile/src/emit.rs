//! SLIR emission: flatten the expanded `CNode` tree into `SoA` sections with
//! interned strings and deduplicated value pools. Deterministic: interning
//! follows a single DFS pre-order walk.

use std::collections::{BTreeSet, HashMap};

use slab_fonts::{self as font_assets};
use slab_kernel::graphemes;
use slab_slir::{
	AnimE, Aval, BindE, CondE, GradE, IconE, ImgE, ListE, ListFieldE, ListItemE, ListItemValueE,
	NONE, ParamE, PatchE, PathE, ShadowE, Slir, TokenE, TransE, TupDynE, attrs as at, aval as av,
	flags as fl, kind as nk,
};
use slab_syntax::diag::Diagnostics;

use crate::{
	Options,
	color::{Paint, rgba_word},
	expand::{
		AttrE, CNode, CPatch, Expanded, ListInfo, ListItemInfo, RVal, SizeSpec, TVal, TokenInfo,
		TupMember,
	},
	fonts,
};

/// Shadow-run dedup key: per shadow `(x, y, blur, rgba, inset)` bit patterns.
type ShadowKey = Vec<(u64, u64, u64, u32, u8)>;

fn token_repr(value: &RVal) -> Option<String> {
	match value {
		RVal::Token { base, .. } => token_repr(base),
		RVal::Num(value) => Some(crate::expand::fmt_g(*value)),
		RVal::Pct(value) => Some(format!("{}%", crate::expand::fmt_g(*value))),
		RVal::Str(value) | RVal::Color(value) | RVal::Kw(value) => Some(value.clone()),
		RVal::Fill(weight) if *weight == 1.0 => Some("fill".to_string()),
		RVal::Fill(weight) => Some(format!("fill:{}", crate::expand::fmt_g(*weight))),
		RVal::Tup(items) => {
			let parts: Option<Vec<_>> = items.iter().map(token_repr).collect();
			parts.map(|parts| parts.join(","))
		},
		_ => None,
	}
}

fn token_tval(value: &RVal) -> Option<TVal> {
	match value {
		RVal::Token { base, .. } => token_tval(base),
		RVal::Num(value) => Some(TVal::Num(*value)),
		RVal::Pct(value) => Some(TVal::Pct(*value)),
		RVal::Str(value) | RVal::Kw(value) => Some(TVal::Str(value.clone())),
		RVal::Color(value) => crate::color::parse_paint(value).map(TVal::Paint),
		RVal::Fill(_) | RVal::Tup(_) => token_repr(value).map(TVal::Str),
		_ => None,
	}
}
/// Collects authored family names and exact numeric weights from an attribute
/// run.
fn collect_font_attrs(
	attrs: &[AttrE],
	families: &mut BTreeSet<String>,
	weights: &mut BTreeSet<u16>,
) {
	for attr in attrs {
		match (attr.id, &attr.val) {
			(at::FAMILY, TVal::Str(family)) => {
				families.insert(family.clone());
			},
			(at::WEIGHT, TVal::Num(weight)) => {
				weights.insert(font_assets::normalize_weight(*weight));
			},
			_ => {},
		}
	}
}

fn collect_node_fonts(node: &CNode, families: &mut BTreeSet<String>, weights: &mut BTreeSet<u16>) {
	collect_font_attrs(&node.attrs, families, weights);
	for patch in &node.patches {
		collect_font_attrs(&patch.attrs, families, weights);
		for child in &patch.children {
			collect_node_fonts(child, families, weights);
		}
	}
	for child in &node.children {
		collect_node_fonts(child, families, weights);
	}
}
fn resolve_text_value(value: &TVal, ex: &Expanded, schema: Option<&ListInfo>) -> Option<String> {
	match value {
		TVal::Token { base, .. } => resolve_text_value(base, ex, schema),
		TVal::Str(text) => Some(text.clone()),
		TVal::Param(param) => ex
			.params
			.get(*param as usize)
			.and_then(|info| resolve_text_value(&info.default, ex, schema)),
		TVal::Prop(field) => schema
			.and_then(|info| info.fields.get(*field as usize))
			.and_then(|info| resolve_text_value(&info.default, ex, schema)),
		_ => None,
	}
}

fn resolve_num_value(value: &TVal, ex: &Expanded, schema: Option<&ListInfo>) -> Option<f64> {
	match value {
		TVal::Token { base, .. } => resolve_num_value(base, ex, schema),
		TVal::Num(number) => Some(*number),
		TVal::Param(param) => ex
			.params
			.get(*param as usize)
			.and_then(|info| resolve_num_value(&info.default, ex, schema)),
		TVal::Prop(field) => schema
			.and_then(|info| info.fields.get(*field as usize))
			.and_then(|info| resolve_num_value(&info.default, ex, schema)),
		_ => None,
	}
}

fn family_from_attrs(
	attrs: &[AttrE],
	inherited: Option<&str>,
	ex: &Expanded,
	schema: Option<&ListInfo>,
) -> Option<String> {
	attrs
		.iter()
		.rev()
		.find(|attr| attr.id == at::FAMILY)
		.map_or_else(
			|| inherited.map(str::to_owned),
			|attr| resolve_text_value(&attr.val, ex, schema),
		)
}

fn weight_from_attrs(
	attrs: &[AttrE],
	inherited: Option<u16>,
	ex: &Expanded,
	schema: Option<&ListInfo>,
) -> Option<u16> {
	attrs
		.iter()
		.rev()
		.find(|attr| attr.id == at::WEIGHT)
		.map_or(inherited, |attr| {
			resolve_num_value(&attr.val, ex, schema).map(font_assets::normalize_weight)
		})
}

fn warn_missing_text_glyphs(
	text: &str,
	family: Option<&str>,
	weight: Option<u16>,
	line: u32,
	coverage: &HashMap<String, HashMap<u16, BTreeSet<u32>>>,
	warned: &mut BTreeSet<(String, u32, u32)>,
	diags: &mut Diagnostics,
) {
	let (Some(family), Some(weight)) = (family, weight) else {
		return;
	};
	let display_family = if family.is_empty() { "sans" } else { family };
	let Some(codepoints) = coverage.get(family).and_then(|faces| faces.get(&weight)) else {
		return;
	};
	for character in text.chars() {
		let codepoint = u32::from(character);
		if !graphemes::requires_glyph(codepoint)
			|| codepoints.contains(&codepoint)
			|| !warned.insert((family.to_string(), codepoint, line))
		{
			continue;
		}
		diags.warn(
			"glyph-missing",
			format!(
				"character '{character}' (U+{codepoint:04X}) is missing from embedded font family \
				 '{display_family}'"
			),
			line,
		);
	}
}

fn each_schema<'a>(
	node: &CNode,
	ex: &'a Expanded,
	parent_schema: Option<&ListInfo>,
) -> Option<&'a ListInfo> {
	if node.kind != nk::EACH {
		return None;
	}
	let target = node.attrs.iter().find(|attr| attr.id == at::EACH)?;
	let schema = match &target.val {
		TVal::Num(param) => ex.params.get(*param as usize)?.list?,
		TVal::Prop(field) => parent_schema?.fields.get(*field as usize)?.sub?,
		_ => return None,
	};
	ex.list_schemas.get(schema as usize)
}

struct GlyphWarnings<'a> {
	coverage: &'a HashMap<String, HashMap<u16, BTreeSet<u32>>>,
	warned:   &'a mut BTreeSet<(String, u32, u32)>,
	diags:    &'a mut Diagnostics,
}

fn warn_value_glyphs(
	value: &TVal,
	family: Option<&str>,
	weight: Option<u16>,
	line: u32,
	ex: &Expanded,
	schema: Option<&ListInfo>,
	warnings: &mut GlyphWarnings<'_>,
) {
	if let Some(content) = resolve_text_value(value, ex, schema) {
		warn_missing_text_glyphs(
			&content,
			family,
			weight,
			line,
			warnings.coverage,
			warnings.warned,
			warnings.diags,
		);
	}
}

fn check_node_glyphs(
	node: &CNode,
	inherited_family: Option<&str>,
	inherited_weight: Option<u16>,
	ex: &Expanded,
	schema: Option<&ListInfo>,
	warnings: &mut GlyphWarnings<'_>,
) {
	let family = family_from_attrs(&node.attrs, inherited_family, ex, schema);
	let weight = weight_from_attrs(&node.attrs, inherited_weight, ex, schema);
	if let Some(content) = &node.content {
		warn_value_glyphs(content, family.as_deref(), weight, node.line, ex, schema, warnings);
	}
	for patch in &node.patches {
		let patch_family = family_from_attrs(&patch.attrs, family.as_deref(), ex, schema);
		let patch_weight = weight_from_attrs(&patch.attrs, weight, ex, schema);
		if let Some(content) = patch.attrs.iter().rev().find(|attr| attr.id == at::CONTENT) {
			warn_value_glyphs(
				&content.val,
				patch_family.as_deref(),
				patch_weight,
				patch.line,
				ex,
				schema,
				warnings,
			);
		}
		for child in &patch.children {
			check_node_glyphs(child, patch_family.as_deref(), patch_weight, ex, schema, warnings);
		}
	}
	let child_schema = each_schema(node, ex, schema).or(schema);
	for child in &node.children {
		check_node_glyphs(child, family.as_deref(), weight, ex, child_schema, warnings);
	}
}

fn collect_resolved_font_attrs(
	attrs: &[AttrE],
	ex: &Expanded,
	schema: Option<&ListInfo>,
	families: &mut BTreeSet<String>,
	weights: &mut BTreeSet<u16>,
) {
	for attr in attrs {
		if attr.id == at::FAMILY {
			if let Some(family) = resolve_text_value(&attr.val, ex, schema) {
				families.insert(family);
			}
		} else if attr.id == at::WEIGHT
			&& let Some(weight) = resolve_num_value(&attr.val, ex, schema)
		{
			weights.insert(font_assets::normalize_weight(weight));
		}
	}
}

fn collect_resolved_node_fonts(
	node: &CNode,
	ex: &Expanded,
	schema: Option<&ListInfo>,
	families: &mut BTreeSet<String>,
	weights: &mut BTreeSet<u16>,
) {
	collect_resolved_font_attrs(&node.attrs, ex, schema, families, weights);
	for patch in &node.patches {
		collect_resolved_font_attrs(&patch.attrs, ex, schema, families, weights);
		for child in &patch.children {
			collect_resolved_node_fonts(child, ex, schema, families, weights);
		}
	}
	let child_schema = each_schema(node, ex, schema).or(schema);
	for child in &node.children {
		collect_resolved_node_fonts(child, ex, child_schema, families, weights);
	}
}

fn warn_missing_glyphs(ex: &Expanded, opts: &Options, diags: &mut Diagnostics) {
	let mut families = BTreeSet::from([String::new()]);
	let mut weights = BTreeSet::from([400u16]);
	for root in &ex.roots {
		collect_node_fonts(root, &mut families, &mut weights);
		collect_resolved_node_fonts(root, ex, None, &mut families, &mut weights);
	}
	families.extend(ex.font_families.iter().cloned());
	weights.extend(ex.font_weights.iter().copied());
	let mut coverage: HashMap<String, HashMap<u16, BTreeSet<u32>>> = HashMap::new();
	for family in families {
		let class = font_assets::classify_family(&family);
		let custom = opts
			.fonts
			.iter()
			.find(|(name, _)| name.eq_ignore_ascii_case(&family))
			.map(|(_, bytes)| bytes.as_slice());
		for &weight in &weights {
			let bytes = custom.unwrap_or_else(|| font_assets::asset(class, weight).bytes);
			let metrics = font_assets::parse_metrics(bytes).expect("registered font parses");
			coverage
				.entry(family.clone())
				.or_default()
				.insert(weight, metrics.cps.into_iter().collect());
		}
	}
	let mut warned = BTreeSet::new();
	let mut warnings = GlyphWarnings { coverage: &coverage, warned: &mut warned, diags };
	for root in &ex.roots {
		check_node_glyphs(root, Some(""), Some(400), ex, None, &mut warnings);
	}
}

type GradKey = (u8, u64, Vec<(u64, u32)>);

struct Emitter<'a> {
	slir:        Slir,
	str_ix:      HashMap<String, u32>,
	aval_ix:     HashMap<(u8, u64), u32>,
	tuple_ix:    HashMap<Vec<u64>, (u32, u32)>,
	tup_dyn_ix:  HashMap<Vec<(u8, u64)>, (u32, u32)>,
	shadow_ix:   HashMap<ShadowKey, (u32, u32)>,
	grad_ix:     HashMap<GradKey, u32>,
	path_ix:     HashMap<(Vec<u8>, Vec<u64>), u32>,
	/// attr runs per node, flattened into ATTR at the end
	runs:        Vec<Vec<(u16, u32)>>,
	/// (node, patch) collected during the walk, pooled after
	patch_specs: Vec<(u32, EPatch)>,
	/// per-param use sites
	sites:       Vec<Vec<(u32, u16)>>,
	anim_names:  Vec<String>,
	diags:       &'a mut Diagnostics,
}

struct EPatch {
	cond:     CondSpecEnc,
	attrs:    Vec<(u16, u32)>,
	children: Vec<u32>,
}

struct CondSpecEnc {
	kind: u8,
	neg:  u8,
	op:   u8,
	num:  f64,
	sym:  u32,
}

impl Emitter<'_> {
	fn intern(&mut self, s: &str) -> u32 {
		if let Some(&ix) = self.str_ix.get(s) {
			return ix;
		}
		let ix = self.slir.strs.len() as u32;
		self.slir.strs.push(s.to_string());
		self.str_ix.insert(s.to_string(), ix);
		ix
	}

	fn aval(&mut self, tag: u8, payload: u64) -> u32 {
		if let Some(&ix) = self.aval_ix.get(&(tag, payload)) {
			return ix;
		}
		let ix = self.slir.avals.len() as u32;
		self.slir.avals.push(Aval { tag, payload });
		self.aval_ix.insert((tag, payload), ix);
		ix
	}

	fn tuple(&mut self, items: &[f64]) -> u32 {
		let key: Vec<u64> = items.iter().map(|f| f.to_bits()).collect();
		let (off, len) = if let Some(&r) = self.tuple_ix.get(&key) {
			r
		} else {
			let off = self.slir.f64s.len() as u32;
			self.slir.f64s.extend_from_slice(items);
			let r = (off, items.len() as u32);
			self.tuple_ix.insert(key, r);
			r
		};
		self.aval(av::TUPLE, Aval::pair(off, len))
	}

	fn tuple_dyn(&mut self, members: &[TupMember], node: u32, attr: u16) -> u32 {
		let key: Vec<(u8, u64)> = members
			.iter()
			.map(|m| match m {
				TupMember::Lit(x) => (0u8, x.to_bits()),
				TupMember::Param(ix) => (1u8, u64::from(*ix)),
			})
			.collect();
		let (off, len) = if let Some(&r) = self.tup_dyn_ix.get(&key) {
			r
		} else {
			let off = self.slir.tup_dyn.len() as u32;
			self.slir.tup_dyn.extend(members.iter().map(|m| match m {
				TupMember::Lit(x) => TupDynE::Lit(*x),
				TupMember::Param(ix) => TupDynE::Param(*ix),
			}));
			let r = (off, members.len() as u32);
			self.tup_dyn_ix.insert(key, r);
			r
		};
		if node != NONE {
			for m in members {
				if let TupMember::Param(ix) = m {
					self.sites[*ix as usize].push((node, attr));
				}
			}
		}
		self.aval(av::TUPLE_DYN, Aval::pair(off, len))
	}

	fn paint(&mut self, p: &Paint) -> u32 {
		match p {
			Paint::None => self.aval(av::PAINT_NONE, 0),
			Paint::Solid(c) => self.aval(av::PAINT_SOLID, rgba_word(*c) as u64),
			Paint::Linear { stops, .. } | Paint::Radial { stops } | Paint::Conic { stops, .. } => {
				let (kind, angle) = match p {
					Paint::Linear { angle, .. } => (0u8, *angle),
					Paint::Conic { from, .. } => (2u8, *from),
					_ => (1u8, 0.0),
				};
				let stops_e: Vec<(f64, u32)> = stops
					.iter()
					.map(|s| (s.offset, rgba_word(s.rgba)))
					.collect();
				let key =
					(kind, angle.to_bits(), stops_e.iter().map(|&(p, c)| (p.to_bits(), c)).collect());
				let gix = if let Some(&g) = self.grad_ix.get(&key) {
					g
				} else {
					let g = self.slir.grads.len() as u32;
					self.slir.grads.push(GradE { kind, angle, stops: stops_e });
					self.grad_ix.insert(key, g);
					g
				};
				self.aval(av::PAINT_GRADIENT, gix as u64)
			},
		}
	}

	fn token_row(
		&mut self,
		path: &str,
		base: &TVal,
		themes: &[(String, TVal)],
		base_repr: u32,
		theme_reprs: &[u32],
	) -> u32 {
		let name = self.intern(path);
		let base = self.aval_of(base, NONE, 0);
		let mut encoded_themes = Vec::with_capacity(themes.len());
		for (index, (theme, value)) in themes.iter().enumerate() {
			let theme = self.intern(theme);
			let value = self.aval_of(value, NONE, 0);
			encoded_themes.push((theme, value, theme_reprs.get(index).copied().unwrap_or(0)));
		}
		let row = self.slir.tokens.len() as u32;
		self
			.slir
			.tokens
			.push(TokenE { name, base, base_repr, themes: encoded_themes });
		row
	}

	fn public_token(&mut self, token: &TokenInfo) {
		let (Some(base), Some(base_repr)) = (token_tval(&token.base), token_repr(&token.base)) else {
			return;
		};
		let mut themes = Vec::with_capacity(token.themes.len());
		let mut reprs = Vec::with_capacity(token.themes.len());
		for (name, value) in &token.themes {
			let (Some(value), Some(repr)) = (token_tval(value), token_repr(value)) else {
				continue;
			};
			themes.push((name.clone(), value));
			reprs.push(self.intern(&repr));
		}
		let base_repr = self.intern(&base_repr);
		self.token_row(&token.path, &base, &themes, base_repr, &reprs);
	}

	fn aval_of(&mut self, tv: &TVal, node: u32, attr: u16) -> u32 {
		match tv {
			TVal::Num(x) => self.aval(av::NUM, x.to_bits()),
			TVal::Pct(x) => self.aval(av::PCT, x.to_bits()),
			TVal::Size(spec) => match spec {
				SizeSpec::Fixed(x) => self.aval(av::SIZE_FIXED, x.to_bits()),
				SizeSpec::Hug => self.aval(av::SIZE_HUG, 0),
				SizeSpec::Fill(wt) => self.aval(av::SIZE_FILL, wt.to_bits()),
				SizeSpec::Pct(p) => self.aval(av::SIZE_PCT, p.to_bits()),
			},
			TVal::Tuple(items) => self.tuple(items),
			TVal::TupleDyn(members) => self.tuple_dyn(members, node, attr),
			TVal::Enum(sym) => {
				let s = self.intern(sym);
				self.aval(av::ENUM_SYM, s as u64)
			},
			TVal::Str(text) => {
				let s = self.intern(text);
				self.aval(av::STR, s as u64)
			},
			TVal::Paint(p) => self.paint(p),
			TVal::PaintCurrent => self.aval(av::PAINT_CURRENT, 0),
			TVal::Color(c) => self.aval(av::COLOR, rgba_word(*c) as u64),
			TVal::Token { path, base, themes } => {
				let row = self.token_row(path, base, themes, 0, &[]);
				self.aval(av::TOKEN_REF, u64::from(row))
			},
			TVal::Shadows(list) => {
				let key: Vec<_> = list
					.iter()
					.map(|s| {
						(s.x.to_bits(), s.y.to_bits(), s.blur.to_bits(), rgba_word(s.rgba), s.inset as u8)
					})
					.collect();
				let (off, len) = if let Some(&r) = self.shadow_ix.get(&key) {
					r
				} else {
					let off = self.slir.shadows.len() as u32;
					for s in list {
						self.slir.shadows.push(ShadowE {
							x:      s.x,
							y:      s.y,
							blur:   s.blur,
							spread: 0.0,
							rgba:   rgba_word(s.rgba),
							inset:  s.inset as u8,
						});
					}
					let r = (off, list.len() as u32);
					self.shadow_ix.insert(key, r);
					r
				};
				self.aval(av::SHADOW_LIST, Aval::pair(off, len))
			},
			TVal::Path(verbs, coords) => {
				let key = (verbs.clone(), coords.iter().map(|c| c.to_bits()).collect::<Vec<_>>());
				let pix = if let Some(&p) = self.path_ix.get(&key) {
					p
				} else {
					let p = self.slir.paths.len() as u32;
					self
						.slir
						.paths
						.push(PathE { verbs: verbs.clone(), coords: coords.clone() });
					self.path_ix.insert(key, p);
					p
				};
				self.aval(av::PATH_REF, pix as u64)
			},
			TVal::Param(ix) => {
				if node != NONE {
					self.sites[*ix as usize].push((node, attr));
				}
				self.aval(av::PARAM_REF, *ix as u64)
			},
			TVal::Prop(field) => self.aval(av::PROP_REF, *field as u64),
			TVal::List(items) => self.list_default(items),
		}
	}

	fn list_default(&mut self, items: &[ListItemInfo]) -> u32 {
		let item_off = self.slir.list_items.len() as u32;
		self
			.slir
			.list_items
			.resize(self.slir.list_items.len() + items.len(), ListItemE {
				field_off: 0,
				field_len: 0,
			});
		for (item_index, item) in items.iter().enumerate() {
			let compiled_values: Vec<ListItemValueE> = item
				.values
				.iter()
				.enumerate()
				.map(|(field, value)| ListItemValueE {
					field: field as u32,
					val:   self.aval_of(value, NONE, 0),
				})
				.collect();
			let value_off = self.slir.list_item_values.len() as u32;
			self.slir.list_item_values.extend(compiled_values);
			self.slir.list_items[item_off as usize + item_index] =
				ListItemE { field_off: value_off, field_len: item.values.len() as u32 };
		}
		self.aval(av::LIST_DEFAULT, Aval::pair(item_off, items.len() as u32))
	}

	fn list_schema(&mut self, list: &ListInfo, param: u32) {
		let field_off = self.slir.list_fields.len() as u32;
		for field in &list.fields {
			let field_name = self.intern(&field.name);
			let field_default = self.aval_of(&field.default, NONE, 0);
			let enum_off = self.slir.list_enum_syms.len() as u32;
			for sym in &field.enum_syms {
				let sym = self.intern(sym);
				self.slir.list_enum_syms.push(sym);
			}
			let field_ty = match &field.ty {
				slab_syntax::ast::ParamType::Text => 0,
				slab_syntax::ast::ParamType::Num => 1,
				slab_syntax::ast::ParamType::Pct => 2,
				slab_syntax::ast::ParamType::Color => 3,
				slab_syntax::ast::ParamType::Bool => 4,
				slab_syntax::ast::ParamType::Enum => 5,
				slab_syntax::ast::ParamType::List(_) => 6,
			};
			self.slir.list_fields.push(ListFieldE {
				name: field_name,
				ty: field_ty,
				default: field_default,
				enum_off,
				enum_len: field.enum_syms.len() as u32,
				sub: field.sub.map_or(0, |row| row + 1),
			});
		}
		self
			.slir
			.lists
			.push(ListE { param, field_off, field_len: list.fields.len() as u32 });
	}

	fn attr_run(&mut self, node: u32, attrs: &[AttrE], content: Option<&TVal>) -> Vec<(u16, u32)> {
		let mut run: Vec<(u16, u32)> = attrs
			.iter()
			.map(|e| (e.id, self.aval_of(&e.val, node, e.id)))
			.collect();
		if let Some(c) = content {
			run.push((at::CONTENT, self.aval_of(c, node, at::CONTENT)));
		}
		run.sort_by_key(|&(id, _)| id);
		run
	}

	fn cond_enc(&mut self, c: &crate::expand::CondSpec) -> CondSpecEnc {
		let (kind, neg, op, num, sym) = c.encode();
		let sym = match c {
			crate::expand::CondSpec::Prop(field, _) => *field,
			_ => self.intern(sym),
		};
		CondSpecEnc { kind, neg, op, num, sym }
	}

	fn walk(&mut self, n: &CNode, parent: u32, detached: bool) -> u32 {
		let ix = self.slir.nodes.len() as u32;
		let key = self.intern(&n.key);
		let id = match &n.id {
			Some(s) => self.intern(s),
			None => 0,
		};
		let nodes = &mut self.slir.nodes;
		nodes.kind.push(n.kind);
		nodes
			.flags
			.push(n.flags | if detached { fl::DETACHED } else { 0 });
		nodes.parent.push(parent);
		nodes.first_child.push(NONE);
		nodes.next_sib.push(NONE);
		nodes.key.push(key);
		nodes.id.push(id);
		nodes.src_line.push(n.line);

		let run = self.attr_run(ix, &n.attrs, n.content.as_ref());
		self.runs.push(run);

		// Signals / holes / bindings / transitions, in pre-order.
		for (name, trigger) in [
			(&n.act, 0),
			(&n.field, 1),
			(&n.submit, 2),
			(&n.cancel, 14),
			(&n.press, 3),
			(&n.context, 4),
			(&n.dblclick, 5),
			(&n.drag, 6),
			(&n.drop, 7),
			(&n.resize, 8),
			(&n.pointer_move, 9),
			(&n.pointer_up, 10),
			(&n.drag_update, 11),
			(&n.drag_end, 12),
		] {
			if let Some(name) = name {
				let string = self.intern(name);
				self.slir.signals.push((string, ix, trigger));
			}
		}
		for (name, trigger) in &n.conditional_signals {
			let string = self.intern(name);
			self.slir.signals.push((string, ix, *trigger));
		}
		if let Some(name) = &n.hole {
			let s = self.intern(name);
			self.slir.holes.push((s, ix));
		}
		if let Some(b) = &n.animate
			&& let Some(anim) = self.anim_names.iter().position(|a| *a == b.name)
		{
			self.slir.bindings.push(BindE {
				node:   ix,
				anim:   anim as u32,
				dur:    b.dur,
				mode:   b.mode,
				easing: b.easing,
				delay:  b.delay,
			});
		}
		for b in &n.conditional_animations {
			if let Some(anim) = self.anim_names.iter().position(|a| *a == b.name) {
				self.slir.bindings.push(BindE {
					node:   ix,
					anim:   anim as u32,
					dur:    b.dur,
					mode:   b.mode,
					easing: b.easing,
					delay:  b.delay,
				});
			}
		}
		if let Some((dur, easing, delay)) = n.transition {
			self
				.slir
				.transitions
				.push(TransE { node: ix, easing, dur, delay });
		}

		// children: base first, then each patch's detached children
		let mut child_ids = Vec::new();
		for c in &n.children {
			child_ids.push(self.walk(c, ix, n.kind == nk::EACH));
		}
		let mut patches = Vec::new();
		for p in &n.patches {
			let mut det = Vec::new();
			for c in &p.children {
				let cid = self.walk(c, ix, true);
				child_ids.push(cid);
				det.push(cid);
			}
			patches.push(self.encode_patch(ix, p, det));
		}
		for (node_ix, ep) in patches {
			self.patch_specs.push((node_ix, ep));
		}
		// sibling links
		for w in child_ids.windows(2) {
			self.slir.nodes.next_sib[w[0] as usize] = w[1];
		}
		if let Some(&first) = child_ids.first() {
			self.slir.nodes.first_child[ix as usize] = first;
		}
		ix
	}

	fn encode_patch(&mut self, node: u32, p: &CPatch, children: Vec<u32>) -> (u32, EPatch) {
		let cond = self.cond_enc(&p.cond);
		let mut attrs: Vec<(u16, u32)> = p
			.attrs
			.iter()
			.map(|e| (e.id, self.aval_of(&e.val, node, e.id)))
			.collect();
		if p.flag_mask != 0 {
			attrs.push((at::FLAGS, self.aval(av::NUM, (p.flag_mask as f64).to_bits())));
		}
		attrs.sort_by_key(|&(id, _)| id);
		(node, EPatch { cond, attrs, children })
	}
}

/// Emit an expanded document as a `Slir` value.
pub fn emit(ex: &Expanded, opts: &Options, diags: &mut Diagnostics) -> Slir {
	warn_missing_glyphs(ex, opts, diags);
	let mut em = Emitter {
		slir: Slir::default(),
		str_ix: HashMap::new(),
		aval_ix: HashMap::new(),
		tuple_ix: HashMap::new(),
		tup_dyn_ix: HashMap::new(),
		shadow_ix: HashMap::new(),
		grad_ix: HashMap::new(),
		path_ix: HashMap::new(),
		runs: Vec::new(),
		patch_specs: Vec::new(),
		sites: vec![Vec::new(); ex.params.len()],
		anim_names: ex.anims.iter().map(|a| a.name.clone()).collect(),
		diags,
	};
	em.intern(""); // string 0 is always the empty string
	// Public logical rows must precede typed use-site rows so path lookup finds
	// the context-independent host value first.
	for token in &ex.tokens {
		em.public_token(token);
	}

	// root: a single root is node 0; multiple roots get a synthesized col
	if ex.roots.len() == 1 {
		em.walk(&ex.roots[0], NONE, false);
	} else {
		let ix = 0u32;
		let nodes = &mut em.slir.nodes;
		nodes.kind.push(nk::COL);
		nodes.flags.push(0);
		nodes.parent.push(NONE);
		nodes.first_child.push(NONE);
		nodes.next_sib.push(NONE);
		nodes.key.push(0);
		nodes.id.push(0);
		nodes.src_line.push(0);
		let w = em.aval(av::SIZE_FILL, 1.0f64.to_bits());
		em.runs.push(vec![(at::W, w)]);
		let mut child_ids = Vec::new();
		for r in &ex.roots {
			child_ids.push(em.walk(r, ix, false));
		}
		for w in child_ids.windows(2) {
			em.slir.nodes.next_sib[w[0] as usize] = w[1];
		}
		if let Some(&first) = child_ids.first() {
			em.slir.nodes.first_child[ix as usize] = first;
		}
	}

	// ICON: detached static vector subtrees are emitted after the live root.
	for icon in &ex.icons {
		let name = em.intern(&icon.name);
		let node = em.walk(&icon.root, NONE, true);
		em.slir
			.icons
			.push(IconE { name, node, viewbox: icon.viewbox });
	}

	// ATTR fenceposts
	let mut index = Vec::with_capacity(em.runs.len() + 1);
	index.push(0u32);
	for run in &em.runs {
		let prev = *index.last().unwrap();
		index.push(prev + run.len() as u32);
		em.slir.attrs.extend_from_slice(run);
	}
	em.slir.attr_index = index;

	// WHEN pools: stable per-node grouping, document order within a node
	em.patch_specs.sort_by_key(|&(node, _)| node);
	// dedup identical conds
	let mut cond_ix: HashMap<(u8, u8, u8, u64, u32), u32> = HashMap::new();
	let specs = std::mem::take(&mut em.patch_specs);
	for (node, ep) in specs {
		let ckey = (ep.cond.kind, ep.cond.neg, ep.cond.op, ep.cond.num.to_bits(), ep.cond.sym);
		let cix = if let Some(&c) = cond_ix.get(&ckey) {
			c
		} else {
			let c = em.slir.conds.len() as u32;
			em.slir.conds.push(CondE {
				kind: ep.cond.kind,
				neg:  ep.cond.neg,
				op:   ep.cond.op,
				num:  ep.cond.num,
				sym:  ep.cond.sym,
			});
			cond_ix.insert(ckey, c);
			c
		};
		let attr_off = em.slir.patch_attrs.len() as u32;
		em.slir.patch_attrs.extend_from_slice(&ep.attrs);
		let child_off = em.slir.patch_children.len() as u32;
		em.slir.patch_children.extend_from_slice(&ep.children);
		em.slir.patches.push(PatchE {
			node,
			cond: cix,
			attr_off,
			attr_len: ep.attrs.len() as u32,
			child_off,
			child_len: ep.children.len() as u32,
		});
	}

	// ANIM tables
	for anim in &ex.anims {
		let name = em.intern(&anim.name);
		let mut stops = Vec::new();
		for (pos, attrs) in &anim.stops {
			let mut run: Vec<(u16, u32)> = attrs
				.iter()
				.map(|e| (e.id, em.aval_of(&e.val, NONE, e.id)))
				.collect();
			run.sort_by_key(|&(id, _)| id);
			let off = em.slir.anim_attrs.len() as u32;
			let len = run.len() as u32;
			em.slir.anim_attrs.extend_from_slice(&run);
			stops.push((*pos, off, len));
		}
		em.slir.anims.push(AnimE { name, stops });
	}

	// Canonical sub-schema rows come first and are never host-addressable.
	// Every list_field_sub points into this prefix, whose list_param entries
	// are the required NONE sentinel. Root params get separate rows below.
	for schema in &ex.list_schemas {
		em.list_schema(schema, NONE);
	}
	for (param, info) in ex.params.iter().enumerate() {
		let Some(row) = info.list else {
			continue;
		};
		em.list_schema(&ex.list_schemas[row as usize], param as u32);
	}

	// PARM
	for (i, p) in ex.params.iter().enumerate() {
		let name = em.intern(&p.name);
		let default = match (&p.default, &p.ty) {
			(TVal::Str(s), _) => {
				let sx = em.intern(s);
				em.aval(av::STR, sx as u64)
			},
			(TVal::Num(x), _) => em.aval(av::NUM, x.to_bits()),
			(TVal::Size(SizeSpec::Pct(x)), _) => em.aval(av::PCT, x.to_bits()),
			(TVal::Color(c), _) => em.aval(av::COLOR, rgba_word(*c) as u64),
			(TVal::Enum(sym), _) => {
				let sx = em.intern(sym);
				em.aval(av::ENUM_SYM, sx as u64)
			},
			(other, _) => em.aval_of(other, NONE, 0),
		};
		let enum_off = em.slir.param_enum_syms.len() as u32;
		for sym in &p.enum_syms {
			let sx = em.intern(sym);
			em.slir.param_enum_syms.push(sx);
		}
		let enum_len = p.enum_syms.len() as u32;
		let site_off = em.slir.param_sites.len() as u32;
		let sites = std::mem::take(&mut em.sites[i]);
		let site_len = sites.len() as u32;
		em.slir.param_sites.extend_from_slice(&sites);
		let ty = match &p.ty {
			slab_syntax::ast::ParamType::Text => 0,
			slab_syntax::ast::ParamType::Num => 1,
			slab_syntax::ast::ParamType::Pct => 2,
			slab_syntax::ast::ParamType::Color => 3,
			slab_syntax::ast::ParamType::Bool => 4,
			slab_syntax::ast::ParamType::Enum => 5,
			slab_syntax::ast::ParamType::List(_) => 6,
		};
		em.slir
			.params
			.push(ParamE { name, ty, default, enum_off, enum_len, site_off, site_len });
	}

	// THEM
	for theme in &ex.themes {
		let name = em.intern(theme);
		em.slir.themes.push(name);
	}

	// FONT tables cross every authored or statically knowable property-bound
	// family with every required snapped weight. The class supplies fallback
	// metrics. Tables backed by a vendored asset stay metrics-only — every
	// kernel resolves the identical bundled face at run time — while
	// host-supplied compile-time faces embed their sfnt bytes.
	let mut families = BTreeSet::from([String::new()]);
	let mut weights = BTreeSet::from([400u16]);
	families.extend(ex.font_families.iter().cloned());
	weights.extend(ex.font_weights.iter().copied());
	for root in &ex.roots {
		collect_node_fonts(root, &mut families, &mut weights);
	}
	for anim in &ex.anims {
		for (_, attrs) in &anim.stops {
			collect_font_attrs(attrs, &mut families, &mut weights);
		}
	}
	for family in families {
		let class = font_assets::classify_family(&family);
		let custom = opts
			.fonts
			.iter()
			.find(|(name, _)| name.eq_ignore_ascii_case(&family))
			.map(|(_, bytes)| bytes.as_slice());
		for &weight in &weights {
			let bytes = custom.unwrap_or_else(|| font_assets::asset(class, weight).bytes);
			let mut table = fonts::build_table(class, weight, bytes);
			if let Some(custom) = custom {
				table.data = custom.to_vec();
			}
			table.family = em.intern(&family);
			em.slir.fonts.push(table);
		}
	}

	// IMGS
	for (src, line) in &ex.images {
		let src_ix = em.intern(src);
		let path = opts.base_dir.join(src);
		let mut img =
			ImgE { src: src_ix, w: 0, h: 0, format: 0, blob_off: 0, blob_len: 0 };
		match match &opts.assets {
			Some(m) => m.get(src.as_str()).cloned(),
			None => std::fs::read(&path).ok(),
		} {
			Some(bytes) => {
				if bytes.len() > 24 && bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
					img.w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
					img.h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
				}
				if opts.embed_assets {
					img.blob_off = em.slir.blob.len() as u32;
					img.blob_len = bytes.len() as u32;
					em.slir.blob.extend_from_slice(&bytes);
				}
			},
			None => {
				em.diags
					.warn("attr", format!("image not found: {src}"), *line);
			},
		}
		em.slir.images.push(img);
	}

	em.slir
}
