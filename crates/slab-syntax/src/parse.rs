//! Recursive-descent parser for the slab syntax (SPEC §2).
//!
//! Ported from the 0.5 reference parser plus the 1.0 additions: `params`
//! blocks and the `export` flag on defs. (`hole`, `act=`, `field=` need no
//! grammar changes.)

use crate::{
	ast::*,
	diag::Diagnostics,
	lex::{Tok, TokKind, lex, py_repr},
};

pub const FLAGS: &[&str] = &[
	"clip",
	"bleed",
	"scroll",
	"nowrap",
	"ellipsis",
	"inert",
	"focusable",
	"multiline",
	"sticky",
	"virtual",
	"drag-ghost",
	"escape-blur",
	"strike",
	"italic",
	"underline",
];

pub struct Parser<'d> {
	toks:  Vec<Tok>,
	pos:   usize,
	diags: &'d mut Diagnostics,
}

impl<'d> Parser<'d> {
	pub fn new(src: &str, diags: &'d mut Diagnostics) -> Self {
		let toks = lex(src, diags);
		Self { toks, pos: 0, diags }
	}

	// -- token helpers ------------------------------------------------------

	fn peek(&self) -> &Tok {
		&self.toks[self.pos.min(self.toks.len() - 1)]
	}

	fn peek_at(&self, k: usize) -> &Tok {
		&self.toks[(self.pos + k).min(self.toks.len() - 1)]
	}

	fn next(&mut self) -> Tok {
		let t = self.toks[self.pos].clone();
		if t.kind != TokKind::Eof {
			self.pos += 1;
		}
		t
	}

	fn at(&self, kind: TokKind) -> bool {
		self.peek().kind == kind
	}

	fn at_id(&self, text: &str) -> bool {
		let t = self.peek();
		t.kind == TokKind::Id && t.text == text
	}

	fn eat(&mut self, kind: TokKind) -> bool {
		if self.at(kind) {
			self.next();
			true
		} else {
			false
		}
	}

	fn expect(&mut self, kind: TokKind, what: &str) -> Tok {
		if self.at(kind) {
			return self.next();
		}
		let t = self.peek().clone();
		self.diags.error(
			"parse",
			format!("expected {what}, got {} {}", t.kind.name(), py_repr(&t.text)),
			t.line,
		);
		Tok { kind, text: String::new(), line: t.line, val: 0.0 }
	}

	fn skip_nl(&mut self) {
		while self.at(TokKind::Nl) {
			self.next();
		}
	}

	/// Reject an import in a braced block and recover at the next line.
	fn reject_block_import(&mut self) -> bool {
		if !self.at_id("import") {
			return false;
		}
		let line = self.next().line;
		self
			.diags
			.error("parse", "import is allowed at top level only", line);
		while !matches!(self.peek().kind, TokKind::Nl | TokKind::Rb | TokKind::Eof) {
			self.next();
		}
		true
	}

	/// Skip a braced block after its opening brace has been consumed.
	fn skip_braced_body(&mut self) {
		let mut depth = 1usize;
		while depth > 0 && !self.at(TokKind::Eof) {
			match self.next().kind {
				TokKind::Lb => depth += 1,
				TokKind::Rb => depth -= 1,
				_ => {},
			}
		}
	}

	/// Recover a run of attributes that started on new lines without `\`.
	fn recover_missing_header_continuation(&mut self) -> bool {
		let starts_attr =
			(self.at(TokKind::Id) && self.peek_at(1).kind == TokKind::Eq) || self.at(TokKind::Eq);
		if !starts_attr {
			return false;
		}
		let line = self.peek().line;
		self.diags.error_with(
			"parse",
			"node-header attribute starts on a new line",
			line,
			"did you mean to continue the previous line with `\\`?",
		);
		loop {
			while !matches!(self.peek().kind, TokKind::Nl | TokKind::Rb | TokKind::Eof) {
				self.next();
			}
			if !self.at(TokKind::Nl) {
				break;
			}
			self.next();
			let next_starts_attr =
				(self.at(TokKind::Id) && self.peek_at(1).kind == TokKind::Eq) || self.at(TokKind::Eq);
			if !next_starts_attr {
				break;
			}
		}
		true
	}

	// -- entry --------------------------------------------------------------

