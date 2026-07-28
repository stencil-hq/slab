//! Web-component generation.
//!
//! Moved lib-side from the CLI so the wasm build can emit the same `gen wc`
//! outputs. Produces a self-contained browser ES module per document plus the
//! shared minified web client and single Rust kernel compiled to WASM, bundled
//! once by `just gen` and baked into the binary (so `gen wc` is relocatable and
//! bun-free at run time).
//!
//! Outputs (deterministic, byte-stable across runs):
//! - `<stem>.js`  — plain browser ES module; imports `./slab-runtime.js`.
//! - `<stem>.d.ts`— typed declarations + `HTMLElementTagNameMap` entries.
//! - `slab-runtime.js` — the shared text runtime bundle.
//! - `wasm/slab_kernel_bg.wasm` — the binary Rust kernel WASM sidecar.
//! - `--separate-ir` additionally emits `<stem>.slir` (and one per export).

use std::fmt::Write as _;

use serde_json::json;
use slab_slir::Slir;
use slab_syntax::{ast::ParamType, diag::Diagnostics};

const MODULE_TEMPLATE: &str = include_str!("../templates/wc_module.tmpl");
const DTS_TEMPLATE: &str = include_str!("../templates/wc_dts.tmpl");
use crate::{
	Options,
	export::{ExportProp, compile_export, exported_def_names},
};

/// `gen wc` options.
pub struct WcOptions {
	/// Override the main element tag (default `slab-<stem>`).
	pub tag:         Option<String>,
	/// Emit separate `.slir` files the classes fetch at runtime.
	pub separate_ir: bool,
}

/// One emitted file. `text` flags UTF-8 (the module, declarations, and runtime
/// source) versus binary (the kernel WASM and separate `.slir` blobs).
pub struct WcFile {
	pub name:  String,
	pub bytes: Vec<u8>,
	pub text:  bool,
}

#[derive(Clone)]
pub(crate) struct ListFieldSpec {
	pub(crate) name:      String,
	pub(crate) ty:        u8,
	pub(crate) enum_syms: Vec<String>,
	/// Zero for scalar fields, otherwise one plus the nested schema row.
	pub(crate) sub:       u32,
}

#[derive(Clone)]
pub(crate) struct ListSpec {
	pub(crate) name:   String,
	pub(crate) param:  u32,
	pub(crate) fields: Vec<ListFieldSpec>,
	pub(crate) row:    u32,
}

pub(crate) struct DocSpec {
	pub(crate) tag:       String,
	pub(crate) class:     String,
	pub(crate) bytes:     Vec<u8>,
	pub(crate) ir_name:   String,
	pub(crate) params:    Vec<(String, ParamType)>,
	pub(crate) lists:     Vec<ListSpec>,
	pub(crate) list_rows: Vec<Vec<ListFieldSpec>>,
	pub(crate) signals:   Vec<(String, bool)>,
	pub(crate) keys:      Vec<(String, String)>,
	pub(crate) item_keys: Vec<ItemKeyGroup>,
}

const fn param_type(ty: u8) -> ParamType {
	match ty {
		1 => ParamType::Num,
		2 => ParamType::Pct,
		3 => ParamType::Color,
		4 => ParamType::Bool,
		5 => ParamType::Enum,
		6 => ParamType::List(String::new()),
		_ => ParamType::Text,
	}
}

/// TS type of a scalar param or list field.
pub(crate) const fn ts_type(ty: &ParamType) -> &'static str {
	match ty {
		ParamType::Text | ParamType::Enum => "string",
		ParamType::Num => "number",
		ParamType::Pct => "number | string",
		ParamType::Color => "string | number",
		ParamType::Bool => "boolean",
		ParamType::List(_) => "unknown[]",
	}
}

pub(crate) fn ts_field_type(field: &ListFieldSpec) -> String {
	if field.ty == 5 && !field.enum_syms.is_empty() {
		return field
			.enum_syms
			.iter()
			.map(|s| js_string(s))
			.collect::<Vec<_>>()
			.join(" | ");
	}
	if field.ty == 6 {
		return "unknown[]".to_string();
	}
	ts_type(&param_type(field.ty)).to_string()
}

