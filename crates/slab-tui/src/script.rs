//! Headless script mode: `--script 'TAB TAB ENTER TYPE:hi'` replays key
//! tokens through inst_dispatch at t = 0,16,32,… (each step frames first,
//! then dispatches — the trace-conformance order), then `--dump-after`
//! writes the final cell grid (plain, or ANSI-colored with --ansi). When
//! a script was given, a trailing `signals: …` line records every emitted
//! signal in order; without a script the plain dump is byte-identical to
//! `slab render FILE --client tui --plain` at the same --width/--height.
//! With `--app player`, signals feed the PlayerApp and `TICK:ms` advances
//! its play clock along with the frame clock.

use crate::app;
use crate::player::PlayerApp;
use slab_kernel::{cells, dispatch, frame as kframe};

pub enum Step {
    Key {
        key: String,
        mods: u32,
    },
    Space,
    Text(String),
    /// Wholesale paste (`E_PASTE`): one undo step in the kernel.
    Paste(String),
    Move {
        x: f64,
        y: f64,
    },
    Click {
        x: f64,
        y: f64,
    },
    Wheel {
        x: f64,
        y: f64,
        dy: f64,
    },
    /// Bare TICK re-frames at the current clock; TICK:ms first advances
    /// the clock (and the app clock) by ms.
    Tick {
        ms: f64,
    },
}

/// Whitespace-separated tokens: TAB STAB ENTER SPACE ESC BACKSPACE DELETE
/// INSERT HOME END PAGEUP PAGEDOWN LEFT RIGHT UP DOWN F1..F24 TICK |
/// TICK:ms | TYPE:text | PASTE:text (`\n` → newline) | MOUSE:x,y |
/// CLICK:x,y | WHEEL:x,y,dy.
pub fn parse(script: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for tok in script.split_whitespace() {
        let named = |key: &str, mods: u32| Step::Key {
            key: key.to_string(),
            mods,
        };
        steps.push(match tok {
            "TAB" => named("Tab", 0),
            "STAB" => named("Tab", app::M_SHIFT),
            "ENTER" => named("Enter", 0),
            "SPACE" => Step::Space,
            "ESC" => named("Escape", 0),
            "BACKSPACE" => named("Backspace", 0),
            "DELETE" => named("Delete", 0),
            "INSERT" => named("Insert", 0),
            "HOME" => named("Home", 0),
            "END" => named("End", 0),
            "PAGEUP" => named("PageUp", 0),
            "PAGEDOWN" => named("PageDown", 0),
            "LEFT" => named("ArrowLeft", 0),
            "RIGHT" => named("ArrowRight", 0),
            "UP" => named("ArrowUp", 0),
            "DOWN" => named("ArrowDown", 0),
            "TICK" => Step::Tick { ms: 0.0 },
            _ => {
                if let Some(number) = tok.strip_prefix('F').and_then(|n| n.parse::<u8>().ok())
                    && (1..=24).contains(&number)
                {
                    named(&format!("F{number}"), 0)
                } else if let Some(text) = tok.strip_prefix("TYPE:") {
                    Step::Text(text.to_string())
                } else if let Some(text) = tok.strip_prefix("PASTE:") {
                    Step::Paste(text.replace("\\n", "\n"))
                } else if let Some(ms) = tok.strip_prefix("TICK:") {
                    let ms: f64 = ms.parse().map_err(|_| format!("bad token '{tok}'"))?;
                    if !ms.is_finite() || ms < 0.0 {
                        return Err(format!("bad token '{tok}' (TICK:ms needs ms >= 0)"));
                    }
                    Step::Tick { ms }
                } else if let Some(xy) = tok.strip_prefix("MOUSE:") {
                    let (x, y) = parse_pair(xy).ok_or_else(|| format!("bad token '{tok}'"))?;
                    Step::Move { x, y }
                } else if let Some(xy) = tok.strip_prefix("CLICK:") {
                    let (x, y) = parse_pair(xy).ok_or_else(|| format!("bad token '{tok}'"))?;
                    Step::Click { x, y }
                } else if let Some(rest) = tok.strip_prefix("WHEEL:") {
                    let p: Vec<&str> = rest.split(',').collect();
                    if p.len() != 3 {
                        return Err(format!("bad token '{tok}' (WHEEL:x,y,dy)"));
                    }
                    let n = |s: &str| s.parse::<f64>().ok();
                    match (n(p[0]), n(p[1]), n(p[2])) {
                        (Some(x), Some(y), Some(dy)) => Step::Wheel { x, y, dy },
                        _ => return Err(format!("bad token '{tok}'")),
                    }
                } else {
                    return Err(format!("unknown script token '{tok}'"));
                }
            }
        });
    }
    Ok(steps)
}

