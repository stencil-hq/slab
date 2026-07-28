//! `slab gen wc` support for standalone exported definitions.
//!
//! It compiles one `export`ed def as a standalone SLIR document, with its
//! props promoted to params (SPEC §13: exported defs replace 0.5's stringly
//! `children()` injection for dynamic lists).
//!
//! Prop-type inference walks the def BODY's direct use sites:
//! - text content of a `text`/`span`/`para` builtin, or a Text-valued attribute
//!   (`act`, `field`, `src`, accessibility labels/relationships, and
//!   live-region string states) → `text`
//! - `checked` accepts either text or bool: an explicit Boolean default or
//!   another Boolean use site selects `bool`; otherwise it remains `text`
//! - a color slot (`bg`, `stroke`, `color`, `mask`, `backdrop-mask`) → `color`
//! - a numeric slot (`w h min-* max-* size weight gap radius stroke-w opacity
//!   tracking leading blur rotate span cols scale smooth value-* level
//!   pos-in-set set-size`, incl. tuple members of `pad offset at stroke-dash
//!   scale grain tilt`) → `num`
//! - a Boolean semantic slot (`expanded`, `selected`, `modal`, `live-atomic`)
//!   or a `when` truthiness condition → `bool`
//! - conflicting votes, no votes, or use sites of any other shape → `text`.
//!
//! Args forwarded to nested def calls cast no vote (only direct builtin use
//! sites are inspected). The declared def default is kept as the param
//! default when its literal shape matches the inferred type; otherwise the
//! type's zero value ("" / 0 / #ffffff / false) is used.

use slab_slir::Slir;
use slab_syntax::{
	ast::{ADef, ANode, Cond, Document, Item, ParamDecl, ParamDefault, ParamType, Value},
	diag::Diagnostics,
};

use crate::{Options, import::Unit};

/// One promoted prop of an exported def.
#[derive(Debug, Clone)]
pub struct ExportProp {
	pub name: String,
	pub ty:   ParamType,
}

/// Names of `export`-flagged defs, document order, later shadowers deduped.
pub fn exported_def_names(units: &[Unit]) -> Vec<String> {
	let mut names = Vec::new();
	for unit in units {
		for definition in &unit.doc.defs {
			names.retain(|name| name != &definition.name);
			if definition.export {
				names.push(definition.name.clone());
			}
		}
	}
	names
}

const COLOR_ATTRS: [&str; 5] = ["bg", "stroke", "color", "mask", "backdrop-mask"];
const TEXT_ATTRS: [&str; 12] = [
	"act",
	"field",
	"src",
	"d",
	"family",
	"label",
	"desc",
	"attach",
	"active-descendant",
	"controls",
	"value-text",
	"live",
];
const TEXT_OR_BOOL_ATTRS: [&str; 1] = ["checked"];
const BOOL_ATTRS: [&str; 7] =
	["expanded", "selected", "modal", "live-atomic", "strike", "italic", "underline"];
const NUM_ATTRS: [&str; 25] = [
	"w",
	"h",
	"min-w",
	"max-w",
	"min-h",
	"max-h",
	"size",
	"weight",
	"gap",
	"radius",
	"stroke-w",
	"opacity",
	"tracking",
	"leading",
	"blur",
	"rotate",
	"span",
	"scale",
	"smooth",
	"value-now",
	"value-min",
	"value-max",
	"level",
	"pos-in-set",
	"set-size",
];
const NUM_TUPLE_ATTRS: [&str; 8] =
	["pad", "offset", "at", "cols", "stroke-dash", "scale", "grain", "tilt"];

struct Vote {
	name:         String,
	ty:           Option<ParamType>,
	conflict:     bool,
	/// A use site whose accepted domain is exactly Text or Bool.
	text_or_bool: bool,
}

fn cast(votes: &mut [Vote], prop: &str, ty: ParamType) {
	let Some(v) = votes.iter_mut().find(|v| v.name == prop) else {
		return;
	};
	match &v.ty {
		None => v.ty = Some(ty),
		Some(t) if t == &ty => {},
		Some(_) => v.conflict = true,
	}
}