	pub fn parse(&mut self) -> Document {
		let mut doc = Document::default();
		self.skip_nl();
		while !self.at(TokKind::Eof) {
			let t = self.peek().clone();
			if t.kind == TokKind::Id && t.text == "import" {
				if let Some(import) = self.parse_import() {
					doc.imports.push(import);
				}
			} else if t.kind == TokKind::Id && t.text == "tokens" {
				let tk = self.parse_tokens();
				for path in doc.tokens.deep_merge(&tk) {
					self.diags.warn(
						"dup-token",
						format!("token '{path}' redefined (last definition wins)"),
						t.line,
					);
				}
			} else if t.kind == TokKind::Id && t.text == "def" {
				let d = self.parse_def();
				if let Some(first) = doc.defs.iter().find(|e| e.name == d.name) {
					self.diags.warn(
						"dup-def",
						format!(
							"component '{}' redefined (first defined at line {}; last definition wins)",
							d.name, first.line
						),
						d.line,
					);
				}
				doc.defs.push(d);
			} else if t.kind == TokKind::Id && t.text == "icon" && self.icon_decl_ahead() {
				doc.icons.push(self.parse_icon());
			} else if t.kind == TokKind::Id && t.text == "anim" {
				let a = self.parse_anim();
				doc.anims.push(a);
			} else if t.kind == TokKind::Id && t.text == "params" {
				self.parse_params(&mut doc.params);
			} else if t.kind == TokKind::Id && t.text == "when" {
				self.next();
				let cond = self.parse_cond();
				self.skip_nl();
				self.expect(TokKind::Lb, "'{'");
				self.skip_nl();
				let mut tk = TokenTree::default();
				while !self.at(TokKind::Rb) && !self.at(TokKind::Eof) {
					if self.reject_block_import() {
						self.skip_nl();
						continue;
					}
					if self.at_id("tokens") {
						let src = self.parse_tokens();
						tk.deep_merge(&src);
					} else {
						let bad = self.next();
						self.diags.error(
							"parse",
							"top-level `when` may only contain tokens overrides",
							bad.line,
						);
					}
					self.skip_nl();
				}
				self.expect(TokKind::Rb, "'}'");
				doc.topwhens.push((cond, tk, t.line));
			} else if t.kind == TokKind::Id && t.text == "theme" {
				let line = self.next().line;
				let name = self.expect(TokKind::Id, "theme name").text;
				self.skip_nl();
				self.expect(TokKind::Lb, "'{'");
				let tokens = self.token_entries();
				self.expect(TokKind::Rb, "'}'");
				doc.topwhens.push((Cond::Theme(name), tokens, line));
			} else if t.kind == TokKind::Id && t.text == "each" {
				self
					.diags
					.error("parse", "`each` is only allowed inside a node block", t.line);
				self.parse_each();
			} else if self.recover_missing_header_continuation() {
				continue;
			} else if t.kind == TokKind::Id {
				let n = self.parse_node();
				doc.roots.push(n);
			} else {
				self.diags.error(
					"parse",
					format!("unexpected {} {}", t.kind.name(), py_repr(&t.text)),
					t.line,
				);
				self.next();
			}
			self.skip_nl();
		}
		doc
	}

	fn parse_import(&mut self) -> Option<AImport> {
		let line = self.expect(TokKind::Id, "'import'").line;
		if !self.at(TokKind::Str) {
			self
				.diags
				.error("parse", "import expects a quoted path", self.peek().line);
			while !matches!(self.peek().kind, TokKind::Nl | TokKind::Eof) {
				self.next();
			}
			return None;
		}
		let path = self.next().text;
		if !matches!(self.peek().kind, TokKind::Nl | TokKind::Eof) {
			self
				.diags
				.error("parse", "expected a newline after the import path", self.peek().line);
			while !matches!(self.peek().kind, TokKind::Nl | TokKind::Eof) {
				self.next();
			}
		}
		Some(AImport { path, line })
	}

	/// `anim NAME { 0% { attrs } 100% { attrs } }` — time-indexed patches.
	fn parse_anim(&mut self) -> AAnim {
		let line = self.expect(TokKind::Id, "'anim'").line;
		let name = self.expect(TokKind::Id, "animation name").text;
		self.skip_nl();
		self.expect(TokKind::Lb, "'{'");
		self.skip_nl();
		let mut stops: Vec<(f64, Vec<(String, Value)>)> = Vec::new();
		while !self.at(TokKind::Rb) && !self.at(TokKind::Eof) {
			if self.reject_block_import() {
				self.skip_nl();
				continue;
			}
			let pct = self.expect(TokKind::Pct, "keyframe position like 0%");
			self.skip_nl();
			self.expect(TokKind::Lb, "'{'");
			let mut attrs: Vec<(String, Value)> = Vec::new();
			self.skip_nl();
			while !self.at(TokKind::Rb) && !self.at(TokKind::Eof) {
				if self.reject_block_import() {
					self.skip_nl();
					continue;
				}
				let key = self.expect(TokKind::Id, "attribute name").text;
				self.expect(TokKind::Eq, "'='");
				let v = self.parse_value();
				set_attr(&mut attrs, key, v);
				self.skip_nl();
			}
			self.expect(TokKind::Rb, "'}'");
			stops.push((pct.val / 100.0, attrs));
			self.skip_nl();
		}
		self.expect(TokKind::Rb, "'}'");
		stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
		AAnim { name, stops, line }
	}

	// -- tokens block -------------------------------------------------------

	fn parse_tokens(&mut self) -> TokenTree {
		self.expect(TokKind::Id, "'tokens'");
		self.skip_nl();
		self.expect(TokKind::Lb, "'{'");
		let out = self.token_entries();
		self.expect(TokKind::Rb, "'}'");
		out
	}