pub(crate) fn js_string(s: &str) -> String {
	let mut out = String::from("\"");
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if c < ' ' => {
				let _ = write!(out, "\\u{:04x}", u32::from(c));
			},
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

fn list_fields(slir: &Slir, row: usize) -> Vec<ListFieldSpec> {
	let list = &slir.lists[row];
	let start = list.field_off as usize;
	let end = start.saturating_add(list.field_len as usize);
	slir
		.list_fields
		.get(start..end)
		.unwrap_or(&[])
		.iter()
		.map(|field| {
			let enum_start = field.enum_off as usize;
			let enum_end = enum_start.saturating_add(field.enum_len as usize);
			ListFieldSpec {
				name:      slir.str_at(field.name).to_string(),
				ty:        field.ty,
				enum_syms: slir
					.list_enum_syms
					.get(enum_start..enum_end)
					.unwrap_or(&[])
					.iter()
					.map(|&sym| slir.str_at(sym).to_string())
					.collect(),
				sub:       field.sub,
			}
		})
		.collect()
}

fn list_rows_of(slir: &Slir) -> Vec<Vec<ListFieldSpec>> {
	(0..slir.lists.len())
		.map(|row| list_fields(slir, row))
		.collect()
}

fn lists_of(slir: &Slir) -> Vec<ListSpec> {
	slir
		.lists
		.iter()
		.enumerate()
		.filter_map(|(row, list)| {
			let param = slir.params.get(list.param as usize)?;
			Some(ListSpec {
				name:   slir.str_at(param.name).to_string(),
				param:  list.param,
				fields: list_fields(slir, row),
				row:    row as u32,
			})
		})
		.collect()
}

pub(crate) fn pascal(s: &str) -> String {
	let mut out = String::new();
	let mut up = true;
	for c in s.chars() {
		if c.is_ascii_alphanumeric() {
			if up && c.is_ascii_alphabetic() {
				out.push(c.to_ascii_uppercase());
			} else {
				out.push(c);
			}
			up = false;
		} else {
			up = true;
		}
	}
	out
}

/// `LogRow` → `log-row` (custom-element tag segment).
fn kebab(s: &str) -> String {
	let mut out = String::new();
	for (i, c) in s.chars().enumerate() {
		if c.is_ascii_uppercase() {
			if i > 0 {
				out.push('-');
			}
			out.push(c.to_ascii_lowercase());
		} else if c.is_ascii_alphanumeric() || c == '-' {
			out.push(c);
		} else {
			out.push('-');
		}
	}
	out
}

fn sanitize_tag(s: &str) -> String {
	s.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() || c == '-' {
				c.to_ascii_lowercase()
			} else {
				'-'
			}
		})
		.collect()
}

fn base64(data: &[u8]) -> String {
	const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
	for chunk in data.chunks(3) {
		let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
		let v = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
		out.push(T[(v >> 18) as usize & 63] as char);
		out.push(T[(v >> 12) as usize & 63] as char);
		out.push(if chunk.len() > 1 {
			T[(v >> 6) as usize & 63] as char
		} else {
			'='
		});
		out.push(if chunk.len() > 2 {
			T[v as usize & 63] as char
		} else {
			'='
		});
	}
	out
}

fn same_list_row(doc: &DocSpec, left: usize, right: usize) -> bool {
	let left = &doc.list_rows[left];
	let right = &doc.list_rows[right];
	left.len() == right.len()
		&& left
			.iter()
			.zip(right)
			.all(|(a, b)| a.name == b.name && a.ty == b.ty && a.sub == b.sub)
}

fn canonical_list_row(doc: &DocSpec, row: usize) -> usize {
	(0..=row)
		.find(|&candidate| same_list_row(doc, candidate, row))
		.unwrap_or(row)
}

pub(crate) fn collect_ts_list_types(
	doc: &DocSpec,
	row: usize,
	name: String,
	names: &mut Vec<(usize, String)>,
	order: &mut Vec<usize>,
) {
	let row = canonical_list_row(doc, row);
	if names.iter().any(|(candidate, _)| *candidate == row) {
		return;
	}
	names.push((row, format!("{name}Item")));
	order.push(row);
	for field in &doc.list_rows[row] {
		if field.sub != 0 {
			collect_ts_list_types(
				doc,
				field.sub as usize - 1,
				format!("{name}{}", pascal(&field.name)),
				names,
				order,
			);
		}
	}
}

pub(crate) fn ts_list_type(names: &[(usize, String)], doc: &DocSpec, row: usize) -> String {
	let row = canonical_list_row(doc, row);
	names
		.iter()
		.find(|(candidate, _)| *candidate == row)
		.map(|(_, name)| name.clone())
		.expect("nested TypeScript list type was not collected")
}

fn is_each_template_descendant(slir: &Slir, node: usize) -> bool {
	let mut parent = slir
		.nodes
		.parent
		.get(node)
		.copied()
		.unwrap_or(slab_slir::NONE);
	while parent != slab_slir::NONE {
		let Ok(parent_index) = usize::try_from(parent) else {
			return false;
		};
		if slir.nodes.kind.get(parent_index) == Some(&slab_slir::kind::EACH) {
			return true;
		}
		parent = slir
			.nodes
			.parent
			.get(parent_index)
			.copied()
			.unwrap_or(slab_slir::NONE);
	}
	false
}

