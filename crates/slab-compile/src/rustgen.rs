//! Generates typed Rust modules for the native GPU client.
//!
//! Moved lib-side from the CLI so the wasm build can emit the same output.
//! Produces a single-file module embedding the SLIR and exposing a typed
//! `Doc` wrapper over `slab_kernel::frame::Instance`.
//!
//! Output is deterministic. Regenerate + reformat with:
//! `cargo run -q -p slab-cli -- gen rust FILE -o OUT.rs && cargo fmt`

use std::path::PathBuf;

use serde_json::json;
use slab_slir::Slir;
use slab_syntax::diag::Diagnostics;

use crate::{
	Options,
	tmpl::{pascal, snake},
};

const TEMPLATE: &str = include_str!("../templates/rust.tmpl");

/// Generates a typed Rust module for compiled `.slab` source.
///
/// `src_name` is the input file path (used only in the generated header
/// comment). Returns the module source (or `None` on compile failure) and the
/// compile diagnostics.
pub fn generate(src: &str, copts: &Options, src_name: &str) -> (Option<String>, Diagnostics) {
	let (module, diagnostics, _) = generate_with_import_paths(src, copts, src_name);
	(module, diagnostics)
}

/// Generate a typed module and return each filesystem import used by the
/// source.
///
/// Build-time hosts use the paths to register precise rebuild dependencies.
pub fn generate_with_import_paths(
	src: &str,
	copts: &Options,
	src_name: &str,
) -> (Option<String>, Diagnostics, Vec<PathBuf>) {
	let mut diagnostics = Diagnostics::new();
	let units = crate::import::closure(src, copts, &mut diagnostics);
	let imports = units
		.iter()
		.filter_map(|unit| unit.abs.clone())
		.collect::<Vec<_>>();
	let slir = crate::compile_units_with_exports(&units, copts, &mut diagnostics);
	let Some(slir) = slir else {
		return (None, diagnostics, imports);
	};
	let bytes = slab_slir::write(&slir);
	let module = emit_module(&slir, &bytes, src_name);
	(Some(module), diagnostics, imports)
}

/// Unique signals in SIGN order: `(name, has_text)`. A name bound to multiple
/// triggers keeps a text payload when any binding is Change, Submit, Resize,
/// or Cancel (matching the `dup-signal` compile warning's resolution).
fn unique_signals(slir: &Slir) -> Vec<(String, bool)> {
	let mut out: Vec<(String, bool)> = Vec::new();
	for &(name, _node, trigger) in &slir.signals {
		let n = slir.str_at(name).to_string();
		let text_bearing = matches!(trigger, 1 | 2 | 8 | 14);
		match out.iter_mut().find(|(en, _)| *en == n) {
			Some((_, has_text)) => *has_text = *has_text || text_bearing,
			None => out.push((n, text_bearing)),
		}
	}
	out
}

fn byte_string(bytes: &[u8]) -> String {
	let mut s = String::with_capacity(bytes.len() * 4 + 2);
	s.push_str("b\"");
	for &b in bytes {
		let _ = std::fmt::Write::write_fmt(&mut s, format_args!("\\x{b:02X}"));
	}
	s.push('"');
	s
}

fn same_list_schema(slir: &Slir, left: usize, right: usize) -> bool {
	let left = &slir.lists[left];
	let right = &slir.lists[right];
	if left.field_len != right.field_len {
		return false;
	}
	(0..left.field_len).all(|offset| {
		let a = &slir.list_fields[(left.field_off + offset) as usize];
		let b = &slir.list_fields[(right.field_off + offset) as usize];
		a.name == b.name && a.ty == b.ty && a.sub == b.sub
	})
}

fn canonical_list_schema(slir: &Slir, row: usize) -> usize {
	(0..=row)
		.find(|&candidate| same_list_schema(slir, candidate, row))
		.unwrap_or(row)
}

