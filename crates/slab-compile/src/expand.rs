//! Resolution + expansion: scalar tokens retain active-theme identity, defs
//! expand (recursion cap 32, prop substitution, prop-truthiness `when`
//! folding, slot splice, multi-node splice), §15.1 keys are assigned, and
//! runtime `when` conditions become patch specs with detached children.
//! Layout/environment evaluation remains kernel territory.

use crate::color::{self, Paint, Rgba};
use slab_fonts;
use slab_kernel::pathdata;
use slab_slir::{attrs as at, cond as ck, flags as fl, kind as nk};
use slab_syntax::ast::*;
use slab_syntax::diag::Diagnostics;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

pub const MAX_DEPTH: usize = 32;

const RENDERER_CLASSES: [&str; 5] = ["web", "gpu", "tui", "svg", "png"];
const ENV_IDENTS: [&str; 4] = ["portrait", "landscape", "dark", "coarse"];
const BUILTINS: [&str; 19] = [
    "box", "row", "col", "wrap", "grid", "stack", "canvas", "para", "group", "text", "span",
    "rect", "img", "path", "icon", "divider", "spacer", "slot", "hole",
];
const ALIGNS: [&str; 5] = ["start", "center", "end", "baseline", "stretch"];
const STACK_ALIGNS: [&str; 9] = [
    "top-start",
    "top",
    "top-end",
    "start",
    "center",
    "end",
    "bottom-start",
    "bottom",
    "bottom-end",
];
const GRAVITIES: [&str; 12] = [
    "below-start",
    "below-center",
    "below-end",
    "above-start",
    "above-center",
    "above-end",
    "left-start",
    "left-center",
    "left-end",
    "right-start",
    "right-center",
    "right-end",
];
const PACKS: [&str; 4] = ["start", "center", "end", "between"];
const NAMED_ACTIVATION_KEYS: [&str; 15] = [
    "Enter",
    "Space",
    "Escape",
    "Tab",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
];
const EASING_NAMES: [(&str, u8); 5] = [
    ("linear", 0),
    ("ease-in", 1),
    ("ease-out", 2),
    ("ease-in-out", 3),
    ("ease", 3),
];
/// Shadow preset entry: `(name, (x, y, blur, color))`.
type ShadowPreset = (&'static str, (f64, f64, f64, &'static str));
const SHADOW_PRESETS: [ShadowPreset; 3] = [
    ("sm", (0.0, 1.0, 2.0, "#0003")),
    ("md", (0.0, 2.0, 6.0, "#0004")),
    ("lg", (0.0, 8.0, 24.0, "#0005")),
];

// ------------------------------------------------------------------ values

/// Resolved scalar value.
#[derive(Debug, Clone, PartialEq)]
pub enum RVal {
    None,
    Num(f64),
    Pct(f64),
    Str(String),
    Color(String),
    Fill(f64),
    Kw(String),
    Tup(Vec<RVal>),
    KeyMap(Vec<(RVal, RVal)>),
    Group(TokenTree),
    /// A scalar/group token reference whose authored-base value is retained
    /// alongside its path until attribute typing can build active-theme values.
    Token {
        path: Vec<String>,
        base: Box<RVal>,
    },
    /// PARM index.
    Param(u32),
    /// Field index in the current `each` template schema.
    Prop(u32),
}

