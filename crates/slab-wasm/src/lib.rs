//! `slab-wasm` — wasm-bindgen bindings exposing the slab compiler + render +
//! gen pipelines to JavaScript (the npm CLI and the playground site consume
//! this). All options/diagnostics cross the boundary as JSON strings; binary
//! data (SLIR, PNG, APNG) crosses as `Vec<u8>` / base64.
//!
//! Built with `wasm-bindgen --target nodejs` for the CLI and `--target web`
//! for the playground (see `scripts/pack.ts`).
use slab_compile::render::{RenderKind, RenderOpts, render as render_slir};
use slab_compile::rustgen::generate as gen_rust_src;
use slab_compile::wc::{WcFile, WcOptions, generate as gen_wc_files};
use slab_compile::{Options, compile, expand};
use slab_syntax::diag::{Diagnostics, Level};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

/// Decode a base64 string into bytes (standard alphabet, with padding).
fn b64_decode(s: &str) -> Vec<u8> {
    let t: [u8; 256] = {
        let mut t = [255u8; 256];
        let mut i = 0;
        while i < 26 {
            t[b'A' as usize + i] = i as u8;
            t[b'a' as usize + i] = (26 + i) as u8;
            i += 1;
        }
        let mut i = 0;
        while i < 10 {
            t[b'0' as usize + i] = (52 + i) as u8;
            i += 1;
        }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let bytes = s.as_bytes();
    let mut buf = [0u8; 4];
    let mut n = 0;
    for &b in bytes {
        if b == b'=' || b == b'\n' || b == b'\r' || b == b' ' {
            continue;
        }
        let v = t[b as usize];
        if v == 255 {
            continue;
        }
        buf[n] = v;
        n += 1;
        if n == 4 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
            out.push((buf[2] << 6) | buf[3]);
            n = 0;
        }
    }
    match n {
        2 => out.push((buf[0] << 2) | (buf[1] >> 4)),
        3 => {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
        }
        _ => {}
    }
    out
}

