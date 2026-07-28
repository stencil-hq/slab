//! Typed host-input coercion shared by CLI, TUI, wasm, and static rendering.

use serde_json::Value;
use slab_kernel::{
	frame::{self as kframe, Instance, ParamValue},
	slir::{self as kslir, Doc},
};

const PARAM_LIST: u32 = 6;

#[derive(Debug)]
struct ListItem {
	key:    String,
	fields: Vec<(String, ParamValue)>,
}

#[derive(Debug)]
enum Prepared {
	Scalar { param: u32, value: ParamValue },
	List { param: u32, len: i32, items: Vec<ListItem> },
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
		_ => return Err(format!("unsupported field type {kind}")),
	}
	Ok(out)
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
	let field_off = doc.list_field_off[list] as usize;
	let field_len = doc.list_field_len[list] as usize;
	let mut items = Vec::with_capacity(array.len());
	let mut keys = Vec::with_capacity(array.len());

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

		let mut fields = Vec::with_capacity(field_len);
		for field in field_off..field_off + field_len {
			let name = kslir::str_at(doc, doc.list_field_name[field]);
			let value = match object.get(name) {
				Some(raw_value) => json_field_value(doc, field, raw_value)
					.map_err(|e| format!("item {index} field '{name}': {e}"))?,
				None => default_field_value(doc, field),
			};
			fields.push((name.to_owned(), value));
		}
		items.push(ListItem { key, fields });
	}

	let len = i32::try_from(items.len()).map_err(|_| "list has too many items".to_string())?;
	Ok(Prepared::List { param: param as u32, len, items })
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
		.map(|(name, raw)| prepare(&inst.doc, name, raw).map_err(|e| format!("param '{name}': {e}")))
		.collect::<Result<Vec<_>, _>>()?;

	for value in prepared {
		match value {
			Prepared::Scalar { param, value } => {
				if !kframe::inst_set_param(inst, param, &value) {
					return Err("validated scalar input was rejected by the kernel".to_string());
				}
			},
			Prepared::List { param, len, items } => {
				if !kframe::inst_set_list_len(inst, param, "", len) {
					return Err("validated list length was rejected by the kernel".to_string());
				}
				for (index, item) in items.iter().enumerate() {
					let index = index as i32;
					if !kframe::inst_set_list_key(inst, param, "", index, &item.key) {
						return Err("validated list key was rejected by the kernel".to_string());
					}
					for (field, field_value) in &item.fields {
						if !kframe::inst_set_list_field(inst, param, "", index, field, field_value) {
							return Err("validated list field was rejected by the kernel".to_string());
						}
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