fn collect_list_types(
	slir: &Slir,
	row: usize,
	name: String,
	names: &mut Vec<(usize, String)>,
	order: &mut Vec<usize>,
) {
	let row = canonical_list_schema(slir, row);
	if names.iter().any(|(candidate, _)| *candidate == row) {
		return;
	}
	names.push((row, name.clone()));
	order.push(row);
	let schema = &slir.lists[row];
	for field_ix in schema.field_off..schema.field_off + schema.field_len {
		let field = &slir.list_fields[field_ix as usize];
		if field.sub == 0 {
			continue;
		}
		let child_name = format!("{name}{}", pascal(slir.str_at(field.name)));
		collect_list_types(slir, field.sub as usize - 1, child_name, names, order);
	}
}

fn list_type_name(names: &[(usize, String)], slir: &Slir, row: usize) -> String {
	let row = canonical_list_schema(slir, row);
	names
		.iter()
		.find(|(candidate, _)| *candidate == row)
		.map(|(_, name)| format!("{name}Item"))
		.expect("nested list schema type was not collected")
}

fn emit_module(slir: &Slir, bytes: &[u8], src_name: &str) -> String {
	let keys: Vec<serde_json::Value> = crate::wc::static_scene_keys(slir)
		.into_iter()
		.map(|(name, key)| {
			json!({
				"const_name": snake(&name).to_uppercase(),
				"key": key,
			})
		})
		.collect();

	let item_key_groups: Vec<serde_json::Value> = crate::wc::item_scene_keys(slir)
		.into_iter()
		.map(|group| {
			let items: Vec<serde_json::Value> = group
				.items
				.into_iter()
				.map(|(name, key)| {
					let mut upper = snake(&name).to_uppercase();
					if upper == "EACH" {
						upper.push_str("_ID");
					}
					json!({
						"const_name": upper,
						"key": key,
					})
				})
				.collect();
			json!({
				"name": group.name,
				"snake_name": snake(&group.name),
				"each_key": group.each_key,
				"items": items,
			})
		})
		.collect();

	let params: Vec<serde_json::Value> = slir
		.params
		.iter()
		.enumerate()
		.map(|(i, p)| {
			json!({
				"index": i,
				"const_name": snake(slir.str_at(p.name)).to_uppercase(),
			})
		})
		.collect();

	let holes: Vec<serde_json::Value> = slir
		.holes
		.iter()
		.enumerate()
		.map(|(i, &(name, _node))| {
			json!({
				"index": i,
				"const_name": snake(slir.str_at(name)).to_uppercase(),
			})
		})
		.collect();

	let sigs = unique_signals(slir);
	let signals: Vec<serde_json::Value> = sigs
		.iter()
		.map(|(name, has_text)| {
			let strref = slir
				.signals
				.iter()
				.find(|&&(n, ..)| slir.str_at(n) == name)
				.map_or(0, |&(n, ..)| n);
			json!({
				"name": name,
				"pascal_name": pascal(name),
				"const_name": snake(name).to_uppercase(),
				"strref": strref,
				"has_text": *has_text,
			})
		})
		.collect();

	let mut list_item_types: Vec<serde_json::Value> = Vec::new();
	let mut list_params: Vec<serde_json::Value> = Vec::new();

	for (param_ix, p) in slir.params.iter().enumerate() {
		if p.ty != 6 {
			continue;
		}
		let schema_row = slir
			.lists
			.iter()
			.position(|schema| schema.param == param_ix as u32)
			.expect("list param missing LIST schema");
		let param_name = slir.str_at(p.name);
		let mut names = Vec::new();
		let mut order = Vec::new();
		collect_list_types(slir, schema_row, pascal(param_name), &mut names, &mut order);
		let root_type = list_type_name(&names, slir, schema_row);

		for &row in &order {
			let schema = &slir.lists[row];
			let item_ty = list_type_name(&names, slir, row);
			let is_canonical = canonical_list_schema(slir, schema_row) == row;
			let doc_comment = if is_canonical {
				format!(
					"One typed item accepted by [`Doc::set_{}`] for list param `{param_name}`.",
					snake(param_name)
				)
			} else {
				format!("One typed nested-list item reachable from list param `{param_name}`.")
			};

			let fields: Vec<serde_json::Value> = (schema.field_off
				..schema.field_off + schema.field_len)
				.map(|field_ix| {
					let field = &slir.list_fields[field_ix as usize];
					let field_name = slir.str_at(field.name);
					let (rust_ty, type_note) = match field.ty {
						0 => ("String".to_string(), "text".to_string()),
						1 => ("f64".to_string(), "number".to_string()),
						2 => ("f64".to_string(), "percentage".to_string()),
						3 => ("Rgba".to_string(), "packed SLIR RGBA color".to_string()),
						4 => ("bool".to_string(), "boolean".to_string()),
						5 => {
							let members: Vec<&str> = (field.enum_off..field.enum_off + field.enum_len)
								.map(|ix| slir.str_at(slir.list_enum_syms[ix as usize]))
								.collect();
							("String".to_string(), format!("enum: {}", members.join(", ")))
						},
						6 if field.sub != 0 => (
							format!("Vec<{}>", list_type_name(&names, slir, field.sub as usize - 1)),
							"nested list".to_string(),
						),
						_ => unreachable!("unknown list field type"),
					};
					json!({
						"name": field_name,
						"snake_name": snake(field_name),
						"rust_type": rust_ty,
						"doc_note": type_note,
					})
				})
				.collect();

			list_item_types.push(json!({
				"name": item_ty,
				"doc_comment": doc_comment,
				"fields": fields,
			}));
		}

		let root_base = root_type.strip_suffix("Item").unwrap_or(&root_type);
		let mut schemas_json = Vec::new();
		for &row in &order {
			let schema = &slir.lists[row];
			let item_ty = list_type_name(&names, slir, row);
			let base_name = item_ty.strip_suffix("Item").unwrap_or(&item_ty);
			let validator_name = format!("validate_{}", snake(base_name));
			let helper_name = format!("set_{}_path", snake(base_name));

			let fields_json: Vec<serde_json::Value> = (schema.field_off
				..schema.field_off + schema.field_len)
				.map(|field_ix| {
					let field = &slir.list_fields[field_ix as usize];
					let field_name = slir.str_at(field.name);
					let member = snake(field_name);
					let is_enum = field.ty == 5;
					let is_list = field.ty == 6;

					let enum_rejected_cond = if is_enum {
						let rejected = (field.enum_off..field.enum_off + field.enum_len)
							.map(|ix| {
								format!(
									"item.{member} != {:?}",
									slir.str_at(slir.list_enum_syms[ix as usize])
								)
							})
							.collect::<Vec<_>>()
							.join(" && ");
						if rejected.is_empty() {
							"true".to_string()
						} else {
							rejected
						}
					} else {
						String::new()
					};

					let (child_validator, child_helper) = if is_list {
						let child_ty = list_type_name(&names, slir, field.sub as usize - 1);
						let child_base = child_ty.strip_suffix("Item").unwrap_or(&child_ty);
						(
							format!("validate_{}", snake(child_base)),
							format!("set_{}_path", snake(child_base)),
						)
					} else {
						(String::new(), String::new())
					};

					let fill_expr = match field.ty {
						0 => format!("pv.s = item.{member}.clone();"),
						1 | 2 => format!("pv.num = item.{member};"),
						3 => format!("pv.rgba = item.{member};"),
						4 => format!("pv.num = if item.{member} {{ 1.0 }} else {{ 0.0 }};"),
						5 => format!("pv.sym = item.{member}.clone();"),
						_ => String::new(),
					};

					json!({
						"field_name": field_name,
						"member": member,
						"ty": field.ty,
						"is_enum": is_enum,
						"enum_rejected_cond": enum_rejected_cond,
						"is_list": is_list,
						"child_validator": child_validator,
						"child_helper": child_helper,
						"fill_expr": fill_expr,
					})
				})
				.collect();

			schemas_json.push(json!({
				"validator_name": validator_name,
				"helper_name": helper_name,
				"item_type": item_ty,
				"fields": fields_json,
			}));
		}

		list_params.push(json!({
			"param_index": param_ix,
			"param_name": param_name,
			"setter_name": snake(param_name),
			"root_item_type": root_type,
			"cache_name": format!("{}_cache", snake(param_name)),
			"root_validator": format!("validate_{}", snake(root_base)),
			"root_helper": format!("set_{}_path", snake(root_base)),
			"schemas": schemas_json,
		}));
	}

	let scalar_params: Vec<serde_json::Value> = slir
		.params
		.iter()
		.enumerate()
		.filter(|(_, p)| p.ty != 6)
		.map(|(i, p)| {
			let name = slir.str_at(p.name).to_string();
			let method_name = snake(&name);
			let (sig, fill_expr) = match p.ty {
				0 => ("v: &str", "pv.s = v.to_string();"),
				1 | 2 => ("v: f64", "pv.num = v;"),
				3 => ("v: Rgba", "pv.rgba = v;"),
				4 => ("v: bool", "pv.num = if v { 1.0 } else { 0.0 };"),
				_ => ("v: &str", "pv.sym = v.to_string();"),
			};
			let doc_note = match p.ty {
				0 => "text".to_string(),
				1 => "num".to_string(),
				2 => "pct (0..100)".to_string(),
				3 => "color, packed with rgba(red, green, blue, alpha)".to_string(),
				4 => "bool".to_string(),
				_ => {
					let members: Vec<&str> = (p.enum_off..p.enum_off + p.enum_len)
						.map(|k| slir.str_at(slir.param_enum_syms[k as usize]))
						.collect();
					format!("enum({})", members.join(", "))
				},
			};
			json!({
				"index": i,
				"name": name,
				"method_name": method_name,
				"ty": p.ty,
				"sig": sig,
				"fill_expr": fill_expr,
				"doc_note": doc_note,
			})
		})
		.collect();

	let ctx = json!({
		"src_name": src_name,
		"bytes_len": bytes.len(),
		"slir_byte_str": byte_string(bytes),
		"keys": keys,
		"item_key_groups": item_key_groups,
		"params": params,
		"holes": holes,
		"signals": signals,
		"list_item_types": list_item_types,
		"list_params": list_params,
		"scalar_params": scalar_params,
	});

	crate::tmpl::render(TEMPLATE, &ctx).expect("rust.tmpl render error")
}

