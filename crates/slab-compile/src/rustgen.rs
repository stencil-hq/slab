//! Generates typed Rust modules for the native GPU client.
//!
//! Moved lib-side from the CLI so the wasm build can emit the same output.
//! Produces a single-file module embedding the SLIR and exposing a typed
//! `Doc` wrapper over `slab_kernel::frame::Instance`.
//!
//! Output is deterministic. Regenerate + reformat with:
//! `cargo run -q -p slab-cli -- gen rust FILE -o OUT.rs && cargo fmt`

use std::{fmt::Write as _, path::PathBuf};

use slab_slir::Slir;
use slab_syntax::diag::Diagnostics;

use crate::Options;
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

/// `PascalCase` from a signal/param name (`row-clicked` -> `RowClicked`).
fn pascal(s: &str) -> String {
	let mut out = String::new();
	let mut up = true;
	for c in s.chars() {
		if c.is_alphanumeric() {
			if up {
				out.extend(c.to_uppercase());
				up = false;
			} else {
				out.push(c);
			}
		} else {
			up = true;
		}
	}
	if out.is_empty() {
		out.push('X');
	}
	out
}

/// `snake_case` identifier fragment (`row-clicked` -> `row_clicked`).
fn snake(s: &str) -> String {
	let mut out = String::new();
	for c in s.chars() {
		if c.is_alphanumeric() {
			out.extend(c.to_lowercase());
		} else {
			out.push('_');
		}
	}
	if out.is_empty() {
		out.push('x');
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

fn byte_string(bytes: &[u8]) -> String {
	let mut s = String::with_capacity(bytes.len() * 4 + 2);
	s.push_str("b\"");
	for &b in bytes {
		let _ = write!(s, "\\x{b:02X}");
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

fn emit_list_setters(
	out: &mut String,
	slir: &Slir,
	param: usize,
	schema_row: usize,
	param_name: &str,
) {
	let mut names = Vec::new();
	let mut order = Vec::new();
	collect_list_types(slir, schema_row, pascal(param_name), &mut names, &mut order);
	for row in &order {
		let schema = &slir.lists[*row];
		let item_ty = list_type_name(&names, slir, *row);
		let base_name = item_ty.strip_suffix("Item").unwrap_or(&item_ty);
		let validator = format!("validate_{}", snake(base_name));
		let helper = format!("set_{}_path", snake(base_name));
		let _ = writeln!(
			out,
			"\n    fn {validator}(items: &[{item_ty}]) -> bool {{\n        let Ok(_) = \
			 i32::try_from(items.len()) else {{ return false }};\n        let mut keys = \
			 std::collections::HashSet::with_capacity(items.len());\n        for (index, item) in \
			 items.iter().enumerate() {{\n            let key = item.key.clone().unwrap_or_else(|| \
			 index.to_string());\n            if key.is_empty() || !keys.insert(key) {{ return \
			 false; }}"
		);
		for field_ix in schema.field_off..schema.field_off + schema.field_len {
			let field = &slir.list_fields[field_ix as usize];
			let member = snake(slir.str_at(field.name));
			if field.ty == 5 {
				let rejected = (field.enum_off..field.enum_off + field.enum_len)
					.map(|ix| {
						format!("item.{member} != {:?}", slir.str_at(slir.list_enum_syms[ix as usize]))
					})
					.collect::<Vec<_>>()
					.join(" && ");
				let rejected = if rejected.is_empty() {
					"true".to_string()
				} else {
					rejected
				};
				let _ = writeln!(out, "            if {rejected} {{ return false; }}");
			} else if field.ty == 6 {
				let child_ty = list_type_name(&names, slir, field.sub as usize - 1);
				let child_base = child_ty.strip_suffix("Item").unwrap_or(&child_ty);
				let child_validator = format!("validate_{}", snake(child_base));
				let _ = writeln!(
					out,
					"            if !Self::{child_validator}(&item.{member}) {{ return false; }}"
				);
			}
		}
		let _ = writeln!(out, "        }}\n        true\n    }}");
		let _ = writeln!(
			out,
			"\n    fn {helper}(\n        &mut self,\n        path: &str,\n        items: \
			 &[{item_ty}],\n        previous: Option<&[{item_ty}]>,\n    ) -> bool {{\n        let \
			 Ok(len) = i32::try_from(items.len()) else {{ return false }};\n        if \
			 !kframe::inst_set_list_len(&mut self.inst, {param}, path, len) {{ return false; }}\n        \
			 for (item_index, item) in items.iter().enumerate() {{\n            let previous = \
			 previous.and_then(|items| items.get(item_index));\n            if previous == \
			 Some(item) {{ continue; }}\n            let Ok(index) = i32::try_from(item_index) else \
			 {{ return false }};\n            if previous.is_none_or(|previous| previous.key != \
			 item.key) {{\n                let key = item.key.clone().unwrap_or_else(|| \
			 item_index.to_string());\n                if !kframe::inst_set_list_key(&mut self.inst, \
			 {param}, path, index, &key) {{ return false; }}\n            }}"
		);
		for field_ix in schema.field_off..schema.field_off + schema.field_len {
			let field = &slir.list_fields[field_ix as usize];
			let field_name = slir.str_at(field.name);
			let member = snake(field_name);
			if field.ty == 6 {
				let child_ty = list_type_name(&names, slir, field.sub as usize - 1);
				let child_base = child_ty.strip_suffix("Item").unwrap_or(&child_ty);
				let child_helper = format!("set_{}_path", snake(child_base));
				let _ = writeln!(
					out,
					"            let child_path = if path.is_empty() {{\n                \
					 format!(\"{{index}}.{field_name}\")\n            }} else {{\n                \
					 format!(\"{{path}}.{{index}}.{field_name}\")\n            }};\n            let \
					 previous_{member} = previous.map(|previous| previous.{member}.as_slice());\n            \
					 if !self.{child_helper}(&child_path, &item.{member}, previous_{member}) {{ return \
					 false; }}"
				);
				continue;
			}
			let fill = match field.ty {
				0 => format!("pv.s = item.{member}.clone();"),
				1 | 2 => format!("pv.num = item.{member};"),
				3 => format!("pv.rgba = item.{member};"),
				4 => format!("pv.num = if item.{member} {{ 1.0 }} else {{ 0.0 }};"),
				5 => format!("pv.sym = item.{member}.clone();"),
				_ => unreachable!("unknown scalar list field type"),
			};
			let _ = writeln!(
				out,
				"            if previous.is_none_or(|previous| previous.{member} != item.{member}) \
				 {{\n                let mut pv = ParamValue {{ kind: {}, num: 0.0, s: String::new(), \
				 rgba: 0, sym: String::new() }};\n                {fill}\n                if \
				 !kframe::inst_set_list_field(&mut self.inst, {param}, path, index, {field_name:?}, \
				 &pv) {{ return false; }}\n            }}",
				field.ty
			);
		}
		let _ = writeln!(out, "        }}\n        true\n    }}");
	}
	let root_type = list_type_name(&names, slir, schema_row);
	let root_base = root_type.strip_suffix("Item").unwrap_or(&root_type);
	let validator = format!("validate_{}", snake(root_base));
	let root_helper = format!("set_{}_path", snake(root_base));
	let cache = format!("{}_cache", snake(param_name));
	let _ = writeln!(
		out,
		"\n    /// Reconciles list param `{param_name}` and all nested lists with the last typed \
		 value.\n    pub fn set_{}(&mut self, items: &[{root_type}]) -> bool {{\n        if \
		 self.{cache}.as_deref() == Some(items) {{ return true; }}\n        if \
		 !Self::{validator}(items) {{ return false; }}\n        let previous = \
		 self.{cache}.take();\n        let updated = self.{root_helper}(\"\", items, \
		 previous.as_deref());\n        self.{cache} = if updated {{ Some(items.to_vec()) }} else \
		 {{ previous }};\n        updated\n    }}",
		snake(param_name)
	);
}

fn emit_module(slir: &Slir, bytes: &[u8], src_name: &str) -> String {
	let mut o = String::new();
	let _ = writeln!(
		o,
		"// GENERATED by `slab gen rust {src_name}` — do not edit.\n// Regenerate: cargo run -q -p \
		 slab-cli -- gen rust {src_name} -o <this file> && cargo fmt\n\n\nuse \
		 slab_kernel::dispatch::{{Effects, Event}};\nuse slab_kernel::flatten::Frame;\nuse \
		 slab_kernel::frame::{{self as kframe, HoleRect, Instance, ParamValue}};\nuse slab_slir;\n"
	);
	let _ = writeln!(o, "/// The compiled SLIR document ({} bytes).", bytes.len());
	let _ = writeln!(o, "pub static SLIR: &[u8] = {};\n", byte_string(bytes));

	o.push_str(
		"/// Packed SLIR color word (red in the low byte, then green, blue, alpha).\npub type Rgba \
		 = u32;\n\n/// Pack straight-alpha RGBA channels for generated color params and list \
		 fields.\npub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Rgba {\n\x20   \
		 u32::from_le_bytes([red, green, blue, alpha])\n}\n\n/// Canonical full scene keys for \
		 authored `#id` nodes.\npub mod keys {\n",
	);
	for (name, key) in crate::wc::static_scene_keys(slir) {
		let _ = writeln!(o, "    pub const {}: &str = {:?};", snake(&name).to_uppercase(), key);
	}
	o.push_str(
        "    /// Join one `each` item into a full canonical scene key.\n\
         \x20   ///\n\
         \x20   /// `item_key(EACH, item, \"\")` addresses the item root; a non-empty\n\
         \x20   /// `rel` appends a template-relative key. `item` is escaped per the\n\
         \x20   /// canonical grammar (`%` → `%25`, `/` → `%2F`, `~` → `%7E`).\n\
         \x20   pub fn item_key(each: &str, item: &str, rel: &str) -> String {\n\
         \x20       let mut escaped = String::with_capacity(item.len());\n\
         \x20       for ch in item.chars() {\n\
         \x20           match ch {\n\
         \x20               '%' => escaped.push_str(\"%25\"),\n\
         \x20               '/' => escaped.push_str(\"%2F\"),\n\
         \x20               '~' => escaped.push_str(\"%7E\"),\n\
         \x20               _ => escaped.push(ch),\n\
         \x20           }\n\
         \x20       }\n\
         \x20       if rel.is_empty() {\n\
         \x20           format!(\"{each}~{escaped}\")\n\
         \x20       } else {\n\
         \x20           format!(\"{each}~{escaped}/{rel}\")\n\
         \x20       }\n\
         \x20   }\n",
    );
	for group in crate::wc::item_scene_keys(slir) {
		let _ = writeln!(
			o,
			"    /// Template-relative scene keys for the `{}` each; join with [`item_key`].",
			group.name
		);
		let _ = writeln!(o, "    pub mod {} {{", snake(&group.name));
		let _ = writeln!(
			o,
			"        /// Canonical key of the each node itself.\n        pub const EACH: &str = {:?};",
			group.each_key
		);
		for (name, key) in &group.items {
			let mut upper = snake(name).to_uppercase();
			if upper == "EACH" {
				upper.push_str("_ID");
			}
			let _ = writeln!(o, "        pub const {upper}: &str = {key:?};");
		}
		o.push_str("    }\n");
	}
	o.push_str("}\n\n");

	for (i, p) in slir.params.iter().enumerate() {
		let _ =
			writeln!(o, "pub const PARAM_{}: u32 = {i};", snake(slir.str_at(p.name)).to_uppercase());
	}
	for (i, &(name, _node)) in slir.holes.iter().enumerate() {
		let _ = writeln!(o, "pub const HOLE_{}: u32 = {i};", snake(slir.str_at(name)).to_uppercase());
	}
	let sigs = unique_signals(slir);
	for (name, _) in &sigs {
		// interned STRS ref of the signal name, for Effects decoding
		let strref = slir
			.signals
			.iter()
			.find(|&&(n, ..)| slir.str_at(n) == name)
			.map_or(0, |&(n, ..)| n);
		let _ = writeln!(o, "const SIG_{}: u32 = {strref};", snake(name).to_uppercase());
	}

	o.push_str(
		"\n/// Metadata captured for every emitted signal.\n#[derive(Debug, Clone, PartialEq)]\npub \
		 struct SignalMeta {\n\x20   /// Document-space pointer x, or `-1.0` for keyboard \
		 origin.\n\x20   pub x: f64,\n\x20   /// Document-space pointer y, or `-1.0` for keyboard \
		 origin.\n\x20   pub y: f64,\n\x20   /// Horizontal delta carried by the originating \
		 event.\n\x20   pub dx: f64,\n\x20   /// Vertical delta carried by the originating \
		 event.\n\x20   pub dy: f64,\n\x20   /// Horizontal drag displacement from the pointer-down \
		 origin.\n\x20   pub drag_dx: f64,\n\x20   /// Vertical drag displacement from the \
		 pointer-down origin.\n\x20   pub drag_dy: f64,\n\x20   /// Modifier bitset active at \
		 emission.\n\x20   pub mods: u32,\n\x20   /// Pointer button code.\n\x20   pub button: \
		 u32,\n\x20   /// Host-computed click count.\n\x20   pub clicks: u32,\n\x20   /// Full key \
		 path of the signal-emitting node.\n\x20   pub key: String,\n\x20   /// Full drag-source \
		 key for Drop, otherwise empty.\n\x20   pub src_key: String,\n\x20   /// Innermost \
		 drag-source item key for `Drop`, otherwise empty.\n\x20   pub src_item: String,\n\x20   /// \
		 Whether `DragEnd` represents abnormal termination.\n\x20   pub cancelled: bool,\n\x20   /// \
		 Whether `DragEnd` delivered `Drop` to an eligible target.\n\x20   pub dropped: bool,\n\x20   \
		 /// Deepest hit-target key on pointer-derived signals, otherwise empty.\n\x20   pub \
		 hit_key: String,\n\x20   /// Pressed key name on keyboard-driven activation, otherwise \
		 empty.\n\x20   pub pressed_key: String,\n}\n\nimpl From<&slab_kernel::dispatch::SigMeta> \
		 for SignalMeta {\n\x20   fn from(meta: &slab_kernel::dispatch::SigMeta) -> Self {\n\x20       \
		 Self {\n\x20           x: meta.x,\n\x20           y: meta.y,\n\x20           dx: \
		 meta.dx,\n\x20           dy: meta.dy,\n\x20           drag_dx: meta.drag_dx,\n\x20           \
		 drag_dy: meta.drag_dy,\n\x20           mods: meta.mods,\n\x20           button: \
		 meta.button,\n\x20           clicks: meta.clicks,\n\x20           key: \
		 meta.key.clone(),\n\x20           src_key: meta.src_key.clone(),\n\x20           src_item: \
		 meta.src_item.clone(),\n\x20           cancelled: meta.cancelled,\n\x20           dropped: \
		 meta.dropped,\n\x20           hit_key: meta.hit_key.clone(),\n\x20           pressed_key: \
		 meta.pressed_key.clone(),\n\x20       }\n\x20   }\n}\n",
	);
	let _ = writeln!(o, "\n/// Signals declared in the document's SIGN table.");
	let _ = writeln!(o, "#[derive(Debug, Clone, PartialEq)]");
	let _ = writeln!(o, "pub enum Signal {{");
	for (name, has_text) in &sigs {
		if *has_text {
			let _ =
				writeln!(o, "    /// A text-bearing signal with list identity and input metadata.");
			let _ = writeln!(
				o,
				"    {} {{\n        /// Committed field text or final resize extent.\n        text: \
				 String,\n        /// Innermost list item key, or empty outside a list.\n        \
				 item: String,\n        /// Input and source metadata.\n        meta: SignalMeta,\n    \
				 }},",
				pascal(name)
			);
		} else {
			let _ = writeln!(o, "    /// A signal carrying list identity and input metadata.");
			let _ = writeln!(
				o,
				"    {} {{\n        /// Innermost list item key, or empty outside a list.\n        \
				 item: String,\n        /// Input and source metadata.\n        meta: SignalMeta,\n    \
				 }},",
				pascal(name)
			);
		}
	}
	let _ = writeln!(o, "}}\n");
	let _ = writeln!(o, "/// Names accepted by the document's signal surface.");
	let _ = writeln!(o, "#[derive(Debug, Clone, Copy, Eq, PartialEq)]");
	let _ = writeln!(o, "pub enum SignalName {{");
	for (name, _) in &sigs {
		let _ = writeln!(o, "    {},", pascal(name));
	}
	let _ = writeln!(o, "}}\n");
	let _ = writeln!(o, "impl SignalName {{");
	let _ = writeln!(o, "    /// Return the authored signal name.");
	let _ = writeln!(o, "    pub const fn as_str(self) -> &'static str {{");
	let _ = writeln!(o, "        match self {{");
	for (name, _) in &sigs {
		let _ = writeln!(o, "            Self::{} => {:?},", pascal(name), name);
	}
	let _ = writeln!(o, "        }}");
	let _ = writeln!(o, "    }}");
	let _ = writeln!(o, "}}\n");
	let mut list_cache_fields = String::new();
	let mut list_cache_initializers = String::new();
	let mut cache_invalidations = String::new();
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
		let cache = format!("{}_cache", snake(param_name));
		let _ = writeln!(list_cache_fields, "    {cache}: Option<Vec<{root_type}>>,");
		let _ = writeln!(list_cache_initializers, "            {cache}: None,");
		let _ = writeln!(cache_invalidations, "        self.{cache} = None;");
		for row in order {
			let schema = &slir.lists[row];
			let item_ty = list_type_name(&names, slir, row);
			if canonical_list_schema(slir, schema_row) == row {
				let _ = writeln!(
					o,
					"/// One typed item accepted by [`Doc::set_{}`] for list param `{param_name}`.",
					snake(param_name)
				);
			} else {
				let _ = writeln!(
					o,
					"/// One typed nested-list item reachable from list param `{param_name}`."
				);
			}
			let _ = writeln!(o, "#[derive(Debug, Clone, PartialEq, Default)]");
			let _ = writeln!(o, "pub struct {item_ty} {{");
			let _ = writeln!(
				o,
				"    /// Stable item key; `None` uses the current positional key.\n    pub key: \
				 Option<String>,"
			);
			for field_ix in schema.field_off..schema.field_off + schema.field_len {
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
				let _ = writeln!(
					o,
					"    /// Schema field `{field_name}` ({type_note}).\n    pub {}: {rust_ty},",
					snake(field_name)
				);
			}
			let _ = writeln!(o, "}}\n");
			let _ = writeln!(
				o,
				"impl {item_ty} {{\n    /// Attach a stable list identity; omit this call to use the \
				 positional key.\n    pub fn with_key(mut self, key: impl Into<String>) -> Self {{\n        \
				 self.key = Some(key.into());\n        self\n    }}\n}}\n"
			);
		}
	}

	// Every signal indexes the parallel item-key and metadata arrays.
	let loop_head = "for (i, &name) in eff.sig_name.iter().enumerate() {";
	let _ = writeln!(
			o,
			"/// The document instance, driven through `slab-kernel`.\npub struct Doc {{\n    /// \
			 Raw kernel instance for advanced integrations.\n    pub inst: Instance,\n    /// \
			 Embedded image bytes, parallel to the document image tables.\n    pub imgs: \
			 Vec<Vec<u8>>,\n{list_cache_fields}}}\n\nimpl Default for Doc {{\n    fn default() -> \
			 Self {{\n        Self::new()\n    }}\n}}\n\nimpl Doc {{\n\x20   /// Create an instance \
			 initialized from the embedded document.\n\x20   pub fn new() -> Self {{\n\x20       let \
			 (doc, imgs) = slab_slir::decode_doc(SLIR).expect(\"embedded SLIR\");\n\x20       let \
			 mut inst = kframe::inst_shell();\n\x20       inst.doc = doc;\n\x20       \
			 kframe::inst_init(&mut inst);\n\x20       Self {{\n\x20           inst,\n\x20           \
			 imgs,\n{list_cache_initializers}        }}\n\x20   }}\n\n\x20   /// Whether the \
			 embedded document decoded successfully.\n\x20   pub const fn ok(&self) -> bool {{\n        \
			 self.inst.ok\n    }}\n\n\x20   /// Env for the gpu client (client code 1); portrait \
			 derives from vw < vh.\n\x20   pub fn set_env(&mut self, vw: f64, vh: f64, dark: bool, \
			 coarse: bool) {{\n\x20       kframe::inst_set_env(&mut self.inst, vw, vh, 1, dark, \
			 coarse);\n\x20   }}\n\n\x20   /// Select a compiler-declared theme; an empty name \
			 restores authored values.\n\x20   pub fn set_theme(&mut self, name: &str) -> bool \
			 {{\n\x20       kframe::inst_set_theme(&mut self.inst, name)\n\x20   }}\n\n\x20   /// \
			 Drop generated list reconciliation snapshots after an external document reload.\n\x20   \
			 /// Call this when a host-mounted `RequestPump` reports `reloaded == true`,\n\x20   /// \
			 before re-synchronizing typed list setters. Safe and idempotent.\n\x20   pub fn \
			 invalidate_caches(&mut self) {{\n{cache_invalidations}    }}\n\n\x20   /// Read one \
			 token resolved through the active theme, with base fallback.\n\x20   pub fn \
			 get_token(&self, path: &str) -> Option<kframe::TokenValue<'_>> {{\n\x20       \
			 kframe::inst_get_token(&self.inst, path)\n\x20   }}\n\n\x20   /// Move focus to a keyed \
			 focusable node; an empty key clears focus.\n\x20   /// `visible` shows the \
			 keyboard-grade focus ring.\n\x20   pub fn set_focus(&mut self, key: &str, visible: \
			 bool) -> bool {{\n\x20       kframe::inst_set_focus(&mut self.inst, key, visible)\n\x20   \
			 }}\n\n\x20   /// Clear focus while retaining field edit buffers.\n\x20   pub fn \
			 clear_focus(&mut self) -> bool {{\n\x20       kframe::inst_clear_focus(&mut \
			 self.inst)\n\x20   }}\n\n\x20   /// Focus and reveal a concrete item in an `each` \
			 list.\n\x20   pub fn focus_item(&mut self, each_key: &str, index: i32) -> bool {{\n\x20       \
			 kframe::inst_focus_item(&mut self.inst, each_key, index)\n\x20   }}\n\n\x20   /// \
			 Explain the most recent failed focus operation.\n\x20   pub fn focus_note(&self) -> \
			 &str {{\n\x20       kframe::inst_focus_note(&self.inst)\n\x20   }}\n\n\x20   /// \
			 Replace a keyed field buffer, reset its edit history, and emit Change.\n\x20   pub fn \
			 set_field_text(&mut self, key: &str, text: &str) -> bool {{\n\x20       \
			 kframe::inst_set_field_text(&mut self.inst, key, text)\n\x20   }}\n\n\x20   /// Return \
			 a keyed field's committed text, or `None` for a non-field key.\n\x20   pub fn \
			 field_text(&self, key: &str) -> Option<String> {{\n\x20       \
			 kframe::inst_field_text(&self.inst, key)\n\x20   }}\n\n\x20   /// Current theme name; \
			 empty means the authored base.\n\x20   pub fn theme(&self) -> String {{\n\x20       \
			 kframe::inst_theme(&self.inst)\n\x20   }}\n\n\x20   /// Solve and flatten the document \
			 at `t_ms` milliseconds.\n\x20   pub fn frame(&mut self, t_ms: f64) -> Frame {{\n\x20       \
			 kframe::inst_frame(&mut self.inst, t_ms)\n\x20   }}\n\n\x20   /// Return the current \
			 host-content hole rectangles.\n\x20   pub fn holes(&mut self) -> Vec<HoleRect> {{\n\x20       \
			 kframe::inst_holes(&mut self.inst)\n\x20   }}\n\n\x20   /// Set a keyed scroll node's \
			 offset on axis 0 (main) or 1 (cross).\n\x20   pub fn set_scroll(&mut self, key: &str, \
			 axis: u32, off: f64) -> bool {{\n\x20       kframe::inst_set_scroll(&mut self.inst, \
			 key, axis, off)\n\x20   }}\n\n\x20   /// Read a keyed scroll node's offset on axis 0 \
			 (main) or 1 (cross).\n\x20   pub fn get_scroll(&self, key: &str, axis: u32) -> f64 \
			 {{\n\x20       kframe::inst_get_scroll(&self.inst, key, axis)\n\x20   }}\n\n\x20   /// \
			 Register or replace a named runtime image and return its unified image index.\n\x20   \
			 pub fn img_register(\n\x20       &mut self,\n\x20       name: &str,\n\x20       w: \
			 u32,\n\x20       h: u32,\n\x20       format: u32,\n\x20       data: &[u8],\n\x20   ) -> \
			 i32 {{\n\x20       kframe::inst_img_register(&mut self.inst, name, w, h, format, \
			 data)\n\x20   }}\n\n\x20   /// Unregister a named runtime image while preserving its \
			 unified image index.\n\x20   pub fn img_unregister(&mut self, name: &str) -> bool \
			 {{\n\x20       kframe::inst_img_unregister(&mut self.inst, name)\n\x20   }}\n\n\x20   \
			 /// Scroll ancestors minimally to reveal a keyed node with the requested margin.\n\x20   \
			 pub fn reveal(&mut self, key: &str, margin: f64) -> bool {{\n\x20       \
			 kframe::inst_reveal(&mut self.inst, key, margin)\n\x20   }}\n\n\x20   /// Reveal an \
			 item in a virtual list using start, center, end, or nearest alignment.\n\x20   pub fn \
			 reveal_item(&mut self, each_key: &str, index: i32, align: u32) -> bool {{\n\x20       \
			 kframe::inst_reveal_item(&mut self.inst, each_key, index, align)\n\x20   }}\n\n\x20   \
			 /// Return a virtual list's materialized half-open item window.\n\x20   pub fn \
			 each_window(&self, each_key: &str) -> (i32, i32) {{\n\x20       \
			 kframe::inst_each_window(&self.inst, each_key)\n\x20   }}\n\n\x20   /// Set the extent \
			 overlay controlled by a keyed divider.\n\x20   pub fn set_divider(&mut self, key: &str, \
			 extent: f64) -> bool {{\n\x20       kframe::inst_set_divider(&mut self.inst, key, \
			 extent)\n\x20   }}\n\n\x20   /// Read a keyed divider's extent overlay, or `-1.0` when \
			 unknown or unset.\n\x20   pub fn get_divider(&self, key: &str) -> f64 {{\n\x20       \
			 kframe::inst_get_divider(&self.inst, key)\n\x20   }}\n\n\x20   /// Dispatch an input \
			 event and return its raw and typed effects.\n\x20   pub fn dispatch(&mut self, ev: \
			 &Event) -> (Effects, Vec<Signal>) {{\n\x20       let eff = kframe::inst_dispatch(&mut \
			 self.inst, ev);\n\x20       let sigs = self.decode_signals(&eff);\n\x20       (eff, \
			 sigs)\n\x20   }}\n\n\x20   /// Drain solve-time signals queued by the most recent \
			 frame.\n\x20   pub fn take_signals(&mut self) -> (Effects, Vec<Signal>) {{\n\x20       \
			 let eff = kframe::inst_take_signals(&mut self.inst);\n\x20       let sigs = \
			 self.decode_signals(&eff);\n\x20       (eff, sigs)\n\x20   }}\n\n\x20   /// Decode \
			 Effects signal refs against the document's STRS pool.\n\x20   pub fn \
			 decode_signals(&self, eff: &Effects) -> Vec<Signal> {{\n\x20       let mut out = \
			 Vec::new();\n\x20       {loop_head}\n\x20           match name {{"
		);
	for (name, has_text) in &sigs {
		let konst = format!("SIG_{}", snake(name).to_uppercase());
		if *has_text {
			let _ = writeln!(
				o,
				"                {konst} => out.push(Signal::{} {{ text: eff.sig_text[i].clone(), \
				 item: eff.sig_item[i].clone(), meta: SignalMeta::from(&eff.sig_meta[i]) }}),",
				pascal(name)
			);
		} else {
			let _ = writeln!(
				o,
				"                {konst} => out.push(Signal::{} {{ item: eff.sig_item[i].clone(), \
				 meta: SignalMeta::from(&eff.sig_meta[i]) }}),",
				pascal(name)
			);
		}
	}
	let _ = writeln!(
		o,
		"                _ => {{}}\n\x20           }}\n\x20       }}\n\x20       out\n\x20   }}"
	);

	for (i, p) in slir.params.iter().enumerate() {
		let name = slir.str_at(p.name).to_string();
		let method = snake(&name);
		if p.ty == 6 {
			let schema_row = slir
				.lists
				.iter()
				.position(|schema| schema.param == i as u32)
				.expect("list param missing LIST schema");
			emit_list_setters(&mut o, slir, i, schema_row, &name);
			continue;
		}
		let (sig, fill) = match p.ty {
			0 => ("v: &str", "pv.s = v.to_string();"),
			1 => ("v: f64", "pv.num = v;"),
			2 => ("v: f64", "pv.num = v;"),
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
		let _ = writeln!(
			o,
			"\n    /// Set param `{name}` ({doc_note}); false = rejected by the kernel.\n\x20   pub \
			 fn set_{method}(&mut self, {sig}) -> bool {{\n\x20       let mut pv = ParamValue \
			 {{\n\x20           kind: {ty},\n\x20           num: 0.0,\n\x20           s: \
			 String::new(),\n\x20           rgba: 0,\n\x20           sym: String::new(),\n\x20       \
			 }};\n\x20       {fill}\n\x20       kframe::inst_set_param(&mut self.inst, {i}, \
			 &pv)\n\x20   }}",
			ty = p.ty
		);
	}
	let _ = writeln!(o, "}}");
	o
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
