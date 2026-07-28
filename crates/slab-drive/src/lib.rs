#![recursion_limit = "256"]

//! Slab Drive Protocol (SDP): deterministic NDJSON automation for one live document.
//!
//! Each UTF-8 line is one request object:
//! `{"id":<optional JSON>,"method":"domain.name","params":{...}}`. Blank lines
//! are ignored. Every non-blank line produces exactly one response line, in
//! order; SDP emits no unsolicited events. Success responses contain `result`
//! and echo an optional request `id`. Failures contain `error.code` and
//! `error.message`; parse failures use a null id.
//!
//! The default transport is stdio. `--port N` listens on `127.0.0.1:N`, accepts
//! one connection at a time, and keeps the session alive across connections.
//! `protocol.quit` ends either transport. The virtual motion clock starts at
//! zero, advances only through `clock.advance` and `render.apng`, and resets on
//! each successful load. Document-dependent methods solve a fresh retained
//! frame before acting, so dispatch and queries share one deterministic scene.
//!
//! A host can keep ownership of its live [`Instance`] with [`RequestPump`].
//! Pump one request from the host event loop, then pass each returned
//! [`Effects`] value through the host's normal signal handler.
//!
//! Parameter writes keep their existing deferred-solve semantics. A transition
//! flip starts at the first solve that observes it. Snapshot scripts must use
//! `render` → `clock.advance` → `render` after a flip.
//! Error codes are `-32700` (invalid JSON), `-32600` (invalid request),
//! `-32601` (unknown method), `-32602` (invalid parameters), and `-32000`
//! (document, key, parameter, theme, render, or filesystem failure).
//!
//! Successful loads create a fresh kernel instance: parameters, states, focus,
//! edits, scroll offsets, and hole sizes reset, while the desired environment,
//! theme, and registered fonts are reapplied. A compile failure is returned as
//! data and leaves the previous document running. An unknown reapplied theme
//! resets to the authored base and sets `theme_reset` in the load result.
//!
//! # Methods
//!
//! `rect` means `{"x":f64,"y":f64,"w":f64,"h":f64}`. `mods` is an optional
//! array containing `shift`, `alt`, `ctrl`, or `meta`. Scene keys are full
//! retained paths returned by `scene.tree`.
//!
//! | Method | Parameters | Result and semantics |
//! |---|---|---|
//! | `protocol.info` | none | `{"name":"sdp","version":1,"doc":path-or-null,"methods":[...]}` |
//! | `protocol.quit` | none | `{"ok":true}`, then the server exits |
//! | `doc.load` | `{"file":str}` | `{"ok":bool,"diags":[{"level","code","msg","line","remedy"?}],"theme_reset"?}` |
//! | `doc.reload` | none | reloads the current path with `doc.load` semantics |
//! | `doc.info` | none | file, parameter declarations, themes, holes, signals, env, and clock |
//! | `env.get` | none | `{"width","height","client","dark","coarse","theme"}` |
//! | `env.set` | any `env.get` fields | merged environment; theme validation runs last |
//! | `clock.get` | none | `{"t":f64}` |
//! | `clock.advance` | `{"ms":f64}` with `ms >= 0` | new `{"t":f64}` |
//! | `param.set` | `{"name":str,"value":any}` or `{"sets":{...}}` | `{"ok":true}` after atomic validation |
//! | `param.get` | `{"name":str}` | `{"value":any}` from the live kernel value |
//! | `field.set` | `{"key":str,"text":str}` | `{"ok":true,"changed":bool}` |
//! | `field.get` | `{"key":str}` | `{"text":str}` |
//! | `state.set` | `{"name":str,"on":bool}` | toggles a global state |
//! | `state.node` | `{"key":str,"name":str,"on":bool}` | toggles a keyed node state |
//! | `focus.get` | none | `{"focus":u32,"key":str,"visible":bool}`; `slir::NONE` means none |
//! | `focus.set` | `{"key":str,"visible":bool?}` | moves or clears focus |
//! | `img.register` | `{"name":str,"w":u32,"h":u32,"format":1,"rgba":[u8...]}` or `format:0,"png_b64":str` | unified `{"img":i32}` |
//! | `img.unregister` | `{"name":str}` | removes a runtime image |
//! | `img.info` | `{"img":i32}` | `{"w","h","format","generation"}` |
//! | `img.data` | `{"img":i32}` | base64 `data` and byte count |
//! | `scroll.get` | `{"key":str,"axis":0|1}` | `{"axis":u32,"off":f64}` |
//! | `scroll.set` | `{"key":str,"axis":0|1,"off":f64}` | the clamped axis-qualified offset |
//! | `scroll.reveal` | `{"key":str,"margin":f64}` | minimally reveals a keyed node |
//! | `list.get_len` | `{"param":str,"path":str}` | `{"len":i32}` |
//! | `list.set_len` | `{"param":str,"path":str,"n":i32}` | resizes a list |
//! | `list.set_field` | `{"param":str,"path":str,"index":i32,"field":str,"kind":str,"value":any}` | sets one typed field |
//! | `list.set_key` | `{"param":str,"path":str,"index":i32,"key":str}` | sets one item key |
//! | `list.reveal_item` | `{"each":str,"index":i32,"align":u32}` | reveals a virtual item |
//! | `list.window` | `{"each":str}` | materialized `{"start":i32,"end":i32}` |
//! | `divider.get` | `{"key":str}` | `{"extent":f64}` |
//! | `divider.set` | `{"key":str,"extent":f64}` | sets a divider overlay |
//! | `hole.list` | none | visible `{"holes":[{"hole","name","x","y","w","h","clip"}]}` |
//! | `hole.size` | `{"name":str,"w":f64,"h":f64}` or numeric `hole` | records host content size |
//! | `scene.tree` | none | flat pre-order entries with stable keys, hierarchy, geometry, flags, scroll data, and resolved accessibility semantics |
//! | `scene.node` | `{"key":str,"states":[str]?}` | one such entry plus hover, pressed, focus, disabled, and requested kernel states |
//! | `scene.text` | `{"key":str}` | `{"text":str,"runs":[{"text","x","y"}]}` for the subtree |
//! | `scene.hit` | `{"x":f64,"y":f64}` | root-to-target `keys`, `nodes`, and `rects` |
//! | `scene.find` | `{"text":str}` | case-sensitive, scene-ordered text matches |
//! | `frame.dump` | none | embedded canonical conformance frame |
//! | `frame.summary` | none | embedded canonical focus, edit, and scroll summary |
//! | `input.event` | trace event object | one dispatch |
//! | `input.pointer` | `{"type":"move"|"down"|"up","x","y","button"?,"clicks"?,"mods"?}` | one pointer dispatch |
//! | `input.click` | `{"x","y"}` xor `{"key":str}`, plus button/clicks/mods | move, down, and up with merged effects |
//! | `input.wheel` | `{"x","y","dx"?,"dy","mods"?}` | one wheel dispatch |
//! | `input.key` | `{"key":str,"mods"?}` | one key-down dispatch |
//! | `input.text` | `{"text":str}` | one text-input dispatch |
//! | `input.paste` | `{"text":str}` | one paste dispatch |
//! | `render.png` | `{"scale":f64?,"path":str?}` | base64 PNG or path, bytes, pixel dimensions, notes |
//! | `render.svg` | `{"path":str?}` | live UTF-8 SVG or path, bytes, notes |
//! | `render.cells` | `{"plain":bool?,"path":str?}` | UTF-8 text or path, columns, rows, notes |
//! | `render.apng` | `{"dur","fps","scale":f64?,"path":str?}` | base64 APNG or path, bytes, frames, new clock |
//!
//! Every input result is `{"effects":{...},"t":f64}`. Effects contain repaint,
//! ordered signals with metadata, changed scroll offsets, caret and IME
//! rectangles, cursor, and focus. Binary inline payloads use padded RFC 4648
//! base64; SVG and cell output remain UTF-8 text.
//! A render `path` writes the payload and returns its path and byte count.
//!
//! # Example
//!
//! ```text
//! printf '%s\n' \
//!   '{"id":1,"method":"render.png","params":{"path":"/tmp/settings.png"}}' \
//!   '{"id":2,"method":"protocol.quit"}' \
//! | slab drive examples/10-settings.slab
//! ```