/// Authored IDs whose canonical keys exist without a runtime list item key.
pub(crate) fn static_scene_keys(slir: &Slir) -> Vec<(String, String)> {
	let mut keys = Vec::new();
	let mut counts = std::collections::HashMap::<String, usize>::new();
	for (node, (&id_ref, &key_ref)) in slir.nodes.id.iter().zip(&slir.nodes.key).enumerate() {
		let id = slir.str_at(id_ref);
		if id.is_empty() || is_each_template_descendant(slir, node) {
			continue;
		}
		let count = counts.entry(id.to_string()).or_default();
		*count += 1;
		let name = if *count == 1 {
			id.to_string()
		} else {
			format!("{id}_{}", *count)
		};
		keys.push((name, slir.str_at(key_ref).to_string()));
	}
	keys
}

/// One `each` list's composable key constants: the each node's own key (full
/// for document-level eaches, template-relative for nested ones) and the
/// template-relative keys of authored ids inside its item template. Hosts
/// join them as `each~item/relative` instead of hand-assembling paths.
pub(crate) struct ItemKeyGroup {
	pub(crate) name:     String,
	pub(crate) each_key: String,
	pub(crate) items:    Vec<(String, String)>,
}

/// Nearest EACH ancestor of `node`, or `None` outside every item template.
fn nearest_each(slir: &Slir, node: usize) -> Option<usize> {
	let mut parent = slir
		.nodes
		.parent
		.get(node)
		.copied()
		.unwrap_or(slab_slir::NONE);
	while parent != slab_slir::NONE {
		let parent_index = usize::try_from(parent).ok()?;
		if slir.nodes.kind.get(parent_index) == Some(&slab_slir::kind::EACH) {
			return Some(parent_index);
		}
		parent = slir
			.nodes
			.parent
			.get(parent_index)
			.copied()
			.unwrap_or(slab_slir::NONE);
	}
	None
}

/// A generated identifier for one canonical key segment (`#title` → `title`,
/// `each@0` → `each_0`); digit-leading results gain an `e` prefix.
fn segment_ident(segment: &str) -> String {
	let trimmed = segment.trim_start_matches('#');
	let mut ident: String = trimmed
		.chars()
		.map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
		.collect();
	if ident.is_empty() || ident.starts_with(|ch: char| ch.is_ascii_digit()) {
		ident.insert(0, 'e');
	}
	ident
}

/// Composable per-each key constants: every EACH node in document order,
/// with the authored ids of its own item template (nested eaches own theirs).
pub(crate) fn item_scene_keys(slir: &Slir) -> Vec<ItemKeyGroup> {
	let mut groups: Vec<ItemKeyGroup> = Vec::new();
	let mut group_of = std::collections::HashMap::<usize, usize>::new();
	let mut group_counts = std::collections::HashMap::<String, usize>::new();
	for (node, &kind) in slir.nodes.kind.iter().enumerate() {
		if kind != slab_slir::kind::EACH {
			continue;
		}
		let each_key = slir.str_at(slir.nodes.key[node]).to_string();
		let segment = each_key.rsplit('/').next().unwrap_or(&each_key);
		let base = segment_ident(segment);
		let count = group_counts.entry(base.clone()).or_default();
		*count += 1;
		let name = if *count == 1 {
			base
		} else {
			format!("{base}_{}", *count)
		};
		group_of.insert(node, groups.len());
		groups.push(ItemKeyGroup { name, each_key, items: Vec::new() });
	}
	let mut item_counts = vec![std::collections::HashMap::<String, usize>::new(); groups.len()];
	for (node, &id_ref) in slir.nodes.id.iter().enumerate() {
		let id = slir.str_at(id_ref);
		if id.is_empty() {
			continue;
		}
		let Some(each) = nearest_each(slir, node) else {
			continue;
		};
		let group = group_of[&each];
		let base = segment_ident(id);
		let count = item_counts[group].entry(base.clone()).or_default();
		*count += 1;
		let name = if *count == 1 {
			base
		} else {
			format!("{base}_{}", *count)
		};
		groups[group]
			.items
			.push((name, slir.str_at(slir.nodes.key[node]).to_string()));
	}
	groups
}

/// The component module is plain browser JS (no build step): it imports the
/// shared minified runtime emitted next to it.
fn merged_signals(docs: &[DocSpec]) -> Vec<(&str, bool)> {
	let mut merged = Vec::new();
	for (name, has_text) in docs.iter().flat_map(|doc| &doc.signals) {
		if let Some((_, merged_has_text)) =
			merged.iter_mut().find(|(candidate, _)| *candidate == name)
		{
			*merged_has_text |= *has_text;
		} else {
			merged.push((name.as_str(), *has_text));
		}
	}
	merged
}

