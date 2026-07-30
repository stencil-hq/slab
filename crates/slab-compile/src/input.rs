//! Typed host-input coercion shared by CLI, TUI, wasm, and static rendering.

use serde_json::Value;
use slab_kernel::{
	frame::{self as kframe, Instance, ParamValue},
	slir::{self as kslir, Doc},
};

const PARAM_LIST: u32 = 6;

const FIELD_LIST: u32 = 6;

/// One path-addressed kernel list write, emitted in application order.
#[derive(Debug)]
enum ListOp {
	Len { path: String, n: i32 },
	Key { path: String, index: i32, key: String },
	Field { path: String, index: i32, field: String, value: ParamValue },
}

#[derive(Debug)]
enum Prepared {
	Scalar { param: u32, value: ParamValue },
	List { param: u32, ops: Vec<ListOp> },
}

const fn empty_value(kind: u32) -> ParamValue {
	ParamValue { kind, num: 0.0, s: String::new(), rgba: 0, sym: String::new() }
}

/// Coerce the scalar spelling shared by CLI `--set` and SDP `param.set`.
///
/// This preserves the historical text/number/percentage/color/bool/enum rules.
/// Percentages accept `60` or `"60%"` and are unclamped: `pct` is the generic
/// parent-relative percentage type, so values above 100% stay legitimate.
pub fn coerce_scalar(kind: u32, raw: &str) -> Result<ParamValue, String> {
	let mut value = empty_value(kind);
	match kind {
		0 => value.s = raw.to_string(),
		1 => {
			value.num = raw
				.parse()
				.map_err(|_| format!("'{raw}' is not a number"))?;
		},
		2 => {
			value.num = raw
				.strip_suffix('%')
				.unwrap_or(raw)
				.parse()
				.map_err(|_| format!("'{raw}' is not a percentage"))?;
		},
		3 => {
			let color =
				crate::color::parse_rgba(raw).ok_or_else(|| format!("'{raw}' is not a color"))?;
			value.rgba = crate::color::rgba_word(color);
		},
		4 => {
			value.num = match raw {
				"true" | "1" | "on" => 1.0,
				"false" | "0" | "off" => 0.0,
				_ => return Err(format!("'{raw}' is not a bool")),
			}
		},
		5 => value.sym = raw.to_string(),
		_ => return Err(format!("unsupported param type {kind}")),
	}
	Ok(value)
}

fn enum_contains(doc: &Doc, off: i32, len: i32, candidate: &str) -> bool {
	(off..off + len).any(|ix| kslir::str_at(doc, doc.list_enum_syms[ix as usize]) == candidate)
}

fn scalar_enum_contains(doc: &Doc, param: usize, candidate: &str) -> bool {
	let off = doc.parm_enum_off[param];
	(off..off + doc.parm_enum_len[param])
		.any(|ix| kslir::str_at(doc, doc.parm_enum_syms[ix as usize]) == candidate)
}

fn default_field_value(doc: &Doc, field: usize) -> ParamValue {
	let kind = doc.list_field_type[field];
	let decoded = slab_kernel::value::decode(doc, doc.list_field_default[field] as i32);
	let mut value = empty_value(kind);
	match kind {
		0 => {
			if decoded.tag == kslir::T_STR {
				value.s.clone_from(&doc.strs[decoded.h as usize]);
			}
		},
		1 | 2 | 4 => value.num = decoded.num,
		3 => value.rgba = decoded.h,
		5 if decoded.tag == kslir::T_ENUM_SYM => {
			value.sym.clone_from(&doc.strs[decoded.h as usize]);
		},
		_ => {},
	}
	value
}

