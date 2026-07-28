//! Typed Go module generation for the Go client runtime (`clients/go`).
//!
//! Produces a single deterministic Go file that wraps a `*slab.Session` and
//! drives it over the Slab Drive Protocol (SDP): typed param setters route
//! through `param.set`, typed list setters reconcile through `list.set_len`,
//! `list.set_key`, and `list.set_field`, and `DecodeSignals` turns raw effects
//! into typed signals. The kernel remains the only owner of layout, hit
//! testing, focus, and editing; the generated code never reimplements any of
//! it.
//!
//! Output is deterministic and gofmt-clean as emitted. Regenerate with:
//! `cargo run -q -p slab-cli -- gen go FILE -o OUT.go [--package NAME]`

use serde_json::json;
use slab_slir::Slir;
use slab_syntax::diag::Diagnostics;

use crate::{
	Options,
	tmpl::{camel, pascal},
};

const TEMPLATE: &str = include_str!("../templates/go.tmpl");

/// Generate the typed Go binding for a compiled `.slab` source.
///
/// `src_name` is the input path used in the header comment and `doc.open`;
/// `package` is the emitted Go package name. Returns source (or `None` on
/// compile failure) and compile diagnostics.
pub fn generate(
	src: &str,
	copts: &Options,
	src_name: &str,
	package: &str,
) -> (Option<String>, Diagnostics) {
	let (slir, diags) = crate::compile_with_exports(src, copts);
	let Some(slir) = slir else {
		return (None, diags);
	};
	let bytes = slab_slir::write(&slir);
	let module = emit_module(&slir, &bytes, src_name, package);
	(Some(module), diags)
}

/// Standard-alphabet base64 with padding. Written here so the crate keeps its
/// existing dependency set; the encoding is fixed, so output stays byte-stable.
fn base64(bytes: &[u8]) -> String {
	const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
	for chunk in bytes.chunks(3) {
		let a = u32::from(chunk[0]);
		let b = u32::from(chunk.get(1).copied().unwrap_or(0));
		let c = u32::from(chunk.get(2).copied().unwrap_or(0));
		let word = (a << 16) | (b << 8) | c;
		out.push(ALPHABET[(word >> 18) as usize & 63] as char);
		out.push(ALPHABET[(word >> 12) as usize & 63] as char);
		out.push(if chunk.len() > 1 {
			ALPHABET[(word >> 6) as usize & 63] as char
		} else {
			'='
		});
		out.push(if chunk.len() > 2 {
			ALPHABET[word as usize & 63] as char
		} else {
			'='
		});
	}
	out
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

fn list_base_name(item_ty: &str) -> String {
	item_ty.strip_suffix("Item").unwrap_or(item_ty).to_string()
}

const fn kind_name(ty: u8) -> &'static str {
	match ty {
		0 => "text",
		1 => "num",
		2 => "pct",
		3 => "color",
		4 => "bool",
		_ => "enum",
	}
}

fn scalar_field_type(slir: &Slir, field: &slab_slir::ListFieldE, owner: &str) -> (String, String) {
	match field.ty {
		0 => ("string".to_string(), "text".to_string()),
		1 => ("float64".to_string(), "number".to_string()),
		2 => ("float64".to_string(), "percentage".to_string()),
		3 => ("Rgba".to_string(), "packed SLIR RGBA color".to_string()),
		4 => ("bool".to_string(), "boolean".to_string()),
		_ => {
			let members: Vec<&str> = (field.enum_off..field.enum_off + field.enum_len)
				.map(|ix| slir.str_at(slir.list_enum_syms[ix as usize]))
				.collect();
			(
				format!("{owner}{}", pascal(slir.str_at(field.name))),
				format!("enum: {}", members.join(", ")),
			)
		},
	}
}

fn field_wire_expr(ty: u8, member: &str) -> String {
	match ty {
		3 => format!("uint32(item.{member})"),
		5 => format!("string(item.{member})"),
		_ => format!("item.{member}"),
	}
}

fn schema_row_of(slir: &Slir, param: usize) -> usize {
	slir
		.lists
		.iter()
		.position(|schema| schema.param == param as u32)
		.expect("list param missing LIST schema")
}