mod wire;
use serde_json::{Map, Value, json};
use slab_compile::render::RegisteredFont;
use slab_kernel::{
    cells,
    dispatch::{self, Effects, Event},
    flatten::{Frame, FrameOp},
    frame::{self, Instance},
    scene, slir, style,
};
use slab_slir::Slir;
use slab_syntax::diag::Diagnostics;
use std::{
    io::{self, BufRead, BufReader, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::ExitCode,
};

/// Native `slab drive` command usage.
pub const DRIVE_USAGE: &str = "\
usage: slab drive [FILE] [--port N] [--width N] [--height N]
                  [--client web|gpu|tui|svg|png] [--font NAME=PATH]...
";

const ERR_PARSE: i32 = -32700;
const ERR_REQUEST: i32 = -32600;
const ERR_METHOD: i32 = -32601;
const ERR_PARAMS: i32 = -32602;
const ERR_DOMAIN: i32 = -32000;

const METHODS: &[&str] = &[
    "protocol.info",
    "protocol.quit",
    "doc.load",
    "doc.reload",
    "doc.info",
    "env.get",
    "env.set",
    "clock.get",
    "clock.advance",
    "param.set",
    "param.get",
    "field.set",
    "field.get",
    "state.set",
    "state.node",
    "focus.get",
    "focus.set",
    "img.register",
    "img.unregister",
    "img.info",
    "img.data",
    "scroll.get",
    "scroll.set",
    "scroll.reveal",
    "list.get_len",
    "list.set_len",
    "list.set_field",
    "list.set_key",
    "list.reveal_item",
    "list.window",
    "divider.get",
    "divider.set",
    "hole.list",
    "hole.size",
    "scene.tree",
    "scene.node",
    "scene.text",
    "scene.hit",
    "scene.find",
    "frame.dump",
    "frame.summary",
    "input.event",
    "input.pointer",
    "input.click",
    "input.wheel",
    "input.key",
    "input.text",
    "input.paste",
    "render.png",
    "render.svg",
    "render.cells",
    "render.apng",
];

type Failure = (i32, String);
type ProtocolResult<T = Value> = Result<T, Failure>;

struct Args {
    file: Option<PathBuf>,
    port: Option<u16>,
    env: EnvSpec,
    fonts: Vec<(String, PathBuf)>,
}

#[derive(Clone)]
struct EnvSpec {
    width: f64,
    height: f64,
    client: String,
    dark: bool,
    coarse: bool,
    theme: String,
}

struct Session {
    fonts: Vec<(String, PathBuf)>,
    env: EnvSpec,
    doc: Option<LoadedDoc>,
    t_ms: f64,
    quit: bool,
    pending_effects: Vec<Effects>,
    capture_effects: bool,
}

struct LoadedDoc {
    path: PathBuf,
    base_dir: PathBuf,
    slir: Slir,
    inst: Instance,
    images: Vec<Vec<u8>>,
    fr: Frame,
    fonts: Vec<RegisteredFont>,
}

impl Session {
    fn new(env: EnvSpec, fonts: Vec<(String, PathBuf)>) -> Self {
        Self {
            fonts,
            env,
            doc: None,
            t_ms: 0.0,
            quit: false,
            pending_effects: Vec::new(),
            capture_effects: false,
        }
    }

    fn load(&mut self, path: &Path) -> Value {
        let (slir, diags) = compile_file(path, true);
        let mut diag_values = diagnostics_json(&diags);
        let Some(slir) = slir else {
            return json!({"ok": false, "diags": diag_values});
        };

        let fonts = match load_registered_fonts(&self.fonts) {
            Ok(fonts) => fonts,
            Err(message) => {
                diag_values.push(protocol_diag("font", message));
                return json!({"ok": false, "diags": diag_values});
            }
        };
        let bytes = slab_slir::write(&slir);
        let (mut inst, images) = match slab_slir::instance(&bytes) {
            Ok(loaded) => loaded,
            Err(message) => {
                diag_values.push(protocol_diag("decode", message));
                return json!({"ok": false, "diags": diag_values});
            }
        };
        register_fonts(&mut inst, &fonts);

        let requested_theme = self.env.theme.clone();
        let theme_reset = !frame::inst_set_theme(&mut inst, &requested_theme);
        if theme_reset {
            self.env.theme.clear();
            let _ = frame::inst_set_theme(&mut inst, "");
        }
        frame::inst_set_env(
            &mut inst,
            self.env.width,
            self.env.height,
            client_code(&self.env.client).expect("validated client name"),
            self.env.dark,
            self.env.coarse,
        );
        let fr = frame::inst_frame(&mut inst, 0.0);
        let base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        self.doc = Some(LoadedDoc {
            path: path.to_path_buf(),
            base_dir,
            slir,
            inst,
            images,
            fr,
            fonts,
        });
        self.t_ms = 0.0;

        let mut result = Map::new();
        result.insert("ok".into(), Value::Bool(true));
        result.insert("diags".into(), Value::Array(diag_values));
        if theme_reset {
            result.insert("theme_reset".into(), Value::Bool(true));
        }
        Value::Object(result)
    }
}
/// One response from a host-pumped SDP request.
pub struct PumpResponse {
    /// JSON response to write as one NDJSON line.
    pub response: Value,
    /// Dispatch effects that the host must pass through its normal handler.
    pub effects: Vec<Effects>,
    /// Whether `protocol.quit` requested server shutdown.
    pub quit: bool,
}

/// Request state for SDP mounted on a caller-owned live kernel instance.
pub struct RequestPump {
    session: Session,
}

impl RequestPump {
    /// Creates a pump for an instance decoded from the matching SLIR and images.
    pub fn new(path: impl Into<PathBuf>, slir: Slir, images: Vec<Vec<u8>>) -> Self {
        let env = EnvSpec {
            width: 800.0,
            height: 600.0,
            client: "gpu".into(),
            dark: false,
            coarse: false,
            theme: String::new(),
        };
        let mut session = Session::new(env, Vec::new());
        session.capture_effects = true;
        let path = path.into();
        session.doc = Some(LoadedDoc {
            base_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            path,
            slir,
            inst: frame::inst_shell(),
            images,
            fr: slab_kernel::flatten::frame_new(),
            fonts: Vec::new(),
        });
        Self { session }
    }

    /// Applies one NDJSON request to the caller-owned instance.
    pub fn request(&mut self, instance: &mut Instance, line: &str) -> PumpResponse {
        let doc = self
            .session
            .doc
            .as_mut()
            .expect("request pump always has a document");
        std::mem::swap(instance, &mut doc.inst);
        self.session.env = EnvSpec {
            width: doc.inst.st.env.vw,
            height: doc.inst.st.env.vh,
            client: client_name(doc.inst.st.env.client).to_string(),
            dark: doc.inst.st.env.dark,
            coarse: doc.inst.st.env.coarse,
            theme: doc.inst.st.env.theme.clone(),
        };

        let response = handle_line(&mut self.session, line);
        let doc = self
            .session
            .doc
            .as_mut()
            .expect("request pump always has a document");
        std::mem::swap(instance, &mut doc.inst);
        PumpResponse {
            response,
            effects: std::mem::take(&mut self.session.pending_effects),
            quit: self.session.quit,
        }
    }
}

/// Runs blocking NDJSON input while the caller keeps host effect handling.
pub fn serve(
    pump: &mut RequestPump,
    instance: &mut Instance,
    mut input: impl BufRead,
    mut output: impl Write,
    mut handle_effects: impl FnMut(&mut Instance, Effects),
) -> io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let result = pump.request(instance, &line);
        for effects in result.effects {
            handle_effects(instance, effects);
        }
        serde_json::to_writer(&mut output, &result.response)?;
        output.write_all(b"\n")?;
        output.flush()?;
        if result.quit {
            break;
        }
    }
    Ok(())
}

fn compile_file(path: &Path, embed_assets: bool) -> (Option<Slir>, Diagnostics) {
    let src = match std::fs::read_to_string(path) {
        Ok(src) => src,
        Err(error) => {
            let mut diagnostics = Diagnostics::new();
            diagnostics.error(
                "parse",
                format!("cannot read {}: {error}", path.display()),
                0,
            );
            return (None, diagnostics);
        }
    };
    let options = slab_compile::Options {
        embed_assets,
        base_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        assets: None,
        fonts: std::collections::HashMap::new(),
    };
    slab_compile::compile(&src, &options)
}

fn load_registered_fonts(fonts: &[(String, PathBuf)]) -> Result<Vec<RegisteredFont>, String> {
    fonts
        .iter()
        .map(|(name, path)| {
            let bytes = std::fs::read(path)
                .map_err(|error| format!("cannot read font {}: {error}", path.display()))?;
            let metrics = slab_fonts::parse_metrics(&bytes)
                .ok_or_else(|| format!("cannot parse font {}", path.display()))?;
            Ok(RegisteredFont {
                name: name.clone(),
                bytes,
                metrics,
            })
        })
        .collect()
}
fn register_fonts(inst: &mut Instance, fonts: &[RegisteredFont]) {
    for font in fonts {
        let metrics = &font.metrics;
        frame::inst_font_register(
            inst,
            &font.name,
            u32::from(metrics.weight),
            u32::from(metrics.upem),
            i32::from(metrics.ascent),
            i32::from(metrics.descent),
            i32::from(metrics.line_gap),
            u32::from(metrics.default_advance),
            &metrics.cps,
            &metrics.gids,
            &metrics.advances,
        );
    }
}

fn diagnostics_json(diags: &Diagnostics) -> Vec<Value> {
    diags
        .0
        .iter()
        .map(|diag| {
            let mut value = Map::new();
            value.insert("level".into(), Value::String(diag.level.to_string()));
            value.insert("code".into(), Value::String(diag.code.to_string()));
            value.insert("msg".into(), Value::String(diag.msg.clone()));
            value.insert("line".into(), json!(diag.line));
            if let Some(remedy) = &diag.remedy {
                value.insert("remedy".into(), Value::String(remedy.clone()));
            }
            Value::Object(value)
        })
        .collect()
}

fn protocol_diag(code: &str, message: String) -> Value {
    json!({"level": "error", "code": code, "msg": message, "line": 0})
}

fn print_load_diags(path: &Path, result: &Value) {
    let Some(diags) = result["diags"].as_array() else {
        return;
    };
    for diag in diags {
        let level = diag["level"].as_str().unwrap_or("error");
        let code = diag["code"].as_str().unwrap_or("drive");
        let message = diag["msg"].as_str().unwrap_or("load failed");
        let line = diag["line"].as_u64().unwrap_or(0);
        eprintln!("{}:{line}: {level}[{code}]: {message}", path.display());
        if let Some(remedy) = diag["remedy"].as_str() {
            for line in remedy.lines() {
                eprintln!("  {line}");
            }
        }
    }
}