fn json_field_value(doc: &Doc, field: usize, value: &Value) -> Result<ParamValue, String> {
	let kind = doc.list_field_type[field];
	let mut out = empty_value(kind);
	match kind {
		0 => {
			out.s = value
				.as_str()
				.ok_or_else(|| "must be a string".to_string())?
				.to_string();
		},
		1 => {
			out.num = value
				.as_f64()
				.ok_or_else(|| "must be a number".to_string())?;
		},
		2 => {
			out.num = value
				.as_f64()
				.ok_or_else(|| "must be a percentage number".to_string())?;
		},
		3 => {
			let raw = value
				.as_str()
				.ok_or_else(|| "must be a color string".to_string())?;
			let color =
				crate::color::parse_rgba(raw).ok_or_else(|| format!("'{raw}' is not a color"))?;
			out.rgba = crate::color::rgba_word(color);
		},
		4 => {
			out.num = if value
				.as_bool()
				.ok_or_else(|| "must be a boolean".to_string())?
			{
				1.0
			} else {
				0.0
			};
		},
		5 => {
			let member = value
				.as_str()
				.ok_or_else(|| "must be an enum string".to_string())?;
			if !enum_contains(
				doc,
				doc.list_field_enum_off[field],
				doc.list_field_enum_len[field],
				member,
			) {
				return Err(format!("unknown enum member '{member}'"));
			}
			out.sym = member.to_string();
		},
		FIELD_LIST => return Err("list fields are prepared recursively".to_string()),
		_ => return Err(format!("unsupported field type {kind}")),
	}
	Ok(out)
}

/// Joins the kernel's `<index>.<field>` list-path grammar (`""` = root).
fn child_path(path: &str, index: usize, field: &str) -> String {
	if path.is_empty() {
		format!("{index}.{field}")
	} else {
		format!("{path}.{index}.{field}")
	}
}

/// Validates one list level and emits its writes, recursing into
/// `list(Def)`-typed fields so nested JSON replaces the whole subtree
/// (SPEC §13.6: every public surface accepts equivalent nested JSON).
fn prepare_list_level(
	doc: &Doc,
	list: usize,
	array: &[Value],
	path: &str,
	ops: &mut Vec<ListOp>,
) -> Result<(), String> {
	let field_off = doc.list_field_off[list] as usize;
	let field_len = doc.list_field_len[list] as usize;
	let len = i32::try_from(array.len()).map_err(|_| "list has too many items".to_string())?;
	ops.push(ListOp::Len { path: path.to_owned(), n: len });

	let mut keys: Vec<String> = Vec::with_capacity(array.len());
	for (index, item) in array.iter().enumerate() {
		let object = item
			.as_object()
			.ok_or_else(|| format!("item {index} must be an object"))?;
		for name in object.keys() {
			if name == "key" {
				continue;
			}
			let known = (field_off..field_off + field_len)
				.any(|field| kslir::str_at(doc, doc.list_field_name[field]) == name.as_str());
			if !known {
				return Err(format!("item {index}: unknown field '{name}'"));
			}
		}

		let key = match object.get("key") {
			Some(Value::String(key)) => key.clone(),
			Some(Value::Number(number)) => {
				let number = number
					.as_f64()
					.filter(|number| number.is_finite())
					.ok_or_else(|| {
						format!("item {index} field 'key' must be a string or finite number")
					})?;
				if number == 0.0 {
					"0".to_string()
				} else {
					number.to_string()
				}
			},
			Some(_) => {
				return Err(format!("item {index} field 'key' must be a string or finite number"));
			},
			None => index.to_string(),
		};
		if keys.iter().any(|prior| prior == &key) {
			return Err(format!("item {index}: duplicate key '{key}'"));
		}
		keys.push(key.clone());
		let item_index = i32::try_from(index).map_err(|_| "list has too many items".to_string())?;
		ops.push(ListOp::Key { path: path.to_owned(), index: item_index, key });

		for field in field_off..field_off + field_len {
			let name = kslir::str_at(doc, doc.list_field_name[field]);
			if doc.list_field_type[field] == FIELD_LIST {
				let Some(raw_value) = object.get(name) else {
					// Whole-tree replacement: an omitted List field is empty
					// (SPEC §13.6), so reapplied items shed stale children.
					ops.push(ListOp::Len { path: child_path(path, index, name), n: 0 });
					continue;
				};
				let nested = raw_value
					.as_array()
					.ok_or_else(|| format!("item {index} field '{name}': must be an array"))?;
				let sub = doc.list_field_sub[field];
				if sub == 0 {
					return Err(format!("item {index} field '{name}': missing nested schema"));
				}
				prepare_list_level(
					doc,
					(sub - 1) as usize,
					nested,
					&child_path(path, index, name),
					ops,
				)
				.map_err(|e| format!("item {index} field '{name}': {e}"))?;
				continue;
			}
			let value = match object.get(name) {
				Some(raw_value) => json_field_value(doc, field, raw_value)
					.map_err(|e| format!("item {index} field '{name}': {e}"))?,
				None => default_field_value(doc, field),
			};
			ops.push(ListOp::Field {
				path: path.to_owned(),
				index: item_index,
				field: name.to_owned(),
				value,
			});
		}
	}
	Ok(())
}