#[cfg(test)]
mod tests {
	use super::generate;
	use crate::Options;

	#[test]
	fn every_generated_signal_variant_carries_shared_metadata() {
		let source = r"
row {
  box press=pressed pointer-move=moved pointer-up=released
  divider w=6 resize=resized
  box dblclick=twice drag=started drag-update=updated drag-end=ended
}
";
		let (module, diagnostics) =
			generate(source, &Options { embed_assets: false, ..Options::default() }, "gestures.slab");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let module = module.expect("gesture module");
		assert_eq!(module.matches("pub struct SignalMeta").count(), 1);
		assert!(module.contains("Pressed {"));
		assert!(module.contains("Resized {"));
		assert!(module.contains("Twice {"));
		assert!(module.contains("text: String,"));
		assert!(module.contains("meta: SignalMeta,"));
		assert!(module.contains("meta: SignalMeta::from(&eff.sig_meta[i])"));
		for field in [
			"pub x: f64",
			"pub y: f64",
			"pub dx: f64",
			"pub dy: f64",
			"pub drag_dx: f64",
			"pub drag_dy: f64",
			"pub mods: u32",
			"pub button: u32",
			"pub clicks: u32",
			"pub key: String",
			"pub src_key: String",
			"pub src_item: String",
			"pub cancelled: bool",
			"pub dropped: bool",
		] {
			assert!(module.contains(field), "{field}");
		}
		assert!(module.contains("pub fn take_signals(&mut self)"));
		assert_eq!(
			module
				.matches("pub fn invalidate_caches(&mut self)")
				.count(),
			1
		);
	}

