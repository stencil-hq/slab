//! `slab-tui` — interactive terminal client for slab documents (P8).
//! Compiles FILE.slab, drives the hand-maintained Rust kernel (`slab-kernel`), and
//! either runs a live crossterm loop or a headless `--script` replay.

mod app;
mod images;
mod interactive;
mod player;
mod script;

use slab_kernel::frame as kframe;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
usage: slab-tui FILE.slab [--width N] [--height N] [--fps N] [--env dark,coarse]
                          [--theme NAME] [--set param=value]... [--debug] [--app player]
                          [--images auto|on|off] [--script 'TOKENS']
                          [--dump-after PATH] [--ansi]
       slab-tui --examples [DIR | FILE.slab] [same options]

  interactive (default): alt-screen loop; terminal size drives env
      (cell = 8x16 slab units), Tab/Enter/arrows/typing and mouse
      click/wheel dispatch into the kernel; Ctrl-C exits. Bracketed
      paste inserts as one undo step; where the terminal speaks the
      kitty keyboard protocol Shift+Enter inserts a newline in
      multiline fields (legacy terminals: Alt+Enter). --debug
      reserves a terminal row for a signal footer.
  --examples  gallery: load every .slab in DIR (default ./examples, or the
              directory of FILE when one is given, starting on that file)
              and switch with Ctrl-N / Ctrl-P; the last terminal row shows
              the position and bindings. Interactive only.
  --set       override a declared param before the first frame, repeatable
              (text/num/pct/color/bool/enum scalars, or a JSON array of
               typed objects for list params; bad input is an error, exit 2)
  --app player  run FILE as the music-player app: transport signals drive
              a 4-track playlist (params re-solved through the kernel),
              play advances elapsed/remain/progress in real time, and
              shuffle/loop show --debug footer badges. The queue hole
              stays blank on tui (host-filled; cells has no host).
  --script    headless: replay tokens at t=0,16,32,... then exit
              (TAB STAB ENTER SPACE BACKSPACE DELETE HOME END LEFT RIGHT
               UP DOWN TICK TICK:ms TYPE:text PASTE:text MOUSE:x,y
               CLICK:x,y WHEEL:x,y,dy)
  --dump-after  write the final cell grid (- = stdout; --ansi keeps the
              SGR colors); with
              --script a trailing 'signals: ...' line is appended.
              Without a script the dump matches
              `slab render FILE --client tui --plain` byte-for-byte.
  --width/--height  headless env in slab units (default 800 x unbounded);
              ignored interactively (terminal size wins)
  --images    kitty-graphics image painting: auto (default) enables it when
              the terminal is kitty/ghostty/WezTerm, on forces it, off keeps
              the shaded cell placeholder (iTerm2's OSC protocol is not
              spoken; it stays on the placeholder)
";

struct Args {
    file: Option<PathBuf>,
    width: f64,
    height: f64,
    fps: f64,
    debug: bool,
    ansi: bool,
    theme: Option<String>,
    env: Vec<String>,
    script: Option<String>,
    dump_after: Option<PathBuf>,
    sets: Vec<(String, String)>,
    app: Option<String>,
    examples: bool,
    images: images::Mode,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        file: None,
        width: 800.0,
        height: 0.0,
        fps: 30.0,
        debug: false,
        ansi: false,
        env: Vec::new(),
        theme: None,
        script: None,
        dump_after: None,
        sets: Vec::new(),
        app: None,
        examples: false,
        images: images::Mode::Auto,
    };
    let mut file = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut val = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match arg.as_str() {
            "--width" => a.width = val("--width")?.parse().map_err(|_| "bad --width")?,
            "--height" => a.height = val("--height")?.parse().map_err(|_| "bad --height")?,
            "--fps" => a.fps = val("--fps")?.parse().map_err(|_| "bad --fps")?,
            "--debug" => a.debug = true,
            "--examples" => a.examples = true,
            "--ansi" => a.ansi = true,
            "--app" => a.app = Some(val("--app")?),
            "--images" => {
                let v = val("--images")?;
                a.images =
                    images::Mode::parse(&v).ok_or(format!("bad --images '{v}' (auto|on|off)"))?;
            }
            "--env" => a.env.extend(val("--env")?.split(',').map(str::to_string)),
            "--theme" => a.theme = Some(val("--theme")?),
            "--script" => a.script = Some(val("--script")?),
            "--dump-after" => a.dump_after = Some(PathBuf::from(val("--dump-after")?)),
            "--set" => {
                let v = val("--set")?;
                let (k, v) = v.split_once('=').ok_or("--set needs param=value")?;
                a.sets.push((k.to_string(), v.to_string()));
            }
            "-h" | "--help" => return Err(USAGE.to_string()),
            other if other.starts_with('-') => return Err(format!("unknown flag {other}")),
            other if file.is_none() => file = Some(PathBuf::from(other)),
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    a.file = file;
    if a.file.is_none() && !a.examples {
        return Err("slab-tui needs a FILE (see --help)".to_string());
    }
    Ok(a)
}

