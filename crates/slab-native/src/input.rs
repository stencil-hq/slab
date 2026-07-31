//! Public winit → kernel input translation for external Rust hosts.
//!
//! Everything here is spec-bearing driver mechanics that every winit host
//! needs verbatim: multi-click counting (500ms / 4u windows), the kernel key
//! vocabulary, mouse-button ids, cursor-motion deltas, kernel cursor → winit
//! `CursorIcon`, IME composition state ([`ImeState`]), and the OS clipboard
//! recipe ([`Clipboard`], [`selection_text`]). The in-repo reference binaries
//! (`view`, `demo`, `player`) consume this module and nothing private, so the
//! public surface is proven sufficient for an out-of-repo host.

use std::time::{Duration, Instant};

use slab_kernel::{
	dispatch::{self as kdispatch, Effects},
	edit,
	frame::{self as kframe, Instance},
};
use winit::{
	dpi::{LogicalPosition, LogicalSize},
	event::{Ime, MouseButton},
	keyboard::{Key, NamedKey},
	window::{CursorIcon, Window},
};

/// Maximum delay between pointer-downs that still extends a multi-click.
pub const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
/// Squared maximum document-space distance (4u) between multi-click downs.
pub const MULTI_CLICK_DISTANCE_SQ: f64 = 4.0 * 4.0;

struct Click {
	at:     Instant,
	x:      f64,
	y:      f64,
	button: u32,
	count:  u32,
}

/// Counts consecutive native pointer-downs in document coordinates.
#[derive(Default)]
pub struct ClickCounter {
	last: Option<Click>,
}

impl ClickCounter {
	/// Records a pointer-down and returns its host-clock click count.
	pub fn pointer_down(&mut self, button: u32, x: f64, y: f64) -> u32 {
		self.click_at(Instant::now(), button, x, y)
	}

	fn click_at(&mut self, now: Instant, button: u32, x: f64, y: f64) -> u32 {
		let count = match &self.last {
			Some(last) => {
				let dx = x - last.x;
				let dy = y - last.y;
				if last.button == button
					&& now.duration_since(last.at) <= MULTI_CLICK_INTERVAL
					&& dy.mul_add(dy, dx * dx) <= MULTI_CLICK_DISTANCE_SQ
				{
					last.count.saturating_add(1)
				} else {
					1
				}
			},
			None => 1,
		};
		self.last = Some(Click { at: now, x, y, button, count });
		count
	}
}

/// Returns a stable kernel button id for every winit mouse button.
pub fn mouse_button_id(button: MouseButton) -> u32 {
	match button {
		MouseButton::Left => 0,
		MouseButton::Middle => 1,
		MouseButton::Right => 2,
		MouseButton::Back => 3,
		MouseButton::Forward => 4,
		MouseButton::Other(button) => 5 + u32::from(button),
	}
}

/// Records a cursor sample and returns its document-space motion delta.
pub fn cursor_delta(previous: &mut Option<(f64, f64)>, current: (f64, f64)) -> (f64, f64) {
	let delta = previous.map_or((0.0, 0.0), |(x, y)| (current.0 - x, current.1 - y));
	*previous = Some(current);
	delta
}

/// Maps a kernel cursor effect to its native window cursor.
pub const fn cursor_icon(cursor: u32) -> CursorIcon {
	match cursor {
		kdispatch::CUR_POINTER => CursorIcon::Pointer,
		kdispatch::CUR_TEXT => CursorIcon::Text,
		kdispatch::CUR_COL_RESIZE => CursorIcon::ColResize,
		kdispatch::CUR_ROW_RESIZE => CursorIcon::RowResize,
		_ => CursorIcon::Default,
	}
}