fn emit_module(slir: &Slir, bytes: &[u8], src_name: &str, package: &str) -> String {
	let signals = unique_signals(slir);
	let has_list = slir.params.iter().any(|p| p.ty == 6);
	let has_scalar = slir.params.iter().any(|p| p.ty != 6);
	let has_color =
		slir.params.iter().any(|p| p.ty == 3) || slir.list_fields.iter().any(|f| f.ty == 3);

	let encoded = base64(bytes);
	let slir_chunks: Vec<String> = encoded
		.as_bytes()
		.chunks(76)
		.map(|chunk| {
			std::str::from_utf8(chunk)
				.expect("base64 output is ASCII")
				.to_string()
		})
		.collect();

	let keys: Vec<serde_json::Value> = crate::wc::static_scene_keys(slir)
		.into_iter()
		.map(|(name, key)| {
			json!({
				"name": name,
				"pascal_name": pascal(&name),
				"key": key,
			})
		})
		.collect();

	let params_json: Vec<serde_json::Value> = slir
		.params
		.iter()
		.map(|p| {
			let name = slir.str_at(p.name);
			json!({
				"name": name,
				"pascal_name": pascal(name),
				"param_type_name": slab_slir::PARAM_TYPE_NAMES
					.get(usize::from(p.ty))
					.copied()
					.unwrap_or("unknown"),
			})
		})
		.collect();

	let enum_params: Vec<serde_json::Value> = slir
		.params
		.iter()
		.filter(|p| p.ty == 5)
		.map(|p| {
			let name = slir.str_at(p.name);
			let members: Vec<String> = (p.enum_off..p.enum_off + p.enum_len)
				.map(|ix| slir.str_at(slir.param_enum_syms[ix as usize]).to_string())
				.collect();
			json!({
				"name": name,
				"pascal_name": pascal(name),
				"members": members,
			})
		})
		.collect();

	let signals_json: Vec<serde_json::Value> = signals
		.iter()
		.map(|(name, has_text)| {
			json!({
				"name": name,
				"pascal_name": pascal(name),
				"has_text": *has_text,
			})
		})
		.collect();

	let mut list_params: Vec<serde_json::Value> = Vec::new();
	for (param_ix, p) in slir.params.iter().enumerate() {
		if p.ty != 6 {
			continue;
		}
		let param_name = slir.str_at(p.name);
		let schema_row = schema_row_of(slir, param_ix);
		let mut names = Vec::new();
		let mut order = Vec::new();
		collect_list_types(slir, schema_row, pascal(param_name), &mut names, &mut order);
		let root = canonical_list_schema(slir, schema_row);

		let mut items_json = Vec::new();
		for &row in &order {
			let schema = &slir.lists[row];
			let item_ty = list_type_name(&names, slir, row);
			let base = list_base_name(&item_ty);
			let is_root = row == root;

			let mut enum_fields = Vec::new();
			for field_ix in schema.field_off..schema.field_off + schema.field_len {
				let field = &slir.list_fields[field_ix as usize];
				if field.ty != 5 {
					continue;
				}
				let field_name = slir.str_at(field.name);
				let members: Vec<String> = (field.enum_off..field.enum_off + field.enum_len)
					.map(|ix| slir.str_at(slir.list_enum_syms[ix as usize]).to_string())
					.collect();
				enum_fields.push(json!({
					"name": field_name,
					"pascal_name": pascal(field_name),
					"members": members,
				}));
			}

			let mut fields_json = Vec::new();
			let mut nested_lists = Vec::new();

			for field_ix in schema.field_off..schema.field_off + schema.field_len {
				let field = &slir.list_fields[field_ix as usize];
				let field_name = slir.str_at(field.name);
				let member = pascal(field_name);
				let is_list = field.ty == 6;
				let is_enum = field.ty == 5;

				let (go_ty, note) = if is_list {
					(
						format!("[]{}", list_type_name(&names, slir, field.sub as usize - 1)),
						"nested list".to_string(),
					)
				} else {
					scalar_field_type(slir, field, &item_ty)
				};

				let (child_type, child_base) = if is_list {
					let child_ty = list_type_name(&names, slir, field.sub as usize - 1);
					let cb = list_base_name(&child_ty);
					nested_lists.push(json!({
						"pascal_name": member,
						"child_base": cb,
					}));
					(child_ty, cb)
				} else {
					(String::new(), String::new())
				};

				let enum_consts: Vec<String> = if is_enum {
					let enum_ty = format!("{item_ty}{member}");
					(field.enum_off..field.enum_off + field.enum_len)
						.map(|ix| {
							format!("{enum_ty}{}", pascal(slir.str_at(slir.list_enum_syms[ix as usize])))
						})
						.collect()
				} else {
					Vec::new()
				};

				fields_json.push(json!({
					"name": field_name,
					"dot_field_name": format!(".{field_name}"),
					"pascal_name": member,
					"is_list": is_list,
					"is_enum": is_enum,
					"go_type": go_ty,
					"note": note,
					"child_type": child_type,
					"child_base": child_base,
					"kind_name": kind_name(field.ty),
					"wire_expr": field_wire_expr(field.ty, &member),
					"enum_consts": enum_consts,
				}));
			}

			items_json.push(json!({
				"type_name": item_ty,
				"base_name": base,
				"is_root": is_root,
				"enum_fields": enum_fields,
				"fields": fields_json,
				"nested_lists": nested_lists,
			}));
		}

		let root_ty = list_type_name(&names, slir, schema_row);
		let root_base = list_base_name(&root_ty);

		list_params.push(json!({
			"param_name": param_name,
			"pascal_name": pascal(param_name),
			"cache_name": format!("{}Cache", camel(param_name)),
			"wire_param": param_name,
			"root_item_type": root_ty,
			"root_base": root_base,
			"items": items_json,
		}));
	}

	let scalar_params: Vec<serde_json::Value> = slir
		.params
		.iter()
		.filter(|p| p.ty != 6)
		.map(|p| {
			let name = slir.str_at(p.name);
			let (go_ty, wire, note) = match p.ty {
				0 => ("string".to_string(), "value".to_string(), "text".to_string()),
				1 => ("float64".to_string(), "value".to_string(), "num".to_string()),
				2 => ("float64".to_string(), "value".to_string(), "pct, 0..100".to_string()),
				3 => (
					"Rgba".to_string(),
					"value.String()".to_string(),
					"color, packed with NewRgba(red, green, blue, alpha)".to_string(),
				),
				4 => ("bool".to_string(), "value".to_string(), "bool".to_string()),
				_ => {
					let members: Vec<&str> = (p.enum_off..p.enum_off + p.enum_len)
						.map(|ix| slir.str_at(slir.param_enum_syms[ix as usize]))
						.collect();
					(pascal(name), "string(value)".to_string(), format!("enum: {}", members.join(", ")))
				},
			};
			json!({
				"name": name,
				"pascal_name": pascal(name),
				"go_type": go_ty,
				"wire_expr": wire,
				"doc_note": note,
			})
		})
		.collect();

	let ctx = json!({
		"src_name": src_name,
		"package": package,
		"bytes_len": bytes.len(),
		"slir_chunks": slir_chunks,
		"has_list": has_list,
		"has_scalar": has_scalar,
		"has_color": has_color,
		"keys": keys,
		"params": params_json,
		"enum_params": enum_params,
		"signals": signals_json,
		"list_params": list_params,
		"scalar_params": scalar_params,
	});

	crate::tmpl::render(TEMPLATE, &ctx).expect("go.tmpl render error")
}