fn parse_cli(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        file: None,
        port: None,
        env: EnvSpec {
            width: 800.0,
            height: 600.0,
            client: "gpu".into(),
            dark: false,
            coarse: false,
            theme: String::new(),
        },
        fonts: Vec::new(),
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match arg.as_str() {
            "--port" => {
                parsed.port = Some(
                    value("--port")?
                        .parse()
                        .map_err(|_| "bad --port".to_string())?,
                );
            }
            "--width" => parsed.env.width = parse_cli_number(&value("--width")?, "--width")?,
            "--height" => {
                parsed.env.height = parse_cli_number(&value("--height")?, "--height")?;
            }
            "--client" => {
                let client = value("--client")?;
                if client_code(&client).is_none() {
                    return Err(format!("unknown client '{client}'"));
                }
                parsed.env.client = client;
            }
            "--font" => {
                let spec = value("--font")?;
                let (name, path) = spec.split_once('=').ok_or("--font needs NAME=PATH")?;
                if name.is_empty() || path.is_empty() {
                    return Err("--font needs NAME=PATH".into());
                }
                parsed.fonts.push((name.to_string(), PathBuf::from(path)));
            }
            other if other.starts_with('-') => return Err(format!("unknown flag {other}")),
            other if parsed.file.is_none() => parsed.file = Some(PathBuf::from(other)),
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    Ok(parsed)
}

fn parse_cli_number(raw: &str, name: &str) -> Result<f64, String> {
    let value = raw.parse::<f64>().map_err(|_| format!("bad {name}"))?;
    if !value.is_finite() {
        return Err(format!("bad {name}"));
    }
    Ok(value)
}

/// Runs an SDP session over stdio or a loopback TCP listener.
pub fn cmd_drive(args: &[String]) -> ExitCode {
    if args == ["--help"] || args == ["-h"] {
        print!("{DRIVE_USAGE}");
        return ExitCode::SUCCESS;
    }
    let args = match parse_cli(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}");
            eprint!("{DRIVE_USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut session = Session::new(args.env, args.fonts);
    if let Some(file) = args.file {
        let result = session.load(&file);
        print_load_diags(&file, &result);
        if result["ok"] != Value::Bool(true) {
            return ExitCode::FAILURE;
        }
    }

    let served = match args.port {
        None => serve_session(
            &mut session,
            std::io::stdin().lock(),
            std::io::stdout().lock(),
        ),
        Some(port) => serve_tcp(&mut session, port),
    };
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: drive: {error}");
            ExitCode::FAILURE
        }
    }
}

fn serve_tcp(session: &mut Session, port: u16) -> io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    eprintln!("sdp: listening on {}", listener.local_addr()?);
    for stream in listener.incoming() {
        let stream = stream?;
        let input = BufReader::new(&stream);
        serve_session(session, input, &stream)?;
        if session.quit {
            break;
        }
    }
    Ok(())
}

fn serve_session(
    session: &mut Session,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    let mut line = String::new();
    while !session.quit {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let response = handle_line(session, &line);
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

fn handle_line(session: &mut Session, line: &str) -> Value {
    let request = match serde_json::from_str::<Value>(line) {
        Ok(request) => request,
        Err(_) => return error_response(None, ERR_PARSE, "parse error"),
    };
    let Some(object) = request.as_object() else {
        return error_response(None, ERR_REQUEST, "request must be a JSON object");
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return error_response(id, ERR_REQUEST, "method must be a string");
    };
    let empty = Value::Object(Map::new());
    let params = object.get("params").unwrap_or(&empty);
    if !params.is_object() {
        return error_response(id, ERR_PARAMS, "params must be an object");
    }
    match handle(session, method, params) {
        Ok(result) => success_response(id, result),
        Err((code, message)) => error_response(id, code, message),
    }
}

fn success_response(id: Option<Value>, result: Value) -> Value {
    let mut response = Map::new();
    if let Some(id) = id {
        response.insert("id".into(), id);
    }
    response.insert("result".into(), result);
    Value::Object(response)
}

fn error_response(id: Option<Value>, code: i32, message: impl Into<String>) -> Value {
    json!({
        "id": id.unwrap_or(Value::Null),
        "error": {"code": code, "message": message.into()}
    })
}

fn invalid(message: impl Into<String>) -> Failure {
    (ERR_PARAMS, message.into())
}

fn domain(message: impl Into<String>) -> Failure {
    (ERR_DOMAIN, message.into())
}

fn params(value: &Value) -> &Map<String, Value> {
    value.as_object().expect("handle_line validates params")
}

fn required_str<'a>(object: &'a Map<String, Value>, name: &str) -> ProtocolResult<&'a str> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("'{name}' must be a string")))
}

fn required_f64(object: &Map<String, Value>, name: &str) -> ProtocolResult<f64> {
    object
        .get(name)
        .and_then(Value::as_f64)
        .ok_or_else(|| invalid(format!("'{name}' must be a number")))
}

fn required_u32(object: &Map<String, Value>, name: &str) -> ProtocolResult<u32> {
    object
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| invalid(format!("'{name}' must be a u32")))
}

fn required_i32(object: &Map<String, Value>, name: &str) -> ProtocolResult<i32> {
    object
        .get(name)
        .and_then(Value::as_i64)
        .and_then(|number| i32::try_from(number).ok())
        .ok_or_else(|| invalid(format!("'{name}' must be an i32")))
}

fn required_axis(object: &Map<String, Value>) -> ProtocolResult<u32> {
    let axis = required_u32(object, "axis")?;
    if axis > 1 {
        return Err(invalid("'axis' must be 0 or 1"));
    }
    Ok(axis)
}

fn optional_f64(object: &Map<String, Value>, name: &str, default: f64) -> ProtocolResult<f64> {
    match object.get(name) {
        Some(value) => value
            .as_f64()
            .ok_or_else(|| invalid(format!("'{name}' must be a number"))),
        None => Ok(default),
    }
}

fn required_bool(object: &Map<String, Value>, name: &str) -> ProtocolResult<bool> {
    object
        .get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid(format!("'{name}' must be a boolean")))
}

fn optional_bool(object: &Map<String, Value>, name: &str, default: bool) -> ProtocolResult<bool> {
    match object.get(name) {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid(format!("'{name}' must be a boolean"))),
        None => Ok(default),
    }
}

fn optional_u32(object: &Map<String, Value>, name: &str, default: u32) -> ProtocolResult<u32> {
    match object.get(name) {
        Some(value) => value
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .ok_or_else(|| invalid(format!("'{name}' must be a u32"))),
        None => Ok(default),
    }
}

fn optional_path(object: &Map<String, Value>) -> ProtocolResult<Option<String>> {
    match object.get("path") {
        Some(value) => value
            .as_str()
            .map(|path| Some(path.to_string()))
            .ok_or_else(|| invalid("'path' must be a string")),
        None => Ok(None),
    }
}

fn ensure_frame(session: &mut Session) -> ProtocolResult<&mut LoadedDoc> {
    let t_ms = session.t_ms;
    let doc = session
        .doc
        .as_mut()
        .ok_or_else(|| domain("no document loaded"))?;
    doc.fr = frame::inst_frame(&mut doc.inst, t_ms);
    Ok(doc)
}

fn handle(session: &mut Session, method: &str, value: &Value) -> ProtocolResult {
    match method {
        "protocol.info" => Ok(protocol_info(session)),
        "protocol.quit" => {
            session.quit = true;
            Ok(json!({"ok": true}))
        }
        "doc.load" => {
            let file = required_str(params(value), "file")?;
            Ok(session.load(Path::new(file)))
        }
        "doc.reload" => {
            let path = session
                .doc
                .as_ref()
                .map(|doc| doc.path.clone())
                .ok_or_else(|| domain("no document loaded"))?;
            Ok(session.load(&path))
        }
        "doc.info" => doc_info(session),
        "env.get" => Ok(env_value(session)),
        "env.set" => env_set(session, params(value)),
        "clock.get" => Ok(json!({"t": session.t_ms})),
        "clock.advance" => clock_advance(session, params(value)),
        "param.set" => param_set(session, params(value)),
        "param.get" => param_get(session, params(value)),
        "field.set" => field_set(session, params(value)),
        "field.get" => field_get(session, params(value)),
        "state.set" => state_set(session, params(value)),
        "state.node" => state_node(session, params(value)),
        "focus.get" => focus_get(session),
        "focus.set" => focus_set(session, params(value)),
        "img.register" => img_register(session, params(value)),
        "img.unregister" => img_unregister(session, params(value)),
        "img.info" => img_info(session, params(value)),
        "img.data" => img_data(session, params(value)),
        "scroll.get" => scroll_get(session, params(value)),
        "scroll.set" => scroll_set(session, params(value)),
        "scroll.reveal" => scroll_reveal(session, params(value)),
        "list.get_len" => list_get_len(session, params(value)),
        "list.set_len" => list_set_len(session, params(value)),
        "list.set_field" => list_set_field(session, params(value)),
        "list.set_key" => list_set_key(session, params(value)),
        "list.reveal_item" => list_reveal_item(session, params(value)),
        "list.window" => list_window(session, params(value)),
        "divider.get" => divider_get(session, params(value)),
        "divider.set" => divider_set(session, params(value)),
        "hole.list" => hole_list(session),
        "hole.size" => hole_size(session, params(value)),
        "scene.tree" => scene_tree(session),
        "scene.node" => scene_node(session, params(value)),
        "scene.text" => scene_text(session, params(value)),
        "scene.hit" => scene_hit(session, params(value)),
        "scene.find" => scene_find(session, params(value)),
        "frame.dump" => frame_dump(session),
        "frame.summary" => frame_summary(session),
        "input.event" => input_event(session, value),
        "input.pointer" => input_pointer(session, params(value)),
        "input.click" => input_click(session, params(value)),
        "input.wheel" => input_wheel(session, params(value)),
        "input.key" => input_key(session, params(value)),
        "input.text" => input_text(session, params(value), dispatch::E_TEXT),
        "input.paste" => input_text(session, params(value), dispatch::E_PASTE),
        "render.png" => render_png(session, params(value)),
        "render.svg" => render_svg(session, params(value)),
        "render.cells" => render_cells(session, params(value)),
        "render.apng" => render_apng(session, params(value)),
        _ => Err((ERR_METHOD, format!("unknown method '{method}'"))),
    }
}