	fn token_entries(&mut self) -> TokenTree {
		let mut out = TokenTree::default();
		self.skip_nl();
		while !self.at(TokKind::Rb) && !self.at(TokKind::Eof) {
			if self.reject_block_import() {
				self.skip_nl();
				continue;
			}
			let name = self.expect(TokKind::Id, "token name").text;
			if self.at(TokKind::Lb) {
				self.next();
				let group = self.token_entries();
				self.expect(TokKind::Rb, "'}'");
				out.0.push((name, TokenEntry::Group(group)));
			} else {
				let v = self.parse_value();
				out.0.push((name, TokenEntry::Value(v)));
			}
			self.skip_nl();
		}
		out
	}

	// -- params block (1.0) -------------------------------------------------

	fn parse_params(&mut self, out: &mut Vec<ParamDecl>) {
		self.expect(TokKind::Id, "'params'");
		self.skip_nl();
		let group = if self.at(TokKind::Id) {
			Some(self.next().text)
		} else {
			None
		};
		self.expect(TokKind::Lb, "'{'");
		self.skip_nl();
		while !self.at(TokKind::Rb) && !self.at(TokKind::Eof) {
			if self.reject_block_import() {
				self.skip_nl();
				continue;
			}
			if self.at(TokKind::Id) && self.peek_at(1).kind == TokKind::Lb {
				let nested = self.next();
				self.next();
				self
					.diags
					.error("param-group", "param groups do not nest", nested.line);
				self.skip_braced_body();
				self.skip_nl();
				continue;
			}
			let leaf = self.expect(TokKind::Id, "param name").text;
			let name = group
				.as_ref()
				.map_or_else(|| leaf.clone(), |prefix| format!("{prefix}.{leaf}"));
			let ty_tok = self.expect(TokKind::Id, "param type (text|num|pct|color|bool|enum|list)");
			let line = ty_tok.line;
			let mut enum_syms = Vec::new();
			let ty = match ty_tok.text.as_str() {
				"text" => ParamType::Text,
				"num" => ParamType::Num,
				"pct" => ParamType::Pct,
				"color" => ParamType::Color,
				"bool" => ParamType::Bool,
				"enum" => {
					self.expect(TokKind::Lp, "'('");
					while !self.at(TokKind::Rp) && !self.at(TokKind::Eof) {
						enum_syms.push(self.expect(TokKind::Id, "enum member").text);
						if !self.eat(TokKind::Comma) {
							break;
						}
					}
					self.expect(TokKind::Rp, "')'");
					ParamType::Enum
				},
				"list" => {
					self.expect(TokKind::Lp, "'('");
					let schema = self.expect(TokKind::Id, "list item definition").text;
					self.expect(TokKind::Rp, "')'");
					ParamType::List(schema)
				},
				other => {
					self
						.diags
						.error("parse", format!("unknown param type {}", py_repr(other)), line);
					ParamType::Text
				},
			};
			self.expect(TokKind::Eq, "'='");
			let default = if matches!(&ty, ParamType::List(_)) {
				ParamDefault::List(self.parse_list_default())
			} else {
				ParamDefault::Scalar(self.parse_scalar())
			};
			out.push(ParamDecl { name, ty, enum_syms, default, line, prop_of: None });
			self.skip_nl();
		}
		self.expect(TokKind::Rb, "'}'");
	}

	fn parse_list_default(&mut self) -> Vec<ListItem> {
		self.expect(TokKind::Ls, "'['");
		self.skip_nl();
		let mut items = Vec::new();
		while !self.at(TokKind::Rs) && !self.at(TokKind::Eof) {
			let name = self.expect(TokKind::Id, "list item definition");
			self.expect(TokKind::Lp, "'('");
			self.skip_nl();
			let mut attrs = Vec::new();
			while !self.at(TokKind::Rp) && !self.at(TokKind::Eof) {
				let key = self.expect(TokKind::Id, "list item field").text;
				self.expect(TokKind::Eq, "'='");
				let value = if self.at(TokKind::Ls) {
					Value::List(self.parse_list_default())
				} else {
					self.parse_scalar()
				};
				set_attr(&mut attrs, key, value);
				self.skip_nl();
				if !self.eat(TokKind::Comma) {
					break;
				}
				self.skip_nl();
			}
			self.expect(TokKind::Rp, "')'");
			items.push(ListItem { name: name.text, attrs, line: name.line });
			self.skip_nl();
			if !self.eat(TokKind::Comma) {
				break;
			}
			self.skip_nl();
		}
		self.expect(TokKind::Rs, "']'");
		items
	}

	// -- def ----------------------------------------------------------------

	fn parse_def(&mut self) -> ADef {
		let line = self.expect(TokKind::Id, "'def'").line;
		let name = self.expect(TokKind::Id, "component name").text;
		if !name.chars().next().is_some_and(|c| c.is_uppercase()) {
			self
				.diags
				.error("parse", format!("component names must be Capitalized: {name}"), line);
		}
		self.expect(TokKind::Lp, "'('");
		let mut params: Vec<(String, Option<Value>)> = Vec::new();
		while !self.at(TokKind::Rp) && !self.at(TokKind::Eof) {
			let pname = self.expect(TokKind::Id, "param name").text;
			let default = if self.eat(TokKind::Eq) {
				Some(self.parse_scalar())
			} else {
				None
			};
			params.push((pname, default));
			if !self.eat(TokKind::Comma) {
				break;
			}
		}
		self.expect(TokKind::Rp, "')'");
		let export = if self.at_id("export") {
			self.next();
			true
		} else {
			false
		};
		self.skip_nl();
		self.expect(TokKind::Lb, "'{'");
		let body = self.parse_children();
		self.expect(TokKind::Rb, "'}'");
		ADef { name, params, export, body, line }
	}

