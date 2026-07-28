//! Single-threaded LSP server; `handle(msg)` returns outgoing responses and
//! notifications.
//!
//! The protocol is testable in-memory. Positions are UTF-16 code units per LSP;
//! internal columns are `char` indices.

use std::{
	collections::{HashMap, HashSet},
	fmt::Write as _,
	path::{Component, Path, PathBuf},
};

use serde_json::{Value, json};
use slab_compile::color::{Rgba, parse_rgba};
use slab_syntax::{
	diag::{Diagnostics, Level},
	lex::TokKind,
	parse::FLAGS,
};

use crate::{
	index::{Index, LTok, Sym, TNode, build_index},
	vocab,
};

// --------------------------------------------------------------- UTF-16 maths

/// UTF-16 code-unit length of a string.
pub fn u16_len(s: &str) -> usize {
	s.chars().map(|c| c.len_utf16()).sum()
}

/// Clamped char index for a UTF-16 column on one line.
pub fn u16_to_idx(s: &str, u16: usize) -> usize {
	let mut acc = 0usize;
	for (i, c) in s.chars().enumerate() {
		if acc >= u16 {
			return i;
		}
		acc += c.len_utf16();
	}
	s.chars().count()
}

/// UTF-16 column for a char index on one line.
pub fn idx_to_u16(s: &str, idx: usize) -> usize {
	s.chars().take(idx).map(|c| c.len_utf16()).sum()
}

const fn severity(level: Level) -> i64 {
	match level {
		Level::Error => 1,
		Level::Warning => 2,
		Level::Note => 3,
	}
}

/// `#rrggbb`, or `#rrggbbaa` when alpha < 255.
fn rgba_hex(c: Rgba) -> String {
	if c[3] == u8::MAX {
		format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
	} else {
		format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
	}
}

fn uri_path(uri: &str) -> Option<PathBuf> {
	uri.strip_prefix("file://")
		.map(percent_decode)
		.map(PathBuf::from)
}

fn lexical_path(path: &Path) -> PathBuf {
	let mut normalized = PathBuf::new();
	for component in path.components() {
		match component {
			Component::CurDir => {},
			Component::ParentDir => {
				if normalized.file_name().is_some() {
					normalized.pop();
				} else if !path.is_absolute() {
					normalized.push("..");
				}
			},
			_ => normalized.push(component.as_os_str()),
		}
	}
	normalized
}

fn path_uri(path: &Path) -> String {
	let absolute = if path.is_absolute() {
		lexical_path(path)
	} else {
		std::env::current_dir()
			.map_or_else(|_| lexical_path(path), |directory| lexical_path(&directory.join(path)))
	};
	let mut encoded = String::new();
	for byte in absolute.to_string_lossy().bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
			encoded.push(char::from(byte));
		} else {
			write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail");
		}
	}
	format!("file://{encoded}")
}

/// Directory of a `file://` uri (percent-decoded); `.` otherwise.
fn uri_dir(uri: &str) -> PathBuf {
	uri_path(uri)
		.and_then(|path| path.parent().map(Path::to_path_buf))
		.unwrap_or_else(|| PathBuf::from("."))
}

fn percent_decode(s: &str) -> String {
	let bytes = s.as_bytes();
	let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'%'
			&& i + 2 < bytes.len()
			&& let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
		{
			out.push(b);
			i += 3;
			continue;
		}
		out.push(bytes[i]);
		i += 1;
	}
	String::from_utf8_lossy(&out).into_owned()
}

#[derive(Default)]
struct LoadedSources {
	sources: HashMap<String, String>,
	uris:    HashMap<String, String>,
}

struct Compilation {
	loaded:      LoadedSources,
	units:       Vec<slab_compile::import::Unit>,
	slir:        Option<slab_slir::Slir>,
	diagnostics: Diagnostics,
}

// --------------------------------------------------------------------- server

/// Single-threaded LSP server over in-memory messages.
pub struct Server {
	pub docs:      HashMap<String, String>,
	pub index:     HashMap<String, Index>,
	pub running:   bool,
	pub exit_code: i32,
	closures:      HashMap<String, Vec<String>>,
	diagnostics:   HashMap<String, HashMap<String, Vec<Value>>>,
	shutdown:      bool,
}

impl Default for Server {
	fn default() -> Self {
		Self::new()
	}
}

impl Server {
	pub fn new() -> Self {
		Self {
			docs:        HashMap::new(),
			index:       HashMap::new(),
			running:     true,
			exit_code:   1,
			shutdown:    false,
			closures:    HashMap::new(),
			diagnostics: HashMap::new(),
		}
	}