fn protocol_info(session: &Session) -> Value {
    json!({
        "name": "sdp",
        "version": 1,
        "doc": session.doc.as_ref().map(|doc| doc.path.display().to_string()),
        "methods": METHODS,
    })
}

fn doc_info(session: &mut Session) -> ProtocolResult {
    let t_ms = session.t_ms;
    let doc = ensure_frame(session)?;
    let kernel = &doc.inst.doc;
    let mut parameters = Vec::with_capacity(kernel.parm_name.len());
    for (index, name_ref) in kernel.parm_name.iter().copied().enumerate() {
        let kind = kernel.parm_type[index];
        let mut parameter = Map::new();
        parameter.insert("name".into(), Value::String(slir::str_at(kernel, name_ref)));
        parameter.insert("type".into(), Value::String(param_type_name(kind).into()));
        if kind == 5 {
            let offset = usize::try_from(kernel.parm_enum_off[index])
                .expect("parameter enum offset must be nonnegative");
            let length = usize::try_from(kernel.parm_enum_len[index])
                .expect("parameter enum length must be nonnegative");
            let members = kernel.parm_enum_syms[offset..offset + length]
                .iter()
                .map(|member| Value::String(slir::str_at(kernel, *member)))
                .collect();
            parameter.insert("enum".into(), Value::Array(members));
        }
        parameters.push(Value::Object(parameter));
    }
    let strings = |refs: &[u32]| {
        refs.iter()
            .map(|value| Value::String(slir::str_at(kernel, *value)))
            .collect::<Vec<_>>()
    };
    Ok(json!({
        "file": doc.path.display().to_string(),
        "params": parameters,
        "themes": strings(&kernel.theme_name),
        "holes": strings(&kernel.hole_name),
        "signals": strings(&kernel.sign_name),
        "env": env_from_instance(&doc.inst),
        "t": t_ms,
    }))
}

fn param_type_name(kind: u32) -> &'static str {
    match kind {
        0 => "text",
        1 => "num",
        2 => "pct",
        3 => "color",
        4 => "bool",
        5 => "enum",
        slir::PARAM_LIST => "list",
        _ => "unknown",
    }
}

fn client_code(name: &str) -> Option<u32> {
    slab_compile::render::client_code(name)
}

fn client_name(code: u32) -> &'static str {
    match code {
        0 => "web",
        1 => "gpu",
        2 => "tui",
        3 => "svg",
        4 => "png",
        _ => "unknown",
    }
}

fn env_from_instance(inst: &Instance) -> Value {
    json!({
        "width": inst.st.env.vw,
        "height": inst.st.env.vh,
        "client": client_name(inst.st.env.client),
        "dark": inst.st.env.dark,
        "coarse": inst.st.env.coarse,
        "theme": inst.st.env.theme,
    })
}

fn env_value(session: &Session) -> Value {
    match &session.doc {
        Some(doc) => env_from_instance(&doc.inst),
        None => json!({
            "width": session.env.width,
            "height": session.env.height,
            "client": session.env.client,
            "dark": session.env.dark,
            "coarse": session.env.coarse,
            "theme": session.env.theme,
        }),
    }
}

fn env_set(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let width = object
        .get("width")
        .map(|_| required_f64(object, "width"))
        .transpose()?;
    let height = object
        .get("height")
        .map(|_| required_f64(object, "height"))
        .transpose()?;
    let dark = object
        .get("dark")
        .map(|_| required_bool(object, "dark"))
        .transpose()?;
    let coarse = object
        .get("coarse")
        .map(|_| required_bool(object, "coarse"))
        .transpose()?;
    let client = object
        .get("client")
        .map(|_| required_str(object, "client"))
        .transpose()?;
    if let Some(client) = client
        && client_code(client).is_none()
    {
        return Err(invalid(format!("unknown client '{client}'")));
    }
    let theme = object
        .get("theme")
        .map(|_| required_str(object, "theme"))
        .transpose()?;

    if let Some(width) = width {
        session.env.width = width;
    }
    if let Some(height) = height {
        session.env.height = height;
    }
    if let Some(client) = client {
        session.env.client = client.to_string();
    }
    if let Some(dark) = dark {
        session.env.dark = dark;
    }
    if let Some(coarse) = coarse {
        session.env.coarse = coarse;
    }
    let environment_changed = width.is_some()
        || height.is_some()
        || client.is_some()
        || dark.is_some()
        || coarse.is_some();
    if environment_changed && let Some(doc) = session.doc.as_mut() {
        frame::inst_set_env(
            &mut doc.inst,
            session.env.width,
            session.env.height,
            client_code(&session.env.client).expect("validated client name"),
            session.env.dark,
            session.env.coarse,
        );
    }

    if let Some(theme) = theme {
        if let Some(doc) = session.doc.as_mut()
            && !frame::inst_set_theme(&mut doc.inst, theme)
        {
            return Err(domain(format!("unknown theme '{theme}'")));
        }
        session.env.theme = theme.to_string();
    }
    Ok(env_value(session))
}

fn clock_advance(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let ms = required_f64(object, "ms")?;
    if ms < 0.0 {
        return Err(invalid("'ms' must be nonnegative"));
    }
    let next = session.t_ms + ms;
    if !next.is_finite() {
        return Err(invalid("clock value is too large"));
    }
    session.t_ms = next;
    Ok(json!({"t": session.t_ms}))
}

fn param_set(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let sets = match (object.get("sets"), object.get("name"), object.get("value")) {
        (Some(sets), None, None) if sets.is_object() => {
            slab_compile::input::sets_from_json(sets).map_err(domain)?
        }
        (Some(_), None, None) => return Err(invalid("'sets' must be an object")),
        (None, Some(name), Some(value)) => {
            let name = name
                .as_str()
                .ok_or_else(|| invalid("'name' must be a string"))?;
            let mut one = Map::new();
            one.insert(name.to_string(), value.clone());
            slab_compile::input::sets_from_json(&Value::Object(one)).map_err(domain)?
        }
        _ => {
            return Err(invalid(
                "param.set needs either 'name' and 'value', or 'sets'",
            ));
        }
    };
    let doc = ensure_frame(session)?;
    slab_compile::input::apply_sets(&mut doc.inst, &sets).map_err(domain)?;
    Ok(json!({"ok": true}))
}
fn param_get(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let name = required_str(object, "name")?;
    let doc = ensure_frame(session)?;
    let raw = frame::inst_param_json(&doc.inst, name)
        .ok_or_else(|| domain(format!("unknown parameter '{name}'")))?;
    let value = serde_json::from_str::<Value>(&raw)
        .map_err(|error| domain(format!("kernel parameter JSON error: {error}")))?;
    Ok(json!({"value": value}))
}

fn field_set(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let text = required_str(object, "text")?.to_string();
    let doc = ensure_frame(session)?;
    let changed = frame::inst_set_field_text(&mut doc.inst, &key, &text);
    Ok(json!({"ok": true, "changed": changed}))
}

fn field_get(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?;
    let doc = ensure_frame(session)?;
    let text = frame::inst_field_text(&doc.inst, key)
        .ok_or_else(|| domain(format!("unknown field '{key}'")))?;
    Ok(json!({"text": text}))
}

fn state_set(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let name = required_str(object, "name")?.to_string();
    let on = required_bool(object, "on")?;
    let doc = ensure_frame(session)?;
    frame::inst_set_state(&mut doc.inst, &name, on);
    Ok(json!({"ok": true}))
}

fn state_node(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let name = required_str(object, "name")?.to_string();
    let on = required_bool(object, "on")?;
    let doc = ensure_frame(session)?;
    if !frame::inst_set_node_state(&mut doc.inst, &key, &name, on) {
        return Err(domain(format!("unknown key '{key}'")));
    }
    Ok(json!({"ok": true}))
}

fn focus_get(session: &mut Session) -> ProtocolResult {
    let doc = ensure_frame(session)?;
    let focus = frame::inst_focus(&doc.inst);
    Ok(json!({
        "focus": focus,
        "key": scene::key_of(&doc.inst.doc, &doc.inst.st.lists, focus),
        "visible": doc.inst.ds.fs.visible,
    }))
}

fn focus_set(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let visible = optional_bool(object, "visible", true)?;
    let doc = ensure_frame(session)?;
    if !frame::inst_set_focus(&mut doc.inst, &key, visible) {
        return Err(domain(format!("cannot focus key '{key}'")));
    }
    Ok(json!({"ok": true}))
}

