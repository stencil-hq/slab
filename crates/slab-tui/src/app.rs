//! Shared driver plumbing: compile a `.slab` file into a live kernel
//! Instance, build kernel Events, and resolve signal names. Everything else
//! (layout, hit, focus, edit, motion, scroll, cells) is kernel-owned; this
//! crate only translates and paints.

use std::path::Path;

use slab_kernel::{dispatch, flatten, frame as kframe};

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
/// Whether a host shortcut consumed a terminal key or should let the kernel
/// handle it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyHandling {
	/// Do not dispatch the key or its paired printable-text event to the kernel.
	Consumed,
	/// Forward the key and any paired printable-text event to the kernel
	/// unchanged.
	Forward,
}

/// A translated terminal key offered to [`Host::on_key`].
///
/// The managed loop only offers keys while focus is outside an edit field, so
/// printable shortcuts cannot steal text entry. `focused_key` is the canonical
/// full scene key and `item` is the innermost stable list-item key, when any.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostKey {
	/// FRAME key name (`Enter`, `ArrowDown`, `F2`, or a printable character).
	pub key:         String,
	/// FRAME modifier bitset (`M_SHIFT | M_ALT | M_CTRL | M_META`).
	pub mods:        u32,
	/// Canonical full key of the focused node.
	pub focused_key: Option<String>,
	/// Innermost stable list-item key containing the focused node.
	pub item:        Option<String>,
}

/// Builds host-shortcut context for a translated key outside an edit field.
///
/// Returns `None` for non-key events and whenever the focused node is currently
/// editing. This is also useful to host-owned loops such as `slab-ratatui`.
pub fn host_key(inst: &kframe::Instance, event: &dispatch::Event) -> Option<HostKey> {
	if event.etype != E_KEY_DOWN {
		return None;
	}
	let focused = kframe::inst_focus(inst);
	if focused != slab_kernel::slir::NONE && dispatch::ed_ix(&inst.ds, focused) >= 0 {
		return None;
	}
	let focused_key = (focused != slab_kernel::slir::NONE)
		.then(|| slab_kernel::scene::key_of(inst.doc(), &inst.st.lists, focused));
	let item = (focused != slab_kernel::slir::NONE)
		.then(|| slab_kernel::list::item_key(&inst.st.lists, inst.doc(), focused))
		.filter(|item| !item.is_empty());
	Some(HostKey { key: event.key.clone(), mods: event.mods, focused_key, item })
}

/// Compiles FILE to SLIR bytes and its formatted §12 warnings.
///
/// Errors come back as joined diagnostics. Nothing is printed: the interactive
/// loop owns the alt screen, so callers surface diagnostics on their own terms.
pub fn compile(file: &Path) -> Result<(Vec<u8>, Vec<String>), String> {
	let src = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
	let opts = slab_compile::Options {
		embed_assets: true,
		base_dir:     file
			.parent()
			.unwrap_or_else(|| Path::new("."))
			.to_path_buf(),
		assets:       None,
		sources:      None,
		fonts:        std::collections::HashMap::new(),
	};
	let (slir, diags) = slab_compile::compile(&src, &opts);
	let name = file.display().to_string();
	if diags.has_errors() {
		let text: Vec<String> = diags.0.iter().map(|d| d.format(&name)).collect();
		return Err(text.join("\n"));
	}
	let slir = slir.ok_or("compile failed: no SLIR produced")?;
	Ok((slab_slir::write(&slir), diags.0.iter().map(|d| d.format(&name)).collect()))
}

/// Decode SLIR bytes into a kernel Instance plus embedded image payloads.
pub fn instance(bytes: &[u8]) -> Result<(kframe::Instance, Vec<Vec<u8>>), String> {
	slab_slir::instance(bytes)
}

const fn event_new(etype: u32) -> dispatch::Event {
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
pub const fn pointer_event(etype: u32, x: f64, y: f64) -> dispatch::Event {
	pointer_button_event(etype, x, y, 0, 0)
}

/// Builds a pointer event with a platform button and host-computed click count.
pub const fn pointer_button_event(
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

pub const fn wheel_event(x: f64, y: f64, dy: f64) -> dispatch::Event {
	let mut ev = event_new(E_WHEEL);
	ev.x = x;
	ev.y = y;
	ev.dy = dy;
	ev
}

/// Driver signal with its resolved name, payload, list identity, and metadata.
pub struct Signal {
	/// Resolved signal name.
	pub name: String,
	/// Committed text for Change, Submit, and Resize signals.
	pub text: String,
	/// Innermost list item key, or empty for a document node.
	pub item: String,
	/// Input and drag-source metadata (key names, drag identities).
	pub meta: dispatch::SigMeta,
}

/// Host hooks the driver loops call between kernel frames — the terminal
/// analogue of wslab's `SlabElement` callbacks. Every method has a no-op
/// default; implement only what the app needs.
pub trait Host {
	/// Reacts to one resolved kernel signal.
	fn on_signal(&mut self, inst: &mut kframe::Instance, signal: &Signal) -> Result<(), String> {
		let _ = (inst, signal);
		Ok(())
	}
	/// Intercepts one translated terminal key while focus is outside an edit
	/// field.
	///
	/// Return [`KeyHandling::Consumed`] after applying a host shortcut, or
	/// [`KeyHandling::Forward`] to preserve normal kernel dispatch. Printable
	/// text paired with a consumed key is consumed with it.
	fn on_key(&mut self, inst: &mut kframe::Instance, key: &HostKey) -> Result<KeyHandling, String> {
		let _ = (inst, key);
		Ok(KeyHandling::Forward)
	}
	/// Advances host time by `dt_ms` before a frame; param writes repaint.
	fn tick(&mut self, inst: &mut kframe::Instance, dt_ms: f64) -> Result<(), String> {
		let _ = (inst, dt_ms);
		Ok(())
	}
	/// Extra text appended to the `--debug` footer.
	fn badges(&self) -> String {
		String::new()
	}
}

impl Host for () {}

/// Resolve all four parallel Effects signal channels.
pub fn collect_signals(inst: &kframe::Instance, eff: &dispatch::Effects, out: &mut Vec<Signal>) {
	for k in 0..eff.sig_name.len() {
		out.push(Signal {
			name: slab_kernel::slir::str_at(inst.doc(), eff.sig_name[k]).to_owned(),
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
pub fn close_instance(inst: &mut kframe::Instance, out: &mut Vec<Signal>) {
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
	let Signal { name, text, item, meta: _ } = signal;
	match (item.is_empty(), text.is_empty()) {
		(true, true) => name.clone(),
		(true, false) => format!("{name}=\"{}\"", escape_signal_value(text)),
		(false, true) => format!("{name}[item=\"{}\"]", escape_signal_value(item)),
		(false, false) => {
			format!("{name}[item=\"{}\"]=\"{}\"", escape_signal_value(item), escape_signal_value(text))
		},
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn host_key_context_preserves_typed_key_and_modifiers() {
		let inst = kframe::inst_shell();
		let event = key_event("d", M_CTRL | M_SHIFT);
		assert_eq!(
			host_key(&inst, &event),
			Some(HostKey {
				key:         "d".to_string(),
				mods:        M_CTRL | M_SHIFT,
				focused_key: None,
				item:        None,
			})
		);
		assert!(host_key(&inst, &text_event("d")).is_none());
	}
}