fn cast_text_or_bool(votes: &mut [Vote], prop: &str) {
	if let Some(vote) = votes.iter_mut().find(|vote| vote.name == prop) {
		vote.text_or_bool = true;
	}
}

fn attr_vote(votes: &mut [Vote], key: &str, value: &Value) {
	match value {
		Value::Kw(k) => {
			if COLOR_ATTRS.contains(&key) {
				cast(votes, k, ParamType::Color);
			} else if TEXT_ATTRS.contains(&key) {
				cast(votes, k, ParamType::Text);
			} else if TEXT_OR_BOOL_ATTRS.contains(&key) {
				cast_text_or_bool(votes, k);
			} else if BOOL_ATTRS.contains(&key) {
				cast(votes, k, ParamType::Bool);
			} else if NUM_ATTRS.contains(&key) || NUM_TUPLE_ATTRS.contains(&key) {
				cast(votes, k, ParamType::Num);
			}
		},
		Value::Tup(items) if NUM_TUPLE_ATTRS.contains(&key) => {
			for item in items {
				if let Value::Kw(k) = item {
					cast(votes, k, ParamType::Num);
				}
			}
		},
		Value::KeyMap(entries) if key == "keys" => {
			for (map_key, signal) in entries {
				if let Value::Kw(prop) = map_key {
					cast(votes, prop, ParamType::Text);
				}
				if let Value::Kw(prop) = signal {
					cast(votes, prop, ParamType::Text);
				}
			}
		},
		_ => {},
	}
}

fn walk(votes: &mut [Vote], items: &[Item]) {
	for item in items {
		match item {
			Item::Node(n) => {
				if matches!(n.name.as_str(), "text" | "span" | "para") {
					for arg in &n.args {
						if let Value::Kw(k) = arg {
							cast(votes, k, ParamType::Text);
						}
					}
				}
				for (key, v) in &n.attrs {
					attr_vote(votes, key, v);
				}
				walk(votes, &n.children);
			},
			Item::When(w) => {
				if let Cond::Ident { name, .. } = &w.cond {
					cast(votes, name, ParamType::Bool);
				}
				for (key, v) in &w.attrs {
					attr_vote(votes, key, v);
				}
				walk(votes, &w.children);
			},
			Item::Text(..) => {},
			Item::Each(_) => {},
		}
	}
}

/// Infer promoted param types for every prop of `def` (see module docs).
pub fn infer_props(def: &ADef) -> Vec<ExportProp> {
	let mut votes: Vec<Vote> = def
		.params
		.iter()
		.map(|(name, _)| Vote {
			name:         name.clone(),
			ty:           None,
			conflict:     false,
			text_or_bool: false,
		})
		.collect();
	for (name, default) in &def.params {
		match default {
			Some(Value::ListSchema(schema)) => {
				cast(&mut votes, name, ParamType::List(schema.clone()));
			},
			Some(Value::Kw(value)) if value == "true" || value == "false" => {
				cast(&mut votes, name, ParamType::Bool);
			},
			_ => {},
		}
	}
	walk(&mut votes, &def.body);
	for (name, default) in &def.params {
		let text_or_bool = votes
			.iter()
			.find(|vote| vote.name == *name)
			.is_some_and(|vote| vote.text_or_bool);
		let named_text_default = match default {
			Some(Value::Str(_)) => true,
			Some(Value::Kw(value)) => value != "true" && value != "false",
			_ => false,
		};
		if text_or_bool && named_text_default {
			cast(&mut votes, name, ParamType::Text);
		}
	}
	for vote in &mut votes {
		if vote.text_or_bool
			&& vote
				.ty
				.as_ref()
				.is_some_and(|ty| !matches!(ty, ParamType::Text | ParamType::Bool))
		{
			vote.conflict = true;
		}
	}
	votes
		.into_iter()
		.map(|vote| ExportProp {
			name: vote.name,
			ty:   if vote.conflict {
				ParamType::Text
			} else {
				vote.ty.unwrap_or(ParamType::Text)
			},
		})
		.collect()
}

