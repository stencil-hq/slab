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
//! Unlike [`crate::rustgen`], which links the kernel directly, the Go binding
//! has no in-process kernel. The document is therefore lowered to SLIR at
//! generation time, embedded base64-encoded, decoded once at package
//! initialization, and installed through the runtime's `doc.open_slir` helper.
//! The runtime never runs the compiler for a generated binding.
//!
//! Output is deterministic and gofmt-clean as emitted. Regenerate with:
//! `cargo run -q -p slab-cli -- gen go FILE -o OUT.go [--package NAME]`

use std::fmt::Write as _;

use slab_slir::Slir;
use slab_syntax::diag::Diagnostics;

use crate::Options;

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

/// lowerCamelCase from a param name (`row-count` -> `rowCount`), for the
/// unexported cache fields and helper methods of the generated `Doc`.
fn camel(s: &str) -> String {
	let mut out = pascal(s);
	let Some(first) = out.chars().next() else {
		return out;
	};
	let lowered: String = first.to_lowercase().collect();
	out.replace_range(0..first.len_utf8(), &lowered);
	out
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

/// Quote a string as a Go interpreted string literal.
fn go_string(s: &str) -> String {
	let mut out = String::with_capacity(s.len() + 2);
	out.push('"');
	for c in s.chars() {
		match c {
			'\\' => out.push_str("\\\\"),
			'"' => out.push_str("\\\""),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 || c as u32 == 0x7f => {
				let _ = write!(out, "\\x{:02x}", c as u32);
			},
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// Emit `const NAME = "" +` followed by one quoted chunk per line. gofmt keeps
/// this shape untouched, so long payloads stay readable without reflowing.
fn emit_string_const(out: &mut String, name: &str, parts: &[String]) {
	if parts.is_empty() {
		let _ = writeln!(out, "const {name} = \"\"");
		return;
	}
	let _ = writeln!(out, "const {name} = \"\" +");
	for (index, part) in parts.iter().enumerate() {
		let tail = if index + 1 == parts.len() { "" } else { " +" };
		let _ = writeln!(out, "\t{part}{tail}");
	}
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

/// The `XxxItem` type name stripped of its `Item` suffix, used to build the
/// package-level `validateXxxItems` / `equalXxxItems` / `cloneXxxItems`
/// helpers and the `setXxxPath` reconciliation method.
fn list_base_name(item_ty: &str) -> String {
	item_ty.strip_suffix("Item").unwrap_or(item_ty).to_string()
}

/// SDP `list.set_field` kind spelling for a PARAM type code.
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

/// The Go type of one scalar list field, plus the human note used in its doc
/// comment. `owner` is the item type the field belongs to, which names the
/// generated enum type for symbol fields.
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

/// The Go expression that carries one scalar list field to `list.set_field`.
fn field_wire_expr(ty: u8, member: &str) -> String {
	match ty {
		3 => format!("uint32(item.{member})"),
		5 => format!("string(item.{member})"),
		_ => format!("item.{member}"),
	}
}

/// Emit the item structs, enum types, and equality/clone/validation helpers for
/// one list param and every schema reachable from it.
fn emit_list_types(out: &mut String, slir: &Slir, schema_row: usize, param_name: &str) {
	let mut names = Vec::new();
	let mut order = Vec::new();
	collect_list_types(slir, schema_row, pascal(param_name), &mut names, &mut order);
	let root = canonical_list_schema(slir, schema_row);
	for &row in &order {
		let schema = &slir.lists[row];
		let item_ty = list_type_name(&names, slir, row);
		let base = list_base_name(&item_ty);

		for field_ix in schema.field_off..schema.field_off + schema.field_len {
			let field = &slir.list_fields[field_ix as usize];
			if field.ty != 5 {
				continue;
			}
			let field_name = slir.str_at(field.name);
			let enum_ty = format!("{item_ty}{}", pascal(field_name));
			let _ = writeln!(
				out,
				"// {enum_ty} is the value type of schema field `{field_name}` on [{item_ty}]."
			);
			let _ = writeln!(out, "type {enum_ty} string\n");
			for ix in field.enum_off..field.enum_off + field.enum_len {
				let member = slir.str_at(slir.list_enum_syms[ix as usize]);
				let konst = format!("{enum_ty}{}", pascal(member));
				let _ = writeln!(
					out,
					"// {konst} is member `{member}` of schema field `{field_name}` on [{item_ty}]."
				);
				let _ = writeln!(out, "const {konst} {enum_ty} = {}\n", go_string(member));
			}
		}

		if row == root {
			let _ = writeln!(
				out,
				"// {item_ty} is one typed item accepted by [Doc.Set{}] for list param `{param_name}`.",
				pascal(param_name)
			);
		} else {
			let _ = writeln!(
				out,
				"// {item_ty} is one typed nested-list item reachable from list param `{param_name}`."
			);
		}
		let _ = writeln!(out, "type {item_ty} struct {{");
		let _ = writeln!(
			out,
			"\t// Key is the stable list identity; an empty Key uses the positional key."
		);
		let _ = writeln!(out, "\tKey string");
		for field_ix in schema.field_off..schema.field_off + schema.field_len {
			let field = &slir.list_fields[field_ix as usize];
			let field_name = slir.str_at(field.name);
			let member = pascal(field_name);
			let (go_ty, note) = if field.ty == 6 {
				(
					format!("[]{}", list_type_name(&names, slir, field.sub as usize - 1)),
					"nested list".to_string(),
				)
			} else {
				scalar_field_type(slir, field, &item_ty)
			};
			let _ = writeln!(out, "\t// {member} is schema field `{field_name}` ({note}).");
			let _ = writeln!(out, "\t{member} {go_ty}");
		}
		let _ = writeln!(out, "}}\n");

		let _ = writeln!(
			out,
			"// equals reports whether two {item_ty} values carry identical field values."
		);
		let _ = writeln!(out, "func (a {item_ty}) equals(b {item_ty}) bool {{");
		let _ = writeln!(out, "\tif a.Key != b.Key {{\n\t\treturn false\n\t}}");
		for field_ix in schema.field_off..schema.field_off + schema.field_len {
			let field = &slir.list_fields[field_ix as usize];
			let member = pascal(slir.str_at(field.name));
			if field.ty == 6 {
				let child_ty = list_type_name(&names, slir, field.sub as usize - 1);
				let child_base = list_base_name(&child_ty);
				let _ = writeln!(
					out,
					"\tif !equal{child_base}Items(a.{member}, b.{member}) {{\n\t\treturn false\n\t}}"
				);
			} else {
				let _ = writeln!(out, "\tif a.{member} != b.{member} {{\n\t\treturn false\n\t}}");
			}
		}
		let _ = writeln!(out, "\treturn true\n}}\n");

		let _ = writeln!(
			out,
			"// equal{base}Items reports whether two {item_ty} slices are element-wise equal."
		);
		let _ = writeln!(
			out,
			"func equal{base}Items(left, right []{item_ty}) bool {{\n\tif len(left) != len(right) \
			 {{\n\t\treturn false\n\t}}\n\tfor index := range left {{\n\t\tif \
			 !left[index].equals(right[index]) {{\n\t\t\treturn false\n\t\t}}\n\t}}\n\treturn \
			 true\n}}\n"
		);

		let nested: Vec<(String, String)> = (schema.field_off..schema.field_off + schema.field_len)
			.filter_map(|field_ix| {
				let field = &slir.list_fields[field_ix as usize];
				if field.ty != 6 {
					return None;
				}
				let child_ty = list_type_name(&names, slir, field.sub as usize - 1);
				Some((pascal(slir.str_at(field.name)), list_base_name(&child_ty)))
			})
			.collect();
		let _ = writeln!(
			out,
			"// clone{base}Items deep-copies items so a cached snapshot never aliases\n// \
			 caller-owned state."
		);
		let _ = writeln!(
			out,
			"func clone{base}Items(items []{item_ty}) []{item_ty} {{\n\tif items == nil \
			 {{\n\t\treturn nil\n\t}}\n\tout := make([]{item_ty}, len(items))\n\tcopy(out, items)"
		);
		if !nested.is_empty() {
			let _ = writeln!(out, "\tfor index := range out {{");
			for (member, child_base) in &nested {
				let _ = writeln!(
					out,
					"\t\tout[index].{member} = clone{child_base}Items(items[index].{member})"
				);
			}
			let _ = writeln!(out, "\t}}");
		}
		let _ = writeln!(out, "\treturn out\n}}\n");

		let _ = writeln!(
			out,
			"// validate{base}Items rejects an oversized list, duplicate item keys, and unknown\n// \
			 enum members before any write reaches the session."
		);
		let _ = writeln!(
			out,
			"func validate{base}Items(items []{item_ty}) error {{\n\tif len(items) > maxListLen \
			 {{\n\t\treturn fmt.Errorf(\"{item_ty} list has %d items, more than the %d the protocol \
			 allows\", len(items), maxListLen)\n\t}}\n\tkeys := make(map[string]struct{{}}, \
			 len(items))\n\tfor index, item := range items {{\n\t\tkey := item.Key\n\t\tif key == \
			 \"\" {{\n\t\t\tkey = strconv.Itoa(index)\n\t\t}}\n\t\tif _, seen := keys[key]; seen \
			 {{\n\t\t\treturn fmt.Errorf(\"{item_ty} %d: duplicate key %q\", index, \
			 key)\n\t\t}}\n\t\tkeys[key] = struct{{}}{{}}"
		);
		for field_ix in schema.field_off..schema.field_off + schema.field_len {
			let field = &slir.list_fields[field_ix as usize];
			let field_name = slir.str_at(field.name);
			let member = pascal(field_name);
			if field.ty == 5 {
				let enum_ty = format!("{item_ty}{member}");
				let members: Vec<String> = (field.enum_off..field.enum_off + field.enum_len)
					.map(|ix| {
						format!("{enum_ty}{}", pascal(slir.str_at(slir.list_enum_syms[ix as usize])))
					})
					.collect();
				let _ = writeln!(out, "\t\tswitch item.{member} {{");
				if !members.is_empty() {
					let _ = writeln!(out, "\t\tcase {}:", members.join(", "));
				}
				let _ = writeln!(
					out,
					"\t\tdefault:\n\t\t\treturn fmt.Errorf(\"{item_ty} %d: field `{field_name}` has \
					 unknown enum member %q\", index, string(item.{member}))\n\t\t}}"
				);
			} else if field.ty == 6 {
				let child_ty = list_type_name(&names, slir, field.sub as usize - 1);
				let child_base = list_base_name(&child_ty);
				let _ = writeln!(
					out,
					"\t\tif err := validate{child_base}Items(item.{member}); err != nil \
					 {{\n\t\t\treturn err\n\t\t}}"
				);
			}
		}
		let _ = writeln!(out, "\t}}\n\treturn nil\n}}\n");
	}
}

/// Emit the reconciliation methods for one list param: one `setXxxPath` per
/// reachable schema plus the exported `SetXxx` entry point.
fn emit_list_setters(out: &mut String, slir: &Slir, schema_row: usize, param_name: &str) {
	let mut names = Vec::new();
	let mut order = Vec::new();
	collect_list_types(slir, schema_row, pascal(param_name), &mut names, &mut order);
	let wire_param = go_string(param_name);
	for &row in &order {
		let schema = &slir.lists[row];
		let item_ty = list_type_name(&names, slir, row);
		let base = list_base_name(&item_ty);
		let _ = writeln!(
			out,
			"// set{base}Path reconciles the {item_ty} list at path against the previous value,\n// \
			 writing only the entries that changed."
		);
		let _ = writeln!(
			out,
			"func (d *Doc) set{base}Path(ctx context.Context, path string, items, previous \
			 []{item_ty}) error {{\n\tif err := d.setListLen(ctx, {wire_param}, path, len(items)); \
			 err != nil {{\n\t\treturn err\n\t}}\n\tfor index, item := range items {{\n\t\tvar prior \
			 *{item_ty}\n\t\tif index < len(previous) {{\n\t\t\tprior = \
			 &previous[index]\n\t\t}}\n\t\tif prior != nil && prior.equals(item) \
			 {{\n\t\t\tcontinue\n\t\t}}\n\t\tif prior == nil || prior.Key != item.Key {{\n\t\t\tkey \
			 := item.Key\n\t\t\tif key == \"\" {{\n\t\t\t\tkey = \
			 strconv.Itoa(index)\n\t\t\t}}\n\t\t\tif err := d.setListKey(ctx, {wire_param}, path, \
			 index, key); err != nil {{\n\t\t\t\treturn err\n\t\t\t}}\n\t\t}}"
		);
		for field_ix in schema.field_off..schema.field_off + schema.field_len {
			let field = &slir.list_fields[field_ix as usize];
			let field_name = slir.str_at(field.name);
			let member = pascal(field_name);
			if field.ty == 6 {
				let child_ty = list_type_name(&names, slir, field.sub as usize - 1);
				let child_base = list_base_name(&child_ty);
				let _ = writeln!(
					out,
					"\t\tchildPath := strconv.Itoa(index) + {}\n\t\tif path != \"\" \
					 {{\n\t\t\tchildPath = path + \".\" + childPath\n\t\t}}\n\t\tvar prior{member} \
					 []{child_ty}\n\t\tif prior != nil {{\n\t\t\tprior{member} = \
					 prior.{member}\n\t\t}}\n\t\tif err := d.set{child_base}Path(ctx, childPath, \
					 item.{member}, prior{member}); err != nil {{\n\t\t\treturn err\n\t\t}}",
					go_string(&format!(".{field_name}"))
				);
				continue;
			}
			let _ = writeln!(
				out,
				"\t\tif prior == nil || prior.{member} != item.{member} {{\n\t\t\tif err := \
				 d.setListField(ctx, {wire_param}, path, index, {}, {}, {}); err != nil \
				 {{\n\t\t\t\treturn err\n\t\t\t}}\n\t\t}}",
				go_string(field_name),
				go_string(kind_name(field.ty)),
				field_wire_expr(field.ty, &member)
			);
		}
		let _ = writeln!(out, "\t}}\n\treturn nil\n}}\n");
	}
	let root_ty = list_type_name(&names, slir, schema_row);
	let root_base = list_base_name(&root_ty);
	let setter = pascal(param_name);
	let cache = format!("{}Cache", camel(param_name));
	let _ = writeln!(
		out,
		"// Set{setter} reconciles list param `{param_name}`, and every nested list under it,\n// \
		 with the last applied value. Unchanged items and fields are never rewritten. A\n// failed \
		 write restores the previous snapshot so the next call resynchronizes."
	);
	let _ = writeln!(
		out,
		"func (d *Doc) Set{setter}(ctx context.Context, items []{root_ty}) error {{\n\tif \
		 d.{cache}Valid && equal{root_base}Items(d.{cache}, items) {{\n\t\treturn nil\n\t}}\n\tif \
		 err := validate{root_base}Items(items); err != nil {{\n\t\treturn err\n\t}}\n\tprevious := \
		 d.{cache}\n\tpreviousValid := d.{cache}Valid\n\td.{cache} = nil\n\td.{cache}Valid = \
		 false\n\tif err := d.set{root_base}Path(ctx, \"\", items, previous); err != nil \
		 {{\n\t\td.{cache} = previous\n\t\td.{cache}Valid = previousValid\n\t\treturn \
		 err\n\t}}\n\td.{cache} = clone{root_base}Items(items)\n\td.{cache}Valid = true\n\treturn \
		 nil\n}}\n"
	);
}

/// The schema row backing a list param, by position in `Slir::lists`.
fn schema_row_of(slir: &Slir, param: usize) -> usize {
	slir
		.lists
		.iter()
		.position(|schema| schema.param == param as u32)
		.expect("list param missing LIST schema")
}

fn emit_module(slir: &Slir, bytes: &[u8], src_name: &str, package: &str) -> String {
	let mut o = String::new();
	let signals = unique_signals(slir);
	let has_list = slir.params.iter().any(|p| p.ty == 6);
	let has_scalar = slir.params.iter().any(|p| p.ty != 6);
	let has_color =
		slir.params.iter().any(|p| p.ty == 3) || slir.list_fields.iter().any(|f| f.ty == 3);

	let _ = writeln!(o, "// GENERATED by `slab gen go {src_name}` — do not edit.");
	let _ = writeln!(
		o,
		"// Regenerate: cargo run -q -p slab-cli -- gen go {src_name} -o <this file> --package \
		 {package}\n"
	);
	let _ = writeln!(
		o,
		"// Package {package} is the generated typed binding for `{src_name}`. It drives a\n// slab \
		 session over the Slab Drive Protocol; the kernel owns layout, hit testing,\n// focus, and \
		 editing, and this package only carries typed values across the wire."
	);
	let _ = writeln!(o, "package {package}\n");

	o.push_str("import (\n\t\"context\"\n\t\"encoding/base64\"\n");
	if has_list || has_color {
		o.push_str("\t\"fmt\"\n");
	}
	if has_list {
		o.push_str("\t\"strconv\"\n");
	}
	o.push_str("\n\t\"github.com/stencil-hq/slab/clients/go/slab\"\n)\n\n");

	let _ = writeln!(
		o,
		"// SourceName is the document name reported to the session when [New] installs [SLIR]."
	);
	let _ = writeln!(o, "const SourceName = {}\n", go_string(src_name));

	let _ = writeln!(
		o,
		"// slirBase64 is the compiled SLIR document ({} bytes) in standard base64. Base64\n// \
		 keeps the generated file compact and byte-for-byte reproducible across runs.",
		bytes.len()
	);
	let encoded = base64(bytes);
	let chunks: Vec<String> = encoded
		.as_bytes()
		.chunks(76)
		.map(|chunk| {
			let mut part = String::with_capacity(chunk.len() + 2);
			part.push('"');
			part.push_str(std::str::from_utf8(chunk).expect("base64 output is ASCII"));
			part.push('"');
			part
		})
		.collect();
	emit_string_const(&mut o, "slirBase64", &chunks);
	o.push_str(
		"\n// SLIR is the compiled SLIR document this package installs, decoded once at\n// package \
		 initialization. The slice is shared, so callers must not modify it.\nvar SLIR []byte\n\n// \
		 init decodes the embedded document. A corrupt payload can only come from a\n// hand-edited \
		 generated file, so it fails loudly rather than silently.\nfunc init() {\n\tdecoded, err := \
		 base64.StdEncoding.DecodeString(slirBase64)\n\tif err != nil {\n\t\tpanic(\"slab: embedded \
		 SLIR payload is corrupt: \" + err.Error())\n\t}\n\tSLIR = decoded\n}\n\n",
	);

	if has_color {
		o.push_str(
			"// Rgba is a packed SLIR color word: red in the low byte, then green, blue, \
			 alpha.\ntype Rgba uint32\n\n// NewRgba packs straight-alpha channels into a SLIR color \
			 word. Go cannot give a\n// function and a type the same name, so the constructor is \
			 NewRgba, not Rgba.\nfunc NewRgba(red, green, blue, alpha uint8) Rgba {\n\treturn \
			 Rgba(uint32(red) | uint32(green)<<8 | uint32(blue)<<16 | uint32(alpha)<<24)\n}\n\n// \
			 String renders the `#rrggbbaa` spelling that `param.set` accepts for colors.\nfunc (c \
			 Rgba) String() string {\n\treturn fmt.Sprintf(\"#%02x%02x%02x%02x\", uint8(c), \
			 uint8(c>>8), uint8(c>>16), uint8(c>>24))\n}\n\n",
		);
	}

	if has_list {
		o.push_str(
			"// maxListLen is the largest list length the protocol's int32 item count can \
			 carry.\nconst maxListLen = 2147483647\n\n",
		);
	}

	for (name, key) in crate::wc::static_scene_keys(slir) {
		let konst = format!("Key{}", pascal(&name));
		let _ = writeln!(o, "// {konst} is the canonical scene key of the authored `#{name}` node.");
		let _ = writeln!(o, "const {konst} = {}\n", go_string(&key));
	}

	for p in &slir.params {
		let name = slir.str_at(p.name);
		let konst = format!("Param{}", pascal(name));
		let _ = writeln!(
			o,
			"// {konst} is the SDP name of param `{name}` ({}).",
			slab_slir::PARAM_TYPE_NAMES
				.get(usize::from(p.ty))
				.copied()
				.unwrap_or("unknown")
		);
		let _ = writeln!(o, "const {konst} = {}\n", go_string(name));
	}

	for p in &slir.params {
		if p.ty != 5 {
			continue;
		}
		let name = slir.str_at(p.name);
		let enum_ty = pascal(name);
		let _ = writeln!(o, "// {enum_ty} is the value type of enum param `{name}`.");
		let _ = writeln!(o, "type {enum_ty} string\n");
		for ix in p.enum_off..p.enum_off + p.enum_len {
			let member = slir.str_at(slir.param_enum_syms[ix as usize]);
			let konst = format!("{enum_ty}{}", pascal(member));
			let _ = writeln!(o, "// {konst} is member `{member}` of enum param `{name}`.");
			let _ = writeln!(o, "const {konst} {enum_ty} = {}\n", go_string(member));
		}
	}

	o.push_str(
		"// SignalName is one authored signal name from the document's SIGN table.\ntype SignalName \
		 string\n\n",
	);
	for (name, has_text) in &signals {
		let konst = format!("Signal{}", pascal(name));
		let note = if *has_text {
			"it carries a text payload"
		} else {
			"it carries no text payload"
		};
		let _ = writeln!(o, "// {konst} is the authored signal `{name}`; {note}.");
		let _ = writeln!(o, "const {konst} SignalName = {}\n", go_string(name));
	}
	o.push_str(
		"// Signal is one decoded emission of a signal this document declares.\ntype Signal struct \
		 {\n\t// Name is the authored signal name.\n\tName SignalName\n\t// Text is the committed \
		 field text or final resize extent, and is empty for\n\t// signals that carry no text \
		 payload.\n\tText string\n\t// Item is the innermost list item key, or empty outside a \
		 list.\n\tItem string\n\t// Meta is the input and source metadata captured at \
		 emission.\n\tMeta slab.SignalMeta\n}\n\n",
	);
	if signals.is_empty() {
		o.push_str(
			"// DecodeSignals converts the ordered effect signals into typed values. This\n// \
			 document declares no signals, so the result is always empty.\nfunc \
			 DecodeSignals(effects slab.Effects) []Signal {\n\treturn nil\n}\n\n",
		);
	} else {
		o.push_str(
			"// DecodeSignals converts the ordered effect signals into typed values, keeping \
			 only\n// the names this document declares and preserving emission order.\nfunc \
			 DecodeSignals(effects slab.Effects) []Signal {\n\tout := make([]Signal, 0, \
			 len(effects.Signals))\n\tfor _, raw := range effects.Signals {\n\t\tname := \
			 SignalName(raw.Name)\n\t\tswitch name {\n",
		);
		let cases: Vec<String> = signals
			.iter()
			.map(|(name, _)| format!("Signal{}", pascal(name)))
			.collect();
		let _ = writeln!(o, "\t\tcase {}:", cases.join(", "));
		o.push_str(
			"\t\tdefault:\n\t\t\tcontinue\n\t\t}\n\t\tout = append(out, Signal{Name: name, Text: \
			 raw.Text, Item: raw.Item, Meta: raw.Meta})\n\t}\n\treturn out\n}\n\n",
		);
	}

	for (param_ix, p) in slir.params.iter().enumerate() {
		if p.ty != 6 {
			continue;
		}
		emit_list_types(&mut o, slir, schema_row_of(slir, param_ix), slir.str_at(p.name));
	}

	o.push_str(
		"// Doc is the typed wrapper over a session holding this document. Every setter\n// routes \
		 through the Slab Drive Protocol, so the kernel stays the single owner of\n// layout, hit \
		 testing, focus, and editing.\ntype Doc struct {\n\t// sess is the session the document was \
		 opened in.\n\tsess *slab.Session\n",
	);
	for (param_ix, p) in slir.params.iter().enumerate() {
		if p.ty != 6 {
			continue;
		}
		let name = slir.str_at(p.name);
		let cache = format!("{}Cache", camel(name));
		let schema_row = schema_row_of(slir, param_ix);
		let mut names = Vec::new();
		let mut order = Vec::new();
		collect_list_types(slir, schema_row, pascal(name), &mut names, &mut order);
		let root_ty = list_type_name(&names, slir, schema_row);
		let _ = writeln!(o, "\t// {cache} is the last value applied by [Doc.Set{}].", pascal(name));
		let _ = writeln!(o, "\t{cache} []{root_ty}");
		let _ = writeln!(o, "\t// {cache}Valid reports whether {cache} holds an applied value.");
		let _ = writeln!(o, "\t{cache}Valid bool");
	}
	o.push_str("}\n\n");

	o.push_str(
		"// New installs the embedded document in sess and returns the typed wrapper. The\n// \
		 document is already compiled, so the runtime never parses `.slab` text and no\n// host \
		 filesystem is involved.\nfunc New(ctx context.Context, sess *slab.Session) (*Doc, error) \
		 {\n\tif err := sess.OpenSLIR(ctx, SLIR, SourceName); err != nil {\n\t\treturn nil, \
		 err\n\t}\n\treturn &Doc{sess: sess}, nil\n}\n\n// Session returns the underlying session, \
		 the escape hatch for SDP methods this\n// binding does not wrap.\nfunc (d *Doc) Session() \
		 *slab.Session {\n\treturn d.sess\n}\n\n",
	);
	// Only a document with list params retains reconciliation snapshots.
	if has_list {
		o.push_str(
			"// InvalidateCaches drops the generated list reconciliation snapshots. Call it \
			 after\n// the document reloads underneath the session, before re-synchronizing the \
			 typed\n// list setters. It is safe and idempotent.\nfunc (d *Doc) InvalidateCaches() {\n",
		);
		for p in &slir.params {
			if p.ty != 6 {
				continue;
			}
			let cache = format!("{}Cache", camel(slir.str_at(p.name)));
			let _ = writeln!(o, "\td.{cache} = nil");
			let _ = writeln!(o, "\td.{cache}Valid = false");
		}
		o.push_str("}\n\n");
	}

	if has_scalar {
		o.push_str(
			"// setParam writes one scalar param through the protocol's `param.set` method.\nfunc (d \
			 *Doc) setParam(ctx context.Context, name string, value any) error {\n\t_, err := \
			 d.sess.Request(ctx, \"param.set\", map[string]any{\"name\": name, \"value\": \
			 value})\n\treturn err\n}\n\n",
		);
	}
	if has_list {
		o.push_str(
			"// setListLen resizes the list a param path addresses.\nfunc (d *Doc) setListLen(ctx \
			 context.Context, param, path string, n int) error {\n\t_, err := d.sess.Request(ctx, \
			 \"list.set_len\", map[string]any{\"param\": param, \"path\": path, \"n\": n})\n\treturn \
			 err\n}\n\n// setListKey writes one list item's stable identity.\nfunc (d *Doc) \
			 setListKey(ctx context.Context, param, path string, index int, key string) error \
			 {\n\t_, err := d.sess.Request(ctx, \"list.set_key\", map[string]any{\"param\": param, \
			 \"path\": path, \"index\": index, \"key\": key})\n\treturn err\n}\n\n// setListField \
			 writes one typed field of one list item.\nfunc (d *Doc) setListField(ctx \
			 context.Context, param, path string, index int, field, kind string, value any) error \
			 {\n\t_, err := d.sess.Request(ctx, \"list.set_field\", map[string]any{\"param\": param, \
			 \"path\": path, \"index\": index, \"field\": field, \"kind\": kind, \"value\": \
			 value})\n\treturn err\n}\n\n",
		);
	}

	for p in &slir.params {
		if p.ty == 6 {
			continue;
		}
		let name = slir.str_at(p.name);
		let setter = pascal(name);
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
		let _ = writeln!(o, "// Set{setter} sets param `{name}` ({note}).");
		let _ = writeln!(
			o,
			"func (d *Doc) Set{setter}(ctx context.Context, value {go_ty}) error {{\n\treturn \
			 d.setParam(ctx, {}, {wire})\n}}\n",
			go_string(name)
		);
	}

	for (param_ix, p) in slir.params.iter().enumerate() {
		if p.ty != 6 {
			continue;
		}
		emit_list_setters(&mut o, slir, schema_row_of(slir, param_ix), slir.str_at(p.name));
	}
	// gofmt allows exactly one newline at end of file.
	while o.ends_with('\n') {
		o.pop();
	}
	o.push('\n');
	o
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