#[cfg(test)]
mod tests {
	use super::generate;
	use crate::Options;

	fn options() -> Options {
		Options { embed_assets: false, ..Options::default() }
	}

	#[test]
	fn every_generated_signal_carries_name_text_item_and_metadata() {
		let source = r"
row {
  box press=pressed pointer-move=moved pointer-up=released
  divider w=6 resize=resized
  box dblclick=twice drag=started drag-update=updated drag-end=ended
}
";
		let (module, diagnostics) = generate(source, &options(), "gestures.slab", "gestures");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let module = module.expect("gesture module");
		assert!(module.contains("package gestures"));
		assert!(module.contains("type SignalName string"));
		assert!(module.contains("const SignalPressed SignalName = \"pressed\""));
		assert!(module.contains("const SignalResized SignalName = \"resized\""));
		assert!(module.contains("const SignalTwice SignalName = \"twice\""));
		assert!(module.contains("Meta slab.SignalMeta"));
		assert_eq!(module.matches("type Signal struct {").count(), 1);
		assert!(module.contains("func DecodeSignals(effects slab.Effects) []Signal {"));
		assert!(module.contains(
			"out = append(out, Signal{Name: name, Text: raw.Text, Item: raw.Item, Meta: raw.Meta})"
		));
		assert!(module.contains(
			"case SignalPressed, SignalMoved, SignalReleased, SignalResized, SignalTwice, \
			 SignalStarted, SignalUpdated, SignalEnded:"
		));
		assert!(
			!module.contains("func (d *Doc) InvalidateCaches()"),
			"a document without list params has no reconciliation snapshots to drop"
		);
	}