	/// Process one JSON-RPC message; returns responses + notifications.
	pub fn handle(&mut self, msg: &Value) -> Vec<Value> {
		let Some(method) = msg.get("method").and_then(Value::as_str) else {
			return Vec::new(); // a response from the client; nothing to do
		};
		let method = method.to_string();
		let mid = msg.get("id").cloned().filter(|v| !v.is_null());
		let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
		if let Some(mid) = mid {
			// request
			let known = Self::is_known(&method);
			if !known {
				return vec![json!({"jsonrpc": "2.0", "id": mid, "error":
                    {"code": -32601, "message": format!("method not found: {method}")}})];
			}
			let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				self.dispatch(&method, &params)
			}));
			match outcome {
				Ok((result, _notes)) => {
					vec![json!({"jsonrpc": "2.0", "id": mid, "result": result})]
				},
				Err(e) => {
					// never crash the loop on a bad request
					let detail = panic_msg(e.as_ref());
					vec![json!({"jsonrpc": "2.0", "id": mid, "error":
                        {"code": -32603, "message": format!("internal error: {detail}")}})]
				},
			}
		} else {
			if !Self::is_known(&method) {
				return Vec::new(); // unknown notification: ignore per spec
			}
			let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				self.dispatch(&method, &params)
			}));
			match outcome {
				Ok((_result, notes)) => notes,
				Err(_) => Vec::new(),
			}
		}
	}

	fn is_known(method: &str) -> bool {
		matches!(
			method,
			"initialize"
				| "initialized"
				| "shutdown"
				| "exit" | "$/cancelRequest"
				| "textDocument/didOpen"
				| "textDocument/didChange"
				| "textDocument/didClose"
				| "textDocument/didSave"
				| "textDocument/completion"
				| "textDocument/hover"
				| "textDocument/definition"
				| "textDocument/documentSymbol"
				| "textDocument/documentColor"
				| "textDocument/colorPresentation"
				| "textDocument/formatting"
				| "slab/preview"
		)
	}

	/// Returns (request result, notifications).
	fn dispatch(&mut self, method: &str, p: &Value) -> (Value, Vec<Value>) {
		match method {
			"initialize" => (Self::initialize(), Vec::new()),
			"shutdown" => {
				self.shutdown = true;
				(Value::Null, Vec::new())
			},
			"exit" => {
				self.exit_code = i32::from(!self.shutdown);
				self.running = false;
				(Value::Null, Vec::new())
			},
			"textDocument/didOpen" => (Value::Null, self.did_open(p)),
			"textDocument/didChange" => (Value::Null, self.did_change(p)),
			"textDocument/didClose" => (Value::Null, self.did_close(p)),
			"textDocument/completion" => (self.completion(p), Vec::new()),
			"textDocument/hover" => (self.hover(p), Vec::new()),
			"textDocument/definition" => (self.definition(p), Vec::new()),
			"textDocument/documentSymbol" => (self.document_symbol(p), Vec::new()),
			"textDocument/documentColor" => (self.document_color(p), Vec::new()),
			"textDocument/colorPresentation" => (Self::color_presentation(p), Vec::new()),
			"textDocument/formatting" => (self.formatting(p), Vec::new()),
			"slab/preview" => (self.preview(p), Vec::new()),
			// initialized, didSave, $/cancelRequest
			_ => (json!([]), Vec::new()),
		}
	}

	// -- lifecycle
	fn initialize() -> Value {
		json!({
			 "capabilities": {
				  "textDocumentSync": {"openClose": true, "change": 2}, // 2 = Incremental
				  "completionProvider": {"triggerCharacters": ["=", ".", "#", ","]},
				  "hoverProvider": true,
				  "definitionProvider": true,
				  "documentSymbolProvider": true,
				  "colorProvider": true,
				  "documentFormattingProvider": true,
			 },
			 "serverInfo": {"name": "slab-lsp", "version": env!("CARGO_PKG_VERSION")},
		})
	}

	// -- document sync
	fn did_open(&mut self, p: &Value) -> Vec<Value> {
		let doc = &p["textDocument"];
		let uri = doc["uri"].as_str().unwrap_or_default().to_string();
		let text = doc["text"].as_str().unwrap_or_default().to_string();
		self.docs.insert(uri.clone(), text);
		self.refresh_related(&uri)
	}

	fn did_change(&mut self, p: &Value) -> Vec<Value> {
		let uri = p["textDocument"]["uri"]
			.as_str()
			.unwrap_or_default()
			.to_string();
		let mut text = self.docs.get(&uri).cloned().unwrap_or_default();
		if let Some(changes) = p.get("contentChanges").and_then(Value::as_array) {
			for ch in changes {
				let new = ch["text"].as_str().unwrap_or_default();
				if let Some(rng) = ch.get("range") {
					text = apply_edit(&text, rng, new);
				} else {
					text = new.to_string();
				}
			}
		}
		self.docs.insert(uri.clone(), text);
		self.refresh_related(&uri)
	}

	fn did_close(&mut self, p: &Value) -> Vec<Value> {
		let uri = p["textDocument"]["uri"]
			.as_str()
			.unwrap_or_default()
			.to_string();
		let roots = self
			.closures
			.iter()
			.filter(|(root, closure)| *root != &uri && closure.contains(&uri))
			.map(|(root, _)| root.clone())
			.collect::<Vec<_>>();
		let mut affected = HashSet::from([uri.clone()]);
		if let Some(previous) = self.diagnostics.remove(&uri) {
			affected.extend(previous.into_keys());
		}
		self.docs.remove(&uri);
		self.index.remove(&uri);
		self.closures.remove(&uri);
		let mut notifications = Vec::new();
		for root in roots {
			if self.docs.contains_key(&root) {
				notifications.extend(self.refresh(&root));
			}
		}
		notifications.extend(self.diagnostic_notifications(&uri, affected));
		notifications
	}

	/// Re-index and compile a root plus its import closure.
	fn refresh(&mut self, uri: &str) -> Vec<Value> {
		let source = self.docs.get(uri).cloned().unwrap_or_default();
		let compilation = self.compile_source(uri, &source);

		self.index.insert(uri.to_string(), build_index(&source));
		for (key, imported) in &compilation.loaded.sources {
			if let Some(owner) = compilation.loaded.uris.get(key) {
				self.index.insert(owner.clone(), build_index(imported));
			}
		}

		let order = compilation
			.units
			.iter()
			.filter_map(|unit| {
				unit
					.file
					.as_ref()
					.and_then(|key| compilation.loaded.uris.get(key))
					.cloned()
					.or_else(|| unit.file.is_none().then(|| uri.to_string()))
			})
			.collect::<Vec<_>>();
		self.closures.insert(uri.to_string(), order);

		let mut grouped = HashMap::<String, Vec<Value>>::new();
		grouped.entry(uri.to_string()).or_default();
		for diagnostic in &compilation.diagnostics.0 {
			let owner = diagnostic
				.file
				.as_ref()
				.and_then(|key| compilation.loaded.uris.get(key))
				.map_or_else(|| uri.to_string(), Clone::clone);
			let lines = self
				.index
				.get(&owner)
				.map_or(&[] as &[String], |index| index.lines.as_slice());
			grouped.entry(owner).or_default().push(json!({
				 "range": diag_range(lines, diagnostic.line),
				 "severity": severity(diagnostic.level),
				 "code": diagnostic.code,
				 "source": "slab",
				 "message": diagnostic.msg,
			}));
		}

		let mut affected = grouped.keys().cloned().collect::<HashSet<_>>();
		if let Some(previous) = self.diagnostics.insert(uri.to_string(), grouped) {
			affected.extend(previous.into_keys());
		}
		self.diagnostic_notifications(uri, affected)
	}

	fn refresh_related(&mut self, uri: &str) -> Vec<Value> {
		let roots = self
			.closures
			.iter()
			.filter(|(root, closure)| *root != uri && closure.iter().any(|owner| owner == uri))
			.map(|(root, _)| root.clone())
			.collect::<Vec<_>>();
		let mut notifications = self.refresh(uri);
		for root in roots {
			if self.docs.contains_key(&root) {
				notifications.extend(self.refresh(&root));
			}
		}
		notifications
	}

	fn source_at_path(&self, path: &Path) -> Option<(String, String)> {
		let normalized = lexical_path(path);
		if let Some((uri, source)) = self.docs.iter().find(|(uri, _)| {
			uri_path(uri)
				.as_deref()
				.map(lexical_path)
				.is_some_and(|candidate| candidate == normalized)
		}) {
			return Some((uri.clone(), source.clone()));
		}
		std::fs::read_to_string(&normalized)
			.ok()
			.map(|source| (path_uri(&normalized), source))
	}

	fn load_sources(&self, root_uri: &str, root_source: &str) -> LoadedSources {
		let base_dir = uri_dir(root_uri);
		let mut loaded = LoadedSources::default();
		let mut seen = HashSet::new();
		let mut pending = vec![(None::<String>, root_source.to_string(), 0_usize)];
		let mut index = 0;
		while index < pending.len() {
			let (importer, source, depth) = pending[index].clone();
			index += 1;
			if depth >= slab_compile::expand::MAX_DEPTH {
				continue;
			}
			let mut diagnostics = Diagnostics::new();
			let document = slab_syntax::parse(&source, &mut diagnostics);
			for import in document.imports {
				let key = slab_compile::import::normalize(importer.as_deref(), &import.path);
				if !seen.insert(key.clone()) {
					continue;
				}
				let Some((uri, imported)) = self.source_at_path(&base_dir.join(&key)) else {
					continue;
				};
				loaded.sources.insert(key.clone(), imported.clone());
				loaded.uris.insert(key.clone(), uri);
				pending.push((Some(key), imported, depth + 1));
			}
		}
		loaded
	}

	fn compile_source(&self, uri: &str, source: &str) -> Compilation {
		let loaded = self.load_sources(uri, source);
		let options = slab_compile::Options {
			base_dir: uri_dir(uri),
			sources: Some(loaded.sources.clone()),
			..Default::default()
		};
		let mut diagnostics = Diagnostics::new();
		let units = slab_compile::import::closure(source, &options, &mut diagnostics);
		let slir = slab_compile::compile_units(&units, &options, &mut diagnostics);
		Compilation { loaded, units, slir, diagnostics }
	}

	fn diagnostic_notifications(&self, primary: &str, affected: HashSet<String>) -> Vec<Value> {
		let mut uris = affected.into_iter().collect::<Vec<_>>();
		uris.sort();
		if let Some(index) = uris.iter().position(|uri| uri == primary) {
			let primary = uris.remove(index);
			uris.insert(0, primary);
		}
		uris
			.into_iter()
			.map(|uri| {
				let mut diagnostics = Vec::new();
				for by_uri in self.diagnostics.values() {
					if let Some(entries) = by_uri.get(&uri) {
						for entry in entries {
							if !diagnostics.contains(entry) {
								diagnostics.push(entry.clone());
							}
						}
					}
				}
				notify(
					"textDocument/publishDiagnostics",
					json!({"uri": uri, "diagnostics": diagnostics}),
				)
			})
			.collect()
	}

	// -- position plumbing
	/// (index, line0, char column) for a positional request.
	fn at<'s>(&'s self, p: &Value) -> Option<(&'s Index, usize, usize)> {
		let uri = p["textDocument"]["uri"].as_str()?;
		let ix = self.index.get(uri)?;
		let pos = &p["position"];
		let line = pos["line"].as_i64().unwrap_or(0).max(0) as usize;
		let line = if ix.lines.is_empty() {
			0
		} else {
			line.min(ix.lines.len() - 1)
		};
		let text = ix.lines.get(line).map_or("", String::as_str);
		let col = u16_to_idx(text, pos["character"].as_i64().unwrap_or(0).max(0) as usize);
		Some((ix, line, col))
	}

	fn owner_uris(&self, uri: &str) -> Vec<String> {
		self
			.closures
			.get(uri)
			.cloned()
			.unwrap_or_else(|| vec![uri.to_string()])
	}

	fn combined_index(&self, uri: &str) -> Option<Index> {
		let mut combined = self.index.get(uri)?.clone();
		combined.defs.clear();
		combined.anims.clear();
		combined.icons.clear();
		combined.list_fields.clear();
		combined.token_paths.clear();
		combined.token_tree.clear();
		combined.group_paths.clear();
		combined.param_paths.clear();

		for owner in self.owner_uris(uri) {
			let Some(index) = self.index.get(&owner) else {
				continue;
			};
			combined.defs.extend(index.defs.clone());
			combined.anims.extend(index.anims.clone());
			combined.icons.extend(index.icons.clone());
			combined.list_fields.extend(index.list_fields.clone());
			combined.token_paths.extend(index.token_paths.clone());
			merge_token_trees(&mut combined.token_tree, &index.token_tree);
			combined.group_paths.extend(index.group_paths.clone());
			for (path, site) in &index.param_paths {
				combined
					.param_paths
					.entry(path.clone())
					.or_insert_with(|| site.clone());
			}
		}
		combined.group_paths.sort();
		combined.group_paths.dedup();
		Some(combined)
	}

	fn import_uri(&self, uri: &str, authored: &str) -> Option<String> {
		let path = lexical_path(&uri_dir(uri).join(authored));
		if let Some((open_uri, _)) = self.docs.iter().find(|(candidate, _)| {
			uri_path(candidate)
				.as_deref()
				.map(lexical_path)
				.is_some_and(|candidate| candidate == path)
		}) {
			return Some(open_uri.clone());
		}
		path.is_file().then(|| path_uri(&path))
	}

	fn import_path_items(uri: &str, prefix: &str) -> Option<Vec<Value>> {
		let trimmed = prefix.trim_start();
		let after_keyword = trimmed.strip_prefix("import")?;
		if !after_keyword.starts_with(char::is_whitespace) {
			return None;
		}
		let partial = after_keyword.trim_start().strip_prefix('"')?;
		if partial.contains('"') {
			return None;
		}
		let (directory, fragment) = partial
			.rsplit_once('/')
			.map_or(("", partial), |(directory, fragment)| (directory, fragment));
		let search_dir = uri_dir(uri).join(directory);
		let mut items = std::fs::read_dir(search_dir)
			.ok()
			.into_iter()
			.flatten()
			.filter_map(Result::ok)
			.filter_map(|entry| {
				let name = entry.file_name().into_string().ok()?;
				if !name.starts_with(fragment) {
					return None;
				}
				let file_type = entry.file_type().ok()?;
				if !file_type.is_dir()
					&& entry.path().extension().and_then(|ext| ext.to_str()) != Some("slab")
				{
					return None;
				}
				let prefix = if directory.is_empty() {
					String::new()
				} else {
					format!("{directory}/")
				};
				let suffix = if file_type.is_dir() { "/" } else { "" };
				let label = format!("{prefix}{name}{suffix}");
				let kind = if file_type.is_dir() { 19 } else { 17 };
				Some(item(&label, kind, "Slab module path", "", None))
			})
			.collect::<Vec<_>>();
		items.sort_by(|left, right| {
			left["label"]
				.as_str()
				.unwrap_or_default()
				.cmp(right["label"].as_str().unwrap_or_default())
		});
		Some(items)
	}

	// -- completion
	fn completion(&self, p: &Value) -> Value {
		let Some((index, line, col)) = self.at(p) else {
			return json!([]);
		};
		let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
		let text = &index.lines[line];
		let prefix_raw: String = text.chars().take(col).collect();
		if let Some(items) = Self::import_path_items(uri, &prefix_raw) {
			return Value::Array(items);
		}
		let prefix = mask_strings(&prefix_raw);
		let context = context(index, line, col);
		let semantic = self.combined_index(uri).unwrap_or_else(|| index.clone());
		let items = complete(&semantic, &prefix, &context);
		let mut seen: Vec<String> = Vec::new();
		let mut out: Vec<Value> = Vec::new();
		for it in items {
			let label = it["label"].as_str().unwrap_or_default().to_string();
			if !seen.contains(&label) {
				seen.push(label);
				out.push(it);
			}
		}
		Value::Array(out)
	}

	// -- hover
	fn hover(&self, p: &Value) -> Value {
		let Some((index, line, col)) = self.at(p) else {
			return Value::Null;
		};
		let Some((tok, prev, nxt)) = tok_at(index, line, col) else {
			return Value::Null;
		};
		let tok = tok.clone();
		let prev = prev.cloned();
		let nxt = nxt.cloned();
		let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
		let markdown = if tok.kind == TokKind::Str
			&& prev
				.as_ref()
				.is_some_and(|token| token.kind == TokKind::Id && token.text == "import")
		{
			self
				.import_uri(uri, &tok.text)
				.map(|target| format!("Imported Slab module [`{}`]({target}).", tok.text))
		} else {
			self
				.combined_index(uri)
				.as_ref()
				.and_then(|semantic| hover_md(semantic, &tok, prev.as_ref(), nxt.as_ref()))
		};
		let Some(markdown) = markdown else {
			return Value::Null;
		};
		json!({
			 "contents": {"kind": "markdown", "value": markdown},
			 "range": rng(&index.lines, tok.line, tok.col, tok.end),
		})
	}

	// -- definition
	fn definition(&self, p: &Value) -> Value {
		let Some((index, line, col)) = self.at(p) else {
			return Value::Null;
		};
		let Some((tok, prev, _)) = tok_at(index, line, col) else {
			return Value::Null;
		};
		let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
		if tok.kind == TokKind::Str
			&& prev.is_some_and(|token| token.kind == TokKind::Id && token.text == "import")
		{
			let Some(owner) = self.import_uri(uri, &tok.text) else {
				return Value::Null;
			};
			let lines = self
				.index
				.get(&owner)
				.map_or(&[] as &[String], |index| index.lines.as_slice());
			return json!({"uri": owner, "range": rng(lines, 0, 0, 0)});
		}

		let owners = self.owner_uris(uri);
		if tok.kind == TokKind::Ref {
			let condition = prev.is_some_and(|token| {
				(token.kind == TokKind::Id && token.text == "when") || token.kind == TokKind::Bang
			});
			let condition_path = format!("param.{}", tok.text);
			if condition {
				for owner in &owners {
					let Some(index) = self.index.get(owner) else {
						continue;
					};
					if let Some((_, line, start, end)) = index.param_paths.get(&condition_path) {
						return json!({
							 "uri": owner,
							 "range": rng(&index.lines, *line, *start, *end),
						});
					}
				}
			}
			for owner in owners.iter().rev() {
				let Some(index) = self.index.get(owner) else {
					continue;
				};
				if let Some((_, line, start, end)) = index.token_paths.get(&tok.text) {
					return json!({
						 "uri": owner,
						 "range": rng(&index.lines, *line, *start, *end),
					});
				}
			}
			for owner in &owners {
				let Some(index) = self.index.get(owner) else {
					continue;
				};
				if let Some((_, line, start, end)) = index.param_paths.get(&tok.text) {
					return json!({
						 "uri": owner,
						 "range": rng(&index.lines, *line, *start, *end),
					});
				}
			}
			return Value::Null;
		}
		if tok.kind == TokKind::Id {
			let follows_icon =
				prev.is_some_and(|token| token.kind == TokKind::Id && token.text == "icon");
			if follows_icon {
				for owner in owners.iter().rev() {
					let Some(index) = self.index.get(owner) else {
						continue;
					};
					if let Some((line, start, end, _)) = index.icons.get(&tok.text) {
						return json!({
							 "uri": owner,
							 "range": rng(&index.lines, *line, *start, *end),
						});
					}
				}
			}
			for owner in owners.iter().rev() {
				let Some(index) = self.index.get(owner) else {
					continue;
				};
				if let Some((line, start, end, _)) = index.defs.get(&tok.text) {
					return json!({
						 "uri": owner,
						 "range": rng(&index.lines, *line, *start, *end),
					});
				}
				if let Some((line, start, end, _)) = index.icons.get(&tok.text) {
					return json!({
						 "uri": owner,
						 "range": rng(&index.lines, *line, *start, *end),
					});
				}
				if let Some((line, start, end)) = index.anims.get(&tok.text) {
					return json!({
						 "uri": owner,
						 "range": rng(&index.lines, *line, *start, *end),
					});
				}
			}
		}
		Value::Null
	}

	// -- symbols
	fn document_symbol(&self, p: &Value) -> Value {
		let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
		let Some(ix) = self.index.get(uri) else {
			return json!([]);
		};
		Value::Array(ix.symbols.iter().map(|s| sym_json(s, &ix.lines)).collect())
	}

	// -- colors
	fn document_color(&self, p: &Value) -> Value {
		let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
		let Some(ix) = self.index.get(uri) else {
			return json!([]);
		};
		Value::Array(
			ix.colors
				.iter()
				.map(|(ln, c0, c1, rgba)| {
					json!({
						 "range": rng(&ix.lines, *ln, *c0, *c1),
						 "color": {
							  "red": rgba[0] as f64 / 255.0,
							  "green": rgba[1] as f64 / 255.0,
							  "blue": rgba[2] as f64 / 255.0,
							  "alpha": rgba[3] as f64 / 255.0,
						 },
					})
				})
				.collect(),
		)
	}

	fn color_presentation(p: &Value) -> Value {
		let c = &p["color"];
		let chan = |k: &str| -> u8 {
			let v = (c[k].as_f64().unwrap_or(0.0) * 255.0).round();
			v.clamp(0.0, 255.0) as u8
		};
		let rgba = [chan("red"), chan("green"), chan("blue"), chan("alpha")];
		json!([{"label": rgba_hex(rgba)}])
	}

	// -- formatting
	/// Whole-document reformat via `slab_syntax::format`; one full-range edit.
	fn formatting(&self, p: &Value) -> Value {
		let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
		let Some(src) = self.docs.get(uri) else {
			return Value::Null;
		};
		let out = slab_syntax::format(src);
		if out == *src {
			return json!([]);
		}
		let last_line = src.split('\n').count() - 1;
		let last_len = u16_len(src.split('\n').next_back().unwrap_or(""));
		json!([{
			 "range": {
				  "start": {"line": 0, "character": 0},
				  "end": {"line": last_line, "character": last_len},
			 },
			 "newText": out,
		}])
	}

	// -- live preview (custom request)
	/// Render the in-memory buffer to SVG: `slab/preview` custom request.
	fn preview(&self, p: &Value) -> Value {
		let uri = p["uri"].as_str().unwrap_or_default();
		let src = self.docs.get(uri).cloned().unwrap_or_default();
		let width = match p.get("width").and_then(Value::as_f64) {
			Some(w) if w > 0.0 => w,
			_ => 800.0,
		};
		let height = p.get("height").and_then(Value::as_f64);
		let states: Vec<String> = p
			.get("states")
			.and_then(Value::as_array)
			.map(|a| {
				a.iter()
					.filter_map(Value::as_str)
					.map(str::to_string)
					.collect()
			})
			.unwrap_or_default();
		let t = p.get("t").and_then(Value::as_f64);

		let base_dir = uri_dir(uri);
		let Compilation { slir, diagnostics, .. } = self.compile_source(uri, &src);
		let mut diags: Vec<Value> = diagnostics
			.0
			.iter()
			.map(|diagnostic| {
				json!({"level": diagnostic.level.to_string(), "code": diagnostic.code,
                       "msg": diagnostic.msg, "line": diagnostic.line,
                       "file": diagnostic.file})
			})
			.collect();
		let empty =
			|diags: Vec<Value>| json!({"svg": "", "width": 0.0, "height": 0.0, "diags": diags});
		let Some(slir) = slir else {
			return empty(diags);
		};
		let bytes = slab_slir::write(&slir);
		let (mut inst, images) = match slab_slir::instance(&bytes) {
			Ok(decoded) => decoded,
			Err(e) => {
				diags.push(json!({"level": "error", "code": "slir",
                    "msg": format!("host decode failed: {e}"), "line": 0}));
				return empty(diags);
			},
		};
		// client 3 = svg (same path as `slab render`)
		slab_kernel::frame::inst_set_env(&mut inst, width, height.unwrap_or(0.0), 3, false, false);
		for st in &states {
			if !st.is_empty() {
				slab_kernel::frame::inst_set_state(&mut inst, st, true);
			}
		}
		let fr = slab_kernel::frame::inst_frame(&mut inst, t.unwrap_or(0.0));
		// layout-time diagnostics accumulate in the kernel instance (§12)
		for k in 0..inst.st.diag_code.len() {
			diags.push(json!({"level": "warning", "code": inst.st.diag_code[k],
                "msg": inst.st.diag_msg[k], "line": inst.st.diag_line[k]}));
		}
		let svg = slab_compile::svg::render_svg(&slir, &images, &[], &[], &fr, &base_dir);
		json!({"svg": svg, "width": fr.width, "height": fr.height, "diags": diags})
	}
}