	fn icon_decl_ahead(&self) -> bool {
		if self.peek_at(1).kind != TokKind::Id {
			return false;
		}
		let mut lookahead = 2usize;
		while self.peek_at(lookahead).kind == TokKind::Nl {
			lookahead += 1;
		}
		let token = self.peek_at(lookahead);
		token.kind == TokKind::Lb
			|| (token.kind == TokKind::Id
				&& token.text == "viewbox"
				&& self.peek_at(lookahead + 1).kind == TokKind::Eq)
	}

	// -- icons ---------------------------------------------------------------

	fn parse_icon(&mut self) -> AIcon {
		let line = self.expect(TokKind::Id, "'icon'").line;
		let name = self.expect(TokKind::Id, "icon name").text;
		let mut viewbox = Value::Num(24.0);
		while !matches!(self.peek().kind, TokKind::Nl | TokKind::Lb | TokKind::Rb | TokKind::Eof) {
			let attr = self.expect(TokKind::Id, "icon attribute");
			self.expect(TokKind::Eq, "'='");
			let value = self.parse_value();
			if attr.text == "viewbox" {
				viewbox = value;
			} else {
				self
					.diags
					.error("parse", format!("unknown icon attribute '{}'", attr.text), attr.line);
			}
		}
		self.skip_nl();
		self.expect(TokKind::Lb, "'{'");
		let body = self.parse_children();
		self.expect(TokKind::Rb, "'}'");
		AIcon { name, viewbox, body, line }
	}

	// -- nodes ----------------------------------------------------------------

	fn parse_node(&mut self) -> ANode {
		let t = self.expect(TokKind::Id, "node name");
		let mut node = ANode {
			name:     t.text,
			id:       None,
			args:     Vec::new(),
			attrs:    Vec::new(),
			flags:    Vec::new(),
			children: Vec::new(),
			line:     t.line,
		};
		if self.at(TokKind::Hash) {
			node.id = Some(self.next().text);
		}
		// header: attrs / flags / args until NL, LB, RB or EOF
		while !self.at(TokKind::Nl)
			&& !self.at(TokKind::Lb)
			&& !self.at(TokKind::Rb)
			&& !self.at(TokKind::Eof)
		{
			let p = self.peek().clone();
			if p.kind == TokKind::Id && self.peek_at(1).kind == TokKind::Eq {
				let key = self.next().text;
				self.next(); // EQ
				let v = self.parse_value();
				set_attr(&mut node.attrs, key, v);
			} else if p.kind == TokKind::Id && FLAGS.contains(&p.text.as_str()) {
				node.flags.push(self.next().text);
			} else if matches!(
				p.kind,
				TokKind::Str | TokKind::Num | TokKind::Pct | TokKind::Hash | TokKind::Ref | TokKind::Id
			) {
				let v = self.parse_value();
				node.args.push(v);
			} else {
				self.diags.error(
					"parse",
					format!("unexpected {} {} in node header", p.kind.name(), py_repr(&p.text)),
					p.line,
				);
				self.next();
			}
		}
		if self.at(TokKind::Lb) {
			self.next();
			node.children = self.parse_children();
			self.expect(TokKind::Rb, "'}'");
		}
		node
	}

	fn parse_each(&mut self) -> AEach {
		let line = self.expect(TokKind::Id, "'each'").line;
		let target = self.next();
		let (param, prop) = match target.kind {
			TokKind::Ref => {
				let mut parts = target.text.split('.');
				let prefix = parts.next().unwrap_or_default();
				let name = parts.collect::<Vec<_>>().join(".");
				if prefix != "param" || name.is_empty() {
					self
						.diags
						.error("parse", "`each` root target must be `param.NAME`", target.line);
				}
				(name, false)
			},
			TokKind::Id => (target.text, true),
			_ => {
				self.diags.error(
					"parse",
					"`each` target must be `param.NAME` or a list-valued item property",
					target.line,
				);
				(String::new(), false)
			},
		};
		let mut id = None;
		let mut attrs = Vec::new();
		let mut flags = Vec::new();
		if self.at(TokKind::Hash) {
			id = Some(self.next().text);
		}
		while !matches!(self.peek().kind, TokKind::Nl | TokKind::Lb | TokKind::Rb | TokKind::Eof) {
			let t = self.peek().clone();
			if t.kind == TokKind::Id && self.peek_at(1).kind == TokKind::Eq {
				let key = self.next().text;
				self.next();
				let value = self.parse_value();
				set_attr(&mut attrs, key, value);
			} else if t.kind == TokKind::Id && t.text == "virtual" {
				flags.push(self.next().text);
			} else {
				self.diags.error(
					"parse",
					format!("unexpected {} {} in `each` header", t.kind.name(), py_repr(&t.text)),
					t.line,
				);
				self.next();
			}
		}
		if self.eat(TokKind::Lb) {
			self
				.diags
				.error("parse", "`each` may not have a child block", line);
			self.parse_children();
			self.expect(TokKind::Rb, "'}'");
		}
		AEach { param, prop, id, attrs, flags, line }
	}