fn emit_module(docs: &[DocSpec], separate_ir: bool) -> String {
	let docs_json: Vec<serde_json::Value> = docs
		.iter()
		.map(|d| {
			let mut observed_attributes = vec!["'theme'".to_string()];
			observed_attributes.extend(
				d.params
					.iter()
					.filter(|(n, _)| n != "theme")
					.map(|(n, _)| format!("'{}'", n.replace(['_', '.'], "-"))),
			);

			let list_rows_json: Vec<serde_json::Value> = d
				.list_rows
				.iter()
				.map(|fields| {
					let fields_json: Vec<serde_json::Value> = fields
						.iter()
						.map(|f| {
							let enum_values: Vec<String> =
								f.enum_syms.iter().map(|v| js_string(v)).collect();
							json!({
								"name": f.name,
								"ty": f.ty,
								"sub": f.sub,
								"enum_syms": f.enum_syms,
								"enum_values": enum_values,
							})
						})
						.collect();
					json!(fields_json)
				})
				.collect();

			let lists_json: Vec<serde_json::Value> = d
				.lists
				.iter()
				.map(|l| {
					let fields_json: Vec<serde_json::Value> = l
						.fields
						.iter()
						.map(|f| {
							let enum_values: Vec<String> =
								f.enum_syms.iter().map(|v| js_string(v)).collect();
							json!({
								"name": f.name,
								"ty": f.ty,
								"sub": f.sub,
								"enum_syms": f.enum_syms,
								"enum_values": enum_values,
							})
						})
						.collect();
					json!({
						"name": l.name,
						"param": l.param,
						"row": l.row,
						"fields": fields_json,
					})
				})
				.collect();

			let params_json: Vec<serde_json::Value> = d
				.params
				.iter()
				.map(|(n, _)| {
					let is_list = d.lists.iter().any(|list| list.name == *n);
					json!({
						"name": n,
						"is_list": is_list,
					})
				})
				.collect();

			let keys_json: Vec<serde_json::Value> = d
				.keys
				.iter()
				.map(|(name, key)| {
					json!({
						"name": name,
						"key": key,
					})
				})
				.collect();

			let item_keys_json: Vec<serde_json::Value> = d
				.item_keys
				.iter()
				.map(|group| {
					let items_json: Vec<serde_json::Value> = group
						.items
						.iter()
						.map(|(name, key)| {
							json!({
								"name": name,
								"key": key,
							})
						})
						.collect();
					json!({
						"name": group.name,
						"each_key": group.each_key,
						"items": items_json,
					})
				})
				.collect();

			json!({
				"tag": d.tag,
				"class": d.class,
				"ir_name": d.ir_name,
				"slir_base64": base64(&d.bytes),
				"observed_attributes": observed_attributes,
				"list_rows": list_rows_json,
				"lists": lists_json,
				"params": params_json,
				"keys": keys_json,
				"item_keys": item_keys_json,
			})
		})
		.collect();

	let merged_signals_json: Vec<serde_json::Value> = merged_signals(docs)
		.into_iter()
		.map(|(name, has_text)| {
			let detail = if has_text {
				"{ text: '', item: '', meta: { x: -1, y: -1, dx: 0, dy: 0, drag_dx: 0, drag_dy: 0, \
				 mods: 0, button: 0, clicks: 0, key: '', src_key: '', src_item: '', cancelled: false, \
				 dropped: false } }"
			} else {
				"{ item: '', meta: { x: -1, y: -1, dx: 0, dy: 0, drag_dx: 0, drag_dy: 0, mods: 0, \
				 button: 0, clicks: 0, key: '', src_key: '', src_item: '', cancelled: false, dropped: \
				 false } }"
			};
			json!({
				"name": name,
				"detail": detail,
			})
		})
		.collect();

	let ctx = json!({
		"docs": docs_json,
		"separate_ir": separate_ir,
		"merged_signals": merged_signals_json,
	});

	crate::tmpl::render(MODULE_TEMPLATE, &ctx).expect("wc_module.tmpl render error")
}

