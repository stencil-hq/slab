//! `slab gen wc` support: compile one `export`ed def as a standalone SLIR
//! document, its props promoted to params (SPEC §13: exported defs replace
//! 0.5's stringly `children()` injection for dynamic lists).
//!
//! Prop-type inference walks the def BODY's direct use sites:
//! - text content of a `text`/`span`/`para` builtin, or a Text-valued
//!   attribute (`act`, `field`, `src`, accessibility labels/relationships,
//!   and live-region string states) → `text`
//! - `checked` accepts either text or bool: an explicit Boolean default or
//!   another Boolean use site selects `bool`; otherwise it remains `text`
//! - a color slot (`bg`, `stroke`, `color`, `mask`, `backdrop-mask`) → `color`
//! - a numeric slot (`w h min-* max-* size weight gap radius stroke-w
//!   opacity tracking leading blur rotate span cols scale smooth value-*
//!   level pos-in-set set-size`, incl. tuple members of
//!   `pad offset at stroke-dash scale grain tilt`) → `num`
//! - a Boolean semantic slot (`expanded`, `selected`, `modal`, `live-atomic`)
//!   or a `when` truthiness condition → `bool`
//! - conflicting votes, no votes, or use sites of any other shape → `text`.
//!
//! Args forwarded to nested def calls cast no vote (only direct builtin use
//! sites are inspected). The declared def default is kept as the param
//! default when its literal shape matches the inferred type; otherwise the
//! type's zero value ("" / 0 / #ffffff / false) is used.

use crate::Options;
use slab_slir::Slir;
use slab_syntax::ast::{
    ADef, ANode, Cond, Document, Item, ParamDecl, ParamDefault, ParamType, Value,
};
use slab_syntax::diag::Diagnostics;

/// One promoted prop of an exported def.
#[derive(Debug, Clone)]
pub struct ExportProp {
    pub name: String,
    pub ty: ParamType,
}

/// Names of `export`-flagged defs, document order, later shadowers deduped.
pub fn exported_def_names(src: &str) -> Vec<String> {
    let mut diags = Diagnostics::new();
    let doc = slab_syntax::parse(src, &mut diags);
    let mut names: Vec<String> = Vec::new();
    for def in &doc.defs {
        if def.export && !names.iter().any(|n| n == &def.name) {
            names.push(def.name.clone());
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
const BOOL_ATTRS: [&str; 5] = ["expanded", "selected", "modal", "live-atomic", "strike"];
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
const NUM_TUPLE_ATTRS: [&str; 8] = [
    "pad",
    "offset",
    "at",
    "cols",
    "stroke-dash",
    "scale",
    "grain",
    "tilt",
];

struct Vote {
    name: String,
    ty: Option<ParamType>,
    conflict: bool,
    /// A use site whose accepted domain is exactly Text or Bool.
    text_or_bool: bool,
}

fn cast(votes: &mut [Vote], prop: &str, ty: ParamType) {
    let Some(v) = votes.iter_mut().find(|v| v.name == prop) else {
        return;
    };
    match &v.ty {
        None => v.ty = Some(ty),
        Some(t) if t == &ty => {}
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
        }
        Value::Tup(items) if NUM_TUPLE_ATTRS.contains(&key) => {
            for item in items {
                if let Value::Kw(k) = item {
                    cast(votes, k, ParamType::Num);
                }
            }
        }
        Value::KeyMap(entries) if key == "keys" => {
            for (map_key, signal) in entries {
                if let Value::Kw(prop) = map_key {
                    cast(votes, prop, ParamType::Text);
                }
                if let Value::Kw(prop) = signal {
                    cast(votes, prop, ParamType::Text);
                }
            }
        }
        _ => {}
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
            }
            Item::When(w) => {
                if let Cond::Ident { name, .. } = &w.cond {
                    cast(votes, name, ParamType::Bool);
                }
                for (key, v) in &w.attrs {
                    attr_vote(votes, key, v);
                }
                walk(votes, &w.children);
            }
            Item::Text(..) => {}
            Item::Each(_) => {}
        }
    }
}

/// Infer promoted param types for every prop of `def` (see module docs).
pub fn infer_props(def: &ADef) -> Vec<ExportProp> {
    let mut votes: Vec<Vote> = def
        .params
        .iter()
        .map(|(name, _)| Vote {
            name: name.clone(),
            ty: None,
            conflict: false,
            text_or_bool: false,
        })
        .collect();
    for (name, default) in &def.params {
        match default {
            Some(Value::ListSchema(schema)) => {
                cast(&mut votes, name, ParamType::List(schema.clone()));
            }
            Some(Value::Kw(value)) if value == "true" || value == "false" => {
                cast(&mut votes, name, ParamType::Bool);
            }
            _ => {}
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
            ty: if vote.conflict {
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

/// Compile `def_name` (which must be `export`-flagged in `src`) into its own
/// SLIR document: tokens/defs/anims carry over, the def's props become
/// params, and the root is a single call passing `param.<prop>` for each.
pub fn compile_export(
    src: &str,
    def_name: &str,
    opts: &Options,
) -> (Option<Slir>, Diagnostics, Vec<ExportProp>) {
    let mut diags = Diagnostics::new();
    let doc = slab_syntax::parse(src, &mut diags);
    if diags.has_errors() {
        return (None, diags, Vec::new());
    }
    let Some(def) = doc.def(def_name).filter(|d| d.export) else {
        diags.error("ref", format!("no exported def '{def_name}'"), 0);
        return (None, diags, Vec::new());
    };
    let props = infer_props(def);

    let mut params: Vec<ParamDecl> = props
        .iter()
        .map(|p| {
            let declared = def
                .params
                .iter()
                .find(|(n, _)| *n == p.name)
                .and_then(|(_, d)| d.as_ref());
            ParamDecl {
                name: p.name.clone(),
                ty: p.ty.clone(),
                enum_syms: Vec::new(),
                default: if matches!(p.ty, ParamType::List(_)) {
                    ParamDefault::List(Vec::new())
                } else {
                    ParamDefault::Scalar(default_for(&p.ty, declared))
                },
                line: def.line,
            }
        })
        .collect();
    // Original doc params stay reachable (a def body may use `param.x`);
    // promoted props shadow same-named ones.
    for decl in &doc.params {
        if !params.iter().any(|p| p.name == decl.name) {
            params.push(decl.clone());
        }
    }

    let root = ANode {
        name: def.name.clone(),
        id: None,
        args: props
            .iter()
            .map(|p| Value::Ref(vec!["param".into(), p.name.clone()]))
            .collect(),
        attrs: Vec::new(),
        flags: Vec::new(),
        children: Vec::new(),
        line: def.line,
    };
    let sdoc = Document {
        tokens: doc.tokens.clone(),
        defs: doc.defs.clone(),
        params,
        icons: doc.icons.clone(),
        roots: vec![root],
        topwhens: doc.topwhens.clone(),
        anims: doc.anims.clone(),
    };

    let expanded = crate::expand::expand(&sdoc, &mut diags);
    if diags.has_errors() {
        return (None, diags, props);
    }
    let slir = crate::emit::emit(&expanded, opts, &mut diags);
    if diags.has_errors() {
        return (None, diags, props);
    }
    (Some(slir), diags, props)
}
