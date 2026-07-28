//! Raw AST for the slab syntax (SPEC §2). No semantic resolution here.

/// Scalar values as parsed; the compiler resolves these against tokens,
/// component props, and params.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    /// `40%` -> Pct(40.0)
    Pct(f64),
    Str(String),
    /// CSS-ish color string: `#0e1116`, `oklch(72% 0.16 250)`, `linear(...)`.
    Color(String),
    /// Dotted reference: `color.bg`, `param.title`.
    Ref(Vec<String>),
    /// Bare ident: keyword or component prop.
    Kw(String),
    /// `fill` / `fill:2`.
    Fill(f64),
    /// Comma tuple of scalars.
    Tup(Vec<Value>),
    /// A typed key-to-signal map, e.g. `Escape:close,F2:rename`.
    KeyMap(Vec<(Value, Value)>),
    /// A nested list literal used by a list-item field default.
    List(Vec<ListItem>),
    /// `list(Def)` type annotation on an exported-def field.
    ListSchema(String),
}

/// A state, viewport, or named-theme condition attached to a `when` block.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    Ident {
        name: String,
        neg: bool,
    },
    Cmp {
        axis: CmpAxis,
        op: CmpOp,
        num: f64,
    },
    /// Selects a compiler-declared theme by host-controlled name.
    Theme(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpAxis {
    W,
    H,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn as_str(self) -> &'static str {
        match self {
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
}

/// An item inside a `{ }` block.
#[derive(Debug, Clone)]
pub enum Item {
    Node(ANode),
    Each(AEach),
    /// Bare string child (a text run inside `para`).
    Text(String, u32),
    When(AWhen),
}

/// Runtime list expansion (`each param.NAME`).
#[derive(Debug, Clone)]
pub struct AEach {
    /// The list parameter name, without the required `param.` prefix.
    pub param: String,
    /// True when the target is an enclosing item property rather than a root param.
    pub prop: bool,
    pub id: Option<String>,
    pub attrs: Vec<(String, Value)>,
    /// Flags authored on the expansion; currently only `virtual` is meaningful.
    pub flags: Vec<String>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct ANode {
    pub name: String,
    pub id: Option<String>,
    pub args: Vec<Value>,
    /// Insertion-ordered; a repeated key replaces the value in place.
    pub attrs: Vec<(String, Value)>,
    pub flags: Vec<String>,
    pub children: Vec<Item>,
    pub line: u32,
}

impl ANode {
    pub fn attr(&self, key: &str) -> Option<&Value> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

#[derive(Debug, Clone)]
pub struct AWhen {
    pub cond: Cond,
    pub attrs: Vec<(String, Value)>,
    pub flags: Vec<String>,
    pub children: Vec<Item>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct ADef {
    pub name: String,
    pub params: Vec<(String, Option<Value>)>,
    pub export: bool,
    pub body: Vec<Item>,
    pub line: u32,
}
/// A named vector icon declaration with a square design box.
#[derive(Debug, Clone)]
pub struct AIcon {
    /// Declaration name used by `icon NAME` nodes.
    pub name: String,
    /// Positive square design-box extent.
    pub viewbox: Value,
    /// Authored static path children.
    pub body: Vec<Item>,
    /// One-based source line of the declaration.
    pub line: u32,
}

/// Declared parameter type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamType {
    Text,
    Num,
    Pct,
    Color,
    Bool,
    Enum,
    /// A runtime list whose item schema is the named exported definition.
    List(String),
}

impl ParamType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ParamType::Text => "text",
            ParamType::Num => "num",
            ParamType::Pct => "pct",
            ParamType::Color => "color",
            ParamType::Bool => "bool",
            ParamType::Enum => "enum",
            ParamType::List(_) => "list",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub name: String,
    pub attrs: Vec<(String, Value)>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamDefault {
    Scalar(Value),
    List(Vec<ListItem>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub ty: ParamType,
    /// Enum member idents (`enum(compact, cozy)`); empty for other types.
    pub enum_syms: Vec<String>,
    pub default: ParamDefault,
    pub line: u32,
}

/// Nested token tree; entries keep document order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenTree(pub Vec<(String, TokenEntry)>);

#[derive(Debug, Clone, PartialEq)]
pub enum TokenEntry {
    Group(TokenTree),
    Value(Value),
}

impl TokenTree {
    pub fn get(&self, key: &str) -> Option<&TokenEntry> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn set(&mut self, key: &str, entry: TokenEntry) {
        if let Some(slot) = self.0.iter_mut().find(|(k, _)| k == key) {
            slot.1 = entry;
        } else {
            self.0.push((key.to_string(), entry));
        }
    }

    /// Deep merge `src` into `self`: groups merge recursively, leaves replace.
    /// Returns the paths of leaves that replaced an existing leaf (dup-token).
    pub fn deep_merge(&mut self, src: &TokenTree) -> Vec<String> {
        let mut dups = Vec::new();
        self.merge_inner(src, &mut String::new(), &mut dups);
        dups
    }

    fn merge_inner(&mut self, src: &TokenTree, prefix: &mut String, dups: &mut Vec<String>) {
        for (k, v) in &src.0 {
            let existing = self.get(k);
            match (existing, v) {
                (Some(TokenEntry::Group(_)), TokenEntry::Group(g)) => {
                    let keep = prefix.len();
                    if !prefix.is_empty() {
                        prefix.push('.');
                    }
                    prefix.push_str(k);
                    if let Some(TokenEntry::Group(dst)) =
                        self.0.iter_mut().find(|(dk, _)| dk == k).map(|(_, e)| e)
                    {
                        dst.merge_inner(g, prefix, dups);
                    }
                    prefix.truncate(keep);
                }
                (Some(_), _) => {
                    let path = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    dups.push(path);
                    self.set(k, v.clone());
                }
                (None, _) => self.set(k, v.clone()),
            }
        }
    }
}

/// Anim keyframes: sorted `(position 0..=1, attrs)` stops.
#[derive(Debug, Clone)]
pub struct AAnim {
    pub name: String,
    pub stops: Vec<(f64, Vec<(String, Value)>)>,
    pub line: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    pub tokens: TokenTree,
    /// Document order; later defs with the same name shadow earlier ones.
    pub defs: Vec<ADef>,
    pub params: Vec<ParamDecl>,
    /// Named vector icon declarations in source order.
    pub icons: Vec<AIcon>,
    pub roots: Vec<ANode>,
    pub topwhens: Vec<(Cond, TokenTree, u32)>,
    pub anims: Vec<AAnim>,
}

impl Document {
    pub fn def(&self, name: &str) -> Option<&ADef> {
        self.defs.iter().rev().find(|d| d.name == name)
    }
}