fn img_register(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let image = wire::runtime_image_input(object).map_err(invalid)?;
    let doc = ensure_frame(session)?;
    let img = frame::inst_img_register(
        &mut doc.inst,
        &image.name,
        image.w,
        image.h,
        image.format,
        &image.data,
    );
    if img < 0 {
        return Err(domain("image registration was rejected"));
    }
    Ok(json!({"img": img}))
}

fn img_unregister(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let name = required_str(object, "name")?.to_string();
    let doc = ensure_frame(session)?;
    if !frame::inst_img_unregister(&mut doc.inst, &name) {
        return Err(domain(format!("unknown runtime image '{name}'")));
    }
    Ok(json!({"ok": true}))
}

fn img_info(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let img = required_i32(object, "img")?;
    let doc = ensure_frame(session)?;
    let (w, h, format, generation) = frame::inst_img_info(&doc.inst, img)
        .ok_or_else(|| domain(format!("unknown image index {img}")))?;
    Ok(json!({
        "w": w,
        "h": h,
        "format": format,
        "generation": generation,
    }))
}

fn img_data(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let img = required_i32(object, "img")?;
    let doc = ensure_frame(session)?;
    if frame::inst_img_info(&doc.inst, img).is_none() {
        return Err(domain(format!("unknown image index {img}")));
    }
    let data = frame::inst_img_bytes(&doc.inst, img);
    Ok(json!({"data": b64(data), "bytes": byte_count(data)?}))
}

fn scroll_get(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let axis = required_axis(object)?;
    let doc = ensure_frame(session)?;
    if scene::node_by_key(&doc.inst.doc, &doc.inst.st.lists, &key) == slir::NONE {
        return Err(domain(format!("unknown key '{key}'")));
    }
    Ok(json!({
        "axis": axis,
        "off": frame::inst_get_scroll(&doc.inst, &key, axis),
    }))
}

fn scroll_set(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let axis = required_axis(object)?;
    let off = required_f64(object, "off")?;
    let t_ms = session.t_ms;
    let doc = ensure_frame(session)?;
    if !frame::inst_set_scroll(&mut doc.inst, &key, axis, off) {
        return Err(domain(format!(
            "key '{key}' is not scrollable on axis {axis}"
        )));
    }
    doc.fr = frame::inst_frame(&mut doc.inst, t_ms);
    Ok(json!({
        "axis": axis,
        "off": frame::inst_get_scroll(&doc.inst, &key, axis),
    }))
}

fn scroll_reveal(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let margin = required_f64(object, "margin")?;
    let t_ms = session.t_ms;
    let doc = ensure_frame(session)?;
    if !frame::inst_reveal(&mut doc.inst, &key, margin) {
        return Err(domain(format!("cannot reveal key '{key}'")));
    }
    doc.fr = frame::inst_frame(&mut doc.inst, t_ms);
    Ok(json!({"ok": true}))
}

fn list_param(inst: &Instance, name: &str) -> ProtocolResult<u32> {
    let param = inst
        .doc
        .parm_name
        .iter()
        .position(|name_ref| inst.doc.strs[*name_ref as usize] == name)
        .ok_or_else(|| domain(format!("unknown parameter '{name}'")))?;
    if inst.doc.parm_type.get(param).copied() != Some(slir::PARAM_LIST) {
        return Err(domain(format!("parameter '{name}' is not a list")));
    }
    u32::try_from(param).map_err(|_| domain("parameter index is too large"))
}

fn list_get_len(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let name = required_str(object, "param")?.to_string();
    let path = required_str(object, "path")?.to_string();
    let doc = ensure_frame(session)?;
    let param = list_param(&doc.inst, &name)?;
    let len = frame::inst_list_len(&doc.inst, param, &path);
    if len < 0 {
        return Err(domain(format!("unknown list path '{path}'")));
    }
    Ok(json!({"len": len}))
}

fn list_set_len(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let name = required_str(object, "param")?.to_string();
    let path = required_str(object, "path")?.to_string();
    let n = required_i32(object, "n")?;
    let doc = ensure_frame(session)?;
    let param = list_param(&doc.inst, &name)?;
    if !frame::inst_set_list_len(&mut doc.inst, param, &path, n) {
        return Err(domain(format!(
            "cannot resize parameter '{name}' at path '{path}'"
        )));
    }
    Ok(json!({"ok": true}))
}

fn list_set_field(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let name = required_str(object, "param")?.to_string();
    let path = required_str(object, "path")?.to_string();
    let index = required_i32(object, "index")?;
    let field = required_str(object, "field")?.to_string();
    let kind = required_str(object, "kind")?;
    let raw = object
        .get("value")
        .ok_or_else(|| invalid("'value' is required"))?;
    let value = wire::typed_value_parts(kind, raw).map_err(invalid)?;
    let doc = ensure_frame(session)?;
    let param = list_param(&doc.inst, &name)?;
    if !frame::inst_set_list_field(&mut doc.inst, param, &path, index, &field, &value) {
        return Err(domain(format!(
            "cannot set field '{field}' on parameter '{name}' at path '{path}'"
        )));
    }
    Ok(json!({"ok": true}))
}

fn list_set_key(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let name = required_str(object, "param")?.to_string();
    let path = required_str(object, "path")?.to_string();
    let index = required_i32(object, "index")?;
    let key = required_str(object, "key")?.to_string();
    let doc = ensure_frame(session)?;
    let param = list_param(&doc.inst, &name)?;
    if !frame::inst_set_list_key(&mut doc.inst, param, &path, index, &key) {
        return Err(domain(format!(
            "cannot set item key on parameter '{name}' at path '{path}'"
        )));
    }
    Ok(json!({"ok": true}))
}

fn list_reveal_item(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let each = required_str(object, "each")?.to_string();
    let index = required_i32(object, "index")?;
    let align = required_u32(object, "align")?;
    if align > 3 {
        return Err(invalid("'align' must be between 0 and 3"));
    }
    let t_ms = session.t_ms;
    let doc = ensure_frame(session)?;
    if !frame::inst_reveal_item(&mut doc.inst, &each, index, align) {
        return Err(domain(format!(
            "cannot reveal item {index} in each '{each}'"
        )));
    }
    doc.fr = frame::inst_frame(&mut doc.inst, t_ms);
    Ok(json!({"ok": true}))
}

fn list_window(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let each = required_str(object, "each")?.to_string();
    let doc = ensure_frame(session)?;
    let (start, end) = frame::inst_each_window(&doc.inst, &each);
    Ok(json!({"start": start, "end": end}))
}

fn divider_get(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let doc = ensure_frame(session)?;
    let node = scene::node_by_key(&doc.inst.doc, &doc.inst.st.lists, &key);
    if node == slir::NONE {
        return Err(domain(format!("key '{key}' is not a divider")));
    }
    let base = slab_kernel::list::base(&doc.inst.st.lists, &doc.inst.doc, node);
    let divider = usize::try_from(base)
        .ok()
        .and_then(|base| doc.inst.doc.node_kind.get(base))
        .copied()
        == Some(slir::K_DIVIDER);
    if !divider {
        return Err(domain(format!("key '{key}' is not a divider")));
    }
    Ok(json!({"extent": frame::inst_get_divider(&doc.inst, &key)}))
}

fn divider_set(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let extent = required_f64(object, "extent")?;
    let doc = ensure_frame(session)?;
    if !frame::inst_set_divider(&mut doc.inst, &key, extent) {
        return Err(domain(format!("key '{key}' is not a divider")));
    }
    Ok(json!({"ok": true}))
}

fn hole_list(session: &mut Session) -> ProtocolResult {
    let doc = ensure_frame(session)?;
    let holes = frame::inst_holes(&mut doc.inst)
        .into_iter()
        .map(|hole| {
            let name_ref = doc.inst.doc.hole_name
                [usize::try_from(hole.hole).expect("hole index must fit usize")];
            json!({
                "hole": hole.hole,
                "name": slir::str_at(&doc.inst.doc, name_ref),
                "x": hole.x,
                "y": hole.y,
                "w": hole.w,
                "h": hole.h,
                "clip": hole.clip,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({"holes": holes}))
}

fn hole_size(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let width = required_f64(object, "w")?;
    let height = required_f64(object, "h")?;
    let name = object.get("name");
    let index = object.get("hole");
    if name.is_some() == index.is_some() {
        return Err(invalid("hole.size needs exactly one of 'name' or 'hole'"));
    }
    let name = name
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| invalid("'name' must be a string"))
        })
        .transpose()?;
    let raw_index = index
        .map(|value| {
            value
                .as_u64()
                .and_then(|number| u32::try_from(number).ok())
                .ok_or_else(|| invalid("'hole' must be a u32"))
        })
        .transpose()?;
    let doc = ensure_frame(session)?;
    let hole = match (name, raw_index) {
        (Some(name), None) => doc
            .inst
            .doc
            .hole_name
            .iter()
            .position(|name_ref| slir::str_at(&doc.inst.doc, *name_ref) == name)
            .and_then(|index| u32::try_from(index).ok())
            .ok_or_else(|| domain(format!("unknown hole '{name}'")))?,
        (None, Some(index)) => {
            if usize::try_from(index)
                .ok()
                .is_none_or(|index| index >= doc.inst.doc.hole_name.len())
            {
                return Err(domain(format!("unknown hole {index}")));
            }
            index
        }
        _ => unreachable!("address form validated"),
    };
    frame::inst_set_hole_size(&mut doc.inst, hole, width, height);
    Ok(json!({"ok": true}))
}

