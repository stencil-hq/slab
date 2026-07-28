//! Per-document index for tokens, definitions, and completion context.
//!
//! It stores located tokens, token trees, symbols, color swatches, and a block
//! map. This direct port of the research `_Indexer` performs a lightweight
//! structural walk over the shared lexer's token stream, recovering columns by
//! line scanning because the lexer keeps line numbers only. All columns are
//! `char` indices into the line.

use std::collections::{BTreeMap, HashMap};

use slab_compile::color::{Rgba, parse_rgba};
use slab_syntax::{
	diag::Diagnostics,
	lex::{TokKind, lex},
};

use crate::vocab;

/// Lexer token with computed source columns (char indices).
#[derive(Debug, Clone)]
pub struct LTok {
	pub kind: TokKind,
	pub text: String,
	/// 0-based line.
	pub line: usize,
	pub col:  usize,
	pub end:  usize,
}

/// One documentSymbol entry (hierarchical).
#[derive(Debug, Clone)]
pub struct Sym {
	pub name:     String,
	pub kind:     u32,
	pub line:     usize,
	pub col:      usize,
	pub end:      usize,
	pub detail:   String,
	pub children: Vec<Self>,
}

impl Sym {
	fn new(name: impl Into<String>, kind: u32, line: usize, col: usize, end: usize) -> Self {
		Self { name: name.into(), kind, line, col, end, detail: String::new(), children: Vec::new() }
	}
}

/// Nested token tree: group or leaf value string.
#[derive(Debug, Clone)]
pub enum TNode {
	Group(BTreeMap<String, Self>),
	Leaf(String),
}

/// Block record: (type, start line, start col, end line, end col).
#[derive(Debug, Clone)]
pub struct Block {
	pub btype: &'static str,
	pub sline: usize,
	pub scol:  usize,
	pub eline: usize,
	pub ecol:  usize,
}

fn node_block_type(name: &str) -> &'static str {
	match name {
		"box" => "box",
		"row" => "row",
		"col" => "col",
		"wrap" => "wrap",
		"grid" => "grid",
		"stack" => "stack",
		"canvas" => "canvas",
		"para" => "para",
		"group" => "group",
		_ => "node",
	}
}

/// Definition site: (line, col, end, detail string).
pub type DefSite = (usize, usize, usize, String);

/// Value site: (value string, line, col, end).
pub type ValSite = (String, usize, usize, usize);

#[derive(Debug, Default, Clone)]
pub struct Index {
	pub lines:       Vec<String>,
	pub toks:        Vec<LTok>,
	/// Token indices per line, in source order.
	pub by_line:     HashMap<usize, Vec<usize>>,
	/// Component name -> (line, col, end, signature).
	pub defs:        BTreeMap<String, DefSite>,
	/// Anim name -> (line, col, end).
	pub anims:       BTreeMap<String, (usize, usize, usize)>,
	/// Icon declaration name -> (line, col, end, declaration signature).
	pub icons:       BTreeMap<String, DefSite>,
	/// List-valued exported-def field name -> element schema (for nested
	/// `each`).
	pub list_fields: BTreeMap<String, String>,
	/// Dotted token path -> (value, line, col, end); first definition wins.
	pub token_paths: BTreeMap<String, ValSite>,
	pub token_tree:  BTreeMap<String, TNode>,
	pub group_paths: Vec<String>,
	/// `param.NAME` -> (typed declaration tail, line, col, end).
	pub param_paths: BTreeMap<String, ValSite>,
	pub colors:      Vec<(usize, usize, usize, Rgba)>,
	pub symbols:     Vec<Sym>,
	pub blocks:      Vec<Block>,
}