fn parse_pair(s: &str) -> Option<(f64, f64)> {
    let (a, b) = s.split_once(',')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// Run the script and return the dump text: the final cell grid (plain
/// or ANSI per `ansi`), plus a trailing `signals: …` line when a script
/// was supplied (even an empty one). With an app, every emitted signal
/// is forwarded to it and TICK:ms drives its clock.
pub fn run(
    inst: &mut kframe::Instance,
    script: Option<&str>,
    t_step: f64,
    ansi: bool,
    mut player: Option<&mut PlayerApp>,
) -> Result<String, String> {
    let mut signals: Vec<app::Signal> = Vec::new();
    let mut t = 0.0;
    let dispatch_at = |inst: &mut kframe::Instance,
                       t: f64,
                       ev: &dispatch::Event,
                       signals: &mut Vec<app::Signal>| {
        kframe::inst_frame(inst, t);
        app::drain_frame_signals(inst, signals);
        let eff = kframe::inst_dispatch(inst, ev);
        app::collect_signals(inst, &eff, signals);
    };
    let steps = match script {
        Some(s) => parse(s)?,
        None => Vec::new(),
    };
    let mut seen = 0usize;
    for step in &steps {
        match step {
            Step::Key { key, mods } => {
                dispatch_at(inst, t, &app::key_event(key, *mods), &mut signals)
            }
            Step::Text(text) => dispatch_at(inst, t, &app::text_event(text), &mut signals),
            Step::Paste(text) => dispatch_at(inst, t, &app::paste_event(text), &mut signals),
            Step::Space => {
                // key-down " " activates a focused button; the text event
                // inserts into a focused field — the other is a no-op.
                dispatch_at(inst, t, &app::key_event(" ", 0), &mut signals);
                let eff = kframe::inst_dispatch(inst, &app::text_event(" "));
                app::collect_signals(inst, &eff, &mut signals);
            }
            Step::Click { x, y } => {
                dispatch_at(
                    inst,
                    t,
                    &app::pointer_event(app::E_POINTER_DOWN, *x, *y),
                    &mut signals,
                );
                t += t_step;
                dispatch_at(
                    inst,
                    t,
                    &app::pointer_event(app::E_POINTER_UP, *x, *y),
                    &mut signals,
                );
            }
            Step::Wheel { x, y, dy } => {
                dispatch_at(inst, t, &app::wheel_event(*x, *y, *dy), &mut signals)
            }
            Step::Move { x, y } => dispatch_at(
                inst,
                t,
                &app::pointer_event(app::E_POINTER_MOVE, *x, *y),
                &mut signals,
            ),
            Step::Tick { ms } => {
                // Advance the app clock, then frame-STEP the interval
                // like the interactive loop would: motion tweens
                // (transition=...) only apply while a solve lands inside
                // their window, so one giant clock jump must not skip
                // them (FRAME.md motion semantics).
                if let Some(p) = player.as_deref_mut() {
                    p.advance(inst, *ms)?;
                }
                let end = t + ms;
                loop {
                    kframe::inst_frame(inst, t);
                    app::drain_frame_signals(inst, &mut signals);
                    if t >= end {
                        break;
                    }
                    t = (t + t_step).min(end);
                }
            }
        }
        if let Some(p) = player.as_deref_mut() {
            while seen < signals.len() {
                p.on_signal(inst, &signals[seen].name)?;
                seen += 1;
            }
        }
        t += t_step;
    }
    let fr = app::settle_frame(inst, t);
    app::drain_frame_signals(inst, &mut signals);
    let grid = cells::cells_with_caret(inst, &fr);
    let mut out = cells::cells_to_text(&grid, !ansi);
    if script.is_some() {
        let list: Vec<String> = signals.iter().map(app::format_signal).collect();
        if list.is_empty() {
            out.push_str("signals:\n");
        } else {
            out.push_str(&format!("signals: {}\n", list.join(" ")));
        }
    }
    Ok(out)
}