fn kind_name(kind: u32) -> &'static str {
    match kind {
        slir::K_ROW => "row",
        slir::K_COL => "col",
        slir::K_WRAP => "wrap",
        slir::K_GRID => "grid",
        slir::K_STACK => "stack",
        slir::K_CANVAS => "canvas",
        slir::K_PARA => "para",
        slir::K_GROUP => "group",
        slir::K_TEXT => "text",
        slir::K_SPAN => "span",
        slir::K_RECT => "rect",
        slir::K_IMG => "img",
        slir::K_PATH => "path",
        slir::K_SPACER => "spacer",
        slir::K_HOLE => "hole",
        slir::K_EACH => "each",
        slir::K_DIVIDER => "divider",
        slir::K_ICON => "icon",
        _ => "unknown",
    }
}

fn scene_entry(doc: &LoadedDoc, index: usize) -> Value {
    let retained = &doc.inst.sc;
    let scene_string = |reference: u32| {
        let index = usize::try_from(reference).expect("scene string index must fit usize");
        doc.inst
            .st
            .scene_strs
            .get(index)
            .map(String::as_str)
            .unwrap_or("")
    };
    let flags = retained.flags[index];
    json!({
        "i": i32::try_from(index).expect("scene index must fit i32"),
        "node": retained.node[index],
        "key": scene::key_of(&doc.inst.doc, &doc.inst.st.lists, retained.node[index]),
        "parent": retained.parent[index],
        "kind": kind_name(retained.kind[index]),
        "x": retained.x[index],
        "y": retained.y[index],
        "w": retained.w[index],
        "h": retained.h[index],
        "radius": retained.radius[index],
        "rot": retained.rot[index],
        "cx": retained.cx[index],
        "cy": retained.cy[index],
        "flags": flags,
        "clip": flags & slir::F_CLIP != 0,
        "scroll": flags & slir::F_SCROLL != 0,
        "scroll_cross_enabled": flags & slir::F_SCROLL_CROSS != 0,
        "inert": flags & slir::F_INERT != 0,
        "focusable": flags & slir::F_FOCUSABLE != 0,
        "detached": flags & slir::F_DETACHED != 0,
        "scroll_off": retained.scroll_off[index],
        "content_main": retained.content_main[index],
        "scroll_cross": retained.scroll_cross[index],
        "content_cross": retained.content_cross[index],
        "role": scene_string(retained.role[index]),
        "label": scene_string(retained.label[index]),
        "desc": scene_string(retained.desc[index]),
        "checked": match retained.checked[index] {
            1 => Value::Bool(false),
            2 => Value::Bool(true),
            3 => Value::String("mixed".into()),
            _ => Value::Null,
        },
        "expanded": match retained.expanded[index] {
            1 => Value::Bool(false),
            2 => Value::Bool(true),
            _ => Value::Null,
        },
        "selected": match retained.selected[index] {
            1 => Value::Bool(false),
            2 => Value::Bool(true),
            _ => Value::Null,
        },
        "active_descendant": scene_string(retained.active_descendant[index]),
        "controls": scene_string(retained.controls[index]),
        "value_now": retained.value_now[index],
        "value_min": retained.value_min[index],
        "value_max": retained.value_max[index],
        "value_text": scene_string(retained.value_text[index]),
        "modal": match retained.modal[index] {
            1 => Value::Bool(false),
            2 => Value::Bool(true),
            _ => Value::Null,
        },
        "live": match retained.live[index] {
            1 => Value::String("off".into()),
            2 => Value::String("polite".into()),
            3 => Value::String("assertive".into()),
            _ => Value::Null,
        },
        "live_atomic": match retained.live_atomic[index] {
            1 => Value::Bool(false),
            2 => Value::Bool(true),
            _ => Value::Null,
        },
        "level": retained.level[index],
        "pos_in_set": retained.pos_in_set[index],
        "set_size": retained.set_size[index],
        "disabled": retained.disabled[index],
        "focused": retained.focused[index],
        "is_row": retained.is_row[index],
    })
}

fn node_scene_index(doc: &LoadedDoc, key: &str) -> ProtocolResult<(u32, usize)> {
    let node = scene::node_by_key(&doc.inst.doc, &doc.inst.st.lists, key);
    if node == slir::NONE {
        return Err(domain(format!("unknown key '{key}'")));
    }
    let index = scene::index_of(&doc.inst.sc, node);
    let index =
        usize::try_from(index).map_err(|_| domain(format!("key '{key}' is not in scene")))?;
    Ok((node, index))
}

fn scene_tree(session: &mut Session) -> ProtocolResult {
    let doc = ensure_frame(session)?;
    let nodes = (0..doc.inst.sc.node.len())
        .map(|index| scene_entry(doc, index))
        .collect::<Vec<_>>();
    Ok(json!({"nodes": nodes}))
}

fn requested_states(object: &Map<String, Value>) -> ProtocolResult<Vec<String>> {
    let Some(value) = object.get("states") else {
        return Ok(Vec::new());
    };
    let states = value
        .as_array()
        .ok_or_else(|| invalid("'states' must be an array of strings"))?;
    states
        .iter()
        .map(|state| {
            state
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| invalid("'states' must be an array of strings"))
        })
        .collect()
}

fn scene_node(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let extras = requested_states(object)?;
    let doc = ensure_frame(session)?;
    let (node, index) = node_scene_index(doc, &key)?;
    let mut states = Map::new();
    states.insert(
        "hover".into(),
        Value::Bool(doc.inst.ds.hover.contains(&node)),
    );
    states.insert("pressed".into(), Value::Bool(doc.inst.ds.pressed == node));
    states.insert("focus".into(), Value::Bool(doc.inst.ds.fs.focus == node));
    states.insert(
        "focus_visible".into(),
        Value::Bool(doc.inst.ds.fs.focus == node && doc.inst.ds.fs.visible),
    );
    states.insert(
        "disabled".into(),
        Value::Bool(dispatch::disabled(&doc.inst.doc, &doc.inst.st, node)),
    );
    for state in extras {
        if !states.contains_key(&state) {
            let on = style::node_state_on(&doc.inst.doc, &doc.inst.st, node, &state);
            states.insert(state, Value::Bool(on));
        }
    }
    let mut entry = scene_entry(doc, index)
        .as_object()
        .expect("scene entry is an object")
        .clone();
    entry.insert("states".into(), Value::Object(states));
    Ok(Value::Object(entry))
}

fn text_in_subtree(doc: &LoadedDoc, target: u32, text_node: u32, chain: &mut Vec<i32>) -> bool {
    let index = scene::index_of(&doc.inst.sc, text_node);
    if index < 0 {
        return false;
    }
    scene::chain(&doc.inst.sc, index, chain);
    chain.iter().any(|index| {
        let index = usize::try_from(*index).expect("scene chain index must be nonnegative");
        doc.inst.sc.node[index] == target
    })
}