/// Re-run the shared lexer and assign each token a column by line scanning.
fn locate(src: &str, lines: &[String]) -> Vec<LTok> {
	let mut diags = Diagnostics::new();
	let toks = lex(src, &mut diags);
	let chars: Vec<Vec<char>> = lines.iter().map(|l| l.chars().collect()).collect();
	let mut out: Vec<LTok> = Vec::new();
	let mut cursors: HashMap<usize, usize> = HashMap::new();
	for t in &toks {
		if t.kind == TokKind::Eof {
			break;
		}
		let ln = (t.line as usize).saturating_sub(1);
		let line: &[char] = if ln < chars.len() { &chars[ln] } else { &[] };
		let cur = cursors.get(&ln).copied().unwrap_or(0);
		if t.kind == TokKind::Str {
			if t.text.contains('\n') {
				// multi-line string: degrade to line start
				out.push(LTok { kind: t.kind, text: t.text.clone(), line: ln, col: 0, end: 0 });
				continue;
			}
			let col = char_find(line, &['"'], cur).unwrap_or(cur);
			let mut j = col + 1;
			while j < line.len() {
				if line[j] == '\\' {
					j += 2;
				} else if line[j] == '"' {
					j += 1;
					break;
				} else {
					j += 1;
				}
			}
			let end = j.min(line.len());
			out.push(LTok { kind: t.kind, text: t.text.clone(), line: ln, col, end });
			cursors.insert(ln, end);
			continue;
		}
		if t.kind == TokKind::Nl {
			// The shared lexer folds `;` into NL with text "\n"; recover the
			// mid-line separator when a `;` still lies ahead of the cursor.
			if let Some(pos) = char_find(line, &[';'], cur) {
				out.push(LTok { kind: t.kind, text: ";".into(), line: ln, col: pos, end: pos + 1 });
				cursors.insert(ln, pos + 1);
			} else {
				out.push(LTok {
					kind: t.kind,
					text: "\n".into(),
					line: ln,
					col:  line.len(),
					end:  line.len(),
				});
				cursors.remove(&ln);
			}
			continue;
		}
		let lex_text: String = if t.kind == TokKind::Hash {
			format!("#{}", t.text)
		} else {
			t.text.clone()
		};
		let needle: Vec<char> = lex_text.chars().collect();
		let pos = char_find(line, &needle, cur)
			.or_else(|| char_find(line, &needle, 0))
			.unwrap_or_else(|| cur.min(line.len()));
		out.push(LTok {
			kind: t.kind,
			text: t.text.clone(),
			line: ln,
			col:  pos,
			end:  pos + needle.len(),
		});
		cursors.insert(ln, pos + needle.len());
	}
	out
}

/// Find `needle` in `hay` starting at char index `from`.
fn char_find(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
	if needle.is_empty() {
		return Some(from.min(hay.len()));
	}
	let mut i = from;
	while i + needle.len() <= hay.len() {
		if hay[i..i + needle.len()] == *needle {
			return Some(i);
		}
		i += 1;
	}
	None
}

/// Rebuild a compact source-ish string from value tokens.
pub fn fmt_toks(toks: &[&LTok]) -> String {
	let mut s = String::new();
	for t in toks {
		let lex_text = match t.kind {
			TokKind::Str => format!("\"{}\"", t.text),
			TokKind::Hash => format!("#{}", t.text),
			_ => t.text.clone(),
		};
		if !s.is_empty()
			&& !matches!(s.chars().last(), Some('(' | '[' | ',' | ':'))
			&& !matches!(lex_text.as_str(), "(" | ")" | "[" | "]" | "," | ":")
		{
			s.push(' ');
		}
		s.push_str(&lex_text);
	}
	s
}

/// Lightweight structural walk over located tokens (columns included).
struct Indexer {
	ix: Index,
	i:  usize,
}

impl Indexer {
	fn new(src: &str) -> Self {
		let lines: Vec<String> = src.split('\n').map(str::to_string).collect();
		let toks = locate(src, &lines);
		let mut by_line: HashMap<usize, Vec<usize>> = HashMap::new();
		for (k, t) in toks.iter().enumerate() {
			by_line.entry(t.line).or_default().push(k);
		}
		Self { ix: Index { lines, toks, by_line, ..Index::default() }, i: 0 }
	}

	// -- token helpers
	fn peek(&self, k: usize) -> Option<&LTok> {
		self.ix.toks.get(self.i + k)
	}

	fn prev_tok(&self) -> Option<&LTok> {
		if self.i > 0 {
			self.ix.toks.get(self.i - 1)
		} else {
			None
		}
	}

	fn next(&mut self) -> Option<LTok> {
		let t = self.ix.toks.get(self.i).cloned();
		if t.is_some() {
			self.i += 1;
		}
		t
	}

	fn at(&self, kind: TokKind) -> bool {
		self.peek(0).is_some_and(|t| t.kind == kind)
	}