	fn parse_children(&mut self) -> Vec<Item> {
		let mut out = Vec::new();
		self.skip_nl();
		while !self.at(TokKind::Rb) && !self.at(TokKind::Eof) {
			let t = self.peek().clone();
			if self.reject_block_import() {
				self.skip_nl();
				continue;
			}
			if t.kind == TokKind::Str {
				self.next();
				out.push(Item::Text(t.text, t.line));
			} else if t.kind == TokKind::Id && t.text == "when" {
				out.push(Item::When(self.parse_when()));
			} else if t.kind == TokKind::Id && t.text == "each" {
				out.push(Item::Each(self.parse_each()));
			} else if self.recover_missing_header_continuation() {
				continue;
			} else if t.kind == TokKind::Id {
				out.push(Item::Node(self.parse_node()));
			} else {
				self.diags.error(
					"parse",
					format!("unexpected {} {} in block", t.kind.name(), py_repr(&t.text)),
					t.line,
				);
				self.next();
			}
			self.skip_nl();
		}
		out
	}

	fn parse_when(&mut self) -> AWhen {
		let line = self.expect(TokKind::Id, "'when'").line;
		let cond = self.parse_cond();
		self.skip_nl();
		self.expect(TokKind::Lb, "'{'");
		let mut attrs: Vec<(String, Value)> = Vec::new();
		let mut flags: Vec<String> = Vec::new();
		let mut children: Vec<Item> = Vec::new();
		self.skip_nl();
		while !self.at(TokKind::Rb) && !self.at(TokKind::Eof) {
			let t = self.peek().clone();
			if self.reject_block_import() {
				self.skip_nl();
				continue;
			}
			if t.kind == TokKind::Id && self.peek_at(1).kind == TokKind::Eq {
				let key = self.next().text;
				self.next();
				let v = self.parse_value();
				set_attr(&mut attrs, key, v);
			} else if t.kind == TokKind::Id
				&& FLAGS.contains(&t.text.as_str())
				&& self.peek_at(1).kind != TokKind::Eq
			{
				flags.push(self.next().text);
			} else if t.kind == TokKind::Str {
				self.next();
				children.push(Item::Text(t.text, t.line));
			} else if t.kind == TokKind::Id && t.text == "when" {
				self.diags.error_with(
					"when-compose",
					"a `when` block cannot be nested inside another `when` patch",
					t.line,
					"use one state patch with themed tokens, e.g. `when hover { bg=color.surface }`; \
					 token references now resolve through the active theme",
				);
				self.parse_when();
			} else if t.kind == TokKind::Id && t.text == "each" {
				children.push(Item::Each(self.parse_each()));
			} else if t.kind == TokKind::Id {
				children.push(Item::Node(self.parse_node()));
			} else {
				self.diags.error(
					"parse",
					format!("unexpected {} {} in when block", t.kind.name(), py_repr(&t.text)),
					t.line,
				);
				self.next();
			}
			self.skip_nl();
		}
		self.expect(TokKind::Rb, "'}'");
		AWhen { cond, attrs, flags, children, line }
	}

	fn parse_cond(&mut self) -> Cond {
		let neg = self.eat(TokKind::Bang);
		let t = if self.at(TokKind::Ref) {
			self.next()
		} else {
			self.expect(TokKind::Id, if neg { "condition name" } else { "condition" })
		};
		if neg {
			return Cond::Ident { name: t.text, neg: true };
		}
		if t.text == "theme" && self.eat(TokKind::Lp) {
			let name = self.expect(TokKind::Id, "theme name").text;
			self.expect(TokKind::Rp, "')'");
			return Cond::Theme(name);
		}
		if (t.text == "w" || t.text == "h") && self.at(TokKind::Cmp) {
			let op = match self.next().text.as_str() {
				"<" => CmpOp::Lt,
				"<=" => CmpOp::Le,
				">" => CmpOp::Gt,
				_ => CmpOp::Ge,
			};
			let num = self.expect(TokKind::Num, "number");
			return Cond::Cmp {
				axis: if t.text == "w" {
					CmpAxis::W
				} else {
					CmpAxis::H
				},
				op,
				num: num.val,
			};
		}
		Cond::Ident { name: t.text, neg: false }
	}

	// -- values ---------------------------------------------------------------

	fn parse_value(&mut self) -> Value {
		let first = self.parse_scalar();
		if self.at(TokKind::Colon) {
			return self.parse_key_map(first);
		}
		if !self.at(TokKind::Comma) {
			return first;
		}
		let mut items = vec![first];
		while self.eat(TokKind::Comma) {
			items.push(self.parse_scalar());
		}
		Value::Tup(items)
	}

	fn parse_key_map(&mut self, first_key: Value) -> Value {
		let mut entries = Vec::new();
		let mut key = first_key;
		loop {
			self.expect(TokKind::Colon, "':' between a key and signal");
			let signal = self.parse_scalar();
			entries.push((key, signal));
			if !self.eat(TokKind::Comma) {
				break;
			}
			key = self.parse_scalar();
			if !self.at(TokKind::Colon) {
				self.diags.error(
					"parse",
					"key map entries must be `Key:signal` pairs",
					self.peek().line,
				);
				break;
			}
		}
		Value::KeyMap(entries)
	}