/// Normalizes winit keyboard keys to the kernel event vocabulary.
pub fn key_name(key: &Key) -> Option<String> {
	let name = match key {
		Key::Named(named) => match named {
			NamedKey::Enter => "Enter",
			NamedKey::Tab => "Tab",
			NamedKey::Space => " ",
			NamedKey::Escape => "Escape",
			NamedKey::Backspace => "Backspace",
			NamedKey::Delete => "Delete",
			NamedKey::Insert => "Insert",
			NamedKey::Home => "Home",
			NamedKey::End => "End",
			NamedKey::PageUp => "PageUp",
			NamedKey::PageDown => "PageDown",
			NamedKey::ArrowLeft => "ArrowLeft",
			NamedKey::ArrowRight => "ArrowRight",
			NamedKey::ArrowUp => "ArrowUp",
			NamedKey::ArrowDown => "ArrowDown",
			NamedKey::F1 => "F1",
			NamedKey::F2 => "F2",
			NamedKey::F3 => "F3",
			NamedKey::F4 => "F4",
			NamedKey::F5 => "F5",
			NamedKey::F6 => "F6",
			NamedKey::F7 => "F7",
			NamedKey::F8 => "F8",
			NamedKey::F9 => "F9",
			NamedKey::F10 => "F10",
			NamedKey::F11 => "F11",
			NamedKey::F12 => "F12",
			NamedKey::F13 => "F13",
			NamedKey::F14 => "F14",
			NamedKey::F15 => "F15",
			NamedKey::F16 => "F16",
			NamedKey::F17 => "F17",
			NamedKey::F18 => "F18",
			NamedKey::F19 => "F19",
			NamedKey::F20 => "F20",
			NamedKey::F21 => "F21",
			NamedKey::F22 => "F22",
			NamedKey::F23 => "F23",
			NamedKey::F24 => "F24",
			_ => return None,
		},
		Key::Character(character) => return Some(character.to_string()),
		_ => return None,
	};
	Some(name.to_string())
}

/// Whether kernel focus currently sits on a field with a bound edit state.
pub fn focus_in_field(i: &Instance) -> bool {
	kdispatch::ed_ix(&i.ds, kframe::inst_focus(i)) >= 0
}

/// Returns the focused field's selected committed text.
///
/// `None` means focus is not in a field; an empty string means a collapsed
/// selection. Offsets are codepoints, matching kernel `EditState`.
pub fn selection_text(i: &Instance) -> Option<String> {
	let index = kdispatch::ed_ix(&i.ds, kframe::inst_focus(i));
	let state = i.ds.ed.get(usize::try_from(index).ok()?)?;
	let lo = usize::try_from(edit::sel_lo(state)).ok()?;
	let hi = usize::try_from(edit::sel_hi(state)).ok()?;
	let cps = state.text.cps();
	Some(slab_kernel::rt::str_from_chars(cps.get(lo..hi)?))
}

/// IME composition tracker translating winit [`Ime`] events into kernel
/// composition events and mirroring kernel IME effects onto the window.
///
/// Per window: feed every `WindowEvent::Ime` to [`ImeState::on_ime`] and
/// dispatch the returned events. Suppress `KeyboardInput` while
/// [`ImeState::composing`] is true. Forward its `text` only when
/// [`ImeState::forwards_key_text`] is true. After each dispatch, call
/// [`ImeState::sync_rect`] and gate [`ImeState::set_allowed`] on
/// [`focus_in_field`].
#[derive(Default)]
pub struct ImeState {
	enabled:   bool,
	composing: bool,
	allowed:   Option<bool>,
	rect:      Option<(f64, f64, f64, f64)>,
}

impl ImeState {
	/// Whether a composition is active; hosts suppress raw key events then.
	pub const fn composing(&self) -> bool {
		self.composing
	}

	/// Whether `KeyboardInput.text` is the active text source.
	///
	/// An enabled IME delivers committed text through `Ime::Commit`, so hosts
	/// must suppress the matching raw key text to prevent duplicate `E_TEXT`.
	pub const fn forwards_key_text(&self) -> bool {
		!self.enabled && !self.composing
	}

	/// Translates one winit IME event into `(etype, text)` kernel events.
	///
	/// Emits `E_COMPOSITION_START`/`_UPDATE`/`_END` around a preedit, maps a
	/// commit without preedit (dead keys) to `E_TEXT`, treats an emptied
	/// preedit as a cancelled composition (`E_COMPOSITION_END` with empty
	/// text), and closes a composition left open by `Ime::Disabled`.
	pub fn on_ime(&mut self, ime: Ime) -> Vec<(u32, String)> {
		let mut out = Vec::new();
		match ime {
			Ime::Enabled => self.enabled = true,
			Ime::Preedit(text, _cursor) => {
				if !text.is_empty() {
					if !self.composing {
						self.composing = true;
						out.push((kdispatch::E_COMPOSITION_START, String::new()));
					}
					out.push((kdispatch::E_COMPOSITION_UPDATE, text));
				} else if self.composing {
					// The IME cleared its preedit (composition cancelled).
					self.composing = false;
					out.push((kdispatch::E_COMPOSITION_END, String::new()));
				}
			},
			Ime::Commit(text) => {
				if self.composing {
					self.composing = false;
					out.push((kdispatch::E_COMPOSITION_END, text));
				} else {
					// Direct commit without preedit (e.g. dead keys).
					out.push((kdispatch::E_TEXT, text));
				}
			},
			Ime::Disabled => {
				self.enabled = false;
				if self.composing {
					self.composing = false;
					out.push((kdispatch::E_COMPOSITION_END, String::new()));
				}
			},
		}
		out
	}