fn panic_msg(e: &(dyn std::any::Any + Send)) -> String {
	if let Some(s) = e.downcast_ref::<&str>() {
		(*s).to_string()
	} else if let Some(s) = e.downcast_ref::<String>() {
		s.clone()
	} else {
		"panic".to_string()
	}
}

// --------------------------------------------------------- request-side logic

fn notify(method: &str, params: Value) -> Value {
	json!({"jsonrpc": "2.0", "method": method, "params": params})
}

fn merge_token_trees(
	target: &mut std::collections::BTreeMap<String, TNode>,
	source: &std::collections::BTreeMap<String, TNode>,
) {
	for (name, node) in source {
		if let Some(TNode::Group(target_group)) = target.get_mut(name)
			&& let TNode::Group(source_group) = node
		{
			merge_token_trees(target_group, source_group);
			continue;
		}
		target.insert(name.clone(), node.clone());
	}
}

/// LSP range on one line, converting char columns to UTF-16 units.
fn rng(lines: &[String], line: usize, c0: usize, c1: usize) -> Value {
	let text = lines.get(line).map_or("", String::as_str);
	json!({
		 "start": {"line": line, "character": idx_to_u16(text, c0)},
		 "end": {"line": line, "character": idx_to_u16(text, c1)},
	})
}