	fn at_k(&self, kind: TokKind, k: usize) -> bool {
		self.peek(k).is_some_and(|t| t.kind == kind)
	}

	fn at_id(&self, text: &str) -> bool {
		self
			.peek(0)
			.is_some_and(|t| t.kind == TokKind::Id && t.text == text)
	}

	fn icon_decl_ahead(&self) -> bool {
		if !self.at_k(TokKind::Id, 1) {
			return false;
		}
		let mut k = 2;
		while self.at_k(TokKind::Nl, k) {
			k += 1;
		}
		self.peek(k).is_some_and(|token| {
			token.kind == TokKind::Lb
				|| (token.kind == TokKind::Id
					&& token.text == "viewbox"
					&& self.at_k(TokKind::Eq, k + 1))
		})
	}

	// -- entry
	fn run(mut self) -> Index {
		while self.peek(0).is_some() {
			let mut syms = std::mem::take(&mut self.ix.symbols);
			self.stmt(false, &mut syms, true);
			self.ix.symbols = syms;
		}
		self.ix
	}

	fn open_block(&mut self, btype: &'static str, lb: &LTok) -> usize {
		self.ix.blocks.push(Block {
			btype,
			sline: lb.line,
			scol: lb.col + 1,
			eline: 1 << 30,
			ecol: 1 << 30,
		});
		self.ix.blocks.len() - 1
	}

	fn close_block(&mut self, rec: usize, rb: Option<(usize, usize)>) {
		if let Some((line, col)) = rb {
			self.ix.blocks[rec].eline = line;
			self.ix.blocks[rec].ecol = col;
		}
	}

	fn block(&mut self, btype: &'static str, in_def: bool, syms: &mut Vec<Sym>) {
		let lb = self.next().expect("block called at LB");
		let rec = self.open_block(btype, &lb);
		while self.peek(0).is_some() && !self.at(TokKind::Rb) {
			self.stmt(in_def, syms, false);
		}
		let rb = self.peek(0).map(|t| (t.line, t.col));
		self.close_block(rec, rb);
		if self.at(TokKind::Rb) {
			self.next();
		}
	}