fn emit_dts(docs: &[DocSpec]) -> String {
	let mut emitted_interfaces = std::collections::HashSet::new();
	let mut list_interfaces: Vec<serde_json::Value> = Vec::new();

	for d in docs {
		for list in &d.lists {
			let mut names = Vec::new();
			let mut order = Vec::new();
			collect_ts_list_types(d, list.row as usize, pascal(&list.name), &mut names, &mut order);
			for row in order {
				let item_name = ts_list_type(&names, d, row);
				if !emitted_interfaces.insert(item_name.clone()) {
					continue;
				}
				let fields_json: Vec<serde_json::Value> = d.list_rows[row]
					.iter()
					.map(|field| {
						let field_ty = if field.sub == 0 {
							ts_field_type(field)
						} else {
							format!("{}[]", ts_list_type(&names, d, field.sub as usize - 1))
						};
						json!({
							"name": field.name,
							"ts_type": field_ty,
						})
					})
					.collect();

				list_interfaces.push(json!({
					"name": item_name,
					"fields": fields_json,
				}));
			}
		}
	}

	let docs_json: Vec<serde_json::Value> = docs
		.iter()
		.map(|d| {
			let keys_json: Vec<serde_json::Value> = d
				.keys
				.iter()
				.map(|(name, key)| json!({ "name": name, "key": key }))
				.collect();

			let item_keys_json: Vec<serde_json::Value> = d
				.item_keys
				.iter()
				.map(|group| {
					let items_json: Vec<serde_json::Value> = group
						.items
						.iter()
						.map(|(name, key)| json!({ "name": name, "key": key }))
						.collect();
					json!({
						"name": group.name,
						"each_key": group.each_key,
						"items": items_json,
					})
				})
				.collect();

			let params_json: Vec<serde_json::Value> = d
				.params
				.iter()
				.map(|(n, ty)| {
					let is_list = d.lists.iter().any(|list| list.name == *n);
					json!({
						"name": n,
						"is_list": is_list,
						"pascal_name": pascal(n),
						"ts_type": ts_type(ty),
					})
				})
				.collect();

			json!({
				"tag": d.tag,
				"class": d.class,
				"keys": keys_json,
				"item_keys": item_keys_json,
				"params": params_json,
				"has_lists": !d.lists.is_empty(),
			})
		})
		.collect();

	let merged_signals_json: Vec<serde_json::Value> = merged_signals(docs)
		.into_iter()
		.map(|(name, has_text)| {
			json!({
				"name": name,
				"detail_type": if has_text { "TextSignalDetail" } else { "SignalDetail" },
			})
		})
		.collect();

	let ctx = json!({
		"list_interfaces": list_interfaces,
		"docs": docs_json,
		"merged_signals": merged_signals_json,
	});

	crate::tmpl::render(DTS_TEMPLATE, &ctx).expect("wc_dts.tmpl render error")
}

/// The shared minified web client.
///
/// Bundled into `gen/web-runtime/slab-runtime.js` by `just web-runtime`
/// (untracked; rebuild it before compiling this crate). `include_str!` keeps
/// this a text sidecar and bakes it into the binary so `gen wc` needs no bun on
/// PATH.
pub const RUNTIME: &str =
	include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../gen/web-runtime/slab-runtime.js"));

/// The single Rust kernel compiled to WASM and loaded by [`RUNTIME`] from its
/// published relative URL. These bytes are emitted as a binary sidecar rather
/// than base64-inlined into JavaScript.
///
/// This embeds the one wasm-bindgen output under `clients/web/wasm/` that the
/// `@stencil-hq/wslab` package also ships; `gen wc` never gets a second copy.
pub const KERNEL_WASM: &[u8] = include_bytes!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../../clients/web/wasm/slab_kernel_bg.wasm"
));

/// Collect signals `(name, has_text)` in SLIR order, deduped.
fn signals_of(slir: &Slir) -> Vec<(String, bool)> {
	let mut sigs: Vec<(String, bool)> = Vec::new();
	for &(name_ref, _, trigger) in &slir.signals {
		let name = slir.str_at(name_ref).to_string();
		let has_text = matches!(trigger, 1 | 2 | 8 | 14);
		if let Some((_, previous_has_text)) =
			sigs.iter_mut().find(|(candidate, _)| *candidate == name)
		{
			*previous_has_text |= has_text;
		} else {
			sigs.push((name, has_text));
		}
	}
	sigs
}

