//! LSP server contract: drives `slab_lsp::Server` directly, no pipes — a
//! feature-for-feature port of the research `tests/test_lsp.py` (19
//! scenarios) plus the 1.0 additions (params definition/completion/hover,
//! `hole` in node completion, params keyword + symbols).

use serde_json::{Value, json};
use slab_lsp::Server;

const URI: &str = "file:///t.slab";

const DOC: &str = r#"tokens {
  color { bg #0e1116; ink #e6edf3; accent oklch(72% 0.16 250) }
}
anim pulse { 0% { opacity=0.4 } 100% { opacity=1 } }
def Chip(chip_label="", tone=color.accent) {
  row pad=4,10 gap=6 radius=999 stroke=tone align=center w=hug {
    text chip_label size=12 color=tone nowrap
  }
}
col#card w=360 bg=color.bg radius=12 pad=24 gap=8 clip {
  text "Pale Green Things" size=18 weight=650 color=color.ink
  Chip#a chip_label="FLAC" tone=color.accent
}
"#;

// a line lifted from examples/11-unicode.slab: astral emoji + ZWJ sequence
const UNI_LINE: &str = "  text#emoji   \"ok \u{1F44D} team \u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467} done\" size=14 color=color.ink";

// 1.0 host surface: params block, param refs, hole
const PARAMS_DOC: &str = r#"params {
  title text = "Settings"
  compact bool = false
}
col w=360 pad=24 gap=8 {
  text param.title size=18
  hole rows w=fill h=120 scroll
}
"#;

fn server_with(text: &str) -> (Server, Vec<Value>) {
    let mut srv = Server::new();
    srv.handle(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {}}));
    let notes = srv.handle(
        &json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
        "textDocument": {"uri": URI, "languageId": "slab", "version": 1, "text": text}}}),
    );
    (srv, notes)
}

fn server() -> (Server, Vec<Value>) {
    server_with(DOC)
}

fn req(srv: &mut Server, method: &str, params: Value) -> Value {
    let out = srv.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}));
    assert!(!out.is_empty(), "no response for {method}");
    assert_eq!(out[0]["id"], json!(1), "{:?}", out);
    assert!(out[0].get("error").is_none(), "{:?}", out[0]);
    out[0]["result"].clone()
}

fn diags(notes: &[Value]) -> Vec<Value> {
    assert!(!notes.is_empty());
    assert_eq!(notes[0]["method"], "textDocument/publishDiagnostics");
    notes[0]["params"]["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .clone()
}

fn labels(items: &Value) -> Vec<String> {
    items
        .as_array()
        .expect("items array")
        .iter()
        .map(|i| i["label"].as_str().unwrap().to_string())
        .collect()
}

fn doc_line(n: usize) -> &'static str {
    DOC.split('\n').nth(n).unwrap()
}

#[test]
fn test_initialize_capabilities() {
    let mut srv = Server::new();
    let out = srv.handle(&json!({"jsonrpc": "2.0", "id": 7, "method": "initialize", "params": {}}));
    let caps = &out[0]["result"]["capabilities"];
    assert_eq!(
        caps["textDocumentSync"],
        json!({"openClose": true, "change": 2}) // Incremental
    );
    let trig = caps["completionProvider"]["triggerCharacters"]
        .as_array()
        .unwrap();
    assert!(trig.contains(&json!("=")));
    assert!(trig.contains(&json!(".")));
    assert_eq!(caps["hoverProvider"], json!(true));
    assert_eq!(caps["definitionProvider"], json!(true));
    assert_eq!(caps["documentSymbolProvider"], json!(true));
    assert_eq!(caps["colorProvider"], json!(true));
}

#[test]
fn test_unknown_method_is_method_not_found() {
    let mut srv = Server::new();
    let out = srv.handle(&json!({"jsonrpc": "2.0", "id": 9, "method": "nope/nope", "params": {}}));
    assert_eq!(out[0]["error"]["code"], json!(-32601));
}

#[test]
fn test_shutdown_exit_lifecycle() {
    let mut srv = Server::new();
    let out = srv.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "shutdown"}));
    assert_eq!(out[0]["result"], Value::Null);
    srv.handle(&json!({"jsonrpc": "2.0", "method": "exit"}));
    assert!(!srv.running);
    assert_eq!(srv.exit_code, 0);
}

#[test]
fn test_did_open_publishes_parse_error_on_right_line() {
    let bad = "col w=360 {\n  text )\n}\n";
    let (_, notes) = server_with(bad);
    let ds = diags(&notes);
    assert!(
        ds.iter().any(|d| d["severity"] == json!(1)
            && d["code"] == json!("parse")
            && d["range"]["start"]["line"] == json!(1)),
        "{ds:?}"
    );
}