/// Encode bytes as base64 (standard alphabet, with padding).
fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let v = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(v >> 18) as usize & 63] as char);
        out.push(T[(v >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(v >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[v as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Serialize diagnostics as JSON: `[{level, code, msg, line, remedy, formatted}]`.
/// `formatted` is `Diag::format(file_name)` so CLIs print byte-identical text.
fn diags_json(diags: &Diagnostics, file: &str) -> String {
    let mut out = String::from("[");
    for (i, d) in diags.0.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let level = match d.level {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Note => "note",
        };
        let remedy = d
            .remedy
            .as_deref()
            .map(|r| serde_json::to_string(r).unwrap_or_else(|_| "null".into()))
            .unwrap_or_else(|| "null".into());
        let formatted = serde_json::to_string(&d.format(file)).unwrap_or_else(|_| "\"\"".into());
        let entry = serde_json::json!({
            "level": level,
            "code": d.code,
            "msg": d.msg,
            "line": d.line,
            "remedy": serde_json::from_str::<serde_json::Value>(&remedy).unwrap_or(serde_json::Value::Null),
            "formatted": serde_json::from_str::<serde_json::Value>(&formatted).unwrap_or(serde_json::Value::String("".into())),
        });
        out.push_str(&serde_json::to_string(&entry).unwrap_or_else(|_| "{}".into()));
    }
    out.push(']');
    out
}

/// Build `Options` from an `assets_json` string (`{"<src>": "<base64>"}`).
fn opts_with_assets(embed: bool, base_dir: &str, assets_json: &str) -> Options {
    let assets = if assets_json.is_empty() || assets_json == "{}" {
        None
    } else {
        let map: HashMap<String, String> = serde_json::from_str(assets_json).unwrap_or_default();
        let mut m = HashMap::new();
        for (k, v) in map {
            m.insert(k, b64_decode(&v));
        }
        Some(m)
    };
    Options {
        embed_assets: embed,
        base_dir: base_dir.into(),
        assets,
    }
}

/// Print diagnostics (exit 1 on errors). Returns JSON diagnostics with
/// `formatted` lines for byte-identical CLI output.
#[wasm_bindgen]
pub fn check(source: &str, file_name: &str) -> String {
    let opts = opts_with_assets(false, ".", "{}");
    let (_, diags) = compile(source, &opts);
    diags_json(&diags, file_name)
}

/// JSON array of image `src` strings the document references (for the CLI to
/// read + base64 into `assets_json`).
#[wasm_bindgen]
pub fn image_srcs(source: &str) -> String {
    let mut diags = Diagnostics::new();
    let doc = slab_syntax::parse(source, &mut diags);
    let ex = expand::expand(&doc, &mut diags);
    let mut seen: Vec<String> = Vec::new();
    for (src, _) in &ex.images {
        if !seen.iter().any(|s| s == src) {
            seen.push(src.clone());
        }
    }
    serde_json::to_string(&seen).unwrap_or_else(|_| "[]".into())
}

/// Compile to SLIR bytes. `Err` = diagnostics JSON (compile failure).
#[wasm_bindgen]
pub fn build(source: &str, assets_json: &str) -> Result<Vec<u8>, JsValue> {
    let opts = opts_with_assets(true, ".", assets_json);
    let (slir, diags) = compile(source, &opts);
    if diags.has_errors() {
        return Err(JsValue::from_str(&diags_json(&diags, "build")));
    }
    let slir = slir.ok_or_else(|| JsValue::from_str(&diags_json(&diags, "build")))?;
    Ok(slab_slir::write(&slir))
}

/// Canonical slir-dump text for SLIR bytes (mirrors `slab dump`).
#[wasm_bindgen]
pub fn dump(slir: &[u8]) -> Result<String, JsValue> {
    match slab_slir::read(slir) {
        Ok(s) => Ok(slab_slir::dump(&s)),
        Err(e) => Err(JsValue::from_str(&e)),
    }
}

/// Render a `.slab` source to SVG/PNG/APNG/TUI. `opts_json` shape:
/// `{kind, client?, theme?, width, height, scale, t, dur, fps, states, env, sets, plain}`.
/// `assets_json` = `{"<src>": "<base64>"}`. Returns
/// `{file:{name, b64?|text?}, notes, summary}` as JSON.
#[wasm_bindgen]
pub fn render(source: &str, opts_json: &str, assets_json: &str) -> Result<String, JsValue> {
    let v: serde_json::Value = serde_json::from_str(opts_json)
        .map_err(|e| JsValue::from_str(&format!("bad opts: {e}")))?;
    let kind = match v["kind"].as_str().unwrap_or("") {
        "svg" => RenderKind::Svg,
        "png" => RenderKind::Png,
        "apng" => RenderKind::Apng,
        "tui" => RenderKind::Tui,
        other => return Err(JsValue::from_str(&format!("unknown render kind '{other}'"))),
    };
    let sets = slab_compile::input::sets_from_json(&v["sets"])
        .map_err(|e| JsValue::from_str(&format!("bad opts: {e}")))?;
    let ropts = RenderOpts {
        kind,
        client: v["client"].as_str().map(String::from),
        theme: v["theme"].as_str().map(String::from),
        width: v["width"].as_f64().unwrap_or(800.0),
        height: v["height"].as_f64().unwrap_or(0.0),
        scale: v["scale"].as_f64().unwrap_or(1.0),
        t: v["t"].as_f64().unwrap_or(0.0),
        dur: v["dur"].as_f64().unwrap_or(2.0),
        fps: v["fps"].as_f64().unwrap_or(20.0),
        states: v["states"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        env: v["env"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        sets,
        plain: v["plain"].as_bool().unwrap_or(false),
        registered_fonts: Vec::new(),
    };
    let opts = opts_with_assets(true, ".", assets_json);
    let (slir, diags) = compile(source, &opts);
    if diags.has_errors() {
        return Err(JsValue::from_str(&diags_json(&diags, "render")));
    }
    let slir = slir.ok_or_else(|| JsValue::from_str(&diags_json(&diags, "render")))?;
    let out =
        render_slir(&slir, &ropts, std::path::Path::new(".")).map_err(|e| JsValue::from_str(&e))?;
    let file = if out.text {
        let text = String::from_utf8(out.bytes).unwrap_or_default();
        serde_json::json!({ "name": "out.txt", "text": text })
    } else {
        serde_json::json!({ "name": "out.bin", "b64": b64_encode(&out.bytes) })
    };
    let result = serde_json::json!({
        "file": file,
        "notes": out.notes,
        "summary": out.summary,
    });
    Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()))
}

/// `gen wc` — emit web-component files. `opts_json`:
/// `{tag?, separateIr, stem}`. Returns `{files:[{name, b64?|text?}], diagnostics:[…]}`.
#[wasm_bindgen]
pub fn gen_wc(source: &str, opts_json: &str, assets_json: &str) -> Result<String, JsValue> {
    let v: serde_json::Value = serde_json::from_str(opts_json)
        .map_err(|e| JsValue::from_str(&format!("bad opts: {e}")))?;
    let wopts = WcOptions {
        tag: v["tag"].as_str().map(String::from),
        separate_ir: v["separateIr"].as_bool().unwrap_or(false),
    };
    let stem = v["stem"].as_str().unwrap_or("slab");
    let copts = opts_with_assets(true, ".", assets_json);
    let (files, diags) = gen_wc_files(source, &copts, &wopts, stem);
    let diags_j = diags_json(&diags, "gen_wc");
    let Some(files) = files else {
        return Err(JsValue::from_str(&diags_j));
    };
    let files_j: Vec<serde_json::Value> = files
        .iter()
        .map(|WcFile { name, bytes, text }| {
            if *text {
                let s = String::from_utf8(bytes.clone()).unwrap_or_default();
                serde_json::json!({ "name": name, "text": s })
            } else {
                serde_json::json!({ "name": name, "b64": b64_encode(bytes) })
            }
        })
        .collect();
    let result = serde_json::json!({
        "files": files_j,
        "diagnostics": serde_json::from_str::<serde_json::Value>(&diags_j).unwrap_or(serde_json::Value::Array(vec![])),
    });
    Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()))
}

/// `gen rust` — emit a typed Rust module. Returns
/// `{module: string, diagnostics:[…]}`.
#[wasm_bindgen]
pub fn gen_rust(source: &str, assets_json: &str) -> Result<String, JsValue> {
    let copts = opts_with_assets(true, ".", assets_json);
    let (module, diags) = gen_rust_src(source, &copts, "gen_rust");
    let diags_j = diags_json(&diags, "gen_rust");
    let Some(module) = module else {
        return Err(JsValue::from_str(&diags_j));
    };
    let result = serde_json::json!({
        "module": module,
        "diagnostics": serde_json::from_str::<serde_json::Value>(&diags_j).unwrap_or(serde_json::Value::Array(vec![])),
    });
    Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()))
}