/// Compile the main document and every exported definition into the
/// [`DocSpec`] list shared by `gen wc` and `gen react`. `None` on compile
/// failure.
pub(crate) fn doc_specs(
	src: &str,
	copts: &Options,
	w: &WcOptions,
	stem: &str,
) -> (Option<Vec<DocSpec>>, Diagnostics) {
	let mut diags = Diagnostics::new();
	let units = crate::import::closure(src, copts, &mut diags);
	let Some(slir) = crate::compile_units(&units, copts, &mut diags) else {
		return (None, diags);
	};

	let main_tag = w
		.tag
		.clone()
		.unwrap_or_else(|| format!("slab-{}", sanitize_tag(stem)));
	let mut docs: Vec<DocSpec> = Vec::new();
	let params: Vec<(String, ParamType)> = slir
		.params
		.iter()
		.map(|p| (slir.str_at(p.name).to_string(), param_type(p.ty)))
		.collect();
	docs.push(DocSpec {
		tag: main_tag,
		class: format!("Slab{}Element", pascal(stem)),
		bytes: slab_slir::write(&slir),
		ir_name: format!("{stem}.slir"),
		params,
		lists: lists_of(&slir),
		list_rows: list_rows_of(&slir),
		signals: signals_of(&slir),
		keys: static_scene_keys(&slir),
		item_keys: item_scene_keys(&slir),
	});

	// Exported defs skip asset embedding: the page shares one registered
	// face set (clients/web registeredFaces), so re-shipping font blobs per
	// element class would only bloat the module.
	let def_opts = Options {
		embed_assets: false,
		base_dir:     copts.base_dir.clone(),
		assets:       None,
		sources:      copts.sources.clone(),
		fonts:        copts.fonts.clone(),
	};
	for def in exported_def_names(&units) {
		let (dslir, ddiags, props) = compile_export(&units, &def, &def_opts);
		diags.0.extend(ddiags.0);
		let Some(dslir) = dslir else {
			return (None, diags);
		};
		docs.push(DocSpec {
			tag:       format!("slab-{}", kebab(&def)),
			class:     format!("Slab{}Element", pascal(&def)),
			bytes:     slab_slir::write(&dslir),
			ir_name:   format!("{stem}.{}.slir", kebab(&def)),
			params:    props
				.iter()
				.map(|ExportProp { name, ty }| (name.clone(), ty.clone()))
				.collect(),
			lists:     lists_of(&dslir),
			list_rows: list_rows_of(&dslir),
			signals:   signals_of(&dslir),
			keys:      static_scene_keys(&dslir),
			item_keys: item_scene_keys(&dslir),
		});
	}
	(Some(docs), diags)
}

/// Assemble the `gen wc` file set for compiled [`DocSpec`]s: the module,
/// declarations, runtime and kernel sidecars, plus `.slir` blobs under
/// `--separate-ir`.
pub(crate) fn files_of(docs: &[DocSpec], w: &WcOptions, stem: &str) -> Vec<WcFile> {
	let module = emit_module(docs, w.separate_ir);
	let dts = emit_dts(docs);
	let mut files = Vec::new();
	if w.separate_ir {
		for d in docs {
			files.push(WcFile { name: d.ir_name.clone(), bytes: d.bytes.clone(), text: false });
		}
	}
	files.push(WcFile { name: format!("{stem}.js"), bytes: module.into_bytes(), text: true });
	files.push(WcFile { name: format!("{stem}.d.ts"), bytes: dts.into_bytes(), text: true });
	files.push(WcFile {
		name:  "slab-runtime.js".to_string(),
		bytes: RUNTIME.as_bytes().to_vec(),
		text:  true,
	});
	files.push(WcFile {
		name:  "wasm/slab_kernel_bg.wasm".to_string(),
		bytes: KERNEL_WASM.to_vec(),
		text:  false,
	});
	files
}

/// Generate the full `gen wc` file set for a `.slab` source.
///
/// `stem` is the output basename (the CLI passes the input file stem). Every
/// successful set includes the text `slab-runtime.js` and binary
/// `wasm/slab_kernel_bg.wasm` sidecars, plus `.slir` blobs under
/// `--separate-ir`; the file list is `None` on compile failure.
pub fn generate(
	src: &str,
	copts: &Options,
	w: &WcOptions,
	stem: &str,
) -> (Option<Vec<WcFile>>, Diagnostics) {
	let (docs, diags) = doc_specs(src, copts, w, stem);
	let Some(docs) = docs else {
		return (None, diags);
	};
	(Some(files_of(&docs, w, stem)), diags)
}

#[cfg(test)]
mod tests {
	use super::{KERNEL_WASM, WcOptions, generate, item_scene_keys, static_scene_keys};
	use crate::Options;

	#[test]
	fn generate_always_emits_binary_kernel_wasm_at_runtime_url() {
		for separate_ir in [false, true] {
			let options = WcOptions { tag: None, separate_ir };
			let (files, _) = generate("text \"hello\"", &Options::default(), &options, "hello");
			let files = files.expect("minimal document should compile");
			let wasm = files
				.iter()
				.find(|file| file.name == "wasm/slab_kernel_bg.wasm")
				.expect("kernel WASM sidecar");
			assert!(!wasm.text);
			assert_eq!(wasm.bytes.as_slice(), KERNEL_WASM);

			let runtime = files
				.iter()
				.find(|file| file.name == "slab-runtime.js")
				.expect("JavaScript runtime sidecar");
			assert!(runtime.text);
		}
	}

