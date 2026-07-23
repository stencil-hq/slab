//! Single-threaded LSP server; `handle(msg)` returns the outgoing messages
//! (response and/or notifications), so the whole protocol is testable
//! in-memory. Positions are UTF-16 code units per LSP; internal columns are
//! `char` indices.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::index::{Index, LTok, Sym, build_index};
use crate::vocab;
use slab_compile::color::{Rgba, parse_rgba};
use slab_syntax::diag::Level;
use slab_syntax::lex::TokKind;
use slab_syntax::parse::FLAGS;

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

fn severity(level: Level) -> i64 {
    match level {
        Level::Error => 1,
        Level::Warning => 2,
        Level::Note => 3,
    }
}

/// `#rrggbb`, or `#rrggbbaa` when alpha < 255.
fn rgba_hex(c: Rgba) -> String {
    if c[3] >= 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}

/// Directory of a `file://` uri (percent-decoded); `.` otherwise.
fn uri_dir(uri: &str) -> PathBuf {
    if let Some(path) = uri.strip_prefix("file://") {
        let decoded = percent_decode(path);
        let p = PathBuf::from(decoded);
        if let Some(parent) = p.parent() {
            return parent.to_path_buf();
        }
    }
    PathBuf::from(".")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// --------------------------------------------------------------------- server

/// Single-threaded LSP server over in-memory messages.
pub struct Server {
    pub docs: HashMap<String, String>,
    pub index: HashMap<String, Index>,
    pub running: bool,
    pub exit_code: i32,
    shutdown: bool,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub fn new() -> Self {
        Server {
            docs: HashMap::new(),
            index: HashMap::new(),
            running: true,
            exit_code: 1,
            shutdown: false,
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
            let known = self.is_known(&method);
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
                }
                Err(e) => {
                    // never crash the loop on a bad request
                    let detail = panic_msg(&e);
                    vec![json!({"jsonrpc": "2.0", "id": mid, "error":
                        {"code": -32603, "message": format!("internal error: {detail}")}})]
                }
            }
        } else {
            if !self.is_known(&method) {
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

    fn is_known(&self, method: &str) -> bool {
        matches!(
            method,
            "initialize"
                | "initialized"
                | "shutdown"
                | "exit"
                | "$/cancelRequest"
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
            "initialize" => (self.initialize(), Vec::new()),
            "shutdown" => {
                self.shutdown = true;
                (Value::Null, Vec::new())
            }
            "exit" => {
                self.exit_code = if self.shutdown { 0 } else { 1 };
                self.running = false;
                (Value::Null, Vec::new())
            }
            "textDocument/didOpen" => (Value::Null, self.did_open(p)),
            "textDocument/didChange" => (Value::Null, self.did_change(p)),
            "textDocument/didClose" => (Value::Null, self.did_close(p)),
            "textDocument/completion" => (self.completion(p), Vec::new()),
            "textDocument/hover" => (self.hover(p), Vec::new()),
            "textDocument/definition" => (self.definition(p), Vec::new()),
            "textDocument/documentSymbol" => (self.document_symbol(p), Vec::new()),
            "textDocument/documentColor" => (self.document_color(p), Vec::new()),
            "textDocument/colorPresentation" => (self.color_presentation(p), Vec::new()),
            "textDocument/formatting" => (self.formatting(p), Vec::new()),
            "slab/preview" => (self.preview(p), Vec::new()),
            // initialized, didSave, $/cancelRequest
            _ => (json!([]), Vec::new()),
        }
    }

    // -- lifecycle
    fn initialize(&self) -> Value {
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
        self.refresh(&uri)
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
        self.refresh(&uri)
    }

    fn did_close(&mut self, p: &Value) -> Vec<Value> {
        let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
        self.docs.remove(uri);
        self.index.remove(uri);
        vec![notify(
            "textDocument/publishDiagnostics",
            json!({"uri": uri, "diagnostics": []}),
        )]
    }

    /// Re-index + re-compile the document, emitting publishDiagnostics.
    fn refresh(&mut self, uri: &str) -> Vec<Value> {
        let src = self.docs.get(uri).cloned().unwrap_or_default();
        let ix = build_index(&src);
        let opts = slab_compile::Options {
            base_dir: uri_dir(uri),
            ..Default::default()
        };
        let (_slir, reported) = slab_compile::compile(&src, &opts);
        let diags: Vec<Value> = reported
            .0
            .iter()
            .map(|d| {
                json!({
                    "range": diag_range(&ix.lines, d.line),
                    "severity": severity(d.level),
                    "code": d.code,
                    "source": "slab",
                    "message": d.msg,
                })
            })
            .collect();
        self.index.insert(uri.to_string(), ix);
        vec![notify(
            "textDocument/publishDiagnostics",
            json!({"uri": uri, "diagnostics": diags}),
        )]
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
        let text = ix.lines.get(line).map(String::as_str).unwrap_or("");
        let col = u16_to_idx(text, pos["character"].as_i64().unwrap_or(0).max(0) as usize);
        Some((ix, line, col))
    }

    // -- completion
    fn completion(&self, p: &Value) -> Value {
        let Some((ix, line, col)) = self.at(p) else {
            return json!([]);
        };
        let text = &ix.lines[line];
        let prefix_raw: String = text.chars().take(col).collect();
        let prefix = mask_strings(&prefix_raw);
        let items = complete(ix, &prefix, &context(ix, line, col));
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
        let Some((ix, line, col)) = self.at(p) else {
            return Value::Null;
        };
        let Some((tok, prev, nxt)) = tok_at(ix, line, col) else {
            return Value::Null;
        };
        let Some(md) = hover_md(ix, tok, prev, nxt) else {
            return Value::Null;
        };
        json!({
            "contents": {"kind": "markdown", "value": md},
            "range": rng(&ix.lines, tok.line, tok.col, tok.end),
        })
    }

    // -- definition
    fn definition(&self, p: &Value) -> Value {
        let Some((ix, line, col)) = self.at(p) else {
            return Value::Null;
        };
        let Some((tok, prev, _)) = tok_at(ix, line, col) else {
            return Value::Null;
        };
        let uri = p["textDocument"]["uri"].as_str().unwrap_or_default();
        if tok.kind == TokKind::Ref {
            if let Some((_, ln, c0, c1)) = ix.token_paths.get(&tok.text) {
                return json!({"uri": uri, "range": rng(&ix.lines, *ln, *c0, *c1)});
            }
            if let Some((_, ln, c0, c1)) = ix.param_paths.get(&tok.text) {
                return json!({"uri": uri, "range": rng(&ix.lines, *ln, *c0, *c1)});
            }
            return Value::Null;
        }
        if tok.kind == TokKind::Id {
            if prev.is_some_and(|token| token.kind == TokKind::Id && token.text == "icon")
                && let Some((ln, c0, c1, _)) = ix.icons.get(&tok.text)
            {
                return json!({"uri": uri, "range": rng(&ix.lines, *ln, *c0, *c1)});
            }
            if let Some((ln, c0, c1, _)) = ix.defs.get(&tok.text) {
                return json!({"uri": uri, "range": rng(&ix.lines, *ln, *c0, *c1)});
            }
            if let Some((ln, c0, c1, _)) = ix.icons.get(&tok.text) {
                return json!({"uri": uri, "range": rng(&ix.lines, *ln, *c0, *c1)});
            }
            if let Some((ln, c0, c1)) = ix.anims.get(&tok.text) {
                return json!({"uri": uri, "range": rng(&ix.lines, *ln, *c0, *c1)});
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

    fn color_presentation(&self, p: &Value) -> Value {
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
    fn preview(&mut self, p: &Value) -> Value {
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
        let opts = slab_compile::Options {
            base_dir: base_dir.clone(),
            ..Default::default()
        };
        let (slir, reported) = slab_compile::compile(&src, &opts);
        let mut diags: Vec<Value> = reported
            .0
            .iter()
            .map(|d| {
                json!({"level": d.level.to_string(), "code": d.code,
                       "msg": d.msg, "line": d.line})
            })
            .collect();
        let empty =
            |diags: Vec<Value>| json!({"svg": "", "width": 0.0, "height": 0.0, "diags": diags});
        let Some(slir) = slir else {
            return empty(diags);
        };
        if reported.has_errors() {
            return empty(diags);
        }
        let bytes = slab_slir::write(&slir);
        let (mut inst, images) = match slab_slir::instance(&bytes) {
            Ok(decoded) => decoded,
            Err(e) => {
                diags.push(json!({"level": "error", "code": "slir",
                    "msg": format!("host decode failed: {e}"), "line": 0}));
                return empty(diags);
            }
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

/// LSP range on one line, converting char columns to UTF-16 units.
fn rng(lines: &[String], line: usize, c0: usize, c1: usize) -> Value {
    let text = lines.get(line).map(String::as_str).unwrap_or("");
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

/// Apply one incremental TextDocumentContentChangeEvent.
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
        starts[ln]
            + u16_to_idx(
                lines[ln],
                pos["character"].as_i64().unwrap_or(0).max(0) as usize,
            )
    };
    let (mut a, mut b) = (offset(&rng["start"]), offset(&rng["end"]));
    if b < a {
        std::mem::swap(&mut a, &mut b);
    }
    // char offsets -> byte offsets
    let byte_at = |chars: usize| -> usize {
        text.char_indices()
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
        .map(|(path, (value, _, _, _))| {
            let kind = if parse_rgba(value).is_some() { 16 } else { 18 };
            item(
                path,
                kind,
                value,
                &format!("token `{path} = {value}`"),
                None,
            )
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
        .map(|(path, (value, _, _, _))| {
            item(path, 6, value, &format!("param `{path}` — `{value}`"), None)
        })
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
        .filter(|(_, (value, _, _, _))| semantic_param_compatible(value, allowed, allow_bool))
        .map(|(path, (value, _, _, _))| {
            item(path, 6, value, &format!("compatible param `{path}`"), None)
        })
        .collect()
}

fn typed_param_items(ix: &Index, accepted: &[&str]) -> Vec<Value> {
    ix.param_paths
        .iter()
        .filter(|(_, (value, _, _, _))| {
            indexed_param_type(value).is_some_and(|ty| accepted.contains(&ty))
        })
        .map(|(path, (value, _, _, _))| {
            item(path, 6, value, &format!("typed param `{path}`"), None)
        })
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
        .filter_map(|(path, (value, _, _, _))| {
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
            item(
                name,
                21,
                "icon",
                &format!("declared vector icon `{sig}`"),
                None,
            )
        })
        .collect()
}

/// Completions for a value position, specialised by attribute and block.
fn value_items(ix: &Index, attr: &str, valpart: &str, ctx: &[&'static str]) -> Vec<Value> {
    let mut items: Vec<Value> = Vec::new();
    for kw in vocab::attr_values(attr).unwrap_or(&[]) {
        items.push(item(
            kw,
            12,
            "",
            vocab::lookup(vocab::VALUE_DOCS, kw).unwrap_or(""),
            None,
        ));
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
        items.extend(semantic_param_items(
            ix,
            &["off", "polite", "assertive"],
            false,
        ));
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
    if matches!(
        attr,
        "value-now" | "value-min" | "value-max" | "level" | "pos-in-set" | "set-size"
    ) {
        items.extend(typed_param_items(ix, &["num"]));
        return items;
    }
    if matches!(
        attr,
        "act"
            | "field"
            | "submit"
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
    if matches!(
        attr,
        "w" | "h" | "min-w" | "max-w" | "min-h" | "max-h" | "cols"
    ) {
        for kw in vocab::SIZING {
            if !items.iter().any(|i| i["label"] == *kw) {
                items.push(item(
                    kw,
                    12,
                    "",
                    vocab::lookup(vocab::VALUE_DOCS, kw).unwrap_or(""),
                    None,
                ));
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

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
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
        if base == "param" && !ix.param_paths.is_empty() {
            return ix
                .param_paths
                .iter()
                .filter(|(_, (value, _, _, _))| {
                    if head == "each" {
                        return list_schema(value).is_some();
                    }
                    let param_ty = indexed_param_type(value);
                    if head == "icon"
                        || matches!(
                            value_attr.as_deref(),
                            Some(
                                "src"
                                    | "d"
                                    | "attach"
                                    | "label"
                                    | "desc"
                                    | "active-descendant"
                                    | "controls"
                                    | "value-text"
                            )
                        )
                    {
                        return param_ty == Some("text");
                    }
                    if value_attr.as_deref() == Some("checked") {
                        return semantic_param_compatible(value, &["false", "true", "mixed"], true);
                    }
                    if value_attr.as_deref() == Some("live") {
                        return semantic_param_compatible(
                            value,
                            &["off", "polite", "assertive"],
                            false,
                        );
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
                            "value-now"
                                | "value-min"
                                | "value-max"
                                | "level"
                                | "pos-in-set"
                                | "set-size"
                        )
                    ) {
                        return param_ty == Some("num");
                    }
                    !matches!(
                        value_attr.as_deref(),
                        Some(
                            "act"
                                | "field"
                                | "submit"
                                | "keys"
                                | "press"
                                | "context"
                                | "dblclick"
                                | "pointer-move"
                                | "pointer-up"
                                | "drag"
                                | "drag-update"
                                | "drag-end"
                                | "drop"
                                | "resize"
                                | "role"
                                | "item-extent"
                                | "overscan"
                                | "viewbox"
                                | "scroll"
                                | "gravity"
                                | "collide"
                        )
                    )
                })
                .map(|(path, (value, _, _, _))| {
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
                    }
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
                item(
                    name,
                    7,
                    "list schema",
                    &format!("typed list item schema `{sig}`"),
                    None,
                )
            })
            .collect();
    }

    // Positional targets have a smaller, typed vocabulary than node headers.
    if let Some(rest) = trimmed.strip_prefix("icon")
        && (rest.starts_with(' ') || rest.starts_with('\t'))
    {
        let target = rest.trim_start();
        if target.is_empty() || (!target.chars().any(char::is_whitespace) && !target.contains('='))
        {
            let mut items = icon_items(ix);
            items.extend(text_param_items(ix));
            return items;
        }
    }
    if let Some(rest) = trimmed.strip_prefix("each")
        && (rest.starts_with(' ') || rest.starts_with('\t'))
    {
        let target = rest.trim_start();
        if target.is_empty() || (!target.chars().any(char::is_whitespace) && !target.contains('='))
        {
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
                    .filter(|(_, (value, _, _, _))| indexed_param_type(value) == Some("bool"))
                    .map(|(path, (value, _, _, _))| {
                        item(path, 6, value, "Boolean parameter condition.", None)
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
    if let Some(eq) = pchars.iter().rposition(|&c| c == '=') {
        if eq > 0 && !matches!(pchars[eq - 1], '<' | '>' | '!') {
            let before: String = pchars[..eq].iter().collect();
            if let Some(attr) = trailing_attr(&before) {
                let valpart: String = pchars[eq + 1..].iter().collect();
                return value_items(ix, &attr, &valpart, ctx);
            }
        }
    }

    let seg_is_stmt = trimmed.is_empty()
        || (trimmed.chars().all(is_ident_char)
            && trimmed.chars().next().is_some_and(is_ident_start));
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
                item(
                    name,
                    7,
                    "builtin",
                    vocab::lookup(vocab::NODE_DOCS, name).unwrap_or(""),
                    None,
                )
            })
            .collect();
        if parent != Some("icon") && parent != Some("para") {
            items.extend(ix.defs.iter().map(|(name, (_, _, _, sig))| {
                item(name, 7, sig, &format!("component `{sig}`"), None)
            }));
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
            for keyword in ["def", "tokens", "theme", "anim", "params", "icon"] {
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
        item(
            flag,
            14,
            "flag",
            vocab::lookup(vocab::FLAG_DOCS, flag).unwrap_or(""),
            None,
        )
    }));
    items
}

/// Markdown hover text for the token under the cursor.
fn hover_md(ix: &Index, tok: &LTok, prev: Option<&LTok>, nxt: Option<&LTok>) -> Option<String> {
    if tok.kind == TokKind::Ref {
        if let Some((value, _, _, _)) = ix.token_paths.get(&tok.text) {
            return Some(format!("`{} = {}`", tok.text, value));
        }
        if let Some((value, _, _, _)) = ix.param_paths.get(&tok.text) {
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
        return Some(format!(
            "`each {word}` — nested list field with item schema `list({schema})`."
        ));
    }

    if prev.is_some_and(|token| {
        (token.kind == TokKind::Id && token.text == "when") || token.kind == TokKind::Bang
    }) && let Some(doc) = vocab::lookup(vocab::VALUE_DOCS, word)
    {
        return Some(format!("`{word}` *(state)* — {doc}"));
    }
    let in_value = prev
        .is_some_and(|token| matches!(token.kind, TokKind::Eq | TokKind::Comma | TokKind::Colon));
    if in_value {
        if ix.anims.contains_key(word) {
            return Some(format!(
                "`anim {word}` — keyframe animation defined in this document."
            ));
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
        return Some(format!(
            "`anim {word}` — keyframe animation defined in this document."
        ));
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