fn prepare_list(doc: &Doc, param: usize, raw: &str) -> Result<Prepared, String> {
	let json: Value = serde_json::from_str(raw).map_err(|e| format!("invalid list JSON: {e}"))?;
	let array = json
		.as_array()
		.ok_or_else(|| "list value must be a JSON array".to_string())?;
	let list = doc
		.list_param
		.iter()
		.position(|&candidate| candidate as usize == param)
		.ok_or_else(|| "list schema is missing".to_string())?;
	let mut ops = Vec::new();
	prepare_list_level(doc, list, array, "", &mut ops)?;
	Ok(Prepared::List { param: param as u32, ops })
}

fn prepare(doc: &Doc, name: &str, raw: &str) -> Result<Prepared, String> {
	let param = (0..doc.parm_name.len())
		.position(|p| kslir::str_at(doc, doc.parm_name[p]) == name)
		.ok_or_else(|| "no such document param".to_string())?;
	let kind = doc.parm_type[param];
	if kind == PARAM_LIST {
		return prepare_list(doc, param, raw);
	}
	let value = coerce_scalar(kind, raw)?;
	if kind == 5 && !scalar_enum_contains(doc, param, &value.sym) {
		return Err(format!("unknown enum member '{}'", value.sym));
	}
	Ok(Prepared::Scalar { param: param as u32, value })
}

/// Apply a batch of raw host overrides. The complete batch is validated before
/// the first mutation, so malformed list entries cannot partially replace the
/// previous value.
pub fn apply_sets(inst: &mut Instance, sets: &[(String, String)]) -> Result<(), String> {
	let prepared = sets
		.iter()
		.map(|(name, raw)| prepare(inst.doc(), name, raw).map_err(|e| format!("param '{name}': {e}")))
		.collect::<Result<Vec<_>, _>>()?;

	for value in prepared {
		match value {
			Prepared::Scalar { param, value } => {
				if !kframe::inst_set_param(inst, param, &value) {
					return Err("validated scalar input was rejected by the kernel".to_string());
				}
			},
			Prepared::List { param, ops } => {
				for op in &ops {
					let applied = match op {
						ListOp::Len { path, n } => kframe::inst_set_list_len(inst, param, path, *n),
						ListOp::Key { path, index, key } => {
							kframe::inst_set_list_key(inst, param, path, *index, key)
						},
						ListOp::Field { path, index, field, value } => {
							kframe::inst_set_list_field(inst, param, path, *index, field, value)
						},
					};
					if !applied {
						return Err("validated list input was rejected by the kernel".to_string());
					}
				}
			},
		}
	}
	Ok(())
}

fn raw_json_value(value: &Value) -> Result<String, String> {
	match value {
		Value::String(s) => Ok(s.clone()),
		other => serde_json::to_string(other).map_err(|e| e.to_string()),
	}
}

/// Normalize wasm/conformance JSON options into the same raw `(name, value)`
/// inputs consumed by CLI and TUI. Accepts either `{name: value}` or the
/// historical `[[name, value], ...]` shape.
pub fn sets_from_json(value: &Value) -> Result<Vec<(String, String)>, String> {
	match value {
		Value::Null => Ok(Vec::new()),
		Value::Object(object) => object
			.iter()
			.map(|(name, value)| Ok((name.clone(), raw_json_value(value)?)))
			.collect(),
		Value::Array(entries) => entries
			.iter()
			.enumerate()
			.map(|(index, entry)| {
				let pair = entry
					.as_array()
					.ok_or_else(|| format!("sets entry {index} must be [name, value]"))?;
				if pair.len() != 2 {
					return Err(format!("sets entry {index} must contain exactly two values"));
				}
				let name = pair[0]
					.as_str()
					.ok_or_else(|| format!("sets entry {index} name must be a string"))?;
				Ok((name.to_string(), raw_json_value(&pair[1])?))
			})
			.collect(),
		_ => Err("sets must be an object or an array of [name, value] pairs".to_string()),
	}
}
