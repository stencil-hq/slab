//! `slab-native` — native wgpu client: open any `.slab` document, or run
//! the baked-in demo binaries' documents (`--demo settings|player|modern`).

use slab_native::{demo, player, view};
use std::process::ExitCode;

const USAGE: &str = "\
usage: slab-native FILE.slab [options]
       slab-native --demo settings|player|modern [options]

options:
  --headless-frame OUT.png   render one frame offscreen and write a PNG
  --width N                  logical width  (default 900; player 360)
  --height N                 logical height (default 640; player 584)
  --t MS                     motion clock for --headless-frame (default 0)
  --scale N                  headless device scale factor (default 1)
  --dark                     env dark flag
  --undecorated              borderless window (no title bar)
  --theme NAME               select a compiler-declared theme
  --frames N                 windowed: exit after N presented frames
  --exit-after-ms MS         windowed: exit after MS milliseconds

undecorated windows set the document-global state `undecorated` (render
your own chrome behind `when undecorated { … }`) and honor the reserved
activation signals act=window-close | window-minimize | window-maximize
(toggle) | window-drag (titlebar region: OS window move on press).
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut demo_name: Option<String> = None;
    let mut file: Option<String> = None;
    let mut opts = demo::Opts::default();
    let mut size_set = (false, false);
    let mut it = args.iter().peekable();
    let err = |m: String| -> ExitCode {
        eprintln!("error: {m}");
        eprint!("{USAGE}");
        ExitCode::from(2)
    };
    while let Some(a) = it.next() {
        let mut val = |name: &str| -> Result<String, String> {
            it.next()
                .cloned()
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match a.as_str() {
            "--demo" => match val("--demo") {
                Ok(v) => demo_name = Some(v),
                Err(e) => return err(e),
            },
            "--headless-frame" => match val("--headless-frame") {
                Ok(v) => opts.headless_out = Some(v.into()),
                Err(e) => return err(e),
            },
            "--width" => match val("--width").and_then(|v| v.parse().map_err(|e| format!("{e}"))) {
                Ok(v) => {
                    opts.width = v;
                    size_set.0 = true;
                }
                Err(e) => return err(e),
            },
            "--height" => {
                match val("--height").and_then(|v| v.parse().map_err(|e| format!("{e}"))) {
                    Ok(v) => {
                        opts.height = v;
                        size_set.1 = true;
                    }
                    Err(e) => return err(e),
                }
            }
            "--t" => match val("--t").and_then(|v| v.parse().map_err(|e| format!("{e}"))) {
                Ok(v) => opts.t = v,
                Err(e) => return err(e),
            },
            "--scale" => match val("--scale").and_then(|v| v.parse().map_err(|e| format!("{e}"))) {
                Ok(v) => opts.scale = Some(v),
                Err(e) => return err(e),
            },
            "--frames" => {
                match val("--frames").and_then(|v| v.parse().map_err(|e| format!("{e}"))) {
                    Ok(v) => opts.max_frames = Some(v),
                    Err(e) => return err(e),
                }
            }
            "--exit-after-ms" => {
                match val("--exit-after-ms").and_then(|v| v.parse().map_err(|e| format!("{e}"))) {
                    Ok(v) => opts.exit_after_ms = Some(v),
                    Err(e) => return err(e),
                }
            }
            "--dark" => opts.dark = true,
            "--undecorated" => opts.undecorated = true,
            "--theme" => match val("--theme") {
                Ok(v) => opts.theme = Some(v),
                Err(e) => return err(e),
            },
            "--help" | "-h" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other if !other.starts_with('-') && file.is_none() => file = Some(other.to_string()),
            other => return err(format!("unknown argument '{other}'")),
        }
    }
    if let Some(path) = &file {
        if demo_name.is_some() {
            return err("pass either FILE.slab or --demo, not both".into());
        }
        if opts.theme.is_some() {
            return err("--theme applies to --demo settings only".into());
        }
        return match view::run(std::path::Path::new(path), opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match demo_name.as_deref() {
        Some("settings") => match demo::run(opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("player") => {
            // the player card is 360u wide, ~584u tall (hug)
            if !size_set.0 {
                opts.width = 360.0;
            }
            if !size_set.1 {
                opts.height = 584.0;
            }
            match player::run(opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("modern") => {
            // the modern FX showcase document is 900u × 640u
            if !size_set.0 {
                opts.width = 900.0;
            }
            if !size_set.1 {
                opts.height = 640.0;
            }
            match view::run_source(
                "13-modern",
                include_str!("../../../examples/13-modern.slab"),
                std::path::PathBuf::from("."),
                opts,
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(other) => err(format!(
            "unknown demo '{other}' (only: settings, player, modern)"
        )),
        None => err("missing FILE.slab or --demo settings|player|modern".into()),
    }
}