/// Documents to drive, plus the one to start on.
///
/// Plain mode is the single positional FILE. `--examples` collects every
/// `.slab` in the target directory — the positional argument when it names a
/// directory, its parent when it names a file (starting there), else
/// `./examples` — sorted by name so the gallery order matches the file order.
fn documents(a: &Args) -> Result<(Vec<PathBuf>, usize), String> {
    let file = a.file.as_deref();
    if !a.examples {
        return Ok((vec![file.expect("checked in parse").to_path_buf()], 0));
    }
    let dir = match file {
        Some(p) if p.is_dir() => p.to_path_buf(),
        Some(p) if p.is_file() => p
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf(),
        Some(p) => return Err(format!("{}: no such file or directory", p.display())),
        None => PathBuf::from("examples"),
    };
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("--examples {}: {e}", dir.display()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "slab"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("--examples: no .slab files in {}", dir.display()));
    }
    let start = file
        .filter(|p| !p.is_dir())
        .and_then(|p| files.iter().position(|f| f == p))
        .unwrap_or(0);
    Ok((files, start))
}

/// One compiled document, ready to drive.
struct Doc {
    inst: kframe::Instance,
    app: Option<player::PlayerApp>,
    /// Embedded image payloads, in document image order.
    images: Vec<Vec<u8>>,
    /// Formatted compile warnings, deferred until the terminal is restored.
    warnings: Vec<String>,
}

/// Compile one document and apply `--theme`, `--set` and `--app` to it.
fn load(a: &Args, file: &std::path::Path) -> Result<Doc, (u8, String)> {
    let (bytes, warnings) = app::compile(file).map_err(|e| (1, e))?;
    let (mut inst, images) = app::instance(&bytes).map_err(|e| (1, e))?;
    if let Some(theme) = &a.theme
        && !kframe::inst_set_theme(&mut inst, theme)
    {
        return Err((2, format!("unknown theme '{theme}'")));
    }
    slab_compile::input::apply_sets(&mut inst, &a.sets).map_err(|e| (2, e))?;
    let app = match a.app.as_deref() {
        None => None,
        Some("player") => Some(player::PlayerApp::new(&mut inst).map_err(|e| (1, e))?),
        Some(other) => return Err((1, format!("unknown --app '{other}' (only: player)"))),
    };
    Ok(Doc {
        inst,
        app,
        images,
        warnings,
    })
}

/// `--env` flags as (dark, coarse).
fn env_flags(a: &Args) -> (bool, bool) {
    (
        a.env.iter().any(|e| e == "dark"),
        a.env.iter().any(|e| e == "coarse"),
    )
}

/// One terminal session over `files`, starting at `start`: Ctrl-N/Ctrl-P
/// reload in place rather than leaving and re-entering the alt screen.
/// Compile warnings are collected for the caller to print afterwards, since
/// stderr lands on top of the document while the alt screen is up.
fn session(
    a: &Args,
    files: &[PathBuf],
    start: usize,
    warnings: &mut Vec<String>,
) -> Result<(), (u8, String)> {
    let (dark, coarse) = env_flags(a);
    let term = interactive::Term::new().map_err(|e| (1, e))?;
    let mut index = start;
    loop {
        let file = &files[index];
        let mut doc = load(a, file)?;
        // Revisiting an example must not repeat its warnings.
        for w in doc.warnings.drain(..) {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        let base_dir = file.parent().unwrap_or(std::path::Path::new("."));
        let imgpaint = images::Images::new(a.images, &doc.inst.doc, &doc.images, base_dir);
        let ui = interactive::Ui {
            fps: a.fps,
            debug: a.debug,
            dark,
            coarse,
            gallery: a.examples.then_some(interactive::Gallery { files, index }),
        };
        match term
            .run(&mut doc.inst, doc.app.as_mut(), imgpaint, &ui)
            .map_err(|e| (1, e))?
        {
            interactive::Exit::Quit => return Ok(()),
            interactive::Exit::Switch(next) => index = next,
        }
    }
}

/// Errors carry the process exit code: 1 = general, 2 = bad --set.
fn run(a: &Args) -> Result<(), (u8, String)> {
    let (files, start) = documents(a).map_err(|e| (1, e))?;

    if a.script.is_some() || a.dump_after.is_some() {
        if a.examples {
            return Err((1, "--examples is interactive-only".to_string()));
        }
        // headless: fixed env like `slab render --client tui`
        let mut doc = load(a, &files[0])?;
        for w in &doc.warnings {
            eprintln!("{w}");
        }
        let (dark, coarse) = env_flags(a);
        kframe::inst_set_env(&mut doc.inst, a.width, a.height, 2, dark, coarse);
        let text = script::run(
            &mut doc.inst,
            a.script.as_deref(),
            16.0,
            a.ansi,
            doc.app.as_mut(),
        )
        .map_err(|e| (1, e))?;
        match &a.dump_after {
            Some(p) if p.as_os_str() != "-" => {
                std::fs::write(p, &text).map_err(|e| (1, format!("{}: {e}", p.display())))?;
            }
            _ => print!("{text}"),
        }
        return Ok(());
    }

    let mut warnings: Vec<String> = Vec::new();
    let out = session(a, &files, start, &mut warnings);
    for w in &warnings {
        eprintln!("{w}");
    }
    out
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = match parse(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match run(&a) {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, e)) => {
            eprintln!("slab-tui: {e}");
            ExitCode::from(code)
        }
    }
}