pub type Scope = HashMap<String, RVal>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeSpec {
    Fixed(f64),
    Hug,
    Fill(f64),
    Pct(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shad {
    pub x: f64,
    pub y: f64,
    pub blur: f64,
    pub rgba: Rgba,
    pub inset: bool,
}

/// Typed, SLIR-ready attribute value.
#[derive(Debug, Clone, PartialEq)]
pub enum TVal {
    Num(f64),
    /// Generic percentage token value (distinct from an attribute-typed size).
    Pct(f64),
    Size(SizeSpec),
    Tuple(Vec<f64>),
    /// Tuple with at least one num/pct param-ref member (SLIR `TupleDyn`).
    TupleDyn(Vec<TupMember>),
    Enum(String),
    Str(String),
    Paint(Paint),
    /// Icon-declaration paint inherited from the icon usage's text color.
    PaintCurrent,
    Color(Rgba),
    /// One typed token use, resolved by the kernel against the active theme.
    Token {
        path: String,
        base: Box<TVal>,
        themes: Vec<(String, TVal)>,
    },
    Shadows(Vec<Shad>),
    Path(Vec<u8>, Vec<f64>),
    Param(u32),
    Prop(u32),
    /// A recursively normalized list-default run.
    List(Vec<ListItemInfo>),
}

/// One member of a dynamic tuple: a literal number or a num/pct param ref.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TupMember {
    Lit(f64),
    Param(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttrE {
    pub id: u16,
    pub val: TVal,
}

/// Deferred `when` condition.
#[derive(Debug, Clone, PartialEq)]
pub enum CondSpec {
    State(String, bool),
    Env(String, bool),
    Client(String, bool),
    Theme(String),
    Prop(u32, bool),
    WCmp(u8, f64),
    HCmp(u8, f64),
}

impl CondSpec {
    pub fn encode(&self) -> (u8, u8, u8, f64, &str) {
        match self {
            CondSpec::State(s, n) => (ck::STATE, *n as u8, 0, 0.0, s),
            CondSpec::Env(s, n) => (ck::ENV, *n as u8, 0, 0.0, s),
            CondSpec::Client(s, n) => (ck::CLIENT, *n as u8, 0, 0.0, s),
            CondSpec::Theme(s) => (ck::THEME, 0, 0, 0.0, s),
            CondSpec::Prop(_, n) => (ck::PROP, *n as u8, 0, 0.0, ""),
            CondSpec::WCmp(op, num) => (ck::WCMP, 0, *op, *num, ""),
            CondSpec::HCmp(op, num) => (ck::HCMP, 0, *op, *num, ""),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnimBind {
    pub name: String,
    pub dur: f64,
    /// 0 loop | 1 once | 2 alternate
    pub mode: u8,
    pub easing: u8,
    pub delay: f64,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct CPatch {
    pub cond: CondSpec,
    pub attrs: Vec<AttrE>,
    pub flag_mask: u16,
    pub children: Vec<CNode>,
    pub line: u32,
}

/// Fully expanded node, ready for SLIR emission.
#[derive(Debug, Clone)]
pub struct CNode {
    pub kind: u8,
    pub line: u32,
    pub id: Option<String>,
    pub key: String,
    pub flags: u16,
    pub attrs: Vec<AttrE>,
    pub content: Option<TVal>,
    pub children: Vec<CNode>,
    pub patches: Vec<CPatch>,
    pub animate: Option<AnimBind>,
    /// Animation bindings declared by deferred conditions on this node.
    pub conditional_animations: Vec<AnimBind>,
    pub transition: Option<(f64, u8, f64)>,
    pub act: Option<String>,
    pub field: Option<String>,
    pub submit: Option<String>,
    /// Pointer-down signal on the primary button.
    pub press: Option<String>,
    /// Secondary-button pointer-down signal.
    pub context: Option<String>,
    /// Double-click pointer-down signal.
    pub dblclick: Option<String>,
    /// Drag-start signal.
    pub drag: Option<String>,
    /// Drop-target signal.
    pub drop: Option<String>,
    /// Divider resize signal.
    pub resize: Option<String>,
    /// Continuous pointer-move signal.
    pub pointer_move: Option<String>,
    /// Primary pointer-up signal.
    pub pointer_up: Option<String>,
    /// Active-drag movement signal.
    pub drag_update: Option<String>,
    /// Drag termination signal.
    pub drag_end: Option<String>,
    /// Signal bindings declared by deferred conditions on this node.
    pub conditional_signals: Vec<(String, u8)>,
    pub hole: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListFieldInfo {
    pub name: String,
    pub ty: ParamType,
    pub enum_syms: Vec<String>,
    pub default: TVal,
    /// Canonical nested schema row, or none for scalar fields.
    pub sub: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItemInfo {
    /// One normalized value per field, in schema order.
    pub values: Vec<TVal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListInfo {
    pub schema: String,
    pub fields: Vec<ListFieldInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    pub name: String,
    pub ty: ParamType,
    pub enum_syms: Vec<String>,
    pub default: TVal,
    /// Canonical list-schema row for a list parameter.
    pub list: Option<u32>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct RAnim {
    pub name: String,
    pub stops: Vec<(f64, Vec<AttrE>)>,
}
/// One validated named icon and its detached static path subtree.
#[derive(Debug, Clone)]
pub struct CIcon {
    /// Declared lookup name.
    pub name: String,
    /// Positive square design-box extent.
    pub viewbox: f64,
    /// Detached static group containing path children.
    pub root: CNode,
}

/// One public scalar-token row and its active-theme values.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenInfo {
    pub path: String,
    pub base: RVal,
    pub themes: Vec<(String, RVal)>,
}

/// Expansion output.
pub struct Expanded {
    pub roots: Vec<CNode>,
    pub params: Vec<ParamInfo>,
    pub anims: Vec<RAnim>,
    /// Canonical, cycle-tolerant list schema rows.
    pub list_schemas: Vec<ListInfo>,
    /// Validated icon declarations in source order.
    pub icons: Vec<CIcon>,
    /// Theme names accepted by the instance API, in source declaration order.
    pub themes: Vec<String>,
    /// Public scalar token rows in authored declaration order.
    pub tokens: Vec<TokenInfo>,
    /// (src, line), first-use order, deduped.
    pub images: Vec<(String, u32)>,
    /// Statically knowable family names used through list-item property bindings.
    pub font_families: BTreeSet<String>,
    /// Statically knowable snapped weights used through list-item property bindings.
    pub font_weights: BTreeSet<u16>,
}

// ---------------------------------------------------------------- context

#[derive(Default)]
struct Keys {
    counters: HashMap<String, u32>,
    seen: HashMap<String, u32>,
}

type KeysRc = Rc<RefCell<Keys>>;

struct SlotPayload {
    children: Vec<Item>,
    scope: Scope,
    key: String,
    keys: KeysRc,
}

struct Ctx<'a> {
    doc: &'a Document,
    diags: &'a mut Diagnostics,
    tokens: &'a TokenTree,
    /// Top-level `when` token override variants: (cond, merged tree).
    variants: Vec<(CondSpec, TokenTree)>,
    /// Fully merged token trees for named themes, in declaration order.
    theme_tokens: Vec<(String, TokenTree)>,
    params: Vec<ParamInfo>,
    anim_names: Vec<String>,
    anim_content: BTreeSet<String>,
    seen_ids: HashMap<String, u32>,
    holes: HashMap<String, u32>,
    signals: Vec<(String, u8, u32)>,
    field_sync_warnings: BTreeSet<(u32, String, String)>,
    images: Vec<(String, u32)>,
    /// Effective child-axis stack; empty means the root column context.
    layout_axes: Vec<bool>,
    /// Static fallbacks required by property-bound list-item font families.
    font_families: BTreeSet<String>,
    /// Static fallbacks required by property-bound list-item font weights.
    font_weights: BTreeSet<u16>,
    /// Non-zero while compiling a runtime `each` template.
    each_depth: u32,
    /// Field schemas for the active runtime `each` templates.
    prop_fields: Vec<Vec<ListFieldInfo>>,
    /// Schema rows whose canonical templates are currently being expanded.
    each_schemas: Vec<u32>,
    /// Canonical list schemas allocated before their fields are populated.
    list_schemas: Vec<ListInfo>,
    /// Non-zero while validating a top-level icon declaration.
    icon_depth: u32,
    /// While non-zero, diagnostics are suppressed (variant re-resolution).
    quiet: u32,
}

impl<'a> Ctx<'a> {
    fn error(&mut self, code: &'static str, msg: String, line: u32) {
        if self.quiet == 0 {
            self.diags.error(code, msg, line);
        }
    }
    fn warn(&mut self, code: &'static str, msg: String, line: u32) {
        if self.quiet == 0 {
            self.diags.warn(code, msg, line);
        }
    }
    fn warn_with(&mut self, code: &'static str, msg: String, line: u32, remedy: String) {
        if self.quiet == 0 {
            self.diags.warn_with(code, msg, line, remedy);
        }
    }
}

// ------------------------------------------------------------------ tokens

fn lookup<'t>(tree: &'t TokenTree, path: &[String]) -> Option<&'t TokenEntry> {
    let mut cur = tree.get(&path[0])?;
    for seg in &path[1..] {
        match cur {
            TokenEntry::Group(g) => cur = g.get(seg)?,
            TokenEntry::Value(_) => return None,
        }
    }
    Some(cur)
}

fn resolve_value(ctx: &mut Ctx, v: &Value, scope: &Scope, line: u32, tree: &TokenTree) -> RVal {
    resolve_value_d(ctx, v, scope, line, tree, 0, true)
}

fn resolve_token_value(ctx: &mut Ctx, path: &[String], line: u32, tree: &TokenTree) -> RVal {
    match lookup(tree, path) {
        Some(TokenEntry::Value(value)) => {
            let value = value.clone();
            resolve_value_d(ctx, &value, &Scope::new(), line, tree, 1, false)
        }
        Some(TokenEntry::Group(group)) => RVal::Group(group.clone()),
        None => RVal::None,
    }
}

fn resolve_value_d(
    ctx: &mut Ctx,
    v: &Value,
    scope: &Scope,
    line: u32,
    tree: &TokenTree,
    depth: usize,
    preserve_token: bool,
) -> RVal {
    if depth > MAX_DEPTH {
        ctx.error("ref", "token reference cycle".into(), line);
        return RVal::None;
    }
    match v {
        Value::Num(x) => RVal::Num(*x),
        Value::Pct(x) => RVal::Pct(*x),
        Value::Str(s) => RVal::Str(s.clone()),
        Value::Color(c) => RVal::Color(c.clone()),
        Value::Fill(wt) => RVal::Fill(*wt),
        Value::Ref(path) => {
            if path[0] == "param" {
                if path.len() == 2
                    && let Some(ix) = ctx.params.iter().position(|p| p.name == path[1])
                {
                    return RVal::Param(ix as u32);
                }
                ctx.error("ref", format!("unknown param '{}'", path.join(".")), line);
                return RVal::None;
            }
            match lookup(tree, path) {
                Some(TokenEntry::Value(got)) => {
                    let got = got.clone();
                    let base =
                        resolve_value_d(ctx, &got, &Scope::new(), line, tree, depth + 1, false);
                    if preserve_token {
                        RVal::Token {
                            path: path.clone(),
                            base: Box::new(base),
                        }
                    } else {
                        base
                    }
                }
                Some(TokenEntry::Group(g)) => {
                    let base = RVal::Group(g.clone());
                    if preserve_token {
                        RVal::Token {
                            path: path.clone(),
                            base: Box::new(base),
                        }
                    } else {
                        base
                    }
                }
                None => {
                    ctx.error("ref", format!("unknown token '{}'", path.join(".")), line);
                    RVal::None
                }
            }
        }
        Value::Kw(name) => {
            if let Some(v) = scope.get(name) {
                return v.clone();
            }
            RVal::Kw(name.clone())
        }
        Value::Tup(items) => RVal::Tup(
            items
                .iter()
                .map(|it| resolve_value_d(ctx, it, scope, line, tree, depth + 1, preserve_token))
                .collect(),
        ),
        Value::KeyMap(entries) => RVal::KeyMap(
            entries
                .iter()
                .map(|(key, signal)| {
                    (
                        resolve_value_d(ctx, key, scope, line, tree, depth + 1, preserve_token),
                        resolve_value_d(ctx, signal, scope, line, tree, depth + 1, preserve_token),
                    )
                })
                .collect(),
        ),
        Value::List(_) | Value::ListSchema(_) => {
            ctx.error(
                "ref",
                "list literals and list(...) are valid only in list schemas".into(),
                line,
            );
            RVal::None
        }
    }
}

fn truthy(v: &RVal) -> bool {
    match v {
        RVal::Token { base, .. } => truthy(base),
        RVal::None => false,
        RVal::Kw(k) => !matches!(k.as_str(), "false" | "none" | ""),
        RVal::Num(x) => *x != 0.0,
        RVal::Str(s) => !s.is_empty(),
        _ => true,
    }
}

fn token_base(rv: &RVal) -> &RVal {
    if let RVal::Token { base, .. } = rv {
        token_base(base)
    } else {
        rv
    }
}

fn first_token_path(rv: &RVal) -> Option<&[String]> {
    match rv {
        RVal::Token { path, .. } => Some(path),
        RVal::Tup(items) => items.iter().find_map(first_token_path),
        RVal::KeyMap(entries) => entries
            .iter()
            .find_map(|(key, value)| first_token_path(key).or_else(|| first_token_path(value))),
        _ => None,
    }
}

fn rval_without_tokens(rv: &RVal) -> RVal {
    match rv {
        RVal::Token { base, .. } => rval_without_tokens(base),
        RVal::Tup(items) => RVal::Tup(items.iter().map(rval_without_tokens).collect()),
        RVal::KeyMap(entries) => RVal::KeyMap(
            entries
                .iter()
                .map(|(key, value)| (rval_without_tokens(key), rval_without_tokens(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn rval_for_theme(ctx: &mut Ctx, rv: &RVal, line: u32, tree: &TokenTree) -> RVal {
    match rv {
        RVal::Token { path, .. } => resolve_token_value(ctx, path, line, tree),
        RVal::Tup(items) => RVal::Tup(
            items
                .iter()
                .map(|item| rval_for_theme(ctx, item, line, tree))
                .collect(),
        ),
        RVal::KeyMap(entries) => RVal::KeyMap(
            entries
                .iter()
                .map(|(key, value)| {
                    (
                        rval_for_theme(ctx, key, line, tree),
                        rval_for_theme(ctx, value, line, tree),
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}
fn to_text(ctx: &Ctx, rv: &RVal) -> String {
    match rv {
        RVal::Token { base, .. } => to_text(ctx, base),
        RVal::Num(x) => fmt_g(*x),
        RVal::Kw(k) => k.clone(),
        RVal::Pct(p) => format!("{}%", fmt_g(round_half_even(*p, 1))),
        RVal::Str(s) => s.clone(),
        RVal::Color(c) => c.clone(),
        RVal::None => String::new(),
        RVal::Prop(field) => {
            if let Some(fields) = ctx.prop_fields.last()
                && let Some(info) = fields.get(*field as usize)
            {
                return info.name.clone();
            }
            if let Some(param) = ctx.params.get(*field as usize) {
                return param.name.clone();
            }
            format!("{rv:?}")
        }
        RVal::Param(param) => {
            if let Some(p) = ctx.params.get(*param as usize) {
                return p.name.clone();
            }
            format!("{rv:?}")
        }
        other => format!("{other:?}"),
    }
}

fn token_text_tval(ctx: &mut Ctx, path: &[String], base: &RVal, line: u32) -> TVal {
    let base_text = to_text(ctx, base);
    let mut themes = Vec::with_capacity(ctx.theme_tokens.len());
    for (theme, tree) in ctx.theme_tokens.clone() {
        let value = resolve_token_value(ctx, path, line, &tree);
        let text = to_text(ctx, &value);
        themes.push((theme, TVal::Str(text)));
    }
    TVal::Token {
        path: path.join("."),
        base: Box::new(TVal::Str(base_text)),
        themes,
    }
}

fn round_half_even(x: f64, decimals: u32) -> f64 {
    let scale = 10f64.powi(decimals as i32);
    (x * scale).round_ties_even() / scale
}

/// Python `%g`-flavored number-to-text for prop substitution: integers
/// print bare, fractions trim trailing zeros (6 decimal places max).
pub fn fmt_g(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    if x == x.trunc() && x.abs() < 1e15 {
        return format!("{}", x as i64);
    }
    let formatted = format!("{x:.6}");
    let t = formatted.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() || t == "-" {
        "0".into()
    } else {
        t.to_string()
    }
}

// -------------------------------------------------------------- conditions

enum CondEval {
    Bool(bool),
    Defer(CondSpec),
}

fn eval_cond(ctx: &mut Ctx, cond: &Cond, scope: &Scope, line: u32) -> CondEval {
    match cond {
        Cond::Cmp { axis, op, num } => {
            let opb = match op {
                CmpOp::Lt => 0,
                CmpOp::Le => 1,
                CmpOp::Gt => 2,
                CmpOp::Ge => 3,
            };
            CondEval::Defer(match axis {
                CmpAxis::W => CondSpec::WCmp(opb, *num),
                CmpAxis::H => CondSpec::HCmp(opb, *num),
            })
        }
        Cond::Theme(name) => CondEval::Defer(CondSpec::Theme(name.clone())),
        Cond::Ident { name, neg } => {
            let neg = *neg;
            if let Some(v) = scope.get(name) {
                if let RVal::Param(ix) = v {
                    return param_cond(ctx, *ix as usize, neg, line);
                }
                if let RVal::Prop(field) = v {
                    return CondEval::Defer(CondSpec::Prop(*field, neg));
                }
                return CondEval::Bool(truthy(v) != neg);
            }
            if RENDERER_CLASSES.contains(&name.as_str()) {
                return CondEval::Defer(CondSpec::Client(name.clone(), neg));
            }
            if ENV_IDENTS.contains(&name.as_str()) {
                return CondEval::Defer(CondSpec::Env(name.clone(), neg));
            }
            if let Some(ix) = ctx.params.iter().position(|p| p.name == *name) {
                return param_cond(ctx, ix, neg, line);
            }
            CondEval::Defer(CondSpec::State(name.clone(), neg))
        }
    }
}

fn param_cond(ctx: &mut Ctx, ix: usize, neg: bool, line: u32) -> CondEval {
    let p = &ctx.params[ix];
    if p.ty == ParamType::Bool {
        CondEval::Defer(CondSpec::State(p.name.clone(), neg))
    } else {
        let msg = format!(
            "param '{}' is {}; only bool params can be used as `when` conditions",
            p.name,
            p.ty.as_str()
        );
        ctx.error("param-type", msg, line);
        CondEval::Bool(false)
    }
}

// ------------------------------------------------------------- attr sink

#[derive(Default, Debug, Clone)]
struct Sink {
    entries: Vec<AttrE>,
    content: Option<TVal>,
    animate: Option<AnimBind>,
    transition: Option<(f64, u8, f64)>,
    act: Option<String>,
    field: Option<String>,
    submit: Option<String>,
    press: Option<String>,
    context: Option<String>,
    dblclick: Option<String>,
    drag: Option<String>,
    drop: Option<String>,
    resize: Option<String>,
    pointer_move: Option<String>,
    pointer_up: Option<String>,
    drag_update: Option<String>,
    drag_end: Option<String>,
    key_signals: Vec<String>,
    key_map: bool,
    /// `field-sync=host|implicit`; absent inherits across a `when` patch.
    field_sync_host: Option<bool>,
    /// Extra flag bits set by attrs (`field`, `press`, and `drag` imply focusable).
    flag_mask: u16,
    /// True while applying deferred patch or keyframe attributes.
    patch_ctx: bool,
    /// True while applying animation keyframe attributes.
    keyframe_ctx: bool,
}

impl Sink {
    fn set(&mut self, id: u16, val: TVal) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.val = val;
        } else {
            self.entries.push(AttrE { id, val });
        }
    }

    fn get(&self, id: u16) -> Option<&TVal> {
        self.entries.iter().find(|e| e.id == id).map(|e| &e.val)
    }
}

fn size_spec(ctx: &mut Ctx, rv: &RVal, line: u32, what: &str) -> Option<TVal> {
    match rv {
        RVal::Num(x) => Some(TVal::Size(SizeSpec::Fixed(*x))),
        RVal::Pct(p) => Some(TVal::Size(SizeSpec::Pct(*p))),
        RVal::Fill(wt) => Some(TVal::Size(SizeSpec::Fill(*wt))),
        RVal::Kw(k) if k == "hug" => Some(TVal::Size(SizeSpec::Hug)),
        RVal::Param(ix) => {
            let p = &ctx.params[*ix as usize];
            if matches!(&p.ty, ParamType::Num | ParamType::Pct) {
                Some(TVal::Param(*ix))
            } else {
                let msg = format!(
                    "param '{}' ({}) cannot be used as a size for {what}",
                    p.name,
                    p.ty.as_str()
                );
                ctx.error("ref", msg, line);
                None
            }
        }
        RVal::Prop(field) => Some(TVal::Prop(*field)),
        _ => {
            ctx.error("ref", format!("invalid size for {what}: {rv:?}"), line);
            None
        }
    }
}

fn num_val(ctx: &mut Ctx, rv: &RVal, line: u32, what: &str) -> Option<f64> {
    match rv {
        RVal::Num(x) => Some(*x),
        _ => {
            ctx.error("ref", format!("{what} expects a number"), line);
            None
        }
    }
}

/// Color-ish string out of a resolved value, `Ok(None)` = explicit none.
fn color_str(ctx: &mut Ctx, rv: &RVal, line: u32, what: &str) -> Result<Option<String>, ()> {
    match rv {
        RVal::Str(s) | RVal::Color(s) => Ok(Some(s.clone())),
        RVal::Kw(k) if k == "none" || k == "transparent" => Ok(None),
        RVal::Kw(k) if k == "white" || k == "black" => Ok(Some(k.clone())),
        RVal::Kw(k) => {
            let msg =
                format!("unknown color '{k}' for {what} (token refs are dotted, e.g. color.{k})");
            ctx.error("ref", msg, line);
            Err(())
        }
        RVal::None => Ok(None),
        RVal::Fill(_) => {
            let msg = format!(
                "{what} expects a color; `fill` is the reserved sizing keyword \
                 (rename the prop/token, e.g. `tone` or `bg`)"
            );
            ctx.error("ref", msg, line);
            Err(())
        }
        _ => {
            ctx.error("ref", format!("{what} expects a color"), line);
            Err(())
        }
    }
}

fn one_shadow(ctx: &mut Ctx, items: &[RVal], line: u32) -> Option<Shad> {
    let mut items = items;
    let mut inset = false;
    if let Some(RVal::Kw(k)) = items.first()
        && k == "inset"
    {
        inset = true;
        items = &items[1..];
    }
    if items.len() == 4
        && let (RVal::Num(x), RVal::Num(y), RVal::Num(b)) = (&items[0], &items[1], &items[2])
        && let RVal::Str(c) | RVal::Color(c) = &items[3]
    {
        if let Some(rgba) = color::parse_rgba(c) {
            return Some(Shad {
                x: *x,
                y: *y,
                blur: *b,
                rgba,
                inset,
            });
        }
        ctx.warn("attr", format!("unparseable shadow color '{c}'"), line);
        return None;
    }
    ctx.error("ref", "shadow expects [inset,]x,y,blur,color".into(), line);
    None
}

fn preset_shadow(name: &str) -> Option<Shad> {
    SHADOW_PRESETS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, (x, y, b, c))| Shad {
            x,
            y,
            blur: b,
            rgba: color::parse_rgba(c).unwrap(),
            inset: false,
        })
}

fn parse_shadows(ctx: &mut Ctx, rv: &RVal, line: u32) -> Option<Vec<Shad>> {
    match rv {
        RVal::Kw(k) if k == "none" || k == "transparent" => Some(vec![]),
        RVal::Kw(k) => {
            if let Some(s) = preset_shadow(k) {
                Some(vec![s])
            } else {
                ctx.error("ref", format!("unknown shadow preset '{k}'"), line);
                None
            }
        }
        RVal::Tup(items) => {
            if items.iter().any(|i| matches!(i, RVal::Num(_))) {
                return one_shadow(ctx, items, line).map(|s| vec![s]);
            }
            let mut out = Vec::new();
            for item in items {
                match item {
                    RVal::Kw(k) if preset_shadow(k).is_some() => {
                        out.push(preset_shadow(k).unwrap());
                    }
                    RVal::Tup(inner) => out.push(one_shadow(ctx, inner, line)?),
                    _ => {
                        ctx.error(
                            "ref",
                            "shadow list items must be presets or x,y,blur,color tokens".into(),
                            line,
                        );
                        return None;
                    }
                }
            }
            Some(out)
        }
        _ => {
            ctx.error(
                "ref",
                "shadow expects a preset, x,y,blur,color, or a list".into(),
                line,
            );
            None
        }
    }
}

fn parse_animate(ctx: &mut Ctx, rv: &RVal, line: u32) -> Option<AnimBind> {
    let one;
    let items: &[RVal] = match rv {
        RVal::Tup(items) => items,
        other => {
            one = [other.clone()];
            &one
        }
    };
    let Some(RVal::Kw(name)) = items.first() else {
        ctx.error("ref", "animate expects name,duration,...".into(), line);
        return None;
    };
    let mut mode = 0u8;
    let mut easing = 0u8;
    let mut nums = Vec::new();
    for it in &items[1..] {
        match it {
            RVal::Num(x) => nums.push(*x),
            RVal::Kw(k) if k == "loop" => mode = 0,
            RVal::Kw(k) if k == "once" => mode = 1,
            RVal::Kw(k) if k == "alternate" => mode = 2,
            RVal::Kw(k) if EASING_NAMES.iter().any(|(n, _)| n == k) => {
                easing = EASING_NAMES.iter().find(|(n, _)| n == k).unwrap().1;
            }
            other => {
                ctx.error("ref", format!("animate: unexpected value {other:?}"), line);
                return None;
            }
        }
    }
    if nums.is_empty() {
        ctx.error(
            "ref",
            "animate needs a duration in ms (animate=pulse,1200)".into(),
            line,
        );
        return None;
    }
    if !ctx.anim_names.iter().any(|n| n == name) {
        ctx.error("ref", format!("unknown anim '{name}'"), line);
        return None;
    }
    Some(AnimBind {
        name: name.clone(),
        dur: nums[0],
        mode,
        easing,
        delay: nums.get(1).copied().unwrap_or(0.0),
        line,
    })
}

fn parse_transition(ctx: &mut Ctx, rv: &RVal, line: u32) -> Option<(f64, u8, f64)> {
    let one;
    let items: &[RVal] = match rv {
        RVal::Tup(items) => items,
        other => {
            one = [other.clone()];
            &one
        }
    };
    let mut easing = 2u8; // ease-out
    let mut nums = Vec::new();
    for it in items {
        match it {
            RVal::Num(x) => nums.push(*x),
            RVal::Kw(k) if EASING_NAMES.iter().any(|(n, _)| n == k) => {
                easing = EASING_NAMES.iter().find(|(n, _)| n == k).unwrap().1;
            }
            other => {
                ctx.error(
                    "ref",
                    format!("transition: unexpected value {other:?}"),
                    line,
                );
                return None;
            }
        }
    }
    if nums.is_empty() {
        ctx.error("ref", "transition needs a duration in ms".into(), line);
        return None;
    }
    Some((nums[0], easing, nums.get(1).copied().unwrap_or(0.0)))
}

/// Collects tuple members, accepting literals and num/pct param refs.
/// All-literal input stays a static `TVal::Tuple`; a mistyped param member
/// reports the standard param-fit error, any other member reports `msg`.
fn tuple_val(ctx: &mut Ctx, items: &[RVal], line: u32, what: &str, msg: &str) -> Option<TVal> {
    let mut members = Vec::with_capacity(items.len());
    for item in items {
        match item {
            RVal::Num(x) => members.push(TupMember::Lit(*x)),
            RVal::Param(ix) => {
                expect_param_ty(ctx, *ix, &[ParamType::Num, ParamType::Pct], line, what)?;
                members.push(TupMember::Param(*ix));
            }
            _ => {
                ctx.error("ref", msg.into(), line);
                return None;
            }
        }
    }
    Some(members_val(members))
}

/// Downgrades an all-literal member list to a static tuple.
fn members_val(members: Vec<TupMember>) -> TVal {
    if members.iter().all(|m| matches!(m, TupMember::Lit(_))) {
        TVal::Tuple(
            members
                .iter()
                .map(|m| match m {
                    TupMember::Lit(x) => *x,
                    TupMember::Param(_) => 0.0,
                })
                .collect(),
        )
    } else {
        TVal::TupleDyn(members)
    }
}

fn expect_param_ty(
    ctx: &mut Ctx,
    ix: u32,
    ok: &[ParamType],
    line: u32,
    what: &str,
) -> Option<TVal> {
    let p = &ctx.params[ix as usize];
    if ok.contains(&p.ty) {
        Some(TVal::Param(ix))
    } else {
        let msg = format!("param '{}' ({}) does not fit {what}", p.name, p.ty.as_str());
        ctx.error("ref", msg, line);
        None
    }
}

fn expect_prop_ty(
    ctx: &mut Ctx,
    field: u32,
    ok: &[ParamType],
    line: u32,
    what: &str,
) -> Option<TVal> {
    let info = ctx
        .prop_fields
        .last()
        .and_then(|fields| fields.get(field as usize))
        .cloned();
    let Some(info) = info else {
        ctx.error("ref", format!("template prop does not fit {what}"), line);
        return None;
    };
    if ok.contains(&info.ty) {
        Some(TVal::Prop(field))
    } else {
        let msg = format!(
            "template prop '{}' ({}) does not fit {what}",
            info.name,
            info.ty.as_str()
        );
        ctx.error("ref", msg, line);
        None
    }
}

fn expect_semantic_enum_param(
    ctx: &mut Ctx,
    ix: u32,
    allowed: &[&str],
    allow_bool: bool,
    line: u32,
    what: &str,
) -> Option<TVal> {
    let parameter = ctx.params[ix as usize].clone();
    let compatible = parameter.ty == ParamType::Text
        || (allow_bool && parameter.ty == ParamType::Bool)
        || (parameter.ty == ParamType::Enum
            && parameter
                .enum_syms
                .iter()
                .all(|symbol| allowed.contains(&symbol.as_str())));
    if compatible {
        Some(TVal::Param(ix))
    } else {
        ctx.error(
            "ref",
            format!(
                "param '{}' ({}) does not fit {what}",
                parameter.name,
                parameter.ty.as_str()
            ),
            line,
        );
        None
    }
}

fn expect_semantic_enum_prop(
    ctx: &mut Ctx,
    field: u32,
    allowed: &[&str],
    allow_bool: bool,
    line: u32,
    what: &str,
) -> Option<TVal> {
    let info = ctx
        .prop_fields
        .last()
        .and_then(|fields| fields.get(field as usize))
        .cloned();
    let Some(info) = info else {
        ctx.error("ref", format!("template prop does not fit {what}"), line);
        return None;
    };
    let compatible = info.ty == ParamType::Text
        || (allow_bool && info.ty == ParamType::Bool)
        || (info.ty == ParamType::Enum
            && info
                .enum_syms
                .iter()
                .all(|symbol| allowed.contains(&symbol.as_str())));
    if compatible {
        Some(TVal::Prop(field))
    } else {
        ctx.error(
            "ref",
            format!(
                "template prop '{}' ({}) does not fit {what}",
                info.name,
                info.ty.as_str()
            ),
            line,
        );
        None
    }
}

fn semantic_number_in_range(name: &str, value: f64) -> bool {
    if !value.is_finite() {
        return false;
    }
    match name {
        "level" | "pos-in-set" => value >= 1.0 && value.fract() == 0.0,
        "set-size" => (value == -1.0) || (value >= 1.0 && value.fract() == 0.0),
        _ => true,
    }
}

fn is_activation_key(name: &str) -> bool {
    if NAMED_ACTIVATION_KEYS.contains(&name) {
        return true;
    }
    if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<u8>().ok())
        && (1..=24).contains(&number)
    {
        return true;
    }
    let mut chars = name.chars();
    matches!((chars.next(), chars.next()), (Some(ch), None) if !ch.is_control() && ch != ',')
}

struct ActivationKeys {
    encoded: String,
    signals: Vec<String>,
    mapped: bool,
}

fn activation_name<'a>(ctx: &mut Ctx, value: &'a RVal, what: &str, line: u32) -> Option<&'a str> {
    match value {
        RVal::Kw(name) | RVal::Str(name) => Some(name),
        _ => {
            ctx.error("ref", format!("{what} expects a name"), line);
            None
        }
    }
}

fn activation_keys(ctx: &mut Ctx, rv: &RVal, line: u32) -> Option<ActivationKeys> {
    if let RVal::KeyMap(entries) = rv {
        let mut encoded = String::new();
        let mut keys = Vec::<String>::new();
        let mut signals = Vec::new();
        for (key, signal) in entries {
            let key = activation_name(ctx, key, "keys map key", line)?;
            let signal = activation_name(ctx, signal, "keys map signal", line)?;
            if !is_activation_key(key) {
                ctx.warn("attr", format!("unknown activation key '{key}'"), line);
            }
            if key.contains(',') {
                ctx.error(
                    "keys",
                    "mapped activation keys cannot contain `,`".into(),
                    line,
                );
                continue;
            }
            if signal.contains(',') || signal.contains(':') {
                ctx.error(
                    "keys",
                    "mapped signal names cannot contain `,` or `:`".into(),
                    line,
                );
                continue;
            }
            let canonical_key = if key == "Space" { " " } else { key };
            if keys.iter().any(|prior| prior == canonical_key) {
                ctx.error("keys", format!("duplicate key mapping for '{key}'"), line);
                continue;
            }
            keys.push(canonical_key.to_owned());
            if !encoded.is_empty() {
                encoded.push(',');
            }
            encoded.push_str(canonical_key);
            encoded.push(':');
            encoded.push_str(signal);
            if !signals.iter().any(|prior| prior == signal) {
                signals.push(signal.to_owned());
            }
        }
        return Some(ActivationKeys {
            encoded,
            signals,
            mapped: true,
        });
    }

    let items = match rv {
        RVal::Tup(items) => items.as_slice(),
        one => std::slice::from_ref(one),
    };
    let mut encoded = String::new();
    for item in items {
        let name = activation_name(ctx, item, "keys", line)?;
        if !is_activation_key(name) {
            ctx.warn("attr", format!("unknown activation key '{name}'"), line);
        }
        if !encoded.is_empty() {
            encoded.push(',');
        }
        encoded.push_str(if name == "Space" { " " } else { name });
    }
    Some(ActivationKeys {
        encoded,
        signals: Vec::new(),
        mapped: false,
    })
}

/// The typed `_apply_attr` port. Applies one resolved attribute value into
/// the sink, mirroring the 0.5 validation and diagnostics.
fn apply_attr_concrete(ctx: &mut Ctx, sink: &mut Sink, key: &str, rv: &RVal, line: u32) {
    if let RVal::Prop(field) = rv {
        match key {
            "d" => {
                if let Some(tv) = expect_prop_ty(ctx, *field, &[ParamType::Text], line, key) {
                    sink.set(at::D, tv);
                }
            }
            "src" => {
                if let Some(tv) = expect_prop_ty(ctx, *field, &[ParamType::Text], line, key) {
                    sink.set(at::SRC, tv);
                }
            }
            "label" | "desc" | "attach" | "active-descendant" | "controls" | "value-text" => {
                if let Some(tv) = expect_prop_ty(ctx, *field, &[ParamType::Text], line, key) {
                    sink.set(at::attr_id(key).expect("text attribute id is defined"), tv);
                }
            }
            "expanded" | "selected" | "modal" | "live-atomic" | "strike" => {
                if let Some(tv) = expect_prop_ty(ctx, *field, &[ParamType::Bool], line, key) {
                    sink.set(
                        at::attr_id(key).expect("boolean attribute id is defined"),
                        tv,
                    );
                }
            }
            "value-now" | "value-min" | "value-max" | "level" | "pos-in-set" | "set-size" => {
                if let Some(tv) = expect_prop_ty(ctx, *field, &[ParamType::Num], line, key) {
                    sink.set(
                        at::attr_id(key).expect("numeric attribute id is defined"),
                        tv,
                    );
                }
            }
            "checked" => {
                if let Some(tv) = expect_semantic_enum_prop(
                    ctx,
                    *field,
                    &["false", "true", "mixed"],
                    true,
                    line,
                    key,
                ) {
                    sink.set(at::CHECKED, tv);
                }
            }
            "live" => {
                if let Some(tv) = expect_semantic_enum_prop(
                    ctx,
                    *field,
                    &["off", "polite", "assertive"],
                    false,
                    line,
                    key,
                ) {
                    sink.set(at::LIVE, tv);
                }
            }
            "role" => ctx.error("ref", "role expects an identifier or string".into(), line),
            "content" | "text" => sink.content = Some(TVal::Prop(*field)),
            "act" | "field" | "submit" | "press" | "context" | "dblclick" | "drag" | "drop"
            | "resize" | "pointer-move" | "pointer-up" | "drag-update" | "drag-end" | "animate"
            | "transition" | "keys" | "field-sync" => ctx.error(
                "ref",
                format!("template prop cannot supply reserved attribute '{key}'"),
                line,
            ),
            _ => {
                if let Some(id) = at::attr_id(key) {
                    sink.set(id, TVal::Prop(*field));
                } else {
                    ctx.warn("attr", format!("unknown attribute '{key}'"), line);
                }
            }
        }
        return;
    }
    // params: size contexts handled in size_spec; other contexts below
    match key {
        "w" | "h" => {
            if let Some(tv) = size_spec(ctx, rv, line, key) {
                sink.set(if key == "w" { at::W } else { at::H }, tv);
            }
        }
        "min-w" | "max-w" | "min-h" | "max-h" => {
            let id = match key {
                "min-w" => at::MIN_W,
                "max-w" => at::MAX_W,
                "min-h" => at::MIN_H,
                _ => at::MAX_H,
            };
            if let RVal::Param(ix) = rv {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Num], line, key) {
                    sink.set(id, tv);
                }
            } else if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(id, TVal::Num(v));
            }
        }
        "pad" => match rv {
            RVal::Num(v) => sink.set(at::PAD, TVal::Tuple(vec![*v, *v, *v, *v])),
            RVal::Tup(items) if items.len() == 2 => {
                if let Some(tv) = tuple_val(ctx, items, line, key, "pad expects 1, 2, or 4 numbers")
                {
                    let quad = match tv {
                        TVal::Tuple(n) => TVal::Tuple(vec![n[0], n[1], n[0], n[1]]),
                        TVal::TupleDyn(m) => members_val(vec![m[0], m[1], m[0], m[1]]),
                        other => other,
                    };
                    sink.set(at::PAD, quad);
                }
            }
            RVal::Tup(items) if items.len() == 4 => {
                if let Some(tv) = tuple_val(ctx, items, line, key, "pad expects 1, 2, or 4 numbers")
                {
                    sink.set(at::PAD, tv);
                }
            }
            _ => ctx.error("ref", "pad expects 1, 2, or 4 numbers".into(), line),
        },
        "gap" => {
            if let RVal::Tup(items) = rv
                && items.len() == 2
            {
                if let Some(tv) = tuple_val(ctx, items, line, key, "gap expects main,cross") {
                    sink.set(at::GAP, tv);
                }
            } else if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(at::GAP, TVal::Num(v));
            }
        }
        "axis" => match rv {
            RVal::Kw(k) if k == "row" || k == "col" => sink.set(at::AXIS, TVal::Enum(k.clone())),
            _ => ctx.error("ref", "axis expects row|col".into(), line),
        },
        "pack" => match rv {
            RVal::Kw(k) if PACKS.contains(&k.as_str()) => sink.set(at::PACK, TVal::Enum(k.clone())),
            _ => ctx.error(
                "ref",
                "pack expects one of ['between', 'center', 'end', 'start']".into(),
                line,
            ),
        },
        "align" => match rv {
            RVal::Kw(k) if ALIGNS.contains(&k.as_str()) || STACK_ALIGNS.contains(&k.as_str()) => {
                sink.set(at::ALIGN, TVal::Enum(k.clone()));
            }
            _ => ctx.error("ref", "invalid align value".into(), line),
        },
        "self" => match rv {
            RVal::Kw(k) if ALIGNS.contains(&k.as_str()) || STACK_ALIGNS.contains(&k.as_str()) => {
                sink.set(at::SELF_ALIGN, TVal::Enum(k.clone()));
            }
            _ => ctx.error("ref", "invalid self value".into(), line),
        },
        "offset" => match rv {
            RVal::Tup(items) if items.len() == 2 => {
                if let Some(tv) = tuple_val(ctx, items, line, key, "offset expects x,y") {
                    sink.set(at::OFFSET, tv);
                }
            }
            _ => ctx.error("ref", "offset expects x,y".into(), line),
        },
        "at" => match rv {
            RVal::Tup(items) if items.len() == 2 => {
                if let Some(tv) = tuple_val(ctx, items, line, key, "at expects x,y") {
                    sink.set(at::AT, tv);
                }
            }
            RVal::Num(v) => sink.set(at::AT, TVal::Tuple(vec![*v, *v])),
            _ => ctx.error("ref", "at expects x,y".into(), line),
        },
        "anchor" => match rv {
            RVal::Kw(k) if STACK_ALIGNS.contains(&k.as_str()) || k == "center" => {
                sink.set(at::ANCHOR, TVal::Enum(k.clone()));
            }
            _ => ctx.error(
                "ref",
                "anchor expects a 9-position keyword (top-start .. bottom-end, center)".into(),
                line,
            ),
        },
        "attach" => match rv {
            RVal::Str(key) => sink.set(at::ATTACH, TVal::Str(key.clone())),
            RVal::Param(ix) => {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Text], line, key) {
                    sink.set(at::ATTACH, tv);
                }
            }
            _ => ctx.error(
                "ref",
                "attach expects a string literal, Text param, or Text item prop".into(),
                line,
            ),
        },
        "gravity" => match rv {
            RVal::Kw(value) if GRAVITIES.contains(&value.as_str()) => {
                sink.set(at::GRAVITY, TVal::Enum(value.clone()));
            }
            _ => ctx.error(
                "ref",
                "gravity expects below|above|left|right with start|center|end alignment".into(),
                line,
            ),
        },
        "collide" => match rv {
            RVal::Kw(value) if matches!(value.as_str(), "auto" | "none") => {
                sink.set(at::COLLIDE, TVal::Enum(value.clone()));
            }
            _ => ctx.error("ref", "collide expects auto|none".into(), line),
        },
        "bg" | "stroke" => {
            let id = if key == "bg" { at::BG } else { at::STROKE };
            if matches!(rv, RVal::Kw(keyword) if keyword == "current") {
                if ctx.icon_depth != 0 {
                    sink.set(id, TVal::PaintCurrent);
                } else {
                    ctx.error(
                        "ref",
                        "`current` paint is only valid inside an icon declaration".into(),
                        line,
                    );
                }
                return;
            }
            if let RVal::Param(ix) = rv {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Color], line, key) {
                    sink.set(id, tv);
                }
                return;
            }
            match color_str(ctx, rv, line, key) {
                Ok(Some(s)) => match color::parse_paint(&s) {
                    Some(p) => sink.set(id, TVal::Paint(p)),
                    None => ctx.warn("attr", format!("unparseable color '{s}' for {key}"), line),
                },
                Ok(None) => {
                    if matches!(rv, RVal::Kw(_)) {
                        // explicit none/transparent: emit so patches can clear
                        sink.set(id, TVal::Paint(Paint::None));
                    }
                }
                Err(()) => {}
            }
        }
        "scroll" => match rv {
            RVal::Kw(mode) if mode == "cross" => sink.flag_mask |= fl::SCROLL_CROSS,
            RVal::Kw(mode) if mode == "both" => {
                sink.flag_mask |= fl::SCROLL | fl::SCROLL_CROSS;
            }
            _ => ctx.error("ref", "scroll expects cross|both".into(), line),
        },
        "scrollbar" => match rv {
            RVal::Kw(k) if matches!(k.as_str(), "auto" | "always" | "never") => {
                sink.set(at::SCROLLBAR, TVal::Enum(k.clone()));
            }
            _ => ctx.error("ref", "scrollbar expects auto|always|never".into(), line),
        },
        "scrollbar-w" => {
            if let RVal::Param(ix) = rv {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Num], line, key) {
                    sink.set(at::SCROLLBAR_W, tv);
                }
            } else if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(at::SCROLLBAR_W, TVal::Num(v.max(0.0)));
            }
        }
        "scrollbar-fg" | "scrollbar-bg" => {
            let id = if key == "scrollbar-fg" {
                at::SCROLLBAR_FG
            } else {
                at::SCROLLBAR_BG
            };
            if let RVal::Param(ix) = rv {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Color], line, key) {
                    sink.set(id, tv);
                }
                return;
            }
            if let Ok(Some(s)) = color_str(ctx, rv, line, key) {
                if let Some(c) = color::parse_rgba(&s) {
                    sink.set(id, TVal::Color(c));
                } else {
                    ctx.warn("attr", format!("unparseable color '{s}' for {key}"), line);
                }
            }
        }
        "stroke-w" => {
            if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(at::STROKE_W, TVal::Num(v));
            }
        }
        "stroke-dash" => {
            let one;
            let items: &[RVal] = match rv {
                RVal::Tup(items) => items,
                other => {
                    one = [other.clone()];
                    &one
                }
            };
            match tuple_val(
                ctx,
                items,
                line,
                key,
                "stroke-dash expects numbers, e.g. stroke-dash=16,14",
            ) {
                Some(TVal::Tuple(n)) if n.len() == 1 => {
                    sink.set(at::STROKE_DASH, TVal::Tuple(vec![n[0], n[0]]));
                }
                Some(TVal::TupleDyn(m)) if m.len() == 1 => {
                    sink.set(at::STROKE_DASH, members_val(vec![m[0], m[0]]));
                }
                Some(tv) => sink.set(at::STROKE_DASH, tv),
                None => {}
            }
        }
        "radius" => match rv {
            RVal::Kw(k) if k == "full" => sink.set(at::RADIUS, TVal::Num(999.0)),
            _ => {
                if let Some(v) = num_val(ctx, rv, line, key) {
                    sink.set(at::RADIUS, TVal::Num(v));
                }
            }
        },
        "shadow" => {
            if let Some(shadows) = parse_shadows(ctx, rv, line) {
                sink.set(at::SHADOW, TVal::Shadows(shadows));
            }
        }
        "stroke-align" => match rv {
            RVal::Kw(k) if matches!(k.as_str(), "inside" | "center" | "outside") => {
                sink.set(at::STROKE_ALIGN, TVal::Enum(k.clone()));
            }
            _ => ctx.error(
                "ref",
                "stroke-align expects inside|center|outside".into(),
                line,
            ),
        },
        "stroke-sides" => {
            let one;
            let items: &[RVal] = match rv {
                RVal::Tup(items) => items,
                other => {
                    one = [other.clone()];
                    &one
                }
            };
            let mut mask = 0u16;
            let mut ok = true;
            for it in items {
                let bit = match it {
                    RVal::Kw(k) => match k.as_str() {
                        "t" | "top" => 1,
                        "r" | "right" => 2,
                        "b" | "bottom" => 4,
                        "l" | "left" => 8,
                        _ => 0,
                    },
                    _ => 0,
                };
                if bit == 0 {
                    ctx.error("ref", "stroke-sides expects t/r/b/l keywords".into(), line);
                    ok = false;
                    break;
                }
                mask |= bit;
            }
            if ok && mask != 0 {
                sink.set(at::STROKE_SIDES, TVal::Num(mask as f64));
            }
        }
        "blur" => {
            if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(at::BLUR, TVal::Num(v.max(0.0)));
            }
        }
        "backdrop" => match rv {
            RVal::Num(v) => sink.set(at::BACKDROP, TVal::Tuple(vec![v.max(0.0), 1.0])),
            RVal::Tup(items) if items.len() == 2 || items.len() == 3 => {
                if let Some(tv) = tuple_val(
                    ctx,
                    items,
                    line,
                    key,
                    "backdrop expects blur, blur,saturation, or blur,saturation,brightness",
                ) {
                    let clamped = match tv {
                        TVal::Tuple(n) => TVal::Tuple(n.into_iter().map(|x| x.max(0.0)).collect()),
                        TVal::TupleDyn(m) => members_val(
                            m.iter()
                                .map(|member| match member {
                                    TupMember::Lit(x) => TupMember::Lit(x.max(0.0)),
                                    TupMember::Param(ix) => TupMember::Param(*ix),
                                })
                                .collect(),
                        ),
                        other => other,
                    };
                    sink.set(at::BACKDROP, clamped);
                }
            }
            _ => ctx.error(
                "ref",
                "backdrop expects blur, blur,saturation, or blur,saturation,brightness".into(),
                line,
            ),
        },
        "scale" => {
            if let RVal::Tup(items) = rv
                && items.len() == 2
            {
                if let Some(tv) =
                    tuple_val(ctx, items, line, key, "scale expects a factor or sx,sy")
                {
                    sink.set(at::SCALE, tv);
                }
            } else if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(at::SCALE, TVal::Num(v));
            }
        }
        "smooth" => {
            if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(at::SMOOTH, TVal::Num(v.clamp(0.0, 1.0)));
            }
        }
        "grain" => match rv {
            RVal::Num(v) => sink.set(at::GRAIN, TVal::Tuple(vec![v.clamp(0.0, 1.0), 1.0])),
            RVal::Tup(items) if items.len() == 2 => {
                if let Some(tv) =
                    tuple_val(ctx, items, line, key, "grain expects amount or amount,size")
                {
                    let clamped = match tv {
                        TVal::Tuple(n) => TVal::Tuple(vec![n[0].clamp(0.0, 1.0), n[1].max(0.01)]),
                        TVal::TupleDyn(m) => members_val(
                            m.iter()
                                .enumerate()
                                .map(|(i, member)| match member {
                                    TupMember::Lit(x) if i == 0 => {
                                        TupMember::Lit(x.clamp(0.0, 1.0))
                                    }
                                    TupMember::Lit(x) => TupMember::Lit(x.max(0.01)),
                                    TupMember::Param(ix) => TupMember::Param(*ix),
                                })
                                .collect(),
                        ),
                        other => other,
                    };
                    sink.set(at::GRAIN, clamped);
                }
            }
            _ => ctx.error("ref", "grain expects amount or amount,size".into(), line),
        },
        "mask" | "backdrop-mask" => {
            let id = if key == "mask" {
                at::MASK
            } else {
                at::BACKDROP_MASK
            };
            if let RVal::Param(ix) = rv {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Color], line, key) {
                    sink.set(id, tv);
                }
                return;
            }
            match color_str(ctx, rv, line, key) {
                Ok(Some(s)) => match color::parse_paint(&s) {
                    Some(p) => sink.set(id, TVal::Paint(p)),
                    None => ctx.warn("attr", format!("unparseable color '{s}' for {key}"), line),
                },
                Ok(None) => {
                    if matches!(rv, RVal::Kw(_)) {
                        // explicit none/transparent: emit so patches can clear
                        sink.set(id, TVal::Paint(Paint::None));
                    }
                }
                Err(()) => {}
            }
        }
        "tilt" => match rv {
            RVal::Num(v) => sink.set(at::TILT, TVal::Tuple(vec![*v, 0.0, 800.0])),
            RVal::Tup(items) if items.len() == 2 || items.len() == 3 => {
                if let Some(tv) = tuple_val(
                    ctx,
                    items,
                    line,
                    key,
                    "tilt expects rx, rx,ry or rx,ry,depth",
                ) {
                    let padded = match tv {
                        TVal::Tuple(mut n) => {
                            if n.len() == 2 {
                                n.push(800.0);
                            }
                            TVal::Tuple(n)
                        }
                        TVal::TupleDyn(mut m) => {
                            if m.len() == 2 {
                                m.push(TupMember::Lit(800.0));
                            }
                            members_val(m)
                        }
                        other => other,
                    };
                    sink.set(at::TILT, padded);
                }
            }
            _ => ctx.error("ref", "tilt expects rx, rx,ry or rx,ry,depth".into(), line),
        },
        "animate" => {
            if sink.keyframe_ctx {
                ctx.warn(
                    "attr",
                    "animate inside an animation keyframe is not supported".into(),
                    line,
                );
                return;
            }
            if let Some(spec) = parse_animate(ctx, rv, line) {
                sink.set(at::ANIMATE, TVal::Str(spec.name.clone()));
                sink.animate = Some(spec);
            }
        }
        "transition" => {
            if sink.patch_ctx {
                ctx.warn(
                    "attr",
                    "transition inside a deferred `when` patch is not supported".into(),
                    line,
                );
                return;
            }
            if let Some(tr) = parse_transition(ctx, rv, line) {
                sink.transition = Some(tr);
            }
        }
        "opacity" => {
            if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(at::OPACITY, TVal::Num(v.clamp(0.0, 1.0)));
            }
        }
        "color" => {
            if let RVal::Param(ix) = rv {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Color], line, key) {
                    sink.set(at::COLOR, tv);
                }
                return;
            }
            if let RVal::Prop(field) = rv {
                if let Some(tv) = expect_prop_ty(ctx, *field, &[ParamType::Color], line, key) {
                    sink.set(at::COLOR, tv);
                }
                return;
            }
            if let Ok(Some(s)) = color_str(ctx, rv, line, key) {
                match color::parse_paint(&s) {
                    Some(Paint::Solid(c)) => sink.set(at::COLOR, TVal::Color(c)),
                    // explicit none: text keeps the inherited color (nothing to clear)
                    Some(Paint::None) => {}
                    Some(p) => sink.set(at::COLOR, TVal::Paint(p)),
                    None => ctx.warn("attr", format!("unparseable color '{s}' for color"), line),
                }
            }
        }
        "family" => match rv {
            RVal::Param(ix) => {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Text], line, key) {
                    sink.set(at::FAMILY, tv);
                }
            }
            RVal::Prop(field) => {
                if let Some(tv) = expect_prop_ty(ctx, *field, &[ParamType::Text], line, key) {
                    sink.set(at::FAMILY, tv);
                }
            }
            RVal::Str(family) | RVal::Kw(family) => {
                sink.set(at::FAMILY, TVal::Str(family.clone()));
            }
            _ => ctx.error("ref", "family expects text".into(), line),
        },
        "role" => match rv {
            RVal::Kw(role) => sink.set(at::ROLE, TVal::Enum(role.clone())),
            RVal::Str(role) => sink.set(at::ROLE, TVal::Str(role.clone())),
            _ => ctx.error("ref", "role expects an identifier or string".into(), line),
        },
        "label" | "desc" => {
            let id = if key == "label" { at::LABEL } else { at::DESC };
            match rv {
                RVal::Str(text) => sink.set(id, TVal::Str(text.clone())),
                RVal::Param(ix) => {
                    if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Text], line, key) {
                        sink.set(id, tv);
                    }
                }
                _ => ctx.error(
                    "ref",
                    format!("{key} expects a string literal, Text param, or Text item prop"),
                    line,
                ),
            }
        }
        "checked" => match rv {
            RVal::Kw(value) if value == "false" => sink.set(at::CHECKED, TVal::Num(0.0)),
            RVal::Kw(value) if value == "true" => sink.set(at::CHECKED, TVal::Num(1.0)),
            RVal::Kw(value) if value == "mixed" => {
                sink.set(at::CHECKED, TVal::Enum(value.clone()));
            }
            RVal::Str(value) if matches!(value.as_str(), "false" | "true" | "mixed") => {
                sink.set(at::CHECKED, TVal::Str(value.clone()));
            }
            RVal::Param(ix) => {
                if let Some(tv) = expect_semantic_enum_param(
                    ctx,
                    *ix,
                    &["false", "true", "mixed"],
                    true,
                    line,
                    key,
                ) {
                    sink.set(at::CHECKED, tv);
                }
            }
            _ => ctx.error("ref", "checked expects false|true|mixed text".into(), line),
        },
        "expanded" | "selected" | "modal" | "live-atomic" => {
            let id = at::attr_id(key).expect("boolean accessibility attribute id is defined");
            match rv {
                RVal::Kw(value) if value == "false" => sink.set(id, TVal::Num(0.0)),
                RVal::Kw(value) if value == "true" => sink.set(id, TVal::Num(1.0)),
                RVal::Param(ix) => {
                    if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Bool], line, key) {
                        sink.set(id, tv);
                    }
                }
                _ => ctx.error("ref", format!("{key} expects a boolean"), line),
            }
        }
        "active-descendant" | "controls" | "value-text" => {
            let id = at::attr_id(key).expect("text accessibility attribute id is defined");
            match rv {
                RVal::Str(text) => sink.set(id, TVal::Str(text.clone())),
                RVal::Param(ix) => {
                    if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Text], line, key) {
                        sink.set(id, tv);
                    }
                }
                _ => ctx.error(
                    "ref",
                    format!("{key} expects a string literal, Text param, or Text item prop"),
                    line,
                ),
            }
        }
        "value-now" | "value-min" | "value-max" | "level" | "pos-in-set" | "set-size" => {
            let id = at::attr_id(key).expect("numeric accessibility attribute id is defined");
            match rv {
                RVal::Num(value) if semantic_number_in_range(key, *value) => {
                    sink.set(id, TVal::Num(*value));
                }
                RVal::Num(value) => ctx.error(
                    "a11y-range",
                    format!("{key} value {value} is outside its valid range"),
                    line,
                ),
                RVal::Param(ix) => {
                    if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Num], line, key) {
                        sink.set(id, tv);
                    }
                }
                _ => ctx.error("ref", format!("{key} expects a number"), line),
            }
        }
        "live" => match rv {
            RVal::Kw(value) if matches!(value.as_str(), "off" | "polite" | "assertive") => {
                sink.set(at::LIVE, TVal::Enum(value.clone()));
            }
            RVal::Str(value) if matches!(value.as_str(), "off" | "polite" | "assertive") => {
                sink.set(at::LIVE, TVal::Str(value.clone()));
            }
            RVal::Param(ix) => {
                if let Some(tv) = expect_semantic_enum_param(
                    ctx,
                    *ix,
                    &["off", "polite", "assertive"],
                    false,
                    line,
                    key,
                ) {
                    sink.set(at::LIVE, tv);
                }
            }
            _ => ctx.error("ref", "live expects off|polite|assertive text".into(), line),
        },
        "strike" => {
            let value = match rv {
                RVal::Kw(value) if value == "false" => Some(TVal::Num(0.0)),
                RVal::Kw(value) if value == "true" => Some(TVal::Num(1.0)),
                RVal::Param(ix) => expect_param_ty(ctx, *ix, &[ParamType::Bool], line, key),
                RVal::Prop(field) => expect_prop_ty(ctx, *field, &[ParamType::Bool], line, key),
                _ => {
                    ctx.error("ref", "strike expects a boolean".into(), line);
                    None
                }
            };
            if let Some(value) = value {
                sink.set(at::STRIKE, value);
            }
        }
        "size" | "leading" | "tracking" => {
            let id = match key {
                "size" => at::SIZE,
                "leading" => at::LEADING,
                _ => at::TRACKING,
            };
            if let RVal::Param(ix) = rv {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Num], line, key) {
                    sink.set(id, tv);
                }
            } else if let RVal::Prop(field) = rv {
                if let Some(tv) = expect_prop_ty(ctx, *field, &[ParamType::Num], line, key) {
                    sink.set(id, tv);
                }
            } else if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(id, TVal::Num(v));
            }
        }
        "weight" => match rv {
            RVal::Param(ix) => {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Num], line, key) {
                    sink.set(at::WEIGHT, tv);
                }
            }
            RVal::Prop(field) => {
                if let Some(tv) = expect_prop_ty(ctx, *field, &[ParamType::Num], line, key) {
                    sink.set(at::WEIGHT, tv);
                }
            }
            RVal::Num(weight) => sink.set(
                at::WEIGHT,
                TVal::Num(slab_fonts::snap_weight(*weight) as f64),
            ),
            _ => ctx.error("ref", "weight expects a number".into(), line),
        },
        "rotate" => {
            if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(at::ROTATE, TVal::Num(v));
            }
        }
        "align-text" => match rv {
            RVal::Kw(k) if matches!(k.as_str(), "start" | "center" | "end") => {
                sink.set(at::ALIGN_TEXT, TVal::Enum(k.clone()));
            }
            _ => ctx.error("ref", "align-text expects start|center|end".into(), line),
        },
        "fit" => match rv {
            RVal::Kw(k) if matches!(k.as_str(), "cover" | "contain" | "stretch") => {
                sink.set(at::FIT, TVal::Enum(k.clone()));
            }
            _ => ctx.error("ref", "fit expects cover|contain|stretch".into(), line),
        },
        "src" => match rv {
            RVal::Str(source) => {
                if ctx.quiet == 0 && !ctx.images.iter().any(|(candidate, _)| candidate == source) {
                    ctx.images.push((source.clone(), line));
                }
                sink.set(at::SRC, TVal::Str(source.clone()));
            }
            RVal::Param(ix) => {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Text], line, key) {
                    sink.set(at::SRC, tv);
                }
            }
            _ => ctx.error(
                "ref",
                "src expects a string literal, Text param, or Text item prop".into(),
                line,
            ),
        },
        "d" => match rv {
            RVal::Param(ix) => {
                if let Some(tv) = expect_param_ty(ctx, *ix, &[ParamType::Text], line, key) {
                    sink.set(at::D, tv);
                }
            }
            RVal::Str(s) => match pathdata::normalize(s) {
                Some((verbs, coords)) => sink.set(at::D, TVal::Path(verbs, coords)),
                None => ctx.warn("attr", format!("invalid path data '{s}'"), line),
            },
            _ => ctx.error("ref", "d expects a string or Text reference".into(), line),
        },
        "cols" => {
            let one;
            let items: &[RVal] = match rv {
                RVal::Tup(items) => items,
                other => {
                    one = [other.clone()];
                    &one
                }
            };
            let mut enc = Vec::with_capacity(items.len() * 2);
            for it in items {
                match size_spec(ctx, it, line, "cols") {
                    Some(TVal::Size(spec)) => {
                        let (tag, v) = match spec {
                            SizeSpec::Fixed(x) => (0.0, x),
                            SizeSpec::Hug => (1.0, 0.0),
                            SizeSpec::Fill(wt) => (2.0, wt),
                            SizeSpec::Pct(p) => (3.0, p),
                        };
                        enc.push(tag);
                        enc.push(v);
                    }
                    Some(TVal::Param(_) | TVal::Prop(_)) => {
                        ctx.error("ref", "dynamic values cannot size grid tracks".into(), line);
                    }
                    _ => {}
                }
            }
            if !enc.is_empty() {
                sink.set(at::COLS, TVal::Tuple(enc));
            }
        }
        "span" => {
            if let Some(v) = num_val(ctx, rv, line, key) {
                sink.set(at::SPAN, TVal::Num((v as i64).max(1) as f64));
            }
        }
        "content" | "text" => match rv {
            RVal::Str(text) => {
                sink.set(at::CONTENT, TVal::Str(text.clone()));
            }
            _ => ctx.error("ref", format!("{key} expects a string"), line),
        },
        "field-sync" => match rv {
            RVal::Kw(mode) | RVal::Str(mode) if mode == "host" => {
                sink.field_sync_host = Some(true);
            }
            RVal::Kw(mode) | RVal::Str(mode) if mode == "implicit" => {
                sink.field_sync_host = Some(false);
            }
            _ => ctx.error(
                "field-sync",
                "field-sync expects `host` or `implicit`".into(),
                line,
            ),
        },
        "keys" => {
            if let Some(keys) = activation_keys(ctx, rv, line) {
                sink.set(at::KEYS, TVal::Str(keys.encoded));
                sink.key_signals = keys.signals;
                sink.key_map = keys.mapped;
                sink.flag_mask |= fl::FOCUSABLE;
            }
        }
        "act" | "field" | "submit" | "press" | "context" | "dblclick" | "drag" | "drop"
        | "resize" | "pointer-move" | "pointer-up" | "drag-update" | "drag-end" => {
            let name = match rv {
                RVal::Kw(k) => Some(k.clone()),
                RVal::Str(s) => Some(s.clone()),
                _ => None,
            };
            match name {
                Some(name) => {
                    let attr = match key {
                        "act" => at::ACT,
                        "field" => at::FIELD,
                        "submit" => at::SUBMIT,
                        "press" => at::PRESS,
                        "context" => at::CONTEXT,
                        "dblclick" => at::DBLCLICK,
                        "drag" => at::DRAG,
                        "drop" => at::DROP,
                        "resize" => at::RESIZE,
                        "pointer-move" => at::POINTER_MOVE,
                        "pointer-up" => at::POINTER_UP,
                        "drag-update" => at::DRAG_UPDATE,
                        "drag-end" => at::DRAG_END,
                        _ => unreachable!(),
                    };
                    sink.set(attr, TVal::Str(name.clone()));
                    match key {
                        "act" => sink.act = Some(name),
                        "field" => sink.field = Some(name),
                        "submit" => sink.submit = Some(name),
                        "press" => sink.press = Some(name),
                        "context" => sink.context = Some(name),
                        "dblclick" => sink.dblclick = Some(name),
                        "drag" => sink.drag = Some(name),
                        "drop" => sink.drop = Some(name),
                        "resize" => sink.resize = Some(name),
                        "pointer-move" => sink.pointer_move = Some(name),
                        "pointer-up" => sink.pointer_up = Some(name),
                        "drag-update" => sink.drag_update = Some(name),
                        "drag-end" => sink.drag_end = Some(name),
                        _ => unreachable!(),
                    }
                    if matches!(key, "act" | "field" | "press" | "drag") {
                        sink.flag_mask |= fl::FOCUSABLE;
                    }
                }
                None => ctx.error("ref", format!("{key} expects a signal name"), line),
            }
        }
        _ => {
            ctx.warn("attr", format!("unknown attribute '{key}'"), line);
        }
    }
}

