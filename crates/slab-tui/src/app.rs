//! Shared driver plumbing: compile a `.slab` file into a live kernel
//! Instance, build kernel Events, and resolve signal names. Everything else
//! (layout, hit, focus, edit, motion, scroll, cells) is kernel-owned; this
//! crate only translates and paints.

use slab_kernel::{dispatch, flatten, frame as kframe};
use std::path::Path;

/// Kernel event type codes (spec/FRAME.md).
pub const E_POINTER_MOVE: u32 = 0;
pub const E_POINTER_DOWN: u32 = 1;
pub const E_POINTER_UP: u32 = 2;
pub const E_WHEEL: u32 = 3;
pub const E_KEY_DOWN: u32 = 4;
pub const E_TEXT: u32 = 5;
pub const E_PASTE: u32 = 6;
pub const E_CLOSE: u32 = 14;

/// Mods bitset (spec/FRAME.md): 1 shift | 2 alt | 4 ctrl | 8 meta.
pub const M_SHIFT: u32 = 1;
pub const M_ALT: u32 = 2;
pub const M_CTRL: u32 = 4;
pub const M_META: u32 = 8;

/// Compile FILE to SLIR bytes plus its formatted §12 warnings; errors come
/// back as the joined diagnostics. Nothing is printed: the interactive loop
/// owns the alt screen, so callers surface diagnostics on their own terms.
pub fn compile(file: &Path) -> Result<(Vec<u8>, Vec<String>), String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let opts = slab_compile::Options {
        embed_assets: true,
        base_dir: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
        assets: None,
    };
    let (slir, diags) = slab_compile::compile(&src, &opts);
    let name = file.display().to_string();
    if diags.has_errors() {
        let text: Vec<String> = diags.0.iter().map(|d| d.format(&name)).collect();
        return Err(text.join("\n"));
    }
    let slir = slir.ok_or("compile failed: no SLIR produced")?;
    Ok((
        slab_slir::write(&slir),
        diags.0.iter().map(|d| d.format(&name)).collect(),
    ))
}

/// Decode SLIR bytes into a kernel Instance plus embedded image payloads.
pub fn instance(bytes: &[u8]) -> Result<(kframe::Instance, Vec<Vec<u8>>), String> {
    slab_slir::instance(bytes)
}

fn event_new(etype: u32) -> dispatch::Event {
    dispatch::Event {
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

pub fn key_event(key: &str, mods: u32) -> dispatch::Event {
    let mut ev = event_new(E_KEY_DOWN);
    ev.key = key.to_string();
    ev.mods = mods;
    ev
}

pub fn text_event(text: &str) -> dispatch::Event {
    let mut ev = event_new(E_TEXT);
    ev.text = text.to_string();
    ev
}

/// Wholesale paste: the kernel places a history barrier and inserts the
/// text as one undo step (`E_PASTE`, spec/FRAME.md).
pub fn paste_event(text: &str) -> dispatch::Event {
    let mut ev = event_new(E_PASTE);
    ev.text = text.to_string();
    ev
}

/// Builds a primary-button pointer event without a click count.
pub fn pointer_event(etype: u32, x: f64, y: f64) -> dispatch::Event {
    pointer_button_event(etype, x, y, 0, 0)
}

/// Builds a pointer event with a platform button and host-computed click count.
pub fn pointer_button_event(
    etype: u32,
    x: f64,
    y: f64,
    button: u32,
    clicks: u32,
) -> dispatch::Event {
    let mut ev = event_new(etype);
    ev.x = x;
    ev.y = y;
    ev.button = button;
    ev.clicks = clicks;
    ev
}

pub fn wheel_event(x: f64, y: f64, dy: f64) -> dispatch::Event {
    let mut ev = event_new(E_WHEEL);
    ev.x = x;
    ev.y = y;
    ev.dy = dy;
    ev
}

/// Driver signal with its resolved name, payload, list identity, and metadata.
pub(crate) struct Signal {
    /// Resolved signal name.
    pub(crate) name: String,
    /// Committed text for Change, Submit, and Resize signals.
    pub(crate) text: String,
    /// Innermost list item key, or empty for a document node.
    pub(crate) item: String,
    /// Input and drag-source metadata (not yet surfaced by the TUI printer).
    #[allow(dead_code)]
    pub(crate) meta: dispatch::SigMeta,
}

/// Resolve all four parallel Effects signal channels.
pub fn collect_signals(inst: &kframe::Instance, eff: &dispatch::Effects, out: &mut Vec<Signal>) {
    for k in 0..eff.sig_name.len() {
        out.push(Signal {
            name: slab_kernel::slir::str_at(&inst.doc, eff.sig_name[k]),
            text: eff.sig_text[k].clone(),
            item: eff.sig_item[k].clone(),
            meta: eff.sig_meta[k].clone(),
        });
    }
}

/// Drains signals queued by a settled frame into the driver's signal stream.
pub fn drain_frame_signals(inst: &mut kframe::Instance, out: &mut Vec<Signal>) {
    let effects = kframe::inst_take_signals(inst);
    collect_signals(inst, &effects, out);
}

/// Ends host-owned pointer gestures and preserves every signal emitted before
/// the instance is discarded.
pub(crate) fn close_instance(inst: &mut kframe::Instance, out: &mut Vec<Signal>) {
    drain_frame_signals(inst, out);
    let effects = kframe::inst_dispatch(inst, &event_new(E_CLOSE));
    collect_signals(inst, &effects, out);
}

fn escape_signal_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Format a signal without losing its optional list-item identity or payload.
///
/// Metadata remains available to interactive hosts but is intentionally absent
/// from the compact, backwards-stable debug footer.
pub fn format_signal(signal: &Signal) -> String {
    let Signal {
        name,
        text,
        item,
        meta: _,
    } = signal;
    match (item.is_empty(), text.is_empty()) {
        (true, true) => name.clone(),
        (true, false) => format!("{name}=\"{}\"", escape_signal_value(text)),
        (false, true) => format!("{name}[item=\"{}\"]", escape_signal_value(item)),
        (false, false) => format!(
            "{name}[item=\"{}\"]=\"{}\"",
            escape_signal_value(item),
            escape_signal_value(text)
        ),
    }
}

/// Solve until stable: post-solve scroll re-clamp and focus restoration
/// mark the instance dirty for the NEXT frame (FRAME.md), so a settled
/// grid needs up to a few passes at the same clock.
pub fn settle_frame(inst: &mut kframe::Instance, t: f64) -> flatten::Frame {
    let mut fr = kframe::inst_frame(inst, t);
    for _ in 0..3 {
        if !inst.dirty {
            break;
        }
        fr = kframe::inst_frame(inst, t);
    }
    fr
}