	// -- statements
	fn stmt(&mut self, in_def: bool, syms: &mut Vec<Sym>, top: bool) {
		if self.at(TokKind::Nl) {
			self.next();
			return;
		}
		if self.at(TokKind::Rb) {
			// stray; caller consumes matched ones
			self.next();
			return;
		}
		let Some(t) = self.peek(0).cloned() else {
			return;
		};
		if top && t.kind == TokKind::Id && t.text == "import" && self.at_k(TokKind::Str, 1) {
			self.next();
			self.next();
			return;
		}
		if t.kind == TokKind::Id && t.text == "tokens" && self.at_k(TokKind::Lb, 1) {
			self.next();
			let mut sym = Sym::new("tokens", 3, t.line, t.col, t.end);
			let lb = self.next().expect("tokens LB");
			let rec = self.open_block("tokens", &lb);
			let mut tree = std::mem::take(&mut self.ix.token_tree);
			self.tokens_block(&[], &mut tree, &mut sym.children);
			self.ix.token_tree = tree;
			let rb = self
				.prev_tok()
				.filter(|p| p.kind == TokKind::Rb)
				.map(|p| (p.line, p.col));
			self.close_block(rec, rb);
			syms.push(sym);
			return;
		}
		if t.kind == TokKind::Id
			&& t.text == "params"
			&& (self.at_k(TokKind::Lb, 1) || (self.at_k(TokKind::Id, 1) && self.at_k(TokKind::Lb, 2)))
		{
			self.next();
			let group = self
				.at(TokKind::Id)
				.then(|| self.next().expect("param group"));
			let symbol_name = group
				.as_ref()
				.map_or_else(|| "params".to_string(), |group| format!("params {}", group.text));
			let symbol_end = group.as_ref().map_or(t.end, |group| group.end);
			let mut sym = Sym::new(symbol_name, 3, t.line, t.col, symbol_end);
			let lb = self.next().expect("params LB");
			let rec = self.open_block("params", &lb);
			self.params_block(group.as_ref().map(|group| group.text.as_str()), &mut sym.children);
			let rb = self
				.prev_tok()
				.filter(|p| p.kind == TokKind::Rb)
				.map(|p| (p.line, p.col));
			self.close_block(rec, rb);
			syms.push(sym);
			return;
		}
		if top && t.kind == TokKind::Id && t.text == "icon" && self.icon_decl_ahead() {
			self.next();
			let name = self.next().expect("icon name");
			let mut header: Vec<LTok> = Vec::new();
			while self.peek(0).is_some() && !self.at(TokKind::Lb) && !self.at(TokKind::Rb) {
				let token = self.next().expect("icon header token");
				if token.kind != TokKind::Nl {
					header.push(token);
				}
			}
			let tail = fmt_toks(&header.iter().collect::<Vec<_>>())
				.replace(" = ", "=")
				.replace("= ", "=")
				.replace(" =", "=");
			let detail = if tail.is_empty() {
				format!("icon {}", name.text)
			} else {
				format!("icon {} {tail}", name.text)
			};
			self
				.ix
				.icons
				.insert(name.text.clone(), (name.line, name.col, name.end, detail.clone()));
			let mut sym = Sym::new(name.text.clone(), 14, name.line, name.col, name.end);
			sym.detail = detail;
			if self.at(TokKind::Lb) {
				self.block("icon", false, &mut sym.children);
			}
			syms.push(sym);
			return;
		}
		if t.kind == TokKind::Id && t.text == "def" && self.at_k(TokKind::Id, 1) {
			self.next();
			let name = self.next().expect("def name");
			let (mut sig, list_fields) = self.def_sig(&name);
			for (field, schema) in list_fields {
				self.ix.list_fields.entry(field).or_insert(schema);
			}
			if self.at_id("export") {
				self.next();
				sig.push_str(" export");
			}
			self
				.ix
				.defs
				.insert(name.text.clone(), (name.line, name.col, name.end, sig.clone()));
			let mut sym = Sym::new(name.text.clone(), 12, name.line, name.col, name.end);
			sym.detail = sig;
			if self.at(TokKind::Lb) {
				self.block("def", true, &mut sym.children);
			}
			syms.push(sym);
			return;
		}
		if t.kind == TokKind::Id && t.text == "anim" && self.at_k(TokKind::Id, 1) {
			self.next();
			let name = self.next().expect("anim name");
			self
				.ix
				.anims
				.insert(name.text.clone(), (name.line, name.col, name.end));
			let mut sym = Sym::new(name.text.clone(), 24, name.line, name.col, name.end);
			sym.detail = "anim".into();
			if self.at(TokKind::Lb) {
				self.block("anim", in_def, &mut sym.children);
			}
			syms.push(sym);
			return;
		}
		if t.kind == TokKind::Id && t.text == "when" {
			self.next();
			while self.peek(0).is_some()
				&& !self.at(TokKind::Lb)
				&& !self.at(TokKind::Nl)
				&& !self.at(TokKind::Rb)
			{
				self.next();
			}
			if self.at(TokKind::Lb) {
				self.block("when", in_def, syms);
			}
			return;
		}
		if t.kind == TokKind::Pct && self.at_k(TokKind::Lb, 1) {
			// anim keyframe
			self.next();
			self.block("keyframe", in_def, syms);
			return;
		}
		if t.kind == TokKind::Str {
			self.next();
			return;
		}
		if t.kind != TokKind::Id && t.kind != TokKind::Ref {
			self.next();
			return;
		}
		// node / attr statement: scan the header
		let head = self.next().expect("stmt head");
		let mut node_id: Option<LTok> = None;
		let mut first = true;
		while self.peek(0).is_some()
			&& !self.at(TokKind::Nl)
			&& !self.at(TokKind::Lb)
			&& !self.at(TokKind::Rb)
		{
			let tok = self.next().expect("header token");
			if tok.kind == TokKind::Hash {
				if first && node_id.is_none() {
					node_id = Some(tok);
				} else {
					self.add_hex(&tok);
				}
			} else if tok.kind == TokKind::Id && self.at(TokKind::Lp) {
				self.color_fn(&tok);
			}
			first = false;
		}
		let label = match &node_id {
			Some(id) => format!("{}#{}", head.text, id.text),
			None => head.text.clone(),
		};
		let want_sym = (top || node_id.is_some())
			&& head.kind == TokKind::Id
			&& vocab::lookup(vocab::ATTR_DOCS, &head.text).is_none();
		if want_sym {
			let end = node_id.as_ref().map_or(head.end, |id| id.end);
			let mut sym = Sym::new(label, 19, head.line, head.col, end);
			if self.at(TokKind::Lb) {
				self.block(node_block_type(&head.text), in_def, &mut sym.children);
			}
			syms.push(sym);
		} else if self.at(TokKind::Lb) {
			self.block(node_block_type(&head.text), in_def, syms);
		}
	}