	#[test]
	fn recursive_list_codegen_validates_then_writes_every_path() {
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
		let (module, diagnostics) =
			generate(source, &Options { embed_assets: false, ..Options::default() }, "trees.slab");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let module = module.expect("recursive list module");
		assert!(module.contains("pub struct TreesItem"));
		assert!(module.contains("pub children: Vec<TreesItem>"));
		assert!(module.contains("fn validate_trees(items: &[TreesItem]) -> bool"));
		assert!(module.contains("Self::validate_trees(&item.children)"));
		assert!(module.contains("if !Self::validate_trees(items) { return false; }"));
		assert!(module.contains("format!(\"{path}.{index}.children\")"));
		assert!(module.contains("let key = item.key.clone().unwrap_or_else(|| index.to_string());"));
		assert!(module.contains("trees_cache: Option<Vec<TreesItem>>"));
		assert!(module.contains("if self.trees_cache.as_deref() == Some(items)"));
		assert!(module.contains("if previous == Some(item) { continue; }"));
	}

	#[test]
	fn host_ergonomics_include_keys_signals_colors_tokens_and_cache_reset() {
		let source = r"
def Row(tone=color.accent) export { row#item bg=tone press=chosen }
tokens { color { accent #336699 } }
params { rows list(Row) = [] }
col#app { col#items { each param.rows } }
";
		let (module, diagnostics) =
			generate(source, &Options { embed_assets: false, ..Options::default() }, "host.slab");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let module = module.expect("host module");
		assert!(module.contains("pub const APP: &str = \"#app\""));
		assert!(module.contains("pub const ITEMS: &str = \"#app/#items\""));
		assert!(!module.contains("\n    pub const ITEM:"));
		assert!(module.contains("pub fn item_key(each: &str, item: &str, rel: &str) -> String"));
		assert!(module.contains("pub mod each_0 {"));
		assert!(module.contains("pub const EACH: &str ="));
		assert!(module.contains("pub const ITEM: &str ="));
		assert!(module.contains("pub hit_key: String"));
		assert!(module.contains("pub pressed_key: String"));
		assert!(module.contains("pub enum SignalName"));
		assert!(module.contains("pub type Rgba = u32"));
		assert!(module.contains("pub const fn rgba("));
		assert!(module.contains("pub tone: Rgba"));
		assert!(module.contains("pub fn with_key(mut self, key: impl Into<String>)"));
		assert!(module.contains("pub fn get_token(&self, path: &str)"));
		assert!(module.contains("pub fn clear_focus(&mut self)"));
		assert!(module.contains("pub fn focus_item(&mut self, each_key: &str, index: i32)"));
		assert!(module.contains("pub fn focus_note(&self) -> &str"));
		assert!(module.contains("#[allow(clippy::missing_const_for_fn)]"));
		assert!(module.contains("pub fn invalidate_caches(&mut self)"));
		assert!(module.contains("self.rows_cache = None"));
	}

	#[test]
	fn grouped_parameter_names_generate_valid_rust_identifiers() {
		let source = r"
params editor { font_size num = 14 }
col { rect w=param.editor.font_size }
";
		let (module, diagnostics) =
			generate(source, &Options { embed_assets: false, ..Options::default() }, "editor.slab");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let module = module.expect("grouped parameter module");
		assert!(module.contains("pub const PARAM_EDITOR_FONT_SIZE: u32 = 0;"));
		assert!(module.contains("pub fn set_editor_font_size(&mut self, v: f64) -> bool"));
		assert!(module.contains("/// Set param `editor.font_size` (num)"));
	}
}