	#[test]
	fn recursive_list_codegen_validates_then_writes_every_nested_path() {
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
		let (module, diagnostics) = generate(source, &options(), "trees.slab", "trees");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let module = module.expect("recursive list module");
		assert!(module.contains("type TreesItem struct {"));
		assert!(module.contains("\tChildren []TreesItem"));
		assert!(module.contains("func validateTreesItems(items []TreesItem) error {"));
		assert!(module.contains("if err := validateTreesItems(item.Children); err != nil {"));
		assert!(module.contains("if err := validateTreesItems(items); err != nil {"));
		assert!(module.contains("childPath := strconv.Itoa(index) + \".children\""));
		assert!(module.contains("childPath = path + \".\" + childPath"));
		assert!(module.contains(
			"if err := d.setTreesPath(ctx, childPath, item.Children, priorChildren); err != nil {"
		));
		assert!(module.contains("key = strconv.Itoa(index)"));
		assert!(
			module.contains(
				"d.setListField(ctx, \"trees\", path, index, \"label\", \"text\", item.Label)"
			)
		);
		assert!(module.contains("\ttreesCache []TreesItem"));
		assert!(module.contains("\ttreesCacheValid bool"));
		assert!(module.contains("if d.treesCacheValid && equalTreesItems(d.treesCache, items) {"));
		assert!(module.contains("if prior != nil && prior.equals(item) {"));
		assert!(module.contains("d.treesCache = cloneTreesItems(items)"));
		assert!(module.contains("func (d *Doc) InvalidateCaches() {"));
		assert!(module.contains("d.treesCache = nil"));
	}

	#[test]
	fn host_ergonomics_include_keys_params_colors_enums_and_cache_reset() {
		let source = r#"
def Row(tone=color.accent) export { row#item bg=tone press=chosen }
tokens { color { accent #336699 } }
params {
  rows list(Row) = []
  title text = "hi"
  ratio pct = 30%
  dense bool = false
  tint color = #112233
  mode enum(compact, cozy) = compact
}
col#app { col#items { each param.rows } }
"#;
		let (module, diagnostics) = generate(source, &options(), "host.slab", "host");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let module = module.expect("host module");
		assert!(module.contains("const KeyApp = \"#app\""));
		assert!(module.contains("const KeyItems = \"#app/#items\""));
		assert!(!module.contains("const KeyItem ="));
		assert!(module.contains("const ParamTitle = \"title\""));
		assert!(module.contains("type Rgba uint32"));
		assert!(module.contains("func NewRgba(red, green, blue, alpha uint8) Rgba {"));
		assert!(module.contains("func (c Rgba) String() string {"));
		assert!(module.contains("func (d *Doc) SetTitle(ctx context.Context, value string) error {"));
		assert!(module.contains("return d.setParam(ctx, \"title\", value)"));
		assert!(
			module.contains("func (d *Doc) SetRatio(ctx context.Context, value float64) error {")
		);
		assert!(module.contains("func (d *Doc) SetDense(ctx context.Context, value bool) error {"));
		assert!(module.contains("func (d *Doc) SetTint(ctx context.Context, value Rgba) error {"));
		assert!(module.contains("return d.setParam(ctx, \"tint\", value.String())"));
		assert!(module.contains("type Mode string"));
		assert!(module.contains("const ModeCompact Mode = \"compact\""));
		assert!(module.contains("func (d *Doc) SetMode(ctx context.Context, value Mode) error {"));
		assert!(module.contains("return d.setParam(ctx, \"mode\", string(value))"));
		assert!(module.contains("const ModeCozy Mode = \"cozy\""));
		assert!(module.contains("\tTone Rgba"));
		assert!(module.contains("uint32(item.Tone)"));
		assert!(module.contains("d.rowsCache = nil"));
		assert!(module.contains("d.rowsCacheValid = false"));
		assert!(module.contains("func New(ctx context.Context, sess *slab.Session) (*Doc, error) {"));
		assert!(module.contains("sess.OpenSLIR(ctx, SLIR, SourceName)"));
		assert!(module.contains("var SLIR []byte"));
		assert!(module.contains("func init() {"));
		assert!(!module.contains("const Source ="));
		assert!(module.contains("const slirBase64 = \"\" +"));
	}

	#[test]
	fn generation_is_byte_identical_across_runs() {
		let source = r#"
def Row(label="") export { text label }
params {
  title text = "hello"
  rows list(Row) = []
}
col#app {
  text param.title
  each param.rows
}
"#;
		let first = generate(source, &options(), "det.slab", "det")
			.0
			.expect("first");
		let second = generate(source, &options(), "det.slab", "det")
			.0
			.expect("second");
		assert_eq!(first, second);
	}
}