	fn parse_scalar(&mut self) -> Value {
		if self.at(TokKind::Ls) {
			return Value::List(self.parse_list_default());
		}
		let t = self.next();
		match t.kind {
			TokKind::Num => Value::Num(t.val),
			TokKind::Pct => Value::Pct(t.val),
			TokKind::Str => Value::Str(t.text),
			TokKind::Hash => Value::Color(format!("#{}", t.text)),
			TokKind::Ref => Value::Ref(t.text.split('.').map(str::to_string).collect()),
			TokKind::Id => {
				if t.text == "fill" {
					if self.at(TokKind::Colon) {
						self.next();
						let n = self.expect(TokKind::Num, "fill weight");
						return Value::Fill(if n.val == 0.0 { 1.0 } else { n.val });
					}
					return Value::Fill(1.0);
				}
				if t.text == "list" && self.at(TokKind::Lp) {
					self.next();
					let schema = self.expect(TokKind::Id, "list item definition").text;
					self.expect(TokKind::Rp, "')'");
					return Value::ListSchema(schema);
				}
				if self.at(TokKind::Lp) {
					// color function: oklch(...), rgb(...), linear(...), radial(...)
					self.next();
					let inner = self.raw_until_rp();
					return Value::Color(format!("{}({})", t.text, inner));
				}
				Value::Kw(t.text)
			},
			_ => {
				self.diags.error(
					"parse",
					format!("expected a value, got {} {}", t.kind.name(), py_repr(&t.text)),
					t.line,
				);
				Value::Kw(String::new())
			},
		}
	}

	/// Reconstruct raw text inside a color-fn call, matching the reference
	/// spacing rules so color strings compare identically.
	fn raw_until_rp(&mut self) -> String {
		let mut parts: Vec<String> = Vec::new();
		let mut depth = 1;
		while depth > 0 && !self.at(TokKind::Eof) {
			let t = self.next();
			match t.kind {
				TokKind::Lp => depth += 1,
				TokKind::Rp => {
					depth -= 1;
					if depth == 0 {
						break;
					}
				},
				_ => {},
			}
			if depth > 0 {
				let s = match t.kind {
					TokKind::Comma => ",".to_string(),
					TokKind::Hash => format!("#{}", t.text),
					_ => t.text,
				};
				parts.push(s);
			}
		}
		let mut out = String::new();
		for p in parts {
			match p.as_str() {
				"," => {
					while out.ends_with(' ') {
						out.pop();
					}
					out.push_str(", ");
				},
				"(" => {
					while out.ends_with(' ') {
						out.pop();
					}
					out.push('(');
				},
				")" => {
					while out.ends_with(' ') {
						out.pop();
					}
					out.push_str(") ");
				},
				_ => {
					out.push_str(&p);
					out.push(' ');
				},
			}
		}
		out.trim().to_string()
	}
}

/// Insert an attr, replacing an existing key in place (dict semantics).
fn set_attr(attrs: &mut Vec<(String, Value)>, key: String, v: Value) {
	if let Some(slot) = attrs.iter_mut().find(|(k, _)| *k == key) {
		slot.1 = v;
	} else {
		attrs.push((key, v));
	}
}