/// Whole-line range for a 1-based Diag line (0 = file head).
fn diag_range(lines: &[String], dline: u32) -> Value {
	if dline == 0 || lines.is_empty() {
		return json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}});
	}
	let line = ((dline - 1) as usize).min(lines.len() - 1);
	let width = u16_len(&lines[line]).max(1);
	json!({"start": {"line": line, "character": 0}, "end": {"line": line, "character": width}})
}

/// Apply one incremental `TextDocumentContentChangeEvent`.
fn apply_edit(text: &str, rng: &Value, new: &str) -> String {
	let lines: Vec<&str> = text.split('\n').collect();
	// char-offset start of each line
	let mut starts: Vec<usize> = Vec::with_capacity(lines.len());
	let mut off = 0usize;
	for l in &lines {
		starts.push(off);
		off += l.chars().count() + 1;
	}
	let offset = |pos: &Value| -> usize {
		let ln = (pos["line"].as_i64().unwrap_or(0).max(0) as usize).min(lines.len() - 1);
		starts[ln] + u16_to_idx(lines[ln], pos["character"].as_i64().unwrap_or(0).max(0) as usize)
	};
	let (mut a, mut b) = (offset(&rng["start"]), offset(&rng["end"]));
	if b < a {
		std::mem::swap(&mut a, &mut b);
	}
	// char offsets -> byte offsets
	let byte_at = |chars: usize| -> usize {
		text
			.char_indices()
			.nth(chars)
			.map_or(text.len(), |(i, _)| i)
	};
	let (ab, bb) = (byte_at(a), byte_at(b));
	format!("{}{}{}", &text[..ab], new, &text[bb..])
}