fn scene_text(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = required_str(object, "key")?.to_string();
    let doc = ensure_frame(session)?;
    let (target, _) = node_scene_index(doc, &key)?;
    let mut chain = Vec::new();
    let runs = doc
        .fr
        .ops
        .iter()
        .filter_map(|operation| {
            let FrameOp::Text(text) = operation else {
                return None;
            };
            if !text_in_subtree(doc, target, text.node, &mut chain) {
                return None;
            }
            let string_index =
                usize::try_from(text.str_ref).expect("text string index must fit usize");
            Some(json!({
                "text": doc.fr.strings[string_index],
                "x": text.x,
                "y": text.y_baseline,
            }))
        })
        .collect::<Vec<_>>();
    let text = runs
        .iter()
        .filter_map(|run| run["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(json!({"text": text, "runs": runs}))
}

fn rect_value(doc: &LoadedDoc, index: usize) -> Value {
    json!({
        "x": doc.inst.sc.x[index],
        "y": doc.inst.sc.y[index],
        "w": doc.inst.sc.w[index],
        "h": doc.inst.sc.h[index],
    })
}

fn scene_hit(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let x = required_f64(object, "x")?;
    let y = required_f64(object, "y")?;
    let doc = ensure_frame(session)?;
    let nodes = frame::inst_hit(&doc.inst, x, y);
    let keys = nodes
        .iter()
        .map(|node| Value::String(scene::key_of(&doc.inst.doc, &doc.inst.st.lists, *node)))
        .collect::<Vec<_>>();
    let rects = nodes
        .iter()
        .filter_map(|node| usize::try_from(scene::index_of(&doc.inst.sc, *node)).ok())
        .map(|index| rect_value(doc, index))
        .collect::<Vec<_>>();
    Ok(json!({"keys": keys, "nodes": nodes, "rects": rects}))
}

fn scene_find(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let needle = required_str(object, "text")?.to_string();
    let doc = ensure_frame(session)?;
    let mut matches = Vec::new();
    for (index, node) in doc.inst.sc.node.iter().copied().enumerate() {
        let matching_text = doc.fr.ops.iter().find_map(|operation| {
            let FrameOp::Text(text) = operation else {
                return None;
            };
            if text.node != node {
                return None;
            }
            let string_index =
                usize::try_from(text.str_ref).expect("text string index must fit usize");
            doc.fr.strings[string_index]
                .contains(&needle)
                .then(|| doc.fr.strings[string_index].clone())
        });
        if let Some(text) = matching_text {
            matches.push(json!({
                "key": scene::key_of(&doc.inst.doc, &doc.inst.st.lists, node),
                "node": node,
                "text": text,
                "rect": rect_value(doc, index),
            }));
        }
    }
    Ok(json!({"matches": matches}))
}

fn parse_kernel_json(raw: &str) -> ProtocolResult {
    serde_json::from_str(raw).map_err(|error| domain(format!("kernel JSON error: {error}")))
}

fn frame_dump(session: &mut Session) -> ProtocolResult {
    let doc = ensure_frame(session)?;
    parse_kernel_json(&slab_kernel::dumpjson::dump(
        &doc.inst.doc,
        &doc.inst.st,
        &doc.fr,
    ))
}

fn frame_summary(session: &mut Session) -> ProtocolResult {
    let doc = ensure_frame(session)?;
    parse_kernel_json(&slab_kernel::dumpjson::dump_trace_summary(
        &doc.inst.doc,
        &doc.inst.st,
        &doc.inst,
    ))
}

fn validate_mods(object: &Map<String, Value>) -> ProtocolResult<u32> {
    let Some(value) = object.get("mods") else {
        return Ok(0);
    };
    let modifiers = value
        .as_array()
        .ok_or_else(|| invalid("'mods' must be an array"))?;
    for modifier in modifiers {
        let Some(modifier) = modifier.as_str() else {
            return Err(invalid("'mods' must contain strings"));
        };
        if !matches!(modifier, "shift" | "alt" | "ctrl" | "meta") {
            return Err(invalid(format!("unknown modifier '{modifier}'")));
        }
    }
    Ok(wire::mods_of(value))
}

fn validate_event(object: &Map<String, Value>) -> ProtocolResult<()> {
    let _ = required_str(object, "type")?;
    for name in ["x", "y", "dx", "dy"] {
        if object.contains_key(name) {
            let _ = required_f64(object, name)?;
        }
    }
    let _ = optional_u32(object, "button", 0)?;
    let _ = optional_u32(object, "clicks", 0)?;
    for name in ["key", "text"] {
        if object.contains_key(name) {
            let _ = required_str(object, name)?;
        }
    }
    let _ = validate_mods(object)?;
    Ok(())
}

fn input_event(session: &mut Session, value: &Value) -> ProtocolResult {
    validate_event(params(value))?;
    let event = wire::build_event(value).map_err(invalid)?;
    dispatch_one(session, &event)
}

fn base_event(etype: u32) -> Event {
    Event {
        etype,
        x: 0.0,
        y: 0.0,
        dx: 0.0,
        dy: 0.0,
        button: 0,
        clicks: 0,
        key: String::new(),
        text: String::new(),
        mods: 0,
    }
}

fn pointer_event(etype: u32, x: f64, y: f64, button: u32, clicks: u32, mods: u32) -> Event {
    let mut event = base_event(etype);
    event.x = x;
    event.y = y;
    event.button = button;
    event.clicks = clicks;
    event.mods = mods;
    event
}

fn input_pointer(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let etype = match required_str(object, "type")? {
        "move" => dispatch::E_POINTER_MOVE,
        "down" => dispatch::E_POINTER_DOWN,
        "up" => dispatch::E_POINTER_UP,
        other => return Err(invalid(format!("unknown pointer type '{other}'"))),
    };
    let event = pointer_event(
        etype,
        required_f64(object, "x")?,
        required_f64(object, "y")?,
        optional_u32(object, "button", 0)?,
        optional_u32(object, "clicks", 0)?,
        validate_mods(object)?,
    );
    dispatch_one(session, &event)
}

fn input_click(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let key = object.get("key");
    let has_x = object.contains_key("x");
    let has_y = object.contains_key("y");
    if key.is_some() && (has_x || has_y) || key.is_none() && !(has_x && has_y) {
        return Err(invalid(
            "input.click needs either 'key' or both 'x' and 'y'",
        ));
    }
    let key = key
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| invalid("'key' must be a string"))
        })
        .transpose()?;
    let point = match &key {
        Some(_) => None,
        None => Some((required_f64(object, "x")?, required_f64(object, "y")?)),
    };
    let button = optional_u32(object, "button", 0)?;
    let clicks = optional_u32(object, "clicks", 1)?;
    let mods = validate_mods(object)?;
    let t_ms = session.t_ms;
    let capture_effects = session.capture_effects;
    let doc = ensure_frame(session)?;
    let (x, y) = match (key, point) {
        (Some(key), None) => {
            let (_, index) = node_scene_index(doc, &key)?;
            (
                doc.inst.sc.x[index] + doc.inst.sc.w[index] / 2.0,
                doc.inst.sc.y[index] + doc.inst.sc.h[index] / 2.0,
            )
        }
        (None, Some(point)) => point,
        _ => unreachable!("click address form validated"),
    };
    let mut effects = dispatch::effects_new();
    let mut captured = Vec::with_capacity(if capture_effects { 3 } else { 0 });
    for etype in [
        dispatch::E_POINTER_MOVE,
        dispatch::E_POINTER_DOWN,
        dispatch::E_POINTER_UP,
    ] {
        let event_clicks = if etype == dispatch::E_POINTER_DOWN {
            clicks
        } else {
            0
        };
        let next = frame::inst_dispatch(
            &mut doc.inst,
            &pointer_event(etype, x, y, button, event_clicks, mods),
        );
        if capture_effects {
            captured.push(next.clone());
        }
        merge_effects(&mut effects, next);
    }
    let result = effects_result(doc, &effects, t_ms);
    if capture_effects {
        session.pending_effects.extend(captured);
    }
    result
}

fn input_wheel(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let mut event = base_event(dispatch::E_WHEEL);
    event.x = required_f64(object, "x")?;
    event.y = required_f64(object, "y")?;
    event.dx = optional_f64(object, "dx", 0.0)?;
    event.dy = required_f64(object, "dy")?;
    event.mods = validate_mods(object)?;
    dispatch_one(session, &event)
}

fn input_key(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let mut event = base_event(dispatch::E_KEY_DOWN);
    event.key = required_str(object, "key")?.to_string();
    event.mods = validate_mods(object)?;
    dispatch_one(session, &event)
}

fn input_text(session: &mut Session, object: &Map<String, Value>, etype: u32) -> ProtocolResult {
    let mut event = base_event(etype);
    event.text = required_str(object, "text")?.to_string();
    dispatch_one(session, &event)
}

fn dispatch_one(session: &mut Session, event: &Event) -> ProtocolResult {
    let t_ms = session.t_ms;
    let capture_effects = session.capture_effects;
    let doc = ensure_frame(session)?;
    let effects = frame::inst_dispatch(&mut doc.inst, event);
    let result = effects_result(doc, &effects, t_ms);
    if capture_effects {
        session.pending_effects.push(effects);
    }
    result
}

fn effects_result(doc: &LoadedDoc, effects: &Effects, t_ms: f64) -> ProtocolResult {
    let effects = parse_kernel_json(&slab_kernel::dumpjson::dump_effects(
        &doc.inst.doc,
        &doc.inst.st,
        effects,
    ))?;
    Ok(json!({"effects": effects, "t": t_ms}))
}

fn merge_effects(combined: &mut Effects, next: Effects) {
    combined.repaint |= next.repaint;
    combined.sig_name.extend(next.sig_name);
    combined.sig_text.extend(next.sig_text);
    combined.sig_item.extend(next.sig_item);
    combined.sig_meta.extend(next.sig_meta);
    combined.scrolls.extend(next.scrolls);
    combined.has_caret = next.has_caret;
    combined.caret_x = next.caret_x;
    combined.caret_y = next.caret_y;
    combined.caret_w = next.caret_w;
    combined.caret_h = next.caret_h;
    combined.has_ime = next.has_ime;
    combined.ime_x = next.ime_x;
    combined.ime_y = next.ime_y;
    combined.ime_w = next.ime_w;
    combined.ime_h = next.ime_h;
    combined.cursor = next.cursor;
    combined.focus = next.focus;
}

fn checked_scale(scale: f64) -> ProtocolResult<f64> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(invalid("'scale' must be positive"));
    }
    Ok(scale)
}

fn runtime_images<'a>(
    inst: &'a Instance,
    frame: &Frame,
) -> Vec<slab_compile::render::RuntimeImage<'a>> {
    let compiled_len = i32::try_from(inst.doc.img_src.len()).expect("image table exceeds i32");
    let mut images: Vec<slab_compile::render::RuntimeImage<'a>> = Vec::new();
    for image in frame.ops.iter().filter_map(|op| match op {
        FrameOp::Image(image) if image.img >= compiled_len => Some(image.img),
        _ => None,
    }) {
        if images.iter().any(|candidate| candidate.image == image) {
            continue;
        }
        let Some((width, height, format, generation)) = frame::inst_img_info(inst, image) else {
            continue;
        };
        images.push(slab_compile::render::RuntimeImage {
            image,
            width,
            height,
            format,
            generation,
            bytes: frame::inst_img_bytes(inst, image),
        });
    }
    images
}