/// Applies an attribute while retaining scalar token identity as one runtime
/// value reference. Named-theme variants are typed in the same attribute
/// context, so direct refs, deferred patches, and component-substituted refs
/// share exactly one representation.
fn apply_attr(ctx: &mut Ctx, sink: &mut Sink, key: &str, rv: &RVal, line: u32) {
    let Some(path) = first_token_path(rv).map(<[String]>::to_vec) else {
        apply_attr_concrete(ctx, sink, key, rv, line);
        return;
    };
    let base_rval = rval_without_tokens(rv);

    let before = sink.clone();
    apply_attr_concrete(ctx, sink, key, &base_rval, line);
    let affected: Vec<u16> = sink
        .entries
        .iter()
        .filter(|entry| before.get(entry.id) != Some(&entry.val))
        .map(|entry| entry.id)
        .collect();
    if affected.is_empty() {
        return;
    }

    let theme_tokens = ctx.theme_tokens.clone();
    let mut variants: Vec<Vec<(String, TVal)>> = vec![Vec::new(); affected.len()];
    for (theme, tree) in theme_tokens {
        let themed = rval_for_theme(ctx, rv, line, &tree);
        let mut themed_sink = before.clone();
        ctx.quiet = ctx.quiet.wrapping_add(1);
        apply_attr_concrete(ctx, &mut themed_sink, key, &themed, line);
        ctx.quiet = ctx.quiet.wrapping_sub(1);
        for (index, &attr) in affected.iter().enumerate() {
            if let Some(value) = themed_sink.get(attr) {
                variants[index].push((theme.clone(), value.clone()));
            }
        }
    }
    let path = path.join(".");
    for (index, attr) in affected.into_iter().enumerate() {
        let Some(base) = sink.get(attr).cloned() else {
            continue;
        };
        sink.set(
            attr,
            TVal::Token {
                path: path.clone(),
                base: Box::new(base),
                themes: std::mem::take(&mut variants[index]),
            },
        );
    }
}