fn default_for(ty: &ParamType, declared: Option<&Value>) -> Value {
	if let Some(v) = declared {
		if let (ParamType::Text, Value::Kw(value)) = (ty, v) {
			return Value::Str(value.clone());
		}
		let fits = matches!(
			(ty, v),
			(ParamType::Text, Value::Str(_))
				| (ParamType::Num, Value::Num(_))
				| (ParamType::Pct, Value::Pct(_))
				| (ParamType::Color, Value::Color(_))
				| (ParamType::List(_), Value::ListSchema(_))
		) || matches!((ty, v), (ParamType::Bool, Value::Kw(k)) if k == "true" || k == "false");
		if fits {
			return v.clone();
		}
	}
	match ty {
		ParamType::Text | ParamType::Enum => Value::Str(String::new()),
		ParamType::List(schema) => Value::ListSchema(schema.clone()),
		ParamType::Num => Value::Num(0.0),
		ParamType::Pct => Value::Pct(0.0),
		ParamType::Color => Value::Color("#ffffff".into()),
		ParamType::Bool => Value::Kw("false".into()),
	}
}

/// Compile an exported definition from a loaded source closure into standalone
/// SLIR. Tokens, definitions, animations, and document params remain available.
pub fn compile_export(
	units: &[Unit],
	def_name: &str,
	opts: &Options,
) -> (Option<Slir>, Diagnostics, Vec<ExportProp>) {
	let mut diags = Diagnostics::new();
	let Some((definition_unit, def)) = units.iter().enumerate().rev().find_map(|(unit, source)| {
		source
			.doc
			.defs
			.iter()
			.rev()
			.find(|definition| definition.name == def_name)
			.map(|definition| (unit, definition))
	}) else {
		diags.error("ref", format!("no exported def '{def_name}'"), 0);
		return (None, diags, Vec::new());
	};
	if !def.export {
		diags.error("ref", format!("no exported def '{def_name}'"), 0);
		return (None, diags, Vec::new());
	}
	let props = infer_props(def);

	let params = props
		.iter()
		.map(|prop| {
			let declared = def
				.params
				.iter()
				.find(|(name, _)| *name == prop.name)
				.and_then(|(_, default)| default.as_ref());
			ParamDecl {
				name:      prop.name.clone(),
				ty:        prop.ty.clone(),
				enum_syms: Vec::new(),
				default:   if matches!(prop.ty, ParamType::List(_)) {
					ParamDefault::List(Vec::new())
				} else {
					ParamDefault::Scalar(default_for(&prop.ty, declared))
				},
				line:      def.line,
				prop_of:   Some(def.name.clone()),
			}
		})
		.collect::<Vec<_>>();
	let root = ANode {
		name:     def.name.clone(),
		id:       None,
		args:     props
			.iter()
			.map(|prop| Value::Ref(vec!["param".into(), prop.name.clone()]))
			.collect(),
		attrs:    Vec::new(),
		flags:    Vec::new(),
		children: Vec::new(),
		line:     def.line,
	};
	let synthetic = Unit {
		file: units[definition_unit].file.clone(),
		abs:  None,
		doc:  Document {
			imports: Vec::new(),
			tokens: Default::default(),
			defs: Vec::new(),
			params,
			icons: Vec::new(),
			roots: vec![root],
			topwhens: Vec::new(),
			anims: Vec::new(),
		},
	};

	let mut export_units = Vec::with_capacity(units.len() + 1);
	export_units.push(synthetic);
	for unit in units {
		let mut unit = unit.clone();
		unit.doc.imports.clear();
		unit.doc.roots.clear();
		unit
			.doc
			.params
			.retain(|declaration| !props.iter().any(|prop| prop.name == declaration.name));
		export_units.push(unit);
	}

	let slir = crate::compile_units(&export_units, opts, &mut diags);
	(slir, diags, props)
}