	/// Consume the parameter list, returning its signature and List-valued
	/// fields.
	fn def_sig(&mut self, name: &LTok) -> (String, Vec<(String, String)>) {
		let mut parts: Vec<String> = Vec::new();
		if self.at(TokKind::Lp) {
			self.next();
			let mut cur: Vec<LTok> = Vec::new();
			while self.peek(0).is_some() && !self.at(TokKind::Rp) && !self.at(TokKind::Lb) {
				let tok = self.next().expect("sig token");
				if tok.kind == TokKind::Comma {
					parts.push(fmt_toks(&cur.iter().collect::<Vec<_>>()));
					cur.clear();
				} else {
					cur.push(tok);
				}
			}
			if !cur.is_empty() {
				parts.push(fmt_toks(&cur.iter().collect::<Vec<_>>()));
			}
			if self.at(TokKind::Rp) {
				self.next();
			}
		}
		let normalized: Vec<String> = parts
			.iter()
			.map(|part| {
				part
					.replace(" = ", "=")
					.replace("= ", "=")
					.replace(" =", "=")
			})
			.collect();
		let list_fields = normalized
			.iter()
			.filter_map(|part| {
				let (field, value) = part.split_once('=')?;
				let schema = value.strip_prefix("list(")?.strip_suffix(')')?;
				(!field.is_empty() && !schema.is_empty())
					.then(|| (field.to_string(), schema.to_string()))
			})
			.collect();
		(format!("def {}({})", name.text, normalized.join(", ")), list_fields)
	}

	/// Entries until the matching RB: leaves and nested groups.
	fn tokens_block(
		&mut self,
		path: &[String],
		tree: &mut BTreeMap<String, TNode>,
		syms: &mut Vec<Sym>,
	) {
		while self.peek(0).is_some() {
			if self.at(TokKind::Nl) {
				self.next();
				continue;
			}
			if self.at(TokKind::Rb) {
				self.next();
				return;
			}
			let name = self.next().expect("token entry name");
			if name.kind != TokKind::Id {
				continue;
			}
			if self.at(TokKind::Lb) {
				let mut p: Vec<String> = path.to_vec();
				p.push(name.text.clone());
				self.ix.group_paths.push(p.join("."));
				let mut sym = Sym::new(name.text.clone(), 3, name.line, name.col, name.end);
				let lb = self.next().expect("group LB");
				let rec = self.open_block("tokens", &lb);
				let mut sub = match tree.remove(&name.text) {
					Some(TNode::Group(g)) => g,
					_ => BTreeMap::new(),
				};
				self.tokens_block(&p, &mut sub, &mut sym.children);
				tree.insert(name.text.clone(), TNode::Group(sub));
				let rb = self.prev_tok().map(|p| (p.line, p.col));
				self.close_block(rec, rb);
				syms.push(sym);
			} else {
				let mut vals: Vec<LTok> = Vec::new();
				while self.peek(0).is_some() && !self.at(TokKind::Nl) && !self.at(TokKind::Rb) {
					let tok = self.next().expect("token value");
					if tok.kind == TokKind::Hash {
						self.add_hex(&tok);
					} else if tok.kind == TokKind::Id && self.at(TokKind::Lp) {
						self.color_fn(&tok);
					}
					vals.push(tok);
				}
				let value = fmt_toks(&vals.iter().collect::<Vec<_>>());
				let mut p: Vec<String> = path.to_vec();
				p.push(name.text.clone());
				self
					.ix
					.token_paths
					.entry(p.join("."))
					.or_insert_with(|| (value.clone(), name.line, name.col, name.end));
				if !matches!(tree.get(&name.text), Some(TNode::Group(_))) {
					tree.insert(name.text.clone(), TNode::Leaf(value.clone()));
				}
				let mut sym = Sym::new(name.text.clone(), 14, name.line, name.col, name.end);
				sym.detail = value;
				syms.push(sym);
			}
		}
	}