/// Blank out string literals so punctuation scans skip their contents.
fn mask_strings(s: &str) -> String {
	let chars: Vec<char> = s.chars().collect();
	let mut out = chars.clone();
	let n = chars.len();
	let mut i = 0usize;
	while i < n {
		if chars[i] == '"' {
			let mut j = i + 1;
			while j < n {
				if chars[j] == '\\' {
					j += 2;
				} else if chars[j] == '"' {
					break;
				} else {
					j += 1;
				}
			}
			for slot in out.iter_mut().take(j.min(n)).skip(i + 1) {
				*slot = ' ';
			}
			i = j + 1;
		} else {
			i += 1;
		}
	}
	out.into_iter().collect()
}

/// Innermost-first block types enclosing (line, col).
fn context(ix: &Index, line: usize, col: usize) -> Vec<&'static str> {
	let pos = (line, col);
	let mut hits: Vec<&crate::index::Block> = ix
		.blocks
		.iter()
		.filter(|b| (b.sline, b.scol) <= pos && pos <= (b.eline, b.ecol))
		.collect();
	hits.sort_by_key(|b| (b.sline, b.scol));
	hits.iter().rev().map(|b| b.btype).collect()
}

/// (token, previous, next) at a position; tolerant of the trailing edge.
fn tok_at(ix: &Index, line: usize, col: usize) -> Option<(&LTok, Option<&LTok>, Option<&LTok>)> {
	let row = ix.by_line.get(&line)?;
	for (i, &ti) in row.iter().enumerate() {
		let t = &ix.toks[ti];
		let next_col = row.get(i + 1).map(|&n| ix.toks[n].col);
		if (t.col <= col && col < t.end) || (col == t.end && next_col.is_none_or(|nc| nc > col)) {
			let prev = if i > 0 {
				Some(&ix.toks[row[i - 1]])
			} else {
				None
			};
			let nxt = row.get(i + 1).map(|&n| &ix.toks[n]);
			return Some((t, prev, nxt));
		}
	}
	None
}

fn item(label: &str, kind: u32, detail: &str, doc: &str, insert: Option<&str>) -> Value {
	let mut it = json!({"label": label, "kind": kind});
	if !detail.is_empty() {
		it["detail"] = json!(detail);
	}
	if !doc.is_empty() {
		it["documentation"] = json!({"kind": "markdown", "value": doc});
	}
	if let Some(ins) = insert {
		it["insertText"] = json!(ins);
	}
	it
}

fn token_ref_items(ix: &Index) -> Vec<Value> {
	ix.token_paths
		.iter()
		.map(|(path, (value, ..))| {
			let kind = if parse_rgba(value).is_some() { 16 } else { 18 };
			item(path, kind, value, &format!("token `{path} = {value}`"), None)
		})
		.collect()
}

fn group_ref_items(ix: &Index) -> Vec<Value> {
	let mut paths: Vec<&String> = ix.group_paths.iter().collect();
	paths.sort();
	paths.dedup();
	paths
		.iter()
		.map(|p| item(p, 18, "token group", &format!("token group `{p}`"), None))
		.collect()
}

fn param_ref_items(ix: &Index) -> Vec<Value> {
	ix.param_paths
		.iter()
		.map(|(path, (value, ..))| item(path, 6, value, &format!("param `{path}` — `{value}`"), None))
		.collect()
}

fn indexed_param_type(value: &str) -> Option<&str> {
	value
		.split(|c: char| c.is_whitespace() || c == '(')
		.find(|part| !part.is_empty())
}

fn semantic_param_compatible(value: &str, allowed: &[&str], allow_bool: bool) -> bool {
	match indexed_param_type(value) {
		Some("bool") => allow_bool,
		Some("enum") => value
			.strip_prefix("enum(")
			.and_then(|rest| rest.split_once(')'))
			.is_some_and(|(members, _)| {
				!members.is_empty()
					&& members
						.split(',')
						.all(|member| allowed.contains(&member.trim()))
			}),
		_ => false,
	}
}

fn semantic_param_items(ix: &Index, allowed: &[&str], allow_bool: bool) -> Vec<Value> {
	ix.param_paths
		.iter()
		.filter(|(_, (value, ..))| semantic_param_compatible(value, allowed, allow_bool))
		.map(|(path, (value, ..))| item(path, 6, value, &format!("compatible param `{path}`"), None))
		.collect()
}

fn typed_param_items(ix: &Index, accepted: &[&str]) -> Vec<Value> {
	ix.param_paths
		.iter()
		.filter(|(_, (value, ..))| indexed_param_type(value).is_some_and(|ty| accepted.contains(&ty)))
		.map(|(path, (value, ..))| item(path, 6, value, &format!("typed param `{path}`"), None))
		.collect()
}

fn text_param_items(ix: &Index) -> Vec<Value> {
	typed_param_items(ix, &["text"])
}

fn list_schema(value: &str) -> Option<&str> {
	value
		.strip_prefix("list(")?
		.split_once(')')
		.map(|(schema, _)| schema)
		.filter(|schema| !schema.is_empty())
}

fn list_param_items(ix: &Index) -> Vec<Value> {
	ix.param_paths
		.iter()
		.filter_map(|(path, (value, ..))| {
			let schema = list_schema(value)?;
			Some(item(
				path,
				6,
				&format!("list({schema})"),
				&format!("root list parameter `{path}: list({schema})`"),
				None,
			))
		})
		.collect()
}

fn list_field_items(ix: &Index) -> Vec<Value> {
	ix.list_fields
		.iter()
		.map(|(field, schema)| {
			item(
				field,
				5,
				&format!("list({schema}) field"),
				&format!("nested list field `{field}: list({schema})`"),
				None,
			)
		})
		.collect()
}