	/// Mirrors the kernel IME rectangle onto the window when it changes.
	pub fn sync_rect(&mut self, window: &Window, eff: &Effects) {
		if !eff.has_ime {
			return;
		}
		let rect = (eff.ime_x, eff.ime_y, eff.ime_w, eff.ime_h);
		if self.rect == Some(rect) {
			return;
		}
		self.rect = Some(rect);
		window.set_ime_cursor_area(
			LogicalPosition::new(eff.ime_x, eff.ime_y),
			LogicalSize::new(eff.ime_w.max(1.0), eff.ime_h),
		);
	}

	/// Enables or disables IME input, deduplicating repeated states.
	pub fn set_allowed(&mut self, window: &Window, allowed: bool) {
		if self.allowed == Some(allowed) {
			return;
		}
		self.allowed = Some(allowed);
		window.set_ime_allowed(allowed);
	}
}

/// Lazily connected OS clipboard (arboard) for the cut/copy/paste recipe.
///
/// The kernel never touches the system clipboard: copy reads
/// [`selection_text`] and writes it here; cut additionally dispatches
/// `E_CUT`; paste dispatches `E_PASTE` with [`Clipboard::read`]'s text.
#[derive(Default)]
pub struct Clipboard {
	inner:  Option<arboard::Clipboard>,
	failed: bool,
}

impl Clipboard {
	fn connect(&mut self) -> Option<&mut arboard::Clipboard> {
		if self.inner.is_none() && !self.failed {
			match arboard::Clipboard::new() {
				Ok(clipboard) => self.inner = Some(clipboard),
				Err(e) => {
					self.failed = true;
					eprintln!("slab-native: clipboard unavailable: {e}");
				},
			}
		}
		self.inner.as_mut()
	}

	/// Returns the clipboard's current text, or `None` when unavailable.
	pub fn read(&mut self) -> Option<String> {
		self.connect()?.get_text().ok()
	}

	/// Replaces the clipboard's text; returns whether the write succeeded.
	pub fn write(&mut self, text: &str) -> bool {
		self
			.connect()
			.is_some_and(|clipboard| clipboard.set_text(text).is_ok())
	}
}

#[cfg(test)]
mod input_tests {
	use std::time::{Duration, Instant};

	use slab_kernel::dispatch as kdispatch;
	use winit::{
		event::{Ime, MouseButton},
		keyboard::{Key, NamedKey},
		window::CursorIcon,
	};

	use super::{ClickCounter, ImeState, cursor_delta, cursor_icon, key_name, mouse_button_id};

	#[test]
	fn cursor_motion_requires_a_prior_sample() {
		let mut previous = None;

		assert_eq!(cursor_delta(&mut previous, (10.0, 20.0)), (0.0, 0.0));
		assert_eq!(cursor_delta(&mut previous, (13.0, 18.0)), (3.0, -2.0));
		previous = None;
		assert_eq!(cursor_delta(&mut previous, (50.0, 60.0)), (0.0, 0.0));
	}

	#[test]
	fn mouse_button_ids_are_stable_and_disjoint() {
		assert_eq!(mouse_button_id(MouseButton::Left), 0);
		assert_eq!(mouse_button_id(MouseButton::Middle), 1);
		assert_eq!(mouse_button_id(MouseButton::Right), 2);
		assert_eq!(mouse_button_id(MouseButton::Back), 3);
		assert_eq!(mouse_button_id(MouseButton::Forward), 4);
		assert_eq!(mouse_button_id(MouseButton::Other(0)), 5);
		assert_eq!(mouse_button_id(MouseButton::Other(u16::MAX)), 5 + u32::from(u16::MAX));
	}