	#[test]
	fn generated_signal_details_share_typed_metadata() {
		let options = WcOptions { tag: None, separate_ir: false };
		let source = r"
row {
  box press=pressed context=menu dblclick=twice drag=started pointer-move=moved pointer-up=released drag-update=updated drag-end=ended drop=dropped
  divider w=6 resize=resized
  box
}
";
		let (files, diagnostics) = generate(
			source,
			&Options { embed_assets: false, ..Options::default() },
			&options,
			"gestures",
		);
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let files = files.expect("gesture document should compile");
		let module = files
			.iter()
			.find(|file| file.name == "gestures.js")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated JavaScript module");
		let declarations = files
			.iter()
			.find(|file| file.name == "gestures.d.ts")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated declarations");
		assert!(module.contains("meta: { x: -1"));
		assert!(module.contains("'resized': { text: '', item: '', meta:"));
		assert_eq!(declarations.matches("interface SignalMeta").count(), 1);
		assert!(declarations.contains("readonly 'pressed': SignalDetail"));
		assert!(declarations.contains("readonly drag_dx: number"));
		assert!(declarations.contains("readonly cancelled: boolean"));
		assert!(declarations.contains("readonly dropped: boolean"));
		assert!(declarations.contains("readonly 'resized': TextSignalDetail"));
		assert!(declarations.contains("export type SignalName = keyof typeof signals"));
	}

	#[test]
	fn recursive_list_codegen_emits_nested_interfaces_and_schema_rows() {
		let source = r#"
def Tree(label="", children=list(Tree)) export {
  col {
    text label
    each children
  }
}
params {
  trees list(Tree) = [
    Tree(label="root", children=[Tree(label="child")])
  ]
}
col { each param.trees }
"#;
		let options = WcOptions { tag: None, separate_ir: false };
		let (files, diagnostics) = generate(
			source,
			&Options { embed_assets: false, ..Options::default() },
			&options,
			"trees",
		);
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let files = files.expect("recursive web component");
		let module = files
			.iter()
			.find(|file| file.name == "trees.js")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated JavaScript module");
		let declarations = files
			.iter()
			.find(|file| file.name == "trees.d.ts")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated declarations");
		assert!(module.contains("static listSchemaRows = ["));
		assert!(module.contains("name: \"children\", type: 6, sub: 1"));
		assert_eq!(declarations.matches("export interface TreesItem").count(), 1);
		assert!(declarations.contains("\"children\"?: TreesItem[]"));
	}

	#[test]
	fn generated_static_keys_resolve_and_exclude_each_templates() {
		let source = r#"
def Row(label="") export {
  row#item w=100 h=20 focusable { text label }
}
params { rows list(Row) = [Row(label="one")] }
col#app { col#list { each param.rows } }
"#;
		let (slir, diagnostics) =
			crate::compile(source, &Options { embed_assets: false, ..Options::default() });
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let slir = slir.expect("key fixture");
		let keys = static_scene_keys(&slir);
		assert!(keys.iter().any(|(name, _)| name == "app"));
		assert!(keys.iter().any(|(name, _)| name == "list"));
		assert!(!keys.iter().any(|(name, _)| name == "item"));

		let bytes = slab_slir::write(&slir);
		let (mut instance, _) = slab_slir::instance(&bytes).expect("decoded key fixture");
		slab_kernel::frame::inst_set_env(&mut instance, 240.0, 120.0, 0, false, false);
		let _ = slab_kernel::frame::inst_frame(&mut instance, 0.0);
		for (_, key) in keys {
			let node = slab_kernel::scene::node_by_key(instance.doc(), &instance.st.lists, &key);
			assert_ne!(node, slab_kernel::slir::NONE, "generated key {key}");
			assert!(slab_kernel::scene::index_of(&instance.sc, node) >= 0);
		}
		let item = slab_kernel::scene::node_by_key(instance.doc(), &instance.st.lists, "#item");
		assert_ne!(item, slab_kernel::slir::NONE, "materialized item locator");
	}