fn render_png(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let scale = checked_scale(optional_f64(object, "scale", 1.0)?)?;
    let path = optional_path(object)?;
    let doc = ensure_frame(session)?;
    let runtime_images = runtime_images(&doc.inst, &doc.fr);
    let bytes = slab_compile::raster::render_png(
        &doc.slir,
        &doc.images,
        &runtime_images,
        &doc.fonts,
        &doc.fr,
        scale,
    )
    .map_err(domain)?;
    let (width, height) = png_dimensions(&bytes)?;
    let notes =
        slab_compile::capsnote::render_notes(&doc.inst.doc, &doc.fr, doc.inst.st.env.client, &[]);
    let mut result = Map::new();
    result.insert("width_px".into(), json!(width));
    result.insert("height_px".into(), json!(height));
    result.insert("notes".into(), json!(notes));
    add_binary_payload(&mut result, &bytes, path.as_deref())?;
    Ok(Value::Object(result))
}

fn png_dimensions(bytes: &[u8]) -> ProtocolResult<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(domain("raster renderer returned invalid PNG data"));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Ok((width, height))
}

fn render_svg(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let path = optional_path(object)?;
    let doc = ensure_frame(session)?;
    let runtime_images = runtime_images(&doc.inst, &doc.fr);
    let svg = slab_compile::svg::render_svg(
        &doc.slir,
        &doc.images,
        &runtime_images,
        &doc.fonts,
        &doc.fr,
        &doc.base_dir,
    );
    let notes =
        slab_compile::capsnote::render_notes(&doc.inst.doc, &doc.fr, doc.inst.st.env.client, &[]);
    let mut result = Map::new();
    result.insert("bytes".into(), json!(byte_count(svg.as_bytes())?));
    result.insert("notes".into(), json!(notes));
    match path {
        Some(path) => {
            std::fs::write(&path, svg.as_bytes())
                .map_err(|error| domain(format!("{path}: {error}")))?;
            result.insert("path".into(), Value::String(path));
        }
        None => {
            result.insert("data".into(), Value::String(svg));
        }
    }
    Ok(Value::Object(result))
}

fn render_cells(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let plain = optional_bool(object, "plain", true)?;
    let path = optional_path(object)?;
    let doc = ensure_frame(session)?;
    let grid = cells::cells_from_frame(&doc.inst.doc, &doc.fr, doc.fr.width, doc.fr.height);
    let text = cells::cells_to_text(&grid, plain);
    let mut notes = grid
        .diag_code
        .iter()
        .zip(&grid.diag_msg)
        .map(|(code, message)| format!("note {code}: {message}"))
        .collect::<Vec<_>>();
    notes.extend(slab_compile::capsnote::render_notes(
        &doc.inst.doc,
        &doc.fr,
        doc.inst.st.env.client,
        &grid.diag_code,
    ));
    let mut result = Map::new();
    result.insert(
        "cols".into(),
        json!(u32::try_from(grid.cols).expect("cell columns must be nonnegative")),
    );
    result.insert(
        "rows".into(),
        json!(u32::try_from(grid.rows).expect("cell rows must be nonnegative")),
    );
    result.insert("notes".into(), json!(notes));
    match path {
        Some(path) => {
            result.insert("bytes".into(), json!(byte_count(text.as_bytes())?));
            std::fs::write(&path, text.as_bytes())
                .map_err(|error| domain(format!("{path}: {error}")))?;
            result.insert("path".into(), Value::String(path));
        }
        None => {
            result.insert("text".into(), Value::String(text));
        }
    }
    Ok(Value::Object(result))
}

fn render_apng(session: &mut Session, object: &Map<String, Value>) -> ProtocolResult {
    let dur = optional_f64(object, "dur", 2.0)?;
    let fps = optional_f64(object, "fps", 20.0)?;
    let scale = checked_scale(optional_f64(object, "scale", 1.0)?)?;
    let path = optional_path(object)?;
    if !dur.is_finite() || dur < 0.0 {
        return Err(invalid("'dur' must be nonnegative"));
    }
    if !fps.is_finite() || fps <= 0.0 {
        return Err(invalid("'fps' must be positive"));
    }
    let frame_count = rounded_frame_count(dur, fps)?;
    let start = session.t_ms;
    let doc = ensure_frame(session)?;
    let mut rendered = Vec::with_capacity(frame_count);
    let mut last_frame = None;
    let mut last_t = start;
    {
        let LoadedDoc {
            slir,
            inst,
            images,
            fonts,
            ..
        } = doc;
        for index in 0..frame_count {
            let index = u32::try_from(index).expect("validated APNG frame count fits u32");
            let t_ms = start + f64::from(index) * 1000.0 / fps;
            let current = frame::inst_frame(inst, t_ms);
            let runtime_images = runtime_images(inst, &current);
            let mut raster =
                slab_compile::raster::Raster::new(slir, images, &runtime_images, fonts, scale);
            rendered.push(raster.render(&current).map_err(domain)?);
            last_t = t_ms;
            last_frame = Some(current);
        }
    }
    let bytes = slab_compile::raster::encode_apng(&rendered, fps, 0).map_err(domain)?;
    doc.fr = last_frame.expect("APNG always renders at least one frame");
    session.t_ms = last_t;

    let mut result = Map::new();
    result.insert(
        "frames".into(),
        json!(u32::try_from(frame_count).expect("validated APNG frame count fits u32")),
    );
    result.insert("t".into(), json!(last_t));
    add_binary_payload(&mut result, &bytes, path.as_deref())?;
    Ok(Value::Object(result))
}

fn rounded_frame_count(dur: f64, fps: f64) -> ProtocolResult<usize> {
    let rounded = (dur * fps).round().max(1.0);
    if !rounded.is_finite() || rounded > f64::from(u32::MAX) {
        return Err(invalid("APNG frame count is too large"));
    }
    format!("{rounded:.0}")
        .parse()
        .map_err(|_| invalid("invalid APNG frame count"))
}

fn add_binary_payload(
    result: &mut Map<String, Value>,
    bytes: &[u8],
    path: Option<&str>,
) -> ProtocolResult<()> {
    result.insert("bytes".into(), json!(byte_count(bytes)?));
    match path {
        Some(path) => {
            std::fs::write(path, bytes).map_err(|error| domain(format!("{path}: {error}")))?;
            result.insert("path".into(), Value::String(path.to_string()));
        }
        None => {
            result.insert("data".into(), Value::String(b64(bytes)));
        }
    }
    Ok(())
}

fn byte_count(bytes: &[u8]) -> ProtocolResult<u64> {
    u64::try_from(bytes.len()).map_err(|_| domain("payload is too large"))
}

fn b64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(char::from(TABLE[usize::from(first >> 2)]));
        output.push(char::from(
            TABLE[usize::from((first & 0x03) << 4 | second >> 4)],
        ));
        if chunk.len() > 1 {
            output.push(char::from(
                TABLE[usize::from((second & 0x0f) << 2 | third >> 6)],
            ));
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(char::from(TABLE[usize::from(third & 0x3f)]));
        } else {
            output.push('=');
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_document() -> (Slir, Instance, Vec<Vec<u8>>) {
        let source = r#"
params {
  draft text = "start"
}
text#field param.draft field=draft size=14 w=200 nowrap
"#;
        let options = slab_compile::Options {
            embed_assets: true,
            base_dir: PathBuf::from("."),
            assets: None,
            fonts: std::collections::HashMap::new(),
        };
        let (slir, diagnostics) = slab_compile::compile(source, &options);
        assert!(!diagnostics.has_errors(), "{diagnostics:?}");
        let slir = slir.expect("test source compiles");
        let bytes = slab_slir::write(&slir);
        let (mut instance, images) = slab_slir::instance(&bytes).expect("test SLIR decodes");
        frame::inst_set_env(&mut instance, 320.0, 100.0, 1, false, false);
        (slir, instance, images)
    }

    fn result(response: &PumpResponse) -> &Value {
        &response.response["result"]
    }

    #[test]
    fn pumps_field_param_and_focus_methods_on_caller_instance() {
        let (slir, mut instance, images) = live_document();
        let mut pump = RequestPump::new("test.slab", slir, images);

        let parameter = pump.request(
            &mut instance,
            r#"{"id":1,"method":"param.get","params":{"name":"draft"}}"#,
        );
        assert_eq!(result(&parameter), &json!({"value": "start"}));

        let changed = pump.request(
            &mut instance,
            r#"{"id":2,"method":"field.set","params":{"key":"field","text":"edited"}}"#,
        );
        assert_eq!(result(&changed), &json!({"ok": true, "changed": true}));
        let field = pump.request(
            &mut instance,
            r#"{"id":3,"method":"field.get","params":{"key":"field"}}"#,
        );
        assert_eq!(result(&field), &json!({"text": "edited"}));

        let _ = pump.request(
            &mut instance,
            r#"{"id":4,"method":"focus.set","params":{"key":"field"}}"#,
        );
        let focus = pump.request(
            &mut instance,
            r#"{"id":5,"method":"focus.get","params":{}}"#,
        );
        assert_eq!(result(&focus)["key"], "#field");
        assert_ne!(result(&focus)["focus"], json!(slir::NONE));
    }

    #[test]
    fn returns_dispatch_effects_for_host_handling() {
        let (slir, mut instance, images) = live_document();
        let mut pump = RequestPump::new("test.slab", slir, images);
        let _ = pump.request(
            &mut instance,
            r#"{"id":1,"method":"focus.set","params":{"key":"field"}}"#,
        );
        let response = pump.request(
            &mut instance,
            r#"{"id":2,"method":"input.text","params":{"text":"!"}}"#,
        );
        assert_eq!(response.effects.len(), 1);
        assert!(!response.effects[0].sig_name.is_empty());
    }
}