	#[test]
	fn keyboard_and_cursor_mappings_match_kernel_vocabulary() {
		assert_eq!(key_name(&Key::Named(NamedKey::ArrowLeft)), Some("ArrowLeft".to_owned()));
		assert_eq!(key_name(&Key::Character("ß".into())), Some("ß".to_owned()));
		assert_eq!(cursor_icon(kdispatch::CUR_POINTER), CursorIcon::Pointer);
		assert_eq!(cursor_icon(kdispatch::CUR_TEXT), CursorIcon::Text);
		assert_eq!(cursor_icon(u32::MAX), CursorIcon::Default);
	}

	#[test]
	fn first_click_starts_at_one_and_matching_click_increments() {
		let start = Instant::now();
		let mut clicks = ClickCounter::default();

		assert_eq!(clicks.click_at(start, 0, 10.0, 20.0), 1);
		assert_eq!(clicks.click_at(start + Duration::from_millis(100), 0, 10.0, 20.0), 2);
	}

	#[test]
	fn click_after_interval_resets() {
		let start = Instant::now();
		let mut clicks = ClickCounter::default();

		assert_eq!(clicks.click_at(start, 0, 10.0, 20.0), 1);
		assert_eq!(clicks.click_at(start + Duration::from_millis(501), 0, 10.0, 20.0), 1);
	}

	#[test]
	fn click_beyond_distance_resets() {
		let start = Instant::now();
		let mut clicks = ClickCounter::default();

		assert_eq!(clicks.click_at(start, 0, 10.0, 20.0), 1);
		assert_eq!(clicks.click_at(start + Duration::from_millis(100), 0, 13.0, 24.0), 1);
	}

	#[test]
	fn click_with_different_button_resets() {
		let start = Instant::now();
		let mut clicks = ClickCounter::default();

		assert_eq!(clicks.click_at(start, 0, 10.0, 20.0), 1);
		assert_eq!(clicks.click_at(start + Duration::from_millis(100), 2, 10.0, 20.0), 1);
	}

	#[test]
	fn preedit_then_commit_brackets_a_composition() {
		let mut ime = ImeState::default();

		let events = ime.on_ime(Ime::Preedit("に".into(), None));
		assert_eq!(events, vec![
			(kdispatch::E_COMPOSITION_START, String::new()),
			(kdispatch::E_COMPOSITION_UPDATE, "に".to_string()),
		]);
		assert!(ime.composing());

		let events = ime.on_ime(Ime::Preedit("にほ".into(), None));
		assert_eq!(events, vec![(kdispatch::E_COMPOSITION_UPDATE, "にほ".to_string())]);

		let events = ime.on_ime(Ime::Commit("日本".into()));
		assert_eq!(events, vec![(kdispatch::E_COMPOSITION_END, "日本".to_string())]);
		assert!(!ime.composing());
	}

	#[test]
	fn commit_without_preedit_is_plain_text() {
		let mut ime = ImeState::default();
		let _ = ime.on_ime(Ime::Enabled);

		let events = ime.on_ime(Ime::Commit("é".into()));
		assert_eq!(events, vec![(kdispatch::E_TEXT, "é".to_string())]);
		assert!(!ime.composing());
		assert!(!ime.forwards_key_text());
		let _ = ime.on_ime(Ime::Disabled);
		assert!(ime.forwards_key_text());
	}

	#[test]
	fn emptied_preedit_cancels_the_composition() {
		let mut ime = ImeState::default();

		let _ = ime.on_ime(Ime::Preedit("か".into(), None));
		let events = ime.on_ime(Ime::Preedit(String::new(), None));
		assert_eq!(events, vec![(kdispatch::E_COMPOSITION_END, String::new())]);
		assert!(!ime.composing());

		// A stray empty preedit outside a composition stays silent.
		assert!(ime.on_ime(Ime::Preedit(String::new(), None)).is_empty());
	}

	#[test]
	fn disabled_closes_an_open_composition() {
		let mut ime = ImeState::default();

		let _ = ime.on_ime(Ime::Preedit("か".into(), None));
		let events = ime.on_ime(Ime::Disabled);
		assert_eq!(events, vec![(kdispatch::E_COMPOSITION_END, String::new())]);
		assert!(!ime.composing());
		assert!(ime.on_ime(Ime::Disabled).is_empty());
	}

	#[test]
	fn enabled_selects_ime_as_the_only_text_source() {
		let mut ime = ImeState::default();
		assert!(ime.forwards_key_text());
		assert!(ime.on_ime(Ime::Enabled).is_empty());
		assert!(!ime.forwards_key_text());
		assert!(ime.on_ime(Ime::Disabled).is_empty());
		assert!(ime.forwards_key_text());
	}
}