fn icon_items(ix: &Index) -> Vec<Value> {
	ix.icons
		.iter()
		.map(|(name, (_, _, _, sig))| {
			item(name, 21, "icon", &format!("declared vector icon `{sig}`"), None)
		})
		.collect()
}

/// Completions for a value position, specialised by attribute and block.
fn value_items(ix: &Index, attr: &str, valpart: &str, ctx: &[&'static str]) -> Vec<Value> {
	let mut items: Vec<Value> = Vec::new();
	for kw in vocab::attr_values(attr).unwrap_or(&[]) {
		items.push(item(kw, 12, "", vocab::lookup(vocab::VALUE_DOCS, kw).unwrap_or(""), None));
	}
	if ctx.contains(&"icon") && matches!(attr, "bg" | "stroke") {
		items.push(item(
			"current",
			12,
			"icon paint",
			vocab::lookup(vocab::VALUE_DOCS, "current").unwrap_or(""),
			None,
		));
	}
	if matches!(attr, "scroll" | "gravity" | "collide") {
		return items;
	}
	if attr == "checked" {
		items.extend(semantic_param_items(ix, &["false", "true", "mixed"], true));
		return items;
	}
	if attr == "live" {
		items.extend(semantic_param_items(ix, &["off", "polite", "assertive"], false));
		return items;
	}
	if matches!(attr, "expanded" | "selected" | "modal" | "live-atomic") {
		items.extend(typed_param_items(ix, &["bool"]));
		return items;
	}
	if matches!(attr, "active-descendant" | "controls" | "value-text") {
		items.extend(text_param_items(ix));
		return items;
	}
	if matches!(attr, "value-now" | "value-min" | "value-max" | "level" | "pos-in-set" | "set-size")
	{
		items.extend(typed_param_items(ix, &["num"]));
		return items;
	}
	if matches!(
		attr,
		"act"
			| "field"
			| "submit"
			| "cancel"
			| "keys"
			| "press"
			| "context"
			| "dblclick"
			| "drag"
			| "pointer-move"
			| "pointer-up"
			| "drag-update"
			| "drag-end"
			| "drop"
			| "resize"
			| "role"
			| "item-extent"
			| "overscan"
			| "viewbox"
	) {
		return items;
	}
	if matches!(attr, "src" | "d" | "attach" | "label" | "desc") {
		items.extend(text_param_items(ix));
		return items;
	}
	if matches!(attr, "w" | "h" | "min-w" | "max-w" | "min-h" | "max-h" | "cols") {
		for kw in vocab::SIZING {
			if !items.iter().any(|i| i["label"] == *kw) {
				items.push(item(kw, 12, "", vocab::lookup(vocab::VALUE_DOCS, kw).unwrap_or(""), None));
			}
		}
	}
	if attr == "animate" && !valpart.contains(',') {
		items = ix
			.anims
			.keys()
			.map(|a| item(a, 23, "anim", &format!("keyframe animation `{a}`"), None))
			.collect();
	} else if attr == "animate" {
		let mut anims: Vec<Value> = ix
			.anims
			.keys()
			.map(|a| item(a, 23, "anim", "", None))
			.collect();
		anims.extend(items);
		items = anims;
	}
	if attr == "style" || attr == "shadow" {
		items.extend(group_ref_items(ix));
	}
	items.extend(token_ref_items(ix));
	items.extend(param_ref_items(ix));
	items
}

const fn is_ident_start(c: char) -> bool {
	c.is_ascii_alphabetic() || c == '_'
}

const fn is_ident_char(c: char) -> bool {
	c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Dotted-path suffix `base.[partial]` at the end of the prefix, if any.
fn dotted_suffix(prefix: &str) -> Option<String> {
	let chars: Vec<char> = prefix.chars().collect();
	let mut i = chars.len();
	// optional partial segment
	let part_end = i;
	while i > 0 && is_ident_char(chars[i - 1]) {
		i -= 1;
	}
	let partial: String = chars[i..part_end].iter().collect();
	if !partial.is_empty() && !is_ident_start(chars[i]) {
		return None;
	}
	if i == 0 || chars[i - 1] != '.' {
		return None;
	}
	i -= 1; // consume the '.'
	// one or more dot-separated segments before it
	let mut segs: Vec<String> = Vec::new();
	loop {
		let end = i;
		while i > 0 && is_ident_char(chars[i - 1]) {
			i -= 1;
		}
		if end == i || !is_ident_start(chars[i]) {
			return None;
		}
		segs.push(chars[i..end].iter().collect());
		if i > 0 && chars[i - 1] == '.' {
			i -= 1;
		} else {
			break;
		}
	}
	segs.reverse();
	Some(segs.join("."))
}

/// Trailing attribute name before `=` (allowing trailing whitespace).
fn trailing_attr(before_eq: &str) -> Option<String> {
	let chars: Vec<char> = before_eq.chars().collect();
	let mut i = chars.len();
	while i > 0 && (chars[i - 1] == ' ' || chars[i - 1] == '\t') {
		i -= 1;
	}
	let end = i;
	while i > 0 && is_ident_char(chars[i - 1]) {
		i -= 1;
	}
	if end == i || !is_ident_start(chars[i]) {
		return None;
	}
	Some(chars[i..end].iter().collect())
}

fn has_attr(prefix: &str, name: &str) -> bool {
	prefix.match_indices(name).any(|(start, _)| {
		let begins_name = prefix[..start]
			.chars()
			.next_back()
			.is_none_or(|c| !is_ident_char(c));
		begins_name && prefix[start + name.len()..].trim_start().starts_with('=')
	})
}

fn trailing_call_arg<'a>(prefix: &'a str, name: &str) -> Option<&'a str> {
	let name_start = prefix.rfind(name)?;
	if prefix[..name_start]
		.chars()
		.next_back()
		.is_some_and(is_ident_char)
	{
		return None;
	}
	let after_name = &prefix[name_start + name.len()..];
	let arg = after_name.strip_prefix('(')?;
	(!arg.contains(')') && arg.chars().all(is_ident_char)).then_some(arg)
}