	/// `params { name type = default }` entries until the matching RB.
	///
	/// Recursive list defaults may span lines. Keep balanced parens/brackets
	/// inside the declaration so nested items never become document statements.
	fn params_block(&mut self, group: Option<&str>, syms: &mut Vec<Sym>) {
		while self.peek(0).is_some() {
			if self.at(TokKind::Nl) {
				self.next();
				continue;
			}
			if self.at(TokKind::Rb) {
				return;
			}
			let name = self.next().expect("param name");
			if name.kind != TokKind::Id {
				continue;
			}
			let mut vals: Vec<LTok> = Vec::new();
			let mut paren_depth = 0_u32;
			let mut list_depth = 0_u32;
			while self.peek(0).is_some() && !self.at(TokKind::Rb) {
				if self.at(TokKind::Nl) && paren_depth == 0 && list_depth == 0 {
					break;
				}
				let tok = self.next().expect("param value");
				match tok.kind {
					TokKind::Lp => paren_depth += 1,
					TokKind::Rp => paren_depth = paren_depth.saturating_sub(1),
					TokKind::Ls => list_depth += 1,
					TokKind::Rs => list_depth = list_depth.saturating_sub(1),
					TokKind::Hash => self.add_hex(&tok),
					TokKind::Id
						if self.at(TokKind::Lp)
							&& (vocab::COLOR_FNS.contains(&tok.text.as_str())
								|| vocab::PAINT_FNS.contains(&tok.text.as_str())) =>
					{
						self.color_fn(&tok);
					},
					_ => {},
				}
				if tok.kind != TokKind::Nl {
					vals.push(tok);
				}
			}
			let value = fmt_toks(&vals.iter().collect::<Vec<_>>());
			let path = group.map_or_else(
				|| format!("param.{}", name.text),
				|group| format!("param.{group}.{}", name.text),
			);
			self
				.ix
				.param_paths
				.entry(path)
				.or_insert_with(|| (value.clone(), name.line, name.col, name.end));
			let mut sym = Sym::new(name.text.clone(), 13, name.line, name.col, name.end);
			sym.detail = value;
			syms.push(sym);
		}
	}

	/// Record a `#hex` value-position color if it parses.
	fn add_hex(&mut self, tok: &LTok) {
		if let Some(rgba) = parse_rgba(&format!("#{}", tok.text)) {
			self.ix.colors.push((tok.line, tok.col, tok.end, rgba));
		}
	}

	/// Consume `fn(...)`; record rgb/oklch/hsl swatches and nested hexes.
	fn color_fn(&mut self, fn_tok: &LTok) {
		let mut inner: Vec<LTok> = Vec::new();
		let mut depth = 0i32;
		let mut last = fn_tok.clone();
		while self.peek(0).is_some() {
			let tok = self.next().expect("color fn token");
			last = tok.clone();
			match tok.kind {
				TokKind::Lp => depth += 1,
				TokKind::Rp => {
					depth -= 1;
					if depth == 0 {
						break;
					}
				},
				_ => {
					if tok.kind == TokKind::Hash && vocab::PAINT_FNS.contains(&fn_tok.text.as_str()) {
						self.add_hex(&tok);
					}
					inner.push(tok);
				},
			}
		}
		if vocab::COLOR_FNS.contains(&fn_tok.text.as_str())
			&& last.kind == TokKind::Rp
			&& last.line == fn_tok.line
		{
			let raw = format!(
				"{}({})",
				fn_tok.text,
				inner
					.iter()
					.filter(|t| t.kind != TokKind::Comma)
					.map(|t| if t.kind == TokKind::Hash {
						format!("#{}", t.text)
					} else {
						t.text.clone()
					})
					.collect::<Vec<_>>()
					.join(" ")
			);
			if let Some(rgba) = parse_rgba(&raw) {
				self
					.ix
					.colors
					.push((fn_tok.line, fn_tok.col, last.end, rgba));
			}
		}
	}
}

/// Index definitions, icons, list fields, tokens, params, symbols, colors, and
/// blocks.
pub fn build_index(src: &str) -> Index {
	Indexer::new(src).run()
}