#[test]
fn test_did_change_incremental_updates_diagnostics() {
    let (mut srv, notes) = server();
    assert!(diags(&notes).is_empty(), "{:?}", diags(&notes));
    // insert a stray ')' at line 10 col 2, then revert it with a second edit
    let notes = srv.handle(
        &json!({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
        "textDocument": {"uri": URI, "version": 2},
        "contentChanges": [{"range": {"start": {"line": 10, "character": 2},
                                      "end": {"line": 10, "character": 2}}, "text": ")"}]}}),
    );
    assert!(diags(&notes).iter().any(|d| d["code"] == json!("parse")));
    let notes = srv.handle(
        &json!({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
        "textDocument": {"uri": URI, "version": 3},
        "contentChanges": [{"range": {"start": {"line": 10, "character": 2},
                                      "end": {"line": 10, "character": 3}}, "text": ""}]}}),
    );
    assert!(diags(&notes).is_empty());
    assert_eq!(srv.docs[URI], DOC); // the two edits round-trip byte-exactly
}

#[test]
fn test_completion_value_position_after_bg() {
    let (mut srv, _) = server();
    let line = doc_line(9);
    let col = line.find("bg=").unwrap() + 3;
    let items = req(
        &mut srv,
        "textDocument/completion",
        json!({"textDocument": {"uri": URI}, "position": {"line": 9, "character": col}}),
    );
    let ls = labels(&items);
    for want in ["color.bg", "color.ink", "color.accent"] {
        assert!(ls.contains(&want.to_string()), "{ls:?}"); // this doc's token refs
    }
    // no node names in values
    assert!(!ls.contains(&"row".to_string()) && !ls.contains(&"col".to_string()));
}

#[test]
fn test_completion_after_dot_offers_next_segments() {
    let (mut srv, _) = server();
    let line = doc_line(9);
    let col = line.find("color.bg").unwrap() + "color.".len();
    let items = req(
        &mut srv,
        "textDocument/completion",
        json!({"textDocument": {"uri": URI}, "position": {"line": 9, "character": col}}),
    );
    let mut ls = labels(&items);
    ls.sort();
    assert_eq!(ls, vec!["accent", "bg", "ink"]);
}

#[test]
fn test_completion_node_position_builtins_and_components() {
    let (mut srv, _) = server();
    let items = req(
        &mut srv,
        "textDocument/completion",
        json!({"textDocument": {"uri": URI}, "position": {"line": 11, "character": 2}}),
    );
    let ls = labels(&items);
    for want in ["row", "col", "text", "canvas", "Chip", "when"] {
        assert!(ls.contains(&want.to_string()), "{ls:?}");
    }
    assert!(!ls.contains(&"def".to_string())); // not at top level here
}

#[test]
fn test_completion_top_level_keywords_and_slot_in_def() {
    let (mut srv, _) = server();
    let top = labels(&req(
        &mut srv,
        "textDocument/completion",
        json!({"textDocument": {"uri": URI}, "position": {"line": 3, "character": 0}}),
    ));
    for want in ["def", "tokens", "anim"] {
        assert!(top.contains(&want.to_string()), "{top:?}");
    }
    assert!(!top.contains(&"slot".to_string()));
    let body = labels(&req(
        &mut srv,
        "textDocument/completion",
        json!({"textDocument": {"uri": URI}, "position": {"line": 6, "character": 4}}),
    ));
    assert!(body.contains(&"slot".to_string()), "{body:?}");
}

#[test]
fn test_hover_attribute_doc() {
    let (mut srv, _) = server();
    let line = doc_line(5);
    let res = req(
        &mut srv,
        "textDocument/hover",
        json!({"textDocument": {"uri": URI},
               "position": {"line": 5, "character": line.find("pad").unwrap() + 1}}),
    );
    assert!(
        res["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Padding"),
        "{res:?}"
    );
}

#[test]
fn test_hover_token_ref_shows_resolved_value() {
    let (mut srv, _) = server();
    let line = doc_line(9);
    let res = req(
        &mut srv,
        "textDocument/hover",
        json!({"textDocument": {"uri": URI},
               "position": {"line": 9, "character": line.find("color.bg").unwrap() + 2}}),
    );
    assert_eq!(res["contents"]["value"], json!("`color.bg = #0e1116`"));
}

#[test]
fn test_definition_component_call() {
    let (mut srv, _) = server();
    let res = req(
        &mut srv,
        "textDocument/definition",
        json!({"textDocument": {"uri": URI}, "position": {"line": 11, "character": 3}}),
    );
    assert_eq!(res["uri"], json!(URI));
    assert_eq!(res["range"]["start"]["line"], json!(4)); // `def Chip(` line
}