/// Parse slab source into a raw AST document.
pub fn parse(src: &str, diags: &mut Diagnostics) -> Document {
	Parser::new(src, diags).parse()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_list_defaults_each_and_multiline() {
		let src = r#"
params {
  tracks list(TrackRow) = [
    TrackRow(title="A", tone=#f00),
    TrackRow(title="B"),
  ]
  title text = "Playlist"
}
col {
  each param.tracks #tracks key=title
  text field=draft multiline submit=send
}
"#;
		let mut diags = Diagnostics::default();
		let doc = parse(src, &mut diags);
		assert!(diags.0.is_empty(), "{:?}", diags.0);
		assert_eq!(doc.params.len(), 2);
		assert_eq!(doc.params[0].ty, ParamType::List("TrackRow".into()));
		let ParamDefault::List(items) = &doc.params[0].default else {
			panic!("expected list default");
		};
		assert_eq!(items.len(), 2);
		assert_eq!(items[0].name, "TrackRow");
		assert_eq!(items[0].attrs.len(), 2);
		assert!(matches!(
			 &doc.params[1].default,
			 ParamDefault::Scalar(Value::Str(value)) if value == "Playlist"
		));
		let Item::Each(each) = &doc.roots[0].children[0] else {
			panic!("expected each item");
		};
		assert_eq!(each.param, "tracks");
		assert_eq!(each.id.as_deref(), Some("tracks"));
		assert_eq!(each.attrs[0].0, "key");
		assert!(!each.prop);
		assert!(each.flags.is_empty());
		let Item::Node(field) = &doc.roots[0].children[1] else {
			panic!("expected field node");
		};
		assert!(field.flags.iter().any(|flag| flag == "multiline"));
		assert!(field.attr("submit").is_some());
	}

	#[test]
	fn parses_cross_scroll_modes_and_sticky_flag() {
		let mut diags = Diagnostics::default();
		let doc = parse("col scroll=both { row scroll=cross { rect sticky } }\n", &mut diags);
		assert!(diags.0.is_empty(), "{:?}", diags.0);
		assert!(matches!(
			 doc.roots[0].attr("scroll"),
			 Some(Value::Kw(mode)) if mode == "both"
		));
		let Item::Node(row) = &doc.roots[0].children[0] else {
			panic!("expected row child");
		};
		assert!(matches!(
			 row.attr("scroll"),
			 Some(Value::Kw(mode)) if mode == "cross"
		));
		let Item::Node(sticky) = &row.children[0] else {
			panic!("expected sticky child");
		};
		assert_eq!(sticky.flags, ["sticky"]);
	}

	#[test]
	fn malformed_lists_and_each_report_parse_diagnostics() {
		let src = r#"
params { tracks list(TrackRow) = [TrackRow("missing-field")] }
col { each tracks }
"#;
		let mut diags = Diagnostics::default();
		parse(src, &mut diags);
		assert!(diags.0.iter().any(|diag| diag.code == "parse"));
	}

	#[test]
	fn parses_recursive_list_fields_nested_defaults_and_each_props() {
		let src = r#"
def Tree(label="", children=list(Tree)) export {
  col {
    text label
    each children
  }
}
params {
  roots list(Tree) = [
    Tree(label="root", children=[Tree(label="child")]),
  ]
}
col scroll {
  each param.roots virtual item-extent=20 overscan=2
}
"#;
		let mut diags = Diagnostics::default();
		let doc = parse(src, &mut diags);
		assert!(diags.0.is_empty(), "{:?}", diags.0);
		assert!(matches!(
			 &doc.defs[0].params[1].1,
			 Some(Value::ListSchema(schema)) if schema == "Tree"
		));
		let ParamDefault::List(items) = &doc.params[0].default else {
			panic!("expected root list default");
		};
		assert!(matches!(
			 &items[0].attrs[1].1,
			 Value::List(children) if children.len() == 1
		));
		let Item::Node(container) = &doc.defs[0].body[0] else {
			panic!("expected tree container");
		};
		let Item::Each(nested) = &container.children[1] else {
			panic!("expected nested each");
		};
		assert!(nested.prop);
		let Item::Each(virtual_each) = &doc.roots[0].children[0] else {
			panic!("expected virtual each");
		};
		assert!(virtual_each.flags.iter().any(|flag| flag == "virtual"));
	}

	#[test]
	fn nested_when_reports_composition_and_themed_token_remedy() {
		let mut diagnostics = Diagnostics::default();
		let document = parse(
			"tokens { color { surface #eee } }\nrect {\n  when theme(dusk) { when hover { \
			 bg=color.surface } }\n}\n",
			&mut diagnostics,
		);
		assert_eq!(diagnostics.0.len(), 1, "{:?}", diagnostics.0);
		let diagnostic = &diagnostics.0[0];
		assert_eq!(diagnostic.code, "when-compose");
		assert!(diagnostic.msg.contains("cannot be nested"));
		assert!(
			diagnostic
				.remedy
				.as_deref()
				.is_some_and(|remedy| remedy.contains("active theme"))
		);
		let Item::When(outer) = &document.roots[0].children[0] else {
			panic!("expected outer when");
		};
		assert!(outer.children.is_empty());
	}

	#[test]
	fn bare_flags_in_when_body_parse_with_trailing_attrs_and_siblings() {
		let mut diagnostics = Diagnostics::default();
		let document =
			parse("text \"t\" {\n  when done { strike color=#888888 nowrap }\n}\n", &mut diagnostics);
		assert!(diagnostics.0.is_empty(), "{:?}", diagnostics.0);
		let Item::When(when) = &document.roots[0].children[0] else {
			panic!("expected when patch");
		};
		assert_eq!(when.flags, ["strike", "nowrap"]);
		assert!(when.attrs.iter().any(|(key, _)| key == "color"));
		assert!(
			when.children.is_empty(),
			"flags must not misparse as nodes: {:?}",
			when.children.len()
		);
	}

	#[test]
	fn flag_named_attr_in_when_body_still_parses_as_attr() {
		let mut diagnostics = Diagnostics::default();
		let document = parse("col {\n  when wide { scroll=cross }\n}\n", &mut diagnostics);
		assert!(diagnostics.0.is_empty(), "{:?}", diagnostics.0);
		let Item::When(when) = &document.roots[0].children[0] else {
			panic!("expected when patch");
		};
		assert!(when.flags.is_empty());
		assert!(when.attrs.iter().any(|(key, _)| key == "scroll"));
	}

	#[test]
	fn parses_cancel_binder_attribute() {
		let mut diagnostics = Diagnostics::default();
		let document = parse("text \"d\" field=draft submit=send cancel=discard\n", &mut diagnostics);
		assert!(diagnostics.0.is_empty(), "{:?}", diagnostics.0);
		assert!(matches!(
			 document.roots[0].attr("cancel"),
			 Some(Value::Kw(name)) if name == "discard"
		));
	}

	#[test]
	fn missing_node_header_continuation_reports_once_and_recovers() {
		let mut diagnostics = Diagnostics::default();
		let document = parse(
			"col w=fill {\n  text \"save\"\n    size=18\n    weight=700\n  rect h=1\n}\n",
			&mut diagnostics,
		);
		assert_eq!(diagnostics.0.len(), 1, "{:?}", diagnostics.0);
		let diagnostic = &diagnostics.0[0];
		assert_eq!(diagnostic.code, "parse");
		assert_eq!(diagnostic.line, 3);
		assert!(
			diagnostic
				.remedy
				.as_deref()
				.is_some_and(|remedy| remedy.contains("continue the previous line with `\\`"))
		);
		assert_eq!(document.roots[0].children.len(), 2);
	}

	#[test]
	fn parses_gesture_signal_attributes_and_states() {
		let source = r"
box press=pressed context=menu dblclick=twice drag=started drop=dropped resize=resized {
  when dragging { opacity=0.5 }
  when drop { stroke=#fff }
}
";
		let mut diagnostics = Diagnostics::default();
		let document = parse(source, &mut diagnostics);
		assert!(diagnostics.0.is_empty(), "{:?}", diagnostics.0);
		let node = &document.roots[0];
		for (attribute, signal) in [
			("press", "pressed"),
			("context", "menu"),
			("dblclick", "twice"),
			("drag", "started"),
			("drop", "dropped"),
			("resize", "resized"),
		] {
			assert!(
				matches!(node.attr(attribute), Some(Value::Kw(name)) if name == signal),
				"{attribute}"
			);
		}
		assert_eq!(node.children.len(), 2);
		assert!(matches!(
			 &node.children[0],
			 Item::When(when)
				  if matches!(
						&when.cond,
						Cond::Ident { name, neg: false } if name == "dragging"
				  )
		));
		assert!(matches!(
			 &node.children[1],
			 Item::When(when)
				  if matches!(&when.cond, Cond::Ident { name, neg: false } if name == "drop")
		));
	}

	#[test]
	fn parses_typed_key_signal_map() {
		let mut diagnostics = Diagnostics::default();
		let document = parse("col keys=Escape:clear,F2:rename { }\n", &mut diagnostics);
		assert!(diagnostics.0.is_empty(), "{:?}", diagnostics.0);
		let Some(Value::KeyMap(entries)) = document.roots[0].attr("keys") else {
			panic!("expected key map");
		};
		assert_eq!(entries.len(), 2);
		assert!(matches!(
			 &entries[0],
			 (Value::Kw(key), Value::Kw(signal)) if key == "Escape" && signal == "clear"
		));
	}

	#[test]
	fn parses_import_group_dotted_conditions_and_each() {
		let source = r#"import "ui/panel.slab"
params panel {
  open bool = true
  rows list(Row) = []
}
col {
  when panel.open { opacity=1 }
  when !panel.open { opacity=0 }
  each param.panel.rows
}
"#;
		let mut diagnostics = Diagnostics::default();
		let document = parse(source, &mut diagnostics);

		assert!(diagnostics.0.is_empty(), "{:?}", diagnostics.0);
		assert_eq!(document.imports, [AImport { path: "ui/panel.slab".into(), line: 1 }]);
		assert_eq!(document.params[0].name, "panel.open");
		assert_eq!(document.params[1].name, "panel.rows");
		assert!(matches!(
			 &document.roots[0].children[0],
			 Item::When(when)
				  if matches!(
						&when.cond,
						Cond::Ident { name, neg: false } if name == "panel.open"
				  )
		));
		assert!(matches!(
			 &document.roots[0].children[1],
			 Item::When(when)
				  if matches!(
						&when.cond,
						Cond::Ident { name, neg: true } if name == "panel.open"
				  )
		));
		assert!(matches!(
			 &document.roots[0].children[2],
			 Item::Each(each) if each.param == "panel.rows"
		));
	}

	#[test]
	fn rejects_unquoted_and_block_imports() {
		let mut diagnostics = Diagnostics::default();
		let document = parse(
			"import panel.slab\ncol {\n  import \"nested.slab\"\n  text \"ok\"\n}\n",
			&mut diagnostics,
		);

		assert!(document.imports.is_empty());
		assert_eq!(document.roots[0].children.len(), 1);
		assert_eq!(diagnostics.0.len(), 2, "{:?}", diagnostics.0);
		assert_eq!(diagnostics.0[0].msg, "import expects a quoted path");
		assert_eq!(diagnostics.0[1].msg, "import is allowed at top level only");
	}

	#[test]
	fn rejects_nested_param_groups_and_recovers() {
		let source = r"params ui {
  dialog {
    open bool = true
  }
  enabled bool = false
}
";
		let mut diagnostics = Diagnostics::default();
		let document = parse(source, &mut diagnostics);

		assert_eq!(document.params.len(), 1);
		assert_eq!(document.params[0].name, "ui.enabled");
		assert_eq!(diagnostics.0.len(), 1, "{:?}", diagnostics.0);
		assert_eq!(diagnostics.0[0].code, "param-group");
		assert_eq!(diagnostics.0[0].msg, "param groups do not nest");
	}
}