/// Context-sensitive completion from the masked line prefix.
fn complete(ix: &Index, prefix: &str, ctx: &[&'static str]) -> Vec<Value> {
	let pchars: Vec<char> = prefix.chars().collect();
	let seg: String = match pchars.iter().rposition(|&c| c == '{' || c == ';') {
		Some(i) => pchars[i + 1..].iter().collect(),
		None => prefix.to_string(),
	};
	let trimmed = seg.trim_start();
	let head = trimmed.split_whitespace().next().unwrap_or("");
	let value_attr = pchars
		.iter()
		.rposition(|&c| c == '=')
		.and_then(|eq| trailing_attr(&pchars[..eq].iter().collect::<String>()));

	// After `.`: next token-path segments. `each param.` only offers lists.
	if let Some(base) = dotted_suffix(prefix) {
		if head == "when" && base != "param" {
			let prefix = format!("param.{base}.");
			let items = ix
				.param_paths
				.iter()
				.filter(|(path, (value, ..))| {
					path.starts_with(&prefix) && indexed_param_type(value) == Some("bool")
				})
				.map(|(path, (value, ..))| {
					let name = path.strip_prefix(&prefix).unwrap_or(path);
					item(name, 6, value, "Boolean parameter condition.", None)
				})
				.collect::<Vec<_>>();
			if !items.is_empty() {
				return items;
			}
		}
		if let Some(group) = base.strip_prefix("param.") {
			let prefix = format!("param.{group}.");
			let items = ix
				.param_paths
				.iter()
				.filter(|(path, _)| path.starts_with(&prefix))
				.map(|(path, (value, ..))| {
					let name = path.strip_prefix(&prefix).unwrap_or(path);
					item(name, 6, value, &format!("param `{name}` — `{value}`"), None)
				})
				.collect::<Vec<_>>();
			if !items.is_empty() {
				return items;
			}
		}
		if base == "param" && !ix.param_paths.is_empty() {
			return ix
				.param_paths
				.iter()
				.filter(|(_, (value, ..))| {
					if head == "each" {
						return list_schema(value).is_some();
					}
					let param_ty = indexed_param_type(value);
					if head == "icon"
						|| matches!(
							value_attr.as_deref(),
							Some(
								"src"
									| "d" | "attach" | "label"
									| "desc" | "active-descendant"
									| "controls" | "value-text"
							)
						) {
						return param_ty == Some("text");
					}
					if value_attr.as_deref() == Some("checked") {
						return semantic_param_compatible(value, &["false", "true", "mixed"], true);
					}
					if value_attr.as_deref() == Some("live") {
						return semantic_param_compatible(value, &["off", "polite", "assertive"], false);
					}
					if matches!(
						value_attr.as_deref(),
						Some("expanded" | "selected" | "modal" | "live-atomic")
					) {
						return param_ty == Some("bool");
					}
					if matches!(
						value_attr.as_deref(),
						Some(
							"value-now" | "value-min" | "value-max" | "level" | "pos-in-set" | "set-size"
						)
					) {
						return param_ty == Some("num");
					}
					!matches!(
						value_attr.as_deref(),
						Some(
							"act"
								| "field" | "submit"
								| "cancel" | "keys"
								| "press" | "context"
								| "dblclick" | "pointer-move"
								| "pointer-up" | "drag"
								| "drag-update" | "drag-end"
								| "drop" | "resize"
								| "role" | "item-extent"
								| "overscan" | "viewbox"
								| "scroll" | "gravity"
								| "collide"
						)
					)
				})
				.map(|(path, (value, ..))| {
					let name = path.strip_prefix("param.").unwrap_or(path);
					item(name, 6, value, &format!("param `{name}` — `{value}`"), None)
				})
				.collect();
		}
		let mut node: Option<&crate::index::TNode> = None;
		let mut tree = Some(&ix.token_tree);
		for segment in base.split('.') {
			node = tree.and_then(|tokens| tokens.get(segment));
			tree = match node {
				Some(crate::index::TNode::Group(group)) => Some(group),
				_ => None,
			};
		}
		if let Some(crate::index::TNode::Group(group)) = node {
			return group
				.iter()
				.map(|(name, sub)| match sub {
					crate::index::TNode::Group(_) => item(name, 18, "group", "", None),
					crate::index::TNode::Leaf(value) => {
						let kind = if parse_rgba(value).is_some() { 16 } else { 18 };
						item(name, kind, value, "", None)
					},
				})
				.collect();
		}
	}

	// `list(` names an exported component schema in params or a def field.
	if trailing_call_arg(prefix, "list").is_some() {
		return ix
			.defs
			.iter()
			.filter(|(_, (_, _, _, sig))| sig.ends_with(" export"))
			.map(|(name, (_, _, _, sig))| {
				item(name, 7, "list schema", &format!("typed list item schema `{sig}`"), None)
			})
			.collect();
	}

	// Positional targets have a smaller, typed vocabulary than node headers.
	if let Some(rest) = trimmed.strip_prefix("icon")
		&& (rest.starts_with(' ') || rest.starts_with('\t'))
	{
		let target = rest.trim_start();
		if target.is_empty() || (!target.chars().any(char::is_whitespace) && !target.contains('=')) {
			let mut items = icon_items(ix);
			items.extend(text_param_items(ix));
			return items;
		}
	}
	if let Some(rest) = trimmed.strip_prefix("each")
		&& (rest.starts_with(' ') || rest.starts_with('\t'))
	{
		let target = rest.trim_start();
		if target.is_empty() || (!target.chars().any(char::is_whitespace) && !target.contains('=')) {
			let mut items = list_param_items(ix);
			items.extend(list_field_items(ix));
			return items;
		}
	}
	if let Some(rest) = trimmed.strip_prefix("when")
		&& (rest.starts_with(' ') || rest.starts_with('\t'))
	{
		let condition = rest.trim_start();
		if condition.is_empty() || !condition.chars().any(char::is_whitespace) {
			let mut items: Vec<Value> = vocab::CONDITIONS
				.iter()
				.map(|name| {
					item(
						name,
						12,
						"condition",
						vocab::lookup(vocab::VALUE_DOCS, name)
							.unwrap_or("Kernel state, environment flag, or renderer condition."),
						None,
					)
				})
				.collect();
			items.extend(
				ix.param_paths
					.iter()
					.filter(|(_, (value, ..))| indexed_param_type(value) == Some("bool"))
					.map(|(path, (value, ..))| {
						let name = path.strip_prefix("param.").unwrap_or(path);
						item(name, 6, value, "Boolean parameter condition.", None)
					}),
			);
			items.push(item(
				"theme",
				3,
				"condition",
				"Match an authored theme by name.",
				Some("theme("),
			));
			return items;
		}
	}

	// Value position: text after the last bare `=`.
	if let Some(eq) = pchars.iter().rposition(|&c| c == '=')
		&& eq > 0
		&& !matches!(pchars[eq - 1], '<' | '>' | '!')
	{
		let before: String = pchars[..eq].iter().collect();
		if let Some(attr) = trailing_attr(&before) {
			let valpart: String = pchars[eq + 1..].iter().collect();
			return value_items(ix, &attr, &valpart, ctx);
		}
	}

	let seg_is_stmt = trimmed.is_empty()
		|| (trimmed.chars().all(is_ident_char) && trimmed.chars().next().is_some_and(is_ident_start));
	if ctx.first().is_some_and(|scope| *scope == "params")
		&& !trimmed.contains('=')
		&& trimmed.chars().any(char::is_whitespace)
	{
		return vocab::PARAM_TYPES
			.iter()
			.map(|ty| {
				let insert = match *ty {
					"enum" | "list" => Some(format!("{ty}(")),
					_ => None,
				};
				item(
					ty,
					25,
					"parameter type",
					vocab::lookup(vocab::TYPE_DOCS, ty).unwrap_or(""),
					insert.as_deref(),
				)
			})
			.collect();
	}
	if seg_is_stmt {
		if ctx.first().is_some_and(|scope| *scope == "tokens") {
			return Vec::new();
		}
		if ctx.first().is_some_and(|scope| *scope == "params") {
			return Vec::new();
		}
		let parent = ctx.first().copied();
		let mut items: Vec<Value> = vocab::builtins()
			.filter(|name| {
				if parent == Some("icon") {
					return *name == "path";
				}
				if parent == Some("para") {
					return *name == "span";
				}
				*name != "divider" || matches!(parent, Some("row" | "col"))
			})
			.map(|name| {
				item(name, 7, "builtin", vocab::lookup(vocab::NODE_DOCS, name).unwrap_or(""), None)
			})
			.collect();
		if parent != Some("icon") && parent != Some("para") {
			items.extend(
				ix.defs.iter().map(|(name, (_, _, _, sig))| {
					item(name, 7, sig, &format!("component `{sig}`"), None)
				}),
			);
		}
		if parent != Some("icon") {
			items.push(item(
				"when",
				14,
				"",
				vocab::lookup(vocab::KEYWORD_DOCS, "when").unwrap_or(""),
				None,
			));
		}
		if !ctx.is_empty() && parent != Some("icon") {
			items.push(item(
				"each",
				14,
				"",
				vocab::lookup(vocab::KEYWORD_DOCS, "each").unwrap_or(""),
				Some("each param."),
			));
		}
		if ctx.is_empty() {
			for keyword in ["def", "import", "tokens", "theme", "anim", "params", "icon"] {
				items.push(item(
					keyword,
					14,
					"",
					vocab::lookup(vocab::KEYWORD_DOCS, keyword).unwrap_or(""),
					None,
				));
			}
		}
		if ctx.contains(&"def") {
			items.push(item(
				"slot",
				14,
				"",
				vocab::lookup(vocab::NODE_DOCS, "slot").unwrap_or(""),
				None,
			));
		}
		return items;
	}

	// `each` has its own compact header: no generic node attributes or flags.
	if head == "each" {
		let mut items: Vec<Value> = vocab::ATTR_DOCS
			.iter()
			.filter(|(name, _)| matches!(*name, "key" | "item-extent" | "overscan"))
			.map(|(name, doc)| item(name, 10, "each attribute", doc, Some(&format!("{name}="))))
			.collect();
		items.push(item(
			"virtual",
			14,
			"each flag",
			vocab::lookup(vocab::FLAG_DOCS, "virtual").unwrap_or(""),
			None,
		));
		return items;
	}

	// General node header, with placement-specific attributes filtered.
	let parent = ctx.first().copied();
	let mut items: Vec<Value> = vocab::ATTR_DOCS
		.iter()
		.filter(|(name, _)| match *name {
			"item-extent" | "overscan" => false,
			"viewbox" => head == "icon",
			"attach" | "gravity" | "collide" => matches!(parent, Some("stack" | "canvas")),
			"drag-update" | "drag-end" => has_attr(trimmed, "drag"),
			"resize" => head == "divider",
			"scroll" => false,
			_ => true,
		})
		.map(|(name, doc)| item(name, 10, "attribute", doc, Some(&format!("{name}="))))
		.collect();
	if vocab::CONTAINERS.contains(&head) || head == "hole" {
		let doc = vocab::lookup(vocab::ATTR_DOCS, "scroll").unwrap_or("");
		for mode in ["cross", "both"] {
			items.push(item(
				&format!("scroll={mode}"),
				12,
				"scroll mode",
				doc,
				Some(&format!("scroll={mode}")),
			));
		}
	}
	let mut flags: Vec<&str> = FLAGS
		.iter()
		.copied()
		.filter(|flag| *flag != "virtual" && (*flag != "drag-ghost" || has_attr(trimmed, "drag")))
		.collect();
	flags.sort_unstable();
	items.extend(flags.iter().map(|flag| {
		item(flag, 14, "flag", vocab::lookup(vocab::FLAG_DOCS, flag).unwrap_or(""), None)
	}));
	items
}

/// Markdown hover text for the token under the cursor.
fn hover_md(ix: &Index, tok: &LTok, prev: Option<&LTok>, nxt: Option<&LTok>) -> Option<String> {
	if tok.kind == TokKind::Ref {
		if let Some((value, ..)) = ix.token_paths.get(&tok.text) {
			return Some(format!("`{} = {}`", tok.text, value));
		}
		if let Some((value, ..)) = ix.param_paths.get(&tok.text) {
			return Some(format!("`{}` — param `{}`", tok.text, value));
		}
		if prev.is_some_and(|token| {
			(token.kind == TokKind::Id && token.text == "when") || token.kind == TokKind::Bang
		}) && let Some((value, ..)) = ix.param_paths.get(&format!("param.{}", tok.text))
		{
			return Some(format!("`{}` — param `{}`", tok.text, value));
		}
		return Some(format!("`{}` — unresolved token reference", tok.text));
	}
	if tok.kind != TokKind::Id {
		return None;
	}
	let word = tok.text.as_str();
	let scopes = context(ix, tok.line, tok.col);
	let follows_icon = prev.is_some_and(|token| token.kind == TokKind::Id && token.text == "icon");
	if follows_icon && let Some((_, _, _, sig)) = ix.icons.get(word) {
		return Some(format!("```slab\n{sig}\n```\nNamed vector icon."));
	}
	if let Some((_, _, _, sig)) = ix.defs.get(word) {
		return Some(format!("```slab\n{sig}\n```"));
	}
	if let Some((_, _, _, sig)) = ix.icons.get(word) {
		return Some(format!("```slab\n{sig}\n```\nNamed vector icon."));
	}
	if prev.is_some_and(|token| token.kind == TokKind::Id && token.text == "each")
		&& let Some(schema) = ix.list_fields.get(word)
	{
		return Some(format!("`each {word}` — nested list field with item schema `list({schema})`."));
	}

	if prev.is_some_and(|token| {
		(token.kind == TokKind::Id && token.text == "when") || token.kind == TokKind::Bang
	}) && let Some(doc) = vocab::lookup(vocab::VALUE_DOCS, word)
	{
		return Some(format!("`{word}` *(state)* — {doc}"));
	}
	let in_value =
		prev.is_some_and(|token| matches!(token.kind, TokKind::Eq | TokKind::Comma | TokKind::Colon));
	if in_value {
		if ix.anims.contains_key(word) {
			return Some(format!("`anim {word}` — keyframe animation defined in this document."));
		}
		if let Some(doc) = vocab::lookup(vocab::VALUE_DOCS, word) {
			return Some(format!("`{word}` — {doc}"));
		}
	}
	if nxt.is_some_and(|token| token.kind == TokKind::Eq)
		&& let Some(doc) = vocab::lookup(vocab::ATTR_DOCS, word)
	{
		return Some(format!("`{word}` — {doc}"));
	}
	let in_type_position = scopes.first().is_some_and(|scope| *scope == "params")
		|| (word == "list" && nxt.is_some_and(|token| token.kind == TokKind::Lp));
	if in_type_position && let Some(doc) = vocab::lookup(vocab::TYPE_DOCS, word) {
		return Some(format!("`{word}` *(type)* — {doc}"));
	}
	if word == "icon" {
		let usage = vocab::lookup(vocab::NODE_DOCS, word).unwrap_or("");
		let declaration = vocab::lookup(vocab::KEYWORD_DOCS, word).unwrap_or("");
		return Some(format!("`icon` — {usage}\n\n{declaration}"));
	}
	if let Some(doc) = vocab::lookup(vocab::NODE_DOCS, word) {
		return Some(format!("`{word}` — {doc}"));
	}
	if let Some(doc) = vocab::lookup(vocab::FLAG_DOCS, word) {
		return Some(format!("`{word}` *(flag)* — {doc}"));
	}
	if let Some(doc) = vocab::lookup(vocab::KEYWORD_DOCS, word) {
		return Some(format!("`{word}` — {doc}"));
	}
	if let Some(doc) = vocab::lookup(vocab::TYPE_DOCS, word) {
		return Some(format!("`{word}` *(type)* — {doc}"));
	}
	if let Some(doc) = vocab::lookup(vocab::ATTR_DOCS, word) {
		return Some(format!("`{word}` — {doc}"));
	}
	if let Some(doc) = vocab::lookup(vocab::VALUE_DOCS, word) {
		return Some(format!("`{word}` — {doc}"));
	}
	if ix.anims.contains_key(word) {
		return Some(format!("`anim {word}` — keyframe animation defined in this document."));
	}
	None
}

fn sym_json(s: &Sym, lines: &[String]) -> Value {
	let r = rng(lines, s.line, s.col, s.end);
	let mut out = json!({
		 "name": s.name,
		 "kind": s.kind,
		 "range": r,
		 "selectionRange": r,
		 "children": s.children.iter().map(|c| sym_json(c, lines)).collect::<Vec<_>>(),
	});
	if !s.detail.is_empty() {
		out["detail"] = json!(s.detail);
	}
	out
}