	#[test]
	fn generated_declarations_expose_kernel_token_lookup() {
		let source = r#"
tokens { color { page #112233 } }
col#canvas bg=color.page { text "tokens" }
"#;
		let options = WcOptions { tag: None, separate_ir: false };
		let (files, diagnostics) = generate(source, &Options::default(), &options, "tokens");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let files = files.expect("token web component");
		let module = files
			.iter()
			.find(|file| file.name == "tokens.js")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated JavaScript module");
		let declarations = files
			.iter()
			.find(|file| file.name == "tokens.d.ts")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated declarations");
		assert!(module.contains("export const SlabTokensElementKeys"));
		assert!(module.contains("\"canvas\": \"#canvas\""));
		assert!(!module.contains("tokenTables"));
		assert!(declarations.contains("getToken(path: string): string | number | undefined"));
		assert!(declarations.contains("focusedKey(): string | null"));
		assert!(declarations.contains("inEditField(): boolean"));
		assert!(declarations.contains("clearFocus(): boolean"));
		assert!(declarations.contains("focusItem(each: string, index: number): boolean"));
		assert!(declarations.contains("focusNote(): string"));
		assert!(declarations.contains("whenSettled(): Promise<void>"));
		assert!(declarations.contains("setFieldText(key: string, text: string): boolean"));
		assert!(declarations.contains("readonly diagnostics: readonly FrameDiagnostic[]"));
		assert!(declarations.contains("type: 'slab-diagnostics'"));
		assert!(declarations.contains("export type SlabTokensElementSceneKey"));
	}
	#[test]
	fn grouped_parameters_use_dotted_properties_and_hyphenated_attributes() {
		let source = r#"
def Row(label="") export { text label }
params editor {
  font_size num = 14
  rows list(Row) = []
}
col {
  rect w=param.editor.font_size
  each param.editor.rows
}
"#;
		let options = WcOptions { tag: None, separate_ir: false };
		let (files, diagnostics) = generate(source, &Options::default(), &options, "editor");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let files = files.expect("grouped parameter web component");
		let module = files
			.iter()
			.find(|file| file.name == "editor.js")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated JavaScript module");
		let declarations = files
			.iter()
			.find(|file| file.name == "editor.d.ts")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated declarations");

		assert!(module.contains("'editor-font-size'"));
		assert!(module.contains("'editor-rows'"));
		assert!(module.contains("get 'editor.font_size'()"));
		assert!(module.contains("setParam('editor.font_size', v)"));
		assert!(declarations.contains("get 'editor.font_size'(): number | undefined"));
		assert!(declarations.contains("get 'editor.rows'(): EditorRowsItem[]"));
	}

	#[test]
	fn item_keys_compose_resolvable_full_paths_and_reach_codegen() {
		let source = r#"
def Row(label="") export {
  row#item w=100 h=20 focusable { text#name label }
}
params { rows list(Row) = [Row(label="one")] }
col#app { col#list { each param.rows key=rows } }
"#;
		let compile_options = Options { embed_assets: false, ..Options::default() };
		let (slir, diagnostics) = crate::compile(source, &compile_options);
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let slir = slir.expect("item key fixture");
		let groups = item_scene_keys(&slir);
		assert_eq!(groups.len(), 1, "one each, one group");
		let group = &groups[0];
		assert_eq!(group.name, "rows");
		assert!(group.items.iter().any(|(name, _)| name == "item"));
		assert!(group.items.iter().any(|(name, _)| name == "name"));

		// Composed `each~item/relative` paths resolve to retained nodes.
		let bytes = slab_slir::write(&slir);
		let (mut instance, _) = slab_slir::instance(&bytes).expect("decoded item key fixture");
		slab_kernel::frame::inst_set_env(&mut instance, 240.0, 120.0, 0, false, false);
		let _ = slab_kernel::frame::inst_frame(&mut instance, 0.0);
		for (name, relative) in &group.items {
			let key = format!("{}~0/{relative}", group.each_key);
			let node = slab_kernel::scene::node_by_key(instance.doc(), &instance.st.lists, &key);
			assert_ne!(node, slab_kernel::slir::NONE, "composed key {name}: {key}");
			assert_eq!(
				slab_kernel::scene::key_of(instance.doc(), &instance.st.lists, node),
				key,
				"composition is canonical"
			);
		}

		// The generated module and declarations expose the same surface.
		let options = WcOptions { tag: None, separate_ir: false };
		let (files, diagnostics) = generate(source, &compile_options, &options, "items");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let files = files.expect("item key web component");
		let module = files
			.iter()
			.find(|file| file.name == "items.js")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated JavaScript module");
		let declarations = files
			.iter()
			.find(|file| file.name == "items.d.ts")
			.and_then(|file| std::str::from_utf8(&file.bytes).ok())
			.expect("generated declarations");
		assert!(module.contains("export { itemKey } from './slab-runtime.js';"));
		assert!(module.contains("export const SlabItemsElementItemKeys = {"));
		assert!(module.contains("\"rows\": {"));
		assert!(declarations.contains("export declare function itemKey("));
		assert!(declarations.contains("export declare const SlabItemsElementItemKeys: {"));
		assert!(declarations.contains("readonly each:"));
	}
}