#[test]
fn test_definition_token_ref() {
    let (mut srv, _) = server();
    let line = doc_line(9);
    let res = req(
        &mut srv,
        "textDocument/definition",
        json!({"textDocument": {"uri": URI},
               "position": {"line": 9, "character": line.find("color.bg").unwrap() + 3}}),
    );
    assert_eq!(res["range"]["start"]["line"], json!(1)); // `bg #0e1116` entry line
}

#[test]
fn test_document_symbols_include_def_and_ids() {
    let (mut srv, _) = server();
    let syms = req(
        &mut srv,
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": URI}}),
    );
    let syms = syms.as_array().unwrap();
    let by_name = |n: &str| -> &Value { syms.iter().find(|s| s["name"] == json!(n)).unwrap() };
    assert_eq!(by_name("Chip")["kind"], json!(12));
    assert_eq!(by_name("pulse")["kind"], json!(24));
    let card = by_name("col#card");
    assert_eq!(card["kind"], json!(19));
    let card_children: Vec<&str> = card["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(card_children.contains(&"Chip#a"), "{card_children:?}");
    let tokens = by_name("tokens");
    let color = tokens["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == json!("color"))
        .unwrap();
    assert_eq!(color["kind"], json!(3));
    let mut names: Vec<&str> = color["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, vec!["accent", "bg", "ink"]);
}

#[test]
fn test_document_color_hex_and_oklch() {
    let (mut srv, _) = server();
    let colors = req(
        &mut srv,
        "textDocument/documentColor",
        json!({"textDocument": {"uri": URI}}),
    );
    let colors = colors.as_array().unwrap();
    let line1 = doc_line(1);
    let starts: Vec<(i64, i64)> = colors
        .iter()
        .map(|c| {
            (
                c["range"]["start"]["line"].as_i64().unwrap(),
                c["range"]["start"]["character"].as_i64().unwrap(),
            )
        })
        .collect();
    assert!(
        starts.contains(&(1, line1.find("#0e1116").unwrap() as i64)),
        "{starts:?}"
    );
    // color function gets a swatch too
    assert!(
        starts.contains(&(1, line1.find("oklch").unwrap() as i64)),
        "{starts:?}"
    );
    for c in colors {
        for k in ["red", "green", "blue", "alpha"] {
            let v = c["color"][k].as_f64().unwrap();
            assert!((0.0..=1.0).contains(&v), "{c:?}");
        }
    }
}

#[test]
fn test_color_presentation_hex_forms() {
    let (mut srv, _) = server();
    let opaque = req(
        &mut srv,
        "textDocument/colorPresentation",
        json!({"textDocument": {"uri": URI}, "range": {},
               "color": {"red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 1.0}}),
    );
    assert_eq!(opaque, json!([{"label": "#ff0000"}]));
    let translucent = req(
        &mut srv,
        "textDocument/colorPresentation",
        json!({"textDocument": {"uri": URI}, "range": {},
               "color": {"red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 0.5}}),
    );
    // 8-digit when alpha < 1
    assert_eq!(translucent, json!([{"label": "#ff000080"}]));
}

#[test]
fn test_unicode_positions_are_utf16() {
    let text = format!("tokens {{ color {{ ink #E8EEF6 }} }}\n{UNI_LINE}\n");
    let (mut srv, notes) = server_with(&text);
    let diagnostics = diags(&notes);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["code"] == "glyph-missing"),
        "{diagnostics:?}"
    );
    let idx = UNI_LINE.find("color=").unwrap();
    // count chars, then UTF-16 units, up to the byte index
    let u16: usize = UNI_LINE[..idx].chars().map(|c| c.len_utf16()).sum();
    let chars: usize = UNI_LINE[..idx].chars().count();
    assert!(u16 > chars); // astral chars really do widen the UTF-16 column
    let res = req(
        &mut srv,
        "textDocument/hover",
        json!({"textDocument": {"uri": URI}, "position": {"line": 1, "character": u16 + 1}}),
    );
    assert!(
        res["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Text color"),
        "{res:?}"
    );
    assert_eq!(res["range"]["start"]["character"], json!(u16));
}

#[test]
fn test_preview_returns_svg() {
    let (mut srv, _) = server();
    let res = req(&mut srv, "slab/preview", json!({"uri": URI}));
    assert!(res["svg"].as_str().unwrap().starts_with("<svg"), "{res:?}");
    assert!(res["width"].as_f64().unwrap() > 0.0);
    assert!(res["height"].as_f64().unwrap() > 0.0);
    assert_eq!(res["diags"], json!([]));
}

#[test]
fn test_preview_on_broken_doc_returns_diags_and_empty_svg() {
    let (mut srv, _) = server_with("col {\n  text )\n}\n");
    let res = req(&mut srv, "slab/preview", json!({"uri": URI}));
    assert_eq!(res["svg"], json!(""));
    assert!(
        res["diags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["level"] == json!("error")),
        "{res:?}"
    );
}

// ------------------------------------------------------------- 1.0 additions

#[test]
fn test_param_definition_goes_to_params_entry() {
    let (mut srv, _) = server_with(PARAMS_DOC);
    let line = PARAMS_DOC.split('\n').nth(5).unwrap();
    let res = req(
        &mut srv,
        "textDocument/definition",
        json!({"textDocument": {"uri": URI},
               "position": {"line": 5, "character": line.find("param.title").unwrap() + 8}}),
    );
    // `title text = "Settings"` entry line
    assert_eq!(res["range"]["start"]["line"], json!(1), "{res:?}");
    assert_eq!(res["range"]["start"]["character"], json!(2));
}

#[test]
fn test_param_completion_after_dot() {
    let (mut srv, _) = server_with(PARAMS_DOC);
    let line = PARAMS_DOC.split('\n').nth(5).unwrap();
    let col = line.find("param.").unwrap() + "param.".len();
    let items = req(
        &mut srv,
        "textDocument/completion",
        json!({"textDocument": {"uri": URI}, "position": {"line": 5, "character": col}}),
    );
    let mut ls = labels(&items);
    ls.sort();
    assert_eq!(ls, vec!["compact", "title"]);
}

#[test]
fn test_param_hover_shows_type_and_default() {
    let (mut srv, _) = server_with(PARAMS_DOC);
    let line = PARAMS_DOC.split('\n').nth(5).unwrap();
    let res = req(
        &mut srv,
        "textDocument/hover",
        json!({"textDocument": {"uri": URI},
               "position": {"line": 5, "character": line.find("param.title").unwrap() + 2}}),
    );
    let md = res["contents"]["value"].as_str().unwrap();
    assert!(md.contains("param") && md.contains("Settings"), "{md}");
}

#[test]
fn test_document_symbols_include_params_block() {
    let (mut srv, _) = server_with(PARAMS_DOC);
    let syms = req(
        &mut srv,
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": URI}}),
    );
    let params = syms
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == json!("params"))
        .expect("params symbol");
    let names: Vec<&str> = params["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["title", "compact"]);
}

#[test]
fn test_completion_node_position_includes_hole() {
    let (mut srv, _) = server();
    let items = req(
        &mut srv,
        "textDocument/completion",
        json!({"textDocument": {"uri": URI}, "position": {"line": 11, "character": 2}}),
    );
    assert!(labels(&items).contains(&"hole".to_string()));
}

#[test]
fn test_completion_top_level_includes_params_keyword() {
    let (mut srv, _) = server();
    let top = labels(&req(
        &mut srv,
        "textDocument/completion",
        json!({"textDocument": {"uri": URI}, "position": {"line": 3, "character": 0}}),
    ));
    assert!(top.contains(&"params".to_string()), "{top:?}");
}

#[test]
fn test_hover_act_attribute_doc() {
    let doc = "col w=200 {\n  row focusable act=save pad=8 { text \"Save\" }\n}\n";
    let (mut srv, _) = server_with(doc);
    let line = doc.split('\n').nth(1).unwrap();
    let res = req(
        &mut srv,
        "textDocument/hover",
        json!({"textDocument": {"uri": URI},
               "position": {"line": 1, "character": line.find("act=").unwrap() + 1}}),
    );
    assert!(
        res["contents"]["value"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("signal"),
        "{res:?}"
    );
}

#[test]
fn test_formatting_full_document_edit() {
    let (mut srv, _) = server_with("col   w = fill {\n      text  \"hi\"   size=12\n}\n");
    let res = req(
        &mut srv,
        "textDocument/formatting",
        json!({"textDocument": {"uri": URI},
               "options": {"tabSize": 2, "insertSpaces": true}}),
    );
    let edits = res.as_array().expect("edit array");
    assert_eq!(edits.len(), 1, "{res:?}");
    assert_eq!(
        edits[0]["newText"],
        json!("col w=fill {\n  text \"hi\" size=12\n}\n")
    );
    assert_eq!(
        edits[0]["range"]["start"],
        json!({"line": 0, "character": 0})
    );
    assert_eq!(edits[0]["range"]["end"], json!({"line": 3, "character": 0}));
}

#[test]
fn test_formatting_canonical_document_no_edits() {
    let (mut srv, _) = server_with("col w=fill {\n  text \"hi\" size=12\n}\n");
    let res = req(
        &mut srv,
        "textDocument/formatting",
        json!({"textDocument": {"uri": URI}, "options": {}}),
    );
    assert_eq!(res, json!([]));
}