// ---------------------------------------------------------------- keys

fn escape_key_segment(segment: &str) -> String {
    let mut escaped = String::with_capacity(segment.len());
    for ch in segment.chars() {
        match ch {
            '%' => escaped.push_str("%25"),
            '/' => escaped.push_str("%2F"),
            '~' => escaped.push_str("%7E"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn segment(ctx: &mut Ctx, a: &ANode, scope: &Scope, keys: &KeysRc) -> String {
    let seg = if let Some(kv) = a.attr("key") {
        let rv = resolve_value(ctx, kv, scope, a.line, ctx.tokens);
        escape_key_segment(&to_text(ctx, &rv))
    } else if let Some(id) = &a.id {
        format!("#{id}")
    } else {
        let mut k = keys.borrow_mut();
        let n = k.counters.entry(a.name.clone()).or_insert(0);
        let seg = format!("{}@{}", a.name, *n);
        *n += 1;
        seg
    };
    {
        let mut k = keys.borrow_mut();
        if let Some(&first) = k.seen.get(&seg) {
            drop(k);
            ctx.warn(
                "dup-key",
                format!("sibling key '{seg}' collides (first used at line {first})"),
                a.line,
            );
        } else {
            k.seen.insert(seg.clone(), a.line);
        }
    }
    if let Some(id) = &a.id {
        if let Some(&first) = ctx.seen_ids.get(id) {
            ctx.warn(
                "dup-id",
                format!("#{id} resolves more than once (first at line {first})"),
                a.line,
            );
        } else {
            ctx.seen_ids.insert(id.clone(), a.line);
        }
    }
    seg
}

fn join_key(parent: &str, seg: &str) -> String {
    if parent.is_empty() {
        seg.to_string()
    } else {
        format!("{parent}/{seg}")
    }
}

fn synth_key(parent: &str, keys: &KeysRc, kind: &str) -> String {
    let mut k = keys.borrow_mut();
    let n = k.counters.entry(kind.to_string()).or_insert(0);
    let seg = format!("{kind}@{}", *n);
    *n += 1;
    drop(k);
    join_key(parent, &seg)
}

// -------------------------------------------------------------- expansion

fn segment_each(ctx: &mut Ctx, a: &AEach, scope: &Scope, keys: &KeysRc) -> String {
    let seg = if let Some((_, value)) = a.attrs.iter().find(|(name, _)| name == "key") {
        let rv = resolve_value(ctx, value, scope, a.line, ctx.tokens);
        escape_key_segment(&to_text(ctx, &rv))
    } else if let Some(id) = &a.id {
        format!("#{id}")
    } else {
        let mut k = keys.borrow_mut();
        let n = k.counters.entry("each".into()).or_insert(0);
        let seg = format!("each@{}", *n);
        *n += 1;
        seg
    };
    {
        let mut k = keys.borrow_mut();
        if let Some(&first) = k.seen.get(&seg) {
            ctx.warn(
                "dup-key",
                format!("sibling key '{seg}' collides (first used at line {first})"),
                a.line,
            );
        } else {
            k.seen.insert(seg.clone(), a.line);
        }
    }
    if let Some(id) = &a.id {
        if let Some(&first) = ctx.seen_ids.get(id) {
            ctx.warn(
                "dup-id",
                format!("#{id} resolves more than once (first at line {first})"),
                a.line,
            );
        } else {
            ctx.seen_ids.insert(id.clone(), a.line);
        }
    }
    seg
}
fn collect_prop_font_fields(
    nodes: &[CNode],
    family_fields: &mut BTreeSet<u32>,
    weight_fields: &mut BTreeSet<u32>,
) {
    for node in nodes {
        for attr in &node.attrs {
            match (attr.id, &attr.val) {
                (at::FAMILY, TVal::Prop(field)) => {
                    family_fields.insert(*field);
                }
                (at::WEIGHT, TVal::Prop(field)) => {
                    weight_fields.insert(*field);
                }
                _ => {}
            }
        }
        for patch in &node.patches {
            for attr in &patch.attrs {
                match (attr.id, &attr.val) {
                    (at::FAMILY, TVal::Prop(field)) => {
                        family_fields.insert(*field);
                    }
                    (at::WEIGHT, TVal::Prop(field)) => {
                        weight_fields.insert(*field);
                    }
                    _ => {}
                }
            }
        }
        // Nested each templates use their own schema field numbering and collect
        // their requirements when that each is expanded.
        if node.kind == nk::EACH {
            continue;
        }
        for patch in &node.patches {
            collect_prop_font_fields(&patch.children, family_fields, weight_fields);
        }
        collect_prop_font_fields(&node.children, family_fields, weight_fields);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_list_default_font_candidates(
    schemas: &[ListInfo],
    schema_row: u32,
    items: &[ListItemInfo],
    target_schema: u32,
    family_fields: &BTreeSet<u32>,
    weight_fields: &BTreeSet<u32>,
    families: &mut BTreeSet<String>,
    weights: &mut BTreeSet<u16>,
) {
    let Some(schema) = schemas.get(schema_row as usize) else {
        return;
    };
    for item in items {
        if schema_row == target_schema {
            for field in family_fields {
                if let Some(TVal::Str(family)) = item.values.get(*field as usize) {
                    families.insert(family.clone());
                }
            }
            for field in weight_fields {
                if let Some(TVal::Num(weight)) = item.values.get(*field as usize) {
                    weights.insert(slab_fonts::snap_weight(*weight));
                }
            }
        }
        for (field, value) in schema.fields.iter().zip(&item.values) {
            if let (Some(sub), TVal::List(nested)) = (field.sub, value) {
                collect_list_default_font_candidates(
                    schemas,
                    sub,
                    nested,
                    target_schema,
                    family_fields,
                    weight_fields,
                    families,
                    weights,
                );
            }
        }
    }
}

fn collect_dynamic_font_candidates(ctx: &mut Ctx, schema_row: u32, template: &[CNode]) {
    let mut family_fields = BTreeSet::new();
    let mut weight_fields = BTreeSet::new();
    collect_prop_font_fields(template, &mut family_fields, &mut weight_fields);
    if family_fields.is_empty() && weight_fields.is_empty() {
        return;
    }

    let mut families = BTreeSet::new();
    let mut weights = BTreeSet::new();
    if let Some(schema) = ctx.list_schemas.get(schema_row as usize) {
        for field in &family_fields {
            if let Some(ListFieldInfo {
                default: TVal::Str(family),
                ..
            }) = schema.fields.get(*field as usize)
            {
                families.insert(family.clone());
            }
        }
        for field in &weight_fields {
            if let Some(ListFieldInfo {
                default: TVal::Num(weight),
                ..
            }) = schema.fields.get(*field as usize)
            {
                weights.insert(slab_fonts::snap_weight(*weight));
            }
        }
    }
    for param in &ctx.params {
        if let (Some(root_schema), TVal::List(items)) = (param.list, &param.default) {
            collect_list_default_font_candidates(
                &ctx.list_schemas,
                root_schema,
                items,
                schema_row,
                &family_fields,
                &weight_fields,
                &mut families,
                &mut weights,
            );
        }
    }
    ctx.font_families.extend(families);
    ctx.font_weights.extend(weights);
}

#[allow(clippy::too_many_arguments)] // Expansion context mirrors the recursive traversal state.
fn expand_each(
    ctx: &mut Ctx,
    a: &AEach,
    scope: &Scope,
    depth: usize,
    parent_key: &str,
    keys: &KeysRc,
    parent_kind: u8,
    parent_flags: u16,
) -> Option<CNode> {
    for (name, _) in &a.attrs {
        if !matches!(name.as_str(), "key" | "item-extent" | "overscan") {
            ctx.warn(
                "attr",
                format!("each accepts key=, item-extent=, overscan=, and #id; ignored '{name}'"),
                a.line,
            );
        }
    }

    let (schema_row, target) = if a.prop {
        match scope.get(&a.param).cloned() {
            Some(RVal::Param(param)) => {
                let Some(info) = ctx.params.get(param as usize) else {
                    ctx.error(
                        "each-target",
                        format!("each target '{}' is not a declared list param", a.param),
                        a.line,
                    );
                    return None;
                };
                let Some(schema_row) = info.list else {
                    ctx.error(
                        "each-target",
                        format!("each target '{}' is not a list-valued prop", a.param),
                        a.line,
                    );
                    return None;
                };
                (schema_row, TVal::Num(f64::from(param)))
            }
            resolved => {
                let Some(fields) = ctx.prop_fields.last() else {
                    ctx.error(
                        "each-target",
                        format!("each target '{}' is not an item property", a.param),
                        a.line,
                    );
                    return None;
                };
                let field = match resolved {
                    Some(RVal::Prop(field)) => field as usize,
                    _ => {
                        let Some(field) = fields.iter().position(|info| info.name == a.param)
                        else {
                            ctx.error(
                                "each-target",
                                format!(
                                    "each target '{}' is not a field of the enclosing item",
                                    a.param
                                ),
                                a.line,
                            );
                            return None;
                        };
                        field
                    }
                };
                let Some(info) = fields.get(field) else {
                    ctx.error(
                        "each-target",
                        format!("each target '{}' is not an item property", a.param),
                        a.line,
                    );
                    return None;
                };
                let Some(sub) = info.sub else {
                    ctx.error(
                        "each-target",
                        format!("each target '{}' is not a list-valued item field", a.param),
                        a.line,
                    );
                    return None;
                };
                (sub, TVal::Prop(field as u32))
            }
        }
    } else {
        let Some(param_ix) = ctx.params.iter().position(|p| p.name == a.param) else {
            ctx.error(
                "each-target",
                format!("each target '{}' is not a declared list param", a.param),
                a.line,
            );
            return None;
        };
        let Some(schema_row) = ctx.params[param_ix].list else {
            ctx.error(
                "each-target",
                format!("each target '{}' is not a list param", a.param),
                a.line,
            );
            return None;
        };
        (schema_row, TVal::Num(param_ix as f64))
    };
    let list = ctx.list_schemas[schema_row as usize].clone();
    let def = ctx.doc.def(&list.schema).cloned()?;

    if parent_kind == nk::PARA
        && !matches!(
            def.body.as_slice(),
            [Item::Node(node)] if node.name == "span"
        )
    {
        ctx.error(
            "each-span",
            "an `each` directly inside `para` requires a schema def containing exactly one span"
                .into(),
            a.line,
        );
        return None;
    }

    let virtual_flag = a.flags.iter().any(|flag| flag == "virtual");
    let mut flags = 0;
    let mut attrs = vec![AttrE {
        id: at::EACH,
        val: target,
    }];
    attrs.push(AttrE {
        id: at::AXIS,
        val: TVal::Enum(
            if ctx.layout_axes.last().copied().unwrap_or(false) {
                "row"
            } else {
                "col"
            }
            .into(),
        ),
    });
    if virtual_flag {
        if parent_flags & fl::SCROLL == 0 || !matches!(parent_kind, nk::ROW | nk::COL) {
            ctx.error(
                "virtual-ctx",
                "`virtual` requires an each directly inside a main-axis scroll row or col".into(),
                a.line,
            );
        }
        let extent = a
            .attrs
            .iter()
            .find(|(name, _)| name == "item-extent")
            .and_then(
                |(_, value)| match resolve_value(ctx, value, scope, a.line, ctx.tokens) {
                    RVal::Num(value) if value.is_finite() && value > 0.0 => Some(value),
                    _ => None,
                },
            );
        if let Some(extent) = extent {
            attrs.push(AttrE {
                id: at::ITEM_EXTENT,
                val: TVal::Num(extent),
            });
        } else {
            ctx.error(
                "virtual-extent",
                "`virtual` requires a positive numeric item-extent".into(),
                a.line,
            );
        }
        let overscan = a
            .attrs
            .iter()
            .find(|(name, _)| name == "overscan")
            .and_then(
                |(_, value)| match resolve_value(ctx, value, scope, a.line, ctx.tokens) {
                    RVal::Num(value) if value.is_finite() && value >= 0.0 => Some(value.floor()),
                    _ => None,
                },
            )
            .unwrap_or(4.0);
        attrs.push(AttrE {
            id: at::OVERSCAN,
            val: TVal::Num(overscan),
        });
        flags |= fl::VIRTUAL;
    }

    let node_key = join_key(parent_key, &segment_each(ctx, a, scope, keys));
    let mut template_scope = Scope::new();
    for (field, info) in list.fields.iter().enumerate() {
        template_scope.insert(info.name.clone(), RVal::Prop(field as u32));
    }
    let mut children = Vec::new();
    if !ctx.each_schemas.contains(&schema_row) {
        ctx.each_schemas.push(schema_row);
        let template_keys: KeysRc = Rc::new(RefCell::new(Keys::default()));
        ctx.prop_fields.push(list.fields.clone());
        ctx.each_depth += 1;
        for item in &def.body {
            match item {
                Item::Node(node) => children.extend(expand_node(
                    ctx,
                    node,
                    &template_scope,
                    depth + 1,
                    "",
                    &template_keys,
                    None,
                )),
                Item::Each(each) => {
                    if let Some(child) = expand_each(
                        ctx,
                        each,
                        &template_scope,
                        depth + 1,
                        "",
                        &template_keys,
                        nk::EACH,
                        0,
                    ) {
                        children.push(child);
                    }
                }
                Item::Text(_, _) | Item::When(_) => {}
            }
        }
        let _ = ctx.prop_fields.pop();
        ctx.each_depth -= 1;
        let _ = ctx.each_schemas.pop();
    }
    for root in &children {
        if !root.children.is_empty() || !matches!(root.kind, nk::TEXT | nk::SPAN) {
            continue;
        }
        let axes = root
            .attrs
            .iter()
            .filter_map(|attr| match (attr.id, &attr.val) {
                (at::W, TVal::Size(SizeSpec::Fill(_))) => Some("w"),
                (at::H, TVal::Size(SizeSpec::Fill(_))) => Some("h"),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !axes.is_empty() {
            ctx.warn(
                "fill-unbounded",
                format!(
                    "{}=fill on leaf each item root '{}' resolves as hug; \
                     wrap the leaf in a row or col with fill sizing",
                    axes.join("="),
                    root.key
                ),
                root.line,
            );
        }
    }
    collect_dynamic_font_candidates(ctx, schema_row, &children);
    Some(CNode {
        kind: nk::EACH,
        line: a.line,
        id: a.id.clone(),
        key: node_key,
        flags,
        attrs,
        content: None,
        children,
        patches: Vec::new(),
        animate: None,
        conditional_animations: vec![],
        transition: None,
        act: None,
        field: None,
        submit: None,
        press: None,
        context: None,
        dblclick: None,
        drag: None,
        drop: None,
        resize: None,
        pointer_move: None,
        pointer_up: None,
        drag_update: None,
        drag_end: None,
        conditional_signals: vec![],
        hole: None,
    })
}

fn expand_node(
    ctx: &mut Ctx,
    a: &ANode,
    scope: &Scope,
    depth: usize,
    parent_key: &str,
    keys: &KeysRc,
    slot: Option<&Rc<SlotPayload>>,
) -> Vec<CNode> {
    if depth > MAX_DEPTH {
        ctx.error("ref", "component recursion depth exceeded".into(), a.line);
        return vec![];
    }
    if a.name == "slot" {
        if let Some(payload) = slot {
            let payload = Rc::clone(payload);
            return resolve_slot(ctx, &payload, depth);
        }
        ctx.warn("attr", "slot outside a component body".into(), a.line);
        return vec![];
    }
    let seg = segment(ctx, a, scope, keys);
    let key = join_key(parent_key, &seg);
    if a.name.chars().next().is_some_and(|c| c.is_uppercase()) {
        return expand_call(ctx, a, scope, depth, key);
    }
    if !BUILTINS.contains(&a.name.as_str()) {
        ctx.error(
            "ref",
            format!("unknown node '{}' (components are Capitalized)", a.name),
            a.line,
        );
        return vec![];
    }
    vec![expand_builtin(ctx, a, scope, depth, key, slot)]
}

fn resolve_slot(ctx: &mut Ctx, payload: &Rc<SlotPayload>, depth: usize) -> Vec<CNode> {
    let mut out = Vec::new();
    for ch in &payload.children {
        match ch {
            Item::Node(node) => out.extend(expand_node(
                ctx,
                node,
                &payload.scope,
                depth,
                &payload.key,
                &payload.keys,
                None,
            )),
            Item::Each(each) => {
                if let Some(node) = expand_each(
                    ctx,
                    each,
                    &payload.scope,
                    depth,
                    &payload.key,
                    &payload.keys,
                    nk::GROUP,
                    0,
                ) {
                    out.push(node);
                }
            }
            Item::Text(_, _) | Item::When(_) => {}
        }
    }
    out
}

fn expand_call(ctx: &mut Ctx, a: &ANode, scope: &Scope, depth: usize, key: String) -> Vec<CNode> {
    let Some(d) = ctx.doc.def(&a.name) else {
        ctx.error("ref", format!("unknown component '{}'", a.name), a.line);
        return vec![];
    };
    let d = d.clone();
    // bind params: named attrs win, then positional args, then defaults
    let mut new_scope = Scope::new();
    let mut args = a.args.iter();
    for (pname, default) in &d.params {
        let v = if let Some(av) = a.attr(pname) {
            resolve_value(ctx, av, scope, a.line, ctx.tokens)
        } else if let Some(arg) = args.next() {
            resolve_value(ctx, arg, scope, a.line, ctx.tokens)
        } else if let Some(dv) = default {
            resolve_value(ctx, dv, scope, a.line, ctx.tokens)
        } else {
            RVal::None
        };
        new_scope.insert(pname.clone(), v);
    }
    for (aname, _) in &a.attrs {
        if aname != "key" && !d.params.iter().any(|(p, _)| p == aname) {
            ctx.warn("attr", format!("{} has no prop '{aname}'", a.name), a.line);
        }
    }
    // caller children resolve in caller scope, spliced at `slot`; expanded
    // body roots and slot children both continue under the call's key.
    let body_keys: KeysRc = Rc::new(RefCell::new(Keys::default()));
    let payload = Rc::new(SlotPayload {
        children: a.children.clone(),
        scope: scope.clone(),
        key: key.clone(),
        keys: Rc::clone(&body_keys),
    });
    let mut out = Vec::new();
    for item in &d.body {
        match item {
            Item::Node(n) if n.name == "slot" => out.extend(resolve_slot(ctx, &payload, depth)),
            Item::Node(n) => out.extend(expand_node(
                ctx,
                n,
                &new_scope,
                depth + 1,
                &key,
                &body_keys,
                Some(&payload),
            )),
            Item::Each(each) => {
                if let Some(node) = expand_each(
                    ctx,
                    each,
                    &new_scope,
                    depth + 1,
                    &key,
                    &body_keys,
                    nk::GROUP,
                    0,
                ) {
                    out.push(node);
                }
            }
            Item::Text(_, _) | Item::When(_) => {}
        }
    }
    // the call-site id lands on the component's (first) root
    if let Some(id) = &a.id
        && let Some(first) = out.first_mut()
        && first.id.is_none()
    {
        first.id = Some(id.clone());
    }
    out
}

/// Resolve + apply all header attrs (style bundle first) into a sink under
/// the given token tree. Content/src/d from positional args included.
fn build_sink(
    ctx: &mut Ctx,
    a: &ANode,
    scope: &Scope,
    tree: &TokenTree,
    kind: u8,
    patch_ctx: bool,
) -> Sink {
    let mut sink = Sink {
        patch_ctx,
        ..Default::default()
    };
    // positional args
    for arg in &a.args {
        let rv = resolve_value(ctx, arg, scope, a.line, tree);
        match kind {
            nk::TEXT | nk::SPAN | nk::PARA => match rv {
                RVal::Token { path, base } => {
                    sink.content = Some(token_text_tval(ctx, &path, &base, a.line));
                }
                RVal::Param(ix) => {
                    if matches!(&ctx.params[ix as usize].ty, ParamType::List(_)) {
                        ctx.error(
                            "param-type",
                            "a list param cannot be used as text content".into(),
                            a.line,
                        );
                    } else {
                        sink.content = Some(TVal::Param(ix));
                    }
                }
                RVal::Prop(field) => sink.content = Some(TVal::Prop(field)),
                _ => {
                    let t = to_text(ctx, &rv);
                    sink.content = Some(TVal::Str(t));
                }
            },
            nk::IMG => apply_attr(ctx, &mut sink, "src", &rv, a.line),
            nk::PATH => apply_attr(ctx, &mut sink, "d", &rv, a.line),
            nk::ICON => match rv {
                RVal::Token { path, base } => {
                    let base = token_base(&base);
                    if matches!(base, RVal::Kw(_) | RVal::Str(_)) {
                        sink.set(at::SRC, token_text_tval(ctx, &path, base, a.line));
                    } else {
                        ctx.error(
                            "ref",
                            "icon name token must resolve to an identifier or string".into(),
                            a.line,
                        );
                    }
                }
                RVal::Kw(name) | RVal::Str(name) => sink.set(at::SRC, TVal::Str(name)),
                RVal::Param(ix) => {
                    if let Some(value) =
                        expect_param_ty(ctx, ix, &[ParamType::Text], a.line, "icon name")
                    {
                        sink.set(at::SRC, value);
                    }
                }
                RVal::Prop(field) => {
                    if let Some(value) =
                        expect_prop_ty(ctx, field, &[ParamType::Text], a.line, "icon name")
                    {
                        sink.set(at::SRC, value);
                    }
                }
                _ => ctx.error(
                    "ref",
                    "icon name expects an identifier, string, or Text reference".into(),
                    a.line,
                ),
            },
            nk::HOLE => {} // hole name; handled by the caller
            _ => ctx.warn(
                "attr",
                format!("ignored positional value on '{}'", a.name),
                a.line,
            ),
        }
    }
    // style bundle first so explicit attrs win
    if let Some(sv) = a.attr("style") {
        let got = resolve_value(ctx, sv, scope, a.line, tree);
        match got {
            RVal::Token { path, base } => {
                if let RVal::Group(group) = token_base(&base) {
                    for (key, entry) in &group.0 {
                        if let TokenEntry::Value(value) = entry {
                            let concrete =
                                resolve_value_d(ctx, value, &Scope::new(), a.line, tree, 0, false);
                            let mut leaf_path = path.clone();
                            leaf_path.push(key.clone());
                            apply_attr(
                                ctx,
                                &mut sink,
                                key,
                                &RVal::Token {
                                    path: leaf_path,
                                    base: Box::new(concrete),
                                },
                                a.line,
                            );
                        }
                    }
                } else {
                    ctx.error("ref", "style= expects a token group".into(), a.line);
                }
            }
            RVal::Group(g) => {
                for (k, v) in &g.0 {
                    if let TokenEntry::Value(val) = v {
                        let rv = resolve_value(ctx, val, &Scope::new(), a.line, tree);
                        apply_attr(ctx, &mut sink, k, &rv, a.line);
                    }
                }
            }
            RVal::None => {}
            _ => ctx.error("ref", "style= expects a token group".into(), a.line),
        }
    }
    for (k, v) in &a.attrs {
        if k == "style" || k == "key" {
            continue; // key= is reserved identity, consumed by the key path
        }
        let rv = resolve_value(ctx, v, scope, a.line, tree);
        apply_attr(ctx, &mut sink, k, &rv, a.line);
    }
    sink
}

fn warn_field_sync(
    ctx: &mut Ctx,
    field: &str,
    content: Option<&TVal>,
    host_managed: bool,
    line: u32,
) {
    if host_managed || ctx.quiet != 0 {
        return;
    }
    let Some(TVal::Param(param)) = content else {
        return;
    };
    let Some(info) = ctx.params.get(*param as usize) else {
        return;
    };
    if info.ty != ParamType::Text || info.name == field {
        return;
    }
    let param_name = info.name.clone();
    if !ctx
        .field_sync_warnings
        .insert((line, field.to_owned(), param_name.clone()))
    {
        return;
    }
    ctx.warn_with(
        "field-sync",
        format!(
            "field signal '{field}' edits content from text param '{param_name}', so implicit synchronization is disabled"
        ),
        line,
        format!(
            "use `field={param_name}` for implicit synchronization; if the host intentionally handles `{field}` Change signals, add `field-sync=host` on this node"
        ),
    );
}

fn expand_builtin(
    ctx: &mut Ctx,
    a: &ANode,
    scope: &Scope,
    depth: usize,
    node_key: String,
    slot: Option<&Rc<SlotPayload>>,
) -> CNode {
    let base_kind = match a.name.as_str() {
        "row" => nk::ROW,
        "col" | "box" => nk::COL,
        "group" => nk::GROUP,
        "wrap" => nk::WRAP,
        "grid" => nk::GRID,
        "stack" => nk::STACK,
        "canvas" => nk::CANVAS,
        "para" => nk::PARA,
        "text" => nk::TEXT,
        "span" => nk::SPAN,
        "rect" => nk::RECT,
        "img" => nk::IMG,
        "path" => nk::PATH,
        "icon" => nk::ICON,
        "divider" => nk::DIVIDER,
        "spacer" => nk::SPACER,
        "hole" => nk::HOLE,
        _ => unreachable!("builtin set checked by caller"),
    };

    let mut sink = build_sink(ctx, a, scope, ctx.tokens, base_kind, false);
    if base_kind == nk::PATH && ctx.icon_depth != 0 && sink.get(at::BG).is_none() {
        sink.set(at::BG, TVal::PaintCurrent);
    }

    // rule 10: client/env-conditional token overrides become per-site patches
    let mut variant_patches: Vec<CPatch> = Vec::new();
    let mut variant_animations = Vec::new();
    let mut variant_signals = Vec::new();
    if !ctx.variants.is_empty() {
        let variants = std::mem::take(&mut ctx.variants);
        for (cond, tree) in &variants {
            ctx.quiet += 1;
            let mut vsink = build_sink(ctx, a, scope, tree, base_kind, false);
            ctx.quiet -= 1;
            let mut patch_attrs: Vec<AttrE> = Vec::new();
            for e in &vsink.entries {
                if sink.get(e.id) != Some(&e.val) {
                    patch_attrs.push(e.clone());
                }
            }
            if vsink.content != sink.content
                && let Some(c) = &vsink.content
            {
                patch_attrs.push(AttrE {
                    id: at::CONTENT,
                    val: c.clone(),
                });
            }
            if patch_attrs.iter().any(|entry| entry.id == at::ANIMATE)
                && let Some(binding) = vsink.animate.take()
            {
                variant_animations.push(binding);
            }
            for (binding, trigger, attr) in [
                (vsink.act.take(), 0, at::ACT),
                (vsink.field.take(), 1, at::FIELD),
                (vsink.submit.take(), 2, at::SUBMIT),
                (vsink.press.take(), 3, at::PRESS),
                (vsink.context.take(), 4, at::CONTEXT),
                (vsink.dblclick.take(), 5, at::DBLCLICK),
                (vsink.drag.take(), 6, at::DRAG),
                (vsink.drop.take(), 7, at::DROP),
                (vsink.resize.take(), 8, at::RESIZE),
                (vsink.pointer_move.take(), 9, at::POINTER_MOVE),
                (vsink.pointer_up.take(), 10, at::POINTER_UP),
                (vsink.drag_update.take(), 11, at::DRAG_UPDATE),
                (vsink.drag_end.take(), 12, at::DRAG_END),
            ] {
                if patch_attrs.iter().any(|entry| entry.id == attr)
                    && let Some(name) = binding
                {
                    variant_signals.push((name, trigger));
                }
            }
            if !patch_attrs.is_empty() {
                variant_patches.push(CPatch {
                    cond: cond.clone(),
                    attrs: patch_attrs,
                    flag_mask: 0,
                    children: vec![],
                    line: a.line,
                });
            }
        }
        ctx.variants = variants;
    }

    // fold axis into the kind for box/row/col (base attrs only)
    let mut kind = base_kind;
    if let Some(TVal::Enum(ax)) = sink.get(at::AXIS).cloned() {
        let axis_row = ax == "row";
        if matches!(a.name.as_str(), "box" | "row" | "col") {
            kind = if axis_row { nk::ROW } else { nk::COL };
            sink.entries.retain(|e| e.id != at::AXIS);
        }
    }

    let mut node = CNode {
        kind,
        line: a.line,
        id: a.id.clone(),
        key: node_key.clone(),
        flags: 0,
        attrs: Vec::new(),
        content: None,
        children: Vec::new(),
        patches: variant_patches,
        animate: None,
        conditional_animations: variant_animations,
        transition: None,
        act: None,
        field: None,
        submit: None,
        press: None,
        context: None,
        dblclick: None,
        drag: None,
        drop: None,
        resize: None,
        pointer_move: None,
        pointer_up: None,
        drag_update: None,
        drag_end: None,
        conditional_signals: variant_signals,
        hole: None,
    };
    for f in &a.flags {
        if f == "strike" {
            sink.set(at::STRIKE, TVal::Num(1.0));
        } else {
            node.flags |= flag_bit(f);
        }
    }
    if kind == nk::DIVIDER {
        node.flags |= fl::FOCUSABLE;
    }

    let child_keys: KeysRc = Rc::new(RefCell::new(Keys::default()));
    let child_is_row = match sink.get(at::AXIS) {
        Some(TVal::Enum(axis)) => axis == "row",
        _ => matches!(kind, nk::ROW | nk::PARA),
    };
    ctx.layout_axes.push(child_is_row);

    for ch in &a.children {
        match ch {
            Item::Text(text, line) => {
                if kind == nk::PARA {
                    node.children.push(CNode {
                        kind: nk::SPAN,
                        line: *line,
                        id: None,
                        key: synth_key(&node_key, &child_keys, "span"),
                        flags: 0,
                        attrs: vec![],
                        content: Some(TVal::Str(text.clone())),
                        children: vec![],
                        patches: vec![],
                        animate: None,
                        conditional_animations: vec![],
                        transition: None,
                        act: None,
                        field: None,
                        submit: None,
                        press: None,
                        context: None,
                        dblclick: None,
                        drag: None,
                        drop: None,
                        resize: None,
                        pointer_move: None,
                        pointer_up: None,
                        drag_update: None,
                        drag_end: None,
                        conditional_signals: vec![],
                        hole: None,
                    });
                } else if matches!(kind, nk::TEXT | nk::SPAN) && sink.content.is_none() {
                    sink.content = Some(TVal::Str(text.clone()));
                } else {
                    ctx.warn("attr", "bare string child outside para/text".into(), *line);
                }
            }
            Item::When(w) => match eval_cond(ctx, &w.cond, scope, w.line) {
                CondEval::Bool(true) => {
                    for (k, v) in &w.attrs {
                        let rv = resolve_value(ctx, v, scope, w.line, ctx.tokens);
                        apply_attr(ctx, &mut sink, k, &rv, w.line);
                    }
                    for f in &w.flags {
                        if f == "strike" {
                            sink.set(at::STRIKE, TVal::Num(1.0));
                        } else {
                            node.flags |= flag_bit(f);
                        }
                    }
                    for c in &w.children {
                        match c {
                            Item::Node(n) => node.children.extend(expand_node(
                                ctx,
                                n,
                                scope,
                                depth + 1,
                                &node_key,
                                &child_keys,
                                slot,
                            )),
                            Item::Each(each) => {
                                if let Some(child) = expand_each(
                                    ctx,
                                    each,
                                    scope,
                                    depth + 1,
                                    &node_key,
                                    &child_keys,
                                    kind,
                                    node.flags | sink.flag_mask,
                                ) {
                                    node.children.push(child);
                                }
                            }
                            Item::Text(text, line) if kind == nk::PARA => {
                                node.children.push(CNode {
                                    kind: nk::SPAN,
                                    line: *line,
                                    id: None,
                                    key: synth_key(&node_key, &child_keys, "span"),
                                    flags: 0,
                                    attrs: vec![],
                                    content: Some(TVal::Str(text.clone())),
                                    children: vec![],
                                    patches: vec![],
                                    animate: None,
                                    conditional_animations: vec![],
                                    transition: None,
                                    act: None,
                                    field: None,
                                    submit: None,
                                    press: None,
                                    context: None,
                                    dblclick: None,
                                    drag: None,
                                    drop: None,
                                    resize: None,
                                    pointer_move: None,
                                    pointer_up: None,
                                    drag_update: None,
                                    drag_end: None,
                                    conditional_signals: vec![],
                                    hole: None,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                CondEval::Bool(false) => {}
                CondEval::Defer(spec) => {
                    let mut psink = Sink {
                        patch_ctx: true,
                        ..Default::default()
                    };
                    for (k, v) in &w.attrs {
                        let rv = resolve_value(ctx, v, scope, w.line, ctx.tokens);
                        apply_attr(ctx, &mut psink, k, &rv, w.line);
                    }
                    if (psink.key_map && (psink.act.is_some() || sink.act.is_some()))
                        || (psink.act.is_some() && sink.key_map)
                    {
                        ctx.error(
                            "keys",
                            "mapped `keys=Key:signal` owns activation routing; remove `act=`"
                                .into(),
                            w.line,
                        );
                    }
                    if (psink.field.is_some() || psink.content.is_some())
                        && let Some(field) = psink.field.as_deref().or(sink.field.as_deref())
                    {
                        warn_field_sync(
                            ctx,
                            field,
                            psink.content.as_ref().or(sink.content.as_ref()),
                            psink
                                .field_sync_host
                                .or(sink.field_sync_host)
                                .unwrap_or(false),
                            w.line,
                        );
                    }
                    let mut flag_mask = psink.flag_mask;
                    for f in &w.flags {
                        if f == "strike" {
                            psink.set(at::STRIKE, TVal::Num(1.0));
                        } else {
                            flag_mask |= flag_bit(f);
                        }
                    }
                    let mut children = Vec::new();
                    for c in &w.children {
                        match c {
                            Item::Node(n) => children.extend(expand_node(
                                ctx,
                                n,
                                scope,
                                depth + 1,
                                &node_key,
                                &child_keys,
                                slot,
                            )),
                            Item::Each(each) => {
                                if let Some(child) = expand_each(
                                    ctx,
                                    each,
                                    scope,
                                    depth + 1,
                                    &node_key,
                                    &child_keys,
                                    kind,
                                    node.flags | sink.flag_mask,
                                ) {
                                    children.push(child);
                                }
                            }
                            Item::Text(_, _) | Item::When(_) => {}
                        }
                    }
                    if let Some(c) = &psink.content {
                        psink.entries.push(AttrE {
                            id: at::CONTENT,
                            val: c.clone(),
                        });
                    }
                    if let Some(binding) = psink.animate.take() {
                        node.conditional_animations.push(binding);
                    }
                    for name in psink.key_signals.drain(..) {
                        node.conditional_signals.push((name, 13));
                    }
                    let conditional_has_field = psink.field.is_some() || sink.field.is_some();
                    for (binding, trigger) in [
                        (psink.act.take(), 0),
                        (psink.field.take(), 1),
                        (psink.submit.take(), 2),
                        (psink.press.take(), 3),
                        (psink.context.take(), 4),
                        (psink.dblclick.take(), 5),
                        (psink.drag.take(), 6),
                        (psink.drop.take(), 7),
                        (psink.resize.take(), 8),
                        (psink.pointer_move.take(), 9),
                        (psink.pointer_up.take(), 10),
                        (psink.drag_update.take(), 11),
                        (psink.drag_end.take(), 12),
                    ] {
                        let Some(name) = binding else {
                            continue;
                        };
                        if trigger == 1 && kind != nk::TEXT {
                            ctx.warn("attr", "field= applies to text nodes".into(), w.line);
                            continue;
                        }
                        if trigger == 2 && (kind != nk::TEXT || !conditional_has_field) {
                            ctx.warn(
                                "attr",
                                "submit= applies only to text nodes with field=".into(),
                                w.line,
                            );
                            continue;
                        }
                        if trigger == 8 && kind != nk::DIVIDER {
                            ctx.warn(
                                "attr",
                                "resize= applies only to divider nodes".into(),
                                w.line,
                            );
                            continue;
                        }
                        node.conditional_signals.push((name, trigger));
                    }
                    node.patches.push(CPatch {
                        cond: spec,
                        attrs: psink.entries,
                        flag_mask,
                        children,
                        line: w.line,
                    });
                }
            },
            Item::Node(n) => {
                node.children.extend(expand_node(
                    ctx,
                    n,
                    scope,
                    depth + 1,
                    &node_key,
                    &child_keys,
                    slot,
                ));
            }
            Item::Each(each) => {
                if let Some(child) = expand_each(
                    ctx,
                    each,
                    scope,
                    depth + 1,
                    &node_key,
                    &child_keys,
                    kind,
                    node.flags | sink.flag_mask,
                ) {
                    node.children.push(child);
                }
            }
        }
    }
    let _ = ctx.layout_axes.pop();

    // the stack-child trap: `align` positions a node's CHILDREN; on a
    // childless node it silently does nothing — the author meant `self=`
    if sink.get(at::ALIGN).is_some()
        && node.children.is_empty()
        && node.patches.iter().all(|patch| patch.children.is_empty())
        && !matches!(kind, nk::TEXT | nk::SPAN | nk::PARA)
    {
        ctx.warn(
            "attr",
            "align= aligns this node's children (it has none); \
             use self= to position the node in its parent"
                .into(),
            a.line,
        );
    }

    // drain the sink now: folded `when` attrs above may still have written
    // animate/transition/act/field/content
    node.flags |= sink.flag_mask;
    if (sink.get(at::SCROLLBAR).is_some()
        || sink.get(at::SCROLLBAR_W).is_some()
        || sink.get(at::SCROLLBAR_FG).is_some()
        || sink.get(at::SCROLLBAR_BG).is_some())
        && (node.flags & (fl::SCROLL | fl::SCROLL_CROSS)) == 0
    {
        ctx.warn(
            "attr",
            "scrollbar attributes apply only to nodes with an active scroll axis".into(),
            a.line,
        );
    }
    if kind != nk::TEXT
        && let Some(bind) = &sink.animate
        && ctx.anim_content.contains(&bind.name)
    {
        ctx.warn(
            "attr",
            format!(
                "animation '{}' has content keyframes, which apply only to text nodes",
                bind.name
            ),
            bind.line,
        );
    }
    if sink.key_map && sink.act.is_some() {
        ctx.error(
            "keys",
            "mapped `keys=Key:signal` owns activation routing; remove `act=`".into(),
            a.line,
        );
    }
    if let Some(field) = sink.field.as_deref() {
        warn_field_sync(
            ctx,
            field,
            sink.content.as_ref(),
            sink.field_sync_host.unwrap_or(false),
            a.line,
        );
    }
    for name in sink.key_signals.drain(..) {
        node.conditional_signals.push((name, 13));
    }
    node.content = sink.content.take();
    node.animate = sink.animate.take();
    node.transition = sink.transition.take();
    node.act = sink.act.take();
    node.field = sink.field.take();
    node.submit = sink.submit.take();
    node.press = sink.press.take();
    node.context = sink.context.take();
    node.dblclick = sink.dblclick.take();
    node.drag = sink.drag.take();
    node.drop = sink.drop.take();
    node.resize = sink.resize.take();
    node.pointer_move = sink.pointer_move.take();
    node.pointer_up = sink.pointer_up.take();
    node.drag_update = sink.drag_update.take();
    node.drag_end = sink.drag_end.take();
    if kind == nk::DIVIDER {
        if node.drag.take().is_some() {
            ctx.warn(
                "attr",
                "`drag=` does not apply to divider; divider owns its resize gesture".into(),
                a.line,
            );
        }
        if node.drag_update.take().is_some() {
            ctx.warn(
                "attr",
                "`drag-update=` does not apply to divider; divider owns its resize gesture".into(),
                a.line,
            );
        }
        if node.drag_end.take().is_some() {
            ctx.warn(
                "attr",
                "`drag-end=` does not apply to divider; divider owns its resize gesture".into(),
                a.line,
            );
        }
        if node.flags & fl::DRAG_GHOST != 0 {
            ctx.warn(
                "attr",
                "`drag-ghost` does not apply to divider; divider owns its resize gesture".into(),
                a.line,
            );
            node.flags &= !fl::DRAG_GHOST;
        }
    }
    if node.drag.is_none() {
        if node.flags & fl::DRAG_GHOST != 0 {
            ctx.warn(
                "attr",
                "`drag-ghost` requires a `drag=` binding on the same node".into(),
                a.line,
            );
            node.flags &= !fl::DRAG_GHOST;
        }
        if node.drag_update.is_some() {
            ctx.warn(
                "attr",
                "`drag-update=` requires a `drag=` binding on the same node".into(),
                a.line,
            );
            node.drag_update = None;
        }
        if node.drag_end.is_some() {
            ctx.warn(
                "attr",
                "`drag-end=` requires a `drag=` binding on the same node".into(),
                a.line,
            );
            node.drag_end = None;
        }
    }

    if kind == nk::ICON {
        if a.args.len() != 1 {
            ctx.error("ref", "icon usage requires exactly one name".into(), a.line);
        }
        if !node.children.is_empty() {
            ctx.warn("attr", "icon usage takes no children".into(), a.line);
            node.children.clear();
        }
    }
    if kind == nk::DIVIDER
        && (!node.children.is_empty()
            || node.patches.iter().any(|patch| !patch.children.is_empty()))
    {
        ctx.warn("attr", "divider takes no children".into(), a.line);
        node.children.clear();
        for patch in &mut node.patches {
            patch.children.clear();
        }
    }

    // hole: name from the first positional arg
    if kind == nk::HOLE {
        if ctx.each_depth != 0 {
            ctx.error(
                "each-nest",
                "holes are not allowed inside an `each` template".into(),
                a.line,
            );
        }
        let name = a.args.first().and_then(|v| match v {
            Value::Kw(k) => Some(k.clone()),
            Value::Str(s) => Some(s.clone()),
            _ => None,
        });
        match name {
            Some(name) => {
                if let Some(&first) = ctx.holes.get(&name) {
                    ctx.error(
                        "dup-hole",
                        format!("hole '{name}' already declared (first at line {first})"),
                        a.line,
                    );
                } else {
                    ctx.holes.insert(name.clone(), a.line);
                }
                node.hole = Some(name);
            }
            None => ctx.error("ref", "hole requires a name (hole NAME)".into(), a.line),
        }
        if !node.children.is_empty() {
            ctx.warn("attr", "hole takes no children".into(), a.line);
            node.children.clear();
        }
    }

    // field= binds an EditState; only text nodes edit.
    if node.field.is_some() && kind != nk::TEXT {
        ctx.warn("attr", "field= applies to text nodes".into(), a.line);
        node.field = None;
    }
    if (node.flags & fl::MULTILINE) != 0 && (kind != nk::TEXT || node.field.is_none()) {
        ctx.warn(
            "attr",
            "`multiline` applies only to text nodes with field=".into(),
            a.line,
        );
        node.flags &= !fl::MULTILINE;
    }
    if node.flags & fl::ESCAPE_BLUR != 0 && (kind != nk::TEXT || node.field.is_none()) {
        ctx.warn(
            "attr",
            "`escape-blur` applies only to text nodes with field=".into(),
            a.line,
        );
        node.flags &= !fl::ESCAPE_BLUR;
    }
    if node.submit.is_some() && (kind != nk::TEXT || node.field.is_none()) {
        ctx.warn(
            "attr",
            "submit= applies only to text nodes with field=".into(),
            a.line,
        );
        node.submit = None;
    }
    if node.resize.is_some() && kind != nk::DIVIDER {
        ctx.warn(
            "attr",
            "resize= applies only to divider nodes".into(),
            a.line,
        );
        node.resize = None;
    }
    if node.flags & fl::DRAG_GHOST != 0 && node.drag.is_none() {
        ctx.warn("attr", "`drag-ghost` requires drag=".into(), a.line);
        node.flags &= !fl::DRAG_GHOST;
    }

    // Signal registry. Change, Submit, and Resize carry text; all other
    // triggers carry the common item identity and metadata only.
    if ctx.quiet == 0 {
        for (name, trigger) in [
            (&node.act, 0),
            (&node.field, 1),
            (&node.submit, 2),
            (&node.press, 3),
            (&node.context, 4),
            (&node.dblclick, 5),
            (&node.drag, 6),
            (&node.drop, 7),
            (&node.resize, 8),
            (&node.pointer_move, 9),
            (&node.pointer_up, 10),
            (&node.drag_update, 11),
            (&node.drag_end, 12),
        ] {
            if let Some(name) = name {
                register_signal(ctx, name.clone(), trigger, a.line);
            }
        }
        for (name, trigger) in &node.conditional_signals {
            register_signal(ctx, name.clone(), *trigger, a.line);
        }
    }

    node.attrs = sink.entries;
    node
}

fn signal_has_text(trigger: u8) -> bool {
    matches!(trigger, 1 | 2 | 8)
}

fn register_signal(ctx: &mut Ctx, name: String, trigger: u8, line: u32) {
    if let Some((_, previous_trigger, first)) =
        ctx.signals.iter().find(|(candidate, previous_trigger, _)| {
            *candidate == name && signal_has_text(*previous_trigger) != signal_has_text(trigger)
        })
    {
        let msg = format!(
            "signal '{name}' already bound with a different payload type \
             ({} at line {first})",
            if signal_has_text(*previous_trigger) {
                "text"
            } else {
                "non-text"
            }
        );
        ctx.warn("dup-signal", msg, line);
    }
    ctx.signals.push((name, trigger, line));
}

fn flag_bit(name: &str) -> u16 {
    match name {
        "clip" => fl::CLIP,
        "bleed" => fl::BLEED,
        "scroll" => fl::SCROLL,
        "nowrap" => fl::NOWRAP,
        "ellipsis" => fl::ELLIPSIS,
        "inert" => fl::INERT,
        "focusable" => fl::FOCUSABLE,
        "multiline" => fl::MULTILINE,
        "sticky" => fl::STICKY,
        "virtual" => fl::VIRTUAL,
        "drag-ghost" => fl::DRAG_GHOST,
        "escape-blur" => fl::ESCAPE_BLUR,
        _ => 0,
    }
}

// ------------------------------------------------------------------- params

fn scalar_zero(ty: &ParamType, enum_syms: &[String], transparent: bool) -> TVal {
    match ty {
        ParamType::Text => TVal::Str(String::new()),
        ParamType::Num => TVal::Num(0.0),
        ParamType::Pct => TVal::Size(SizeSpec::Pct(0.0)),
        ParamType::Color => TVal::Color(if transparent {
            [0, 0, 0, 0]
        } else {
            [0, 0, 0, 255]
        }),
        ParamType::Bool => TVal::Num(0.0),
        ParamType::Enum => TVal::Enum(enum_syms.first().cloned().unwrap_or_default()),
        ParamType::List(_) => TVal::List(Vec::new()),
    }
}

fn scalar_value(
    ctx: &mut Ctx,
    ty: &ParamType,
    enum_syms: &[String],
    value: &Value,
    line: u32,
) -> Option<TVal> {
    let rv = resolve_value(ctx, value, &Scope::new(), line, ctx.tokens);
    match ty {
        ParamType::Text => match &rv {
            RVal::Str(s) => Some(TVal::Str(s.clone())),
            _ => None,
        },
        ParamType::Num => match rv {
            RVal::Num(x) => Some(TVal::Num(x)),
            _ => None,
        },
        ParamType::Pct => match rv {
            RVal::Pct(p) => Some(TVal::Size(SizeSpec::Pct(p))),
            _ => None,
        },
        ParamType::Color => match &rv {
            RVal::Color(s) | RVal::Str(s) => color::parse_rgba(s).map(TVal::Color),
            _ => None,
        },
        ParamType::Bool => match &rv {
            RVal::Kw(k) if k == "true" || k == "false" => {
                Some(TVal::Num((k == "true") as i32 as f64))
            }
            _ => None,
        },
        ParamType::Enum => match &rv {
            RVal::Kw(k) if enum_syms.contains(k) => Some(TVal::Enum(k.clone())),
            _ => None,
        },
        ParamType::List(_) => None,
    }
}

fn check_param_default(ctx: &mut Ctx, decl: &ParamDecl) -> TVal {
    let ParamDefault::Scalar(value) = &decl.default else {
        ctx.error(
            "param-type",
            format!("default for scalar param '{}' must be a scalar", decl.name),
            decl.line,
        );
        return scalar_zero(&decl.ty, &decl.enum_syms, false);
    };
    if let Some(value) = scalar_value(ctx, &decl.ty, &decl.enum_syms, value, decl.line) {
        value
    } else {
        ctx.error(
            "param-type",
            format!(
                "default for param '{}' does not fit its declared type {}",
                decl.name,
                decl.ty.as_str()
            ),
            decl.line,
        );
        scalar_zero(&decl.ty, &decl.enum_syms, false)
    }
}

fn ensure_list_schema(ctx: &mut Ctx, schema: &str, line: u32, owner: &str) -> Option<u32> {
    if let Some(row) = ctx
        .list_schemas
        .iter()
        .position(|candidate| candidate.schema == schema)
    {
        return Some(row as u32);
    }
    let Some(def) = ctx.doc.def(schema).cloned() else {
        ctx.error(
            "list-def",
            format!("list '{owner}' references unknown def '{schema}'"),
            line,
        );
        return None;
    };
    if !def.export {
        ctx.error(
            "list-def",
            format!("list schema def '{schema}' must be exported"),
            line,
        );
        return None;
    }

    // Allocate before resolving fields: recursive and mutually-recursive defs
    // find this placeholder and terminate, then the second pass fills it.
    let row = ctx.list_schemas.len() as u32;
    ctx.list_schemas.push(ListInfo {
        schema: schema.into(),
        fields: Vec::new(),
    });
    let props = crate::export::infer_props(&def);
    let mut fields = Vec::with_capacity(props.len());
    for prop in props {
        if prop.name == "key" {
            ctx.error(
                "list-def",
                format!("list schema def '{schema}' may not declare reserved prop 'key'"),
                def.line,
            );
            continue;
        }
        let declared = def
            .params
            .iter()
            .find(|(name, _)| name == &prop.name)
            .and_then(|(_, value)| value.as_ref());
        let (default, sub) = match &prop.ty {
            ParamType::List(sub_schema) => {
                let sub = ensure_list_schema(ctx, sub_schema, def.line, schema);
                (TVal::List(Vec::new()), sub)
            }
            _ => (
                declared
                    .and_then(|value| scalar_value(ctx, &prop.ty, &[], value, def.line))
                    .unwrap_or_else(|| scalar_zero(&prop.ty, &[], true)),
                None,
            ),
        };
        fields.push(ListFieldInfo {
            name: prop.name,
            ty: prop.ty,
            enum_syms: Vec::new(),
            default,
            sub,
        });
    }
    ctx.list_schemas[row as usize].fields = fields;
    Some(row)
}

fn compile_list_items(
    ctx: &mut Ctx,
    schema_row: u32,
    raw_items: &[slab_syntax::ast::ListItem],
    owner: &str,
) -> Vec<ListItemInfo> {
    let schema = ctx.list_schemas[schema_row as usize].clone();
    let mut items = Vec::with_capacity(raw_items.len());
    for item in raw_items {
        let mut valid = true;
        if item.name != schema.schema {
            ctx.error(
                "param-type",
                format!(
                    "list '{owner}' expects {}(...) items, found {}(...)",
                    schema.schema, item.name
                ),
                item.line,
            );
            valid = false;
        }
        for (name, _) in &item.attrs {
            if !schema.fields.iter().any(|field| field.name == *name) {
                ctx.error(
                    "param-type",
                    format!("unknown field '{name}' in list item for '{owner}'"),
                    item.line,
                );
                valid = false;
            }
        }
        let mut values = Vec::with_capacity(schema.fields.len());
        for field in &schema.fields {
            let authored = item
                .attrs
                .iter()
                .find(|(name, _)| *name == field.name)
                .map(|(_, value)| value);
            let value = match (&field.ty, field.sub, authored) {
                (ParamType::List(_), Some(sub), Some(Value::List(nested))) => {
                    TVal::List(compile_list_items(ctx, sub, nested, &field.name))
                }
                (ParamType::List(_), _, Some(_)) => {
                    ctx.error(
                        "param-type",
                        format!(
                            "field '{}' in list '{owner}' expects a list literal",
                            field.name
                        ),
                        item.line,
                    );
                    valid = false;
                    field.default.clone()
                }
                (ParamType::List(_), _, None) => field.default.clone(),
                (_, _, Some(value)) => {
                    if let Some(value) =
                        scalar_value(ctx, &field.ty, &field.enum_syms, value, item.line)
                    {
                        value
                    } else {
                        ctx.error(
                            "param-type",
                            format!(
                                "field '{}' in list '{owner}' expects {}",
                                field.name,
                                field.ty.as_str()
                            ),
                            item.line,
                        );
                        valid = false;
                        field.default.clone()
                    }
                }
                (_, _, None) => field.default.clone(),
            };
            values.push(value);
        }
        if !valid {
            // Keep the row shape deterministic even though diagnostics prevent emission.
            values.resize_with(schema.fields.len(), || TVal::Num(0.0));
        }
        items.push(ListItemInfo { values });
    }
    items
}

fn compile_list_info(ctx: &mut Ctx, decl: &ParamDecl, schema: &str) -> Option<(u32, TVal)> {
    let row = ensure_list_schema(ctx, schema, decl.line, &decl.name)?;
    let ParamDefault::List(default_items) = &decl.default else {
        ctx.error(
            "param-type",
            format!(
                "default for list param '{}' must be a list literal",
                decl.name
            ),
            decl.line,
        );
        return Some((row, TVal::List(Vec::new())));
    };
    Some((
        row,
        TVal::List(compile_list_items(ctx, row, default_items, &decl.name)),
    ))
}

/// Names a def param must not use without a `warn[shadow]` (rule 8).
fn shadow_warns(ctx: &mut Ctx, doc: &Document) {
    for d in &doc.defs {
        for (p, _) in &d.params {
            if slab_slir::attrs::attr_id(p).is_some()
                || slab_syntax::parse::FLAGS.contains(&p.as_str())
            {
                let msg = format!(
                    "def {}: param '{p}' shadows the '{p}' attribute; bare '{p}' in \
                     value position resolves to the param",
                    d.name
                );
                ctx.warn("shadow", msg, d.line);
            } else if p == "fill" || p == "hug" {
                let msg = format!(
                    "def {}: param '{p}' shadows the reserved sizing keyword; \
                     '{p}' in value position stays the keyword and the param is unreachable",
                    d.name
                );
                ctx.warn("shadow", msg, d.line);
            }
        }
    }
}

// -------------------------------------------------------------------- entry

fn collect_theme_items(items: &[Item], mentions: &mut Vec<(u32, String)>) {
    for item in items {
        match item {
            Item::Node(node) => collect_theme_items(&node.children, mentions),
            Item::When(when) => {
                if let Cond::Theme(name) = &when.cond {
                    mentions.push((when.line, name.clone()));
                }
                collect_theme_items(&when.children, mentions);
            }
            Item::Each(_) | Item::Text(_, _) => {}
        }
    }
}

fn collect_themes(doc: &Document) -> Vec<String> {
    let mut mentions = Vec::new();
    for (cond, _, line) in &doc.topwhens {
        if let Cond::Theme(name) = cond {
            mentions.push((*line, name.clone()));
        }
    }
    for root in &doc.roots {
        collect_theme_items(&root.children, &mut mentions);
    }
    for def in &doc.defs {
        collect_theme_items(&def.body, &mut mentions);
    }
    mentions.sort_by_key(|(line, _)| *line);
    let mut themes = Vec::new();
    for (_, name) in mentions {
        if !themes.contains(&name) {
            themes.push(name);
        }
    }
    themes
}

fn collect_token_paths(tree: &TokenTree, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    for (name, entry) in &tree.0 {
        prefix.push(name.clone());
        match entry {
            TokenEntry::Group(group) => collect_token_paths(group, prefix, out),
            TokenEntry::Value(_) => out.push(prefix.clone()),
        }
        prefix.pop();
    }
}

fn collect_token_infos(ctx: &mut Ctx, base: &TokenTree) -> Vec<TokenInfo> {
    let mut paths = Vec::new();
    collect_token_paths(base, &mut Vec::new(), &mut paths);
    let theme_tokens = ctx.theme_tokens.clone();
    paths
        .into_iter()
        .map(|path| {
            let base = resolve_token_value(ctx, &path, 0, base);
            let themes = theme_tokens
                .iter()
                .map(|(name, tree)| (name.clone(), resolve_token_value(ctx, &path, 0, tree)))
                .collect();
            TokenInfo {
                path: path.join("."),
                base,
                themes,
            }
        })
        .collect()
}
fn icon_value_is_static(value: &TVal) -> bool {
    !matches!(value, TVal::Param(_) | TVal::Prop(_) | TVal::TupleDyn(_))
}

fn icon_node_is_static(node: &CNode) -> bool {
    node.attrs
        .iter()
        .all(|attr| attr.id != at::KEYS && icon_value_is_static(&attr.val))
        && node.content.as_ref().is_none_or(icon_value_is_static)
        && node.patches.is_empty()
        && node.animate.is_none()
        && node.transition.is_none()
        && node.act.is_none()
        && node.field.is_none()
        && node.submit.is_none()
        && node.press.is_none()
        && node.context.is_none()
        && node.dblclick.is_none()
        && node.drag.is_none()
        && node.drop.is_none()
        && node.resize.is_none()
        && node.pointer_move.is_none()
        && node.pointer_up.is_none()
        && node.drag_update.is_none()
        && node.drag_end.is_none()
        && node.hole.is_none()
        && node.children.iter().all(icon_node_is_static)
}

fn compile_icons(ctx: &mut Ctx) -> Vec<CIcon> {
    let declarations = ctx.doc.icons.clone();
    let mut first_lines: HashMap<String, u32> = HashMap::new();
    let mut icons = Vec::new();
    for declaration in declarations {
        if let Some(first) = first_lines.get(&declaration.name) {
            ctx.error(
                "icon-dup",
                format!(
                    "icon '{}' is already declared at line {first}",
                    declaration.name
                ),
                declaration.line,
            );
            continue;
        }
        first_lines.insert(declaration.name.clone(), declaration.line);

        let tokens = ctx.tokens;
        let viewbox = match resolve_value(
            ctx,
            &declaration.viewbox,
            &Scope::new(),
            declaration.line,
            tokens,
        ) {
            RVal::Num(value) if value.is_finite() && value > 0.0 => value,
            _ => {
                ctx.error(
                    "icon-body",
                    format!(
                        "icon '{}' viewbox must be a positive number",
                        declaration.name
                    ),
                    declaration.line,
                );
                continue;
            }
        };
        if declaration.body.is_empty()
            || declaration.body.iter().any(|item| {
                !matches!(
                    item,
                    Item::Node(node)
                        if node.name == "path"
                            && node.children.is_empty()
                            && node.id.is_none()
                )
            })
        {
            ctx.error(
                "icon-body",
                format!(
                    "icon '{}' body must contain one or more static path nodes",
                    declaration.name
                ),
                declaration.line,
            );
            continue;
        }

        let authored = ANode {
            name: "group".into(),
            id: None,
            args: Vec::new(),
            attrs: Vec::new(),
            flags: Vec::new(),
            children: declaration.body.clone(),
            line: declaration.line,
        };
        let saved_variants = std::mem::take(&mut ctx.variants);
        ctx.icon_depth = ctx.icon_depth.wrapping_add(1);
        let keys = Rc::new(RefCell::new(Keys::default()));
        let mut expanded = expand_node(
            ctx,
            &authored,
            &Scope::new(),
            0,
            &format!("@icon:{}", declaration.name),
            &keys,
            None,
        );
        ctx.icon_depth = ctx.icon_depth.wrapping_sub(1);
        ctx.variants = saved_variants;
        let Some(root) = expanded.pop() else {
            continue;
        };
        if !icon_node_is_static(&root) {
            ctx.error(
                "icon-body",
                format!(
                    "icon '{}' paths must use only static values and `current` paint",
                    declaration.name
                ),
                declaration.line,
            );
            continue;
        }
        icons.push(CIcon {
            name: declaration.name,
            viewbox,
            root,
        });
    }
    icons
}

fn collect_static_scene_keys(node: &CNode, keys: &mut BTreeSet<String>, synthetic: bool) {
    if !synthetic {
        keys.insert(node.key.clone());
    }
    let children_are_synthetic = synthetic || node.kind == nk::EACH;
    for child in &node.children {
        collect_static_scene_keys(child, keys, children_are_synthetic);
    }
    for patch in &node.patches {
        for child in &patch.children {
            collect_static_scene_keys(child, keys, children_are_synthetic);
        }
    }
}

fn semantic_attr<'a>(node: &'a CNode, patch: Option<&'a CPatch>, id: u16) -> Option<&'a TVal> {
    patch
        .and_then(|patch| patch.attrs.iter().find(|attr| attr.id == id))
        .or_else(|| node.attrs.iter().find(|attr| attr.id == id))
        .map(|attr| &attr.val)
}

fn semantic_num(node: &CNode, patch: Option<&CPatch>, id: u16) -> Option<f64> {
    match semantic_attr(node, patch, id) {
        Some(TVal::Num(value)) => Some(*value),
        _ => None,
    }
}

fn validate_semantic_values(
    ctx: &mut Ctx,
    node: &CNode,
    patch: Option<&CPatch>,
    keys: &BTreeSet<String>,
) {
    let line = patch.map_or(node.line, |patch| patch.line);
    let minimum = semantic_num(node, patch, at::VALUE_MIN);
    let maximum = semantic_num(node, patch, at::VALUE_MAX);
    let current = semantic_num(node, patch, at::VALUE_NOW);
    if minimum.zip(maximum).is_some_and(|(min, max)| min > max) {
        ctx.error(
            "a11y-range",
            "value-min must not exceed value-max".into(),
            line,
        );
    }
    if current.zip(minimum).is_some_and(|(now, min)| now < min)
        || current.zip(maximum).is_some_and(|(now, max)| now > max)
    {
        ctx.error(
            "a11y-range",
            "value-now must be within value-min and value-max".into(),
            line,
        );
    }
    if let (Some(position), Some(size)) = (
        semantic_num(node, patch, at::POS_IN_SET),
        semantic_num(node, patch, at::SET_SIZE),
    ) && size != -1.0
        && position > size
    {
        ctx.error(
            "a11y-range",
            "pos-in-set must not exceed set-size".into(),
            line,
        );
    }

    for (id, name) in [
        (at::ACTIVE_DESCENDANT, "active-descendant"),
        (at::CONTROLS, "controls"),
    ] {
        let Some(TVal::Str(target)) = semantic_attr(node, patch, id) else {
            continue;
        };
        if target.is_empty() {
            continue;
        }
        if !keys.contains(target) {
            ctx.error(
                "a11y-key",
                format!("{name} target '{target}' is not a static scene key"),
                line,
            );
        } else if id == at::ACTIVE_DESCENDANT && !target.starts_with(&format!("{}/", node.key)) {
            ctx.error(
                "a11y-key",
                format!(
                    "active-descendant target '{target}' is not a descendant of '{}'",
                    node.key
                ),
                line,
            );
        }
    }
}

fn validate_semantic_tree(ctx: &mut Ctx, node: &CNode, keys: &BTreeSet<String>) {
    validate_semantic_values(ctx, node, None, keys);
    for patch in &node.patches {
        validate_semantic_values(ctx, node, Some(patch), keys);
    }
    for child in &node.children {
        validate_semantic_tree(ctx, child, keys);
    }
    for patch in &node.patches {
        for child in &patch.children {
            validate_semantic_tree(ctx, child, keys);
        }
    }
}
fn has_attach_attrs(attrs: &[AttrE]) -> bool {
    attrs
        .iter()
        .any(|attr| matches!(attr.id, at::ATTACH | at::GRAVITY | at::COLLIDE))
}

fn validate_attach_context(ctx: &mut Ctx, node: &CNode, parent_kind: Option<u8>) {
    let has_attach = has_attach_attrs(&node.attrs)
        || node
            .patches
            .iter()
            .any(|patch| has_attach_attrs(&patch.attrs));
    if has_attach && !matches!(parent_kind, Some(nk::STACK | nk::CANVAS)) {
        ctx.error(
            "attach-ctx",
            "attach, gravity, and collide are valid only on direct children of stack or canvas"
                .into(),
            node.line,
        );
    }
    for child in &node.children {
        validate_attach_context(ctx, child, Some(node.kind));
    }
    for patch in &node.patches {
        for child in &patch.children {
            validate_attach_context(ctx, child, Some(node.kind));
        }
    }
}

fn sticky_line(node: &CNode) -> Option<u32> {
    if node.flags & fl::STICKY != 0 {
        return Some(node.line);
    }
    node.patches
        .iter()
        .find(|patch| patch.flag_mask & fl::STICKY != 0)
        .map(|patch| patch.line)
}

fn validate_sticky_context(ctx: &mut Ctx, node: &CNode, parent_flags: Option<u16>) {
    if let Some(line) = sticky_line(node)
        && parent_flags.is_none_or(|flags| flags & fl::SCROLL == 0)
    {
        ctx.error(
            "sticky-ctx",
            "sticky is valid only on a direct child of a main-axis scroll container".into(),
            line,
        );
    }
    for child in &node.children {
        validate_sticky_context(ctx, child, Some(node.flags));
    }
    for patch in &node.patches {
        for child in &patch.children {
            validate_sticky_context(ctx, child, Some(node.flags));
        }
    }
}

fn validate_divider_context(ctx: &mut Ctx, node: &CNode, valid_position: bool) {
    if node.kind == nk::DIVIDER && !valid_position {
        ctx.error(
            "divider-ctx",
            "divider must be a non-first, non-last direct child of row or col".into(),
            node.line,
        );
    }
    let child_count = node.children.len();
    for (index, child) in node.children.iter().enumerate() {
        let middle = matches!(node.kind, nk::ROW | nk::COL)
            && index > 0
            && index.wrapping_add(1) < child_count;
        validate_divider_context(ctx, child, middle);
    }
    for patch in &node.patches {
        for child in &patch.children {
            // Conditional child insertion cannot guarantee both adjacent panes.
            validate_divider_context(ctx, child, false);
        }
    }
}

/// Expand a parsed document into emission-ready trees and tables.
pub fn expand(doc: &Document, diags: &mut Diagnostics) -> Expanded {
    // Named themes are complete token tables: each starts from authored base,
    // then every declaration for that name merges in document order.
    let base_tokens = &doc.tokens;
    let themes = collect_themes(doc);
    let mut theme_tokens: Vec<(String, TokenTree)> = themes
        .iter()
        .map(|name| (name.clone(), doc.tokens.clone()))
        .collect();
    for (condition, overrides, _) in &doc.topwhens {
        let Cond::Theme(name) = condition else {
            continue;
        };
        if let Some((_, tree)) = theme_tokens
            .iter_mut()
            .find(|(candidate, _)| candidate == name)
        {
            tree.deep_merge(overrides);
        }
    }
    let mut ctx = Ctx {
        doc,
        diags,
        tokens: base_tokens,
        variants: vec![],
        theme_tokens,
        params: vec![],
        anim_names: doc.anims.iter().map(|a| a.name.clone()).collect(),
        anim_content: BTreeSet::new(),
        seen_ids: HashMap::new(),
        holes: HashMap::new(),
        signals: vec![],
        field_sync_warnings: BTreeSet::new(),
        images: vec![],
        layout_axes: vec![],
        font_families: BTreeSet::new(),
        font_weights: BTreeSet::new(),
        each_depth: 0,
        prop_fields: vec![],
        each_schemas: vec![],
        list_schemas: vec![],
        icon_depth: 0,
        quiet: 0,
    };

    // Params first: `when` conditions and `param.` refs need them.
    for decl in &doc.params {
        if let Some(first) = ctx.params.iter().find(|p| p.name == decl.name) {
            let msg = format!(
                "duplicate param '{}' (first declared at line {}; first declaration wins)",
                decl.name, first.line
            );
            ctx.warn("dup-param", msg, decl.line);
            continue;
        }
        ctx.params.push(ParamInfo {
            name: decl.name.clone(),
            ty: decl.ty.clone(),
            enum_syms: decl.enum_syms.clone(),
            default: scalar_zero(&decl.ty, &decl.enum_syms, false),
            list: None,
            line: decl.line,
        });
    }
    for decl in &doc.params {
        if let Some(ix) = ctx
            .params
            .iter()
            .position(|p| p.name == decl.name && p.line == decl.line)
        {
            match &decl.ty {
                ParamType::List(schema) => {
                    if let Some((row, default)) = compile_list_info(&mut ctx, decl, schema) {
                        ctx.params[ix].list = Some(row);
                        ctx.params[ix].default = default;
                    }
                }
                _ => {
                    let tv = check_param_default(&mut ctx, decl);
                    ctx.params[ix].default = tv;
                }
            }
        }
    }

    // Non-theme top-level token conditions retain rule-10 site expansion.
    // Named themes use the runtime token table instead.
    let mut variants: Vec<(CondSpec, TokenTree)> = Vec::new();
    for (cond, overrides, line) in &doc.topwhens {
        if matches!(cond, Cond::Theme(_)) {
            continue;
        }
        match eval_cond(&mut ctx, cond, &Scope::new(), *line) {
            CondEval::Defer(spec) => {
                let mut merged = doc.tokens.clone();
                merged.deep_merge(overrides);
                variants.push((spec, merged));
            }
            CondEval::Bool(_) => {}
        }
    }
    ctx.variants = variants;

    let icons = compile_icons(&mut ctx);
    shadow_warns(&mut ctx, doc);

    // Animation values retain token identity just like ordinary attributes.
    let mut anims = Vec::new();
    for anim in &doc.anims {
        let mut stops = Vec::new();
        for (pos, attrs) in &anim.stops {
            let mut sink = Sink {
                patch_ctx: true,
                keyframe_ctx: true,
                ..Default::default()
            };
            for (key, value) in attrs {
                let resolved =
                    resolve_value(&mut ctx, value, &Scope::new(), anim.line, base_tokens);
                apply_attr(&mut ctx, &mut sink, key, &resolved, anim.line);
            }
            if sink.get(at::CONTENT).is_some() {
                ctx.anim_content.insert(anim.name.clone());
            }
            stops.push((*pos, sink.entries));
        }
        anims.push(RAnim {
            name: anim.name.clone(),
            stops,
        });
    }

    let root_keys: KeysRc = Rc::new(RefCell::new(Keys::default()));
    let mut roots = Vec::new();
    for node in &doc.roots {
        roots.extend(expand_node(
            &mut ctx,
            node,
            &Scope::new(),
            0,
            "",
            &root_keys,
            None,
        ));
    }

    for root in &roots {
        validate_attach_context(&mut ctx, root, None);
        validate_divider_context(&mut ctx, root, false);
        validate_sticky_context(&mut ctx, root, None);
    }
    let mut static_scene_keys = BTreeSet::new();
    for root in &roots {
        collect_static_scene_keys(root, &mut static_scene_keys, false);
    }
    for root in &roots {
        validate_semantic_tree(&mut ctx, root, &static_scene_keys);
    }

    let tokens = collect_token_infos(&mut ctx, base_tokens);
    Expanded {
        roots,
        params: ctx.params,
        anims,
        list_schemas: ctx.list_schemas,
        icons,
        themes,
        tokens,
        images: ctx.images,
        font_families: ctx.font_families,
        font_weights: ctx.font_weights,
    }
}
